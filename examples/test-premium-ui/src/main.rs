//! # Vieww Premium UI — visual integration test
//!
//! This is intentionally one screen rather than a catalogue of isolated demos.
//! The question is not "does a rounded rectangle exist?"; the question is
//! whether the framework can compose the things that a premium application
//! needs into a coherent, dense, animated interface without visual breakage.
//!
//! The test renders a deterministic sequence through Vieww's own native
//! rasterizer and writes `test-premium-ui.gif` plus a first/last PNG. Review the
//! GIF first. The numbers printed at the end are supporting evidence, not the
//! acceptance criterion: the picture is the acceptance criterion.
//!
//! ```console
//! cargo run --release -p test-premium-ui -- /tmp/vieww-premium-test
//! ```
//!
//! The sequence exercises, in one composition:
//! - retained-style widget composition: Flex / Stack / Positioned / Grid-like cards
//! - typography and hierarchy: multiple sizes, weights and dense metadata
//! - surfaces: gradients, rounded corners, borders, opacity, shadows
//! - effects: clipped groups, blur/backdrop filtering, transforms
//! - controls: buttons, chips, checkbox, switch, slider, progress
//! - data: line chart, bar chart, animated KPI values
//! - vectors: custom-drawn brand mark and iconography
//! - animation: card lift, metric pulse, chart motion, active navigation, reveal
//! - layout stress: everything is sized to a real app shell rather than a toy panel
//!
//! It deliberately uses the CPU/native rasterizer because this test is for the
//! framework's complete visual surface. Backend parity is evaluated separately.

use std::f32::consts::PI;
use std::path::{Path as FsPath, PathBuf};
use std::time::{Duration, Instant};

use gif::{Encoder, Frame, Repeat};
use vieww_foundation::{
    Border, Color, EdgeInsets, Gradient, Offset, Path, Shadow, Size, Sketchbook,
};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::{PaintWith, Painting};

const WIDTH: f32 = 1120.0;
const HEIGHT: f32 = 720.0;
const FRAMES: usize = 56;
const FRAME_MS: u16 = 45;

const BG: Color = Color::rgb(10, 13, 20);
const SURFACE: Color = Color::rgb(18, 23, 33);
const SURFACE_2: Color = Color::rgb(22, 28, 41);
const INK: Color = Color::rgb(241, 245, 249);
const MUTED: Color = Color::rgb(151, 164, 184);
const ACCENT: Color = Color::rgb(104, 145, 255);
const CYAN: Color = Color::rgb(73, 211, 210);
const VIOLET: Color = Color::rgb(178, 112, 255);
const MINT: Color = Color::rgb(94, 224, 163);
const WHITE_12: Color = Color::rgba(255, 255, 255, 12);
const WHITE_20: Color = Color::rgba(255, 255, 255, 20);
const BLACK_90: Color = Color::rgba(0, 0, 0, 90);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test-premium-ui-out"));
    std::fs::create_dir_all(&out)?;

    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    let mut renderer = NativeRenderer::new();
    let mut frames = Vec::with_capacity(FRAMES);
    let mut total = Duration::ZERO;
    let mut worst = Duration::ZERO;

    vieww_render::overflow::forget_reported();

    for i in 0..FRAMES {
        let t = i as f32 / (FRAMES - 1) as f32;
        let time = t * 2.0 * PI;

        driver.set_root(root(t, time));
        driver.draw_frame_at(Duration::from_secs_f32(t * 2.8));

        let start = Instant::now();
        let (pixels, _) =
            renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        let elapsed = start.elapsed();
        total += elapsed;
        worst = worst.max(elapsed);
        frames.push(pixels.data().to_vec());
    }

    let overflows = vieww_render::overflow::reported();
    let moving = frames.windows(2).filter(|w| w[0] != w[1]).count();

    write_gif(
        &frames,
        WIDTH as u32,
        HEIGHT as u32,
        &out.join("test-premium-ui.gif"),
    )?;
    write_png_from_rgba(
        &frames[0],
        WIDTH as u32,
        HEIGHT as u32,
        &out.join("test-premium-ui-first.png"),
    )?;
    write_png_from_rgba(
        frames.last().expect("frames are non-empty"),
        WIDTH as u32,
        HEIGHT as u32,
        &out.join("test-premium-ui-last.png"),
    )?;

    println!("Vieww premium visual integration test");
    println!("  frames:      {FRAMES}");
    println!("  moving:      {moving}/{count}", count = FRAMES - 1);
    println!(
        "  render mean: {:.2} ms",
        total.as_secs_f64() * 1000.0 / FRAMES as f64
    );
    println!("  render worst: {:.2} ms", worst.as_secs_f64() * 1000.0);
    println!("  overflows:   {overflows}");
    println!(
        "  gif:         {}",
        out.join("test-premium-ui.gif").display()
    );

    std::fs::write(
        out.join("metrics.txt"),
        format!(
            "frames={FRAMES}\nmoving={moving}\nrender_mean_ms={:.3}\nrender_worst_ms={:.3}\noverflows={overflows}\n",
            total.as_secs_f64() * 1000.0 / FRAMES as f64,
            worst.as_secs_f64() * 1000.0,
        ),
    )?;

    if overflows > 0 {
        return Err(format!("layout overflow reported in {overflows} frame(s)").into());
    }
    if moving < FRAMES / 2 {
        return Err("visual sequence barely changed; animation appears dead".into());
    }

    Ok(())
}

fn root(t: f32, time: f32) -> WidgetNode {
    let phase = (t * 3.0).fract();
    let lift = (ease_out_back((t * 1.7).fract()) * 1.0).min(1.0);
    let pulse = 0.5 + 0.5 * time.sin();
    let sweep = 0.5 + 0.5 * (time * 0.9).sin();
    let active = ((t * 4.0).floor() as usize) % 4;

    Stack::new()
        .push(
            Positioned::fill().child(
                Container::new()
                    .size(WIDTH, HEIGHT)
                    .gradient(
                        Gradient::radial(Offset::new(0.52, 0.28), 0.88).with_stops(&[
                            (0.0, Color::rgb(26, 33, 52)),
                            (0.46, BG),
                            (1.0, Color::rgb(6, 8, 13)),
                        ]),
                    )
                    .child(SizedBox::from_size(Size::new(WIDTH, HEIGHT))),
            ),
        )
        .push(
            Positioned::fill().child(
                // A large soft blurred blob gives the glass surfaces something
                // meaningful to filter instead of filtering a flat colour.
                Filtered::blur(18.0).child(
                    Container::new()
                        .size(340.0, 260.0)
                        .alignment(Alignment::CENTER)
                        .gradient(Gradient::radial_fill().with_stops(&[
                            (0.0, Color::rgba(104, 145, 255, 80)),
                            (0.55, Color::rgba(178, 112, 255, 28)),
                            (1.0, Color::rgba(0, 0, 0, 0)),
                        ]))
                        .child(SizedBox::from_size(Size::new(340.0, 260.0))),
                ),
            ),
        )
        .push(
            Positioned::new()
                .left(18.0)
                .top(18.0)
                .child(sidebar(active, pulse)),
        )
        .push(
            Positioned::new()
                .left(248.0)
                .top(18.0)
                .child(main_content(t, time, lift, sweep)),
        )
        .push(
            Positioned::new()
                .left(800.0 + lift * 12.0)
                .top(520.0 - lift * 8.0)
                .child(spotlight_card(phase, pulse)),
        )
        .push(
            Positioned::new()
                .left(950.0)
                .top(590.0)
                .child(notification_badge(active)),
        )
        .push(
            Positioned::new().left(0.0).top(0.0).child(
                // A restrained vignette tests a full-surface translucent layer.
                Container::new()
                    .size(WIDTH, HEIGHT)
                    .gradient(
                        Gradient::radial(Offset::new(0.5, 0.5), 0.70)
                            .with_stops(&[(0.72, Color::rgba(0, 0, 0, 0)), (1.0, BLACK_90)]),
                    )
                    .child(SizedBox::from_size(Size::new(WIDTH, HEIGHT))),
            ),
        )
        .into()
}

fn sidebar(active: usize, pulse: f32) -> WidgetNode {
    let items = ["Overview", "Analytics", "Activity", "Settings"];
    let mut body = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(9.0)
        .push(brand_header())
        .push(SizedBox::from_size(Size::new(194.0, 14.0)));

    for (index, label) in items.iter().enumerate() {
        let selected = index == active;
        let icon = match index {
            0 => icons::chevron_right(),
            1 => icons::chevron_up(),
            2 => icons::chevron_down(),
            _ => icons::chevron_left(),
        };
        body = body.push(nav_item(label, icon, selected, pulse));
    }

    body = body
        .push(SizedBox::from_size(Size::new(194.0, 22.0)))
        .push(section_label("WORKSPACE"))
        .push(workspace_row("Northstar", MINT, "PRO"))
        .push(workspace_row("Studio", VIOLET, ""));

    body = body
        .push(SizedBox::from_size(Size::new(194.0, 112.0)))
        .push(
            Container::new()
                .decoration(
                    BoxDecoration::new()
                        .gradient(Gradient::vertical().with_stops(&[
                            (0.0, Color::rgba(104, 145, 255, 36)),
                            (1.0, Color::rgba(178, 112, 255, 10)),
                        ]))
                        .radius(16.0)
                        .border(Border::new(WHITE_20, 1.0)),
                )
                .padding(EdgeInsets::all(14.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(8.0)
                        .children(children![
                            Text::new("Premium surface").color(INK).size(12.0).bold(),
                            Text::new("All primitives in one composition")
                                .color(MUTED)
                                .size(10.0),
                            LinearProgress::new(0.74)
                                .thickness(5.0)
                                .label("test coverage"),
                        ]),
                ),
        );

    Container::new()
        .color(Color::rgba(13, 18, 27, 228))
        .radius(20.0)
        .padding(EdgeInsets::all(12.0))
        .decoration(BoxDecoration::new().shadow(Shadow::new(
            Color::rgba(0, 0, 0, 120),
            Offset::new(0.0, 12.0),
            26.0,
        )))
        .child(body)
        .into()
}

fn brand_header() -> WidgetNode {
    Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .spacing(10.0)
        .children(children![
            brand_mark(34.0),
            Text::new("VIEWW").color(INK).size(14.0).bold()
        ])
        .into()
}

fn brand_mark(size: f32) -> WidgetNode {
    Painting::sized(
        Size::square(size),
        PaintWith::new(move |book: &mut Sketchbook, s: Size| {
            let center = Offset::new(s.width / 2.0, s.height / 2.0);
            let r = s.width * 0.46;
            book.circle(
                center,
                r,
                Gradient::radial_fill().with_stops(&[(0.0, CYAN), (0.46, ACCENT), (1.0, VIOLET)]),
            );

            let mut p = Path::new();
            p.move_to(Offset::new(10.0, s.height * 0.50));
            p.cubic_to(
                Offset::new(s.width * 0.24, s.height * 0.20),
                Offset::new(s.width * 0.72, s.height * 0.20),
                Offset::new(s.width - 8.0, s.height * 0.52),
            );
            p.cubic_to(
                Offset::new(s.width * 0.70, s.height * 0.78),
                Offset::new(s.width * 0.30, s.height * 0.78),
                Offset::new(10.0, s.height * 0.50),
            );
            p.close();
            book.fill(p, Color::rgba(255, 255, 255, 92));
        }),
    )
    .into()
}

fn nav_item(label: &str, icon: IconData, selected: bool, pulse: f32) -> WidgetNode {
    let fill = if selected {
        Color::rgba(104, 145, 255, 42 + (pulse * 18.0) as u8)
    } else {
        Color::rgba(255, 255, 255, 0)
    };
    Container::new()
        .color(fill)
        .radius(11.0)
        .padding(EdgeInsets::symmetric(10.0, 9.0))
        .size(194.0, 35.0)
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(9.0)
                .children(children![
                    Icon::new(icon)
                        .size(14.0)
                        .color(if selected { ACCENT } else { MUTED }),
                    Text::new(label)
                        .size(11.0)
                        .color(if selected { INK } else { MUTED })
                        .bold(),
                    Flexible::expanded(1).child(SizedBox::shrink()),
                    if selected {
                        WidgetNode::from(Container::new().color(ACCENT).radius(3.0).size(3.0, 18.0))
                    } else {
                        WidgetNode::from(SizedBox::shrink())
                    },
                ]),
        )
        .into()
}

fn workspace_row(name: &str, color: Color, tag: &str) -> WidgetNode {
    let avatar = Avatar::initials(name).size(24.0).color(color);
    Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .spacing(8.0)
        .children(children![
            avatar,
            Text::new(name).color(INK).size(10.0),
            Flexible::expanded(1).child(SizedBox::shrink()),
            if tag.is_empty() {
                WidgetNode::from(SizedBox::shrink())
            } else {
                WidgetNode::from(Chip::new(tag).selected(true))
            },
        ])
        .into()
}

fn section_label(label: &str) -> WidgetNode {
    Text::new(label)
        .color(Color::rgb(98, 112, 132))
        .size(8.0)
        .bold()
        .into()
}

fn main_content(t: f32, time: f32, lift: f32, sweep: f32) -> WidgetNode {
    let progress = 0.48 + 0.28 * sweep;
    let kpi_bump = 1.0 + 0.035 * time.sin();
    let chart = animated_chart(sweep);

    let header = Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(children![
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(3.0)
                .children(children![
                    Text::new("Overview").color(INK).size(27.0).bold(),
                    Text::new("Your interface, treated as a product — not a widget collection")
                        .color(MUTED)
                        .size(10.0),
                ]),
            Flexible::expanded(1).child(SizedBox::shrink()),
            Chip::new("LIVE").selected(true),
            SizedBox::from_size(Size::new(10.0, 1.0)),
            Avatar::initials("Teja Kota").size(32.0).color(ACCENT),
        ]);

    let kpis = Flex::row().spacing(12.0).children(children![
        metric_card(
            "Reach",
            format_compact(184_200.0 * kpi_bump),
            "+12.8%",
            CYAN,
            0.84
        ),
        metric_card(
            "Engagement",
            format_percent(68.0 + 4.0 * sweep),
            "+4.2%",
            VIOLET,
            0.68
        ),
        metric_card(
            "Conversion",
            format_percent(12.0 + 2.5 * sweep),
            "+1.7%",
            MINT,
            0.53
        ),
    ]);

    let content_cards = Flex::row().spacing(12.0).children(children![
        chart_card(chart, progress, lift),
        control_card(t),
    ]);

    Container::new()
        .size(858.0, HEIGHT - 36.0)
        .padding(EdgeInsets::all(22.0))
        .decoration(
            BoxDecoration::new()
                .color(Color::rgba(12, 17, 25, 210))
                .radius(22.0)
                .border(Border::new(Color::rgba(255, 255, 255, 16), 1.0))
                .shadow(Shadow::new(
                    Color::rgba(0, 0, 0, 110),
                    Offset::new(0.0, 14.0),
                    34.0,
                )),
        )
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(16.0)
                .children(children![header, kpis, content_cards]),
        )
        .into()
}

fn metric_card(title: &str, value: String, delta: &str, _color: Color, fill: f32) -> WidgetNode {
    Container::new()
        .size(260.0, 172.0)
        .decoration(
            BoxDecoration::new()
                .color(SURFACE)
                .radius(16.0)
                .border(Border::new(WHITE_12, 1.0))
                .shadow(Shadow::new(
                    Color::rgba(0, 0, 0, 55),
                    Offset::new(0.0, 8.0),
                    18.0,
                )),
        )
        .padding(EdgeInsets::all(15.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(7.0)
                .children(children![
                    Text::new(title).color(MUTED).size(9.0).bold(),
                    Text::new(value).color(INK).size(24.0).bold(),
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .spacing(7.0)
                        .children(children![
                            Chip::new(delta).selected(true),
                            Flexible::expanded(1).child(SizedBox::shrink()),
                            CircularProgress::new(fill).size(28.0).thickness(3.0),
                        ]),
                ]),
        )
        .into()
}

fn animated_chart(progress: f32) -> WidgetNode {
    let raw = [
        28.0, 39.0, 33.0, 48.0, 44.0, 56.0, 52.0, 64.0, 58.0, 72.0, 69.0, 81.0,
    ];
    let data: Vec<f32> = raw
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let wave = ((progress * 2.0 + i as f32 * 0.11) * PI).sin() * 1.8;
            value + wave
        })
        .collect();
    LineChart::new(data)
        .color(ACCENT)
        .line_width(2.4)
        .show_dots(true)
        .size(Size::new(460.0, 198.0))
        .label("Views over twelve periods")
        .into()
}

fn chart_card(chart: WidgetNode, progress: f32, lift: f32) -> WidgetNode {
    let mini_bar = BarChart::new(vec![32.0, 50.0, 41.0, 62.0, 57.0, 74.0])
        .color(CYAN)
        .bar_gap(5.0)
        .size(Size::new(180.0, 110.0))
        .label("Secondary channel activity");

    Container::new()
        .size(530.0, 386.0)
        .padding(EdgeInsets::all(16.0))
        .decoration(
            BoxDecoration::new()
                .color(SURFACE_2)
                .radius(16.0)
                .border(Border::new(WHITE_12, 1.0))
                .shadow(Shadow::new(
                    Color::rgba(0, 0, 0, 45),
                    Offset::new(0.0, 8.0),
                    18.0,
                )),
        )
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(7.0)
                .children(children![
                    Flex::row().children(children![
                        Text::new("Performance").color(INK).size(12.0).bold(),
                        Flexible::expanded(1).child(SizedBox::shrink()),
                        Text::new("LAST 30 DAYS").color(MUTED).size(8.0).bold(),
                    ]),
                    chart,
                    Flex::row().children(children![
                        Text::new(format!("{}% active", (progress * 100.0) as u32))
                            .color(CYAN)
                            .size(9.0)
                            .bold(),
                        Flexible::expanded(1).child(SizedBox::shrink()),
                        Text::new("moving target").color(MUTED).size(8.0),
                    ]),
                    WidgetNode::from(
                        Transformed::translate(Offset::new(0.0, -lift * 3.0)).child(mini_bar),
                    ),
                ]),
        )
        .into()
}

fn control_card(t: f32) -> WidgetNode {
    let toggle = (t * 2.0).sin() > 0.0;
    let check = (t * 1.3).cos() > -0.4;
    let slider = 0.35 + 0.45 * (0.5 + 0.5 * (t * 1.4).sin());

    let glass = Filtered::blur(8.0)
        .with_backdrop()
        .tint(Color::rgb(55, 76, 112), 0.28)
        .child(
            Container::new()
                .color(Color::rgba(255, 255, 255, 10))
                .radius(15.0)
                .border(Border::new(Color::rgba(255, 255, 255, 28), 1.0))
                .size(232.0, 116.0)
                .padding(EdgeInsets::all(12.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(6.0)
                        .children(children![
                            Text::new("Frosted glass").color(INK).size(10.0).bold(),
                            Text::new("Backdrop filter + tint + border")
                                .color(MUTED)
                                .size(8.0),
                            Flex::row().children(children![
                                Button::new("Primary").on_pressed(|| {}),
                                SizedBox::from_size(Size::new(8.0, 1.0)),
                                Button::new("Secondary")
                                    .style(ButtonStyle::Outlined)
                                    .on_pressed(|| {}),
                            ]),
                        ]),
                ),
        );

    Container::new()
        .size(270.0, 388.0)
        .padding(EdgeInsets::all(14.0))
        .decoration(
            BoxDecoration::new()
                .color(SURFACE)
                .radius(16.0)
                .border(Border::new(WHITE_12, 1.0)),
        )
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(10.0)
                .children(children![
                    Text::new("Controls & surfaces")
                        .color(INK)
                        .size(12.0)
                        .bold(),
                    control_row(
                        "Signal enabled",
                        Switch::new(toggle).label("Signal enabled")
                    ),
                    control_row("Auto-sync", Checkbox::new(check).label("Auto-sync")),
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(3.0)
                        .children(children![
                            Text::new("Intensity").color(MUTED).size(8.0),
                            Slider::new(slider)
                                .label("Intensity")
                                .range(0.0, 100.0)
                                .divisions(10),
                        ]),
                    LinearProgress::new(slider)
                        .thickness(6.0)
                        .label("operation progress"),
                    glass,
                ]),
        )
        .into()
}

fn control_row(label: &str, control: impl Into<WidgetNode>) -> WidgetNode {
    Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(children![
            Text::new(label).color(INK).size(9.0),
            Flexible::expanded(1).child(SizedBox::shrink()),
            control.into(),
        ])
        .into()
}

fn spotlight_card(phase: f32, pulse: f32) -> WidgetNode {
    let glow = Color::rgba(104, 145, 255, (30.0 + 26.0 * pulse) as u8);
    Container::new()
        .size(285.0, 172.0)
        .padding(EdgeInsets::all(14.0))
        .decoration(
            BoxDecoration::new()
                .gradient(
                    Gradient::vertical()
                        .with_stops(&[(0.0, glow), (1.0, Color::rgba(104, 145, 255, 5))]),
                )
                .radius(16.0)
                .border(Border::new(Color::rgba(104, 145, 255, 65), 1.0))
                .shadow(Shadow::new(
                    Color::rgba(104, 145, 255, 40),
                    Offset::new(0.0, 8.0),
                    20.0,
                )),
        )
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(7.0)
                .children(children![
                    Text::new("NOW TESTING")
                        .color(Color::rgb(134, 173, 255))
                        .size(8.0)
                        .bold(),
                    Text::new("Composed interaction surface")
                        .color(INK)
                        .size(13.0)
                        .bold(),
                    Text::new("Layout + effects + data + controls + motion")
                        .color(MUTED)
                        .size(8.0),
                    Flex::row().children(children![
                        Chip::new("CLIP").selected(phase > 0.20),
                        Chip::new("BLUR").selected(phase > 0.45),
                        Chip::new("MOTION").selected(phase > 0.70),
                    ]),
                ]),
        )
        .into()
}

fn notification_badge(active: usize) -> WidgetNode {
    let avatar = Badge::new(Avatar::initials("V W").size(38.0).color(VIOLET)).count(3);
    let text = match active {
        0 => "Everything healthy",
        1 => "Analytics refreshed",
        2 => "New activity",
        _ => "Settings updated",
    };
    Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .spacing(9.0)
        .children(children![avatar, Text::new(text).color(MUTED).size(8.0)])
        .into()
}

fn format_percent(value: f32) -> String {
    format!("{value:.1}%")
}

fn format_compact(value: f32) -> String {
    if value >= 100_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

fn ease_out_back(t: f32) -> f32 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

fn write_gif(
    frames: &[Vec<u8>],
    width: u32,
    height: u32,
    path: &FsPath,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder = Encoder::new(file, width as u16, height as u16, &[])?;
    encoder.set_repeat(Repeat::Infinite)?;
    for rgba in frames {
        let mut frame = Frame::from_rgba_speed(width as u16, height as u16, &mut rgba.clone(), 10);
        frame.delay = FRAME_MS / 10;
        encoder.write_frame(&frame)?;
    }
    Ok(())
}

fn write_png_from_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    path: &FsPath,
) -> Result<(), Box<dyn std::error::Error>> {
    // Use the `image` crate only in this example; the framework itself does not
    // need it for rendering. Keeping this conversion here makes the integration
    // test self-contained and leaves the core dependency graph untouched.
    let image = ::image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or("invalid RGBA frame dimensions")?;
    image.save(path)?;
    Ok(())
}
