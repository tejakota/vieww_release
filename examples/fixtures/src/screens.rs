//! Tier 2 — whole screens, the shapes real applications actually have.
//!
//! The tier-0 grids answer "what does this primitive cost". These answer the
//! only question that matters afterwards: *does an interface built out of them
//! hold a frame budget.* They are built the way an application would build
//! them — nested containers with rounded corners and padding, panels inside
//! panels, elevation, translucency — rather than the way a benchmark would.
//! That is the point: the clip bug was invisible to every micro-benchmark and
//! unmissable in a screen, because only a screen nests a rounded panel around
//! four hundred other things.

use vieww_foundation::{Color, EdgeInsets, Gradient, Offset, Rect, Shadow, Size};
use vieww_widget::prelude::*;
use vieww_widget::{Clip, Filtered, PaintWith, Painting};

use crate::primitives::{ACCENT, INK, MINT, MUTED, PAPER, VIOLET};

const SURFACE: Color = Color::rgb(22, 25, 33);
const PANEL: Color = Color::rgb(28, 32, 42);
const EDGE: Color = Color::rgb(44, 50, 64);
const DIM: Color = Color::rgb(140, 150, 168);

/// A settings form: the most ordinary screen there is, and the one whose
/// per-row cost multiplies fastest.
pub(crate) fn settings() -> WidgetNode {
    // **Seven rows, and the count is load-bearing.**
    //
    // This was nine, and nine did not fit: the card wanted 613px of an
    // available 512, so `RenderColumn` reported a 101px overflow on every run
    // of the gallery and the saved PNG — the reference picture for what a
    // settings form looks like in this framework — showed the eighth row
    // sliced through the middle and the ninth missing altogether.
    //
    // It went unfixed because the warning went to stderr in the middle of a
    // tool that prints a line per picture, which is the exact failure mode
    // `vieww_render::overflow`'s own module doc describes. `runner.rs` now
    // counts overflows per fixture and `main.rs` exits non-zero on one, so this
    // cannot silently come back.
    //
    // Seven rather than a scroll view on purpose: this fixture exists to price
    // *chrome* — rows, switches, rules, a rounded clip and a shadow — and
    // wrapping it in a viewport would add culling to the thing being measured
    // and make its number incomparable with the run before it. A settings
    // screen that genuinely scrolls is what `list_view` and the virtualised
    // scrolling suite are for.
    const ROWS: [(&str, &str, bool); 7] = [
        ("Appearance", "Match the system theme", true),
        ("Reduced motion", "Skip non-essential animation", false),
        ("Live preview", "Re-render on every keystroke", true),
        ("Format on save", "Run the formatter before writing", true),
        ("Telemetry", "Share anonymous usage data", false),
        ("Auto-update", "Install updates in the background", true),
        (
            "Hardware cursor",
            "Let the compositor draw the caret",
            false,
        ),
    ];

    let mut list = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(0.0);
    for (i, (title, subtitle, on)) in ROWS.iter().enumerate() {
        list = list.push(row(title, subtitle, *on, i + 1 < ROWS.len()));
    }

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(24.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(16.0)
                .push(Text::new("Settings").color(INK).size(24.0).bold())
                .push(
                    // The rounded card every settings screen has — and the
                    // exact shape that used to charge each of the nine rows a
                    // full-card clip rasterisation of its own.
                    Container::new()
                        .color(Color::WHITE)
                        .radius(14.0)
                        .shadow(Shadow::new(
                            Color::rgba(20, 30, 60, 28),
                            Offset::new(0.0, 4.0),
                            18.0,
                        ))
                        .child(Clip::rounded(14.0).child(list)),
                ),
        )
        .into()
}

fn row(title: &str, subtitle: &str, on: bool, rule: bool) -> WidgetNode {
    let content = Container::new().padding(EdgeInsets::all(14.0)).child(
        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .push(
                Flexible::expanded(1).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(3.0)
                        .push(Text::new(title).color(INK).size(14.0))
                        .push(Text::new(subtitle).color(MUTED).size(12.0)),
                ),
            )
            .push(toggle(on)),
    );

    if rule {
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .push(content)
            .push(
                Container::new()
                    .color(Color::rgb(233, 236, 242))
                    .height(1.0),
            )
            .into()
    } else {
        content.into()
    }
}

fn toggle(on: bool) -> WidgetNode {
    Container::new()
        .color(if on {
            ACCENT
        } else {
            Color::rgb(214, 219, 228)
        })
        .radius(11.0)
        .size(38.0, 22.0)
        .alignment(if on {
            Alignment::CENTER_RIGHT
        } else {
            Alignment::CENTER_LEFT
        })
        .padding(EdgeInsets::all(3.0))
        .child(
            Container::new()
                .color(Color::WHITE)
                .radius(8.0)
                .size(16.0, 16.0)
                .shadow(Shadow::new(
                    Color::rgba(0, 0, 0, 60),
                    Offset::new(0.0, 1.0),
                    3.0,
                )),
        )
        .into()
}

/// A dark dashboard: gradients, elevation, charts and stat tiles. Everything
/// a "modern" interface leans on at once, which is also everything that is
/// expensive at once.
pub(crate) fn dashboard() -> WidgetNode {
    let stats = [
        ("Frame", "5.8 ms", MINT),
        ("Damage", "2.7 %", ACCENT),
        ("Raster", "51k px", VIOLET),
        ("Reused", "1.82M px", Color::rgb(240, 160, 70)),
    ];

    let mut tiles = Flex::row().spacing(12.0);
    for (label, value, colour) in stats {
        tiles = tiles.push(
            Flexible::expanded(1).child(
                Container::new()
                    .gradient(Gradient::vertical().with_stops(&[
                        (0.0, Color::rgb(34, 39, 51)),
                        (1.0, Color::rgb(26, 30, 40)),
                    ]))
                    .radius(12.0)
                    .padding(EdgeInsets::all(14.0))
                    .child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(6.0)
                            .push(Text::new(label).color(DIM).size(11.0))
                            .push(Text::new(value).color(colour).size(22.0).bold())
                            .push(Container::new().color(colour).radius(2.0).size(48.0, 3.0)),
                    ),
            ),
        );
    }

    Container::new()
        .color(Color::rgb(12, 14, 19))
        .padding(EdgeInsets::all(20.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(16.0)
                .push(
                    Text::new("Renderer telemetry")
                        .color(Color::WHITE)
                        .size(20.0)
                        .bold(),
                )
                .push(tiles)
                .push(
                    Container::new()
                        .color(PANEL)
                        .radius(14.0)
                        .padding(EdgeInsets::all(16.0))
                        .child(
                            Flex::column()
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .spacing(12.0)
                                .push(
                                    Text::new("Frame time, last 64 frames")
                                        .color(DIM)
                                        .size(12.0),
                                )
                                .push(Painting::sized(
                                    Size::new(760.0, 180.0),
                                    PaintWith::new(|book, size| {
                                        sparkline(book, size);
                                    }),
                                )),
                        ),
                ),
        )
        .into()
}

/// A frame-time trace, drawn by hand: an area fill under a curve, a rule, and
/// per-sample dots. The shape of every performance chart, and a good test of
/// long polyline fills.
fn sparkline(book: &mut vieww_foundation::Sketchbook, size: Size) {
    const N: usize = 64;
    let samples: Vec<f32> = (0..N)
        .map(|i| {
            let t = i as f32 / N as f32;
            let base = 5.0 + 2.5 * (t * 9.0).sin() + 1.4 * (t * 23.0).cos();
            if i == 41 {
                base + 7.0
            } else {
                base
            }
        })
        .collect();
    let max = 16.7f32;
    let x = |i: usize| i as f32 / (N - 1) as f32 * size.width;
    let y = |v: f32| size.height - (v / max).clamp(0.0, 1.0) * size.height;

    // The budget line, so the trace is readable as pass/fail rather than as a
    // shape.
    let mut budget = vieww_foundation::Path::new();
    budget.move_to(Offset::new(0.0, y(16.7)));
    budget.line_to(Offset::new(size.width, y(16.7)));
    book.stroke(budget, Color::rgba(240, 90, 90, 140), 1.0);

    let mut area = vieww_foundation::Path::new();
    area.move_to(Offset::new(0.0, size.height));
    for (i, v) in samples.iter().enumerate() {
        area.line_to(Offset::new(x(i), y(*v)));
    }
    area.line_to(Offset::new(size.width, size.height));
    area.close();
    book.fill(
        area,
        Gradient::vertical().with_stops(&[
            (0.0, Color::rgba(58, 122, 246, 140)),
            (1.0, Color::rgba(58, 122, 246, 10)),
        ]),
    );

    let mut line = vieww_foundation::Path::new();
    for (i, v) in samples.iter().enumerate() {
        if i == 0 {
            line.move_to(Offset::new(x(i), y(*v)));
        } else {
            line.line_to(Offset::new(x(i), y(*v)));
        }
    }
    book.stroke(line, ACCENT, 2.0);

    for (i, v) in samples.iter().enumerate() {
        if i % 4 == 0 {
            book.circle(Offset::new(x(i), y(*v)), 2.4, Color::WHITE);
        }
    }
}

/// An IDE shell at the size a laptop actually has: activity bar, file tree,
/// tab strip, syntax-coloured editor, a problems panel and a status bar.
///
/// This is the fixture that stands in for the studio, at the studio's own
/// window size, so a change to the framework can be judged against the thing
/// it is for without building the application.
pub(crate) fn editor() -> WidgetNode {
    Container::new()
        .color(SURFACE)
        .child(
            Flex::column()
                .push(title_bar())
                .push(
                    Flexible::expanded(1).child(
                        Flex::row()
                            .push(activity_bar())
                            .push(file_tree())
                            .push(Flexible::expanded(1).child(editor_pane())),
                    ),
                )
                .push(problems_panel())
                .push(status_bar()),
        )
        .into()
}

/// The same shell with the treatments a "premium" interface adds: a blurred
/// translucent command palette over it, real elevation on the panels, and a
/// soft vignette. Every one of these is a layer, and layers are where a
/// compositor either pools its buffers or falls over.
pub(crate) fn editor_glass() -> WidgetNode {
    Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            editor(),
            // A dim behind the palette — one full-surface translucent layer.
            Positioned::new()
                .left(0.0)
                .right(0.0)
                .top(0.0)
                .bottom(0.0)
                .child(Container::new().color(Color::rgba(6, 8, 12, 150))),
            Positioned::new()
                .left(383.0)
                .top(80.0)
                .child(command_palette()),
        ])
        .into()
}

fn command_palette() -> WidgetNode {
    const ITEMS: [(&str, &str); 7] = [
        ("Render: Run", "F5"),
        ("Render: Run and Watch", "Shift F5"),
        ("View: Toggle Problems", "Ctrl J"),
        ("Go to File...", "Ctrl P"),
        ("Preferences: Open Settings", "Ctrl ,"),
        ("Developer: Frame Statistics", ""),
        ("Developer: Reload Window", ""),
    ];

    let mut list = Flex::column().cross_axis_alignment(CrossAxisAlignment::Start);
    for (i, (name, shortcut)) in ITEMS.iter().enumerate() {
        list =
            list.push(
                Container::new()
                    .color(if i == 0 {
                        Color::rgba(58, 122, 246, 60)
                    } else {
                        Color::TRANSPARENT
                    })
                    .padding(EdgeInsets::all(10.0))
                    .child(
                        Flex::row()
                            .push(Flexible::expanded(1).child(
                                Text::new(*name).color(Color::rgb(226, 232, 244)).size(13.0),
                            ))
                            .push(Text::new(*shortcut).color(DIM).size(11.0)),
                    ),
            );
    }

    Container::new()
        .radius(14.0)
        .shadow(Shadow::new(
            Color::rgba(0, 0, 0, 160),
            Offset::new(0.0, 24.0),
            60.0,
        ))
        .width(600.0)
        .child(Clip::rounded(14.0).child(Stack::new().children(children![
                    // The frosted backdrop: a blurred layer under the
                    // panel's own translucent fill.
                    Filtered::blur(14.0).child(
                        Container::new()
                            .gradient(Gradient::vertical().with_stops(&[
                                (0.0, Color::rgba(60, 70, 96, 220)),
                                (1.0, Color::rgba(30, 36, 50, 230)),
                            ]))
                            .width(600.0)
                            .height(300.0),
                    ),
                    Container::new().padding(EdgeInsets::all(12.0)).child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(8.0)
                            .push(
                                Container::new()
                                    .color(Color::rgba(0, 0, 0, 80))
                                    .radius(8.0)
                                    .padding(EdgeInsets::all(10.0))
                                    .child(Text::new("> render").color(Color::WHITE).size(14.0)),
                            )
                            .push(list),
                    ),
                ])))
        .into()
}

fn title_bar() -> WidgetNode {
    Container::new()
        .color(Color::rgb(26, 29, 38))
        .padding(EdgeInsets::all(8.0))
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(8.0)
                .push(dot(Color::rgb(240, 96, 88)))
                .push(dot(Color::rgb(244, 190, 79)))
                .push(dot(Color::rgb(97, 197, 84)))
                .push(SizedBox::from_size(Size::new(14.0, 1.0)))
                .push(Text::new("main.rs — vieww studio").color(DIM).size(12.0)),
        )
        .into()
}

fn dot(colour: Color) -> WidgetNode {
    Container::new()
        .color(colour)
        .radius(6.0)
        .size(11.0, 11.0)
        .into()
}

fn activity_bar() -> WidgetNode {
    let mut column = Flex::column().spacing(6.0);
    for i in 0..7 {
        column = column.push(
            Container::new()
                .color(if i == 0 {
                    Color::rgba(58, 122, 246, 46)
                } else {
                    Color::TRANSPARENT
                })
                .radius(8.0)
                .padding(EdgeInsets::all(9.0))
                .child(
                    Container::new()
                        .color(if i == 0 { Color::WHITE } else { DIM })
                        .radius(3.0)
                        .size(18.0, 18.0),
                ),
        );
    }
    Container::new()
        .color(Color::rgb(19, 21, 28))
        .padding(EdgeInsets::all(6.0))
        .child(column)
        .into()
}

fn file_tree() -> WidgetNode {
    const FILES: [(&str, usize, bool); 14] = [
        ("crates", 0, true),
        ("vieww-paint", 1, true),
        ("native", 2, true),
        ("clip.rs", 3, false),
        ("reference.rs", 3, false),
        ("target.rs", 3, false),
        ("geometry", 3, true),
        ("fill.rs", 4, false),
        ("stroke.rs", 4, false),
        ("scene.rs", 2, false),
        ("damage.rs", 2, false),
        ("vieww-render", 1, true),
        ("driver.rs", 2, false),
        ("apps", 0, true),
    ];

    let mut column = Flex::column().cross_axis_alignment(CrossAxisAlignment::Start);
    for (name, depth, folder) in FILES {
        column = column.push(
            Container::new().padding(EdgeInsets::all(4.0)).child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(6.0)
                    .push(SizedBox::from_size(Size::new(depth as f32 * 12.0, 1.0)))
                    .push(
                        Container::new()
                            .color(if folder {
                                Color::rgb(224, 176, 90)
                            } else {
                                DIM
                            })
                            .radius(2.0)
                            .size(11.0, 11.0),
                    )
                    .push(
                        Text::new(name)
                            .color(if folder {
                                Color::rgb(214, 220, 234)
                            } else {
                                Color::rgb(168, 176, 194)
                            })
                            .size(12.0),
                    ),
            ),
        );
    }

    Container::new()
        .color(Color::rgb(23, 26, 34))
        .width(230.0)
        .padding(EdgeInsets::all(8.0))
        .child(column)
        .into()
}

fn editor_pane() -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .push(tab_strip())
        .push(Flexible::expanded(1).child(crate::vectors::code_block()))
        .into()
}

fn tab_strip() -> WidgetNode {
    let mut row = Flex::row().spacing(1.0);
    for (i, name) in ["native.rs", "clip.rs", "reference.rs", "fill.rs"]
        .iter()
        .enumerate()
    {
        row = row.push(
            Container::new()
                .color(if i == 1 {
                    Color::rgb(16, 18, 24)
                } else {
                    Color::rgb(24, 27, 35)
                })
                .padding(EdgeInsets::all(10.0))
                .child(
                    Text::new(*name)
                        .color(if i == 1 { Color::WHITE } else { DIM })
                        .size(12.0),
                ),
        );
    }
    Container::new()
        .color(Color::rgb(24, 27, 35))
        .child(row)
        .into()
}

fn problems_panel() -> WidgetNode {
    const PROBLEMS: [(&str, &str, Color); 4] = [
        (
            "clip.rs:147",
            "clip mask resolved per command",
            Color::rgb(240, 120, 90),
        ),
        (
            "native.rs:214",
            "damage ignored by present()",
            Color::rgb(244, 190, 79),
        ),
        (
            "fill.rs:160",
            "no active edge table",
            Color::rgb(244, 190, 79),
        ),
        (
            "target.rs:88",
            "layer buffer not pooled",
            Color::rgb(140, 190, 250),
        ),
    ];

    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(2.0);
    for (where_, what, colour) in PROBLEMS {
        column = column.push(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(8.0)
                .push(Container::new().color(colour).radius(4.0).size(8.0, 8.0))
                .push(Text::new(where_).color(DIM).size(11.0))
                .push(Text::new(what).color(Color::rgb(206, 214, 230)).size(11.0)),
        );
    }

    Container::new()
        .color(PANEL)
        .border(vieww_foundation::Border::new(EDGE, 1.0))
        .padding(EdgeInsets::all(10.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(8.0)
                .push(Text::new("PROBLEMS").color(DIM).size(10.0).bold())
                .push(column),
        )
        .into()
}

fn status_bar() -> WidgetNode {
    Container::new()
        .color(ACCENT)
        .padding(EdgeInsets::all(5.0))
        .child(
            Flex::row()
                .spacing(16.0)
                .push(Text::new("main").color(Color::WHITE).size(11.0))
                .push(Text::new("Rust").color(Color::WHITE).size(11.0))
                .push(Text::new("Ln 147, Col 22").color(Color::WHITE).size(11.0))
                .push(Text::new("26 ms/frame").color(Color::WHITE).size(11.0)),
        )
        .into()
}

/// Kept for the motion fixtures, which reuse the shell but need it framed at
/// an arbitrary size.
#[allow(dead_code)]
pub(crate) fn editor_in(bounds: Rect) -> WidgetNode {
    SizedBox::from_size(Size::new(bounds.width(), bounds.height()))
        .child(editor())
        .into()
}
