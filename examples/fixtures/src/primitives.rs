//! Tier 0 — one primitive per fixture, drawn many times.
//!
//! Each of these is deliberately boring to look at and deliberately
//! *repetitive*: the same primitive, several hundred times, so that the
//! reported cost is the primitive's own and a per-command number means
//! something. A fixture that mixes six primitives tells you a frame was slow;
//! these tell you which primitive made it slow.
//!
//! The pairs matter more than the fixtures. `fills` against `fills_rrect_clip`
//! is the clip question; `fills` against `fills_gradient` is the shading
//! question; `layers_flat` against `layers_nested` is the compositing
//! question. Read them in pairs.

use vieww_foundation::{BlendMode, Color, EdgeInsets, Gradient, Offset, Path, Rect, Shadow, Size};
use vieww_widget::prelude::*;
use vieww_widget::{Clip, Filtered, PaintWith, Painting};

pub(crate) const INK: Color = Color::rgb(23, 30, 42);
pub(crate) const MUTED: Color = Color::rgb(110, 122, 140);
pub(crate) const PAPER: Color = Color::rgb(247, 248, 250);
pub(crate) const ACCENT: Color = Color::rgb(58, 122, 246);
pub(crate) const VIOLET: Color = Color::rgb(168, 85, 247);
pub(crate) const MINT: Color = Color::rgb(76, 187, 129);

/// The grid every tier-0 fixture draws, so the only difference between them
/// is the treatment applied to it.
const COLS: usize = 24;
const ROWS: usize = 16;
const CELL: f32 = 34.0;

fn cell_rect(i: usize) -> Rect {
    let x = (i % COLS) as f32 * CELL + 4.0;
    let y = (i / COLS) as f32 * CELL + 4.0;
    Rect::new(x, y, x + CELL - 8.0, y + CELL - 8.0)
}

fn cell_color(i: usize) -> Color {
    let t = (i % 37) as f32 / 37.0;
    Color::rgb(
        (40.0 + t * 180.0) as u8,
        (90.0 + t * 90.0) as u8,
        (200.0 - t * 60.0) as u8,
    )
}

const COUNT: usize = COLS * ROWS;

/// Plain axis-aligned fills. The control every other tier-0 fixture is read
/// against.
pub(crate) fn fills() -> WidgetNode {
    Painting::new(PaintWith::new(|book, _size| {
        for i in 0..COUNT {
            book.rect(cell_rect(i), cell_color(i));
        }
    }))
    .into()
}

/// The same fills, rounded. Rounded corners mean flattened cubics and a
/// larger edge list per shape — the cost of *curves*, isolated.
pub(crate) fn fills_rounded() -> WidgetNode {
    Painting::new(PaintWith::new(|book, _size| {
        for i in 0..COUNT {
            book.rrect(cell_rect(i), 8.0, cell_color(i));
        }
    }))
    .into()
}

/// The same fills, gradient-shaded. Isolates per-pixel shading from per-shape
/// geometry: identical edges, a ramp evaluated at every covered pixel.
pub(crate) fn fills_gradient() -> WidgetNode {
    Painting::new(PaintWith::new(|book, _size| {
        let ramp = Gradient::vertical().with_stops(&[(0.0, ACCENT), (1.0, VIOLET)]);
        for i in 0..COUNT {
            book.rrect(cell_rect(i), 6.0, ramp);
        }
    }))
    .into()
}

/// The same fills under **one rounded clip covering the whole surface**.
///
/// This is the fixture the gallery exists for. Against [`fills`] it asks the
/// question that took a studio from a frame to a hang: does a clip cost the
/// subtree once, or every command inside it once each? A renderer that
/// re-resolves the clip per command shows up here as a two-orders-of-magnitude
/// gap between two pictures that differ only at the corners.
pub(crate) fn fills_rrect_clip() -> WidgetNode {
    Clip::rounded(18.0)
        .child(Painting::new(PaintWith::new(|book, _size| {
            for i in 0..COUNT {
                book.rect(cell_rect(i), cell_color(i));
            }
        })))
        .into()
}

/// The same again, three clips deep. Nesting must add work proportional to
/// the number of *clips*, not to clips times commands.
pub(crate) fn fills_nested_clips() -> WidgetNode {
    Clip::rounded(18.0)
        .child(
            Container::new().padding(EdgeInsets::all(10.0)).child(
                Clip::rounded(14.0).child(Container::new().padding(EdgeInsets::all(10.0)).child(
                    Clip::oval().child(Painting::new(PaintWith::new(|book, _size| {
                        for i in 0..COUNT {
                            book.rect(cell_rect(i), cell_color(i));
                        }
                    }))),
                )),
            ),
        )
        .into()
}

/// Strokes rather than fills: stroke expansion, joins and caps.
pub(crate) fn strokes() -> WidgetNode {
    Painting::new(PaintWith::new(|book, _size| {
        for i in 0..COUNT {
            book.stroke_rrect(cell_rect(i), 6.0, cell_color(i), 2.0);
        }
    }))
    .into()
}

/// Open curved paths — the flattener's actual workload, not a rounded box's
/// four fixed corners.
pub(crate) fn curves() -> WidgetNode {
    Painting::new(PaintWith::new(|book, _size| {
        for i in 0..COUNT {
            let r = cell_rect(i);
            let mut path = Path::new();
            path.move_to(Offset::new(r.left, r.bottom));
            path.cubic_to(
                Offset::new(r.left, r.top),
                Offset::new(r.right, r.bottom),
                Offset::new(r.right, r.top),
            );
            book.stroke(path, cell_color(i), 2.0);
        }
    }))
    .into()
}

/// Shadows. Each is a blurred mask of its own, so this is the fixture that
/// says whether a card list can afford elevation.
pub(crate) fn shadows() -> WidgetNode {
    Painting::new(PaintWith::new(|book, _size| {
        for i in 0..COUNT {
            book.shadow(
                cell_rect(i),
                8.0,
                Shadow::new(Color::rgba(0, 0, 0, 70), Offset::new(0.0, 3.0), 10.0),
            );
            book.rrect(cell_rect(i), 8.0, Color::WHITE);
        }
    }))
    .into()
}

/// Flat sibling layers — one `PushLayer`/`PopLayer` pair per cell, each with
/// its own offscreen buffer to allocate, fill and composite back.
pub(crate) fn layers_flat() -> WidgetNode {
    let mut stack = Stack::new().fit(StackFit::Expand);
    for i in 0..COUNT {
        let r = cell_rect(i);
        stack = stack.push(
            Positioned::new().left(r.left).top(r.top).child(
                Opacity::new(0.6).child(
                    Container::new()
                        .color(cell_color(i))
                        .radius(6.0)
                        .size(r.width(), r.height()),
                ),
            ),
        );
    }
    stack.into()
}

/// Layers *nested* rather than side by side. Depth, not count: an offscreen
/// buffer inside an offscreen buffer inside another is where a compositor
/// that reallocates instead of pooling gets found out.
pub(crate) fn layers_nested() -> WidgetNode {
    let mut node: WidgetNode = Container::new()
        .color(ACCENT)
        .radius(8.0)
        .size(160.0, 160.0)
        .into();
    for depth in 0..24 {
        let alpha = 0.97;
        node = Opacity::new(alpha)
            .child(
                Container::new()
                    .padding(EdgeInsets::all(6.0))
                    .color(if depth % 2 == 0 {
                        Color::rgba(255, 255, 255, 20)
                    } else {
                        Color::rgba(0, 0, 0, 16)
                    })
                    .radius(10.0)
                    .child(node),
            )
            .into();
    }
    Container::new()
        .color(PAPER)
        .alignment(Alignment::CENTER)
        .child(node)
        .into()
}

/// The blend modes, one tile each, over a shared gradient backdrop — the
/// isolated-group path, which is the expensive one and the one that is easy
/// to get visibly wrong.
pub(crate) fn blend_modes() -> WidgetNode {
    const MODES: [(BlendMode, &str); 12] = [
        (BlendMode::Normal, "normal"),
        (BlendMode::Multiply, "multiply"),
        (BlendMode::Screen, "screen"),
        (BlendMode::Overlay, "overlay"),
        (BlendMode::Darken, "darken"),
        (BlendMode::Lighten, "lighten"),
        (BlendMode::ColorDodge, "dodge"),
        (BlendMode::ColorBurn, "burn"),
        (BlendMode::HardLight, "hard light"),
        (BlendMode::SoftLight, "soft light"),
        (BlendMode::Difference, "difference"),
        (BlendMode::Exclusion, "exclusion"),
    ];

    let mut tiles: Vec<WidgetNode> = Vec::new();
    for (mode, name) in MODES {
        tiles.push(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(6.0)
                .push(
                    Container::new()
                        .gradient(Gradient::horizontal().with_stops(&[
                            (0.0, ACCENT),
                            (0.5, MINT),
                            (1.0, VIOLET),
                        ]))
                        .radius(8.0)
                        .size(150.0, 90.0)
                        .alignment(Alignment::CENTER)
                        .child(
                            // `Opacity::blend` is the widget-level route to an
                            // isolated group with a blend mode — the same
                            // `PushLayer` the renderer resolves into its own
                            // buffer before compositing back.
                            Opacity::new(1.0).blend(mode).child(Painting::sized(
                                Size::new(110.0, 60.0),
                                PaintWith::new(|book, _size| {
                                    book.circle(
                                        Offset::new(35.0, 30.0),
                                        26.0,
                                        Color::rgb(250, 200, 90),
                                    );
                                    book.circle(
                                        Offset::new(75.0, 30.0),
                                        26.0,
                                        Color::rgb(90, 200, 250),
                                    );
                                }),
                            )),
                        ),
                )
                .push(Text::new(name).color(MUTED).size(11.0))
                .into(),
        );
    }
    let grid = Grid::columns(4).gap(12.0).children(tiles);

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(16.0))
        .child(grid)
        .into()
}

/// Blur, at several sigmas. A blur's cost is area times kernel, and its
/// *bounds* are the thing that goes wrong quietly — a blur cut off square at
/// its own edge is the classic tell.
pub(crate) fn blurs() -> WidgetNode {
    let mut row = Flex::row().spacing(16.0);
    for sigma in [0.0f32, 2.0, 6.0, 12.0, 20.0] {
        row = row.push(
            Flex::column()
                .spacing(8.0)
                .push(
                    Container::new()
                        .color(Color::WHITE)
                        .radius(12.0)
                        .padding(EdgeInsets::all(14.0))
                        .child(
                            Filtered::blur(sigma).child(
                                Container::new()
                                    .gradient(Gradient::radial_fill().with_stops(&[
                                        (0.0, Color::rgb(255, 210, 90)),
                                        (1.0, VIOLET),
                                    ]))
                                    .radius(16.0)
                                    .size(110.0, 110.0),
                            ),
                        ),
                )
                .push(Text::new(format!("σ {sigma:.0}")).color(MUTED).size(11.0)),
        );
    }
    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(20.0))
        .alignment(Alignment::CENTER)
        .child(row)
        .into()
}

/// Transforms: rotation and scale, which turn every axis-aligned fast path
/// off and put the general rasteriser to work.
pub(crate) fn transforms() -> WidgetNode {
    let mut stack = Stack::new().fit(StackFit::Expand);
    for i in 0..96 {
        let t = i as f32 / 96.0;
        let angle = t * std::f32::consts::TAU;
        let radius = 60.0 + t * 180.0;
        stack = stack.push(
            Positioned::new()
                .left(400.0 + radius * angle.cos() - 24.0)
                .top(280.0 + radius * angle.sin() - 24.0)
                .child(
                    Transformed::rotate(angle).child(
                        Transformed::scale(0.5 + t, 0.5 + t).child(
                            Container::new()
                                .color(cell_color(i))
                                .radius(4.0)
                                .size(30.0, 12.0),
                        ),
                    ),
                ),
        );
    }
    Container::new()
        .color(Color::rgb(16, 18, 24))
        .child(stack)
        .into()
}
