//! Fine-grained reactivity: signals whose readers are tracked per element.
//!
//! The model is fine-grained rather than top-down re-rendering, and
//! deliberately so (`docs/DESIGN.md` §1). Reading a signal *during a build*
//! records that the building element depends on it. Writing marks exactly those
//! elements pending. Nothing else in the tree learns anything happened.
//!
//! That is the whole reason a rebuild is cheap: the framework never diffs a
//! tree to discover what changed, because the write already said so.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};

use crate::ElementId;
use vieww_foundation::{FastMap, FastSet};

/// Identifies one signal within a [`Runtime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SignalId(u64);

/// Something that can read a signal and be told when it changed.
///
/// An element is the obvious one. A [`Memo`] is the other: it reads signals to
/// compute its value, so it has to appear in the same subscriber tables — and
/// it is *not* an element, so it cannot borrow an `ElementId` without the
/// pending set trying to rebuild something that has no build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Reader {
    Element(ElementId),
    /// Identified by the memo's own signal id, which is also what its readers
    /// subscribe to — so a chain of memos is just the same table twice.
    Memo(SignalId),
}

/// How many times [`Runtime::settle_memos`] will go round before it decides a
/// memo graph is not converging.
///
/// A memo that reads another memo is a chain, and each pass resolves one link;
/// a hundred is far past any real derivation and short enough that a cycle
/// fails in microseconds rather than hanging the frame.
const MAX_MEMO_PASSES: usize = 100;

#[derive(Default)]
struct RuntimeInner {
    next_signal: Cell<u64>,
    /// Readers currently computing, innermost last. A signal read attributes
    /// itself to the last entry.
    tracking: RefCell<Vec<Reader>>,
    /// signal -> readers that read it during their last build.
    subscribers: RefCell<FastMap<SignalId, FastSet<Reader>>>,
    /// reader -> signals it read. The reverse index, so that re-running or
    /// unmounting a reader can drop its old subscriptions without scanning
    /// every signal in the runtime.
    dependencies: RefCell<FastMap<Reader, FastSet<SignalId>>>,
    /// Elements awaiting rebuild. A set, so writing the same signal ten times
    /// between frames still costs one rebuild.
    pending: RefCell<FastSet<ElementId>>,
    /// Every live memo, by its own signal id, so a notification can reach one.
    ///
    /// `Weak`, so a memo the application dropped does not keep recomputing for
    /// nobody — and so the runtime does not become an owner of every derived
    /// value ever created.
    memos: RefCell<FastMap<SignalId, std::rc::Weak<dyn Recompute>>>,
    /// Memos whose inputs changed and whose value has not been re-derived yet.
    stale: RefCell<FastSet<SignalId>>,
}

/// The type-erased half of a [`Memo`], so the runtime can re-derive one without
/// knowing what it holds.
trait Recompute {
    /// Re-run the computation. Returns `true` if the value actually changed,
    /// which is what decides whether anybody is told.
    fn recompute(&self) -> bool;
}

/// Shared owner of every signal and every subscription in one tree.
///
/// Cheap to clone — it is a handle, and every clone addresses the same
/// reactive graph.
#[derive(Clone, Default)]
pub struct Runtime {
    inner: Rc<RuntimeInner>,
}

impl Runtime {
    /// A runtime with no signals and nothing subscribed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a signal holding `value`.
    #[must_use]
    pub fn signal<T: 'static>(&self, value: T) -> Signal<T> {
        let id = SignalId(self.inner.next_signal.get());
        self.inner.next_signal.set(id.0 + 1);
        Signal {
            id,
            value: Rc::new(RefCell::new(value)),
            runtime: self.clone(),
        }
    }

    /// `true` if both handles address the same reactive graph.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// Begin attributing signal reads to `element`.
    pub(crate) fn push_tracking(&self, element: ElementId) {
        self.inner
            .tracking
            .borrow_mut()
            .push(Reader::Element(element));
    }

    /// Stop attributing reads to the innermost element.
    pub(crate) fn pop_tracking(&self) {
        self.inner.tracking.borrow_mut().pop();
    }

    /// How many builds are currently open.
    ///
    /// Paired with [`truncate_tracking`](Self::truncate_tracking) to restore the
    /// stack after a build that unwound, which is the one case `pop_tracking`
    /// cannot be trusted for: the push and the pop sit either side of arbitrary
    /// user code, and a panic between them skips the pop.
    ///
    /// The element tree does not currently re-enter a build from inside one, so
    /// a single `pop` would in fact be enough today. This is depth-based anyway
    /// because the stack *is* a stack — [`push_tracking`] is public to the crate
    /// and the runtime already supports nesting — and an unwind restore that
    /// only works while nothing nests is a trap for whoever nests first.
    ///
    /// [`push_tracking`]: Self::push_tracking
    pub(crate) fn tracking_depth(&self) -> usize {
        self.inner.tracking.borrow().len()
    }

    /// Drop every tracking frame opened since the stack was `depth` deep.
    pub(crate) fn truncate_tracking(&self, depth: usize) {
        self.inner.tracking.borrow_mut().truncate(depth);
    }

    /// Record that whatever is currently computing read `signal`.
    fn track(&self, signal: SignalId) {
        let Some(&reader) = self.inner.tracking.borrow().last() else {
            // Read outside a build — from a test, an event handler, or app
            // code. Nothing depends on it, so there is nothing to record.
            return;
        };
        self.inner
            .subscribers
            .borrow_mut()
            .entry(signal)
            .or_default()
            .insert(reader);
        self.inner
            .dependencies
            .borrow_mut()
            .entry(reader)
            .or_default()
            .insert(signal);
    }

    /// Mark every reader of `signal`: an element to rebuild, a memo to
    /// re-derive.
    fn notify(&self, signal: SignalId) {
        let readers: Vec<Reader> = {
            let subscribers = self.inner.subscribers.borrow();
            let Some(readers) = subscribers.get(&signal) else {
                return;
            };
            readers.iter().copied().collect()
        };
        let mut pending = self.inner.pending.borrow_mut();
        let mut stale = self.inner.stale.borrow_mut();
        for reader in readers {
            match reader {
                Reader::Element(element) => {
                    pending.insert(element);
                }
                Reader::Memo(memo) => {
                    stale.insert(memo);
                }
            }
        }
    }

    /// Drop every subscription held by `element`.
    ///
    /// Called before a rebuild (its reads are about to be recorded afresh, and
    /// a branch it no longer takes must not keep waking it) and on unmount.
    pub(crate) fn clear_dependencies(&self, element: ElementId) {
        self.clear_reader(Reader::Element(element));
    }

    /// The same, for any reader.
    fn clear_reader(&self, reader: Reader) {
        let Some(signals) = self.inner.dependencies.borrow_mut().remove(&reader) else {
            return;
        };
        let mut subscribers = self.inner.subscribers.borrow_mut();
        for signal in signals {
            if let Some(readers) = subscribers.get_mut(&signal) {
                readers.remove(&reader);
                if readers.is_empty() {
                    subscribers.remove(&signal);
                }
            }
        }
    }

    /// Mark `element` pending directly, without going through a signal.
    pub(crate) fn mark_pending(&self, element: ElementId) {
        self.inner.pending.borrow_mut().insert(element);
    }

    /// Forget that `element` was pending — it has been rebuilt, or unmounted.
    pub(crate) fn clear_pending(&self, element: ElementId) {
        self.inner.pending.borrow_mut().remove(&element);
    }

    /// Snapshot the pending pending set without clearing it.
    ///
    /// The scheduler re-reads this before every single rebuild rather than
    /// draining once, because rebuilding a parent can clean a child that was
    /// independently pending. Draining up front would rebuild that child a second
    /// time, against a widget the parent had already replaced.
    pub(crate) fn pending_ids(&self) -> Vec<ElementId> {
        self.inner.pending.borrow().iter().copied().collect()
    }

    /// How many elements are waiting to rebuild.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.inner.pending.borrow().len()
    }

    /// How many elements currently subscribe to anything.
    ///
    /// Exposed for tests and devtools: a number that climbs frame over frame is
    /// a subscription leak.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.inner.dependencies.borrow().len()
    }

    /// A value derived from other signals, recomputed when they change and
    /// published only when the result actually differs.
    ///
    /// # What this is for
    ///
    /// A signal is a *place*; a memo is a *rule*. Everything an application
    /// derives — "the open files, without their text", "which line the gutter
    /// marks", "is the form submittable" — is a pure function of state that is
    /// already in signals, and writing it out by hand costs two things a
    /// framework should be paying instead:
    ///
    /// * **A door for every writer.** The derived value has to be recomputed
    ///   wherever an input is written, so every new write site is a chance to
    ///   forget one, and the failure is a stale screen rather than a compile
    ///   error. `viewwstudio` carries a test whose entire job is to catch that.
    /// * **A comparison at every door.** Publishing unconditionally rebuilds
    ///   everything downstream on every write, which is what
    ///   [`set_if_changed`](Signal::set_if_changed) exists to avoid — one more
    ///   thing to remember at each site.
    ///
    /// A memo does both, once, from the definition:
    ///
    /// ```
    /// # use vieww_element::Runtime;
    /// let rt = Runtime::new();
    /// let first = rt.signal("Ada".to_string());
    /// let last = rt.signal("Lovelace".to_string());
    ///
    /// let full = {
    ///     let (first, last) = (first.clone(), last.clone());
    ///     rt.memo(move || format!("{} {}", first.get(), last.get()))
    /// };
    /// assert_eq!(full.get(), "Ada Lovelace");
    ///
    /// last.set("Byron".to_string());
    /// assert_eq!(full.get(), "Ada Byron");
    /// ```
    ///
    /// # `T: PartialEq` is the whole point
    ///
    /// The comparison is what makes a memo worth having over a closure: a
    /// derivation whose *result* did not change tells nobody, so a keystroke
    /// that leaves the derived value equal costs the elements reading it
    /// nothing. A `T` with no equality has no way to make that claim, so it is
    /// a bound rather than an option.
    ///
    /// # When it re-derives
    ///
    /// Lazily, and at most once per change: a write marks the memo stale, and
    /// the value is re-derived by the next [`get`](Memo::get) or by
    /// [`settle_memos`](Self::settle_memos), whichever comes first. The element
    /// tree settles memos before it rebuilds, so a build never reads a stale
    /// one. Dependencies are re-recorded on every derivation, so a memo that
    /// stops reading a signal stops hearing about it — the same rule an
    /// element's build follows.
    ///
    /// # Do not write a signal from inside one
    ///
    /// A memo is a derivation and must be side-effect free, for the reason
    /// `docs/DESIGN.md` §1 gives about builds: the dependency tracking that
    /// decides what re-derives is unsound if computing a value changes the
    /// state it was computed from. Writing from a memo is not detected; it
    /// produces a graph that either settles at an arbitrary point or trips
    /// [`settle_memos`](Self::settle_memos)' pass limit.
    #[must_use]
    pub fn memo<T: PartialEq + 'static>(&self, compute: impl Fn() -> T + 'static) -> Memo<T> {
        let id = SignalId(self.inner.next_signal.get());
        self.inner.next_signal.set(id.0 + 1);
        let inner = Rc::new(MemoInner {
            id,
            runtime: self.clone(),
            compute: Box::new(compute),
            value: RefCell::new(None),
        });
        self.inner
            .memos
            .borrow_mut()
            .insert(id, Rc::downgrade(&(Rc::clone(&inner) as Rc<dyn Recompute>)));
        // Derived once here rather than on first read, so `Memo::peek` outside
        // a build is meaningful immediately and the dependency edges exist
        // before anything can write.
        inner.recompute();
        Memo { inner }
    }

    /// Re-derive every memo whose inputs have changed, and mark the elements
    /// reading the ones that actually moved.
    ///
    /// Run by [`ElementTree::rebuild_pending`](crate::ElementTree::rebuild_pending)
    /// before it rebuilds anything, so the tree never builds against a stale
    /// derivation. Idempotent and free when nothing is stale.
    ///
    /// Returns how many memos were re-derived, which is what a test asserts to
    /// show that an unrelated write cost nothing.
    ///
    /// # Panics
    ///
    /// If the graph has not converged after `MAX_MEMO_PASSES`, which means a
    /// memo is writing to something it reads — see [`memo`](Self::memo).
    pub fn settle_memos(&self) -> usize {
        let mut derived = 0usize;
        for pass in 0..MAX_MEMO_PASSES {
            let batch: Vec<SignalId> = {
                let mut stale = self.inner.stale.borrow_mut();
                if stale.is_empty() {
                    return derived;
                }
                stale.drain().collect()
            };
            for id in batch {
                // Taken out of the map so the borrow ends before `recompute`,
                // which reads signals and can touch every table here.
                let memo = self.inner.memos.borrow().get(&id).and_then(Weak::upgrade);
                let Some(memo) = memo else {
                    // Dropped by the application. Its row is swept below.
                    continue;
                };
                memo.recompute();
                derived += 1;
            }
            let _ = pass;
        }
        assert!(
            self.inner.stale.borrow().is_empty(),
            "memos did not settle after {MAX_MEMO_PASSES} passes — \
             a memo is writing to a signal it reads; a derivation must be side-effect free"
        );
        derived
    }

    /// How many memos are stale — a diagnostic, and what a test asserts to show
    /// that a write reached the derivation it should have.
    #[must_use]
    pub fn stale_memo_count(&self) -> usize {
        self.inner.stale.borrow().len()
    }

    /// Forget a dropped memo's row and its subscriptions.
    fn retire_memo(&self, id: SignalId) {
        self.inner.memos.borrow_mut().remove(&id);
        self.inner.stale.borrow_mut().remove(&id);
        self.clear_reader(Reader::Memo(id));
        // And anything that was reading *this* memo: its id is a signal id like
        // any other, so it has its own subscriber row to clear.
        self.inner.subscribers.borrow_mut().remove(&id);
    }
}

/// The shared half of a [`Memo`].
struct MemoInner<T: 'static> {
    id: SignalId,
    runtime: Runtime,
    compute: Box<dyn Fn() -> T>,
    /// `None` only between construction and the first derivation, which
    /// [`Runtime::memo`] does immediately.
    value: RefCell<Option<T>>,
}

impl<T: PartialEq + 'static> Recompute for MemoInner<T> {
    fn recompute(&self) -> bool {
        // Dependencies are recorded afresh, exactly as an element's build is:
        // a derivation that stops reading a signal must stop hearing about it.
        self.runtime.clear_reader(Reader::Memo(self.id));
        let depth = self.runtime.tracking_depth();
        self.runtime
            .inner
            .tracking
            .borrow_mut()
            .push(Reader::Memo(self.id));
        // `catch_unwind` is deliberately *not* used here. A panicking
        // derivation is a bug in the application's own pure function, and the
        // element tree's error boundary already covers the build that reads it;
        // swallowing it here would produce a memo holding a stale value with
        // nothing to say so.
        let next = (self.compute)();
        self.runtime.truncate_tracking(depth);

        let changed = {
            let mut held = self.value.borrow_mut();
            let changed = held.as_ref() != Some(&next);
            *held = Some(next);
            changed
        };
        if changed {
            self.runtime.notify(self.id);
        }
        changed
    }
}

/// A value derived from other signals.
///
/// Created by [`Runtime::memo`], which is where the argument for this type
/// lives. Cloning gives another handle to the same derivation, the way cloning
/// a [`Signal`] does — it does not duplicate the computation.
pub struct Memo<T: 'static> {
    inner: Rc<MemoInner<T>>,
}

impl<T: PartialEq + 'static> Memo<T> {
    /// This memo's identity within its runtime. It is a signal id, because to
    /// everything downstream a memo *is* a signal.
    #[must_use]
    pub fn id(&self) -> SignalId {
        self.inner.id
    }

    /// Read the derived value, subscribing whatever is currently computing.
    ///
    /// Re-derives first if an input has changed since the last read, so a
    /// caller can never observe a stale value — the settle pass is an
    /// optimisation that batches this, not the only thing that does it.
    #[must_use]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(Clone::clone)
    }

    /// Read the derived value through a closure, subscribing the reader.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.refresh();
        // Subscribed *after* refreshing, deliberately. Re-deriving notifies the
        // memo's existing readers, and the one calling right now is about to be
        // handed the fresh value — marking it pending would cost it a rebuild
        // to reach the answer it already has.
        self.inner.runtime.track(self.inner.id);
        let held = self.inner.value.borrow();
        f(held.as_ref().expect("a memo derives on construction"))
    }

    /// Read without subscribing, and without re-deriving.
    ///
    /// The `peek` of a memo: for a handler asking "what is the value right
    /// now". It can be one write behind if nothing has settled since — call
    /// [`Runtime::settle_memos`] first if that matters.
    #[must_use]
    pub fn peek(&self) -> T
    where
        T: Clone,
    {
        self.inner
            .value
            .borrow()
            .clone()
            .expect("a memo derives on construction")
    }

    /// `true` if an input has changed and the value has not been re-derived.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.inner
            .runtime
            .inner
            .stale
            .borrow()
            .contains(&self.inner.id)
    }

    /// The runtime this memo belongs to.
    #[must_use]
    pub fn runtime(&self) -> &Runtime {
        &self.inner.runtime
    }

    fn refresh(&self) {
        if self.is_stale() {
            self.inner
                .runtime
                .inner
                .stale
                .borrow_mut()
                .remove(&self.inner.id);
            self.inner.recompute();
        }
    }
}

impl<T: 'static> Clone for Memo<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T: fmt::Debug + 'static> fmt::Debug for Memo<T> {
    /// Untracked and without re-deriving: a debug print must never create a
    /// subscription, and must never be the thing that runs a computation.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memo")
            .field("id", &self.inner.id)
            .field("value", &self.inner.value.borrow())
            .finish()
    }
}

impl<T: 'static> Drop for MemoInner<T> {
    /// The last handle going away takes the memo's row and its subscriptions
    /// with it, so a runtime that outlives a screen does not accumulate
    /// derivations nothing reads.
    fn drop(&mut self) {
        self.runtime.retire_memo(self.id);
    }
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("signals", &self.inner.next_signal.get())
            .field("subscribers", &self.subscriber_count())
            .field("pending", &self.pending_count())
            .finish()
    }
}

/// A mutable value that rebuilds exactly the elements that read it.
///
/// Cloning is cheap and gives another handle to the *same* value — a signal is
/// a shared cell, not a copy. Widgets hold signals; the widget stays immutable
/// because the signal's value lives outside it.
pub struct Signal<T: 'static> {
    id: SignalId,
    value: Rc<RefCell<T>>,
    runtime: Runtime,
}

impl<T: 'static> Signal<T> {
    /// This signal's identity within its runtime.
    #[must_use]
    pub const fn id(&self) -> SignalId {
        self.id
    }

    /// Read the value, subscribing the element that is currently building.
    #[must_use]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(Clone::clone)
    }

    /// Read the value through a closure, subscribing the building element.
    ///
    /// Use this over [`get`](Self::get) when `T` is expensive to clone, or does
    /// not implement `Clone` at all.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.runtime.track(self.id);
        f(&self.value.borrow())
    }

    /// Read without subscribing.
    ///
    /// For reads that must not create a dependency — an event handler asking
    /// "what is the value right now", where rebuilding on change would be
    /// wrong or circular.
    #[must_use]
    pub fn peek(&self) -> T
    where
        T: Clone,
    {
        self.value.borrow().clone()
    }

    /// Replace the value and mark every subscriber pending.
    ///
    /// This does **not** rebuild anything. It schedules; the tree rebuilds when
    /// [`ElementTree::rebuild_pending`](crate::ElementTree::rebuild_pending) runs.
    /// Writing ten times before the next frame still costs one rebuild, and a
    /// write from inside an event handler cannot re-enter a build.
    pub fn set(&self, value: T) {
        *self.value.borrow_mut() = value;
        self.runtime.notify(self.id);
    }

    /// Replace the value, and mark subscribers pending **only if it differs**.
    ///
    /// [`set`](Self::set) cannot do this: `T` is not `PartialEq` in general, and
    /// a signal holding a closure or a `dyn` handle has no equality to ask
    /// about. So the comparison is opt-in on the types that have one, which in
    /// practice is every flag, index and enum in an application's state.
    ///
    /// # Why an application wants this
    ///
    /// A write that changes nothing still schedules a rebuild of every element
    /// that read the signal, and "set a flag that is already set" is not an
    /// unusual mistake — it is the *normal* shape of a handler that describes
    /// the world it wants rather than the delta it is making. `viewwstudio`'s
    /// `mark_dirty` is the example this was written for: it sets
    /// `dirty = true` on every keystroke, and after the first keystroke in a
    /// session that is always a no-op write. It was rebuilding the title bar,
    /// the status bar, the preview pane and the Render button on every
    /// character typed, to arrive at exactly the tree they already had.
    ///
    /// # When *not* to use it
    ///
    /// When the value is a large structure whose comparison costs more than the
    /// rebuild it might save — a `Signal<Rc<Vec<Buffer>>>` holding a megabyte of
    /// text — and when the write is known to change something. Equality on an
    /// `Rc` is cheap only if the impl short-circuits on pointer identity, and
    /// most do not.
    /// # It still stores, even when it does not notify
    ///
    /// The comparison decides **who is told**, not what is held: the value goes
    /// in either way. That costs one move — a pointer swap and a refcount for
    /// the `Rc`s this is usually called with — and it buys the one guarantee
    /// worth having, which is that this method and [`set`](Self::set) leave the
    /// signal holding exactly the same thing.
    ///
    /// The alternative, returning early without storing, is cheaper and is what
    /// most reactive libraries do. It is also a silent data loss for any type
    /// whose `PartialEq` is coarser than its contents — a struct comparing on an
    /// id, an enum comparing on its discriminant, a newtype ignoring a cache
    /// field. Nothing in the signature warns about that, the value that is
    /// dropped is the *new* one, and the bug it produces is a field that is
    /// permanently one write stale. Paying a move to make that impossible is
    /// the right trade for a method whose whole purpose is to be reached for
    /// without much thought.
    pub fn set_if_changed(&self, value: T)
    where
        T: PartialEq,
    {
        // Scoped so the borrow ends before `notify`, which can reach a
        // subscriber that reads this same signal.
        let changed = {
            let mut held = self.value.borrow_mut();
            let changed = *held != value;
            *held = value;
            changed
        };
        if changed {
            self.runtime.notify(self.id);
        }
    }

    /// Mutate the value in place and mark every subscriber pending.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.value.borrow_mut());
        self.runtime.notify(self.id);
    }

    /// The runtime this signal belongs to.
    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }
}

impl<T: 'static> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            value: Rc::clone(&self.value),
            runtime: self.runtime.clone(),
        }
    }
}

impl<T: fmt::Debug + 'static> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Untracked: a debug print must never create a subscription.
        write!(f, "Signal({:?})", self.value.borrow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ElementId = ElementId::new(0, 0, 0);
    const B: ElementId = ElementId::new(0, 1, 0);

    #[test]
    fn a_read_outside_a_build_subscribes_nothing() {
        let rt = Runtime::new();
        let count = rt.signal(0);

        assert_eq!(count.get(), 0);
        count.set(1);

        assert_eq!(rt.pending_count(), 0);
        assert_eq!(rt.subscriber_count(), 0);
    }

    /// The point of `set_if_changed`: a write that changes nothing schedules
    /// nothing. `viewwstudio`'s `mark_dirty` sets an already-`true` flag on
    /// every keystroke, and with a plain `set` that rebuilt four regions per
    /// character typed.
    #[test]
    fn setting_the_same_value_schedules_nothing() {
        let rt = Runtime::new();
        let dirty = rt.signal(false);

        rt.push_tracking(A);
        let _ = dirty.get();
        rt.pop_tracking();

        dirty.set_if_changed(false);
        assert_eq!(rt.pending_count(), 0, "nothing changed");

        dirty.set_if_changed(true);
        assert_eq!(rt.pending_count(), 1, "it went true");
        assert!(dirty.get());

        rt.clear_pending(A);
        dirty.set_if_changed(true);
        assert_eq!(rt.pending_count(), 0, "it was already true");
    }

    /// `set_if_changed` still *stores* the value it was handed even when it
    /// does not notify — the comparison decides who is told, not what is held.
    /// A type whose `PartialEq` ignores a field would otherwise silently keep
    /// the old one.
    #[test]
    fn an_equal_write_is_still_a_write() {
        /// Equal on `id` alone, so two values can compare equal and differ.
        struct Tagged {
            id: u8,
            note: &'static str,
        }
        impl PartialEq for Tagged {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }

        let rt = Runtime::new();
        let held = rt.signal(Tagged {
            id: 1,
            note: "before",
        });

        rt.push_tracking(A);
        let _ = held.with(|value| value.id);
        rt.pop_tracking();

        held.set_if_changed(Tagged {
            id: 1,
            note: "after",
        });
        assert_eq!(rt.pending_count(), 0, "equal, so nobody is told");
        assert_eq!(held.with(|value| value.note), "after", "but it is stored");
    }

    #[test]
    fn only_elements_that_read_the_signal_go_pending() {
        let rt = Runtime::new();
        let watched = rt.signal(0);
        let ignored = rt.signal(0);

        rt.push_tracking(A);
        let _ = watched.get();
        rt.pop_tracking();

        rt.push_tracking(B);
        let _ = ignored.get();
        rt.pop_tracking();

        watched.set(1);

        assert_eq!(rt.pending_ids(), vec![A], "B never read the written signal");
    }

    #[test]
    fn repeated_writes_between_frames_collapse_to_one_rebuild() {
        let rt = Runtime::new();
        let count = rt.signal(0);

        rt.push_tracking(A);
        let _ = count.get();
        rt.pop_tracking();

        count.set(1);
        count.set(2);
        count.update(|v| *v += 1);

        assert_eq!(rt.pending_count(), 1);
        assert_eq!(count.peek(), 3);
    }

    #[test]
    fn peek_reads_without_subscribing() {
        let rt = Runtime::new();
        let count = rt.signal(7);

        rt.push_tracking(A);
        assert_eq!(count.peek(), 7);
        rt.pop_tracking();

        count.set(8);
        assert_eq!(rt.pending_count(), 0);
    }

    #[test]
    fn clearing_dependencies_stops_the_element_waking_up() {
        let rt = Runtime::new();
        let count = rt.signal(0);

        rt.push_tracking(A);
        let _ = count.get();
        rt.pop_tracking();
        assert_eq!(rt.subscriber_count(), 1);

        rt.clear_dependencies(A);
        assert_eq!(rt.subscriber_count(), 0);

        count.set(1);
        assert_eq!(rt.pending_count(), 0, "a cleared element must stay asleep");
    }

    #[test]
    fn nested_builds_attribute_reads_to_the_innermost_element() {
        let rt = Runtime::new();
        let inner_only = rt.signal(0);

        rt.push_tracking(A);
        rt.push_tracking(B);
        let _ = inner_only.get();
        rt.pop_tracking();
        rt.pop_tracking();

        inner_only.set(1);
        assert_eq!(rt.pending_ids(), vec![B]);
    }

    #[test]
    fn cloned_signals_share_one_value() {
        let rt = Runtime::new();
        let a = rt.signal(String::from("one"));
        let b = a.clone();

        b.set(String::from("two"));
        assert_eq!(a.peek(), "two");
        assert_eq!(a.id(), b.id());
    }
}
