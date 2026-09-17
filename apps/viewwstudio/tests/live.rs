//! The built-in live demo respects the device it is drawn inside.
//!
//! # The reported defect
//!
//! From a screenshot of the live preview in an iPhone frame: every screen's
//! heading — "Inbox", "Detail", "Settings" — ran underneath the dynamic island.
//!
//! Two causes, one on each side of the boundary:
//!
//! 1. `live::LiveApp` was a plain column against the whole device rectangle. It
//!    never asked for the safe area, so it could not have been inset by it.
//! 2. `ui::preview::device` hands the guest a `ViewMetrics` whose `safe_area` is
//!    **zeroed** unless the "Safe area" toggle is on — deliberately, because for
//!    a screen somebody is iterating on that toggle is a question about their own
//!    layout. The demo is not a screen anybody is iterating on, and it inherited
//!    the answer meant for one.
//!
//! Cause 1 is fixed the same way it always was: the demo wraps its body in
//! `SafeArea` on a background that reaches the physical edges.
//!
//! # Cause 2 was fixed twice, and the second fix is the real one
//!
//! The first fix gave the *demo* the device's real metrics whatever the toggle
//! said, and this file asserted that by pinning "the demo lays out identically
//! with the toggle on and off". It worked, and it was an exemption: a guest
//! screen — the thing an actual user is looking at — still got the zeroed
//! insets while `ui::preview::device` painted the island over it regardless.
//! So **every new project's first render showed its own heading behind a black
//! pill**, which is the same defect this file was opened for, on the screen
//! that matters more.
//!
//! The island is now drawn only when the insets are published, so the toggle
//! means one coherent thing on both paths:
//!
//! * **On** (the default) — the frame has an island and a home bar, the guest
//!   and the demo are both told about them, and neither can land underneath.
//! * **Off** — no island, no home bar, no insets, everything full-bleed. There
//!   is nothing to sit under.
//!
//! # What is asserted
//!
//! That neither state can put a heading under an obstruction: with the toggle
//! on the demo is inset and moves down, with it off there is nothing to be
//! inset from. Pinning "identical in both states" is no longer available — the
//! two states are deliberately different now — so the property is stated
//! directly instead, in the terms the defect was reported in.

use vieww_foundation::{EdgeInsets, Rect, Size, TargetPlatform, ViewMetrics};
use vieww_render::FrameDriver;
use vieww_widget::Inherited;
use viewwstudio::{Shell, Studio};

const WINDOW: Size = Size::new(1440.0, 900.0);
/// An iPhone 15 Pro, which is what the preview opens on.
const DEVICE: Size = Size::new(393.0, 852.0);

/// A workspace on disk with a `live.rs` in it, so the preview has a file to
/// draw. Returned so the caller can edit or delete it.
fn workspace_with_live(name: &str, source: Option<&str>) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "viewwstudio-live-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // Canonical for the same reason as tests/saving.rs: the studio canonicalizes
    // a workspace root, and macOS's temp directory is behind a symlink.
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    std::fs::write(root.join("screen.rs"), "pub fn screen() {}\n").unwrap();
    if let Some(source) = source {
        std::fs::write(viewwstudio::livedoc::path_in(&root), source).unwrap();
    }
    root
}

fn live_shell(insets_toggle: bool) -> (FrameDriver, Studio) {
    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let root = workspace_with_live("shell", Some(viewwstudio::livedoc::TEMPLATE));
    let studio = Studio::with_workspace(&runtime, viewwstudio::Workspace::open(&root))
        .with_host(TargetPlatform::Linux);
    studio.accept_live_preview();
    studio.show_insets.set(insets_toggle);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    driver.draw_frame();
    (driver, studio)
}

/// Where a label the demo publishes ends up, in window coordinates.
fn label_bounds(driver: &FrameDriver, label: &str) -> Option<Rect> {
    driver
        .semantics()
        .nodes()
        .iter()
        .find(|node| node.label.as_deref() == Some(label))
        .map(|node| node.bounds)
}

/// The heading of the demo's first screen, which is the thing that was under the
/// island in the report.
const HEADING: &str = "Inbox";

/// **The reported defect, in the terms it was reported in.**
///
/// With the safe area simulated — which is the default — the frame draws an
/// island across the top of the screen, and the demo's heading must be clear of
/// it. `Platform::Ios` reserves 47 points; the island itself is 36 tall sitting
/// 11 from the top, so anything at or below 47 is safe by construction.
#[test]
fn the_demo_heading_clears_the_island_when_the_island_is_drawn() {
    let (driver, studio) = live_shell(true);
    let heading = label_bounds(&driver, HEADING).expect("the demo publishes its heading");

    // Where the device's screen starts in the window, so the heading's offset
    // can be read in device points rather than window ones.
    let frame_top = label_bounds(&driver, HEADING)
        .map(|_| studio.preview_view_metrics().safe_area.top)
        .expect("metrics");
    assert!(
        frame_top > 0.0,
        "with the toggle on the preview must publish a top inset, or there is \
         nothing for the demo to be inset by"
    );

    let (without, _) = live_shell(false);
    let full_bleed = label_bounds(&without, HEADING).expect("the demo publishes its heading");

    assert!(
        heading.origin().dy > full_bleed.origin().dy,
        "the heading must sit lower when the island is drawn than when it is \
         not — inset {heading:?} against full-bleed {full_bleed:?}"
    );
}

/// And with the simulation off there is no island to clear.
///
/// The pair is what makes the toggle honest: an obstruction drawn over a screen
/// that was told there is none is the defect, and it can be reintroduced either
/// by publishing no insets while still drawing the island *or* by drawing the
/// island while publishing none. This asserts the second half — that the frame
/// stops drawing it — through the widget tree, because a cutout has no
/// semantics node to find.
#[test]
fn no_island_is_drawn_when_the_safe_area_is_not_simulated() {
    let device = ViewMetrics {
        size: DEVICE,
        device_pixel_ratio: 3.0,
        safe_area: EdgeInsets::only(0.0, 47.0, 0.0, 34.0),
        view_insets: EdgeInsets::ZERO,
    };

    assert_eq!(
        viewwstudio::ui::preview::simulated_metrics(device, true).safe_area,
        device.safe_area,
        "on, the device's own insets are published and the frame draws the \
         island that occupies them"
    );
    assert_eq!(
        viewwstudio::ui::preview::simulated_metrics(device, false).safe_area,
        EdgeInsets::ZERO,
        "off, nothing is reserved — and `cutouts` is gated on the same flag, so \
         nothing is drawn over the screen either"
    );
    assert_eq!(
        viewwstudio::ui::preview::simulated_metrics(device, false).size,
        DEVICE,
        "only the safe area changes; the screen is the same screen"
    );
}

/// And the layout they agree on is the inset one.
///
/// Mounted directly against a surface whose metrics this test chooses, rather
/// than inside the shell: in the shell the device frame is scaled and centred in
/// whatever room the pane has, so window coordinates cannot be compared across
/// two devices without re-deriving where the frame landed. Here the numbers are
/// the demo's own.
#[test]
fn the_demo_insets_itself_by_the_safe_area_it_is_given() {
    let under = |safe_area: EdgeInsets| {
        let mut driver = FrameDriver::new(DEVICE);
        viewwstudio::install(&mut driver);
        let runtime = driver.elements().runtime().clone();
        let state = viewwstudio::live::LiveState::new(&runtime);
        driver.set_root(Inherited::new(
            ViewMetrics {
                size: DEVICE,
                device_pixel_ratio: 3.0,
                safe_area,
                view_insets: EdgeInsets::ZERO,
            },
            viewwstudio::live::LiveApp {
                state,
                source: viewwstudio::live::LiveSource::Doc(
                    viewwstudio::livedoc::parse(viewwstudio::livedoc::TEMPLATE).unwrap(),
                ),
                mounts: std::collections::HashMap::new(),
            },
        ));
        driver.draw_frame();
        driver.draw_frame();
        label_bounds(&driver, HEADING).expect("the demo publishes its heading")
    };

    // An iPhone: 47 points of dynamic island at the top, 34 of home indicator.
    let inset = under(EdgeInsets::only(0.0, 47.0, 0.0, 34.0));
    let flat = under(EdgeInsets::ZERO);

    assert!(
        inset.top >= 47.0,
        "the heading starts at {} — inside the 47 points the island covers",
        inset.top
    );
    assert!(
        inset.top - flat.top >= 40.0,
        "the heading barely moved for a 47-point notch: {} to {}",
        flat.top,
        inset.top
    );
}

/// The question that started this: is the preview the user's, or a demo?
#[test]
fn the_preview_draws_the_workspaces_own_live_file() {
    let root = workspace_with_live("mine", None);
    std::fs::write(
        viewwstudio::livedoc::path_in(&root),
        "live! {\n  screen \"Only\" {\n    title \"My own heading\"\n    row \"Mine\" \"\"\n  }\n}\n",
    )
    .unwrap();

    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::with_workspace(&runtime, viewwstudio::Workspace::open(&root))
        .with_host(TargetPlatform::Linux);
    studio.accept_live_preview();
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    driver.draw_frame();

    assert!(
        label_bounds(&driver, "My own heading").is_some(),
        "the frame is showing something other than this workspace's live.rs"
    );
    assert!(
        label_bounds(&driver, "Inbox").is_none(),
        "the built-in demo is still in there somewhere"
    );
}

/// And it follows the file *as it is typed*, which is the whole difference
/// between this and Render.
#[test]
fn editing_the_buffer_changes_the_frame_without_saving_or_compiling() {
    let root = workspace_with_live("typing", Some(viewwstudio::livedoc::TEMPLATE));
    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::with_workspace(&runtime, viewwstudio::Workspace::open(&root))
        .with_host(TargetPlatform::Linux);
    studio.accept_live_preview();
    studio.open_path(viewwstudio::livedoc::path_in(&root));
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    driver.draw_frame();
    assert!(
        label_bounds(&driver, "Inbox").is_some(),
        "the template's heading"
    );

    // Type over the title. No save, no render.
    let mut value = studio.active().expect("live.rs is open").value;
    value.text = value
        .text
        .replace("title \"Inbox\"", "title \"Typed just now\"");
    studio.edit(value);
    driver.draw_frame();
    driver.draw_frame();

    assert!(
        label_bounds(&driver, "Typed just now").is_some(),
        "the frame did not follow the edit"
    );
    assert!(
        !std::fs::read_to_string(viewwstudio::livedoc::path_in(&root))
            .unwrap()
            .contains("Typed just now"),
        "and nothing was written to disk to make it happen"
    );
}

/// No file, no preview — and the studio says so rather than showing a demo.
#[test]
fn a_workspace_without_the_file_is_told_so() {
    let root = workspace_with_live("empty", None);
    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::with_workspace(&runtime, viewwstudio::Workspace::open(&root))
        .with_host(TargetPlatform::Linux);

    assert!(!studio.has_live_file());

    // Asking for the live preview opens the "no live.rs" dialog rather than
    // mounting anything.
    studio.run(viewwstudio::command::Command::LivePreview);
    assert!(studio.live_missing.get(), "the missing-file dialog");
    assert!(!studio.live_preview.get(), "and nothing was mounted");

    // And the offered action writes the file and opens it.
    studio.live_missing.set(false);
    studio.create_live_file();
    assert!(studio.has_live_file(), "live.rs was written");
    assert!(viewwstudio::livedoc::parse(
        &std::fs::read_to_string(viewwstudio::livedoc::path_in(&root)).unwrap()
    )
    .is_ok());

    driver.set_root(Shell {
        studio: studio.clone(),
    });
    studio.run(viewwstudio::command::Command::LivePreview);
    assert!(studio.live_caution.get(), "now it asks the caution instead");
}

/// A broken file shows the line, in the frame, rather than a blank device.
#[test]
fn a_file_that_does_not_parse_says_which_line() {
    let root = workspace_with_live(
        "broken",
        Some("live! {\n  screen \"Home\" {\n    heading \"typo\"\n  }\n}\n"),
    );
    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::with_workspace(&runtime, viewwstudio::Workspace::open(&root))
        .with_host(TargetPlatform::Linux);
    studio.accept_live_preview();
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    driver.draw_frame();

    let labels: Vec<String> = driver
        .semantics()
        .nodes()
        .iter()
        .filter_map(|node| node.label.clone())
        .collect();
    assert!(
        labels.iter().any(|label| label.contains("Line 3")),
        "the frame does not name the line: {labels:?}"
    );
}
