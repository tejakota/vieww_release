//! The shell, repainted incrementally, against a full repaint — in both themes.
//!
//! # Why the studio and not a fixture
//!
//! Because the studio is the largest tree the workspace builds, and because the
//! defect class this guards against is one that only shows up at scale: damage
//! that is a fraction short is invisible on a fixture with four boxes in it and
//! accumulates into visible residue on a shell with a chrome set, an editor, a
//! sidebar and a panel all animating at once.
//!
//! It is also where it was reported. "The icons on the left vertical bar are
//! smeared, in dark mode only" is what an under-reported damage region looks
//! like from the outside, and the "dark mode only" half is perception rather
//! than rendering: a faint residue of light ink on `#181818` is a large relative
//! change in luminance, and the same residue of dark ink on `#F8F8F8` is not.
//! So both themes are driven here, and the assertion is on pixels rather than
//! on how they look.
//!
//! See `vieww_test_harness::visual` for the invariant itself.

use vieww_foundation::{Color, Offset, Size};
use vieww_render::FrameDriver;
use vieww_test_harness::assert_partial_repaint_is_complete;
use viewwstudio::{Shell, Studio};

const SURFACE: Size = Size::new(900.0, 640.0);

/// A mounted, settled shell in the requested theme.
fn shell(dark: bool) -> (FrameDriver, Studio) {
    let mut driver = FrameDriver::new(SURFACE);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.note_window_size(SURFACE);
    studio.splash.set(false);
    studio.dark.set(dark);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    driver.draw_frame();
    (driver, studio)
}

/// The background the persistent target starts from.
///
/// Black rather than the theme's own ground, deliberately: an area the shell
/// never draws over shows through as black in both the incremental and the full
/// render, so it cancels — and anywhere it does *not* cancel is a pixel one
/// path drew and the other did not, which is the whole question.
const BASE: Color = Color::BLACK;

/// **Hovering down the activity bar**, which is where it was reported.
///
/// Each button carries an `Animated` selection value and a hover wash, so
/// moving the pointer down the column has several boxes fading at once — the
/// case where a damage region that is a fraction short accumulates fastest.
fn hover_the_activity_bar(dark: bool) {
    let (mut driver, _studio) = shell(dark);

    assert_partial_repaint_is_complete(&mut driver, SURFACE, BASE, 48, |driver, frame| {
        // Down the column and back up, a step at a time, so every button is
        // entered and left.
        let step = frame % 24;
        #[expect(clippy::cast_precision_loss, reason = "a small frame count")]
        let y = 80.0 + (if step < 12 { step } else { 23 - step }) as f32 * 40.0;
        driver.handle_hover(Some(Offset::new(24.0, y)));
        driver.draw_frame_at(std::time::Duration::from_millis(16 * frame as u64));
    });
}

#[test]
fn the_activity_bar_repaints_completely_in_the_dark_theme() {
    hover_the_activity_bar(true);
}

#[test]
fn the_activity_bar_repaints_completely_in_the_light_theme() {
    hover_the_activity_bar(false);
}

/// **Switching views**, which moves the selection rail and rebuilds the whole
/// sidebar under it — a large change beside a small animated one.
#[test]
fn switching_views_repaints_completely() {
    let (mut driver, studio) = shell(true);
    let views = viewwstudio::state::View::ALL;

    assert_partial_repaint_is_complete(&mut driver, SURFACE, BASE, 24, |driver, frame| {
        studio.view.set(views[frame % views.len()]);
        driver.draw_frame_at(std::time::Duration::from_millis(16 * frame as u64));
    });
}

/// **Folding the sidebar**, which resizes a region rather than recolouring one:
/// the pane's old width has to be damaged as well as its new one.
#[test]
fn folding_the_sidebar_repaints_completely() {
    let (mut driver, studio) = shell(true);

    assert_partial_repaint_is_complete(&mut driver, SURFACE, BASE, 16, |driver, frame| {
        studio.sidebar_width.set(if frame % 2 == 0 {
            0.0
        } else {
            viewwstudio::state::OPEN_SIDEBAR
        });
        driver.draw_frame_at(std::time::Duration::from_millis(16 * frame as u64));
    });
}

/// **A settled window asks for nothing.** The other half of the contract: a
/// frame where nothing changed must report no damage, or the whole mechanism is
/// a full repaint with extra steps.
#[test]
fn an_idle_shell_reports_no_damage() {
    let (mut driver, _studio) = shell(true);
    driver.draw_frame();
    driver.draw_frame();
    assert!(
        driver.damage().is_clean(),
        "a settled shell damaged {:?}",
        driver.damage().repaint_regions()
    );
}
