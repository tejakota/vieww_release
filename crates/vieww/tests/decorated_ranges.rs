//! A squiggle asked for on a `TextField` reaches the scene.
//!
//! ```console
//! cargo test -p vieww --test decorated_ranges
//! ```
//!
//! # Why this test exists at all
//!
//! `RenderEditableText`'s own tests prove that a `TextDecoration` handed to the
//! render object is drawn in the right place. They cannot prove that a
//! decoration handed to the *widget* ever reaches it — the widget stores it, a
//! registration in `factory.rs` copies it across, and a missing line there is
//! invisible to both sides. That is not a hypothetical failure mode in this
//! repository: `TRACKER.md` records cut, copy and paste being fully implemented
//! on both sides of exactly this boundary with no wire between them, and the
//! whole feature doing nothing for a milestone.
//!
//! So this drives the real stack — a widget tree built, mounted, laid out and
//! painted into a scene — and asserts on the fill commands the backend would
//! receive. A wave and an outline arrive as **fills of pre-expanded outlines**
//! rather than strokes: the pen is converted before the canvas sees it, the
//! same conversion `RenderIcon::paint` makes, so the marks rasterise like any
//! other shape on every backend rather than like a thin stroke on each of
//! them differently.

use std::rc::Rc;

use vieww::foundation::{Color, Size, TextDecoration, TextDecorationShape, TextRange};
use vieww::prelude::*;
use vieww::{FrameDriver, TextEditingValue};

const SURFACE: f32 = 400.0;
const TEXT: &str = "let x = 1;";

/// The scene a field with `decorations` paints.
fn scene_of(decorations: Vec<TextDecoration>) -> vieww::paint::Scene {
    let mut driver = FrameDriver::new(Size::square(SURFACE));
    let value = driver
        .elements()
        .runtime()
        .signal(TextEditingValue::new(TEXT));

    let sink = value.clone();
    let root: WidgetNode = SizedBox::width(SURFACE)
        .child(
            TextField::new(value.get())
                .size(20.0)
                // Off, so the caret's own fill cannot be mistaken for a mark.
                .show_cursor(false)
                .decorations(Rc::new(decorations))
                .on_changed(Rc::new(move |next| sink.set(next))),
        )
        .into();

    driver.elements().set_root(root);
    driver.draw_frame();
    driver.scene().clone()
}

/// Every filled path a mark leaves in a scene, with the paint it was filled
/// in. Squiggles and boxes arrive here — pre-expanded from their pens — and
/// nothing else in an undecorated, cursor-less field draws a `FillPath`, so
/// what this finds *is* the decoration.
fn marks(scene: &vieww::paint::Scene) -> Vec<(vieww::foundation::Rect, Color)> {
    scene
        .commands()
        .iter()
        .filter_map(|command| match command {
            vieww::paint::Command::FillPath { path, paint, .. } => {
                Some((path.bounds(), paint.color))
            }
            _ => None,
        })
        .collect()
}

/// Strokes still reaching the scene as strokes. Nothing should arrive here:
/// a decoration drawn through the stroke primitive has handed its crispness
/// back to the rasteriser, which is what the outline conversion exists to
/// avoid.
fn strokes(scene: &vieww::paint::Scene) -> Vec<(vieww::foundation::Rect, Color)> {
    scene
        .commands()
        .iter()
        .filter_map(|command| match command {
            vieww::paint::Command::StrokePath { path, paint, .. } => {
                Some((path.bounds(), paint.color))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_squiggle_asked_for_on_the_widget_is_drawn_in_the_scene() {
    let bare = scene_of(Vec::new());
    assert!(
        marks(&bare).is_empty(),
        "an undecorated field fills no path, so a fill below means the mark"
    );
    assert!(
        strokes(&bare).is_empty(),
        "and nothing is stroked either — the bare scene draws only glyphs"
    );

    let marked = scene_of(vec![TextDecoration::squiggle(
        TextRange::new(4, 5),
        Color::RED,
    )]);
    let waves = marks(&marked);
    assert_eq!(
        waves.len(),
        1,
        "the decoration did not survive the widget-to-render crossing"
    );
    assert_eq!(waves[0].1, Color::RED);
    // The expanded pen, not a raw stroke: a decoration handed to the
    // rasteriser as a stroke is the exact primitive the conversion retired.
    assert!(
        strokes(&marked).is_empty(),
        "the wave reached the scene as a stroke — the pen should be a fill"
    );

    // Under one character of ten, not under the line. A wave spanning the whole
    // field is the signature of a decoration whose range was ignored.
    let width = waves[0].0.right - waves[0].0.left;
    assert!(
        width < SURFACE / 4.0,
        "the wave spans {width:.1} of a {SURFACE:.0} field — the range was dropped"
    );
}

#[test]
fn several_decorations_of_different_shapes_coexist() {
    let scene = scene_of(vec![
        TextDecoration::squiggle(TextRange::new(0, 3), Color::RED),
        TextDecoration::boxed(TextRange::new(4, 5), Color::BLUE),
        TextDecoration::squiggle(TextRange::new(8, 9), Color::RED)
            .shape(TextDecorationShape::Underline),
    ]);

    // Two filled paths — the wave and the outline, both pre-expanded from
    // their pens. The underline is a plain rectangle fill, which is the
    // deliberate asymmetry `paint_decorations` documents: a straight rule is
    // a rectangle already, and expanding a pen to draw one is more work for
    // the same pixels.
    let filled = marks(&scene);
    assert_eq!(filled.len(), 2, "one wave and one outline");
    assert!(filled.iter().any(|(_, color)| *color == Color::BLUE));
    assert!(filled.iter().any(|(_, color)| *color == Color::RED));
    assert!(
        strokes(&scene).is_empty(),
        "no mark should reach the backend as a stroke"
    );
}

#[test]
fn a_field_that_asks_for_no_decorations_pays_for_none() {
    // The guarantee that makes it safe to re-derive the whole list on every
    // keystroke: an empty list is an early return, not a loop over nothing that
    // still crosses the canvas.
    let scene = scene_of(Vec::new());
    let shape_commands = scene
        .commands()
        .iter()
        .filter(|command| {
            matches!(
                command,
                vieww::paint::Command::StrokePath { .. } | vieww::paint::Command::FillPath { .. }
            )
        })
        .count();
    assert_eq!(shape_commands, 0);
}
