//! Cold start: process entry → the studio's first complete frame.
//!
//! The number a launch film can honestly show is not "how long cargo took" or
//! "how long a warm frame costs" — it is how long a user waits between
//! double-clicking the icon and seeing an interface. That is: construct the
//! frame driver, install the studio, build the whole widget tree for the first
//! time, lay it out, and paint it. Everything up to pixels-ready.
//!
//! ```console
//! cargo run --release -p viewwstudio --example coldstart
//! ```

use std::time::{Duration, Instant};

use vieww_foundation::Size;
use vieww_render::FrameDriver;
use viewwstudio::Studio;

const WINDOW: Size = Size {
    width: 1440.0,
    height: 900.0,
};

fn main() {
    // Taken as early as possible so the measurement includes every allocation,
    // font load and tree construction the first frame depends on.
    let entry = Instant::now();

    let mut driver = FrameDriver::new(WINDOW);
    let t_driver = entry.elapsed();
    viewwstudio::install(&mut driver);
    let t_install = entry.elapsed();
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.note_window_size(WINDOW);
    let installed = entry.elapsed();
    println!(
        "    FrameDriver::new            {:>8.2} ms\n    viewwstudio::install        {:>8.2} ms\n    Studio::new                 {:>8.2} ms",
        t_driver.as_secs_f64() * 1e3,
        (t_install - t_driver).as_secs_f64() * 1e3,
        (installed - t_install).as_secs_f64() * 1e3,
    );

    // The first frame: build the full tree, lay it out, paint it to a scene.
    driver.draw_frame_at(Duration::from_millis(0));
    let first_frame = entry.elapsed();

    // A second frame, to show what the first one paid for that the next does
    // not: caches warm, tree already built, nothing dirty.
    let before_second = Instant::now();
    driver.draw_frame_at(Duration::from_millis(16));
    let second_frame = before_second.elapsed();

    println!("cold start, {WINDOW:?}\n");
    println!(
        "  driver + studio constructed   {:>8.2} ms",
        installed.as_secs_f64() * 1e3
    );
    println!(
        "  first complete frame          {:>8.2} ms   (process entry → scene ready)",
        first_frame.as_secs_f64() * 1e3
    );
    println!(
        "  second frame                  {:>8.2} ms",
        second_frame.as_secs_f64() * 1e3
    );
}
