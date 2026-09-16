//! Work that happens off the UI thread, and how its result gets back.
//!
//! Two traits and a state machine, and deliberately nothing else — no executor,
//! no futures, no runtime.
//!
//! The design was written before the code and the design's central claim held:
//! **the element layer needed no changes at all.** The result path is
//! `ElementState::take_pending`, the spawn is `create_state`, and the cancellation
//! is `Drop` — three mechanisms that already existed for other reasons.
//!
//! # Why there is no executor here
//!
//! An application doing anything slow has already chosen a runtime, and being
//! made to run a second one is worse than having none. [`Spawn`] is
//! `Box<dyn FnOnce() + Send>`, which tokio, smol, rayon and a bare
//! `std::thread` can all drive in one line, so vieww takes no position.
//!
//! Taking no position is not the same as leaving the common case unserved,
//! which is what [`Threads`] alone did: an application with no runtime got
//! one OS thread per in-flight task. [`Pool`] is the bounded default that
//! needs no runtime and no dependency — still not an executor, still not
//! polling futures, just a fixed set of workers behind the same seam.
//!
//! # The constraint everything is arranged around
//!
//! `Signal<T>` holds `Rc<RefCell<T>>` and is `!Send` by construction — see
//! `docs/DESIGN.md` §1, which chose that deliberately. **A worker thread cannot
//! write a signal.** So the result crosses back through a channel, and the
//! element that was waiting notices it during the frame, on the UI thread,
//! where writing is allowed.
//!
//! ```
//! use vieww_foundation::task::{AsyncValue, Inline, NoWaker, Task};
//! use std::sync::Arc;
//!
//! let mut task: Task<u32, String> =
//!     Task::spawn(&Inline, Arc::new(NoWaker), || Ok(2 + 2));
//!
//! assert!(task.poll(), "the value arrived, so the element must rebuild");
//! assert!(matches!(task.value(), AsyncValue::Ready(4)));
//! assert!(!task.poll(), "and nothing has changed since");
//! ```

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

/// Runs work somewhere that is not the UI thread.
///
/// The whole seam. An application that already has a runtime implements this
/// over it; one that does not uses [`Threads`].
pub trait Spawn: Send + Sync + 'static {
    /// Run `work`, not here.
    ///
    /// Must not block the caller: this is called from the UI thread, usually
    /// from an element being mounted, and a frame is waiting on it.
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>);
}

/// A thread per task.
///
/// The obvious implementation, and a perfectly good one for a handful of
/// concurrent loads. It is not a thread *pool* — a screen that starts two
/// hundred tasks starts two hundred threads. For that, use [`Pool`], which is
/// bounded and costs no more to reach for.
#[derive(Debug, Clone, Copy, Default)]
pub struct Threads;

impl Spawn for Threads {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        // The handle is dropped, which detaches. Nothing joins these: the
        // result arrives through the channel, and a task whose receiver is gone
        // has nobody to report to anyway.
        std::thread::spawn(work);
    }
}

/// A fixed set of worker threads sharing one queue.
///
/// # Why this exists, given [`Threads`] already did
///
/// [`Threads`]'s own doc names the problem it has: "a screen that starts two
/// hundred tasks starts two hundred threads", and its advice was to bring a
/// real runtime. That is right for an application which already has one, and
/// wrong for the far more common case — an application whose async work is a
/// dozen image loads and two HTTP calls, which then has to choose a runtime,
/// add it to its manifest and write a [`Spawn`] impl over it purely to avoid
/// a thread-per-image failure mode it will not notice until a list scrolls.
///
/// So this is the third option: bounded concurrency, no dependency, and
/// nothing to configure in the case where the right number of threads is
/// obviously "about as many as there are cores".
///
/// # What it is not
///
/// Not a runtime. No work stealing, no timer wheel, no `async fn`, no I/O
/// reactor, and it does not poll futures — [`Spawn`] hands over a blocking
/// closure and this runs blocking closures. An application that wants
/// `async`/`await` still wants tokio or smol behind [`Spawn`], and that is
/// still one line. This module's opening note stands: vieww takes no position
/// on runtimes. It just no longer charges an application for not having one.
///
/// # Behaviour worth knowing
///
/// The queue is unbounded and the workers are not. Submitting more work than
/// the pool can chew queues it rather than blocking the caller — blocking the
/// caller is the one thing [`Spawn::spawn`]'s contract forbids — so a burst of
/// two hundred loads becomes two hundred queued closures and
/// [`threads`](Self::threads) threads, which is exactly the trade this type
/// exists to make.
///
/// **A panicking task does not take its worker with it.** The worker catches
/// it, which is the same per-task guarantee `std::thread::spawn` gives under
/// [`Threads`]; without it a pool would lose a worker permanently the first
/// time somebody unwrapped a `None`, and lose all of them eventually — a
/// program that gets slower and then silently stops loading anything. The task
/// still reports [`AsyncValue::Lost`] through the same `Drop`-guard wake path
/// as before, so the panic is visible on screen rather than only in a log.
///
/// Dropping the pool lets queued work finish and then joins every worker,
/// rather than detaching: a process that exits while a worker is still writing
/// is the other way to lose data here.
///
/// ```
/// use vieww_foundation::task::{AsyncValue, NoWaker, Pool, Task};
/// use std::sync::Arc;
///
/// let pool = Pool::new();
/// let mut task: Task<u32, String> =
///     Task::spawn(&pool, Arc::new(NoWaker), || Ok(2 + 2));
///
/// // The work is on a worker thread, so wait for it — which is what a frame
/// // does not do, and why `poll` is the shape it is.
/// while !task.poll() {
///     std::thread::yield_now();
/// }
/// assert!(matches!(task.value(), AsyncValue::Ready(4)));
/// ```
pub struct Pool {
    /// `None` once [`Drop`] has taken it, which is itself the stop signal: a
    /// channel with no senders left ends its receiver's loop, so there is no
    /// separate shutdown flag that could disagree with reality.
    sender: Option<mpsc::Sender<Box<dyn FnOnce() + Send + 'static>>>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl Pool {
    /// A pool with one worker per available core.
    ///
    /// `available_parallelism`, not a constant — and clamped to at least one,
    /// because it returns an error on platforms that cannot answer and a pool
    /// with zero workers accepts work and never runs it. That presents as an
    /// application which loads forever, the least debuggable failure available
    /// here.
    #[must_use]
    pub fn new() -> Self {
        let threads = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self::with_threads(threads)
    }

    /// A pool with exactly `threads` workers, clamped to at least one.
    ///
    /// For when the right number is known and is not the core count: a pool
    /// dedicated to reads off one spinning disk wants one, and a test wants
    /// one so that completion order is deterministic.
    #[must_use]
    pub fn with_threads(threads: usize) -> Self {
        let threads = threads.max(1);
        let (sender, receiver) = mpsc::channel::<Box<dyn FnOnce() + Send + 'static>>();
        // One `Mutex<Receiver>` shared by every worker rather than a lock-free
        // queue: the lock is held for the length of a `recv` handoff and never
        // across running a task, so workers contend for a pointer move rather
        // than for an image decode.
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..threads)
            .map(|_| {
                let receiver = Arc::clone(&receiver);
                std::thread::spawn(move || loop {
                    // Scoped, so the guard drops before the work runs. Holding
                    // it across the call would serialise the whole pool into
                    // one worker with extra steps — and would do it silently.
                    let work = {
                        let Ok(queue) = receiver.lock() else {
                            // Poisoned: a worker died somewhere the catch below
                            // does not cover. Retire this worker rather than
                            // spin on the error forever.
                            return;
                        };
                        queue.recv()
                    };
                    let Ok(work) = work else { return };
                    // See this type's doc. `AssertUnwindSafe` is sound because
                    // nothing here is observed after the catch: the closure is
                    // consumed either way, and whatever it captured is the
                    // task's own business.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
                })
            })
            .collect();
        Self {
            sender: Some(sender),
            workers,
        }
    }

    /// How many workers this pool has.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.workers.len()
    }
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Pool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pool")
            .field("threads", &self.workers.len())
            .finish()
    }
}

impl Spawn for Pool {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        if let Some(sender) = &self.sender {
            // A send failure means every worker is gone, which can only be
            // true during drop. Dropping the work is the right answer: there
            // is nothing left to run it on, and the task's own receiver
            // reports `Lost` rather than waiting for a result that cannot come.
            let _ = sender.send(work);
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // Closing the channel is the shutdown signal — see `sender`'s comment.
        // It has to happen *before* the join below rather than as part of
        // `self` being dropped afterwards: a worker still holding a live
        // sender blocks forever on work that can no longer arrive.
        self.sender = None;
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// Runs work immediately, on the calling thread.
///
/// For tests, and for a headless harness that wants an application's async
/// paths to resolve deterministically rather than at some point later. It makes
/// [`Task::poll`] true on the very next call, which is exactly what a test
/// wants and exactly what a UI must not do.
#[derive(Debug, Clone, Copy, Default)]
pub struct Inline;

impl Spawn for Inline {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        work();
    }
}

/// Asks the platform for a frame, from any thread.
///
/// # Why this is not optional
///
/// A [`Task`]'s result is noticed during a frame. An application waiting on a
/// network response is not drawing frames — nothing is animating and nobody is
/// touching the screen — so without this the response sits in a channel until
/// the user happens to prod the app. "It updates when you touch it" is not a
/// loading state.
///
/// `FrameScheduler::request_frame` cannot do this job: it is `!Send` and it is
/// not reachable from a worker. The platform bridge implements this over
/// whatever its event loop offers.
///
/// # The one part of async that has never been verified on a device
///
/// Everything else here is covered by tests. **This is not**, and it cannot be:
/// the failure it prevents is a value arriving at a genuinely idle application,
/// and a desktop under test is never idle — a mouse crossing the window
/// produces the frame that hides the bug. A test's waker sets a flag, which
/// proves the call happens and nothing about whether a real event loop wakes.
///
/// So the outstanding check is on a phone, with nothing touching the screen: a
/// task that completes after a second must draw. Until somebody does that, treat
/// "the app updates while idle" as designed rather than as known.
///
/// The related trap is already fixed and worth not reintroducing: the wake fires
/// from a `Drop` guard, so it happens on the unwind path too. A plain
/// `waker.wake()` after the work is skipped when the work panics — which leaves
/// the application asleep holding a task that will never resolve, and makes
/// [`AsyncValue::Lost`] invisible by never drawing the frame that would show it.
pub trait FrameWaker: Send + Sync + 'static {
    /// Cause a frame to happen, soon.
    ///
    /// Called from the worker thread, possibly while it is unwinding — see
    /// [`Task::spawn`]. Must not panic and must not block.
    fn wake(&self);
}

/// A waker that does nothing.
///
/// Correct for a headless harness driving frames itself, and for [`Inline`],
/// where the work is finished before there is anything to wake.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoWaker;

impl FrameWaker for NoWaker {
    fn wake(&self) {}
}

/// A waker that counts, for tests.
#[derive(Debug, Clone, Default)]
pub struct WakeCount(Arc<AtomicUsize>);

impl WakeCount {
    /// A fresh counter, at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many times anything has woken through this.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl FrameWaker for WakeCount {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// What a widget waiting on a [`Task`] has to draw.
///
/// # Four states, and the fourth one was not in the design
///
/// The design specified three — pending, ready, failed — on the argument that
/// omitting the error case teaches every application to `unwrap`, which on a
/// phone is the *common* path because an app is offline more often than a
/// desktop is. That still holds.
///
/// Writing it turned up a fourth: **the work can end without producing either.**
/// A
/// panicking worker unwinds, its sender drops, and no `E` was ever created, so
/// there is nothing to put in `Failed`.
///
/// Leaving that as `Pending` would be an infinite spinner, which is the worst
/// of the available outcomes — it looks like a slow network rather than a bug,
/// so nobody investigates. Hence [`Lost`](Self::Lost).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncValue<T, E> {
    /// Still running.
    Pending,
    /// Finished, with a value.
    Ready(T),
    /// Finished, with an error the work produced deliberately.
    Failed(E),
    /// Ended without producing either — in practice, the work panicked.
    ///
    /// With a crash reporter installed the panic itself has already been
    /// captured with its backtrace, so this is the *display* side of something
    /// already recorded rather than the only trace of it.
    Lost,
}

impl<T, E> AsyncValue<T, E> {
    /// `true` while the work is still running.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    /// `true` once it has stopped, however it stopped.
    #[must_use]
    pub const fn is_settled(&self) -> bool {
        !self.is_pending()
    }

    /// The value, if there is one.
    #[must_use]
    pub const fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    /// The error, if the work produced one. `None` for [`Lost`](Self::Lost),
    /// which by definition has no `E`.
    #[must_use]
    pub const fn failed(&self) -> Option<&E> {
        match self {
            Self::Failed(error) => Some(error),
            _ => None,
        }
    }
}

/// Wakes when it is dropped, however that happens.
///
/// The wake has to be sent whether the work returned or unwound. A bare call
/// after `work()` is skipped by a panic, which leaves the application asleep
/// holding a task that will never resolve — the exact case [`AsyncValue::Lost`]
/// exists to display, made invisible by never drawing another frame.
struct WakeOnDrop(Arc<dyn FrameWaker>);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        self.0.wake();
    }
}

/// One piece of off-thread work, and the value it will produce.
///
/// Held by the `ElementState` of the widget waiting on it, and **that placement
/// is the whole design.** It buys two mechanisms without building either.
///
/// # The result path already existed
///
/// `ElementState::take_pending` is polled every frame, answers "did anything write
/// this state since the last one", and rebuilds exactly that element. That is
/// precisely what a finished request needs, so no async phase was added to the
/// pipeline and nothing drains a queue in the platform layer — which would also
/// have put the mechanism somewhere it could not be tested without a window.
///
/// `take_pending`'s own documentation warns it is for *ephemeral view state a
/// gesture writes, nothing else*, so using it here needs saying out loud. The
/// distinction it draws is **durable versus ephemeral ownership**, not gestures:
/// an in-flight request's *status* is ephemeral state nothing above the widget
/// wants to own, exactly like a press highlight. The *data* is usually the
/// opposite and belongs in a `Signal` if anyone else reads it. Status in the
/// state, payload in a signal — a split rather than a fudge.
///
/// # Cancellation is `Drop`, because disposal already is
///
/// Unmounting drops the state, which drops the receiver, which makes the
/// worker's `send` fail; the worker discards the value and exits. No new hook,
/// and nothing an implementor can forget to call — a dropped receiver is the one
/// cancellation signal that cannot go stale, because there is no flag to leave
/// unset.
///
/// **This stops the result being delivered, not the work.** Aborting an
/// in-flight request belongs to whatever is doing it, and a [`Spawn`] that wants
/// that carries its own token. Claiming otherwise would be the API lying about
/// what it controls.
pub struct Task<T, E> {
    value: AsyncValue<T, E>,
    /// `None` once the result has arrived and the channel is spent.
    receiver: Option<Receiver<Result<T, E>>>,
}

impl<T: fmt::Debug, E: fmt::Debug> fmt::Debug for Task<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Task")
            .field("value", &self.value)
            .field("running", &self.receiver.is_some())
            .finish()
    }
}

impl<T: Send + 'static, E: Send + 'static> Task<T, E> {
    /// Start `work` on `spawner`, and ask `waker` for a frame when it ends.
    ///
    /// Returns immediately, [`Pending`](AsyncValue::Pending) — unless `spawner`
    /// is [`Inline`], which finishes the work before this returns and leaves the
    /// result waiting for the first [`poll`](Self::poll).
    ///
    /// # Where to call it
    ///
    /// From `ElementState::mounted`, or from `widget_updated` when the thing
    /// being awaited changes. **Never from `build`** — `docs/DESIGN.md` §1 makes
    /// a build side-effect free, and a build that spawned would fire again on
    /// every unrelated rebuild. That is the classic refetch bug,
    /// and the hooks that avoid it already exist.
    pub fn spawn<F>(spawner: &dyn Spawn, waker: Arc<dyn FrameWaker>, work: F) -> Self
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();

        spawner.spawn(Box::new(move || {
            // Constructed before the work, so it is dropped — and therefore
            // wakes — on the way out of an unwind as well as a return.
            let _wake = WakeOnDrop(waker);
            // A failed send means the waiting element was unmounted while this
            // was running. Dropping the value is the correct response and is
            // what makes unmounting a cancellation.
            let _ = sender.send(work());
        }));

        Self {
            value: AsyncValue::Pending,
            receiver: Some(receiver),
        }
    }

    /// A task that is finished before it starts.
    ///
    /// For a widget handed a value it already has — a cache hit — so that the
    /// waiting and the not-waiting cases are the same type and the same widget.
    #[must_use]
    pub const fn ready(value: T) -> Self {
        Self {
            value: AsyncValue::Ready(value),
            receiver: None,
        }
    }

    /// A task that has already failed, with a reason.
    ///
    /// For a caller that knows the answer before it starts: a capability that
    /// is not available on this platform, a request rejected on its arguments,
    /// a permission already refused. The alternative — spawning a worker whose
    /// only job is to return an error — costs a thread hop to deliver a value
    /// that was known at the call site, and makes an immediate refusal
    /// indistinguishable to the caller from a slow one.
    ///
    /// Distinct from [`lost`](Self::lost): that one has no `E` at all, and the
    /// difference is what a screen can put in front of a user.
    #[must_use]
    pub const fn failed(error: E) -> Self {
        Self {
            value: AsyncValue::Failed(error),
            receiver: None,
        }
    }

    /// A task whose work was never started.
    ///
    /// For a caller that has nothing to run and must still produce a `Task` —
    /// `AsyncBuilder` asked to mount a second time, whose `FnOnce` the first
    /// mount already took. [`Lost`](AsyncValue::Lost) rather than
    /// [`Pending`](AsyncValue::Pending) because nothing is coming, and a
    /// spinner that will never resolve is the worst way to say so.
    #[must_use]
    pub const fn lost() -> Self {
        Self {
            value: AsyncValue::Lost,
            receiver: None,
        }
    }

    /// Take delivery of the result if it has arrived.
    ///
    /// Returns `true` if the value changed, which is exactly
    /// `ElementState::take_pending`'s contract — return it from there and the
    /// element tree rebuilds this element and nothing else.
    ///
    /// Cheap enough to call on every frame: a `try_recv` on an empty channel,
    /// and nothing at all once the task has settled.
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.receiver else {
            return false;
        };

        match receiver.try_recv() {
            Ok(Ok(value)) => self.settle(AsyncValue::Ready(value)),
            Ok(Err(error)) => self.settle(AsyncValue::Failed(error)),
            Err(TryRecvError::Empty) => false,
            // The sender is gone and sent nothing, so the work ended without
            // producing a result. See `AsyncValue::Lost`.
            Err(TryRecvError::Disconnected) => self.settle(AsyncValue::Lost),
        }
    }

    /// Record the outcome and close the channel.
    fn settle(&mut self, value: AsyncValue<T, E>) -> bool {
        self.value = value;
        self.receiver = None;
        true
    }

    /// What to draw.
    #[must_use]
    pub const fn value(&self) -> &AsyncValue<T, E> {
        &self.value
    }

    /// `true` while the result has not arrived.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.value.is_pending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Spin until `done`, or fail. Bounded, so a broken task fails the suite
    /// rather than hanging it.
    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if done() {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn an_inline_task_is_ready_on_the_first_poll() {
        let mut task: Task<u32, ()> = Task::spawn(&Inline, Arc::new(NoWaker), || Ok(7));
        assert!(task.is_pending(), "nothing is delivered until it is polled");
        assert!(task.poll());
        assert_eq!(task.value().ready(), Some(&7));
    }

    #[test]
    fn polling_a_settled_task_reports_no_further_change() {
        let mut task: Task<u32, ()> = Task::spawn(&Inline, Arc::new(NoWaker), || Ok(7));
        assert!(task.poll());
        assert!(
            !task.poll(),
            "a state that keeps reporting pending rebuilds its element forever"
        );
        assert!(!task.poll());
    }

    #[test]
    fn an_error_the_work_produced_arrives_as_failed() {
        let mut task: Task<u32, String> =
            Task::spawn(&Inline, Arc::new(NoWaker), || Err("no network".to_owned()));
        assert!(task.poll());
        assert_eq!(task.value().failed(), Some(&"no network".to_owned()));
        assert!(task.value().is_settled());
    }

    #[test]
    fn a_panicking_worker_ends_as_lost_rather_than_pending_forever() {
        let waker = WakeCount::new();
        let mut task: Task<u32, String> = Task::spawn(&Threads, Arc::new(waker.clone()), || {
            panic!("the work broke")
        });

        until("the worker to die", || waker.count() > 0);
        until("the task to notice", || task.poll());

        assert_eq!(
            *task.value(),
            AsyncValue::Lost,
            "staying Pending would be an infinite spinner that looks like a \
             slow network rather than a bug"
        );
    }

    #[test]
    fn the_waker_fires_even_when_the_work_panics() {
        let waker = WakeCount::new();
        let _task: Task<u32, ()> =
            Task::spawn(&Threads, Arc::new(waker.clone()), || panic!("broken"));

        until("a wake", || waker.count() == 1);
        assert_eq!(
            waker.count(),
            1,
            "without a wake nothing draws another frame, so nothing ever \
             observes the failure"
        );
    }

    #[test]
    fn the_waker_fires_exactly_once_for_work_that_succeeds() {
        let waker = WakeCount::new();
        let mut task: Task<u32, ()> = Task::spawn(&Threads, Arc::new(waker.clone()), || Ok(1));

        until("the result", || task.poll());
        assert_eq!(waker.count(), 1);
        assert!(!task.poll());
        assert_eq!(waker.count(), 1, "polling does not wake anything");
    }

    #[test]
    fn a_task_dropped_before_it_finishes_does_not_take_the_worker_with_it() {
        let waker = WakeCount::new();
        let (gate, wait) = mpsc::channel::<()>();

        let task: Task<u32, ()> = Task::spawn(&Threads, Arc::new(waker.clone()), move || {
            // Held until the test has dropped the task, so the send below is
            // guaranteed to be the unmounted case.
            let _ = wait.recv();
            Ok(1)
        });

        // What unmounting does: the state is dropped, and the receiver with it.
        drop(task);
        let _ = gate.send(());

        until("the worker to finish", || waker.count() == 1);
        // Reaching here at all is the assertion: a send on a dropped receiver
        // returns an error rather than panicking, so the worker exits cleanly.
    }

    #[test]
    fn a_ready_task_needs_no_spawner_and_reports_no_change() {
        let mut task: Task<u32, ()> = Task::ready(3);
        assert_eq!(task.value().ready(), Some(&3));
        assert!(
            !task.poll(),
            "a cache hit is not a change; the first build already showed it"
        );
    }

    #[test]
    fn a_real_thread_is_pending_until_the_work_lands() {
        let waker = WakeCount::new();
        let (release, wait) = mpsc::channel::<()>();
        let mut task: Task<u32, ()> = Task::spawn(&Threads, Arc::new(waker.clone()), move || {
            let _ = wait.recv();
            Ok(9)
        });

        assert!(!task.poll(), "nothing has been sent yet");
        assert!(task.is_pending());

        let _ = release.send(());
        until("the value", || task.poll());
        assert_eq!(task.value().ready(), Some(&9));
    }

    // ── Pool ────────────────────────────────────────────────────────────────

    /// The bound is the whole point: `Threads` would start one thread per
    /// task, and this must not.
    #[test]
    fn a_pool_runs_more_tasks_than_it_has_threads() {
        let pool = Pool::with_threads(2);
        assert_eq!(pool.threads(), 2);

        let done = Arc::new(AtomicUsize::new(0));
        for _ in 0..50 {
            let done = Arc::clone(&done);
            pool.spawn(Box::new(move || {
                done.fetch_add(1, Ordering::SeqCst);
            }));
        }

        // Dropping joins every worker after the queue drains, which is the
        // documented shutdown behaviour and also what makes this assertion
        // deterministic rather than a sleep.
        drop(pool);
        assert_eq!(done.load(Ordering::SeqCst), 50);
    }

    /// `Pool::new` must never produce a pool that accepts work and never runs
    /// it — see its doc on why zero workers is the worst available failure.
    #[test]
    fn a_default_pool_has_at_least_one_worker() {
        assert!(Pool::new().threads() >= 1);
        assert_eq!(
            Pool::with_threads(0).threads(),
            1,
            "zero is clamped, not honoured"
        );
    }

    /// The guarantee that separates this from a naive shared-queue pool: one
    /// task panicking must not retire the worker that ran it, or a pool
    /// degrades to nothing over the life of a process.
    #[test]
    fn a_panicking_task_does_not_kill_its_worker() {
        // One worker, so the task after the panic *must* run on the same
        // thread that panicked. With more, this could pass by luck.
        let pool = Pool::with_threads(1);
        let survived = Arc::new(AtomicUsize::new(0));

        pool.spawn(Box::new(|| panic!("this task fails")));
        for _ in 0..5 {
            let survived = Arc::clone(&survived);
            pool.spawn(Box::new(move || {
                survived.fetch_add(1, Ordering::SeqCst);
            }));
        }

        drop(pool);
        assert_eq!(
            survived.load(Ordering::SeqCst),
            5,
            "every task after the panic ran on the same single worker"
        );
    }

    /// A panicking task still has to reach the element as `Lost`, through the
    /// same `Drop`-guard wake path `Threads` uses — the panic must be visible
    /// on screen, not only in a log.
    #[test]
    fn a_task_that_panics_on_a_pool_reports_lost_and_wakes() {
        let waker = WakeCount::new();
        let pool = Pool::with_threads(1);
        let mut task: Task<u32, ()> =
            Task::spawn(&pool, Arc::new(waker.clone()), || panic!("no value"));

        until("the task to resolve", || task.poll());
        assert!(matches!(task.value(), AsyncValue::Lost));
        assert!(waker.count() >= 1, "the frame must be asked for");
    }

    /// Dropping the pool waits for in-flight work rather than detaching it.
    #[test]
    fn dropping_a_pool_lets_queued_work_finish() {
        let pool = Pool::with_threads(1);
        let finished = Arc::new(AtomicUsize::new(0));
        {
            let finished = Arc::clone(&finished);
            pool.spawn(Box::new(move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                finished.store(1, Ordering::SeqCst);
            }));
        }

        drop(pool);
        assert_eq!(
            finished.load(Ordering::SeqCst),
            1,
            "drop joined the worker instead of detaching it mid-write"
        );
    }

    /// A pool drives the real `Task` path end to end, the same way `Threads`
    /// is checked to.
    #[test]
    fn a_pool_backed_task_delivers_its_value() {
        let waker = WakeCount::new();
        let pool = Pool::with_threads(2);
        let mut task: Task<u32, ()> = Task::spawn(&pool, Arc::new(waker.clone()), || Ok(9));

        until("the value", || task.poll());
        assert_eq!(task.value().ready(), Some(&9));
    }
}
