//! The product page as a vieww tree, rendered to DOM.
//!
//! # What the browser does that the tree used to
//!
//! The canvas version built a scrollbar, measured every section so the nav
//! could land on it, ran fade-and-lift arithmetic against the scroll offset,
//! and tracked hover through a `Pressable`. All four are things a browser
//! already does, and doing them by hand is most of why that page read as
//! hand-made. Here the document scrolls, `#fragment` links land, CSS reveals on
//! scroll, and `:hover` is a selector. What is left is the page.
//!
//! The widgets are the same widgets — a tree that also rasterises, which is
//! what `examples/render.rs` still does with it. [`Styled`] carries the
//! declarations no rasteriser can honour, [`Tag`] names the element a box
//! should be, and [`Canvas`] is the seam where a real vieww application is
//! painted into the page by vieww's own renderer.

use vieww::foundation::{
    Color, EdgeInsets, FontFamily, Gradient, Offset, Shadow, TextAlign, TextStyle,
};
use vieww::prelude::*;
use vieww_platform_web_dom::{Canvas, Styled, Tag};

use crate::{highlight, url_for, Os, ASSETS};

// ─── the palette ──────────────────────────────────────────────────────────
//
// `src/app/globals.css`, converted out of oklch. The names are theirs.

/// `--background`.
const GROUND: Color = Color::rgb(0x0F, 0x0D, 0x0B);
/// `--foreground`.
const INK: Color = Color::rgb(0xF8, 0xF4, 0xF2);
/// `--muted-foreground`.
const INK_2: Color = Color::rgb(0xA6, 0x9C, 0x95);
const INK_3: Color = Color::rgb(0x78, 0x71, 0x6B);
/// `--primary`. Purple, as asked for, where their CSS has blue.
const ACCENT: Color = Color::rgb(0xB4, 0x91, 0xFF);
const LIVE: Color = Color::rgb(0x4E, 0xBE, 0x7D);

// ─── the small vocabulary this page is written in ─────────────────────────

fn sans(size: f32, color: Color) -> TextStyle {
    TextStyle::new(size)
        .family(FontFamily::Named("Geist"))
        .color(color)
        .line_height(1.55)
}

fn mono(size: f32, color: Color) -> TextStyle {
    TextStyle::new(size)
        .family(FontFamily::Named("Geist Mono"))
        .color(color)
        .line_height(1.5)
}

fn text(content: &str, style: TextStyle) -> WidgetNode {
    Text::new(content.to_string()).style(style).into()
}

/// A heading, in the semantic element it deserves.
fn heading(level: &'static str, content: &str, size: f32, align: TextAlign) -> WidgetNode {
    Tag::new(
        level,
        Text::new(content.to_string())
            .style(
                sans(size, INK)
                    .weight(FontWeight::Bold)
                    .line_height(1.07)
                    .letter_spacing(size * -0.023),
            )
            .align(align),
    )
    .into()
}

/// The column every section is measured against — `max-w-7xl`.
fn column(child: impl Into<WidgetNode>) -> WidgetNode {
    Styled::css(
        "width:100%;max-width:1280px;margin:0 auto;padding-left:20px;padding-right:20px",
        child,
    )
    .into()
}

/// Fades and lifts as it comes into view.
///
/// `animation-timeline: view()` — the browser drives it off the scroll
/// position with no observer, no listener and no per-frame work in this
/// application at all.
fn reveal(child: impl Into<WidgetNode>) -> WidgetNode {
    Styled::class("vw-reveal", child).into()
}

fn spacer(height: f32) -> WidgetNode {
    SizedBox::height(height).into()
}

fn col(spacing: f32, children: Vec<WidgetNode>) -> Flex {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .spacing(spacing)
        .children(children)
}

fn row(spacing: f32, children: Vec<WidgetNode>) -> Flex {
    Flex::row()
        .main_axis_size(MainAxisSize::Min)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .spacing(spacing)
        .children(children)
}

/// A section: the vertical rhythm, and an `id` for the nav to land on.
fn section(id: &'static str, children: Vec<WidgetNode>) -> WidgetNode {
    Tag::new(
        "section",
        Styled::class("vw-section", column(col(0.0, children))),
    )
    .id(id)
    .into()
}

/// The pill above a heading.
fn eyebrow(label: &str) -> WidgetNode {
    Styled::css(
        "display:inline-flex;align-items:center;gap:7px;border-radius:999px;\
         border:1px solid rgba(180,145,255,0.25);background:rgba(180,145,255,0.10);\
         padding:5px 12px;width:fit-content;margin:0 auto",
        row(
            7.0,
            children![
                Container::new().color(ACCENT).radius(999.0).size(5.0, 5.0),
                text(
                    label,
                    mono(10.5, ACCENT)
                        .letter_spacing(1.6)
                        .weight(FontWeight::Medium)
                ),
            ]
            .into_iter()
            .collect(),
        ),
    )
    .into()
}

/// Eyebrow, heading, lead — the block every section opens with.
fn section_head(label: &str, title: &str, lead: &str) -> WidgetNode {
    reveal(
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .spacing(0.0)
            .children(children![
                eyebrow(label),
                spacer(18.0),
                Styled::css(
                    "max-width:768px;margin:0 auto",
                    heading("h2", title, 44.0, TextAlign::Center)
                )
                .with("vw-h2"),
                spacer(16.0),
                Styled::css(
                    "max-width:672px;margin:0 auto",
                    Text::new(lead.to_string())
                        .style(sans(18.0, INK_2).line_height(1.62))
                        .align(TextAlign::Center)
                )
                .with("vw-lead"),
            ]),
    )
}

// ─── the mark ─────────────────────────────────────────────────────────────

/// The application's mark, drawn as a widget rather than shipped as an asset.
///
/// # This is `apps/viewwstudio/src/ui/brand.rs`, not an approximation
///
/// The mark exists in the repository exactly once, as fractions of a side, so
/// that the `.icns`, the splash, the title-bar glyph and this page cannot
/// drift. An earlier version of this page invented its own geometry and its
/// own colours and got every number wrong — the ground was the page's surface
/// colour instead of `#14161A`, the radii were `0.28`/`0.14` instead of
/// `0.22`/`0.08`, and the panels were in the wrong places at the wrong sizes.
/// These are `brand.rs`'s, to the digit.
///
/// ```text
/// ground   0.00, 0.00, 1.00 x 1.00   radius 0.22
/// editor   0.16, 0.20, 0.40 x 0.60   radius 0.08
/// preview  0.44, 0.32, 0.40 x 0.48   radius 0.08   (drawn on top)
/// ```
///
/// Two overlapping rounded panels on a rounded square: the editor and the
/// preview, which is what the application *is*.
fn mark(side: f32) -> WidgetNode {
    // `brand.rs`'s constants. The ground carries a ramp and a rim of light,
    // because "a flat charcoal square with two rectangles on it is a wireframe
    // of a logo" — its words.
    let ground_top = Color::rgb(0x24, 0x27, 0x2E);
    let ground = Color::rgb(0x14, 0x16, 0x1A);
    // `PANEL`, lifted from `#252A33` in the repository because six points of
    // luminance above the ground is invisible at title-bar size.
    let panel = Color::rgb(0x46, 0x4E, 0x5E);
    // `theme::ACCENT` is `PURPLE`: the ramp runs far-to-near, top to bottom.
    let accent_far = Color::rgb(0xB4, 0x91, 0xFF);
    let accent_near = Color::rgb(0x7E, 0x5C, 0xE8);

    let at = |fx: f32, fy: f32, fw: f32, fh: f32, block: Container| -> WidgetNode {
        Positioned::new()
            .left(side * fx)
            .top(side * fy)
            .child(block.radius(side * 0.08).child(SizedBox::from_size(
                vieww::foundation::Size::new(side * fw, side * fh),
            )))
            .into()
    };

    let mut base = Container::new()
        .color(ground)
        .gradient(Gradient::vertical().between(ground_top, ground))
        .radius(side * 0.22)
        .size(side, side);
    // Below about twenty points a shadow is a smudge under a glyph rather than
    // depth under an object — again, `brand.rs`'s rule and its threshold.
    if side >= 20.0 {
        base = base.shadow(Shadow::new(
            Color::rgba(0, 0, 0, 130),
            Offset::new(0.0, side * 0.05),
            side * 0.14,
        ));
    }

    // The rim is an *inset shadow*, not a border. `brand.rs` fills a rounded
    // ring inside the body's edge precisely so the panels' fractions still
    // measure from the body — a real border shrinks the padding box, and the
    // absolutely-positioned panels then resolve one point in from where the
    // repository puts them. See `.vw-mark` in `index.html`.
    Styled::class(
        "vw-mark",
        base.child(Stack::new().fit(StackFit::Expand).children(children![
            Styled::class(
                "vw-mark-a",
                at(0.16, 0.20, 0.40, 0.60, Container::new().color(panel))
            ),
            // The accent panel is the one piece of colour, so it is the one
            // that gets a ramp and a shadow.
            Styled::class(
                "vw-mark-b",
                at(
                    0.44,
                    0.32,
                    0.40,
                    0.48,
                    Container::new()
                        .color(accent_near)
                        .gradient(Gradient::vertical().between(accent_far, accent_near))
                        .shadow(Shadow::new(
                            Color::rgba(0, 0, 0, 90),
                            Offset::new(0.0, side * 0.02),
                            side * 0.07,
                        ))
                )
            ),
        ])),
    )
    .into()
}

// ─── controls ─────────────────────────────────────────────────────────────

fn button(label: &str, href: &str, filled: bool) -> WidgetNode {
    let css = if filled {
        "display:inline-flex;align-items:center;justify-content:center;gap:8px;\
         border-radius:999px;background:linear-gradient(180deg,#C0A3FF,#7E5CE8);\
         color:#0F0D0B;padding:11px 22px;text-decoration:none;font-weight:600;\
         box-shadow:0 8px 24px rgba(126,92,232,0.35),inset 0 1px 0 rgba(255,255,255,0.35);\
         white-space:nowrap"
    } else {
        "display:inline-flex;align-items:center;justify-content:center;gap:8px;\
         border-radius:999px;border:1px solid rgba(255,255,255,0.16);\
         background:rgba(255,255,255,0.04);padding:11px 22px;text-decoration:none;\
         backdrop-filter:blur(8px);white-space:nowrap"
    };
    Tag::link(
        href.to_string(),
        Styled::css(
            css,
            text(
                label,
                sans(14.5, if filled { GROUND } else { INK }).weight(FontWeight::Medium),
            ),
        )
        .with("vw-btn"),
    )
    .into()
}

// ─── the hero ─────────────────────────────────────────────────────────────

fn hero(host: Option<Os>) -> WidgetNode {
    let primary = match host {
        Some(Os::MacOs) => "Download for macOS",
        Some(Os::Windows) => "Download for Windows",
        _ => "Download for Linux",
    };
    Tag::new(
        "header",
        Styled::class(
            "vw-hero",
            Stack::new().children(children![
                // **The glow, and only the glow.**
                //
                // There was a 56px rule grid behind this too. It was texture
                // for its own sake: ruled lines running behind a logo and a
                // 72pt headline are what the eye catches instead of the words,
                // and masking it out of the middle only proved it had nowhere
                // left to be. The lit ground is the whole backdrop now.
                //
                // Inline rather than in the class because an absolutely
                // positioned child of a grid is laid out against its **grid
                // area**, not the container's padding box — so the DOM walk
                // must see `position:absolute` to know not to hand it a
                // `grid-area`. When it could not see it (the declaration was in
                // a stylesheet), the backdrop began one `padding-top` below the
                // nav and left a bar of flat black across the top of the page.
                Styled::empty("position:absolute;inset:0;pointer-events:none").with("vw-hero-glow"),
                column(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .spacing(0.0)
                        .children(children![
                            Styled::class("vw-rise vw-d0 vw-markglow", mark(84.0)),
                            spacer(24.0),
                            Styled::class("vw-rise vw-d1 vw-pill", status_pill()),
                            spacer(30.0),
                            Styled::css(
                                "max-width:980px;margin:0 auto",
                                Flex::column()
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .spacing(2.0)
                                    .children(children![
                                        Styled::class(
                                            "vw-h1 vw-rise vw-d2",
                                            heading(
                                                "h1",
                                                "A UI framework in Rust.",
                                                72.0,
                                                TextAlign::Center
                                            )
                                        ),
                                        // The second sentence in a
                                        // white-to-accent gradient clipped to
                                        // the glyphs. No rasteriser primitive
                                        // expresses this.
                                        Styled::class(
                                            "vw-h1 vw-gradient-text vw-rise vw-d3",
                                            heading(
                                                "div",
                                                "Three trees, one job each.",
                                                72.0,
                                                TextAlign::Center
                                            )
                                        ),
                                    ])
                            ),
                            spacer(26.0),
                            Styled::class(
                                "vw-rise vw-d4",
                                Styled::css(
                                    "max-width:660px;margin:0 auto",
                                    Text::new(
                                        "Widget → Element → RenderObject, with a renderer that \
                                         goes all the way down to the pixels — no wgpu, no Skia. \
                                         viewwstudio is the desktop editor and device-framed \
                                         preview for the screens you write."
                                            .to_string()
                                    )
                                    .style(sans(20.0, INK_2).line_height(1.6))
                                    .align(TextAlign::Center)
                                )
                                .with("vw-lead-hero")
                            ),
                            spacer(36.0),
                            Styled::class(
                                "vw-cta vw-rise vw-d5",
                                Flex::row()
                                    .main_axis_size(MainAxisSize::Min)
                                    .main_axis_alignment(MainAxisAlignment::Center)
                                    .spacing(12.0)
                                    .children(children![
                                        button(primary, "#download", true),
                                        button("See the architecture", "#architecture", false),
                                    ])
                            ),
                            spacer(18.0),
                            Styled::class(
                                "vw-rise vw-d5",
                                Text::new(
                                    "Apache-2.0 · no telemetry · one file and a C linker"
                                        .to_string()
                                )
                                .style(mono(11.5, INK_3))
                                .align(TextAlign::Center)
                            ),
                            spacer(52.0),
                            Styled::class("vw-rise vw-d6", stat_strip()),
                        ])
                ),
            ]),
        ),
    )
    .id("top")
    .into()
}

fn status_pill() -> WidgetNode {
    Styled::css(
        "display:inline-flex;align-items:center;gap:9px;border-radius:999px;\
         border:1px solid rgba(255,255,255,0.12);background:rgba(43,37,33,0.5);\
         backdrop-filter:blur(10px);padding:6px 14px;margin:0 auto;width:fit-content;\
         max-width:100%",
        row(
            9.0,
            children![
                Styled::class(
                    "vw-live",
                    Container::new().color(LIVE).radius(999.0).size(7.0, 7.0)
                ),
                text("v0.1.0", mono(11.5, INK)),
                text("·", mono(11.5, INK_3)),
                text(
                    "Renders on real Android and iOS hardware",
                    mono(11.5, INK_2)
                ),
            ]
            .into_iter()
            .collect(),
        ),
    )
    .into()
}

/// Four measured numbers, in a hairline grid.
fn stat_strip() -> WidgetNode {
    let cell = |value: &str, unit: &str, note: &str| -> WidgetNode {
        Styled::class(
            "vw-stat",
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(5.0)
                .children(children![
                    Flex::row()
                        .main_axis_size(MainAxisSize::Min)
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Baseline)
                        .spacing(3.0)
                        .children(children![
                            Styled::class(
                                "vw-stat-v",
                                text(value, mono(25.0, INK).weight(FontWeight::Bold))
                            ),
                            Styled::class("vw-stat-u", text(unit, mono(12.0, ACCENT))),
                        ]),
                    Styled::class(
                        "vw-stat-n",
                        Text::new(note.to_string())
                            .style(sans(11.5, INK_3))
                            .align(TextAlign::Center)
                    ),
                ]),
        )
        .into()
    };
    Styled::css(
        "max-width:800px;margin:0 auto;border:1px solid rgba(255,255,255,0.10);\
         border-radius:16px;overflow:hidden;background:rgba(255,255,255,0.08);\
         box-shadow:0 20px 50px rgba(0,0,0,0.35)",
        Styled::class(
            "vw-stats",
            Flex::row().children(children![
                cell("59.3", "fps", "on a phone, 10s"),
                cell("4.09", "ms", "median frame work"),
                cell("3 / 10", "", "render objects relaid out"),
                cell("0", "frames", "touched when idle"),
            ]),
        ),
    )
    .into()
}

// ─── the product ──────────────────────────────────────────────────────────

/// **Show the thing.** A product page for an editor that never shows the
/// editor is a press release.
fn product() -> WidgetNode {
    section(
        "studio",
        vec![
            section_head(
                "THE EDITOR",
                "The editor on the left. Your screen on the right.",
                "A file tree, a Rust buffer, and the widget tree that buffer builds — mounted at \
                 an iPhone's metrics, inside its safe area, with the Problems panel reporting on \
                 the compile that produced it.",
            ),
            spacer(52.0),
            reveal(window_frame("viewwstudio — scratch.rs", "studio.png")),
            spacer(20.0),
            Text::new(
                "Every pixel of that window is a vieww widget tree, drawn by vieww's own \
                 rasteriser."
                    .to_string(),
            )
            .style(mono(12.0, INK_3))
            .align(TextAlign::Center)
            .into(),
        ],
    )
}

/// A screenshot in application chrome — the traffic lights and a title bar.
fn window_frame(title: &str, asset: &str) -> WidgetNode {
    Styled::class(
        "vw-window",
        col(
            0.0,
            children![
                Styled::css(
                    "border-bottom:1px solid rgba(255,255,255,0.07);\
                     background:rgba(255,255,255,0.03);padding:11px 14px",
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .spacing(10.0)
                        .children(children![
                            row(
                                7.0,
                                children![
                                    dot(Color::rgb(0xFF, 0x5F, 0x57)),
                                    dot(Color::rgb(0xFE, 0xBC, 0x2E)),
                                    dot(Color::rgb(0x28, 0xC8, 0x40)),
                                ]
                                .into_iter()
                                .collect()
                            ),
                            Flexible::new(1).child(
                                Text::new(title.to_string())
                                    .style(mono(11.5, INK_3))
                                    .align(TextAlign::Center)
                            ),
                            SizedBox::width(46.0),
                        ])
                ),
                Styled::empty(format!(
                    "width:100%;aspect-ratio:1600/1000;background-image:url('{asset}');\
                     background-size:cover;background-position:top center;\
                     background-color:#0B0A09"
                )),
            ]
            .into_iter()
            .collect(),
        ),
    )
    .into()
}

fn dot(color: Color) -> WidgetNode {
    Container::new()
        .color(color)
        .radius(999.0)
        .size(11.0, 11.0)
        .into()
}

// ─── architecture ─────────────────────────────────────────────────────────

/// The idea the framework is named for, as three linked panels.
fn architecture() -> WidgetNode {
    let tree = |n: &str, name: &str, role: &str, body: &str| -> WidgetNode {
        reveal(Styled::class(
            "vw-card vw-tree",
            Container::new().padding(EdgeInsets::all(24.0)).child(col(
                10.0,
                children![
                    row(
                        10.0,
                        children![
                            Styled::css(
                                "width:26px;height:26px;border-radius:8px;\
                                 background:rgba(180,145,255,0.14);\
                                 border:1px solid rgba(180,145,255,0.30);\
                                 display:flex;align-items:center;justify-content:center",
                                text(n, mono(12.0, ACCENT).weight(FontWeight::Bold))
                            ),
                            text(
                                role,
                                mono(10.0, INK_3)
                                    .letter_spacing(1.5)
                                    .weight(FontWeight::Medium)
                            ),
                        ]
                        .into_iter()
                        .collect()
                    ),
                    text(
                        name,
                        sans(19.0, INK).weight(FontWeight::Bold).line_height(1.25)
                    ),
                    text(body, sans(14.0, INK_2)),
                ]
                .into_iter()
                .collect(),
            )),
        ))
    };
    section(
        "architecture",
        vec![
            section_head(
                "ARCHITECTURE",
                "Three trees, one job each.",
                "The split that makes a rebuild cheap: a description you throw away every frame, \
                 a tree that persists and holds your state, and geometry that is only laid out \
                 again when something actually moved.",
            ),
            spacer(52.0),
            Styled::class(
                "vw-grid3",
                Flex::column().spacing(16.0).children(children![
                    tree(
                        "1",
                        "Widget",
                        "DESCRIPTION",
                        "Cheap, immutable, and rebuilt freely. It describes what you want on \
                         screen and owns nothing — which is what makes declaring the whole tree \
                         every frame a reasonable thing to do."
                    ),
                    tree(
                        "2",
                        "Element",
                        "IDENTITY AND STATE",
                        "The tree that persists between builds. It holds your state, reconciles \
                         the new description against the old one, and decides what actually \
                         changed. A signal read in a build subscribes exactly this element."
                    ),
                    tree(
                        "3",
                        "RenderObject",
                        "LAYOUT AND PAINT",
                        "Constraints down, sizes up, and a paint pass into a scene. Relayout \
                         boundaries mean a colour change repaints without measuring anything, \
                         which is where the 3-of-10 in the numbers above comes from."
                    ),
                ]),
            )
            .into(),
        ],
    )
}

// ─── the island ───────────────────────────────────────────────────────────

/// The island's CSS box. The widget tree inside is laid out against exactly
/// this, and the bezel around it is a `border-radius` on the wrapper.
const DEMO_W: f32 = 300.0;
const DEMO_H: f32 = 520.0;

/// The live demo: DOM around it, vieww's own rasteriser inside it.
fn demo() -> WidgetNode {
    section(
        "demo",
        vec![
            section_head(
                "LIVE DEMO",
                "This is not a screenshot.",
                "The phone below is a vieww widget tree — a Signal, a Button and a Text — \
                 compiled to WebAssembly and drawn by vieww's own CPU rasteriser into a canvas \
                 in this page. Tap it.",
            ),
            spacer(48.0),
            reveal(Styled::block(
                "display:flex;justify-content:center",
                Styled::class(
                    "vw-phone",
                    Styled::css(
                        "border-radius:26px;overflow:hidden;background:#0F0D0B;max-width:100%",
                        Canvas::new("vw-demo", DEMO_W, DEMO_H, mount_demo),
                    ),
                ),
            )),
            spacer(18.0),
            Text::new(
                "A real vieww screen, running here — the only pixels on this page vieww painted."
                    .to_string(),
            )
            .style(mono(12.0, INK_3))
            .align(TextAlign::Center)
            .into(),
        ],
    )
}

/// Start a genuine vieww application on the canvas the DOM walk just created.
#[cfg(target_arch = "wasm32")]
fn mount_demo(id: &str) {
    use vieww_platform_web::{WebApp, WebSurface};
    let Ok(surface) = WebSurface::by_id(id) else {
        return;
    };
    // The canvas carries the size the page laid out for it in CSS, not in its
    // `width`/`height` attributes — which default to 300x150 and are what
    // `by_id` starts from.
    let mut surface = surface;
    let _ = surface.resize(vieww::foundation::Size::new(DEMO_W, DEMO_H));
    let _ = WebApp::new(surface)
        .background(GROUND)
        .mount_with(|driver| {
            driver.set_fonts(crate::fonts());
            let runtime = driver.elements().runtime().clone();
            let count = runtime.signal(0_i32);
            driver.set_root(crate::counter_screen(count));
        })
        .map(std::mem::forget);
}

#[cfg(not(target_arch = "wasm32"))]
fn mount_demo(_id: &str) {}

// ─── code ─────────────────────────────────────────────────────────────────

/// One source sample, tokenised and coloured.
///
/// `highlight` is the page's own five-token Rust lexer, shared with the canvas
/// build and memoised there for the same reason it is cheap here: the samples
/// are `&'static str` and never change. It returns `Span`s, and `RichText` is
/// what the DOM walk turns into one coloured `<span>` per token.
fn code_block(filename: &str, lang: &str, source: &[&str]) -> WidgetNode {
    let lines: Vec<WidgetNode> = source
        .iter()
        .enumerate()
        .map(|(index, line)| {
            row(
                0.0,
                children![
                    Styled::css(
                        "width:34px;flex:none;text-align:right;padding-right:16px;\
                         user-select:none;opacity:.45",
                        text(&(index + 1).to_string(), mono(12.5, INK_3))
                    ),
                    Flexible::new(1).child(if line.is_empty() {
                        text(" ", mono(12.5, INK_2))
                    } else {
                        RichText::new(highlight(line, 12.5)).into()
                    }),
                ]
                .into_iter()
                .collect(),
            )
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .into()
        })
        .collect();

    Styled::class(
        "vw-code",
        col(
            0.0,
            children![
                Styled::css(
                    "border-bottom:1px solid rgba(255,255,255,0.07);\
                     background:rgba(255,255,255,0.03);padding:11px 14px",
                    Flex::row()
                        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            row(
                                8.0,
                                children![
                                    dot(Color::rgb(0xFF, 0x5F, 0x57)),
                                    dot(Color::rgb(0xFE, 0xBC, 0x2E)),
                                    dot(Color::rgb(0x28, 0xC8, 0x40)),
                                    SizedBox::width(6.0),
                                    text(filename, mono(12.0, INK_2)),
                                ]
                                .into_iter()
                                .collect()
                            ),
                            Styled::css(
                                "border:1px solid rgba(255,255,255,0.10);border-radius:6px;\
                                 padding:2px 8px",
                                text(lang, mono(10.0, INK_3).letter_spacing(1.0))
                            ),
                        ])
                ),
                Styled::css("padding:18px 16px;overflow-x:auto", col(3.0, lines)),
            ]
            .into_iter()
            .collect(),
        ),
    )
    .into()
}

const RUST_SAMPLE: &[&str] = &[
    "use vieww::prelude::*;",
    "",
    "// Everything a preview needs: one function named screen.",
    "pub fn screen() -> impl Widget {",
    "    let count = use_signal(0);",
    "",
    "    Container::new()",
    "        .color(theme.colors.surface)",
    "        .padding(EdgeInsets::all(24.0))",
    "        .child(",
    "            Flex::column()",
    "                .spacing(12.0)",
    "                .children(children![",
    "                    Text::new(\"Counter\").style(theme.text.headline),",
    "                    Text::new(format!(\"Tapped {count} times\")),",
    "                    Button::new(\"Add one\").on_pressed(move || count += 1),",
    "                ]),",
    "        )",
    "}",
];

const SAY_SAMPLE: &[&str] = &[
    "// The same screen in Say, which the studio compiles",
    "// to the Rust above before anything else happens.",
    "",
    "keep a whole number called count starting at 0",
    "",
    "screen \"Home\":",
    "    a column, spaced 12, children aligned to the start:",
    "        a heading \"Counter\"",
    "        a label \"Tapped \\(count) times\"",
    "        a button \"Add one\" which when tapped:",
    "            add 1 to count",
];

fn code_section() -> WidgetNode {
    section(
        "code",
        vec![
            section_head(
                "CODE",
                "Composition by method call. No macros to learn.",
                "A widget is a cheap, immutable description of intent. You build one by calling \
                 methods on it and hand it children with children![] — and if you would rather \
                 not write Rust at all, the studio compiles Say into exactly the same file.",
            ),
            spacer(52.0),
            // **The pair reveals as one.** A scroll-driven `view()` timeline
            // is per element, and two blocks of different heights side by side
            // finish at different moments — which reads as the shorter one
            // being misaligned rather than as it arriving. Revealing the
            // container gives both children one timeline.
            reveal(Styled::class(
                "vw-grid2",
                Flex::column().spacing(16.0).children(children![
                    code_block("screen.rs", "rust", RUST_SAMPLE),
                    code_block("counter.say", "say", SAY_SAMPLE),
                ]),
            )),
        ],
    )
}

// ─── features ─────────────────────────────────────────────────────────────

fn card(eyebrow_text: &str, title: &str, body: &str) -> WidgetNode {
    reveal(Styled::class(
        "vw-card",
        Container::new().padding(EdgeInsets::all(24.0)).child(col(
            9.0,
            children![
                text(
                    eyebrow_text,
                    mono(10.0, ACCENT)
                        .letter_spacing(1.5)
                        .weight(FontWeight::Medium)
                ),
                text(
                    title,
                    sans(16.5, INK).weight(FontWeight::Bold).line_height(1.3)
                ),
                text(body, sans(14.0, INK_2)),
            ]
            .into_iter()
            .collect(),
        )),
    ))
}

fn features() -> WidgetNode {
    section(
        "features",
        vec![
            section_head(
                "WHAT IS IN IT",
                "An editor that knows it is editing a screen.",
                "Not a text box with a compile button. The studio understands what a vieww screen \
                 is, which is why it can mount one at an iPhone's metrics and tell you when it \
                 will not fit.",
            ),
            spacer(52.0),
            Styled::class(
                "vw-grid2",
                Flex::column().spacing(16.0).children(children![
                    card(
                        "PREVIEW",
                        "Three platforms, one switch",
                        "iOS, Android and Desktop frames, with safe area, dark mode and a live \
                         toggle. Switching platform changes the metrics and which theme \
                         conventions apply, with no `if platform ==` anywhere in your code."
                    ),
                    card(
                        "TOOLCHAIN",
                        "It brings its own compiler",
                        "The bundle carries the rustc it was built by and the vieww rlibs it was \
                         linked against, so host and guest are one compilation by construction. \
                         You do not need Rust installed."
                    ),
                    card(
                        "EDITOR",
                        "Rust editing, not a text box",
                        "Highlighting, folding, find and replace, multi-buffer tabs, completion \
                         through an LSP when one is on the machine, and a command palette where \
                         every action is named once."
                    ),
                    card(
                        "PANELS",
                        "Problems, Output, Run, Tasks, Rustc, Timings",
                        "Compiler diagnostics land against the line that caused them. Timings \
                         says where the last Render went, so a slow loop is a number rather than \
                         a feeling."
                    ),
                    card(
                        "SAFETY",
                        "It will not lose your buffer",
                        "Autosaved recovery files, a close that asks before dropping a modified \
                         buffer, undo history per buffer, and a session scratch that survives a \
                         restart."
                    ),
                    card(
                        "PROOF",
                        "The studio is built with vieww",
                        "Every pixel of its window is a vieww widget tree drawn by vieww's own \
                         rasteriser. The editor is the largest application built with the \
                         framework it edits."
                    ),
                ]),
            )
            .into(),
        ],
    )
}

// ─── the renderer ─────────────────────────────────────────────────────────

fn renderer() -> WidgetNode {
    section(
        "renderer",
        vec![
            section_head(
                "RENDERER",
                "No wgpu. No Skia. vieww's own, all the way down.",
                "The scene goes through vieww's render graph to a CPU rasteriser, or to \
                 vieww-gpu's Vulkan backend — verified against that rasteriser pixel for pixel. \
                 Every pixel of the three pictures below came out of it.",
            ),
            spacer(52.0),
            // Three pictures, and every pixel of all three came out of
            // `vieww-paint` — see `examples/figures.rs`. The page used to show
            // two images from `examples/fixtures` instead: a blend-mode
            // checkerboard and a frame of the animation showcase. Those are
            // *correctness* pictures — a checkerboard exists so a human can see
            // premultiplication go wrong, and the harsh red-orange-blue is
            // there to make an error obvious. On a product page they read as
            // somebody's QA output.
            reveal(Styled::class(
                "vw-grid3",
                Flex::column().spacing(16.0).children(children![
                    figure(
                        "figure-blend.png",
                        "Blend modes",
                        "five groups composited in one pass"
                    ),
                    figure(
                        "figure-depth.png",
                        "Shadow and blur",
                        "a Gaussian group behind translucent glass"
                    ),
                    figure(
                        "figure-easing.png",
                        "Curves",
                        "tickers, tweens and a spring that overshoots"
                    ),
                ]),
            )),
        ],
    )
}

fn figure(asset: &str, title: &str, note: &str) -> WidgetNode {
    WidgetNode::from(Styled::class(
        "vw-card",
        col(
            0.0,
            children![
                // `cover`, not `contain`: these are compositions that bleed to
                // their own edges, so letterboxing them inside a card would put
                // a second frame around a picture that already has one.
                Styled::empty(format!(
                    "width:100%;aspect-ratio:16/10;background-image:url('{asset}');\
                     background-size:cover;background-position:center;\
                     background-color:#0B0A09"
                )),
                Styled::css(
                    "border-top:1px solid rgba(255,255,255,0.07);padding:15px 20px",
                    col(
                        3.0,
                        children![
                            text(title, sans(14.5, INK).weight(FontWeight::Bold)),
                            text(note, mono(11.5, INK_3)),
                        ]
                        .into_iter()
                        .collect()
                    )
                ),
            ]
            .into_iter()
            .collect(),
        ),
    ))
}

// ─── platforms ────────────────────────────────────────────────────────────

fn platforms() -> WidgetNode {
    let tile = |name: &str, detail: &str, note: &str| -> WidgetNode {
        reveal(Styled::class(
            "vw-card",
            Container::new().padding(EdgeInsets::all(22.0)).child(col(
                7.0,
                children![
                    text(name, sans(16.0, INK).weight(FontWeight::Bold)),
                    text(detail, mono(11.5, ACCENT)),
                    text(note, sans(13.5, INK_2)),
                ]
                .into_iter()
                .collect(),
            )),
        ))
    };
    section(
        "platforms",
        vec![
            section_head(
                "PLATFORMS",
                "One tree. Four places to put it.",
                "The same widget tree, the same layout, the same rasteriser — so a screen that is \
                 right on the desktop is right on the phone, and the difference is metrics rather \
                 than a second implementation.",
            ),
            spacer(52.0),
            Styled::class(
                "vw-grid4",
                Flex::column().spacing(16.0).children(children![
                    tile(
                        "iOS",
                        "arm64 · real hardware",
                        "Safe areas, Cupertino conventions and the platform's own text metrics."
                    ),
                    tile(
                        "Android",
                        "arm64 · APK",
                        "Android scroll physics by default, because a list that flings wrong is \
                         the first thing anyone notices."
                    ),
                    tile(
                        "Desktop",
                        "Linux · macOS · Windows",
                        "A winit window over a raw Vulkan swapchain, or the CPU rasteriser where \
                         there is no GPU."
                    ),
                    tile(
                        "Web",
                        "wasm32 · canvas or DOM",
                        "Either the rasteriser on a canvas, or — as on this page — the same tree \
                         translated to DOM and CSS."
                    ),
                ]),
            )
            .into(),
        ],
    )
}

// ─── get started ──────────────────────────────────────────────────────────

fn get_started() -> WidgetNode {
    let step = |n: &str, title: &str, body: &str| -> WidgetNode {
        row(
            14.0,
            children![
                Styled::css(
                    "width:28px;height:28px;flex:none;border-radius:999px;\
                     background:rgba(180,145,255,0.14);border:1px solid rgba(180,145,255,0.30);\
                     display:flex;align-items:center;justify-content:center",
                    text(n, mono(12.0, ACCENT).weight(FontWeight::Bold))
                ),
                Flexible::new(1).child(col(
                    3.0,
                    children![
                        text(title, sans(15.5, INK).weight(FontWeight::Bold)),
                        text(body, sans(14.0, INK_2)),
                    ]
                    .into_iter()
                    .collect()
                )),
            ]
            .into_iter()
            .collect(),
        )
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .into()
    };
    section(
        "start",
        vec![
            section_head(
                "GET STARTED",
                "Before the first Render.",
                "The download carries the compiler and the libraries. What it cannot carry is a \
                 linker — so the one thing to install is a C toolchain.",
            ),
            spacer(52.0),
            reveal(Styled::class(
                "vw-start",
                Flex::column().spacing(20.0).children(children![
                    WidgetNode::from(Styled::class(
                        "vw-card",
                        Container::new().padding(EdgeInsets::all(26.0)).child(col(
                            20.0,
                            children![
                                step(
                                    "1",
                                    "Install a C toolchain",
                                    "build-essential on Linux, the Xcode Command Line Tools on \
                                     macOS, or the MSVC Build Tools on Windows. A preview is a \
                                     shared library, and rustc links it through cc."
                                ),
                                step(
                                    "2",
                                    "Download and open the studio",
                                    "No Rust install, no cargo, no toolchain file. The bundle \
                                     carries the rustc it was built by."
                                ),
                                step(
                                    "3",
                                    "Write a screen and press Render",
                                    "One function named screen() in the buffer is everything a \
                                     preview needs."
                                ),
                            ]
                            .into_iter()
                            .collect()
                        ))
                    )),
                    terminal(),
                ]),
            )),
        ],
    )
}

/// The install line, with a copy button. Every developer page has one, and it
/// is the one piece of the page a reader is meant to take away with them.
fn terminal() -> WidgetNode {
    Styled::class(
        "vw-term",
        col(
            0.0,
            children![
                Styled::css(
                    "border-bottom:1px solid rgba(255,255,255,0.07);\
                     background:rgba(255,255,255,0.03);padding:10px 14px",
                    Flex::row()
                        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            text("Linux · one line", mono(11.5, INK_3)),
                            Styled::class(
                                "vw-copy",
                                Styled::css(
                                    "border:1px solid rgba(255,255,255,0.12);border-radius:7px;\
                                     padding:3px 10px;cursor:pointer",
                                    text("Copy", mono(10.5, INK_2))
                                )
                            ),
                        ])
                ),
                Styled::css(
                    "padding:16px",
                    col(
                        6.0,
                        children![
                            row(
                                8.0,
                                children![
                                    text("$", mono(13.0, ACCENT)),
                                    Styled::class(
                                        "vw-cmd",
                                        text("sudo apt install build-essential", mono(13.0, INK))
                                    ),
                                ]
                                .into_iter()
                                .collect()
                            ),
                            row(
                                8.0,
                                children![
                                    text("$", mono(13.0, ACCENT)),
                                    text(
                                        "sudo dpkg -i viewwstudio-linux-x86_64.deb",
                                        mono(13.0, INK)
                                    ),
                                ]
                                .into_iter()
                                .collect()
                            ),
                        ]
                        .into_iter()
                        .collect()
                    )
                ),
            ]
            .into_iter()
            .collect(),
        ),
    )
    .into()
}

// ─── downloads ────────────────────────────────────────────────────────────

fn downloads(host: Option<Os>) -> WidgetNode {
    let asset_row = |label: &str, asset: &str, size: &str| -> WidgetNode {
        Tag::link(
            url_for(asset),
            Styled::class(
                "vw-row",
                Container::new()
                    .padding(EdgeInsets::symmetric(20.0, 15.0))
                    .child(
                        Flex::row()
                            .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                col(
                                    3.0,
                                    children![
                                        text(label, sans(14.5, INK)),
                                        text(asset, mono(11.5, ACCENT)),
                                    ]
                                    .into_iter()
                                    .collect()
                                )
                                .cross_axis_alignment(CrossAxisAlignment::Start),
                                text(size, mono(11.5, INK_3)),
                            ]),
                    ),
            ),
        )
        .into()
    };
    let primary = match host {
        Some(Os::MacOs) => "Download for macOS",
        Some(Os::Windows) => "Download for Windows",
        _ => "Download for Linux",
    };
    section(
        "download",
        vec![
            section_head(
                "DOWNLOAD",
                "One file, and a linker.",
                "Every build carries the compiler and the libraries inside it, which is why each \
                 one is around 400 MB and why there is nothing to configure after it lands.",
            ),
            spacer(38.0),
            Styled::block(
                "display:flex;justify-content:center",
                button(primary, &url_for(preferred(host).file), true),
            )
            .into(),
            spacer(36.0),
            reveal(Styled::css(
                "border:1px solid rgba(255,255,255,0.10);border-radius:16px;overflow:hidden;\
                 background:rgba(255,255,255,0.02)",
                Styled::class(
                    "vw-rows",
                    col(
                        0.0,
                        ASSETS
                            .iter()
                            .map(|asset| asset_row(asset.label, asset.file, asset.size))
                            .collect(),
                    ),
                ),
            )),
        ],
    )
}

/// The build to put on the big button.
fn preferred(host: Option<Os>) -> &'static crate::Asset {
    let wanted = host.unwrap_or(Os::Linux);
    ASSETS
        .iter()
        .find(|asset| asset.os == wanted)
        .unwrap_or(&ASSETS[0])
}

// ─── the closing call ─────────────────────────────────────────────────────

fn closing(host: Option<Os>) -> WidgetNode {
    let primary = match host {
        Some(Os::MacOs) => "Download for macOS",
        Some(Os::Windows) => "Download for Windows",
        _ => "Download for Linux",
    };
    Tag::new(
        "section",
        Styled::css(
            "padding-top:40px;padding-bottom:120px",
            column(reveal(Styled::class(
                "vw-closing",
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(0.0)
                    .children(children![
                        heading(
                            "h2",
                            "Write a screen. Press Render.",
                            40.0,
                            TextAlign::Center
                        ),
                        spacer(14.0),
                        Styled::css(
                            "max-width:560px;margin:0 auto",
                            Text::new(
                                "Apache-2.0, no telemetry, and a build that carries everything it \
                                 needs except a linker."
                                    .to_string()
                            )
                            .style(sans(17.0, INK_2))
                            .align(TextAlign::Center)
                        ),
                        spacer(28.0),
                        Flex::row()
                            .main_axis_size(MainAxisSize::Min)
                            .main_axis_alignment(MainAxisAlignment::Center)
                            .spacing(12.0)
                            .children(children![
                                button(primary, &url_for(preferred(host).file), true),
                                button("Read the guide", "#start", false),
                            ]),
                    ]),
            ))),
        ),
    )
    .into()
}

// ─── the nav ──────────────────────────────────────────────────────────────

/// Sticky, translucent, and blurred over whatever is behind it.
fn nav() -> WidgetNode {
    let link = |label: &str, href: &'static str| -> WidgetNode {
        Tag::link(
            href.to_string(),
            Styled::class(
                "vw-navlink",
                text(label, sans(14.0, INK_2).weight(FontWeight::Medium)),
            ),
        )
        .into()
    };
    Tag::new(
        "nav",
        Styled::class(
            "vw-nav",
            column(Styled::css(
                "height:64px",
                Flex::row()
                    .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![
                        Tag::link(
                            "#top",
                            Styled::css(
                                "text-decoration:none",
                                row(
                                    10.0,
                                    children![
                                        mark(22.0),
                                        text(
                                            "vieww Studio",
                                            sans(15.0, INK).weight(FontWeight::Bold)
                                        ),
                                        Styled::css(
                                            "border:1px solid rgba(255,255,255,0.12);\
                                             border-radius:999px;padding:2px 8px",
                                            text("0.1.0", mono(10.5, INK_3))
                                        ),
                                    ]
                                    .into_iter()
                                    .collect()
                                )
                            )
                        ),
                        Styled::class(
                            "vw-navlinks",
                            row(
                                28.0,
                                children![
                                    link("Editor", "#studio"),
                                    link("Architecture", "#architecture"),
                                    link("Demo", "#demo"),
                                    link("Code", "#code"),
                                    link("Platforms", "#platforms"),
                                ]
                                .into_iter()
                                .collect()
                            )
                        ),
                        button("Download", "#download", true),
                    ]),
            )),
        ),
    )
    .into()
}

// ─── the footer ───────────────────────────────────────────────────────────

fn footer() -> WidgetNode {
    let group = |title: &str, links: Vec<(&str, String)>| -> WidgetNode {
        col(
            10.0,
            std::iter::once(text(
                title,
                mono(10.0, INK_3)
                    .letter_spacing(1.6)
                    .weight(FontWeight::Medium),
            ))
            .chain(links.into_iter().map(|(label, href)| {
                Tag::link(
                    href,
                    Styled::class("vw-foot", text(label, sans(14.0, INK_2))),
                )
                .into()
            }))
            .collect(),
        )
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .into()
    };
    Tag::new(
        "footer",
        Styled::css(
            "border-top:1px solid rgba(255,255,255,0.08);padding:56px 0 44px",
            column(col(
                40.0,
                children![
                    Styled::class(
                        "vw-footgrid",
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .children(children![
                                col(
                                    12.0,
                                    children![
                                        row(
                                            10.0,
                                            children![
                                                mark(24.0),
                                                text(
                                                    "vieww Studio",
                                                    sans(15.0, INK).weight(FontWeight::Bold)
                                                ),
                                            ]
                                            .into_iter()
                                            .collect()
                                        ),
                                        Styled::css(
                                            "max-width:260px",
                                            text(
                                                "A UI framework in Rust, and the editor that \
                                                 previews it on real device metrics.",
                                                sans(13.5, INK_3)
                                            )
                                        ),
                                    ]
                                    .into_iter()
                                    .collect()
                                )
                                .cross_axis_alignment(CrossAxisAlignment::Start),
                                group(
                                    "STUDIO",
                                    vec![
                                        ("Download", "#download".into()),
                                        ("Get started", "#start".into()),
                                        ("Live demo", "#demo".into()),
                                    ]
                                ),
                                group(
                                    "FRAMEWORK",
                                    vec![
                                        ("Architecture", "#architecture".into()),
                                        ("Renderer", "#renderer".into()),
                                        ("Platforms", "#platforms".into()),
                                    ]
                                ),
                                group(
                                    "PROJECT",
                                    vec![
                                        ("Source", format!("https://github.com/{}", crate::repo())),
                                        (
                                            "Releases",
                                            format!(
                                                "https://github.com/{}/releases",
                                                crate::repo()
                                            )
                                        ),
                                        (
                                            "Issues",
                                            format!("https://github.com/{}/issues", crate::repo())
                                        ),
                                    ]
                                ),
                            ])
                    ),
                    Styled::css(
                        "border-top:1px solid rgba(255,255,255,0.06);padding-top:22px",
                        Styled::class(
                            "vw-footbottom",
                            Flex::row()
                                .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .children(children![
                                    text("vieww Studio 0.1.0 — Apache-2.0", mono(11.5, INK_3)),
                                    text(
                                        "This page is a vieww widget tree, rendered to DOM.",
                                        mono(11.5, INK_3)
                                    ),
                                ])
                        )
                    ),
                ]
                .into_iter()
                .collect(),
            )),
        ),
    )
    .into()
}

// ─── the page ─────────────────────────────────────────────────────────────

fn divider() -> WidgetNode {
    Styled::empty(
        "max-width:1280px;margin:0 auto;width:100%;height:1px;\
         background:linear-gradient(90deg,transparent,rgba(255,255,255,0.09),transparent)",
    )
    .into()
}

/// The whole thing.
#[must_use]
pub fn page(host: Option<Os>) -> WidgetNode {
    Styled::css(
        "background:#0F0D0B;color:#F8F4F2;min-height:100vh",
        col(
            0.0,
            children![
                nav(),
                hero(host),
                Tag::new(
                    "main",
                    col(
                        0.0,
                        children![
                            product(),
                            divider(),
                            architecture(),
                            divider(),
                            demo(),
                            divider(),
                            features(),
                            divider(),
                            code_section(),
                            divider(),
                            renderer(),
                            divider(),
                            platforms(),
                            divider(),
                            get_started(),
                            divider(),
                            downloads(host),
                            closing(host),
                        ]
                        .into_iter()
                        .collect()
                    )
                ),
                footer(),
            ]
            .into_iter()
            .collect(),
        ),
    )
    .into()
}

/// The mark on its own, for `examples/domdump.rs`.
#[must_use]
pub fn debug_mark() -> WidgetNode {
    mark(22.0)
}
