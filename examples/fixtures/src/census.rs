//! GPU coverage census, CPU/GPU parity and CPU/GPU timing over the whole
//! fixture gallery.
//!
//! `cargo run --release -p fixtures -- <outdir> --census`
//!
//! # What this answers that nothing in the workspace answers today
//!
//! `vieww_gpu::ScenePlan::is_complete` is the gate that decides whether a frame
//! can go to the GPU at all, and no measurement has ever been taken of how many
//! *real screens* pass it. This walks every fixture in the gallery and reports:
//!
//! 1. how many plan complete today,
//! 2. which `Unsupported` kind blocks how many screens — and, greedily, which
//!    order of fixing them unblocks the most per unit of work,
//! 3. for every complete fixture, whether the GPU's pixels match the CPU
//!    rasterizer's, held to the same bound `vieww-hal`'s own suite uses,
//! 4. CPU time versus GPU time for those fixtures on this machine.
//!
//! # The parity bound, and why it is not "every pixel"
//!
//! The two rasterizers are allowed to disagree in exactly one place. The CPU
//! path computes analytic coverage per scanline; the GPU path triangulates and
//! lets the hardware sample, with no multisampling. Along a shape's boundary
//! those produce different values *by design*, and on a high-contrast edge the
//! difference reaches most of the channel's range — so a whole-image `max`
//! reports 200-plus for a frame that is perfectly correct, and reports it for
//! every frame, which makes the number useless.
//!
//! So this uses `crates/vieww-hal/tests/vulkan_scene.rs`'s own rule, copied
//! rather than reinvented: a pixel counts as a mismatch only if it differs
//! **and** the *reference* image has no coverage transition within one pixel of
//! it. An edge is a property of the picture, not of the diff. What is left is
//! the class of failure that matters — a shape misplaced, dropped, mis-coloured
//! or mirrored — and that cannot hide inside an interior.
//!
//! Both numbers are printed. The strict one is context; the interior one is the
//! verdict. Any fixture with a non-zero interior count also gets a triptych
//! written out — CPU, GPU, and the amplified difference, side by side — because
//! the next question after "they disagree" is always "show me".
//!
//! # The timing caveat, stated rather than buried
//!
//! `SceneRenderer::render_planned` renders **and reads the frame back into host
//! memory**, because that is what a headless parity harness needs. A real
//! window would present the swapchain image and never pay that copy. So the GPU
//! column is an upper bound on GPU frame cost. Treat a GPU number that merely
//! ties the CPU as a win — and treat any GPU number at all as meaningless until
//! the interior parity count for that fixture is zero, because a renderer that
//! draws less is faster for a reason nobody wants.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vieww_gpu::{Planner, Unsupported};
use vieww_hal::vulkan::{SceneRenderer, VulkanDevice};
use vieww_hal::Device as _;
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;

use crate::runner::Fixture;

const WARMUP: usize = 1;
const RUNS: usize = 5;

/// How far apart two neighbouring reference pixels must be before this counts
/// as a coverage transition. `vulkan_scene.rs`'s value, unchanged.
const EDGE_THRESHOLD: u8 = 8;

/// Everything measured about one fixture.
struct Row {
    name: &'static str,
    commands: usize,
    complete: bool,
    triangles: usize,
    draw_calls: usize,
    unsupported: BTreeMap<Unsupported, usize>,
    cpu: Duration,
    gpu: Option<Duration>,
    /// Largest single-channel difference anywhere, edges included. Context.
    worst_any: Option<u8>,
    /// Channels differing by more than one step of 1/255, edges included.
    over_one_any: Option<usize>,
    /// Pixels that differ **and** sit in a flat region of the reference. The
    /// strict number.
    interior: Option<usize>,
    /// Pixels that differ by more than [`BAND_TOLERANCE`] **and** sit further
    /// than [`BAND_RADIUS`] pixels from any transition in the reference. The
    /// verdict — see `compare`.
    beyond_band: Option<usize>,
}

pub(crate) fn run(fixtures: &[Fixture]) {
    let out = out_dir();
    println!("\n=== vieww GPU coverage census ===\n");

    // Declared before the renderer so the renderer, which borrows it, is
    // dropped first.
    let device = match VulkanDevice::new() {
        Ok(device) => {
            let info = device.info();
            println!(
                "device   : {} [{}] via {}",
                info.name, info.device_type, info.backend
            );
            Some(device)
        }
        Err(error) => {
            println!("device   : NONE ({error})");
            println!("           coverage is still reported; parity and timing are skipped.");
            None
        }
    };
    let mut gpu = match device.as_ref() {
        Some(device) => match SceneRenderer::new(device) {
            Ok(renderer) => Some(renderer),
            Err(error) => {
                println!("pipeline : FAILED to build ({error})");
                None
            }
        },
        None => None,
    };
    println!("fixtures : {}", fixtures.len());
    println!("diffs to : {}\n", out.display());

    let mut rows = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        rows.push(measure(fixture, gpu.as_mut(), &out));
    }

    print_table(&rows);
    print_coverage(&rows);
    print_parity(&rows, &out);
    print_timing(&rows);
}

/// Where to write diff images: the first argument that is not a flag, matching
/// how `main` reads its own output directory.
fn out_dir() -> PathBuf {
    let dir: PathBuf = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .unwrap_or_else(|| "census-out".to_owned())
        .into();
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn measure(fixture: &Fixture, gpu: Option<&mut SceneRenderer>, out: &Path) -> Row {
    vieww_render::overflow::forget_reported();

    let mut driver = FrameDriver::new(fixture.size);
    driver.elements().set_root((fixture.build)());
    driver.draw_frame();

    let width = fixture.size.width as u32;
    let height = fixture.size.height as u32;

    let mut planner = Planner::new();
    let plan = planner.plan(driver.scene(), fixture.size.width, fixture.size.height);

    // ---- CPU, best of RUNS after a warm-up, exactly as `runner::run` times it.
    let mut cpu_renderer = NativeRenderer::new();
    for _ in 0..WARMUP {
        cpu_renderer
            .render_to_pixels(driver.scene(), width, height, fixture.background)
            .expect("cpu warm-up");
    }
    let mut cpu = Duration::MAX;
    let mut cpu_pixels = Vec::new();
    for _ in 0..RUNS {
        let start = Instant::now();
        let (pixels, _report) = cpu_renderer
            .render_to_pixels(driver.scene(), width, height, fixture.background)
            .expect("cpu timed render");
        cpu = cpu.min(start.elapsed());
        cpu_pixels = pixels.data().to_vec();
    }

    // ---- GPU, only for a plan the backend will accept.
    let mut gpu_time = None;
    let mut worst_any = None;
    let mut over_one_any = None;
    let mut interior = None;
    let mut beyond_band = None;
    if plan.is_complete() {
        if let Some(renderer) = gpu {
            let mut ok = true;
            for _ in 0..WARMUP {
                if let Err(error) =
                    renderer.render_planned(&planner, &plan, width, height, fixture.background)
                {
                    println!("  {}: gpu warm-up failed: {error}", fixture.name);
                    ok = false;
                    break;
                }
            }
            if ok {
                let mut best = Duration::MAX;
                let mut last = Vec::new();
                for _ in 0..RUNS {
                    let start = Instant::now();
                    match renderer.render_planned(
                        &planner,
                        &plan,
                        width,
                        height,
                        fixture.background,
                    ) {
                        Ok(pixels) => {
                            best = best.min(start.elapsed());
                            last = pixels;
                        }
                        Err(error) => {
                            println!("  {}: gpu render failed: {error}", fixture.name);
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    gpu_time = Some(best);
                    let (worst, over, inside, band) =
                        compare(&last, &cpu_pixels, width as usize, height as usize);
                    worst_any = Some(worst);
                    over_one_any = Some(over);
                    interior = Some(inside);
                    beyond_band = Some(band);
                    if inside > 0 {
                        write_triptych(out, fixture.name, &cpu_pixels, &last, width, height);
                    }
                }
            }
        }
    }

    Row {
        name: fixture.name,
        commands: driver.scene().len(),
        complete: plan.is_complete(),
        triangles: plan.triangle_count(),
        draw_calls: plan.draw_call_count(),
        unsupported: plan.unsupported.clone(),
        cpu,
        gpu: gpu_time,
        worst_any,
        over_one_any,
        interior,
        beyond_band,
    }
}

/// How far from a reference transition a disagreement still counts as the
/// geometry-edge band. See `compare`.
const BAND_RADIUS: i64 = 2;
/// A reference transition, for the band rule: any step at all above rounding.
const BAND_EDGE: u8 = 2;
/// A disagreement, for the band rule.
const BAND_TOLERANCE: u8 = 8;

/// `(worst difference anywhere, channels over 1/255 anywhere, strict interior
/// mismatches, mismatches beyond the geometry-edge band)`.
///
/// # Two interior rules, and why the verdict is the second
///
/// The third number is `vulkan_scene.rs`'s `interior_mismatches` rule: a
/// pixel differing by more than 1/255 that is not within one pixel of a >8
/// step in the reference. It is kept and printed.
///
/// It over-reports one known class, and only one: the GPU fills geometry
/// **without edge antialiasing**. Where the CPU's antialiased ramp is low
/// contrast — a grey ring inside a translucent layer, a shallow-angle edge
/// whose ramp spans two pixels, an edge softened by a blur — the reference
/// step between neighbours is under 8, so the strict rule calls it flat.
///
/// The fourth number separates that class out: a pixel differing by more than
/// 8 that is more than two pixels from *any* step (> 2) in the reference.
/// Every compositor feature — layers, blends, filters, masks, shadows,
/// gradients, images — shows up there if it is wrong, because those errors
/// move flat regions; missing edge AA cannot. Geometry edge antialiasing is
/// a tracked GPU gap (`docs/GPU-RENDERER-STATUS.md`), not a pass.
fn compare(gpu: &[u8], cpu: &[u8], w: usize, h: usize) -> (u8, usize, usize, usize) {
    if gpu.len() != cpu.len() {
        let n = gpu.len().max(cpu.len());
        return (255, n, n, n);
    }
    let channel = |buffer: &[u8], index: usize, c: usize| buffer[index * 4 + c];
    let mut worst = 0u8;
    let mut over = 0usize;
    for (a, b) in gpu.iter().zip(cpu.iter()) {
        let diff = a.abs_diff(*b);
        worst = worst.max(diff);
        if diff > 1 {
            over += 1;
        }
    }

    let differs =
        |index: usize| (0..4).any(|c| channel(gpu, index, c).abs_diff(channel(cpu, index, c)) > 1);
    // A transition in the *reference*: this is where antialiasing lives, and
    // the one place two rasterizers are allowed to disagree.
    let reference_edge = |x: usize, y: usize| {
        let index = y * w + x;
        (-1i64..=1).any(|dy| {
            (-1i64..=1).any(|dx| {
                let nx = x as i64 + dx;
                let ny = y as i64 + dy;
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    return false;
                }
                let neighbour = ny as usize * w + nx as usize;
                (0..4).any(|c| {
                    channel(cpu, index, c).abs_diff(channel(cpu, neighbour, c)) > EDGE_THRESHOLD
                })
            })
        })
    };

    let mut inside = 0usize;
    for y in 0..h {
        for x in 0..w {
            if differs(y * w + x) && !reference_edge(x, y) {
                inside += 1;
            }
        }
    }

    let near_transition = |x: usize, y: usize| {
        let index = y * w + x;
        (-BAND_RADIUS..=BAND_RADIUS).any(|dy| {
            (-BAND_RADIUS..=BAND_RADIUS).any(|dx| {
                let nx = x as i64 + dx;
                let ny = y as i64 + dy;
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    return false;
                }
                let neighbour = ny as usize * w + nx as usize;
                (0..4).any(|c| {
                    channel(cpu, index, c).abs_diff(channel(cpu, neighbour, c)) > BAND_EDGE
                })
            })
        })
    };
    let mut band = 0usize;
    for y in 0..h {
        for x in 0..w {
            let index = y * w + x;
            let d = (0..4)
                .map(|c| channel(gpu, index, c).abs_diff(channel(cpu, index, c)))
                .max()
                .unwrap_or(0);
            if d > BAND_TOLERANCE && !near_transition(x, y) {
                band += 1;
            }
        }
    }
    (worst, over, inside, band)
}

/// CPU | GPU | amplified difference, side by side, as a binary PPM.
///
/// PPM rather than PNG so this needs no encoder and therefore no new
/// dependency in a crate that exists to measure rendering. Any viewer opens it;
/// `magick x.ppm x.png` converts one.
fn write_triptych(out: &Path, name: &str, cpu: &[u8], gpu: &[u8], w: u32, h: u32) {
    let (w, h) = (w as usize, h as usize);
    let total = w * 3 + 8;
    let mut body = Vec::with_capacity(total * h * 3);
    for y in 0..h {
        for panel in 0..3 {
            for x in 0..w {
                let i = (y * w + x) * 4;
                match panel {
                    0 => body.extend_from_slice(&cpu[i..i + 3]),
                    1 => body.extend_from_slice(&gpu[i..i + 3]),
                    _ => {
                        // Amplified, so a difference of two is visible rather
                        // than a black square that says "no difference".
                        for c in 0..3 {
                            let d = gpu[i + c].abs_diff(cpu[i + c]);
                            body.push(d.saturating_mul(8));
                        }
                    }
                }
            }
            if panel < 2 {
                body.extend_from_slice(&[255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0]);
            }
        }
    }
    let path = out.join(format!("diff-{}.ppm", name.replace('/', "-")));
    if let Ok(mut file) = std::fs::File::create(&path) {
        let _ = write!(file, "P6\n{total} {h}\n255\n");
        let _ = file.write_all(&body);
    }
}

fn print_table(rows: &[Row]) {
    println!(
        "{:<30} {:>6} {:>9} {:>8} {:>6} {:>8} {:>8} {:>8} {:>9}",
        "fixture", "cmds", "plans", "tris", "draws", "cpu", "gpu", "maxdiff", "interior"
    );
    println!("{}", "-".repeat(122));
    for row in rows {
        let plans = if row.complete { "COMPLETE" } else { "cpu-only" };
        let gpu = row
            .gpu
            .map_or_else(|| "-".to_owned(), |d| format!("{:.2}ms", ms(d)));
        let diff = row
            .worst_any
            .map_or_else(|| "-".to_owned(), |d| d.to_string());
        let inside = row
            .interior
            .map_or_else(|| "-".to_owned(), |c| c.to_string());
        println!(
            "{:<30} {:>6} {:>9} {:>8} {:>6} {:>6.2}ms {:>8} {:>8} {:>9}",
            row.name,
            row.commands,
            plans,
            row.triangles,
            row.draw_calls,
            ms(row.cpu),
            gpu,
            diff,
            inside
        );
        if !row.complete {
            let mut kinds: Vec<String> = row
                .unsupported
                .iter()
                .map(|(kind, count)| format!("{}x{count}", kind.name()))
                .collect();
            kinds.sort();
            println!("{:<30}   blocked by: {}", "", kinds.join(", "));
        }
    }
}

fn print_coverage(rows: &[Row]) {
    let total = rows.len();
    let complete = rows.iter().filter(|r| r.complete).count();
    println!("\n--- coverage ---------------------------------------------------------------");
    println!(
        "{complete} of {total} fixtures plan complete today ({:.0}%).",
        percent(complete, total)
    );

    let mut blocks: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut sole: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut commands: BTreeMap<&'static str, usize> = BTreeMap::new();
    for row in rows.iter().filter(|r| !r.complete) {
        for (kind, count) in &row.unsupported {
            *blocks.entry(kind.name()).or_insert(0) += 1;
            *commands.entry(kind.name()).or_insert(0) += count;
        }
        if row.unsupported.len() == 1 {
            let kind = row.unsupported.keys().next().expect("one key");
            *sole.entry(kind.name()).or_insert(0) += 1;
        }
    }

    println!(
        "\n{:<14} {:>10} {:>14} {:>12}",
        "kind", "screens", "sole blocker", "commands"
    );
    println!("{}", "-".repeat(54));
    let mut by_screens: Vec<_> = blocks.iter().collect();
    by_screens.sort_by(|a, b| b.1.cmp(a.1));
    for (kind, count) in by_screens {
        println!(
            "{:<14} {:>10} {:>14} {:>12}",
            kind,
            count,
            sole.get(kind).copied().unwrap_or(0),
            commands.get(kind).copied().unwrap_or(0)
        );
    }

    println!("\nGreedy fix order — screens complete after each gap is closed:");
    let mut fixed: BTreeSet<Unsupported> = BTreeSet::new();
    let all: BTreeSet<Unsupported> = rows
        .iter()
        .flat_map(|r| r.unsupported.keys().copied())
        .collect();
    let mut step = 1;
    while fixed.len() < all.len() {
        let mut best: Option<(Unsupported, usize)> = None;
        for candidate in all.difference(&fixed) {
            let mut trial = fixed.clone();
            trial.insert(*candidate);
            let count = rows
                .iter()
                .filter(|r| r.unsupported.keys().all(|k| trial.contains(k)))
                .count();
            if best.is_none_or(|(_, seen)| count > seen) {
                best = Some((*candidate, count));
            }
        }
        let (kind, count) = best.expect("a candidate remains");
        fixed.insert(kind);
        println!(
            "  {step}. + {:<12} -> {count:>3} of {total} complete ({:.0}%)",
            kind.name(),
            percent(count, total)
        );
        step += 1;
    }
}

fn print_parity(rows: &[Row], out: &Path) {
    let measured: Vec<&Row> = rows.iter().filter(|r| r.gpu.is_some()).collect();
    println!("\n--- parity against the CPU oracle ------------------------------------------");
    if measured.is_empty() {
        println!("no fixture was rendered on the GPU, so there is nothing to compare.");
        return;
    }
    let worst = measured
        .iter()
        .filter_map(|r| r.worst_any)
        .max()
        .unwrap_or(0);
    let bad: Vec<&&Row> = measured
        .iter()
        .filter(|r| r.interior.is_some_and(|c| c > 0))
        .collect();

    let over: usize = measured.iter().filter_map(|r| r.over_one_any).sum();
    println!("{} fixtures rendered on both.", measured.len());
    println!(
        "  whole image, edges included : worst channel {worst}/255, {over} channels over 1/255"
    );
    println!("                                context only — two rasterizers differ on every");
    println!("                                antialiased edge, by design and by documentation");
    println!(
        "  interiors only, the verdict : {} of {} fixtures have any mismatch",
        bad.len(),
        measured.len()
    );

    let beyond: Vec<&&Row> = measured
        .iter()
        .filter(|r| r.beyond_band.is_some_and(|c| c > 0))
        .collect();
    println!(
        "  beyond the geometry-edge band: {} of {} fixtures (> {BAND_TOLERANCE}, > {BAND_RADIUS} px from any reference step)",
        beyond.len(),
        measured.len()
    );

    if bad.is_empty() {
        println!("\nVERDICT: PASS. Every disagreement sits on a coverage transition in the");
        println!("reference, which is where the two rasterizers are allowed to differ.");
    } else if beyond.is_empty() {
        println!("\nVERDICT: PASS for compositing; KNOWN GAP for geometry edge antialiasing.");
        println!("Every strict-rule mismatch is within {BAND_RADIUS} px of a transition in the reference —");
        println!("the GPU fills geometry without edge AA. Strict counts, for tracking that gap:");
        let mut sorted: Vec<&&Row> = bad.clone();
        sorted.sort_by_key(|r| std::cmp::Reverse(r.interior.unwrap_or(0)));
        for row in sorted {
            println!("  {:<30} {:>8} px", row.name, row.interior.unwrap_or(0));
        }
    } else {
        println!("\nVERDICT: a real disagreement, in flat regions where there is no excuse.");
        println!("Worst first, with a CPU | GPU | amplified-difference triptych written out:");
        let mut sorted: Vec<&&Row> = beyond.clone();
        sorted.sort_by_key(|r| std::cmp::Reverse(r.beyond_band.unwrap_or(0)));
        for row in sorted {
            println!(
                "  {:<30} {:>8} px beyond band ({} strict)    {}",
                row.name,
                row.beyond_band.unwrap_or(0),
                row.interior.unwrap_or(0),
                out.join(format!("diff-{}.ppm", row.name.replace('/', "-")))
                    .display()
            );
        }
        println!("\nTiming for these fixtures means nothing until this is closed: a renderer");
        println!("that draws less is faster for a reason nobody wants.");
    }
}

fn print_timing(rows: &[Row]) {
    let measured: Vec<&Row> = rows.iter().filter(|r| r.gpu.is_some()).collect();
    println!("\n--- timing (GPU column includes full readback; see the module doc) ---------");
    if measured.is_empty() {
        println!("no GPU timings were taken.");
        return;
    }
    let clean: Vec<&&Row> = measured.iter().filter(|r| r.interior == Some(0)).collect();
    let cpu: f64 = measured.iter().map(|r| ms(r.cpu)).sum();
    let gpu: f64 = measured.iter().filter_map(|r| r.gpu).map(ms).sum();
    println!("over all {} complete fixtures:", measured.len());
    println!(
        "  cpu {cpu:.2}ms   gpu {gpu:.2}ms   ratio {:.2}x",
        cpu / gpu.max(1e-9)
    );
    if clean.len() != measured.len() {
        let ccpu: f64 = clean.iter().map(|r| ms(r.cpu)).sum();
        let cgpu: f64 = clean.iter().filter_map(|r| r.gpu).map(ms).sum();
        println!(
            "over the {} that also pass interior parity — the only trustworthy number:",
            clean.len()
        );
        if clean.is_empty() {
            println!("  none. There is no trustworthy timing on this machine yet.");
        } else {
            println!(
                "  cpu {ccpu:.2}ms   gpu {cgpu:.2}ms   ratio {:.2}x",
                ccpu / cgpu.max(1e-9)
            );
        }
    }

    let mut wins = measured.clone();
    wins.sort_by(|a, b| {
        let ra = ms(a.cpu) / ms(a.gpu.expect("filtered")).max(1e-9);
        let rb = ms(b.cpu) / ms(b.gpu.expect("filtered")).max(1e-9);
        rb.partial_cmp(&ra).expect("finite")
    });
    println!(
        "\n  {:<30} {:>8} {:>8} {:>8} {:>9}",
        "fixture", "cpu", "gpu", "ratio", "interior"
    );
    for row in &wins {
        println!(
            "  {:<30} {:>6.2}ms {:>6.2}ms {:>7.2}x {:>9}",
            row.name,
            ms(row.cpu),
            ms(row.gpu.expect("filtered")),
            ms(row.cpu) / ms(row.gpu.expect("filtered")).max(1e-9),
            row.interior
                .map_or_else(|| "-".to_owned(), |c| c.to_string())
        );
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}
