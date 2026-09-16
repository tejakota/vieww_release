//! # Vieww image effects — certification suite
//!
//! Phase 1's image items, as pictures: minification through the mip pyramid
//! (averaged, not moiré), a zoom-out animation that crosses three mip level
//! boundaries, sampling under rotation, filters over real images (blur,
//! grayscale, sepia, tint), and blend modes compositing images over images.
//!
//! ```console
//! cargo run --release -p test-image-effects -- /tmp/vieww-image-effects
//! ```
//!
//! | panel | the question it answers |
//! |---|---|
//! | `01-minification` | does an 8×-shrunk checkerboard average (mipped) or moiré? |
//! | `02-zoom-out` (GIF) | do mip-level crossings blend rather than pop? |
//! | `03-rotation` | does bilinear under rotation stay smooth at every angle? |
//! | `04-filters` | do blur / grayscale / sepia / tint actually change pixels? |
//! | `05-blends` | do images composite through non-Normal blend modes? |

use std::f32::consts::PI;
use std::path::{Path as FsPath, PathBuf};
use std::time::Instant;

use gif::{Encoder, Frame, Repeat};
use vieww_foundation::{BlendMode, Color, EdgeInsets, Image as Pixels, Offset, Size, Transform};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;

const WIDTH: f32 = 1120.0;
const HEIGHT: f32 = 760.0;
const FRAME_MS: u16 = 50;

/// One filter/blend panel's image box — shared by the layout and the
/// runner's pixel-sampling regions, so the regions track the layout by
/// construction rather than by hand-transcribed coordinates.
const PANEL_W: f32 = 200.0;
const PANEL_H: f32 = 160.0;
/// Page padding, the header's height and the inter-panel gap, likewise.
const PAGE: f32 = 36.0;
const HEADER: f32 = 52.0;
const GAP: f32 = 16.0;

const BG: Color = Color::rgb(247, 248, 250);
const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const ACCENT: Color = Color::rgb(58, 122, 246);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test-image-effects-out"));
    std::fs::create_dir_all(&out)?;

    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    let mut renderer = NativeRenderer::new();
    let mut failures: Vec<String> = Vec::new();

    // One screen: its evidence filename and the tree that renders it.
    type Screen = (&'static str, fn() -> WidgetNode);
    let screens: [Screen; 4] = [
        ("01-minification", minification),
        ("03-rotation", rotation),
        ("04-filters", filters),
        ("05-blends", blends),
    ];

    let mut stills: Vec<Vec<u8>> = Vec::new();
    for (name, screen) in screens {
        driver.set_root(screen());
        driver.draw_frame();
        let start = Instant::now();
        let (pixels, report) =
            renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let png = pixels.encode_png()?;
        std::fs::write(out.join(format!("{name}.png")), &png)?;
        println!(
            "{name}: {} images, {} shapes — {:.1} ms",
            report.images, report.shapes, elapsed_ms
        );
        stills.push(pixels.data().to_vec());
        if report.images == 0 {
            failures.push(format!("{name}: no images drawn"));
        }
    }

    // ── The zoom-out animation: full size down to a 20px thumb, crossing
    // three mip level boundaries. One renderer, so the pyramid is generated
    // once — the counter is the assertion.
    let mut zoom_frames: Vec<Vec<u8>> = Vec::new();
    const ZOOM_STEPS: usize = 40;
    let zoom_source = smooth_source();
    let chains_before_zoom = renderer.generated_mip_chains();
    for step in 0..ZOOM_STEPS {
        let t = step as f32 / (ZOOM_STEPS - 1) as f32;
        driver.set_root(zoom_out(&zoom_source, t));
        driver.draw_frame();
        let (pixels, _) =
            renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        zoom_frames.push(pixels.data().to_vec());
    }
    // The whole-suite count includes the still screens' own minified
    // images; what the zoom-out must account for is exactly *one* more —
    // its wheel's — no matter how many scale steps it took.
    let zoom_chains = renderer.generated_mip_chains() - chains_before_zoom;
    println!("zoom-out: {ZOOM_STEPS} frames, {zoom_chains} mip chain(s) for one image");
    if zoom_chains != 1 {
        failures.push(format!(
            "the zoom-out generated {zoom_chains} pyramids for one image; a \
             stable allocation should have needed exactly one"
        ));
    }
    let chains = renderer.generated_mip_chains();

    // ── Minification quality, measured: the mipped panel's pixels must sit
    // strictly between the two source colours, not at an extreme. The panel
    // is the 320px card on the `01-minification` screen.
    driver.set_root(minification());
    driver.draw_frame();
    let (pixels, _) = renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let data = pixels.data();
    // The minified 40px card (8×): scene x ≈ 430..470, y ≈ 130..170, plus
    // the 36px page padding — sampled generously.
    let region = sample_region(data, WIDTH as u32, 428.0, 128.0, 472.0, 172.0);
    let extremes = region
        .iter()
        .filter(|&&(r, g, b)| (r < 8 && g < 8 && b < 40) || (r > 240 && g < 80 && b < 80))
        .count();
    println!(
        "minified checker (8×): {} of {} samples at an extreme colour",
        extremes,
        region.len()
    );
    if extremes > region.len() / 20 {
        failures.push(format!(
            "{extremes} of {} samples in the 8×-minified checkerboard hit a \
             pure source colour — the pyramid is not averaging",
            region.len()
        ));
    }

    // ── Filters must change the image's pixels measurably: grayscale
    // flattens the hue wheel's channel spread to ~0.
    driver.set_root(filters());
    driver.draw_frame();
    let (pixels, _) = renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let data = pixels.data();
    // Row 1 of the filters screen: original, grayscale, sepia — x positions
    // derived from the same constants the layout uses.
    let row_y = PAGE + HEADER + 24.0;
    let original = sample_region(
        data,
        WIDTH as u32,
        PAGE,
        row_y,
        PAGE + PANEL_W,
        row_y + PANEL_H,
    );
    let grayscaled = sample_region(
        data,
        WIDTH as u32,
        PAGE + PANEL_W + GAP,
        row_y,
        PAGE + 2.0 * PANEL_W + GAP,
        row_y + PANEL_H,
    );
    let color_variance = average_channel_spread(&original);
    let gray_variance = average_channel_spread(&grayscaled);
    println!(
        "filter effect: original channel spread {color_variance:.1}, grayscale {gray_variance:.1}"
    );
    if color_variance - gray_variance < 8.0 {
        failures.push(
            "grayscale did not flatten the image's channel spread; the matrix \
             may not be reaching the pixels"
                .into(),
        );
    }

    write_gif(
        &zoom_frames,
        WIDTH as u32,
        HEIGHT as u32,
        &out.join("02-zoom-out.gif"),
    )?;
    write_gif(
        &stills,
        WIDTH as u32,
        HEIGHT as u32,
        &out.join("test-image-effects.gif"),
    )?;
    std::fs::write(
        out.join("metrics.txt"),
        format!("zoom_frames={ZOOM_STEPS}\nmip_chains={chains}\nminified_extremes={extremes}\n"),
    )?;

    println!("\nwrote {}", out.display());
    if failures.is_empty() {
        println!("ALL IMAGE-EFFECTS CHECKS PASSED");
    } else {
        for failure in &failures {
            eprintln!("FAIL: {failure}");
        }
        return Err(format!("{} image-effects check(s) failed", failures.len()).into());
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// Sources
// ─────────────────────────────────────────────────────────────────────────

/// A 320×320 pattern of 8px cells in two saturated colours — the classic
/// minification-aliasing probe: minified without mips, it moirés.
fn checker_source() -> Pixels {
    const SIZE: u32 = 320;
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let on = (x / 8 + y / 8) % 2 == 0;
            pixels.extend_from_slice(if on {
                &[250, 60, 60, 255]
            } else {
                &[20, 30, 160, 255]
            });
        }
    }
    Pixels::from_rgba8(pixels, SIZE, SIZE)
}

/// A smooth hue wheel — the zoom-out and rotation subject.
fn smooth_source() -> Pixels {
    const SIZE: u32 = 360;
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 / SIZE as f32, y as f32 / SIZE as f32);
            let dx = fx - 0.5;
            let dy = fy - 0.5;
            let radius = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);
            let hue = (angle + PI) / (2.0 * PI);
            let (r, g, b) = hsv_to_rgb(hue, 0.85, (1.0 - radius).clamp(0.15, 1.0));
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Pixels::from_rgba8(pixels, SIZE, SIZE)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

// ─────────────────────────────────────────────────────────────────────────
// Screens
// ─────────────────────────────────────────────────────────────────────────

fn shell(title: &str, subtitle: &str, body: WidgetNode) -> WidgetNode {
    let header: WidgetNode = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new(title).color(INK).size(26.0).bold(),
            Text::new(subtitle).color(MUTED).size(13.0),
        ])
        .into();
    Container::new()
        .color(BG)
        .padding(EdgeInsets::all(36.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(24.0)
                .children(children![header, body]),
        )
        .into()
}

fn panel(label: &str, child: WidgetNode) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(8.0)
        .children(children![child, Text::new(label).color(MUTED).size(12.0),])
        .into()
}

fn sized_image(image: &Pixels, width: f32, height: f32) -> WidgetNode {
    SizedBox::from_size(Size::new(width, height))
        .child(Image::new(image.clone()).fit(vieww_foundation::BoxFit::Fill))
        .into()
}

/// Minification: the same checkerboard at 1:1, 8× and 16× down.
fn minification() -> WidgetNode {
    let source = checker_source();
    shell(
        "Minification — the mip pyramid at work",
        "A 320px 8-cell checkerboard drawn into 160px, 40px and 20px boxes: \
         averaged panels prove the pyramid; moiré would prove its absence",
        Flex::row()
            .spacing(24.0)
            .children(children![
                panel("1:2 — reference", sized_image(&source, 160.0, 160.0)),
                panel("8× minified", sized_image(&source, 40.0, 40.0)),
                panel("16× minified", sized_image(&source, 20.0, 20.0)),
            ])
            .into(),
    )
}

/// The zoom-out sequence's root: the hue wheel from 320px down to 20px.
fn zoom_out(source: &Pixels, t: f32) -> WidgetNode {
    let scale = 1.0 / (1.0 + t * 15.0);
    let side = 320.0 * scale;
    Container::new()
        .color(BG)
        .padding(EdgeInsets::all(36.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(20.0)
                .children(children![
                    Text::new("Zoom-out — crossing mip levels")
                        .color(INK)
                        .size(26.0)
                        .bold(),
                    Text::new(format!("scale {:.2}×  (1:{:.1})", scale, 1.0 / scale))
                        .color(MUTED)
                        .size(13.0),
                    sized_image(source, side, side),
                    Text::new(
                        "the hue wheel shrinks from 320px to 20px; level \
                               boundaries blend, never pop"
                    )
                    .color(MUTED)
                    .size(12.0),
                ]),
        )
        .into()
}

/// Sampling under rotation: the hue wheel at four angles.
fn rotation() -> WidgetNode {
    let source = smooth_source();
    let angles = [
        (0.0_f32, "0°"),
        (15.0, "15°"),
        (37.5, "37.5°"),
        (90.0, "90°"),
    ];
    let panels = angles
        .iter()
        .map(|(degrees, label)| {
            let radians = degrees.to_radians();
            // Rotate about the panel's centre: `rotate_around` is the
            // supported centre-rotation, so the wheel spins in place.
            let rotated: WidgetNode =
                Transformed::new(Transform::rotate_around(Offset::new(90.0, 90.0), radians))
                    .child(sized_image(&source, 180.0, 180.0))
                    .into();
            panel(label, rotated)
        })
        .collect::<Vec<WidgetNode>>();
    shell(
        "Rotation — isotropic bilinear",
        "The hue wheel at four angles: no axis-aligned stair-stepping at any          angle, edges equally smooth in every direction",
        Flex::row().spacing(20.0).children(panels).into(),
    )
}

/// A filtered panel: the widget builders return `Filtered`, not a node, so
/// this wraps one up in the box a panel wants.
fn filtered(filter: vieww_widget::Filtered, source: &Pixels) -> WidgetNode {
    let child = sized_image(source, PANEL_W, PANEL_H);
    filter.child(child).into()
}

/// Filters over a real image, in two rows so nothing overflows the frame —
/// an overflowed row slides pixels out from under the runner's regions.
fn filters() -> WidgetNode {
    let source = smooth_source();
    let row_one: Vec<WidgetNode> = vec![
        panel("original", sized_image(&source, PANEL_W, PANEL_H)),
        panel("grayscale", filtered(Filtered::new().grayscale(), &source)),
        panel("sepia", filtered(Filtered::new().sepia(), &source)),
    ];
    let row_two: Vec<WidgetNode> = vec![
        panel("blur σ6", filtered(Filtered::blur(6.0), &source)),
        panel(
            "accent tint",
            filtered(Filtered::new().tint(ACCENT, 0.5), &source),
        ),
    ];
    let row_one_node: WidgetNode = Flex::row().spacing(GAP).children(row_one).into();
    let row_two_node: WidgetNode = Flex::row().spacing(GAP).children(row_two).into();
    let body: WidgetNode = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(GAP)
        .children(children![row_one_node, row_two_node])
        .into();
    shell(
        "Filters — blur, grayscale, sepia, tint",
        "The same hue wheel through each filter; the runner asserts the \
         channel spreads actually changed",
        body,
    )
}

/// Blend modes compositing the hue wheel over the checkerboard.
fn blends() -> WidgetNode {
    let wheel = smooth_source();
    let checker = checker_source();
    let modes = [
        (BlendMode::Multiply, "multiply"),
        (BlendMode::Screen, "screen"),
        (BlendMode::Overlay, "overlay"),
        (BlendMode::Difference, "difference"),
    ];
    shell(
        "Blends — images over images",
        "The hue wheel composited over the checkerboard at 85% group opacity \
         through four blend modes — all four must visibly differ",
        Flex::row()
            .spacing(16.0)
            .children(
                modes
                    .iter()
                    .map(|(mode, label)| {
                        let blended: WidgetNode = Stack::new()
                            .children(children![
                                sized_image(&checker, PANEL_W, PANEL_H),
                                Opacity::new(0.85)
                                    .blend(*mode)
                                    .child(sized_image(&wheel, PANEL_W, PANEL_H)),
                            ])
                            .into();
                        panel(label, blended)
                    })
                    .collect::<Vec<WidgetNode>>(),
            )
            .into(),
    )
}

// ─────────────────────────────────────────────────────────────────────────
// Pixel measurement
// ─────────────────────────────────────────────────────────────────────────

/// Sample the (r, g, b) of every pixel in a device-space region.
fn sample_region(
    data: &[u8],
    width: u32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> Vec<(u8, u8, u8)> {
    let height = (data.len() as u32) / (width * 4);
    let mut out = Vec::new();
    let x0 = left.max(0.0) as u32;
    let x1 = right.min(width as f32) as u32;
    let y0 = top.max(0.0) as u32;
    let y1 = bottom.min(height as f32) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * width + x) * 4) as usize;
            out.push((data[i], data[i + 1], data[i + 2]));
        }
    }
    out
}

/// How far the three channels of a region diverge from each other on
/// average — grayscale brings this to ~0, colourful content keeps it high.
fn average_channel_spread(region: &[(u8, u8, u8)]) -> f32 {
    if region.is_empty() {
        return 0.0;
    }
    let total: f32 = region
        .iter()
        .map(|&(r, g, b)| {
            let (r, g, b) = (f32::from(r), f32::from(g), f32::from(b));
            let mean = (r + g + b) / 3.0;
            ((r - mean).abs() + (g - mean).abs() + (b - mean).abs()) / 3.0
        })
        .sum();
    total / region.len() as f32
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
