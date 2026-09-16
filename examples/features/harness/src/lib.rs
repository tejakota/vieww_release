//! One entry point for every `examples/features/*` example.
//!
//! ```console
//! cargo run -p feature-rectangle                      # a window
//! VIEWW_SHOT=shots/features cargo run -p feature-rectangle   # a PNG, no display
//! ```
//!
//! # Why this exists
//!
//! Every example builds the same thing — a `FrameDriver` with a root set on it
//! — and [`App::run`](vieww_platform_winit::App::run) hands that driver to a
//! closure. A headless render needs exactly the same closure and a
//! [`FrameDriver`] it made itself, so one function can serve both and the
//! examples do not have to choose. Without this, an example is only ever
//! checked by compiling it, and "it compiles" has never been the same claim as
//! "it draws the thing".
//!
//! # Animated examples
//!
//! A still of an animation at t=0 usually proves nothing: an opacity that
//! starts at zero and a broken animation that stays at zero look alike. So when
//! the tree is still animating after the first frame, this writes a strip of
//! frames sampled across [`VIEWW_SHOT_MS`](self#environment) instead of one,
//! and the frames must differ for the example to have shown anything.
//!
//! # Environment
//!
//! | variable | meaning |
//! |---|---|
//! | `VIEWW_SHOT` | directory to write PNGs into; unset means open a window |
//! | `VIEWW_SHOT_MS` | length of the sampled window for animations (default 2000) |
//! | `VIEWW_SHOT_FRAMES` | how many frames that window is sampled at (default 5) |

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use vieww_foundation::{Color, Size};
use vieww_paint::native::NativeRenderer;
use vieww_platform_winit::App;
use vieww_render::FrameDriver;

mod page;
pub use page::{fits, set_page};

/// The error every example's `main` returns.
pub type Error = Box<dyn std::error::Error>;

/// Something waiting for the frame clock: given the timestamp of the first
/// frame that draws, it starts whatever it captured.
type Start = Box<dyn Fn(Duration)>;

thread_local! {
    /// Animations waiting for a clock. See [`on_first_frame`].
    static STARTS: RefCell<Vec<Start>> = const { RefCell::new(Vec::new()) };
}

/// Start something once the frame clock exists, rather than at time zero.
///
/// # The bug this exists to prevent
///
/// `Animation::forward(Duration::ZERO)` means "this began at timestamp zero",
/// and frame timestamps are measured from when the application started. A
/// window costs a surface, a device and a first shader compile before it can
/// draw, so the first frame can land a second or more into that clock — and a
/// 1.4 s entrance started at zero is then already **over** before anything is
/// on screen. It does not look slow; it looks static, which is why it read as
/// "the animations do not run" rather than as a timing bug.
///
/// Headless there is no such gap, so the queue is drained at zero and the
/// strips are unchanged.
pub fn on_first_frame(start: impl Fn(Duration) + 'static) {
    STARTS.with_borrow_mut(|starts| starts.push(Box::new(start)));
}

fn run_starts(now: Duration) {
    let queued = STARTS.with_borrow_mut(std::mem::take);
    for start in queued {
        start(now);
    }
}

/// A named thing to do to the tree, and how far into the run to photograph it.
///
/// For the examples driven by **input** rather than by time — a scroll
/// position, a value a control was set to — where a still at t=0 is a picture
/// of nothing having happened yet. In a window these are ignored: a window has
/// real input.
///
/// The number is milliseconds on one clock that runs across the whole strip,
/// so a step that changes a target and asks for a shot 300 ms later gets the
/// implicit animation *between* the two, rather than its endpoints. It must not
/// go backwards; equal values mean "no time passed", which is what an example
/// with nothing animating wants.
pub type Steps = Vec<(String, u64, Box<dyn Fn(&mut FrameDriver)>)>;

/// Open a window, or — under `VIEWW_SHOT` — rasterise one PNG per step.
///
/// `build` returns the steps it wants photographed. Returning an empty vec is
/// [`launch`].
///
/// # Errors
///
/// As [`launch`].
pub fn launch_with<F>(title: &str, size: Size, build: F) -> Result<(), Error>
where
    F: FnOnce(&mut FrameDriver) -> Steps,
{
    match std::env::var_os("VIEWW_SHOT") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir)?;
            let name = shot_name();
            let mut driver = FrameDriver::new(size);
            let steps = build(&mut driver);
            run_starts(Duration::ZERO);
            if steps.is_empty() {
                return frames(&dir, &name, title, size, &mut driver);
            }
            let mut renderer = NativeRenderer::new();
            let mut clock = 0.0;
            for (index, (label, at, step)) in steps.iter().enumerate() {
                step(&mut driver);
                #[allow(clippy::cast_precision_loss)]
                let due = *at as f64;
                // Stepped rather than jumped, for the reason in `frames`.
                while clock < due {
                    clock = (clock + 1000.0 / 60.0).min(due);
                    driver.draw_frame_at(Duration::from_secs_f64(clock / 1000.0));
                }
                driver.draw_frame_at(Duration::from_secs_f64(clock / 1000.0));
                let path = dir.join(format!("{name}-{index}-{label}.png"));
                write(&mut renderer, &mut driver, size, &path, title)?;
            }
            Ok(())
        }
        None => window(title, size, |driver| {
            // The steps are dropped: in a window the input is real. What must
            // not be dropped is whatever `build` kept alive inside them, which
            // is why they are held to the end of the closure.
            let _steps = build(driver);
        }),
    }
}

/// The window path both entry points share.
///
/// # The epoch
///
/// `App` stamps its own the moment `run` begins, and frame timestamps are
/// measured from it. Nothing exposes it, so this takes one immediately before
/// `run` — the two are microseconds apart, which is nothing against the
/// hundreds of milliseconds a first frame costs — and hands it to whatever
/// [`on_first_frame`] queued, on the first frame that actually draws.
fn window<F>(title: &str, size: Size, build: F) -> Result<(), Error>
where
    F: FnOnce(&mut FrameDriver),
{
    let epoch = Instant::now();
    let mut started = false;
    App::new()
        .title(title)
        .size(fits(size))
        .before_frame(move |_driver| {
            if !started {
                started = true;
                run_starts(epoch.elapsed());
            }
        })
        .run(build)?;
    Ok(())
}

/// Open a window, or — under `VIEWW_SHOT` — rasterise the same tree to PNG.
///
/// `build` is whatever the example would have passed to `App::run`.
///
/// # Errors
///
/// The platform error from opening the window, or the I/O error from writing
/// the PNGs.
pub fn launch<F>(title: &str, size: Size, build: F) -> Result<(), Error>
where
    F: FnOnce(&mut FrameDriver),
{
    match std::env::var_os("VIEWW_SHOT") {
        Some(dir) => shoot(&PathBuf::from(dir), title, size, build),
        None => window(title, size, build),
    }
}

/// The background a shot is composited onto.
///
/// White rather than transparent: a PNG with an alpha channel and a bug that
/// drops every fill looks like a blank page in one viewer and like a
/// checkerboard in another, and only one of those reads as "broken".
const BASE: Color = Color::WHITE;

fn shoot<F>(dir: &PathBuf, title: &str, size: Size, build: F) -> Result<(), Error>
where
    F: FnOnce(&mut FrameDriver),
{
    std::fs::create_dir_all(dir)?;
    let name = shot_name();
    let mut driver = FrameDriver::new(size);
    build(&mut driver);
    run_starts(Duration::ZERO);
    frames(dir, &name, title, size, &mut driver)
}

/// One PNG for a still tree; a strip of them for one that is still moving.
///
/// # Why the frames are stepped rather than jumped to
///
/// A tween can be sampled at any instant, but a **spring** cannot: it is
/// integrated, so handing it one 500 ms step is a different simulation from
/// sixty 16 ms ones, and usually a visibly wrong one. So time advances at a
/// frame's worth at a time and only the sample points are written — which is
/// also what the window does, and the point of a shot is to be the same
/// picture the window would show.
fn frames(
    dir: &std::path::Path,
    name: &str,
    title: &str,
    size: Size,
    driver: &mut FrameDriver,
) -> Result<(), Error> {
    let mut renderer = NativeRenderer::new();
    driver.draw_frame_at(Duration::ZERO);

    if !driver.is_animating() {
        return write(
            &mut renderer,
            driver,
            size,
            &dir.join(format!("{name}.png")),
            title,
        );
    }

    let span = env_number("VIEWW_SHOT_MS", 2000.0);
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let samples = env_number("VIEWW_SHOT_FRAMES", 5.0).max(2.0) as u32;
    const STEP_MS: f64 = 1000.0 / 60.0;

    let mut shot = 0;
    let mut at = 0.0;
    while shot < samples {
        let due = span * f64::from(shot) / f64::from(samples - 1);
        while at < due {
            at = (at + STEP_MS).min(due);
            driver.draw_frame_at(Duration::from_secs_f64(at / 1000.0));
        }
        driver.draw_frame_at(Duration::from_secs_f64(at / 1000.0));
        write(
            &mut renderer,
            driver,
            size,
            &dir.join(format!("{name}-{shot}.png")),
            title,
        )?;
        shot += 1;
    }
    Ok(())
}

/// The example's own name, taken from the binary rather than from the title.
///
/// The title is prose meant for a title bar (`"00 — rectangle"`), so deriving a
/// file name from it produces em dashes and spaces in paths. The binary is
/// already named after the package.
///
/// # Why the extension is stripped
///
/// `file_stem` rather than `file_name`, and the difference is the whole point of
/// `ci/certify/shot-suite.sh`. On Windows the binary is `feature-rectangle.exe`, so
/// `file_name` produced `feature-rectangle.exe.png` there and
/// `feature-rectangle.png` everywhere else — and a cross-platform comparison
/// keyed on file name would then have reported **every single shot as present on
/// one side only**, on the one platform whose output most needs checking. It
/// would have looked like a broken run rather than like a naming bug, which is
/// the expensive kind of wrong.
fn shot_name() -> String {
    std::env::args()
        .next()
        .and_then(|path| {
            PathBuf::from(path)
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "shot".to_owned())
}

fn env_number(key: &str, fallback: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn write(
    renderer: &mut NativeRenderer,
    driver: &mut FrameDriver,
    size: Size,
    path: &std::path::Path,
    title: &str,
) -> Result<(), Error> {
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let (png, report) =
        renderer.render_to_png(driver.scene(), size.width as u32, size.height as u32, BASE)?;
    std::fs::write(path, png)?;
    // The counters are half the proof: an empty sheet with `0 shapes` is a
    // tree that never built, and an empty sheet with `40 shapes` is a
    // rasteriser or a layout that put everything off-screen. They are
    // different bugs and the picture alone cannot tell them apart.
    println!(
        "{title}: {} -> {} shapes, {} glyph runs ({} glyphs), {} clips, {} layers",
        path.display(),
        report.shapes,
        report.glyph_runs,
        report.glyphs,
        report.clips,
        report.layers,
    );
    Ok(())
}
