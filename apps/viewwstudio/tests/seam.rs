//! The split between two panes says it is a split.
//!
//! # The reported defect
//!
//! "There is a minimizer functionality between panes, but the minimizer
//! icon/pointer is not showing up. The user will get confused."
//!
//! Both halves were true. The seam is the six-point gutter between two cards:
//! it resizes them, and down to `MIN_PANE` or `MIN_SIDEBAR` it is how a pane is
//! made small. It painted nothing at rest — correct, the ground behind it is the
//! separator — and it also painted nothing while the pointer was on it, and the
//! window showed a plain arrow over it, because nothing in the workspace could
//! ask for a cursor at all. `Cursor::ResizeColumn` existed, and the winit
//! backend had always passed the shape through; `RenderEditableText`'s I-beam
//! was the only object that ever answered.
//!
//! So there were two things to add, and this file asserts both:
//!
//! 1. the pointer takes the resize shape over the seam, and only there;
//! 2. a grip is drawn while the pointer is on it, and not before.
//!
//! And a third, which is what makes the first one honest: **the band where the
//! pointer promises a resize is the band where a drag actually starts**. The
//! divider asks `GestureDetector` for a twelve-point touch target while drawing
//! six, which reads like a three-point margin on each side where the shape and
//! the gesture would disagree — but hit expansion is a fallback for a point that
//! hit nothing, and both neighbouring cards take the tight pass across their
//! whole width. Driven through real pointer events, the drag starts across the
//! six drawn points and nowhere else. That is a coincidence of two mechanisms
//! and exactly the kind of thing that stops being true quietly, so it is
//! measured here rather than reasoned about in a comment.

use std::time::Duration;

use vieww_foundation::{Cursor, Offset, PointerEvent, PointerId, Size, TargetPlatform};
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

const WINDOW: Size = Size::new(1440.0, 900.0);
/// Down the middle of the window, well away from the tab strip and the status
/// bar, so a band found here is the seam rather than a control near it.
const MIDDLE: f32 = 400.0;

fn shell() -> (FrameDriver, Studio) {
    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime).with_host(TargetPlatform::Linux);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    (driver, studio)
}

/// Contiguous x ranges at `y` where the pointer takes the resize shape.
fn resize_bands(driver: &FrameDriver, y: f32) -> Vec<(u32, u32)> {
    let mut bands: Vec<(u32, u32)> = Vec::new();
    for x in 0..(WINDOW.width as u32) {
        if driver.cursor_at(Offset::new(x as f32, y)) != Cursor::ResizeColumn {
            continue;
        }
        match bands.last_mut() {
            Some(last) if last.1 + 1 == x => last.1 = x,
            _ => bands.push((x, x)),
        }
    }
    bands
}

#[test]
fn the_pointer_takes_the_resize_shape_over_a_seam_and_nowhere_else() {
    let (driver, _) = shell();
    let bands = resize_bands(&driver, MIDDLE);

    assert_eq!(
        bands.len(),
        2,
        "one seam beside the sidebar and one beside the preview: {bands:?}"
    );
    for (from, to) in &bands {
        let width = to - from + 1;
        assert!(
            width >= 4,
            "a seam {width} points wide is a shape nobody can land on: {bands:?}"
        );
    }

    // The middle of the code pane is not a seam. (It is a text field, so the
    // shape there is the I-beam — what matters is that it is not a resize.)
    let middle_of_editor = Offset::new(600.0, MIDDLE);
    assert_ne!(
        driver.cursor_at(middle_of_editor),
        Cursor::ResizeColumn,
        "the resize shape leaked outside the gutter"
    );
}

/// The grip is drawn under the pointer and only then.
///
/// Measured off the scene rather than off a signal: a flag that says "hovered"
/// while nothing is painted is exactly the defect that was reported.
#[test]
fn the_grip_is_drawn_while_the_pointer_is_on_the_seam() {
    let (mut driver, studio) = shell();
    let bands = resize_bands(&driver, MIDDLE);
    let (from, to) = bands[0];
    let centre = Offset::new(f32::midpoint(from as f32, to as f32), MIDDLE);

    // Commands rather than `Scene::fills`: the grip has a radius, so it is
    // recorded as a filled *path* and `fills()` — which is rectangles only —
    // cannot see it. A measurement that cannot see the thing it is measuring
    // reads exactly like the defect.
    let in_the_gutter = |driver: &FrameDriver| {
        driver
            .scene()
            .commands()
            .iter()
            .filter(|command| {
                let paint = match command {
                    vieww_paint::Command::FillRect { paint, .. }
                    | vieww_paint::Command::FillPath { paint, .. } => *paint,
                    _ => return false,
                };
                let rect = command.bounds();
                rect.left >= from as f32 - 1.0
                    && rect.right <= to as f32 + 2.0
                    && rect.height() > 8.0
                    && paint.color.a > 0
            })
            .count()
    };

    assert_eq!(
        in_the_gutter(&driver),
        0,
        "the seam painted something before anybody pointed at it — the gutter \
         is the separator at rest, and a line drawn on it is a second one"
    );

    driver.handle_hover(Some(centre));
    driver.draw_frame_at(Duration::from_millis(16));

    assert!(studio.sidebar_seam_hovered.get(), "the seam noticed");
    assert_eq!(
        in_the_gutter(&driver),
        1,
        "no grip appeared under the pointer"
    );

    // And it goes away again.
    driver.handle_hover(Some(Offset::new(600.0, MIDDLE)));
    driver.draw_frame_at(Duration::from_millis(32));
    assert_eq!(
        in_the_gutter(&driver),
        0,
        "the grip stayed behind after the pointer left"
    );
}

/// A press, a drag of `by` points to the right, and a release — the sequence a
/// mouse actually produces, including the frame past the press timeout that the
/// gesture arena needs before it will call anything a drag.
fn drag(driver: &mut FrameDriver, from: Offset, by: f32, clock: u64) {
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        from,
        Duration::from_millis(clock),
    ));
    driver.draw_frame_at(Duration::from_millis(clock));
    driver.draw_frame_at(Duration::from_millis(clock + 110));

    let mut previous = from;
    for step in 1u8..=6 {
        let to = Offset::new(from.dx + f32::from(step) * by / 6.0, from.dy);
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            to,
            Duration::from_millis(clock + 110 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(clock + 110 + u64::from(step)));
        previous = to;
    }

    driver.handle_pointer(&PointerEvent::up(
        PointerId(1),
        previous,
        Duration::from_millis(clock + 130),
    ));
    driver.draw_frame_at(Duration::from_millis(clock + 130));
}

/// What the pointer promises is what a drag delivers.
#[test]
fn a_drag_starts_exactly_where_the_resize_shape_is_offered() {
    let (mut driver, studio) = shell();
    let bands = resize_bands(&driver, MIDDLE);
    let (from, to) = bands[0];

    // Six points either side of the seam, so both the shape's edges and the
    // touch target's unused reach are inside the sweep.
    let mut dragged: Vec<u32> = Vec::new();
    let mut clock = 0;
    for x in (from - 6)..=(to + 6) {
        let before = studio.sidebar_width.get();
        clock += 500;
        drag(&mut driver, Offset::new(x as f32, MIDDLE), 30.0, clock);
        if (studio.sidebar_width.get() - before).abs() > 0.5 {
            dragged.push(x);
        }
        // Put the pane back, so each probe starts from the same place and a
        // pane that has walked to its clamp cannot make later probes look dead.
        studio.sidebar_width.set(before);
        driver.draw_frame_at(Duration::from_millis(clock + 200));
    }

    let promised: Vec<u32> = (from..=to).collect();
    assert_eq!(
        dragged, promised,
        "the resize cursor is offered across {promised:?} but a drag starts \
         across {dragged:?} — one of them is lying to the user"
    );
}

/// The bottom panel got a seam of its own.
///
/// Reported: "no pane dragger/hider for the bottom terminal/output window pane,
/// similar to which we have for editor and renderer." The panel was a fixed 176
/// points; the tab strip could collapse it and bring it back, and that was all.
#[test]
fn the_bottom_panel_has_a_seam_that_resizes_it() {
    let (mut driver, studio) = shell();

    // Where the row-resize shape is offered, down the middle of the window.
    let x = 640.0;
    let mut bands: Vec<(u32, u32)> = Vec::new();
    for y in 0..(WINDOW.height as u32) {
        if driver.cursor_at(Offset::new(x, y as f32)) != Cursor::ResizeRow {
            continue;
        }
        match bands.last_mut() {
            Some(last) if last.1 + 1 == y => last.1 = y,
            _ => bands.push((y, y)),
        }
    }
    assert_eq!(
        bands.len(),
        1,
        "one seam, above the panel and below the panes: {bands:?}"
    );

    let (from, to) = bands[0];
    let centre = Offset::new(x, f32::midpoint(from as f32, to as f32));
    let before = studio.panel_height.get();

    // Up makes it taller — the panel is the pane below the seam.
    drag_by(&mut driver, centre, Offset::new(0.0, -60.0), 1000);
    let taller = studio.panel_height.get();
    assert!(
        taller > before,
        "dragging the seam up did not make the panel taller: {before} -> {taller}"
    );

    // And down makes it shorter again.
    let seam_now = (0..(WINDOW.height as u32))
        .find(|y| driver.cursor_at(Offset::new(x, *y as f32)) == Cursor::ResizeRow)
        .expect("the seam moved with the panel and is still there");
    drag_by(
        &mut driver,
        Offset::new(x, seam_now as f32 + 3.0),
        Offset::new(0.0, 40.0),
        3000,
    );
    assert!(
        studio.panel_height.get() < taller,
        "dragging down did not give the space back"
    );
}

/// Collapsed, there is no seam: the panel is its tab strip, there is no height
/// to drag, and a resize cursor over a strip that cannot resize is a promise
/// nothing keeps. The strip's own click is the hider, and it already worked.
#[test]
fn a_collapsed_panel_offers_no_seam() {
    let (mut driver, studio) = shell();
    studio.panel_open.set(false);
    driver.draw_frame_at(Duration::from_millis(16));

    let x = 640.0;
    let any = (0..(WINDOW.height as u32))
        .any(|y| driver.cursor_at(Offset::new(x, y as f32)) == Cursor::ResizeRow);
    assert!(!any, "a collapsed panel still offered a row resize");
}

/// A drag of `by` from `from`, in the shape the gesture arena accepts.
fn drag_by(driver: &mut FrameDriver, from: Offset, by: Offset, clock: u64) {
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        from,
        Duration::from_millis(clock),
    ));
    driver.draw_frame_at(Duration::from_millis(clock));
    driver.draw_frame_at(Duration::from_millis(clock + 110));
    let mut previous = from;
    for step in 1u8..=8 {
        let to = Offset::new(
            from.dx + by.dx * f32::from(step) / 8.0,
            from.dy + by.dy * f32::from(step) / 8.0,
        );
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            to,
            Duration::from_millis(clock + 110 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(clock + 110 + u64::from(step)));
        previous = to;
    }
    driver.handle_pointer(&PointerEvent::up(
        PointerId(1),
        previous,
        Duration::from_millis(clock + 140),
    ));
    driver.draw_frame_at(Duration::from_millis(clock + 140));
}

// ----- what the pane floor owes the toolbar ---------------------------------
//
// Reported from a screenshot of a session that dragged the preview seam as far
// right as it goes: the pane stopped at its floor, the toolbar did not — its
// fixed-width row kept laying out at natural size, and the Render button, last
// in the row and the one control that starts a compile, slid under the pane's
// rounded clip until only "Ren" of it was on screen. It read as "the render
// pane is not minimizing, the render button is going behind the window".
//
// The bar reflows onto two rows below `ui::preview::COMPACT_BAR_BELOW` so that
// cannot happen. This pins the property the reflow exists to keep, at the
// floor and at the default width both: every control in the preview toolbar is
// *fully* inside the preview card — no part of any control is clipped away.

/// The toolbar texts that must be on screen, whole, whatever the pane width.
const TOOLBAR: [&str; 5] = [
    "\"iOS\"",
    "\"Safe area\"",
    "\"Dark\"",
    "\"Live\"",
    "\"Render\"",
];

/// The vertical band the preview toolbar occupies: below the title bar's
/// menus, above the stage's empty-state prose.
///
/// The band exists because two of the toolbar's words have exact-text
/// neighbours elsewhere in the shell: the title bar has a *Render* menu item at
/// the top of the window, and the stage says "Press Render to compile…"
/// further down. Neither is the control, and a test that cannot tell them
/// apart counts six and passes vacuously against whichever "Render" it met
/// first.
const TOOLBAR_BAND: std::ops::Range<f32> = 50.0..200.0;

/// The toolbar's controls, with whether each is wholly inside every box that
/// clips it.
///
/// The centre-only version of "inside" passes while a button is halfway under
/// the pane's clip — which is exactly the reported defect — so the whole box
/// is what is checked.
fn toolbar_controls(driver: &FrameDriver) -> Vec<(String, Offset, Size, bool)> {
    let tree = driver.renders();
    let Some(root) = tree.root() else {
        return Vec::new();
    };
    let mut ancestry: Vec<vieww_render::RenderId> = Vec::new();
    let mut out = Vec::new();
    for (depth, node) in tree.describe_subtree(root, 20_000) {
        ancestry.truncate(depth);
        ancestry.push(node.id);
        let Some((_, text)) = node.properties.iter().find(|(key, _)| *key == "text") else {
            continue;
        };
        if !TOOLBAR.contains(&text.as_str()) {
            continue;
        }
        let at = tree.global_offset(node.id);
        if !TOOLBAR_BAND.contains(&at.dy) {
            continue;
        }
        let (left, right) = (at.dx, at.dx + node.size.width);
        let inside = ancestry.iter().all(|id| {
            tree.describe(*id).is_some_and(|parent| {
                let at = tree.global_offset(*id);
                left >= at.dx - 0.5 && right <= at.dx + parent.size.width + 0.5
            })
        });
        out.push((text.clone(), at, node.size, inside));
    }
    out
}

#[test]
fn the_preview_toolbar_stays_wholly_inside_the_pane_at_its_floor() {
    for width in [
        viewwstudio::state::MIN_PANE,
        380.0,
        viewwstudio::ui::preview::COMPACT_BAR_BELOW - 1.0,
        460.0,
    ] {
        let (mut driver, studio) = shell();
        studio.preview_width.set(width);
        driver.draw_frame();

        let controls = toolbar_controls(&driver);
        assert_eq!(
            controls.len(),
            TOOLBAR.len(),
            "at width {width} the toolbar should show all five controls"
        );
        for (text, at, _, inside) in &controls {
            assert!(
                *inside,
                "at width {width}, {text} at ({:.0},{:.0}) is partly outside \
                 a box that clips it",
                at.dx, at.dy
            );
        }
    }
}

/// The reflow itself: narrow panes stack the toolbar, wide ones do not.
///
/// A bar that quietly went back to one row everywhere would still pass the
/// containment test above right up until the widths stopped fitting again.
#[test]
fn the_narrow_toolbar_reflows_onto_two_rows() {
    // Narrow: the Render button shares a row with the platform picker, and
    // the toggles are below them both.
    let (mut driver, studio) = shell();
    studio.preview_width.set(viewwstudio::state::MIN_PANE);
    driver.draw_frame();
    let narrow = toolbar_controls(&driver);
    let y = |needle: &str| {
        narrow
            .iter()
            .find(|(text, _, _, _)| text == needle)
            .map_or(f32::NAN, |(_, at, _, _)| at.dy)
    };
    assert_eq!(
        narrow.len(),
        TOOLBAR.len(),
        "the narrow toolbar shows everything"
    );
    assert!(
        (y("\"iOS\"") - y("\"Render\"")).abs() < 5.0,
        "narrow: the picker and the Render button should share a row"
    );
    assert!(
        y("\"Safe area\"") > y("\"Render\"") + 10.0,
        "narrow: the toggles should sit below the picker's row"
    );

    // Wide: everything on one row. The studio handle is not needed here — the
    // default shell is already wide — so it is bound to `_` rather than left
    // to warn.
    let (mut driver, _studio) = shell();
    driver.draw_frame();
    let wide = toolbar_controls(&driver);
    let y = |needle: &str| {
        wide.iter()
            .find(|(text, _, _, _)| text == needle)
            .map_or(f32::NAN, |(_, at, _, _)| at.dy)
    };
    assert_eq!(
        wide.len(),
        TOOLBAR.len(),
        "the wide toolbar shows everything"
    );
    assert!(
        (y("\"iOS\"") - y("\"Render\"")).abs() < 5.0
            && (y("\"iOS\"") - y("\"Safe area\"")).abs() < 5.0,
        "wide: the whole toolbar should share one row"
    );
}
