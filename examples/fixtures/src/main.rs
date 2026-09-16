//! The vieww fixture gallery: every rendering feature, tiny to complex, as a
//! picture and a number.
//!
//! ```console
//! cargo run --release -p fixtures            # writes ./fixtures-out/*.png
//! cargo run --release -p fixtures -- /tmp/x  # somewhere else
//! ```
//!
//! # What this is for
//!
//! A framework's rendering is not "working" because a demo opened. It is
//! working when every primitive it offers costs what it should, composes with
//! the others, and can be *looked at* — which is why every fixture here writes
//! a PNG as well as a timing, and why the animated ones write GIFs.
//!
//! The gallery is graduated on purpose. A single complex screen tells you a
//! frame took eleven milliseconds and nothing about why. Two hundred
//! rectangles, then the same two hundred rounded, then the same two hundred
//! under a clip, tell you exactly which of those three things is expensive —
//! and the answer has already been surprising once: the clip, by a factor of
//! six hundred.

mod census;
mod interaction;
mod motion;
mod primitives;
mod runner;
mod screens;
mod vectors;

use std::path::PathBuf;

use vieww_foundation::{Color, Size};

use runner::Fixture;

const SURFACE: Size = Size::new(840.0, 560.0);

fn main() {
    if std::env::args().any(|a| a == "--census") {
        census::run(&catalogue());
        return;
    }

    let mut args = std::env::args().skip(1);
    let out: PathBuf = args.next().unwrap_or_else(|| "fixtures-out".into()).into();
    // **A filter, because a profiler needs one fixture and not twenty-three.**
    //
    // "This fixture is over budget" is the start of the question, not the end
    // of it, and the next step is always `valgrind --tool=callgrind` — which,
    // run over the whole gallery, attributes the answer to twenty-two other
    // fixtures as well as the one being asked about. Finding out that the
    // gallery's blurs were *not* where `23-editor-glass` spent its time took
    // two wrong guesses and a wasted afternoon before this argument existed.
    //
    // A substring, not a flag: `-- out/ editor` runs `22-editor` and
    // `23-editor-glass`, `-- out/ 23-` runs one.
    let only: Option<String> = args.next().filter(|a| !a.starts_with("--"));
    std::fs::create_dir_all(&out).expect("creating the output directory");

    let fixtures: Vec<Fixture> = catalogue()
        .into_iter()
        .filter(|f| only.as_ref().is_none_or(|needle| f.name.contains(needle)))
        .collect();
    if fixtures.is_empty() {
        eprintln!(
            "no fixture matches {:?} — run with no filter to list them all",
            only.unwrap_or_default()
        );
        std::process::exit(1);
    }
    let mut measured = Vec::with_capacity(fixtures.len());
    for fixture in &fixtures {
        measured.push(runner::run(fixture, &out));
    }
    runner::print_table(&measured);

    // The motion and interaction suites are the whole-gallery run's job. A
    // filtered run is somebody looking at one still fixture under a profiler,
    // and adding five GIF encodes to that is noise in the profile.
    if only.is_none() {
        motion::run_all(&out);
        interaction::run();
        interaction::run_glass();
    }

    println!("\nwrote {} PNGs to {}", fixtures.len(), out.display());

    // **Exit non-zero on a layout overflow, so `ci/check/checks.sh` can refuse one.**
    //
    // The gallery is where a rendering change is reviewed, and a fixture whose
    // own column does not fit its own box is not a rendering of this framework
    // — it is a picture of a broken layout, presented as the reference for what
    // correct output looks like. Printing a warning made that a thing a
    // reviewer might notice; exiting non-zero makes it a thing a script
    // refuses, which is exactly what `vieww_render::overflow::reported`'s own
    // doc says the count is for.
    let overflows = runner::total_overflows(&measured);
    if overflows > 0 {
        std::process::exit(1);
    }
}

/// Every still fixture, ordered simplest first.
fn catalogue() -> Vec<Fixture> {
    let paper = primitives::PAPER;
    vec![
        // ── tier 0: one primitive at a time ───────────────────────────
        Fixture {
            name: "00-fills",
            about: "384 plain fills — the control",
            size: SURFACE,
            background: paper,
            build: primitives::fills,
        },
        Fixture {
            name: "00-fills-rounded",
            about: "the same fills, rounded — cost of curves",
            size: SURFACE,
            background: paper,
            build: primitives::fills_rounded,
        },
        Fixture {
            name: "00-fills-gradient",
            about: "the same fills, gradient-shaded — cost of shading",
            size: SURFACE,
            background: paper,
            build: primitives::fills_gradient,
        },
        Fixture {
            name: "00-fills-rrect-clip",
            about: "the same fills under ONE rounded clip",
            size: SURFACE,
            background: paper,
            build: primitives::fills_rrect_clip,
        },
        Fixture {
            name: "00-fills-nested-clips",
            about: "the same fills, three clips deep",
            size: SURFACE,
            background: paper,
            build: primitives::fills_nested_clips,
        },
        Fixture {
            name: "00-strokes",
            about: "stroke expansion: joins, caps",
            size: SURFACE,
            background: paper,
            build: primitives::strokes,
        },
        Fixture {
            name: "00-curves",
            about: "open cubics — the flattener's real workload",
            size: SURFACE,
            background: paper,
            build: primitives::curves,
        },
        Fixture {
            name: "00-shadows",
            about: "384 blurred shadow masks",
            size: SURFACE,
            background: paper,
            build: primitives::shadows,
        },
        Fixture {
            name: "00-layers-flat",
            about: "384 sibling isolated layers",
            size: SURFACE,
            background: paper,
            build: primitives::layers_flat,
        },
        Fixture {
            name: "00-layers-nested",
            about: "24 layers deep — compositor depth",
            size: SURFACE,
            background: paper,
            build: primitives::layers_nested,
        },
        Fixture {
            name: "00-blend-modes",
            about: "12 blend modes over one backdrop",
            size: SURFACE,
            background: paper,
            build: primitives::blend_modes,
        },
        Fixture {
            name: "00-blurs",
            about: "blur at five sigmas, incl. bounds",
            size: SURFACE,
            background: paper,
            build: primitives::blurs,
        },
        Fixture {
            name: "00-transforms",
            about: "96 rotated + scaled boxes",
            size: SURFACE,
            background: Color::rgb(16, 18, 24),
            build: primitives::transforms,
        },
        // ── tier 1: vectors, icons, text ──────────────────────────────
        Fixture {
            name: "10-icon-grid",
            about: "120 vector icons, four sizes",
            size: SURFACE,
            background: paper,
            build: vectors::icon_grid,
        },
        Fixture {
            name: "10-icon-strokes",
            about: "stroked line-art icons at small sizes",
            size: SURFACE,
            background: paper,
            build: vectors::icon_strokes,
        },
        Fixture {
            name: "11-typography",
            about: "a type scale, real glyph outlines",
            size: SURFACE,
            background: paper,
            build: vectors::typography,
        },
        Fixture {
            name: "11-code-block",
            about: "syntax-coloured code — many small runs",
            size: SURFACE,
            background: Color::rgb(16, 18, 24),
            build: vectors::code_block,
        },
        Fixture {
            name: "12-avatar",
            about: "one complex layered vector portrait",
            size: SURFACE,
            background: paper,
            build: vectors::avatar,
        },
        Fixture {
            name: "12-avatar-wall",
            about: "24 of them, clipped to circles",
            size: SURFACE,
            background: paper,
            build: vectors::avatar_wall,
        },
        // ── tier 2: real screens ──────────────────────────────────────
        Fixture {
            name: "20-settings",
            about: "a settings form: rows, switches, chrome",
            size: SURFACE,
            background: paper,
            build: screens::settings,
        },
        Fixture {
            name: "21-dashboard",
            about: "cards, charts, elevation, gradients",
            size: SURFACE,
            background: Color::rgb(12, 14, 19),
            build: screens::dashboard,
        },
        Fixture {
            name: "22-editor",
            about: "an IDE shell: sidebar, code, panels",
            size: Size::new(1366.0, 679.0),
            background: Color::rgb(16, 18, 24),
            build: screens::editor,
        },
        Fixture {
            name: "23-editor-glass",
            about: "the same, with blur, shadows and overlays",
            size: Size::new(1366.0, 679.0),
            background: Color::rgb(16, 18, 24),
            build: screens::editor_glass,
        },
    ]
}
