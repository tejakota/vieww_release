//! Tier 1 — vector art, icons and text: the things a framework is judged on
//! before anybody reads a single feature list.
//!
//! Icons and type are where a rendering engine's quality is actually visible.
//! A 13-point icon with a stroke half a pixel wide either reads or turns to
//! mush; a paragraph either has even colour or it does not. Both are also
//! *many small shapes*, which is a different cost profile from the tier-0
//! grids: per-shape overhead dominates instead of per-pixel work, and an
//! engine can be excellent at one and terrible at the other.

use vieww_foundation::{
    parse_path_data, Color, EdgeInsets, IconData, Offset, Path, Rect, Shadow, Size, Transform,
};
use vieww_widget::prelude::*;
use vieww_widget::{Clip, PaintWith, Painting};

use crate::primitives::{ACCENT, INK, MINT, MUTED, PAPER, VIOLET};

/// A small centreline icon set, as SVG path data on a 24-unit grid.
///
/// Written out rather than pulled from the built-in set because the point is
/// *variety of geometry*: closed polygons, open polylines, arcs, holes and
/// long curve chains all rasterise differently, and a fixture built from ten
/// chevrons would exercise one of those five times over.
const ICONS: [(&str, &str); 10] = [
    ("folder", "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"),
    ("search", "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM16 16l5 5"),
    ("code", "M9 6l-5 6 5 6M15 6l5 6-5 6"),
    ("bolt", "M13 2L4 14h7l-1 8 9-12h-7z"),
    ("heart", "M12 21C7 17 3 13.5 3 9.5A4.5 4.5 0 0 1 12 7a4.5 4.5 0 0 1 9 2.5c0 4-4 7.5-9 11.5z"),
    ("gear", "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2"),
    ("chart", "M4 20V10M10 20V4M16 20v-7M22 20H2"),
    ("cloud", "M7 18a4 4 0 0 1 0-8 5 5 0 0 1 9.6-1.6A3.5 3.5 0 0 1 18 18z"),
    ("layers", "M12 3l9 5-9 5-9-5zM3 13l9 5 9-5"),
    ("clock", "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2"),
];

fn icon_data(index: usize) -> IconData {
    parse_path_data(ICONS[index % ICONS.len()].1)
        .map_or_else(|_| IconData::square24(Path::new()), IconData::square24)
}

/// The same ten icons filled, at four sizes — 120 of them.
///
/// Four sizes rather than one because an icon set's real test is whether it
/// survives being small. The 13-point row is where a rasteriser without
/// sub-pixel coverage falls apart, and it is the row a studio's activity bar
/// actually uses.
pub(crate) fn icon_grid() -> WidgetNode {
    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(18.0)
        .push(
            Text::new("icons — filled, four sizes")
                .color(INK)
                .size(15.0)
                .bold(),
        );

    for size in [13.0f32, 18.0, 28.0, 44.0] {
        let mut row = Flex::row().spacing(14.0);
        for i in 0..ICONS.len() {
            row = row.push(Icon::new(icon_data(i)).size(size).color(tint(i)));
        }
        column = column.push(
            Flex::row()
                .spacing(16.0)
                .push(
                    SizedBox::from_size(Size::new(46.0, 1.0))
                        .child(Text::new(format!("{size:.0}pt")).color(MUTED).size(11.0)),
                )
                .push(row),
        );
    }

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(24.0))
        .child(column)
        .into()
}

/// The same ten icons as *centreline strokes*, which is the other icon
/// tradition and a genuinely different rasterisation path: stroke expansion
/// with joins and caps, not a filled outline.
pub(crate) fn icon_strokes() -> WidgetNode {
    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(18.0)
        .push(
            Text::new("icons — centreline strokes")
                .color(INK)
                .size(15.0)
                .bold(),
        );

    for (size, weight) in [(14.0f32, 1.5f32), (20.0, 1.6), (30.0, 1.8), (46.0, 2.0)] {
        let mut row = Flex::row().spacing(14.0);
        for i in 0..ICONS.len() {
            row = row.push(
                Icon::new(icon_data(i))
                    .size(size)
                    .stroke(weight)
                    .color(tint(i)),
            );
        }
        column = column.push(
            Flex::row()
                .spacing(16.0)
                .push(
                    SizedBox::from_size(Size::new(46.0, 1.0))
                        .child(Text::new(format!("{size:.0}pt")).color(MUTED).size(11.0)),
                )
                .push(row),
        );
    }

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(24.0))
        .child(column)
        .into()
}

fn tint(i: usize) -> Color {
    const TINTS: [Color; 5] = [INK, ACCENT, VIOLET, MINT, Color::rgb(240, 150, 60)];
    TINTS[i % TINTS.len()]
}

/// A type scale, plus a paragraph. Real glyph outlines, real line breaking.
pub(crate) fn typography() -> WidgetNode {
    const BODY: &str = "A rendering engine is judged on paragraphs before it is judged on \
        anything else. Even colour, an unbroken baseline grid and edges that do not shimmer \
        at small sizes are what make an interface feel built rather than assembled.";

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(28.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(10.0)
                .children(children![
                    Text::new("Display 40").color(INK).size(40.0).bold(),
                    Text::new("Headline 28").color(INK).size(28.0).bold(),
                    Text::new("Title 20").color(INK).size(20.0).bold(),
                    Text::new("Subtitle 16").color(MUTED).size(16.0),
                    SizedBox::from_size(Size::new(0.0, 8.0)),
                    Text::new(BODY).color(INK).size(14.0),
                    SizedBox::from_size(Size::new(0.0, 8.0)),
                    Text::new(BODY).color(MUTED).size(12.0),
                    SizedBox::from_size(Size::new(0.0, 8.0)),
                    Text::new(
                        "11pt caption — the size at which anti-aliasing stops being optional"
                    )
                    .color(MUTED)
                    .size(11.0),
                ]),
        )
        .into()
}

/// Syntax-coloured code: dozens of short, differently-coloured runs on each
/// line, which is the *worst* shape of text workload — every colour change is
/// a separate glyph run, and a studio's editor is nothing but this.
pub(crate) fn code_block() -> WidgetNode {
    const LINES: [&[(&str, u8)]; 18] = [
        &[("//", 2), (" the frame the whole gallery is about", 2)],
        &[],
        &[
            ("pub fn", 0),
            (" present", 1),
            ("(", 3),
            ("&mut self", 0),
            (", scene: ", 3),
            ("&Scene", 4),
            (") {", 3),
        ],
        &[
            ("    let", 0),
            (" (pixels, report) = ", 3),
            ("self", 0),
            (".cpu", 3),
        ],
        &[
            ("        .render_to_pixels", 1),
            ("(scene, w, h, base)?;", 3),
        ],
        &[],
        &[("    if", 0), (" logical == physical {", 3)],
        &[
            ("        Cow", 4),
            ("::", 3),
            ("Borrowed", 1),
            ("(pixels.data())", 3),
        ],
        &[("    } else {", 3)],
        &[
            ("        Cow", 4),
            ("::", 3),
            ("Owned", 1),
            ("(upscale(pixels))", 3),
        ],
        &[("    }", 3)],
        &[("}", 3)],
        &[],
        &[("#[test]", 5)],
        &[("fn", 0), (" one_clip_rasterises_once", 1), ("() {", 3)],
        &[("    assert_eq!", 1), ("(misses, ", 3), ("1", 6), (");", 3)],
        &[("}", 3)],
        &[],
    ];
    const PALETTE: [Color; 7] = [
        Color::rgb(197, 134, 192), // keyword
        Color::rgb(220, 220, 170), // function
        Color::rgb(106, 153, 85),  // comment
        Color::rgb(212, 212, 212), // plain
        Color::rgb(78, 201, 176),  // type
        Color::rgb(156, 220, 254), // attribute
        Color::rgb(181, 206, 168), // number
    ];

    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(3.0);
    for (n, line) in LINES.iter().enumerate() {
        let mut row = Flex::row().push(
            SizedBox::from_size(Size::new(38.0, 1.0)).child(
                Text::new(format!("{:>3}", n + 1))
                    .color(Color::rgb(90, 96, 110))
                    .size(12.0),
            ),
        );
        for (text, colour) in line.iter() {
            row = row.push(Text::new(*text).color(PALETTE[*colour as usize]).size(13.0));
        }
        column = column.push(row);
    }

    Container::new()
        .color(Color::rgb(16, 18, 24))
        .padding(EdgeInsets::all(20.0))
        .child(column)
        .into()
}

/// One layered vector portrait: curves, gradients, clips, shadows and
/// overlapping translucent layers, all in one shape tree.
///
/// This is the "complex vector" end of the gallery — the closest a 2D engine
/// gets to the shading a 3D avatar implies, and a much harder rasterisation
/// problem than any grid of boxes: long curve chains, clipped groups, and
/// soft edges that reveal every seam if compositing is not isolated properly.
fn face(book: &mut vieww_foundation::Sketchbook, o: Offset, s: f32, skin: Color, hair: Color) {
    use vieww_foundation::Gradient;

    let p = |x: f32, y: f32| Offset::new(o.dx + x * s, o.dy + y * s);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Rect::new(o.dx + x0 * s, o.dy + y0 * s, o.dx + x1 * s, o.dy + y1 * s)
    };

    // The disc behind the head, so the portrait reads as a badge.
    book.circle(
        p(50.0, 50.0),
        48.0 * s,
        Gradient::radial_fill().with_stops(&[
            (0.0, Color::rgb(238, 242, 250)),
            (1.0, Color::rgb(214, 222, 238)),
        ]),
    );

    // Shoulders: one closed curve, clipped by the disc above it.
    let mut shoulders = Path::new();
    shoulders.move_to(p(14.0, 100.0));
    shoulders.cubic_to(p(20.0, 74.0), p(38.0, 68.0), p(50.0, 68.0));
    shoulders.cubic_to(p(62.0, 68.0), p(80.0, 74.0), p(86.0, 100.0));
    shoulders.line_to(p(14.0, 100.0));
    shoulders.close();
    book.fill(
        shoulders,
        Gradient::vertical().with_stops(&[(0.0, ACCENT), (1.0, Color::rgb(38, 82, 180))]),
    );

    // Neck, then head — head over neck so the jaw line is the head's own.
    book.rrect(r(43.0, 54.0, 57.0, 70.0), 6.0 * s, shade(skin, 0.86));
    book.circle(p(50.0, 40.0), 22.0 * s, skin);

    // Hair: a crescent, drawn as one path with a curved underside.
    let mut fringe = Path::new();
    fringe.move_to(p(28.0, 40.0));
    fringe.cubic_to(p(28.0, 16.0), p(72.0, 16.0), p(72.0, 40.0));
    fringe.cubic_to(p(66.0, 30.0), p(58.0, 26.0), p(50.0, 30.0));
    fringe.cubic_to(p(42.0, 34.0), p(34.0, 34.0), p(28.0, 40.0));
    fringe.close();
    book.fill(fringe, hair);

    // Eyes and brows: small shapes at small scale, which is where a
    // rasteriser's coverage accuracy shows.
    for dx in [-8.0f32, 8.0] {
        book.circle(p(50.0 + dx, 40.0), 3.2 * s, Color::rgb(38, 44, 58));
        book.circle(
            p(50.0 + dx + 1.0, 39.0),
            1.1 * s,
            Color::rgba(255, 255, 255, 210),
        );
        let mut brow = Path::new();
        brow.move_to(p(50.0 + dx - 5.0, 32.0));
        brow.cubic_to(
            p(50.0 + dx - 2.0, 30.0),
            p(50.0 + dx + 2.0, 30.0),
            p(50.0 + dx + 5.0, 32.0),
        );
        book.stroke(brow, shade(hair, 0.8), 1.6 * s);
    }

    // A smile, as an arc rather than a polyline.
    let mut smile = Path::new();
    smile.move_to(p(42.0, 47.0));
    smile.cubic_to(p(46.0, 52.0), p(54.0, 52.0), p(58.0, 47.0));
    book.stroke(smile, shade(skin, 0.6), 1.8 * s);

    // A translucent highlight over the whole head — an isolated layer, so it
    // must not leak past the head's own silhouette.
    book.layer(
        0.35,
        0.0,
        Some(Path::rounded_rect(r(28.0, 18.0, 72.0, 62.0), 22.0 * s)),
        |inner| {
            inner.circle(p(41.0, 32.0), 12.0 * s, Color::rgba(255, 255, 255, 180));
        },
    );
}

fn shade(color: Color, factor: f32) -> Color {
    Color::rgba(
        (f32::from(color.r) * factor) as u8,
        (f32::from(color.g) * factor) as u8,
        (f32::from(color.b) * factor) as u8,
        color.a,
    )
}

pub(crate) fn avatar() -> WidgetNode {
    Container::new()
        .color(PAPER)
        .alignment(Alignment::CENTER)
        .child(
            Container::new()
                .color(Color::WHITE)
                .radius(24.0)
                .shadow(Shadow::new(
                    Color::rgba(20, 30, 60, 60),
                    Offset::new(0.0, 12.0),
                    40.0,
                ))
                .padding(EdgeInsets::all(24.0))
                .child(Painting::sized(
                    Size::new(400.0, 400.0),
                    PaintWith::new(|book, _size| {
                        face(
                            book,
                            Offset::new(0.0, 0.0),
                            4.0,
                            Color::rgb(242, 205, 178),
                            Color::rgb(60, 44, 40),
                        );
                    }),
                )),
        )
        .into()
}

/// Twenty-four portraits, each clipped to a circle: the "team page" workload,
/// and the one where a per-command clip cost multiplies by everything drawn
/// inside every avatar.
pub(crate) fn avatar_wall() -> WidgetNode {
    const SKINS: [Color; 6] = [
        Color::rgb(242, 205, 178),
        Color::rgb(216, 168, 132),
        Color::rgb(176, 124, 92),
        Color::rgb(126, 84, 60),
        Color::rgb(250, 220, 200),
        Color::rgb(198, 146, 110),
    ];
    const HAIRS: [Color; 6] = [
        Color::rgb(60, 44, 40),
        Color::rgb(28, 26, 30),
        Color::rgb(140, 96, 52),
        Color::rgb(96, 60, 120),
        Color::rgb(200, 160, 70),
        Color::rgb(40, 70, 110),
    ];

    let mut portraits: Vec<WidgetNode> = Vec::new();
    for i in 0..24 {
        let skin = SKINS[i % SKINS.len()];
        let hair = HAIRS[(i * 5) % HAIRS.len()];
        portraits.push(
            Clip::oval()
                .child(Painting::sized(
                    Size::new(88.0, 88.0),
                    PaintWith::new(move |book, _size| {
                        face(book, Offset::new(0.0, 0.0), 0.88, skin, hair);
                    }),
                ))
                .into(),
        );
    }
    let grid = Grid::columns(8).gap(10.0).children(portraits);

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(20.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(14.0)
                .push(
                    Text::new("24 clipped vector portraits")
                        .color(INK)
                        .size(15.0)
                        .bold(),
                )
                .push(grid),
        )
        .into()
}

/// A transform-heavy variant kept out of the catalogue's hot path but useful
/// when chasing the general (non-axis-aligned) rasteriser.
#[allow(dead_code)]
pub(crate) fn avatar_rotated() -> WidgetNode {
    Painting::new(PaintWith::new(|book, _size| {
        for i in 0..12 {
            let angle = i as f32 / 12.0 * std::f32::consts::TAU;
            book.transformed(
                Transform::translate(Offset::new(
                    420.0 + 180.0 * angle.cos(),
                    280.0 + 180.0 * angle.sin(),
                ))
                .then(Transform::rotate(angle)),
                |inner| {
                    face(
                        inner,
                        Offset::new(-40.0, -40.0),
                        0.8,
                        Color::rgb(242, 205, 178),
                        Color::rgb(60, 44, 40),
                    );
                },
            );
        }
    }))
    .into()
}
