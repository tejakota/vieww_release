//! A window nobody is touching, updating itself.
//!
//! ```console
//! # one value a second. Wait five seconds, then click — see "how to check it".
//! cargo run -p vieww-platform-winit --example stream --release
//!
//! # a value every millisecond, which is fifteen or so per frame
//! cargo run -p vieww-platform-winit --example stream --release -- --fast
//!
//! # unattended: exits after 12 frames, or hangs if the wake is broken
//! timeout 40 cargo run -p vieww-platform-winit --example stream --release -- --frames 12
//! ```
//!
//! # What this is for, and it is one claim no test can make
//!
//! `vieww_foundation::task` documents the single part of async that has never
//! been verified anywhere: **a value arriving at a genuinely idle application**.
//! A test's [`FrameWaker`](vieww_foundation::task::FrameWaker) sets a flag,
//! which proves the call happens and nothing whatever about whether a real event
//! loop wakes up — and a desktop under test is never idle enough to show the
//! difference, because a mouse crossing the window produces the frame that hides
//! the bug.
//!
//! Streams make that the *normal* case rather than a one-off, so this screen is
//! arranged to have no other reason to draw. **Nothing here animates.** There is
//! no ticker, no spring, no repeating tween. The only thing that can produce a
//! frame is the stream's waker: worker thread → `Emitter::emit` → `Waker` →
//! `EventLoopProxy` → a frame.
//!
//! # How to check it, in five seconds
//!
//! Watching the number climb is not enough on its own, because moving a mouse
//! over a window produces frames of its own — and those are exactly the frames
//! that make a broken wake look like a working one.
//!
//! **The jump is the signature.** With a broken wake, values pile up in the
//! channel while nothing draws, so the first frame after a pause drains all of
//! them at once. So: leave the pointer still for five seconds, then click once.
//!
//! - The number **jumps by about five** — broken. Nothing was drawing, and your
//!   click is what delivered the backlog.
//! - The number **does not move** — working. It was already current, which it
//!   could only be if the values had been drawing themselves.
//!
//! `--frames N` is the same check without a person: nothing here asks for a
//! frame except a value arriving, so a broken wake cannot reach the count and
//! the process hangs rather than exiting. That is for a device, or for CI, where
//! there is nobody to watch.
//!
//! # The three numbers, and why there are three
//!
//! `--fast` emits far quicker than a display can show, which is the case the
//! whole module is arranged around. The screen states the consequence rather
//! than hiding it:
//!
//! - **received** — every value, counted by
//!   [`StreamBuilder::on_value`](vieww_widget::StreamBuilder::on_value), which
//!   is handed each one in order. This is the lossless path a chat feed would
//!   append from.
//! - **drawn** — the newest value at the moment the frame ran, which is what
//!   `view` gets. A frame *is* the current state of the world, not a replay of
//!   it.
//! - **skipped** — `Stream::skipped`, the ones superseded inside a single frame
//!   and never shown. Published rather than swallowed, so an application that
//!   cared can find out that it should have.
//!
//! Two relationships hold, and the second is the interesting one.
//!
//! **`received == drawn + 1`, always.** The producer emits `0, 1, 2, …`, so the
//! newest value's *number* is one less than the count of them. It holds at one
//! value a second and it holds at a thousand, which is the point: `on_value` is
//! handed every value whatever the frame rate does, so this equation breaking
//! would mean the lossless path had lost something.
//!
//! **`received - skipped` is the number of frames that showed a new value**,
//! because a drain of `n` values counts `n - 1` of them skipped. That makes the
//! screen a frame counter as well as a stream: measured here, 4431 values
//! arrived across 290 frames in five seconds — 58fps, and 15.3 values coalesced
//! into each frame.
//!
//! That ratio *is* the design. The UI thread rebuilt 290 times rather than 4431
//! times for the same feed, and the alternative to skipping 93% of the values
//! was not showing them — a display cannot — but queueing them behind a
//! renderer that falls further behind for as long as the producer runs.
//!
//! At one value a second none of that applies: `skipped` stays 0 and every value
//! gets a frame of its own.

use std::sync::Arc;
use std::time::Duration;

use vieww_element::Signal;
use vieww_foundation::stream::{Stream, StreamState};
use vieww_foundation::task::Threads;
use vieww_foundation::{Color, Size};
use vieww_platform_winit::App;
use vieww_widget::prelude::*;
use vieww_widget::StreamBuilder;

const SURFACE: Size = Size {
    width: 520.0,
    height: 300.0,
};
const LABEL: Color = Color::rgb(150, 150, 165);
const VALUE: Color = Color::rgb(235, 235, 245);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fast = has_flag("--fast");
    let period = if fast {
        Duration::from_millis(1)
    } else {
        Duration::from_secs(1)
    };

    let mut app = App::new()
        .title("vieww — nobody is touching this window")
        .size(SURFACE)
        .background(Color::rgb(18, 18, 22));
    // The whole point of the unattended mode: nothing here asks for a frame
    // except a value arriving, so a broken wake cannot reach the count and the
    // process hangs instead of exiting. See "watching is not a test" above.
    if let Some(frames) = frames_from_args() {
        app = app.exit_after_frames(frames);
    }
    // Taken before `run` consumes the app: this is what a worker thread turns a
    // value into a frame with, and it is the whole point of the example.
    let waker = app.waker();

    let report = app.run(move |driver| {
        // Written from `on_value` and read from `view`. A signal rather than a
        // counter in the state because it is the shape an application would use
        // — a chat feed's `Signal<Vec<Message>>` is this with a body.
        let received = driver.elements().runtime().signal(0_u64);
        let counter = received.clone();

        let root = StreamBuilder::new(
            Arc::new(Threads),
            Arc::new(waker),
            move |emit| {
                let mut tick = 0_u64;
                loop {
                    // `false` means the element was unmounted — the window is
                    // closing. A producer that ignored this would keep counting
                    // into a closed channel until the process died.
                    if !emit.emit(tick) {
                        break;
                    }
                    tick += 1;
                    std::thread::sleep(period);
                }
                Ok::<_, String>(())
            },
            move |stream: &Stream<u64, String>| screen(stream, &received),
        )
        // `peek`, not `get`: this runs inside `take_pending`, where there is no
        // element building and a subscription would belong to nobody.
        .on_value(move |_| counter.set(counter.peek() + 1));

        driver.set_root(root);
    })?;

    println!("{report}");
    Ok(())
}

/// The whole screen: three numbers and what they mean.
fn screen(stream: &Stream<u64, String>, received: &Signal<u64>) -> WidgetNode {
    let drawn = match stream.latest() {
        Some(value) => format!("{value}"),
        None => String::from("—"),
    };
    // The read *is* the subscription, so `on_value`'s write rebuilds this
    // element and nothing else.
    let count = received.get();

    let state = match stream.state() {
        StreamState::Live => String::from("live"),
        StreamState::Ended => String::from("ended"),
        StreamState::Failed(error) => format!("failed: {error}"),
        StreamState::Lost => String::from("lost — the producer panicked"),
    };

    Center::new()
        .child(Flex::column().children(children![
            Text::new(drawn).size(72.0).color(VALUE),
            Text::new(format!("received {count}")).size(16.0).color(LABEL),
            Text::new(format!("skipped {}", stream.skipped()))
                .size(16.0)
                .color(LABEL),
            Text::new(state).size(14.0).color(LABEL),
        ]))
        .into()
}

/// `true` if `flag` was passed.
fn has_flag(flag: &str) -> bool {
    std::env::args().skip(1).any(|arg| arg == flag)
}

/// `--frames N`, if it was passed.
fn frames_from_args() -> Option<u64> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            return args.next().and_then(|count| count.parse().ok());
        }
    }
    None
}
