//! The shell's motion, recorded frame by frame against a supplied clock.
//!
//! ```console
//! cargo run -p viewwstudio --example motion -- out/ [--only=tabs] [--fps=25] [--light]
//! ```
//!
//! # Why a still picture is not enough any more
//!
//! `examples/screenshot` proves what the shell *looks like*; it cannot prove
//! anything about what it does between two looks. Every transition added to
//! this studio — a tab indicator that grows from its middle, an activity rail
//! that slides, a segmented thumb that travels, a button that sinks under a
//! press — is invisible to a single frame, and three of the four were wrong in
//! their first version in ways only a sequence showed: one drew nothing at all,
//! one drew at the wrong size, one snapped instead of moving.
//!
//! So each scene here drives the studio through a change and rasterises every
//! frame of it at a fixed rate. The clock is `draw_frame_at`'s, not the wall's,
//! so a given millisecond always produces the same PNG and the sequence can be
//! encoded to a GIF and looked at, or diffed frame for frame in CI.
//!
//! # The scenes
//!
//! One per transition, named on the command line with `--only=`. Each is a list
//! of moments: at time *t*, do this. Between the moments nothing happens except
//! time passing, which is exactly the thing being photographed.

use std::path::PathBuf;
use std::time::Duration;

use vieww_foundation::{Offset, PointerEvent, PointerId};
use vieww_render::FrameDriver;
use viewwstudio::compile::{Session, Toolchain};
use viewwstudio::state::{PanelTab, Platform, RightTab};
use viewwstudio::{Shell, Studio, View, Workspace, WINDOW};

/// One thing that happens at one moment in a scene.
type Beat = (u64, Box<dyn Fn(&Studio, &mut FrameDriver)>);

/// A recording: what to call it, how long it runs, and what happens when.
struct Scene {
    name: &'static str,
    /// Milliseconds. Every frame between 0 and this is written.
    length: u64,
    beats: Vec<Beat>,
}

fn screens() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/screens"))
}

/// A tap: down and up at the same instant, which is what a click is.
fn tap(driver: &mut FrameDriver, at: Offset, now: u64) {
    let stamp = Duration::from_millis(now);
    driver.handle_pointer(&PointerEvent::down(PointerId(1), at, stamp));
    driver.handle_pointer(&PointerEvent::up(PointerId(1), at, stamp));
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .map_or_else(|| PathBuf::from("motion"), PathBuf::from);
    let rest: Vec<String> = args.collect();
    let light = rest.iter().any(|a| a == "--light");
    let only: Option<&str> = rest.iter().find_map(|a| a.strip_prefix("--only="));
    let fps: u64 = rest
        .iter()
        .find_map(|a| a.strip_prefix("--fps="))
        .and_then(|n| n.parse().ok())
        .unwrap_or(25);
    let step = 1000 / fps.max(1);

    for scene in scenes() {
        if only.is_some_and(|name| name != scene.name) {
            continue;
        }
        record(&scene, &out, step, light);
    }
}

/// Mount a fresh studio and walk one scene, writing a PNG per frame.
///
/// Fresh per scene deliberately: a shell carried from one scene to the next
/// starts its animations from wherever the last one left them, which makes a
/// recording depend on the order the scenes were listed in.
fn record(scene: &Scene, out: &std::path::Path, step: u64, light: bool) {
    let directory = out.join(scene.name);
    std::fs::create_dir_all(&directory).expect("creating the scene's directory");

    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let target = std::env::var_os(viewwstudio::install::TARGET_DIR_ENV).map_or_else(
        || PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug")),
        PathBuf::from,
    );
    let studio = Studio::with_workspace(&runtime, Workspace::open(&screens()))
        .with_toolchain(Toolchain::discover(&target), Session::new(0x5C_2EE0).ok());
    studio.note_window_size(WINDOW);
    studio.dark.set(!light);
    // No splash: it is two and a half seconds of an animation that has its own
    // probe, and it would be the only thing in every scene's first sixty frames.
    studio.splash.set(false);
    driver.set_root(Shell {
        studio: studio.clone(),
    });

    let mut cpu = vieww_paint::native::NativeRenderer::new();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the window is a few hundred points on each side"
    )]
    let (width, height) = (WINDOW.width as u32, WINDOW.height as u32);
    let background = if light {
        viewwstudio::StudioTheme::light().window
    } else {
        viewwstudio::StudioTheme::dark().window
    };

    let mut next = 0;
    let mut frame = 0;
    let mut now = 0;
    while now <= scene.length {
        // Everything scheduled at or before this moment, before the frame that
        // shows it — a beat drawn one frame late is a transition that starts
        // one frame late, which at 25fps is 40ms of the thing being measured.
        while next < scene.beats.len() && scene.beats[next].0 <= now {
            (scene.beats[next].1)(&studio, &mut driver);
            next += 1;
        }
        driver.draw_frame_at(Duration::from_millis(now));
        let (png, _) = cpu
            .render_to_png(driver.scene(), width, height, background)
            .expect("rasterising");
        std::fs::write(directory.join(format!("{frame:04}.png")), png).expect("writing the PNG");
        frame += 1;
        now += step;
    }
    println!("{}: {frame} frames in {}", scene.name, directory.display());
}

/// Where each control is in the window, so a scene can press it.
///
/// Coordinates rather than a lookup, because the point of this file is to drive
/// the studio the way a pointer does — through hit testing, from a position —
/// rather than to call the handler a control would have called and photograph a
/// state nothing on screen ever passed through.
#[expect(
    unreachable_pub,
    reason = "an example's private module; `pub` is what keeps the names readable at the call sites"
)]
mod at {
    use vieww_foundation::Offset;

    /// The activity bar's entries, top of the column downward.
    pub fn activity(index: usize) -> Offset {
        #[expect(
            clippy::cast_precision_loss,
            reason = "the activity bar has single-digit entries"
        )]
        Offset::new(31.0, 71.0 + index as f32 * 40.0)
    }

    /// The editor's tabs, left to right. Widths vary; these are measured off
    /// `examples/screenshot`'s picture of the bundled workspace.
    pub const TABS: [Offset; 4] = [
        Offset::new(385.0, 68.0),
        Offset::new(545.0, 68.0),
        Offset::new(685.0, 68.0),
        Offset::new(820.0, 68.0),
    ];

    /// The preview pane's platform picker.
    pub const PLATFORMS: [Offset; 3] = [
        Offset::new(1016.0, 99.0),
        Offset::new(1081.0, 99.0),
        Offset::new(1145.0, 99.0),
    ];

    /// The Render button.
    pub const RENDER: Offset = Offset::new(1391.0, 99.0);

    /// The bottom panel's tabs.
    pub const PANEL_TABS: [Offset; 3] = [
        Offset::new(375.0, 666.0),
        Offset::new(458.0, 666.0),
        Offset::new(511.0, 666.0),
    ];
}

fn scenes() -> Vec<Scene> {
    vec![
        Scene {
            name: "tabs",
            length: 2400,
            beats: vec![
                (400, Box::new(|_, d| tap(d, at::TABS[2], 400))),
                (1200, Box::new(|_, d| tap(d, at::TABS[0], 1200))),
                (1800, Box::new(|_, d| tap(d, at::TABS[3], 1800))),
            ],
        },
        Scene {
            name: "activity",
            length: 3200,
            beats: vec![
                (400, Box::new(|_, d| tap(d, at::activity(1), 400))),
                (1200, Box::new(|_, d| tap(d, at::activity(4), 1200))),
                (2000, Box::new(|_, d| tap(d, at::activity(3), 2000))),
                (2700, Box::new(|_, d| tap(d, at::activity(0), 2700))),
            ],
        },
        Scene {
            name: "platform",
            length: 2600,
            beats: vec![
                (400, Box::new(|_, d| tap(d, at::PLATFORMS[1], 400))),
                (1200, Box::new(|_, d| tap(d, at::PLATFORMS[2], 1200))),
                (2000, Box::new(|_, d| tap(d, at::PLATFORMS[0], 2000))),
            ],
        },
        Scene {
            name: "render-button",
            length: 2000,
            beats: vec![
                // Hover, hold, press, release, leave. The press wash and the
                // lift are separate animations on separate controllers, and
                // this is the sequence in which they disagree if they are
                // going to.
                (
                    300,
                    Box::new(|_, d| {
                        d.handle_hover(Some(at::RENDER));
                    }),
                ),
                (
                    900,
                    Box::new(|_, d| {
                        d.handle_pointer(&PointerEvent::down(
                            PointerId(1),
                            at::RENDER,
                            Duration::from_millis(900),
                        ));
                    }),
                ),
                (
                    1250,
                    Box::new(|_, d| {
                        d.handle_pointer(&PointerEvent::up(
                            PointerId(1),
                            at::RENDER,
                            Duration::from_millis(1250),
                        ));
                    }),
                ),
                (
                    1600,
                    Box::new(|_, d| {
                        d.handle_hover(None);
                    }),
                ),
            ],
        },
        Scene {
            name: "panel",
            length: 2400,
            beats: vec![
                (400, Box::new(|_, d| tap(d, at::PANEL_TABS[1], 400))),
                (1200, Box::new(|_, d| tap(d, at::PANEL_TABS[2], 1200))),
                (1800, Box::new(|_, d| tap(d, at::PANEL_TABS[0], 1800))),
            ],
        },
        Scene {
            name: "panes",
            length: 3000,
            beats: vec![
                // The two big reveals: the bottom panel and the right pane.
                (300, Box::new(|studio, _| studio.panel_open.set(false))),
                (1100, Box::new(|studio, _| studio.right_open.set(false))),
                (
                    1900,
                    Box::new(|studio, _| {
                        studio.right_open.set(true);
                        studio.right_tab.set(RightTab::Preview);
                    }),
                ),
                (
                    2500,
                    Box::new(|studio, _| {
                        studio.panel_open.set(true);
                        studio.panel_tab.set(PanelTab::Problems);
                    }),
                ),
            ],
        },
        Scene {
            name: "views",
            length: 3400,
            beats: vec![
                (300, Box::new(|studio, _| studio.view.set(View::Toolchain))),
                (1100, Box::new(|studio, _| studio.view.set(View::Learn))),
                (1900, Box::new(|studio, _| studio.view.set(View::Docs))),
                (2600, Box::new(|studio, _| studio.view.set(View::Explorer))),
            ],
        },
        Scene {
            name: "platforms-frame",
            length: 3200,
            beats: vec![
                (
                    300,
                    Box::new(|studio, _| studio.platform.set(Platform::Android)),
                ),
                (
                    1400,
                    Box::new(|studio, _| studio.platform.set(Platform::Desktop)),
                ),
                (
                    2400,
                    Box::new(|studio, _| studio.platform.set(Platform::Ios)),
                ),
            ],
        },
    ]
}
