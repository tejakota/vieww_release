//! Visual audit sheets — one PNG per capability that was *claimed* but never
//! *shown*.
//!
//! ```console
//! cargo run -p audit-sheets --features vieww/cpu
//! ```
//!
//! The repository's own rule is that a visual change is not done until somebody
//! has looked at a picture of it. These sheets apply the same rule to the
//! progress table: every row scored 100% that no example ever rendered gets a
//! picture here, or gets marked as not working.
//!
//! Each sheet is deliberately a *proof*, not a demo — the layout is chosen so
//! that a broken capability shows up as a visibly missing or unchanged thing
//! rather than as a subtly wrong number.

use std::path::{Path, PathBuf};

use vieww::foundation::{Color, Image as Pixels, Size, TargetPlatform, TextDirection};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;
use vieww_element::scroll_anim::ScrollTimeline;
use vieww_element::{Runtime, SharedRegistry, Signal};
use vieww_render::FrameDriver;
use vieww_widget::{Filtered, Motion, RouteTransition, ThemeData};

const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const PAPER: Color = Color::rgb(247, 248, 250);
const ACCENT: Color = Color::rgb(58, 122, 246);
const GOOD: Color = Color::rgb(22, 141, 92);
const BAD: Color = Color::rgb(200, 48, 48);

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "shots/audit".into())
        .into();
    std::fs::create_dir_all(&out).expect("creating the output directory");

    sheet(&out, "01-theme.png", 900.0, 760.0, theme_sheet());
    sheet(
        &out,
        "02-route-transitions.png",
        980.0,
        900.0,
        transition_sheet(),
    );
    sheet(&out, "04-images.png", 900.0, 620.0, image_sheet());
    sheet(&out, "05-effects.png", 940.0, 900.0, effects_sheet());
    sheet(&out, "06-text-i18n.png", 900.0, 700.0, text_sheet());

    scroll_sheet(&out);
    signal_granularity_sheet(&out);
    interaction_motion_sheet(&out);
    sheet(
        &out,
        "09-motion-tokens.png",
        900.0,
        420.0,
        motion_token_sheet(),
    );
    sheet(
        &out,
        "10-text-fallback.png",
        900.0,
        560.0,
        text_fallback_sheet(),
    );
    timeline_from_handler_sheet(&out);
    shared_element_sheet(&out);

    println!("\nsheets in {}", out.display());
}

// ── Sheet 1: theming ───────────────────────────────────────────────────────
//
// The progress table scores theming 100% and no example ever set one. If
// `ThemeData` really reaches the controls, the same control row under three
// different themes must come out three visibly different colours. If it does
// not, all three rows look identical and the row is a lie.

fn theme_sheet() -> WidgetNode {
    page(
        "Theming — one control row, three themes",
        "identical widget code under ThemeData::light / dark / adaptive(Apple). \
         Different pixels = tokens reach the controls.",
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(20.0)
            .children(children![
                themed("ThemeData::light()", ThemeData::light()),
                themed("ThemeData::dark()", ThemeData::dark()),
                themed(
                    "ThemeData::adaptive(Apple, dark = false)",
                    ThemeData::adaptive(TargetPlatform::IOS, false),
                ),
            ]),
    )
}

fn themed(label: &str, data: ThemeData) -> WidgetNode {
    let surface = data.colors.surface;
    let on_surface = data.colors.on_surface;
    let corner = data.metrics.corner;

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new(label).color(MUTED).size(12.0),
            Theme::new(data).child(
                Container::new()
                    .color(surface)
                    .radius(corner)
                    .padding(EdgeInsets::all(16.0))
                    .child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(12.0)
                            .children(children![
                                Flex::row().spacing(12.0).children(children![
                                    Button::new("Primary"),
                                    Chip::new("Chip"),
                                    Badge::new(Text::new("inbox").color(on_surface).size(13.0))
                                        .count(9),
                                ]),
                                Flex::row().spacing(12.0).children(children![
                                    Checkbox::new(true),
                                    Switch::new(true),
                                    Radio::new(true),
                                    LinearProgress::new(0.6),
                                ]),
                                Text::new(format!(
                                    "corner {corner}  ·  touch target from the same tokens"
                                ))
                                .color(on_surface)
                                .size(12.0),
                            ]),
                    ),
            ),
        ])
        .into()
}

// ── Sheet 2: route transitions ─────────────────────────────────────────────
//
// `RouteTransition::wrap(screen, t, surface)` is a pure function of `t`, so a
// filmstrip at t = 0, .25, .5, .75, 1 is the whole transition, frozen. A
// transition that does nothing produces five identical cells.

fn transition_sheet() -> WidgetNode {
    page(
        "Route transitions — the same screen at five points of t",
        "RouteTransition::wrap(screen, t, surface). Five identical cells = the \
         transition does nothing.",
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(18.0)
            .children(children![
                filmstrip("Fade", RouteTransition::Fade),
                filmstrip("SlideFromEnd", RouteTransition::SlideFromEnd),
                filmstrip("SlideFromBottom", RouteTransition::SlideFromBottom),
                filmstrip(
                    "Custom (scale-ish: fade + slide down)",
                    RouteTransition::custom(|screen, t, surface| {
                        Opacity::new(t)
                            .child(
                                Transformed::translate(Offset::new(
                                    0.0,
                                    surface.height * (1.0 - t) * 0.5,
                                ))
                                .child(screen),
                            )
                            .into()
                    }),
                ),
            ]),
    )
}

fn filmstrip(label: &str, transition: RouteTransition) -> WidgetNode {
    const CELL: Size = Size {
        width: 150.0,
        height: 110.0,
    };

    let cells: Vec<WidgetNode> =
        [0.0f32, 0.25, 0.5, 0.75, 1.0]
            .iter()
            .map(|&t| {
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(4.0)
                    .children(children![
                        Text::new(format!("t = {t}")).color(MUTED).size(11.0),
                        Container::new()
                            .color(Color::rgb(233, 237, 243))
                            .radius(6.0)
                            .child(Clip::rounded(6.0).child(SizedBox::from_size(CELL).child(
                                Stack::new().push(transition.wrap(screen_card(), t, CELL),),
                            ),),),
                    ])
                    .into()
            })
            .collect();

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new(label).color(INK).size(14.0).bold(),
            Flex::row().spacing(10.0).children(cells),
        ])
        .into()
}

/// The "screen" being pushed. Deliberately full-bleed and high contrast so any
/// translation or fade is unmissable.
fn screen_card() -> WidgetNode {
    Container::new()
        .color(ACCENT)
        .padding(EdgeInsets::all(12.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(6.0)
                .children(children![
                    Text::new("Screen B").color(Color::WHITE).size(15.0).bold(),
                    Container::new()
                        .color(Color::rgba(255, 255, 255, 120))
                        .radius(3.0)
                        .child(SizedBox::from_size(Size::new(110.0, 8.0))),
                    Container::new()
                        .color(Color::rgba(255, 255, 255, 120))
                        .radius(3.0)
                        .child(SizedBox::from_size(Size::new(80.0, 8.0))),
                ]),
        )
        .into()
}

// ── Sheet 3: scroll-driven animation ───────────────────────────────────────
//
// Rendered as its own multi-frame sheet: a `ScrollTimeline` is *updated*
// between frames and each frame is written out, so the pictures show the
// header actually collapsing rather than a static tree that could have been
// hardcoded.

fn scroll_sheet(out: &Path) {
    const W: f32 = 520.0;
    const H: f32 = 360.0;

    let mut driver = FrameDriver::new(Size::new(W, H));
    let mut renderer = NativeRenderer::new();
    let runtime = driver.elements().runtime().clone();

    let timeline = ScrollTimeline::new(&runtime, 0.0, 240.0);
    let progress = timeline.progress();

    driver.elements().set_root(CollapsingHeader {
        progress: progress.clone(),
    });

    for (index, offset) in [0.0f32, 60.0, 120.0, 180.0, 240.0].iter().enumerate() {
        timeline.update(*offset);
        driver.draw_frame();
        let name = format!("03-scroll-driven-{index}-offset{offset}.png");
        write(&mut renderer, &mut driver, out, &name, W, H);
    }
}

/// A header whose height, colour and title size are all functions of scroll
/// progress — the CSS `animation-timeline: scroll()` case.
///
/// The signal is read inside `build`, which is what subscribes this element.
#[derive(Debug)]
struct CollapsingHeader {
    progress: Signal<f32>,
}

impl Widget for CollapsingHeader {
    fn debug_name(&self) -> &'static str {
        "CollapsingHeader"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let t = self.progress.get();
        let height = 160.0 - 100.0 * t;
        let title = 26.0 - 10.0 * t;
        let fade = 1.0 - t;

        Container::new()
            .color(PAPER)
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .children(children![
                        Container::new()
                            .color(Color::rgb(
                                (58.0 + 100.0 * t) as u8,
                                (122.0 - 40.0 * t) as u8,
                                (246.0 - 60.0 * t) as u8,
                            ))
                            .padding(EdgeInsets::all(16.0))
                            .child(
                                SizedBox::from_size(Size::new(W_INNER, height)).child(
                                    Flex::column()
                                        .cross_axis_alignment(CrossAxisAlignment::Start)
                                        .spacing(8.0)
                                        .children(children![
                                            Text::new("Scroll-driven header")
                                                .color(Color::WHITE)
                                                .size(title)
                                                .bold(),
                                            Opacity::new(fade).child(
                                                Text::new("subtitle fades out as progress climbs",)
                                                    .color(Color::WHITE)
                                                    .size(13.0),
                                            ),
                                            Text::new(format!("progress = {t:.2}"))
                                                .color(Color::WHITE)
                                                .size(12.0),
                                        ]),
                                ),
                            ),
                        Container::new().padding(EdgeInsets::all(16.0)).child(
                            Flex::column().spacing(8.0).children(children![
                                bar(440.0),
                                bar(400.0),
                                bar(420.0),
                                bar(360.0),
                            ]),
                        ),
                    ]),
            )
            .into()
    }
}

const W_INNER: f32 = 470.0;

vieww::widget::widget_node_from!(CollapsingHeader);

fn bar(width: f32) -> WidgetNode {
    Container::new()
        .color(Color::rgb(226, 232, 240))
        .radius(4.0)
        .child(SizedBox::from_size(Size::new(width, 14.0)))
        .into()
}

// ── Sheet 4: images ────────────────────────────────────────────────────────
//
// The previous audit's sharpest guess was that the framework had no image
// path at all, because five example apps rendered zero pictures. This sheet
// settles it: a generated bitmap through the widget, the render leaf, the
// display list and the CPU rasteriser, in four `BoxFit` modes.

fn image_sheet() -> WidgetNode {
    page(
        "Images — a real bitmap through the whole pipeline",
        "checkerboard + gradient generated in memory, drawn via Image → \
         Command::DrawImage → NativeRenderer. Four BoxFit modes in one 140x100 box.",
        Flex::row().spacing(16.0).children(children![
            fitted("Fill", BoxFit::Fill),
            fitted("Contain", BoxFit::Contain),
            fitted("Cover", BoxFit::Cover),
            fitted("None", BoxFit::None),
        ]),
    )
}

fn fitted(label: &str, fit: BoxFit) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new(label).color(INK).size(13.0).bold(),
            Container::new()
                .color(Color::rgb(226, 232, 240))
                .radius(8.0)
                .child(
                    Clip::rounded(8.0).child(
                        SizedBox::from_size(Size::new(160.0, 110.0))
                            .child(Image::new(sample_image(64, 64)).fit(fit)),
                    ),
                ),
        ])
        .into()
}

/// A checkerboard with a diagonal gradient — square, so a non-square box makes
/// every `BoxFit` mode visibly different.
fn sample_image(width: u32, height: u32) -> Pixels {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let check = ((x / 8) + (y / 8)) % 2 == 0;
            let ramp = ((x + y) as f32 / (width + height) as f32 * 255.0) as u8;
            if check {
                pixels.extend_from_slice(&[ramp, 60, 200, 255]);
            } else {
                pixels.extend_from_slice(&[250, 200_u8.saturating_sub(ramp), 60, 255]);
            }
        }
    }
    Pixels::from_rgba8(pixels, width, height)
}

// ── Sheet 5: effects ───────────────────────────────────────────────────────
//
// The one sheet whose job is to show a *failure*. `vieww-effects` ships tested
// CPU blur and colour-matrix kernels, and widget wrappers that never call
// them. Left column: the widget. Right column: the same kernel run over the
// pixels directly and shown as an `Image`. If the two disagree, the widget is
// a stub — which its own doc comments admit, and which no picture has shown.

fn effects_sheet() -> WidgetNode {
    let source = sample_image(96, 96);

    let mut blurred = source.pixels().to_vec();
    vieww::foundation::blur_rgba(&mut blurred, 96, 96, 6.0);
    let blurred = Pixels::from_rgba8(blurred, 96, 96);

    let mut grey = source.pixels().to_vec();
    vieww::foundation::apply_color_matrix_premultiplied(
        &mut grey,
        &vieww::foundation::grayscale_matrix(),
    );
    let grey = Pixels::from_rgba8(grey, 96, 96);

    page(
        "Effects — the widget against the kernel, now agreeing",
        "left: what the widget renders through Command::PushLayer's ImageFilter. \
         right: the same kernel run over the pixels directly. They must match.",
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(20.0)
            .children(children![
                effect_row(
                    "Filtered::blur(6.0) — through the render pipeline",
                    Filtered::blur(6.0)
                        .child(
                            SizedBox::from_size(Size::new(120.0, 120.0))
                                .child(Image::new(source.clone())),
                        )
                        .into(),
                    "blur_rgba(sigma 6) on the pixels directly",
                    SizedBox::from_size(Size::new(120.0, 120.0))
                        .child(Image::new(blurred))
                        .into(),
                ),
                effect_row(
                    "FilterChain(grayscale) — the crate that was a stub",
                    vieww_effects::FilterChain::new()
                        .filter(vieww_effects::Filter::grayscale())
                        .child(
                            SizedBox::from_size(Size::new(120.0, 120.0))
                                .child(Image::new(source.clone())),
                        )
                        .into(),
                    "apply_color_matrix(grayscale) on the pixels directly",
                    SizedBox::from_size(Size::new(120.0, 120.0))
                        .child(Image::new(grey))
                        .into(),
                ),
                effect_row(
                    "Filtered::blur(4.0).tint(WHITE, 0.45) — frosted",
                    Filtered::blur(4.0)
                        .tint(Color::WHITE, 0.45)
                        .child(
                            SizedBox::from_size(Size::new(120.0, 120.0))
                                .child(Image::new(source.clone())),
                        )
                        .into(),
                    "Filtered::new().sepia() — one composed matrix",
                    Filtered::new()
                        .sepia()
                        .child(
                            SizedBox::from_size(Size::new(120.0, 120.0))
                                .child(Image::new(source.clone())),
                        )
                        .into(),
                ),
            ]),
    )
}

fn effect_row(
    widget_label: &str,
    widget: WidgetNode,
    kernel_label: &str,
    kernel: WidgetNode,
) -> WidgetNode {
    Flex::row()
        .spacing(24.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(children![
            labelled(widget_label, BAD, widget),
            labelled(kernel_label, GOOD, kernel),
        ])
        .into()
}

fn labelled(label: &str, tint: Color, child: WidgetNode) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new(label).color(tint).size(12.0).bold(),
            Container::new()
                .color(Color::rgb(233, 237, 243))
                .radius(8.0)
                .padding(EdgeInsets::all(8.0))
                .child(child),
        ])
        .into()
}

// ── Sheet 6: text and i18n ─────────────────────────────────────────────────
//
// The text rows were scored from the hand-rolled `shaping`/`bidi` modules. The
// path that actually renders goes through `Paragraph`, which is cosmic-text —
// rustybuzz shaping, unicode-bidi reordering, real line breaking. This sheet
// renders the scripts the embedded subset covers, so the difference between
// "scored 25%" and "what the pixels do" is visible.

fn text_sheet() -> WidgetNode {
    page(
        "Text — what the shaping path actually renders",
        "Paragraph → cosmic-text (rustybuzz + unicode-bidi). Embedded subset \
         covers Latin, Hebrew and Arabic; anything else falls back.",
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(14.0)
            .children(children![
                specimen(
                    "Latin",
                    "The quick brown fox — fi ffl ligatures, kerning: AVA To."
                ),
                specimen("Hebrew (RTL)", "שלום עולם — טקסט עברי"),
                specimen("Arabic (RTL, joining)", "مرحبا بالعالم — نص عربي"),
                specimen(
                    "Bidi: Latin host, RTL run, digits",
                    "Total שלום 1234 due — numbers stay LTR inside an RTL run",
                ),
                rtl_specimen(
                    "Arabic host with a Latin run (Directionality::Rtl)",
                    "مرحبا vieww 2026"
                ),
                specimen("Devanagari (not in the subset)", "नमस्ते दुनिया"),
                specimen("CJK (not in the subset)", "你好世界 こんにちは"),
                Text::new(
                    "Wrapping: this line is deliberately long so that the line \
                     breaker has to find a break point inside it rather than at \
                     an obvious one, which is the case a naive breaker gets wrong.",
                )
                .color(INK)
                .size(13.0),
            ]),
    )
}

fn specimen(label: &str, body: &str) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(2.0)
        .children(children![
            Text::new(label).color(MUTED).size(11.0),
            Text::new(body).color(INK).size(18.0),
        ])
        .into()
}

fn rtl_specimen(label: &str, body: &str) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(2.0)
        .children(children![
            Text::new(label).color(MUTED).size(11.0),
            Directionality::new(TextDirection::Rtl).child(
                Container::new()
                    .color(Color::rgb(237, 241, 247))
                    .radius(4.0)
                    .padding(EdgeInsets::all(8.0))
                    .child(
                        SizedBox::from_size(Size::new(700.0, 30.0))
                            .child(Text::new(body).color(INK).size(18.0)),
                    ),
            ),
        ])
        .into()
}

// ── Sheet 7: signal granularity ────────────────────────────────────────────
//
// The open question from the first audit: does reading a signal during tree
// *construction* (as the motion showcase's drag and timeline panels do)
// subscribe the root, subscribe nothing, or subscribe the reader?
//
// `Runtime::track` only records a read when a build is on the tracking stack.
// A read outside one is dropped. This sheet renders that: two panels driven by
// the same signal — one reading it in `Widget::build`, one reading it at
// construction time — with the signal changed between frames. If the theory
// holds, only the left panel moves, and the showcase's panels are inert.

fn signal_granularity_sheet(out: &Path) {
    const W: f32 = 640.0;
    const H: f32 = 280.0;

    let mut driver = FrameDriver::new(Size::new(W, H));
    let mut renderer = NativeRenderer::new();
    let runtime: Runtime = driver.elements().runtime().clone();

    let position = runtime.signal(0.0f32);

    // Built once, exactly as `showcase_screen` does: the construction-time read
    // happens here, before `set_root`, outside any build.
    let root = granularity_screen(&position);
    driver.elements().set_root(root);

    driver.draw_frame();
    println!(
        "signal sheet — frame 1: {} subscribers, {} pending",
        runtime.subscriber_count(),
        runtime.pending_count()
    );
    write(
        &mut renderer,
        &mut driver,
        out,
        "07-signals-before.png",
        W,
        H,
    );

    position.set(300.0);
    println!(
        "signal sheet — after set(300): {} pending elements",
        runtime.pending_count()
    );
    driver.draw_frame();
    write(
        &mut renderer,
        &mut driver,
        out,
        "07-signals-after.png",
        W,
        H,
    );
}

fn granularity_screen(position: &Signal<f32>) -> WidgetNode {
    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(20.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(18.0)
                .children(children![
                    Text::new("Signal read in build() vs at construction")
                        .color(INK)
                        .size(16.0)
                        .bold(),
                    Text::new(
                        "same signal, set to 300 between the two frames. \
                         A handle that did not move was never subscribed.",
                    )
                    .color(MUTED)
                    .size(12.0),
                    track_label("read inside Widget::build", GOOD),
                    // Reads the signal in `build`, so it subscribes.
                    WidgetNode::from(Handle {
                        position: position.clone()
                    }),
                    track_label("read at construction time (the showcase's pattern)", BAD),
                    // Reads the signal *now*, outside any build. Nothing records it.
                    track(position.get()),
                ]),
        )
        .into()
}

fn track_label(label: &str, tint: Color) -> WidgetNode {
    Text::new(label).color(tint).size(12.0).bold().into()
}

/// The track plus a handle at `x`.
fn track(x: f32) -> WidgetNode {
    Stack::new()
        .push(
            Container::new()
                .color(Color::rgb(226, 232, 240))
                .radius(2.0)
                .child(SizedBox::from_size(Size::new(560.0, 6.0))),
        )
        .push(
            Positioned::new().left(x.min(540.0)).top(-7.0).child(
                Container::new()
                    .color(ACCENT)
                    .radius(10.0)
                    .child(SizedBox::square(20.0)),
            ),
        )
        .into()
}

#[derive(Debug)]
struct Handle {
    position: Signal<f32>,
}

impl Widget for Handle {
    fn debug_name(&self) -> &'static str {
        "Handle"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        track(self.position.get())
    }
}

vieww::widget::widget_node_from!(Handle);

// ── Sheet 8: motion started from a handler ─────────────────────────────────
//
// The first audit read "no route from a widget callback to `Tickers`" as a
// property of the motion system. It is narrower than that: `Animation<T>`,
// `ScrollController` and `NavigatorController` all take `&mut Tickers` **once**
// at setup and expose `&self` methods afterwards, so a handler can start, stop
// and retarget them. Only `Timeline::play` needs the collection at the moment
// it fires, and its `TimelineHandle` can cancel but not replay.
//
// This sheet is the proof: an `Animation` attached at setup, started by a
// closure standing in for a tap handler, sampled every four frames.

fn interaction_motion_sheet(out: &Path) {
    use std::time::Duration;
    use vieww::animation::{Tickers, Tween};
    use vieww_element::Animation;

    const W: f32 = 640.0;
    const H: f32 = 200.0;

    let mut driver = FrameDriver::new(Size::new(W, H));
    let mut renderer = NativeRenderer::new();
    let runtime: Runtime = driver.elements().runtime().clone();
    let mut tickers = Tickers::new();

    // Setup: created and attached once, exactly where a real app would do it.
    let slide = Animation::new(
        &runtime,
        Tween::new(0.0f32, 540.0),
        Duration::from_millis(400),
    );
    slide.attach(&mut tickers);

    let position = slide.signal();
    driver.elements().set_root(MotionStrip { position });

    driver.draw_frame();
    write(
        &mut renderer,
        &mut driver,
        out,
        "08-handler-motion-0.png",
        W,
        H,
    );

    // "Tap." Nothing here has a `&mut Tickers` — only `&self` on the animation.
    let start = Duration::from_millis(0);
    let handler = {
        let slide = slide.clone();
        move || slide.forward(start)
    };
    handler();

    for (index, ms) in [80u64, 160, 240, 400].iter().enumerate() {
        tickers.advance(Duration::from_millis(*ms));
        driver.draw_frame();
        write(
            &mut renderer,
            &mut driver,
            out,
            &format!("08-handler-motion-{}.png", index + 1),
            W,
            H,
        );
    }
}

#[derive(Debug)]
struct MotionStrip {
    position: Signal<f32>,
}

impl Widget for MotionStrip {
    fn debug_name(&self) -> &'static str {
        "MotionStrip"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let x = self.position.get();
        Container::new()
            .color(PAPER)
            .padding(EdgeInsets::all(20.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(16.0)
                    .children(children![
                        Text::new("Animation started from a handler, not from setup")
                            .color(INK)
                            .size(15.0)
                            .bold(),
                        Text::new(format!("Animation::forward takes &self — x = {x:.1}"))
                            .color(MUTED)
                            .size(12.0),
                        track(x),
                    ]),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(MotionStrip);

// ── Sheet 9: motion tokens ─────────────────────────────────────────────────
//
// The M3-Expressive gap: springs existed and were good, and no theme could
// reach them. `ThemeData` now carries a `Motion` alongside `colors` and
// `metrics`, with the spatial/effects split that keeps an opacity spring from
// overshooting past 1.0 and clamping.

fn motion_token_sheet() -> WidgetNode {
    page(
        "Motion tokens — the same controls, two motion schemes",
        "ThemeData now carries Motion beside colors and metrics. Controls read \
         it through Animated::themed and Pressable::themed.",
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(16.0)
            .children(children![
                scheme_row("Motion::standard()", Motion::standard()),
                scheme_row("Motion::expressive()", Motion::expressive()),
            ]),
    )
}

fn scheme_row(label: &str, motion: Motion) -> WidgetNode {
    let damping = |preset: vieww::animation::SpringPreset| preset.spec().damping_ratio;
    let overshoots = |d: f32| {
        if d < 1.0 {
            "overshoots"
        } else {
            "settles flat"
        }
    };

    Container::new()
        .color(Color::WHITE)
        .radius(10.0)
        .padding(EdgeInsets::all(16.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(8.0)
                .children(children![
                    Text::new(label).color(INK).size(15.0).bold(),
                    token_line(
                        "spatial_default",
                        damping(motion.spatial_default),
                        overshoots(damping(motion.spatial_default)),
                        if damping(motion.spatial_default) < 1.0 {
                            GOOD
                        } else {
                            MUTED
                        },
                    ),
                    token_line(
                        "effects_default",
                        damping(motion.effects_default),
                        // The invariant: never under-damped, in either scheme.
                        "never overshoots — an opacity past 1.0 clamps",
                        GOOD,
                    ),
                    Text::new(format!(
                        "durations {}ms / {}ms / {}ms",
                        motion.duration_short.as_millis(),
                        motion.duration_medium.as_millis(),
                        motion.duration_long.as_millis(),
                    ))
                    .color(MUTED)
                    .size(12.0),
                ]),
        )
        .into()
}

fn token_line(name: &str, damping: f32, note: &str, tint: Color) -> WidgetNode {
    Flex::row()
        .spacing(10.0)
        .children(children![
            Container::new()
                .color(tint)
                .radius(3.0)
                .child(SizedBox::from_size(Size::new(6.0, 16.0))),
            Text::new(format!("{name}  damping {damping:.2}"))
                .color(INK)
                .size(13.0),
            Text::new(note).color(MUTED).size(12.0),
        ])
        .into()
}

// ── Sheet 10: text fallback ────────────────────────────────────────────────
//
// The worst failure the audit found: an uncovered script rendered as *nothing*.
// Not tofu — blank space with correct advances, which reads as a layout bug.
// `RenderText` now strokes its own box per `.notdef` glyph, so a missing font
// looks like a missing font.

fn text_fallback_sheet() -> WidgetNode {
    page(
        "Missing glyphs — blank space, or a box that says so",
        "The embedded subset is Latin + Hebrew + Arabic. DejaVu's .notdef inks \
         nothing, so uncovered scripts used to vanish silently.",
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(14.0)
            .children(children![
                specimen("Covered — Latin", "The quick brown fox"),
                specimen("Covered — Arabic", "مرحبا بالعالم"),
                specimen("Covered — Hebrew", "שלום עולם"),
                specimen("Not covered — CJK", "你好世界 こんにちは"),
                specimen("Not covered — Devanagari", "नमस्ते दुनिया"),
                specimen(
                    "Mixed: covered text with an uncovered word",
                    "Total 世界 due"
                ),
                Text::new(
                    "Each box is one glyph the font could not supply, drawn by \
                     RenderText rather than by the font. GlyphRun::missing_count \
                     puts the same fact in a number.",
                )
                .color(MUTED)
                .size(12.0),
            ]),
    )
}

// ── Sheet 11: a timeline played from a handler ─────────────────────────────
//
// `Timeline::play` needs `&mut Tickers` at the moment it fires, and a widget
// callback has none — so the motion showcase had to delete a "Reveal" button.
// `Timeline::attach` takes the collection once at setup and hands back a
// `TimelinePlayer` a handler can own.

fn timeline_from_handler_sheet(out: &Path) {
    use std::time::Duration;
    use vieww::animation::Tickers;
    use vieww_element::TimelineBuilder;

    const W: f32 = 620.0;
    const H: f32 = 320.0;

    let mut driver = FrameDriver::new(Size::new(W, H));
    let mut renderer = NativeRenderer::new();
    let runtime: Runtime = driver.elements().runtime().clone();
    let mut tickers = Tickers::new();

    let rows: Vec<Signal<f32>> = (0..8).map(|_| runtime.signal(0.0f32)).collect();

    // Setup — the one place with a `&mut Tickers`.
    let reveal = TimelineBuilder::new()
        .stagger(Duration::from_millis(45), |s| {
            rows.iter().fold(s, |b, row| b.action(row, 0.0, 1.0))
        })
        .build()
        .attach(&mut tickers);

    driver
        .elements()
        .set_root(StaggerRows { rows: rows.clone() });
    driver.draw_frame();
    write(
        &mut renderer,
        &mut driver,
        out,
        "11-timeline-0-before-tap.png",
        W,
        H,
    );

    // "Tap." A `'static` closure holding nothing but the player.
    let on_pressed: Box<dyn Fn()> = Box::new({
        let reveal = reveal.clone();
        move || reveal.play()
    });
    on_pressed();

    let mut t = Duration::ZERO;
    for (index, frames) in [6usize, 12, 40].iter().enumerate() {
        for _ in 0..*frames {
            t += Duration::from_millis(16);
            tickers.advance(t);
        }
        driver.draw_frame();
        write(
            &mut renderer,
            &mut driver,
            out,
            &format!("11-timeline-{}-after-tap.png", index + 1),
            W,
            H,
        );
    }
}

/// Rows that each read their own opacity signal **inside `build`**, which is
/// what subscribes them — the bug sheet 7 found in the motion showcase.
#[derive(Debug)]
struct StaggerRows {
    rows: Vec<Signal<f32>>,
}

impl Widget for StaggerRows {
    fn debug_name(&self) -> &'static str {
        "StaggerRows"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let rows: Vec<WidgetNode> = self
            .rows
            .iter()
            .map(|opacity| {
                WidgetNode::from(StaggerRow {
                    opacity: opacity.clone(),
                })
            })
            .collect();

        Container::new()
            .color(PAPER)
            .padding(EdgeInsets::all(20.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(8.0)
                    .children(children![
                        Text::new("Timeline played from a tap handler")
                            .color(INK)
                            .size(16.0)
                            .bold(),
                        Text::new("Timeline::attach at setup, TimelinePlayer::play in the closure")
                            .color(MUTED)
                            .size(12.0),
                        Flex::column().spacing(6.0).children(rows),
                    ]),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(StaggerRows);

#[derive(Debug)]
struct StaggerRow {
    opacity: Signal<f32>,
}

impl Widget for StaggerRow {
    fn debug_name(&self) -> &'static str {
        "StaggerRow"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Opacity::new(self.opacity.get().clamp(0.0, 1.0))
            .child(
                Container::new()
                    .color(ACCENT)
                    .radius(4.0)
                    .child(SizedBox::from_size(Size::new(520.0, 22.0))),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(StaggerRow);

// ── Sheet 12: shared-element transitions ───────────────────────────────────
//
// The marquee item on both mobile platforms, and the one thing route
// transitions structurally could not do: an element that exists on two screens
// moving between them instead of being replaced.
//
// This sheet is the whole protocol, not a mock-up of it. The grid screen is
// laid out and painted so its tagged card reports a real rectangle; the detail
// screen is laid out and painted so its hero reports another; then the flight
// interpolates between the two measured rectangles. Nothing here is a
// hard-coded coordinate — if `Measured` reported the wrong box, the card would
// fly to the wrong place and the sheet would show it.

fn shared_element_sheet(out: &Path) {
    use vieww_element::SharedFlight;

    const W: f32 = 460.0;
    const H: f32 = 400.0;
    const TAG: &str = "photo-3";

    // ── Capture: screen A ─────────────────────────────────────────────
    let registry = SharedRegistry::new();
    let mut driver = FrameDriver::new(Size::new(W, H));
    let mut renderer = NativeRenderer::new();

    driver
        .elements()
        .set_root(grid_screen(&registry, TAG, false));
    driver.draw_frame();
    let from = registry.snapshot();
    write(
        &mut renderer,
        &mut driver,
        out,
        "12-shared-0-grid.png",
        W,
        H,
    );

    // ── Capture: screen B ─────────────────────────────────────────────
    registry.clear();
    let mut detail = FrameDriver::new(Size::new(W, H));
    detail
        .elements()
        .set_root(detail_screen(&registry, TAG, false));
    detail.draw_frame();
    let to = registry.snapshot();
    write(
        &mut renderer,
        &mut detail,
        out,
        "12-shared-4-detail.png",
        W,
        H,
    );

    let flight = SharedFlight::between(&from, &to);
    let tag = vieww_element::SharedTag::new(TAG);
    println!(
        "shared element — {} flying; {:?} → {:?}",
        flight.len(),
        flight.get(&tag).map(|f| f.from),
        flight.get(&tag).map(|f| f.to),
    );

    // ── Fly ───────────────────────────────────────────────────────────
    //
    // Both originals hidden, the copy over the top. The grid fades out and the
    // detail fades in underneath it, which is the ordinary route transition
    // doing its half of the job.
    for (index, t) in [0.25f32, 0.5, 0.75].iter().enumerate() {
        let mut frame = FrameDriver::new(Size::new(W, H));
        let registry = SharedRegistry::new();
        let stage = Stack::new()
            .push(Opacity::new(1.0 - t).child(grid_screen(&registry, TAG, true)))
            .push(Opacity::new(*t).child(detail_screen(&registry, TAG, true)))
            .push(flight.overlay(*t, |_| hero_card()));

        frame.elements().set_root(stage);
        frame.draw_frame();
        write(
            &mut renderer,
            &mut frame,
            out,
            &format!("12-shared-{}-t{:.2}.png", index + 1, t),
            W,
            H,
        );
    }
}

/// The card that flies. Built once at its **destination** size and scaled — see
/// `Flight::scale_at` for why not rebuilt per frame.
fn hero_card() -> WidgetNode {
    Container::new()
        .color(ACCENT)
        .radius(10.0)
        .child(SizedBox::from_size(Size::new(HERO_W, HERO_H)))
        .into()
}

const HERO_W: f32 = 412.0;
const HERO_H: f32 = 150.0;

/// Screen A: a grid of cards, one of them tagged.
fn grid_screen(registry: &SharedRegistry, tag: &str, hidden: bool) -> WidgetNode {
    use vieww_element::SharedElement;

    let cells: Vec<WidgetNode> = (1..=6)
        .map(|index| {
            let card: WidgetNode = Container::new()
                .color(if index == 3 {
                    ACCENT
                } else {
                    Color::rgb(206, 216, 230)
                })
                .radius(8.0)
                .child(SizedBox::from_size(Size::new(120.0, 80.0)))
                .into();

            if index == 3 {
                SharedElement::new(tag, registry)
                    .hidden(hidden)
                    .child(card)
                    .into()
            } else {
                card
            }
        })
        .collect();

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(24.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(14.0)
                .children(children![
                    Text::new("Gallery").color(INK).size(20.0).bold(),
                    Grid::columns(3).gap(12.0).children(cells),
                ]),
        )
        .into()
}

/// Screen B: the detail view, whose hero carries the same tag.
fn detail_screen(registry: &SharedRegistry, tag: &str, hidden: bool) -> WidgetNode {
    use vieww_element::SharedElement;

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(24.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(14.0)
                .children(children![
                    SharedElement::new(tag, registry)
                        .hidden(hidden)
                        .child(hero_card()),
                    Text::new("Photo 3").color(INK).size(20.0).bold(),
                    Text::new(
                        "The hero above carries the same tag as the third \
                               card in the gallery."
                    )
                    .color(MUTED)
                    .size(13.0),
                ]),
        )
        .into()
}

// ── Sheet plumbing ─────────────────────────────────────────────────────────

fn page(title: &str, subtitle: &str, body: impl Into<WidgetNode>) -> WidgetNode {
    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(24.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(6.0)
                .children(children![
                    Text::new(title).color(INK).size(20.0).bold(),
                    Text::new(subtitle).color(MUTED).size(12.0),
                    SizedBox::from_size(Size::new(1.0, 10.0)),
                    body.into(),
                ]),
        )
        .into()
}

fn sheet(out: &Path, name: &str, width: f32, height: f32, root: WidgetNode) {
    let mut driver = FrameDriver::new(Size::new(width, height));
    let mut renderer = NativeRenderer::new();
    driver.elements().set_root(root);
    driver.draw_frame();
    write(&mut renderer, &mut driver, out, name, width, height);
}

fn write(
    renderer: &mut NativeRenderer,
    driver: &mut FrameDriver,
    out: &Path,
    name: &str,
    width: f32,
    height: f32,
) {
    let (png, report) = renderer
        .render_to_png(driver.scene(), width as u32, height as u32, Color::WHITE)
        .expect("rasterising through vieww's own renderer");
    let path = out.join(name);
    std::fs::write(&path, png).expect("writing the PNG");
    println!(
        "  {name}: {} shapes, {} glyph runs ({} glyphs), {} clips, {} shadows, {} layers",
        report.shapes,
        report.glyph_runs,
        report.glyphs,
        report.clips,
        report.shadows,
        report.layers
    );
}
