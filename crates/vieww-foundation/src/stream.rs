//! Work that produces *many* values over time, and how each of them gets back.
//!
//! [`task`](crate::task) is one value, once. This is the other lifecycle: a
//! websocket, a location feed, a download reporting progress, a clock. The seam
//! is deliberately the same one — [`Spawn`] and [`FrameWaker`] are reused
//! unchanged, so an application that has already implemented them for tasks has
//! implemented them for streams.
//!
//! ```
//! use vieww_foundation::stream::{Stream, StreamState};
//! use vieww_foundation::task::{Inline, NoWaker};
//! use std::sync::Arc;
//!
//! let mut stream: Stream<u32, String> =
//!     Stream::spawn(&Inline, Arc::new(NoWaker), |emit| {
//!         emit.emit(1);
//!         emit.emit(2);
//!         Ok(())
//!     });
//!
//! assert!(stream.poll(), "values arrived, so the element must rebuild");
//! assert_eq!(stream.latest(), Some(&2), "the newest one is what to draw");
//! assert_eq!(stream.skipped(), 1, "and 1 was superseded before it was drawn");
//! assert_eq!(*stream.state(), StreamState::Ended);
//! ```
//!
//! # The decision this module is arranged around: a frame is slower than a
//! producer
//!
//! A display updates sixty times a second and a sensor does not care. Something
//! has to decide what happens when a hundred values arrive between two frames,
//! and there are only two honest answers: draw the newest and say how many were
//! skipped, or keep all of them and grow without bound.
//!
//! This keeps **the newest**, because that is what a rendered frame *is* — the
//! current state of the world, not a replay of it — and because the unbounded
//! version has no failure mode short of running out of memory. The count is
//! published rather than swallowed ([`skipped`](Stream::skipped)) so an
//! application that cared can find out that it should have.
//!
//! **An application that needs every value has
//! [`poll_each`](Stream::poll_each)**, which is handed each one in arrival order
//! as it is drained. That is where a chat feed appends to its `Signal<Vec<_>>`.
//! The split is the one [`Task`](crate::task::Task) already draws: status in the
//! state, payload in a signal.

use std::fmt;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

use crate::task::{FrameWaker, Spawn};

/// What crosses the channel.
///
/// One channel rather than one per kind, so that **ordering is free**: a value
/// emitted immediately before the end cannot overtake the end and be dropped as
/// arriving after it.
enum Message<T, E> {
    Value(T),
    Ended,
    Failed(E),
}

/// The worker's end of a [`Stream`]: what it pushes values into.
///
/// Handed to the work rather than returned to the caller, for the same reason
/// [`Task`](crate::task::Task)'s work takes nothing — the UI thread must not be
/// able to hold a sender, because then the stream could never end.
pub struct Emitter<T, E> {
    sender: Sender<Message<T, E>>,
    waker: Arc<dyn FrameWaker>,
}

impl<T, E> fmt::Debug for Emitter<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Emitter").finish_non_exhaustive()
    }
}

impl<T, E> Emitter<T, E> {
    /// Publish a value, and ask for the frame that will show it.
    ///
    /// Returns `false` once nobody is listening — the element was unmounted —
    /// which is the **cancellation signal a loop should check**:
    ///
    /// ```
    /// # use vieww_foundation::stream::Emitter;
    /// # fn producer(emit: &Emitter<u32, ()>) {
    /// for tick in 0..10 {
    ///     if !emit.emit(tick) {
    ///         // Nobody is watching any more. A producer that ignores this keeps
    ///         // working into a closed channel: correct, and a waste of a thread
    ///         // until it happens to finish.
    ///         break;
    ///     }
    /// }
    /// # }
    /// ```
    ///
    /// # Why every emit wakes
    ///
    /// The same reason [`Task`](crate::task::Task) wakes on completion: a value
    /// arriving at an application that is not drawing sits in the channel until
    /// somebody prods the screen. Waking per value looks wasteful next to waking
    /// per frame, and is not — a wake asks for *a* frame, the request is
    /// idempotent until that frame runs, and the frame then drains everything
    /// that arrived in the meantime. A producer faster than the display costs
    /// one redundant request per value and never one extra frame.
    pub fn emit(&self, value: T) -> bool {
        let sent = self.sender.send(Message::Value(value)).is_ok();
        if sent {
            self.waker.wake();
        }
        sent
    }
}

/// How a [`Stream`] is doing, beyond whatever its latest value is.
///
/// The states [`AsyncValue`](crate::task::AsyncValue) has, minus the one a
/// stream cannot be in. There is no `Pending`: a stream with no value yet is
/// [`Live`](Self::Live) with `latest() == None`, because "still going" and
/// "nothing yet" are independent for a stream in a way they never are for a
/// task — a live feed that has already produced ten values is both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState<E> {
    /// Still producing.
    Live,
    /// Finished, deliberately and successfully.
    Ended,
    /// Finished, with an error the work produced deliberately.
    Failed(E),
    /// Ended without saying so — in practice, the work panicked.
    ///
    /// The same fourth state [`AsyncValue::Lost`](crate::task::AsyncValue::Lost)
    /// exists for, reached the same way: the sender dropped without a terminal
    /// message. Leaving it [`Live`](Self::Live) would be a feed that has
    /// silently stopped, which looks exactly like a feed with nothing to say.
    Lost,
}

impl<E> StreamState<E> {
    /// `true` while more values may arrive.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(self, Self::Live)
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
/// [`Task`](crate::task::Task) has the identical guard for the identical reason:
/// a wake placed after the work is skipped by a panic, which leaves the
/// application asleep holding a stream that has already died — making
/// [`StreamState::Lost`] invisible by never drawing the frame that would show
/// it.
struct WakeOnDrop(Arc<dyn FrameWaker>);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        self.0.wake();
    }
}

/// A source of many values, and the newest one to have arrived.
///
/// Held by the `ElementState` of the widget watching it, exactly as
/// [`Task`](crate::task::Task) is, and for the same two mechanisms: the result
/// path is `ElementState::take_pending`, and cancellation is `Drop` dropping the
/// receiver.
pub struct Stream<T, E> {
    latest: Option<T>,
    state: StreamState<E>,
    skipped: u64,
    /// `None` once the stream has finished and the channel is spent.
    receiver: Option<Receiver<Message<T, E>>>,
}

impl<T: fmt::Debug, E: fmt::Debug> fmt::Debug for Stream<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stream")
            .field("latest", &self.latest)
            .field("state", &self.state)
            .field("skipped", &self.skipped)
            .finish()
    }
}

impl<T: Send + 'static, E: Send + 'static> Stream<T, E> {
    /// Start `work` on `spawner`, waking through `waker` for every value.
    ///
    /// The work is handed an [`Emitter`] and returns when it is done: `Ok(())`
    /// for a feed that finished, `Err(e)` for one that broke. Returning is what
    /// ends the stream — there is no separate close.
    ///
    /// # Where to call it
    ///
    /// From `create_state`, never from `build`, for the reason
    /// [`Task::spawn`](crate::task::Task::spawn) gives at length: a build that
    /// spawned would open a second socket on every unrelated rebuild.
    pub fn spawn<W>(spawner: &dyn Spawn, waker: Arc<dyn FrameWaker>, work: W) -> Self
    where
        W: FnOnce(&Emitter<T, E>) -> Result<(), E> + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();

        spawner.spawn(Box::new(move || {
            // Dropped last, so the wake happens on the way out of an unwind as
            // well as a return — see `WakeOnDrop`.
            let _wake = WakeOnDrop(Arc::clone(&waker));

            let emitter = Emitter { sender, waker };
            // The terminal message travels the same channel as the values, so a
            // value emitted just before this cannot be overtaken by it. A failed
            // send means the element was unmounted, which is not an error: it is
            // what cancellation looks like from in here.
            let _ = match work(&emitter) {
                Ok(()) => emitter.sender.send(Message::Ended),
                Err(error) => emitter.sender.send(Message::Failed(error)),
            };
        }));

        Self {
            latest: None,
            state: StreamState::Live,
            skipped: 0,
            receiver: Some(receiver),
        }
    }
}

impl<T, E> Stream<T, E> {
    /// A stream that never starts, already ended.
    ///
    /// For a caller that must produce one and has nothing to run —
    /// `StreamBuilder` mounted a second time, whose `FnOnce` the first mount
    /// took. [`Lost`](StreamState::Lost) rather than [`Live`](StreamState::Live)
    /// because nothing is coming, and a feed that will never produce is worse
    /// disguised as a quiet one.
    #[must_use]
    pub const fn lost() -> Self {
        Self {
            latest: None,
            state: StreamState::Lost,
            skipped: 0,
            receiver: None,
        }
    }

    /// Take delivery of everything that has arrived, keeping the newest value.
    ///
    /// Returns `true` if anything changed, which is exactly
    /// `ElementState::take_pending`'s contract. Cheap on every frame: a `try_recv`
    /// on an empty channel, and nothing at all once the stream has finished.
    pub fn poll(&mut self) -> bool {
        self.poll_each(|_| {})
    }

    /// The same, showing `each` every value in arrival order as it is drained.
    ///
    /// **This is the lossless path.** [`poll`](Self::poll) keeps only the newest
    /// value because that is what a frame draws; a caller that must see all of
    /// them — appending to a list, summing a total — takes them here, where they
    /// arrive in order and none is skipped.
    ///
    /// Called from `take_pending`, which is a lifecycle hook and not a build, so
    /// writing a `Signal` from `each` is allowed and is the intended use. Doing
    /// the same thing from `build` would violate `DESIGN.md` §1.
    pub fn poll_each(&mut self, mut each: impl FnMut(&T)) -> bool {
        let mut arrived = 0_u64;
        let mut changed = false;

        // The borrow taken here ends at `try_recv` below rather than spanning
        // the body, which is what lets the arms write `self`. Kept as its own
        // statement to make that visible, since the arms depending on it is not
        // obvious from reading them.
        //
        // `settle` clears the receiver, so the terminal arms need no `break` of
        // their own — the next turn's condition is the same question.
        while let Some(receiver) = &self.receiver {
            let message = receiver.try_recv();

            match message {
                Ok(Message::Value(value)) => {
                    each(&value);
                    self.latest = Some(value);
                    arrived += 1;
                    changed = true;
                }
                Ok(Message::Ended) => changed |= self.settle(StreamState::Ended),
                Ok(Message::Failed(error)) => changed |= self.settle(StreamState::Failed(error)),
                Err(TryRecvError::Empty) => break,
                // The sender is gone and never said why, so the work ended
                // without finishing. See `StreamState::Lost`.
                Err(TryRecvError::Disconnected) => changed |= self.settle(StreamState::Lost),
            }
        }

        // Everything but the last one was superseded before it could be drawn.
        // Counted per drain rather than per value, because a value that was the
        // newest when its frame ran was *shown* — it is only the ones overtaken
        // inside a single frame that nobody ever saw.
        self.skipped += arrived.saturating_sub(1);
        changed
    }

    /// Record the outcome and close the channel.
    fn settle(&mut self, state: StreamState<E>) -> bool {
        self.state = state;
        self.receiver = None;
        true
    }

    /// The newest value, if one has arrived.
    #[must_use]
    pub const fn latest(&self) -> Option<&T> {
        self.latest.as_ref()
    }

    /// How the stream is doing.
    #[must_use]
    pub const fn state(&self) -> &StreamState<E> {
        &self.state
    }

    /// `true` while more values may arrive.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        self.state.is_live()
    }

    /// How many values were superseded inside a single frame and never drawn.
    ///
    /// Zero for anything slower than the display, which is most things. A number
    /// that climbs says the producer is outrunning the screen — which is not by
    /// itself a fault, and is worth knowing before deciding it is not.
    #[must_use]
    pub const fn skipped(&self) -> u64 {
        self.skipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Inline, NoWaker, Threads, WakeCount};
    use std::time::{Duration, Instant};

    /// Spin until `done`, or fail. Bounded, so a broken stream fails the suite
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

    fn inline<T: Send + 'static, E: Send + 'static>(
        work: impl FnOnce(&Emitter<T, E>) -> Result<(), E> + Send + 'static,
    ) -> Stream<T, E> {
        Stream::spawn(&Inline, Arc::new(NoWaker), work)
    }

    #[test]
    fn many_values_arrive_and_the_newest_is_the_one_to_draw() {
        let mut stream: Stream<u32, ()> = inline(|emit| {
            for value in 1..=3 {
                emit.emit(value);
            }
            Ok(())
        });

        assert!(stream.poll());
        assert_eq!(stream.latest(), Some(&3));
        assert_eq!(*stream.state(), StreamState::Ended);
    }

    #[test]
    fn values_overtaken_inside_one_frame_are_counted_rather_than_hidden() {
        // The decision this module is arranged around, asserted. Three values
        // arrived between two frames; one was drawn and two were not, and an
        // application that cared can find that out.
        let mut stream: Stream<u32, ()> = inline(|emit| {
            for value in 1..=3 {
                emit.emit(value);
            }
            Ok(())
        });

        stream.poll();
        assert_eq!(stream.skipped(), 2);
    }

    #[test]
    fn a_value_drawn_by_its_own_frame_was_not_skipped() {
        // Counted per drain, not per value. A stream slower than the display
        // shows every value it produces, and reporting those as skipped would
        // make the number useless by making it always non-zero.
        let (send, wait) = mpsc::channel::<u32>();
        let mut stream: Stream<u32, ()> = Stream::spawn(&Threads, Arc::new(NoWaker), move |emit| {
            while let Ok(value) = wait.recv() {
                emit.emit(value);
            }
            Ok(())
        });

        for value in 1..=3 {
            let _ = send.send(value);
            until("the value", || stream.poll());
            assert_eq!(stream.latest(), Some(&value));
        }

        assert_eq!(
            stream.skipped(),
            0,
            "each value had a frame of its own, so none was superseded unseen"
        );
    }

    #[test]
    fn poll_each_sees_every_value_including_the_superseded_ones() {
        let mut stream: Stream<u32, ()> = inline(|emit| {
            for value in 1..=4 {
                emit.emit(value);
            }
            Ok(())
        });

        let mut seen = Vec::new();
        assert!(stream.poll_each(|value| seen.push(*value)));

        assert_eq!(
            seen,
            vec![1, 2, 3, 4],
            "in arrival order and complete — this is the path a chat feed \
             appends from, and dropping any of them would be data loss rather \
             than a dropped frame"
        );
        assert_eq!(
            stream.latest(),
            Some(&4),
            "and the newest is still the one to draw"
        );
    }

    #[test]
    fn an_error_ends_the_stream_and_keeps_the_last_good_value() {
        let mut stream: Stream<u32, String> = inline(|emit| {
            emit.emit(7);
            Err(String::from("socket closed"))
        });

        assert!(stream.poll());
        assert_eq!(
            stream.latest(),
            Some(&7),
            "a feed that breaks after ten minutes should not blank the screen \
             it spent ten minutes filling"
        );
        assert_eq!(
            stream.state().failed(),
            Some(&String::from("socket closed"))
        );
        assert!(!stream.is_live());
    }

    #[test]
    fn a_settled_stream_reports_no_further_change() {
        let mut stream: Stream<u32, ()> = inline(|emit| {
            emit.emit(1);
            Ok(())
        });

        assert!(stream.poll());
        assert!(
            !stream.poll(),
            "a state that keeps reporting pending rebuilds its element forever"
        );
    }

    #[test]
    fn a_panicking_producer_ends_as_lost_rather_than_live_forever() {
        let waker = WakeCount::new();
        let mut stream: Stream<u32, ()> =
            Stream::spawn(&Threads, Arc::new(waker.clone()), |emit| {
                emit.emit(1);
                panic!("the feed broke")
            });

        until("the worker to die", || waker.count() > 1);
        until("the stream to notice", || {
            stream.poll();
            !stream.is_live()
        });

        assert_eq!(
            *stream.state(),
            StreamState::Lost,
            "staying Live would be a feed that silently stopped, which looks \
             exactly like a feed with nothing to say"
        );
        assert_eq!(
            stream.latest(),
            Some(&1),
            "and what it did send is still there"
        );
    }

    #[test]
    fn every_value_asks_for_a_frame() {
        let waker = WakeCount::new();
        let _stream: Stream<u32, ()> = Stream::spawn(&Threads, Arc::new(waker.clone()), |emit| {
            for value in 1..=3 {
                emit.emit(value);
            }
            Ok(())
        });

        // Three values and the drop guard. Without the per-value wake a feed
        // arriving at an idle application waits for somebody to touch the screen.
        until("three values and the end", || waker.count() == 4);
    }

    #[test]
    fn a_dropped_stream_tells_its_producer_to_stop() {
        // Cancellation, from the producer's side: the loop asks rather than
        // being told, and `emit` answering false is the whole mechanism.
        let waker = WakeCount::new();
        let (gate, wait) = mpsc::channel::<()>();
        let ran = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&ran);

        let stream: Stream<u32, ()> =
            Stream::spawn(&Threads, Arc::new(waker.clone()), move |emit| {
                // Held until the test has dropped the stream, so the emit below is
                // guaranteed to be the unmounted case.
                let _ = wait.recv();
                for value in 0..1000 {
                    if !emit.emit(value) {
                        break;
                    }
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(())
            });

        drop(stream);
        let _ = gate.send(());
        until("the producer to give up", || waker.count() > 0);

        assert_eq!(
            ran.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the first emit already had nobody to deliver to, so the loop \
             stopped there rather than producing 999 more values nobody wanted"
        );
    }

    #[test]
    fn a_lost_stream_needs_no_spawner_and_reports_no_change() {
        let mut stream: Stream<u32, ()> = Stream::lost();
        assert_eq!(*stream.state(), StreamState::Lost);
        assert!(!stream.poll(), "there is no channel to drain");
    }
}
