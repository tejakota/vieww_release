//! A widget that redraws every time a feed says something.
//!
//! The application-facing half of [`vieww_foundation::stream`], and the sibling
//! of [`AsyncBuilder`](crate::AsyncBuilder). Everything in that module's
//! reasoning about *where the work starts* applies here unchanged — the producer
//! is spawned from `create_state`, never from `build`, so a rebuild for an
//! unrelated reason cannot open a second socket.
//!
//! # What is different from `AsyncBuilder`, and it is only one thing
//!
//! A task settles once, so its widget rebuilds once. A stream does not settle,
//! so its widget rebuilds **per frame in which values arrived** — not per value,
//! because the frame drains everything waiting and draws the newest. That is the
//! whole difference in behaviour, and it is why `Stream` and `Task` are separate
//! types rather than one type with a flag: the states they can be in are not the
//! same shape, and a `Live` stream holding a value has no equivalent in
//! `AsyncValue`.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use vieww_foundation::stream::{Emitter, Stream};
use vieww_foundation::task::{FrameWaker, Spawn};
use vieww_foundation::Key;

use crate::{BuildContext, ElementState, Widget, WidgetKind, WidgetNode};

/// The producer, boxed. `Send` because it crosses to a worker, and **not**
/// `Sync` — the same argument [`AsyncBuilder`](crate::AsyncBuilder)'s `Work`
/// makes: it is called once, on one thread, and demanding `Sync` would rule out
/// every closure that captures a `Receiver`.
type Produce<T, E> = Box<dyn FnOnce(&Emitter<T, E>) -> Result<(), E> + Send>;

/// What to draw, given whatever the stream has so far.
///
/// Handed the [`Stream`] rather than a value, because there are two independent
/// things to draw from — the latest value and whether the feed is still live —
/// and a live feed that has already produced is both. `&Stream` and not `&mut`,
/// so a view cannot drain the thing it is drawing.
type View<T, E> = Rc<dyn Fn(&Stream<T, E>) -> WidgetNode>;

/// Every value, in order, for a caller that must not miss one.
type OnValue<T> = Rc<dyn Fn(&T)>;

/// Runs a producer off the UI thread and rebuilds as its values arrive.
///
/// ```
/// use std::sync::Arc;
/// use vieww_foundation::stream::{Stream, StreamState};
/// use vieww_foundation::task::{Inline, NoWaker};
/// use vieww_widget::prelude::*;
/// use vieww_widget::StreamBuilder;
///
/// let widget = StreamBuilder::new(
///     Arc::new(Inline),
///     Arc::new(NoWaker),
///     |emit| {
///         emit.emit(String::from("connected"));
///         Ok::<_, String>(())
///     },
///     |stream: &Stream<String, String>| match (stream.latest(), stream.state()) {
///         (Some(line), StreamState::Live) => Text::new(line.clone()).into(),
///         (Some(line), _) => Text::new(format!("{line} (ended)")).into(),
///         (None, StreamState::Live) => Text::new("Connecting…").into(),
///         (None, _) => Text::new("Nothing arrived").into(),
///     },
/// );
/// # let _ = widget;
/// ```
///
/// # It does not restart
///
/// One mount, one producer, exactly as `AsyncBuilder`. A widget whose input
/// changes carries a [`key`](Self::key) derived from that input, so
/// reconciliation replaces the element and the new one opens its own feed.
pub struct StreamBuilder<T, E> {
    /// Taken by the first `create_state`. `RefCell` rather than `Mutex` because
    /// widgets live on the UI thread and nothing here is shared across one.
    produce: RefCell<Option<Produce<T, E>>>,
    spawner: Arc<dyn Spawn>,
    waker: Arc<dyn FrameWaker>,
    view: View<T, E>,
    on_value: Option<OnValue<T>>,
    key: Option<Key>,
}

/// Hand-written: the fields that matter are closures, and none of those is
/// `Debug`.
impl<T, E> fmt::Debug for StreamBuilder<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamBuilder")
            .field("key", &self.key)
            .field("accumulating", &self.on_value.is_some())
            .finish_non_exhaustive()
    }
}

impl<T, E> StreamBuilder<T, E>
where
    T: Send + fmt::Debug + 'static,
    E: Send + fmt::Debug + 'static,
{
    /// Run `produce` on `spawner`, waking through `waker`, and draw with `view`.
    pub fn new<P, V>(
        spawner: Arc<dyn Spawn>,
        waker: Arc<dyn FrameWaker>,
        produce: P,
        view: V,
    ) -> Self
    where
        P: FnOnce(&Emitter<T, E>) -> Result<(), E> + Send + 'static,
        V: Fn(&Stream<T, E>) -> WidgetNode + 'static,
    {
        Self {
            produce: RefCell::new(Some(Box::new(produce))),
            spawner,
            waker,
            view: Rc::new(view),
            on_value: None,
            key: None,
        }
    }

    /// See **every** value, in arrival order, not just the newest.
    ///
    /// `view` draws the latest value because that is what a frame is. This is
    /// the other half: a handler that runs once per value as the frame drains
    /// them, for a feed where each one matters — appending chat messages to a
    /// `Signal<Vec<_>>`, accumulating a total, writing a log.
    ///
    /// It runs from `take_pending`, which is a lifecycle hook rather than a build,
    /// so writing a signal from here is allowed and is the point. Doing the same
    /// from `view` would be a side effect in a build, which `DESIGN.md` §1
    /// forbids and which would fire again on every unrelated rebuild.
    ///
    /// **And the write lands in the frame it was made in.** `poll_states` — the
    /// thing that calls `take_pending` — runs immediately before `rebuild_pending`
    /// in the build phase, so a signal written here marks its readers pending in time
    /// for the rebuild that is about to happen. That is the whole difference
    /// between writing from here and writing from *layout*, which lands after
    /// both and needs `FrameDriver::drive` to ask for another frame to show it.
    #[must_use]
    pub fn on_value<F>(mut self, handler: F) -> Self
    where
        F: Fn(&T) + 'static,
    {
        self.on_value = Some(Rc::new(handler));
        self
    }

    /// Give this widget an explicit identity.
    ///
    /// The way to say "this is now watching something else": a key derived from
    /// whatever the feed depends on means a change to that input replaces the
    /// element, and the replacement opens its own producer.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl<T, E> Widget for StreamBuilder<T, E>
where
    T: Send + fmt::Debug + 'static,
    E: Send + fmt::Debug + 'static,
{
    fn debug_name(&self) -> &'static str {
        "StreamBuilder"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// Opens the feed. Once per mount, which is what makes a rebuild unable to
    /// open a second one.
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        // `FnOnce`, so it can only be taken once. Unreachable twice in a tree,
        // reachable by calling this by hand — and a `Lost` stream is far better
        // than a panic or a feed that will never produce.
        let stream = match self.produce.borrow_mut().take() {
            Some(produce) => Stream::spawn(self.spawner.as_ref(), Arc::clone(&self.waker), produce),
            None => Stream::lost(),
        };
        Some(Box::new(StreamElementState {
            stream,
            on_value: self.on_value.clone(),
        }))
    }

    fn build(&self, context: &BuildContext) -> WidgetNode {
        context
            .state::<StreamElementState<T, E>, _>(|state| (self.view)(&state.stream))
            // No state means this is being built outside a tree — a direct
            // `build` call, or `debug_tree`. A stream that was never started has
            // produced nothing and never will, which is `lost` rather than a
            // connecting spinner that would be a lie.
            .unwrap_or_else(|| (self.view)(&Stream::lost()))
    }
}

impl<T, E> From<StreamBuilder<T, E>> for WidgetNode
where
    T: Send + fmt::Debug + 'static,
    E: Send + fmt::Debug + 'static,
{
    fn from(widget: StreamBuilder<T, E>) -> Self {
        Self::new(widget)
    }
}

/// The watching element's state: one stream, and optionally somebody to tell
/// about every value.
///
/// It owns the `Stream`, and therefore the receiving end of the channel — so
/// unmounting drops this, the channel closes, and the producer's next `emit`
/// answers `false`. That is the whole cancellation mechanism, and it is the same
/// one `AsyncBuilder` uses.
struct StreamElementState<T, E> {
    stream: Stream<T, E>,
    on_value: Option<OnValue<T>>,
}

/// Hand-written, because `ElementState` requires `Debug` and a handler is a
/// closure. Whether one is *installed* is the part a tree dump can use: it is
/// the difference between a widget that may be dropping values and one that
/// cannot be.
impl<T: fmt::Debug, E: fmt::Debug> fmt::Debug for StreamElementState<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamElementState")
            .field("stream", &self.stream)
            .field("accumulating", &self.on_value.is_some())
            .finish()
    }
}

impl<T, E> ElementState for StreamElementState<T, E>
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

    /// The whole result path, and the reuse is the same one `Task` argues for:
    /// a feed's *status* is ephemeral state nobody above the widget wants to
    /// own, exactly like a press highlight, while the data it carries usually is
    /// owned and belongs in a `Signal` — which is what
    /// [`on_value`](StreamBuilder::on_value) is for.
    ///
    /// Returns `true` on any frame in which values arrived, so the element
    /// rebuilds once per frame rather than once per value however fast the
    /// producer runs.
    fn take_pending(&mut self) -> bool {
        match &self.on_value {
            Some(handler) => self.stream.poll_each(|value| handler(value)),
            None => self.stream.poll(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::mpsc;

    use vieww_foundation::stream::StreamState;
    use vieww_foundation::task::{Inline, NoWaker, Threads};

    use crate::Text;

    fn view(stream: &Stream<u32, String>) -> WidgetNode {
        match (stream.latest(), stream.state()) {
            (Some(value), StreamState::Live) => Text::new(format!("live {value}")).into(),
            (Some(value), _) => Text::new(format!("done {value}")).into(),
            (None, StreamState::Live) => Text::new("waiting").into(),
            (None, _) => Text::new("nothing").into(),
        }
    }

    fn builder<P>(spawner: Arc<dyn Spawn>, produce: P) -> StreamBuilder<u32, String>
    where
        P: FnOnce(&Emitter<u32, String>) -> Result<(), String> + Send + 'static,
    {
        StreamBuilder::new(spawner, Arc::new(NoWaker), produce, view)
    }

    /// The state a mount would create, driven by hand — `create_state` is the
    /// spawn point, so calling it *is* mounting as far as the producer knows.
    fn mount(widget: &StreamBuilder<u32, String>) -> Box<dyn ElementState> {
        widget
            .create_state()
            .expect("StreamBuilder always has state")
    }

    fn stream_of(state: &dyn ElementState) -> &Stream<u32, String> {
        &state
            .as_any()
            .downcast_ref::<StreamElementState<u32, String>>()
            .expect("its own state type")
            .stream
    }

    #[test]
    fn a_producer_starts_once_per_mount_rather_than_once_per_build() {
        // The refetch bug, in its streaming form: a build that spawned would
        // open one socket per frame.
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&runs);
        let widget = builder(Arc::new(Inline), move |emit| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            emit.emit(1);
            Ok(())
        });

        let _state = mount(&widget);
        let context = BuildContext::root();
        for _ in 0..10 {
            let _ = widget.build(&context);
        }

        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn one_rebuild_per_frame_however_many_values_arrived_in_it() {
        // **The behavioural difference from `AsyncBuilder`, asserted.** Three
        // values landed between two frames; the element rebuilds once and draws
        // the newest, because that is what a frame is.
        let widget = builder(Arc::new(Inline), |emit| {
            for value in 1..=3 {
                emit.emit(value);
            }
            Ok(())
        });
        let mut state = mount(&widget);

        assert!(state.take_pending(), "values arrived");
        assert!(
            !state.take_pending(),
            "and the queue is drained — a state that stays pending rebuilds its \
             element on every frame forever"
        );

        let stream = stream_of(state.as_ref());
        assert_eq!(stream.latest(), Some(&3));
        assert_eq!(
            stream.skipped(),
            2,
            "two were superseded before a frame ran"
        );
    }

    #[test]
    fn on_value_sees_the_ones_the_view_never_will() {
        // The lossless path. `view` draws value 3; the handler sees 1, 2 and 3,
        // which is what makes a chat feed possible on top of a "latest wins"
        // stream.
        let seen = Rc::new(RefCell::new(Vec::new()));
        let recorder = Rc::clone(&seen);

        let widget = builder(Arc::new(Inline), |emit| {
            for value in 1..=3 {
                emit.emit(value);
            }
            Ok(())
        })
        .on_value(move |value| recorder.borrow_mut().push(*value));

        let mut state = mount(&widget);
        assert!(state.take_pending());

        assert_eq!(*seen.borrow(), vec![1, 2, 3]);
        assert_eq!(stream_of(state.as_ref()).latest(), Some(&3));
    }

    #[test]
    fn the_handler_runs_from_the_lifecycle_and_not_from_a_build() {
        // `on_value` writing a signal is the intended use, and it is only sound
        // because it runs from `take_pending`. A build must stay side-effect free,
        // so building ten times must not call it once.
        let calls = Rc::new(Cell::new(0_usize));
        let counted = Rc::clone(&calls);

        let widget = builder(Arc::new(Inline), |emit| {
            emit.emit(1);
            Ok(())
        })
        .on_value(move |_| counted.set(counted.get() + 1));

        let mut state = mount(&widget);
        let context = BuildContext::root();
        for _ in 0..10 {
            let _ = widget.build(&context);
        }
        assert_eq!(calls.get(), 0, "a build is not a delivery");

        assert!(state.take_pending());
        assert_eq!(calls.get(), 1, "the frame is");
    }

    #[test]
    fn a_feed_that_ends_keeps_what_it_produced() {
        let widget = builder(Arc::new(Inline), |emit| {
            emit.emit(9);
            Err(String::from("socket closed"))
        });
        let mut state = mount(&widget);
        assert!(state.take_pending());

        let stream = stream_of(state.as_ref());
        assert_eq!(stream.latest(), Some(&9));
        assert_eq!(
            stream.state().failed(),
            Some(&String::from("socket closed"))
        );
        assert!(
            !stream.is_live(),
            "a feed that broke after producing should not blank the screen it \
             filled, and should not claim to still be connected either"
        );
    }

    #[test]
    fn unmounting_tells_the_producer_to_stop() {
        let (gate, wait) = mpsc::channel::<()>();
        let (report, stopped) = mpsc::channel::<u32>();

        let widget = builder(Arc::new(Threads), move |emit| {
            // Held until the test has dropped the state, so the emits below are
            // guaranteed to be the unmounted case.
            let _ = wait.recv();
            let mut sent = 0;
            for value in 0..1000 {
                if !emit.emit(value) {
                    break;
                }
                sent += 1;
            }
            let _ = report.send(sent);
            Ok(())
        });

        // What unmounting does: the element tree drops the state.
        let state = mount(&widget);
        drop(state);
        let _ = gate.send(());

        let sent = stopped
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the producer should notice and report");
        assert_eq!(
            sent, 0,
            "the first emit already had nobody to deliver to, so a producer \
             that checks stops there"
        );
    }

    #[test]
    fn a_widget_built_with_no_state_at_all_reports_nothing_rather_than_waiting() {
        let widget = builder(Arc::new(Inline), |emit| {
            emit.emit(1);
            Ok(())
        });
        // `debug_tree` and a direct `build` call both land here. "waiting" would
        // be a spinner for a feed that was never opened.
        let node = widget.build(&BuildContext::root());
        assert!(crate::debug_tree(node).contains("nothing"));
    }
}
