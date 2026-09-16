//! A widget that shows something else while it waits.
//!
//! The application-facing half of the async seam. Everything underneath is
//! [`vieww_foundation::task`]; this is the twenty lines that put it in a tree.
//!
//! # The spawn is in the lifecycle, never in `build`
//!
//! `DESIGN.md` §1 makes `build` side-effect free, and spawning is a side effect:
//! a `build` that spawned would fire again on every rebuild, so a widget that
//! rebuilt for an *unrelated* reason would issue a second request. That is the
//! classic future-builder footgun, where the future is constructed inline in
//! `build` and refetches whenever a parent rebuilds.
//!
//! So the task starts in `create_state`, which runs once. Reloading is done by
//! changing the widget's [`Key`](vieww_foundation::Key): a new key is a new
//! element, and a new element is a new task. Restarting from `widget_updated`
//! would also work and is not implemented.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use vieww_foundation::task::{AsyncValue, FrameWaker, Spawn, Task};
use vieww_foundation::Key;

use crate::{BuildContext, ElementState, Widget, WidgetKind, WidgetNode};

/// The work, boxed. Named because `clippy::type_complexity` is right about the
/// inline version, and because the bounds are the interesting part: `Send`,
/// because it crosses to a worker thread, and **not** `Sync`.
///
/// The first version stored an `Arc<dyn Fn + Send + Sync>`, since `create_state`
/// only has `&self` and cannot move a value out. That works and it demands
/// `Sync` from every caller's closure — for work that is called exactly once, on
/// exactly one thread, which never needed it. It rules out closures capturing a
/// `Receiver`, a `Cell`, or anything else `Send + !Sync`, and it did so
/// immediately: two of the tests below could not be written.
///
/// `FnOnce` in a `RefCell`, taken on mount, asks for exactly what is used.
type Work<T, E> = Box<dyn FnOnce() -> Result<T, E> + Send>;

/// What to draw, given whatever the task has so far.
///
/// `Rc` rather than `Arc`: this one never leaves the UI thread.
type View<T, E> = Rc<dyn Fn(&AsyncValue<T, E>) -> WidgetNode>;

/// Runs work off the UI thread and builds from whatever state it is in.
///
/// The classic future builder, without its worst habit — see "the refetch bug"
/// below.
///
/// ```
/// use std::sync::Arc;
/// use vieww_foundation::task::{AsyncValue, Inline, NoWaker};
/// use vieww_widget::prelude::*;
/// use vieww_widget::AsyncBuilder;
///
/// let widget = AsyncBuilder::new(
///     Arc::new(Inline),
///     Arc::new(NoWaker),
///     || Ok::<_, String>(String::from("loaded")),
///     |value: &AsyncValue<String, String>| match value {
///         AsyncValue::Pending => Text::new("Loading…").into(),
///         AsyncValue::Ready(text) => Text::new(text.clone()).into(),
///         AsyncValue::Failed(error) => Text::new(format!("Failed: {error}")).into(),
///         AsyncValue::Lost => Text::new("Something broke").into(),
///     },
/// );
/// # let _ = widget;
/// ```
///
/// # The refetch bug this does not have
///
/// The usual way to get this wrong is to construct the future inside `build`.
/// Then any rebuild — a parent re-rendering for an unrelated reason, a theme
/// change, a sibling's animation — starts the work again, and a screen that
/// looks fine issues one network request per frame.
///
/// It cannot happen here, because the work is started from `create_state`,
/// which runs once when the element is mounted, and `build` only ever *reads*
/// what the task has. `docs/DESIGN.md` §1 requires a side-effect-free build for
/// its own reasons, and this is what that requirement buys.
///
/// # It does not restart
///
/// One mount, one run. A widget whose *input* changes — a detail screen moving
/// to a different id — should carry a [`key`](Self::key) derived from that
/// input, so reconciliation replaces the element and the new one starts its own
/// task. Restarting in place, through `ElementState::widget_updated`, is the
/// obvious extension and is deliberately not here yet: it needs a rule for what
/// counts as "the input changed", and a key is that rule already.
pub struct AsyncBuilder<T, E> {
    /// Taken by the first `create_state`. `RefCell` rather than `Mutex` because
    /// widgets live on the UI thread and nothing here is shared across one.
    work: RefCell<Option<Work<T, E>>>,
    spawner: Arc<dyn Spawn>,
    waker: Arc<dyn FrameWaker>,
    view: View<T, E>,
    key: Option<Key>,
}

/// Hand-written: four of the five fields are closures, and none of those is
/// `Debug`. The key is the only part a tree dump can usefully show.
impl<T, E> fmt::Debug for AsyncBuilder<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsyncBuilder")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl<T, E> AsyncBuilder<T, E>
where
    T: Send + fmt::Debug + 'static,
    E: Send + fmt::Debug + 'static,
{
    /// Run `work` on `spawner`, waking through `waker`, and draw with `view`.
    ///
    /// `waker` is what turns a finished task into a frame. On a real
    /// application that is `App::waker`; a headless harness driving its own
    /// frames uses `NoWaker`.
    pub fn new<W, V>(spawner: Arc<dyn Spawn>, waker: Arc<dyn FrameWaker>, work: W, view: V) -> Self
    where
        W: FnOnce() -> Result<T, E> + Send + 'static,
        V: Fn(&AsyncValue<T, E>) -> WidgetNode + 'static,
    {
        Self {
            work: RefCell::new(Some(Box::new(work))),
            spawner,
            waker,
            view: Rc::new(view),
            key: None,
        }
    }

    /// Give this widget an explicit identity.
    ///
    /// The way to say "this is now loading something else": a key derived from
    /// whatever the work depends on means a change to that input replaces the
    /// element, and the replacement starts its own task. See "it does not
    /// restart" above.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl<T, E> Widget for AsyncBuilder<T, E>
where
    T: Send + fmt::Debug + 'static,
    E: Send + fmt::Debug + 'static,
{
    fn debug_name(&self) -> &'static str {
        "AsyncBuilder"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// Starts the work. Called once per mount, which is what makes the refetch
    /// bug structurally impossible rather than merely avoided.
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        // `FnOnce`, so it can only be taken once. A widget instance is built
        // fresh on every rebuild and mounted at most once, so the `None` arm is
        // unreachable in a tree — but it is reachable by calling `create_state`
        // twice by hand, and reporting `Lost` is a great deal better than
        // panicking or silently showing a spinner that will never resolve.
        let task = match self.work.borrow_mut().take() {
            Some(work) => Task::spawn(self.spawner.as_ref(), Arc::clone(&self.waker), work),
            None => Task::lost(),
        };
        Some(Box::new(AsyncState { task }))
    }

    fn build(&self, context: &BuildContext) -> WidgetNode {
        context
            .state::<AsyncState<T, E>, _>(|state| (self.view)(state.task.value()))
            // No state means this is being built outside a tree — a test
            // calling `build` directly, or `debug_tree`. Pending is the honest
            // answer: nothing has been started, so nothing has finished.
            .unwrap_or_else(|| (self.view)(&AsyncValue::Pending))
    }
}

impl<T, E> From<AsyncBuilder<T, E>> for WidgetNode
where
    T: Send + fmt::Debug + 'static,
    E: Send + fmt::Debug + 'static,
{
    fn from(widget: AsyncBuilder<T, E>) -> Self {
        Self::new(widget)
    }
}

/// The waiting element's state: one task, and nothing else.
///
/// It owns the `Task`, and therefore the receiving end of the channel. That is
/// the cancellation mechanism — unmounting drops this, which drops the
/// receiver, which makes the worker's send fail so it discards its result and
/// exits.
///
/// No new hook, and nothing an implementor can forget: disposal already drops
/// state and its subscriptions. It stops the *delivery*, not the work — see
/// [`Task`] for why claiming otherwise would be the API lying.
#[derive(Debug)]
struct AsyncState<T, E> {
    task: Task<T, E>,
}

impl<T, E> ElementState for AsyncState<T, E>
where
    T: Send + fmt::Debug + 'static,
    E: Send + fmt::Debug + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    /// The whole result path.
    ///
    /// `take_pending` is polled every frame and means "something wrote this state
    /// since you last asked"; `Task::poll` returns `true` exactly once, on the
    /// frame the value lands. So the element rebuilds then and not before, and
    /// no other element rebuilds at all.
    ///
    /// This reuse is deliberate rather than convenient, and the argument is
    /// worth restating because `take_pending`'s own documentation says it is for
    /// *ephemeral view state a gesture writes, nothing else*. The distinction it
    /// draws is durable versus ephemeral **ownership**: a request's
    /// pending/ready/failed status is nobody's to own, exactly like a press
    /// highlight. The data that comes back usually *is* owned, and belongs in a
    /// `Signal` — which is why `view` is handed the value rather than this
    /// widget writing one.
    fn take_pending(&mut self) -> bool {
        self.task.poll()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::task::{Inline, NoWaker, Threads, WakeCount};

    use crate::Text;

    fn view(value: &AsyncValue<u32, String>) -> WidgetNode {
        match value {
            AsyncValue::Pending => Text::new("pending").into(),
            AsyncValue::Ready(number) => Text::new(format!("ready {number}")).into(),
            AsyncValue::Failed(error) => Text::new(format!("failed {error}")).into(),
            AsyncValue::Lost => Text::new("lost").into(),
        }
    }

    fn builder<W>(
        spawner: Arc<dyn Spawn>,
        waker: Arc<dyn FrameWaker>,
        work: W,
    ) -> AsyncBuilder<u32, String>
    where
        W: FnOnce() -> Result<u32, String> + Send + 'static,
    {
        AsyncBuilder::new(spawner, waker, work, view)
    }

    /// The state a mount would create, driven by hand — `create_state` is the
    /// spawn point, so calling it *is* mounting as far as the task is concerned.
    fn mount(widget: &AsyncBuilder<u32, String>) -> Box<dyn ElementState> {
        widget
            .create_state()
            .expect("AsyncBuilder always has state")
    }

    #[test]
    fn a_task_starts_once_per_mount_rather_than_once_per_build() {
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let widget = builder(Arc::new(Inline), Arc::new(NoWaker), move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(1)
        });

        let mut state = mount(&widget);
        let context = BuildContext::root();
        for _ in 0..10 {
            let _ = widget.build(&context);
        }

        assert_eq!(
            runs.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "constructing the work inside build is the refetch bug: ten \
             rebuilds would be ten network requests"
        );
        assert!(state.take_pending(), "the inline result is waiting");
    }

    #[test]
    fn the_first_build_shows_pending_and_a_later_one_shows_the_value() {
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let widget = builder(Arc::new(Threads), Arc::new(NoWaker), move || {
            let _ = wait.recv();
            Ok(42)
        });
        let mut state = mount(&widget);

        assert!(!state.take_pending(), "nothing has arrived");

        let _ = release.send(());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.take_pending() {
            assert!(std::time::Instant::now() < deadline, "the value never came");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let ready = state
            .as_any()
            .downcast_ref::<AsyncState<u32, String>>()
            .expect("its own state type");
        assert_eq!(ready.task.value().ready(), Some(&42));
    }

    #[test]
    fn a_settled_task_stops_reporting_pending() {
        let widget = builder(Arc::new(Inline), Arc::new(NoWaker), || Ok(1));
        let mut state = mount(&widget);

        assert!(state.take_pending());
        assert!(
            !state.take_pending(),
            "a state that stays pending rebuilds its element on every frame \
             forever, which looks exactly like a runaway animation"
        );
    }

    #[test]
    fn an_error_reaches_the_view_rather_than_being_swallowed() {
        let widget = builder(Arc::new(Inline), Arc::new(NoWaker), || {
            Err(String::from("offline"))
        });
        let mut state = mount(&widget);
        assert!(state.take_pending());

        let settled = state
            .as_any()
            .downcast_ref::<AsyncState<u32, String>>()
            .expect("its own state type");
        assert_eq!(
            settled.task.value().failed(),
            Some(&String::from("offline"))
        );
    }

    #[test]
    fn unmounting_before_the_work_finishes_cancels_the_delivery() {
        let waker = WakeCount::new();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let widget = builder(Arc::new(Threads), Arc::new(waker.clone()), move || {
            let _ = wait.recv();
            Ok(1)
        });

        // What unmounting does: the element tree drops the state.
        let state = mount(&widget);
        drop(state);
        let _ = release.send(());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while waker.count() == 0 {
            assert!(std::time::Instant::now() < deadline, "the worker never ran");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // Reaching here is the assertion: the worker sent into a dropped
        // receiver and exited instead of panicking.
    }

    #[test]
    fn a_widget_built_with_no_state_at_all_reports_pending() {
        let widget = builder(Arc::new(Inline), Arc::new(NoWaker), || Ok(1));
        // `debug_tree` and a direct `build` call both land here.
        let node = widget.build(&BuildContext::root());
        assert!(crate::debug_tree(node).contains("pending"));
    }
}
