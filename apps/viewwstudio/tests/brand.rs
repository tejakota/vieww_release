//! The application's mark, rasterised — on the launch splash and in the strip.
//!
//! # Why this is a pixel test rather than a tree test
//!
//! The splash's whole failure mode was invisibility. It was in the tree on
//! every launch — `Shell` mounted it, a dump would have found it — and it drew
//! nothing at all, because it was built once at zero elapsed and never rebuilt.
//! A test that asked the tree whether a `Splash` was present would have passed
//! against the broken version on every one of those launches.
//!
//! So these ask the only questions that can tell the difference: is the window
//! actually covered, does the driver keep asking for the frames the animation
//! needs, and is all of that gone when it is over.
//!
//! The last one is about the same mark at the other end of its size range —
//! eighteen points, at the left of the title bar, where three drawn imitations
//! of macOS window buttons used to sit.

use std::time::Duration;

use vieww_foundation::{Color, Offset, PointerEvent, PointerId, Size};
use vieww_render::FrameDriver;
use viewwstudio::ui::splash::SPLASH_DURATION;
use viewwstudio::{Shell, Studio};

/// The page the splash draws behind its mark, from `ui::splash`.
const BACKDROP: (u8, u8, u8) = (0x0E, 0x10, 0x13);

/// A mounted shell with the splash raised, and the renderer to read it with.
fn launching(window: Size) -> (FrameDriver, Studio, vieww_paint::native::NativeRenderer) {
    let mut driver = FrameDriver::new(window);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.note_window_size(window);
    studio.splash.set(true);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    (driver, studio, vieww_paint::native::NativeRenderer::new())
}

/// One rasterised frame, to read colours out of.
///
/// Rasterising is the expensive part — a 1366×679 window is nearly a million
/// pixels through the CPU backend — so a test that wants a region asks for the
/// frame once and reads it many times. The first version of the title-bar test
/// below called `pixel` per coordinate and took eighty seconds to look at a
/// 60×40 corner.
struct Frame {
    data: Vec<(u8, u8, u8)>,
    width: u32,
}

impl Frame {
    fn at(&self, x: u32, y: u32) -> (u8, u8, u8) {
        self.data[(y * self.width + x) as usize]
    }
}

/// Rasterise what the driver last painted, as the window would show it.
fn frame(
    driver: &FrameDriver,
    cpu: &mut vieww_paint::native::NativeRenderer,
    window: Size,
) -> Frame {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a window size"
    )]
    let (width, height) = (window.width as u32, window.height as u32);
    let (pixels, _) = cpu
        .render_to_pixels(driver.scene(), width, height, Color::hex(0x00_0000))
        .expect("rasterising");
    // `data()` is tightly packed straight-alpha RGBA8, four bytes per pixel.
    // Everything these tests sample is opaque, so straight and premultiplied
    // agree there.
    Frame {
        data: pixels
            .data()
            .chunks_exact(4)
            .map(|c| (c[0], c[1], c[2]))
            .collect(),
        width,
    }
}

/// The colour at `at`, for the tests that want one pixel.
fn pixel(
    driver: &FrameDriver,
    cpu: &mut vieww_paint::native::NativeRenderer,
    window: Size,
    at: (u32, u32),
) -> (u8, u8, u8) {
    frame(driver, cpu, window).at(at.0, at.1)
}

/// Close enough that antialiasing and rounding do not decide the test.
fn near(left: (u8, u8, u8), right: (u8, u8, u8)) -> bool {
    let delta = |a: u8, b: u8| i16::from(a).abs_diff(i16::from(b));
    delta(left.0, right.0) <= 2 && delta(left.1, right.1) <= 2 && delta(left.2, right.2) <= 2
}

/// Whether `found` is a point on the ramp between two stops.
///
/// Each channel inside the span the two stops bracket, with three of slack for
/// the rasteriser's rounding — and a saturation floor, because every grey in
/// the chrome is trivially "between" two colours on all three channels and
/// without it this would pass on a mark that was never drawn.
fn on_ramp(found: (u8, u8, u8), from: (u8, u8, u8), to: (u8, u8, u8)) -> bool {
    let within = |value: u8, a: u8, b: u8| {
        let (low, high) = if a <= b { (a, b) } else { (b, a) };
        value as i16 >= i16::from(low) - 3 && value as i16 <= i16::from(high) + 3
    };
    let channels = [found.0, found.1, found.2];
    let spread = channels.iter().max().unwrap() - channels.iter().min().unwrap();
    spread > 40
        && within(found.0, from.0, to.0)
        && within(found.1, from.1, to.1)
        && within(found.2, from.2, to.2)
}

#[test]
fn the_splash_covers_the_chrome_from_the_first_frame() {
    let window = Size::new(1366.0, 679.0);
    let (mut driver, _studio, mut cpu) = launching(window);

    driver.draw_frame_at(Duration::ZERO);
    assert!(
        near(pixel(&driver, &mut cpu, window, (4, 4)), BACKDROP),
        "the first frame of a launch shows the splash, not a flash of chrome"
    );
}

#[test]
fn it_keeps_asking_for_frames_while_it_plays_and_stops_when_it_lands() {
    let window = Size::new(1366.0, 679.0);
    let (mut driver, _studio, _cpu) = launching(window);

    driver.draw_frame_at(Duration::ZERO);
    assert!(
        driver.is_animating(),
        "an animation nobody schedules frames for is a still picture — which is \
         exactly what the first version of this was"
    );

    driver.draw_frame_at(Duration::from_millis(900));
    assert!(driver.is_animating(), "still playing at 900ms");

    driver.draw_frame_at(SPLASH_DURATION + Duration::from_millis(50));
    assert!(
        !driver.is_animating(),
        "and it stops asking once it is over, so an idle studio idles"
    );
}

#[test]
fn it_uncovers_the_studio_when_it_is_done() {
    let window = Size::new(1366.0, 679.0);
    let (mut driver, _studio, mut cpu) = launching(window);

    driver.draw_frame_at(Duration::ZERO);
    driver.draw_frame_at(SPLASH_DURATION + Duration::from_millis(50));

    assert!(
        !near(pixel(&driver, &mut cpu, window, (4, 4)), BACKDROP),
        "the splash is still on screen after its own duration"
    );
}

#[test]
fn a_click_skips_it() {
    let window = Size::new(1366.0, 679.0);
    let (mut driver, studio, mut cpu) = launching(window);
    driver.draw_frame_at(Duration::ZERO);

    // A press in the middle of the window, which during the splash is the
    // splash and behind it is the editor. The tap has to land on the splash:
    // an overlay that leaks its clicks opens whatever the pointer happened to
    // be over on the way past.
    let at = Offset::new(window.width / 2.0, window.height / 2.0);
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        at,
        Duration::from_millis(200),
    ));
    driver.draw_frame_at(Duration::from_millis(200));
    // One frame past the press timeout, which is when the recogniser reports.
    driver.draw_frame_at(Duration::from_millis(310));

    assert!(!studio.splash.get(), "a click did not skip the splash");
    driver.draw_frame_at(Duration::from_millis(320));
    assert!(
        !near(pixel(&driver, &mut cpu, window, (4, 4)), BACKDROP),
        "it was skipped and still drawn"
    );
}

#[test]
fn it_fits_a_window_far_smaller_than_the_one_it_was_designed_against() {
    // The first version placed a 360-point mark at 180 from the top with a
    // 72-point wordmark below it — 920 points of composition, which overflowed
    // the default window and left the laptop one with nothing but the mark's
    // top half. Everything is a fraction of the window now, so the only real
    // question is whether a small one still gets the whole picture.
    let window = Size::new(560.0, 360.0);
    let (mut driver, _studio, mut cpu) = launching(window);
    // The first frame is what mounts the animation, and the clock starts from
    // *there* rather than from timestamp zero — which is the property that
    // makes the splash independent of how long the studio took to reach its
    // first frame. So: mount, then jump to the middle of the hold.
    driver.draw_frame_at(Duration::ZERO);
    driver.draw_frame_at(Duration::from_millis(1400));

    // The corner is the page, and the middle is the mark: if the composition
    // had been clipped or pushed off, one of the two would be wrong.
    assert!(near(pixel(&driver, &mut cpu, window, (4, 4)), BACKDROP));
    assert!(
        !near(pixel(&driver, &mut cpu, window, (280, 150)), BACKDROP),
        "the mark is missing from the middle of a small window"
    );
}

#[test]
fn the_strip_carries_the_mark_rather_than_three_imitation_window_buttons() {
    // The dots were decoration: `winit` owns the real close, minimise and zoom,
    // and this bar is drawn inside the client area. A picture of controls that
    // are not there is worse than no picture — on Linux and Windows the real
    // buttons are at the *other* end of the window, so the red one was an
    // invitation to click something that could not be clicked.
    let window = Size::new(1366.0, 679.0);
    let mut driver = FrameDriver::new(window);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.note_window_size(window);
    // No splash: this is the strip as it looks for the rest of the session.
    driver.set_root(Shell { studio });
    driver.draw_frame_at(Duration::ZERO);

    let mut cpu = vieww_paint::native::NativeRenderer::new();
    let painted = frame(&driver, &mut cpu, window);
    let corner: Vec<(u8, u8, u8)> = (0..40)
        .flat_map(|y| (0..60).map(move |x| (x, y)))
        .map(|(x, y)| painted.at(x, y))
        .collect();

    // **On the ramp, not equal to either end of it.**
    //
    // The accent panel is a gradient now, and at eighteen points it is about
    // seven pixels tall — so neither stop is necessarily painted exactly, and
    // asking for one within two of `#7E5CE8` failed on a mark that was drawn
    // perfectly. What is true of every pixel of that panel and of nothing else
    // in the corner is that it lies *between* the two stops and is coloured
    // rather than grey.
    assert!(
        corner
            .iter()
            .any(|found| on_ramp(*found, ACCENT, ACCENT_FAR)),
        "the mark's accent panel is not in the corner of the window"
    );
    for dot in [(0xFF, 0x5F, 0x57), (0xFE, 0xBC, 0x2E), (0x28, 0xC8, 0x40)] {
        assert!(
            !corner.iter().any(|found| near(*found, dot)),
            "a traffic light is still drawn at {dot:?}"
        );
    }
}

/// The mark's one piece of colour, from `ui::brand`.
///
/// The panel is a ramp now, not a fill, so this is one end of it and the test
/// that looks for it allows the other — see `near`'s tolerance and
/// `ACCENT_FAR`.
const ACCENT: (u8, u8, u8) = (
    viewwstudio::theme::ACCENT.dark_near.r,
    viewwstudio::theme::ACCENT.dark_near.g,
    viewwstudio::theme::ACCENT.dark_near.b,
);

/// The far end of the accent panel's ramp, which is what the top of the panel
/// is actually painted.
const ACCENT_FAR: (u8, u8, u8) = (
    viewwstudio::theme::ACCENT.dark_far.r,
    viewwstudio::theme::ACCENT.dark_far.g,
    viewwstudio::theme::ACCENT.dark_far.b,
);
