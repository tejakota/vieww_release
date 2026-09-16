//! Compare two directories of `VIEWW_SHOT` output and say whether two machines
//! drew the same frames.
//!
//! ```console
//! # on each machine
//! ci/certify/shot-suite.sh
//!
//! # then, with both directories in reach
//! cargo run -p shot-diff -- out/shots/linux-x86_64-20260909 out/shots/macos-aarch64-20260909
//! ```
//!
//! # Why a byte comparison is not enough, and why it is where this starts
//!
//! `docs/RELEASE-CHECK.md` makes a claim that is unusual for a UI framework and
//! is the reason this program can exist at all: the pixels in these shots are
//! produced by vieww's own CPU rasterizer, not by asking the OS to draw
//! anything, so **the same scene must render the same bytes on every platform**.
//! There is no "rendering noise" to threshold away — no subpixel policy the
//! platform chose, no compositor gamma, no GPU driver rounding a coverage value
//! its own way. Two runs of the same commit either agree exactly or something
//! is different, and that something is worth a name.
//!
//! So `sha256`-style byte equality would answer the question. It just answers it
//! uselessly: "these two PNGs differ" is where the work starts, not where it
//! ends, and a person handed that sentence has to open both files in an image
//! viewer and hunt. This writes the answer down instead — how many pixels, by
//! how much, and *where* — because the shape of a difference is what identifies
//! it:
//!
//! | what the report looks like | what it usually is |
//! |---|---|
//! | a handful of pixels, delta 1–2, scattered along glyph edges | font fallback picked a different face, or a different version of the same one |
//! | whole glyphs missing or shifted, delta 255 | the font is absent on that machine entirely |
//! | one rectangular region, uniform delta | a layout difference — a control sized from a system metric |
//! | every pixel differs by a constant | a colour-pipeline difference (sRGB decode on one side and not the other) |
//! | identical pixels, different bytes | two PNG encoders, which cannot happen here and means the shots were not both written by `Pixels::encode_png` |
//!
//! That last row is why this decodes rather than hashing. A pixel comparison
//! can tell "the picture is the same and the file is not" from "the picture is
//! different", and a hash cannot tell them apart at all.
//!
//! # What it does not do
//!
//! It does not know which side is right. Both directories are just directories;
//! the first is called the baseline because a comparison needs an order, not
//! because it is the truth. A difference is a fact about a pair of machines, and
//! which of them is wrong is a question for the person reading the report.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use vieww_foundation::Image;
use vieww_paint::native::Pixels;

/// How the two sides of one shot compared.
#[derive(Debug)]
enum Verdict {
    /// The same bytes. Nothing was decoded; nothing needed to be.
    SameBytes,
    /// Different bytes, identical pixels — see the module docs' last table row.
    SamePixels,
    /// Every pixel within `--tolerance`, which is 0 unless somebody asked for
    /// otherwise. Carries how many pixels were not exactly equal, because a
    /// tolerated difference is still a difference and a report that hides it is
    /// how a tolerance grows until it means nothing.
    Tolerated { pixels: u64, worst: u8 },
    /// Different pictures.
    Differs(Difference),
    /// Present on one side only.
    Missing { side: Side },
    /// Both sides exist and are not the same shape, so there is no per-pixel
    /// question to ask.
    Size {
        baseline: (u32, u32),
        candidate: (u32, u32),
    },
    /// One side would not decode.
    Unreadable { side: Side, why: String },
}

impl Verdict {
    /// Whether this verdict should fail the run.
    ///
    /// `Missing` is the caller's decision rather than this type's, so it takes
    /// the flag; everything else decides for itself.
    const fn is_failure(&self, allow_missing: bool) -> bool {
        match self {
            Self::SameBytes | Self::SamePixels | Self::Tolerated { .. } => false,
            Self::Missing { .. } => !allow_missing,
            Self::Differs(_) | Self::Size { .. } | Self::Unreadable { .. } => true,
        }
    }

    /// The one-word column in the report.
    const fn tag(&self) -> &'static str {
        match self {
            Self::SameBytes => "identical",
            Self::SamePixels => "same-pixels",
            Self::Tolerated { .. } => "tolerated",
            Self::Differs(_) => "DIFFERS",
            Self::Missing { .. } => "MISSING",
            Self::Size { .. } => "SIZE",
            Self::Unreadable { .. } => "UNREADABLE",
        }
    }
}

/// Which directory a one-sided fact is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Baseline,
    Candidate,
}

impl Side {
    const fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }
}

/// What differs, in the terms that identify the cause.
#[derive(Debug)]
struct Difference {
    /// Pixels whose channels are not all equal.
    pixels: u64,
    /// Total pixels, so the count above can be read as a fraction without the
    /// reader needing the image size from somewhere else.
    total: u64,
    /// The largest per-channel delta anywhere in the image. 1–2 is a
    /// rasterisation edge; 255 is something that is there on one side and not
    /// the other.
    worst: u8,
    /// The smallest box containing every differing pixel, as
    /// `(left, top, right, bottom)` — exclusive on the far edges.
    ///
    /// **This is the field that names the cause most often.** A box the width of
    /// a text run is a font; a box the size of a control is a layout; a box the
    /// size of the image is a colour pipeline.
    bounds: (u32, u32, u32, u32),
    /// Whether the alpha channel is among what differs. Called out separately
    /// because a difference confined to alpha is invisible in a viewer that
    /// composites onto white, which is every viewer a person is likely to open
    /// these in.
    alpha_only: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(error) => {
            eprintln!("shot-diff: {error}");
            ExitCode::from(2)
        }
    }
}

/// Parsed command line. Kept as a struct so `run` reads as a sequence of steps
/// rather than as argument handling with a comparison buried in it.
#[derive(Debug)]
struct Args {
    baseline: PathBuf,
    candidate: PathBuf,
    out: PathBuf,
    tolerance: u8,
    allow_missing: bool,
    quiet: bool,
}

const USAGE: &str = "\
usage: shot-diff <baseline-dir> <candidate-dir> [options]

  --out DIR         where diff images and index.html are written
                    (default: <candidate-dir>/../shot-diff)
  --tolerance N     per-channel delta to treat as equal (default 0)
  --allow-missing   a shot present on one side only is reported, not failed
  --quiet           only print the summary and the failures
  -h, --help        this

exit: 0 everything matched, 1 something differed, 2 the run itself failed
";

fn parse() -> Result<Args, String> {
    let mut positional = Vec::new();
    let mut out = None;
    let mut tolerance = 0_u8;
    let mut allow_missing = false;
    let mut quiet = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--out" => {
                out = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| String::from("--out needs a path"))?,
                ));
            }
            "--tolerance" => {
                tolerance = args
                    .next()
                    .ok_or_else(|| String::from("--tolerance needs a number"))?
                    .parse()
                    .map_err(|_| String::from("--tolerance takes 0-255"))?;
            }
            "--allow-missing" => allow_missing = true,
            "--quiet" => quiet = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other}\n\n{USAGE}"));
            }
            other => positional.push(PathBuf::from(other)),
        }
    }

    let [baseline, candidate] = <[PathBuf; 2]>::try_from(positional)
        .map_err(|_| format!("two directories are needed\n\n{USAGE}"))?;

    let out = out.unwrap_or_else(|| {
        candidate
            .parent()
            .unwrap_or(Path::new("."))
            .join("shot-diff")
    });

    Ok(Args {
        baseline,
        candidate,
        out,
        tolerance,
        allow_missing,
        quiet,
    })
}

fn run() -> Result<usize, String> {
    let args = parse()?;

    let baseline = shots(&args.baseline)?;
    let candidate = shots(&args.candidate)?;
    if baseline.is_empty() && candidate.is_empty() {
        return Err(format!(
            "no .png files in either {} or {}",
            args.baseline.display(),
            args.candidate.display()
        ));
    }

    std::fs::create_dir_all(&args.out)
        .map_err(|error| format!("creating {}: {error}", args.out.display()))?;

    // Every name from both sides, in one order, so the report lists a shot that
    // exists on one machine only in the place it would have been.
    let names: Vec<&String> = baseline
        .keys()
        .chain(candidate.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut verdicts = Vec::with_capacity(names.len());
    for name in names {
        let verdict = compare(
            name,
            baseline.get(name),
            candidate.get(name),
            args.tolerance,
            &args.out,
        );
        if !args.quiet || verdict.is_failure(args.allow_missing) {
            println!("{:<12} {name}{}", verdict.tag(), detail(&verdict));
        }
        verdicts.push((name.clone(), verdict));
    }

    let failures = verdicts
        .iter()
        .filter(|(_, verdict)| verdict.is_failure(args.allow_missing))
        .count();

    let report = args.out.join("index.html");
    std::fs::write(&report, html(&args, &verdicts))
        .map_err(|error| format!("writing {}: {error}", report.display()))?;

    println!(
        "\n{} shot(s), {} matched, {failures} failed — {}",
        verdicts.len(),
        verdicts.len() - failures,
        report.display()
    );
    Ok(failures)
}

/// Every `.png` directly in `dir`, keyed by file name.
///
/// Not recursive on purpose: `VIEWW_SHOT` writes flat, and a walk would happily
/// pick up the diff directory if somebody pointed it at an output folder.
fn shots(dir: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|error| format!("reading {}: {error}", dir.display()))?;
    let mut found = BTreeMap::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("reading {}: {error}", dir.display()))?
            .path();
        if path.extension().is_some_and(|ext| ext == "png") {
            if let Some(name) = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            {
                found.insert(name, path);
            }
        }
    }
    Ok(found)
}

fn compare(
    name: &str,
    baseline: Option<&PathBuf>,
    candidate: Option<&PathBuf>,
    tolerance: u8,
    out: &Path,
) -> Verdict {
    let (Some(baseline), Some(candidate)) = (baseline, candidate) else {
        return Verdict::Missing {
            side: if baseline.is_none() {
                Side::Baseline
            } else {
                Side::Candidate
            },
        };
    };

    let (left, right) = match (std::fs::read(baseline), std::fs::read(candidate)) {
        (Ok(left), Ok(right)) => (left, right),
        (Err(error), _) => {
            return Verdict::Unreadable {
                side: Side::Baseline,
                why: error.to_string(),
            }
        }
        (_, Err(error)) => {
            return Verdict::Unreadable {
                side: Side::Candidate,
                why: error.to_string(),
            }
        }
    };

    // The cheap answer first, and it is the answer almost every time: two
    // machines running the same commit are expected to produce identical files,
    // so the common path decodes nothing.
    if left == right {
        return Verdict::SameBytes;
    }

    let left = match decode(&left) {
        Ok(image) => image,
        Err(why) => {
            return Verdict::Unreadable {
                side: Side::Baseline,
                why,
            }
        }
    };
    let right = match decode(&right) {
        Ok(image) => image,
        Err(why) => {
            return Verdict::Unreadable {
                side: Side::Candidate,
                why,
            }
        }
    };

    if left.width() != right.width() || left.height() != right.height() {
        return Verdict::Size {
            baseline: (left.width(), left.height()),
            candidate: (right.width(), right.height()),
        };
    }

    let Some(difference) = measure(&left, &right) else {
        return Verdict::SamePixels;
    };

    if difference.worst <= tolerance {
        return Verdict::Tolerated {
            pixels: difference.pixels,
            worst: difference.worst,
        };
    }

    // Written before the verdict is returned rather than by the caller, because
    // the two images are decoded here and nowhere else, and handing them back
    // just to write a file would double the peak memory for no gain.
    if let Err(error) = write_diff(name, &left, &right, out) {
        eprintln!("shot-diff: could not write the diff image for {name}: {error}");
    }

    Verdict::Differs(difference)
}

fn decode(bytes: &[u8]) -> Result<Image, String> {
    vieww_asset::decode(bytes)
        .map(|(image, _format)| image)
        .map_err(|error| error.to_string())
}

/// `None` when every byte of every pixel agrees.
fn measure(left: &Image, right: &Image) -> Option<Difference> {
    let (width, height) = (left.width(), left.height());
    let (left, right) = (left.pixels(), right.pixels());

    let mut pixels = 0_u64;
    let mut worst = 0_u8;
    let mut alpha_only = true;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0_u32, 0_u32);

    for (index, (a, b)) in left.chunks_exact(4).zip(right.chunks_exact(4)).enumerate() {
        if a == b {
            continue;
        }
        pixels += 1;

        for channel in 0..4 {
            let delta = a[channel].abs_diff(b[channel]);
            worst = worst.max(delta);
            if delta != 0 && channel != 3 {
                alpha_only = false;
            }
        }

        // `index` is bounded by the pixel count, which came from a `u32` width
        // times a `u32` height that this build already holds in memory as RGBA —
        // so the product fits, and the cast cannot be the thing that is wrong
        // here.
        #[allow(clippy::cast_possible_truncation)]
        let index = index as u32;
        let (x, y) = (index % width, index / width);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    if pixels == 0 {
        return None;
    }

    Some(Difference {
        pixels,
        total: u64::from(width) * u64::from(height),
        worst,
        bounds: (min_x, min_y, max_x + 1, max_y + 1),
        alpha_only,
    })
}

/// Write `<name>` as a picture of *where* the two disagree.
///
/// # Why it is not a side-by-side
///
/// A side-by-side puts the burden back on the reader's eyes, which is the thing
/// this program exists to remove — and the differences it is looking for are
/// routinely a delta of 1 on a glyph rim, which no eye finds in a pair of 480px
/// panels. So the baseline is washed out to a fifth of its contrast and every
/// differing pixel is painted magenta at an opacity scaled by how far apart the
/// two are. The result is a map: the layout stays legible enough to say *what*
/// moved, and the marks are the only saturated thing in the frame.
///
/// **Washed towards white rather than dimmed towards black**, which is not a
/// taste question. Every shot in this repository is composited onto
/// [`BASE`](../../features/harness/src/lib.rs) — white — so scaling the channels
/// down turned a page of dark text on white into dark text on dark grey, and the
/// one thing a person needs from the faded layer is to recognise the screen it
/// came from.
///
/// Magenta because it is the one hue no widget in this repository's theme uses,
/// so a mark can never be mistaken for content.
fn write_diff(name: &str, left: &Image, right: &Image, out: &Path) -> Result<(), String> {
    let (width, height) = (left.width(), left.height());
    let mut data = Vec::with_capacity(left.pixels().len());

    for (a, b) in left
        .pixels()
        .chunks_exact(4)
        .zip(right.pixels().chunks_exact(4))
    {
        let delta = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
        if delta == 0 {
            let wash = |c: u8| 255 - (255 - c) / 5;
            data.extend_from_slice(&[wash(a[0]), wash(a[1]), wash(a[2]), 255]);
            continue;
        }
        // One hue, mixed towards white by how far apart the two pixels are: a
        // delta of 255 is full magenta, a delta of 1 is pink. The floor of a
        // third is the load-bearing part — the faint differences are the ones
        // worth finding, and mixing linearly from zero would draw a delta of 1
        // at 1/255 opacity and hide exactly the case this was written for.
        let strength = 85 + u16::from(delta) * 170 / 255;
        let mix = |target: u8| {
            #[allow(clippy::cast_possible_truncation)]
            {
                (255 - u16::from(255 - target) * strength / 255) as u8
            }
        };
        data.extend_from_slice(&[mix(255), mix(0), mix(170), 255]);
    }

    let png = Pixels::from_rgba8(data, width, height)
        .encode_png()
        .map_err(|error| error.to_string())?;
    let path = out.join(format!("diff-{name}"));
    std::fs::write(&path, png).map_err(|error| format!("{}: {error}", path.display()))
}

/// The trailing half of a console line: what the verdict knows, in words.
fn detail(verdict: &Verdict) -> String {
    match verdict {
        Verdict::SameBytes | Verdict::SamePixels => String::new(),
        Verdict::Tolerated { pixels, worst } => {
            format!(" — {pixels} pixel(s) differ by at most {worst}, within tolerance")
        }
        Verdict::Missing { side } => format!(" — not in the {} directory", side.name()),
        Verdict::Size {
            baseline,
            candidate,
        } => format!(
            " — {}x{} vs {}x{}",
            baseline.0, baseline.1, candidate.0, candidate.1
        ),
        Verdict::Unreadable { side, why } => format!(" — the {} side: {why}", side.name()),
        Verdict::Differs(difference) => {
            let (left, top, right, bottom) = difference.bounds;
            format!(
                " — {} of {} pixel(s), worst channel delta {}, inside ({left},{top})-({right},{bottom}){}",
                difference.pixels,
                difference.total,
                difference.worst,
                if difference.alpha_only {
                    ", alpha only (invisible over white)"
                } else {
                    ""
                }
            )
        }
    }
}

/// The report, as one self-contained page.
///
/// No stylesheet and no script from anywhere: this is opened from a file path on
/// a machine that may have just been handed the directory over a USB stick, and
/// a report that needs the network to render is a report nobody reads.
fn html(args: &Args, verdicts: &[(String, Verdict)]) -> String {
    let failures = verdicts
        .iter()
        .filter(|(_, verdict)| verdict.is_failure(args.allow_missing))
        .count();

    let mut page = String::new();
    let _ = write!(
        page,
        "<!doctype html><meta charset=utf-8><title>shot-diff</title>\
         <style>\
         body{{font:14px/1.5 system-ui,sans-serif;margin:2rem;max-width:70rem}}\
         table{{border-collapse:collapse;width:100%}}\
         td,th{{text-align:left;padding:.4rem .6rem;border-bottom:1px solid #ddd;vertical-align:top}}\
         .DIFFERS,.MISSING,.SIZE,.UNREADABLE{{color:#b00;font-weight:600}}\
         .identical,.same-pixels,.tolerated{{color:#666}}\
         img{{max-width:22rem;border:1px solid #ddd;margin-top:.4rem}}\
         code{{background:#f4f4f4;padding:0 .2rem}}\
         </style>\
         <h1>shot-diff</h1>\
         <p>baseline <code>{}</code><br>candidate <code>{}</code><br>\
         tolerance {}, {} shot(s), <strong>{failures} failed</strong></p>\
         <table><tr><th>shot<th>verdict<th>detail</tr>",
        escape(&args.baseline.display().to_string()),
        escape(&args.candidate.display().to_string()),
        args.tolerance,
        verdicts.len(),
    );

    for (name, verdict) in verdicts {
        let image = if matches!(verdict, Verdict::Differs(_)) {
            format!("<br><img src=\"diff-{}\" alt=\"\">", escape(name))
        } else {
            String::new()
        };
        let _ = write!(
            page,
            "<tr><td>{}<td class=\"{}\">{}<td>{}{image}</tr>",
            escape(name),
            verdict.tag(),
            verdict.tag(),
            escape(detail(verdict).trim_start_matches(" — ")),
        );
    }

    page.push_str("</table>");
    page
}

/// Enough escaping for a file name and a path in an attribute.
///
/// These strings come from a directory listing on the machine running this, not
/// from anywhere hostile — but a shot named with an ampersand would silently
/// break the table, and that is a bug that costs an hour before anybody suspects
/// the report rather than the shots.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
