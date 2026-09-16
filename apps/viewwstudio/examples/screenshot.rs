//! Rasterise the shell headless and write a PNG. No window, no display.
//!
//! ```console
//! cargo run -p viewwstudio --example screenshot -- shell.png
//! cargo run -p viewwstudio --example screenshot -- shell-light.png --light
//! ```
//!
//! # It does not need a GPU
//!
//! It renders through vieww's own rasterizer, this crate's one and only
//! renderer, which needs no graphics adapter at all — rather than printing
//! "no graphics adapter; nothing rendered" and exiting 0, which is what an
//! earlier, vello-based version of this file did on a machine with no
//! adapter, and which is indistinguishable from success to anything reading
//! an exit code. Every machine can now produce this picture.
//!
//! # Why this exists
//!
//! This repository's own gallery example says it twice: a `Radio` whose dot was
//! an ellipse passed the entire suite, and a frame overlay was wrong for as long
//! as the platform crate existed. Both were obvious the moment somebody looked.
//! A shell assembled from eight modules has the same failure mode and no
//! keyboard-driven way to catch it, so the picture is produced by the build
//! rather than by a person with a window open.

use std::path::PathBuf;

use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use viewwstudio::compile::{Session, Toolchain};
use viewwstudio::state::{PanelTab, Platform};
use viewwstudio::{Shell, Studio, Workspace, WINDOW};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .map_or_else(|| PathBuf::from("shell.png"), PathBuf::from);
    let rest: Vec<String> = args.collect();
    let light = rest.iter().any(|a| a == "--light");
    let failed = rest.iter().any(|a| a == "--failed");
    let android = rest.iter().any(|a| a == "--android");
    let collapsed = rest.iter().any(|a| a == "--collapsed");
    // Which sidebar view to open, so a screenshot can show something other than
    // the Explorer. `--view=toolchains` is the one N4 is checked with.
    let view = rest.iter().find_map(|a| a.strip_prefix("--view="));
    // Which accent to shoot. The palette is eight ramps that every accented
    // surface in the shell reads from, so "does the accent reach everything"
    // is a question one picture per colour answers and no unit test does.
    let accent = rest.iter().find_map(|a| a.strip_prefix("--accent="));
    // The Open Folder overlay. It is the one piece of the shell with no
    // keyboard-driven way to be looked at in CI, and this file's whole argument
    // is that a picture nobody has to open a window for is the one that gets
    // looked at.
    let picker = rest.iter().any(|a| a == "--picker");
    // The window the user reported the clipping in: 1366×679 is a laptop with
    // the browser chrome still on screen, and it is where a fixed-size device
    // frame runs out of room first.
    let small = rest.iter().any(|a| a == "--small");

    // `--font=20` — the setting the editor reads. Here so a picture can show
    // that changing it changes what is drawn, which is the one thing a
    // round-trip test through the settings file cannot check.
    let font: Option<f32> = rest
        .iter()
        .find_map(|a| a.strip_prefix("--font="))
        .and_then(|value| value.parse().ok());

    // `--open=Cargo.toml` — a file beside the bundled screens, so a picture
    // can show a language other than Rust.
    let open = rest.iter().find_map(|a| a.strip_prefix("--open="));

    // `--zoom=1` — the preview's own zoom, so a picture can show the previewed
    // widgets at the size somebody would judge them at rather than at whatever
    // fraction fits the pane.
    let zoom: Option<f32> = rest
        .iter()
        .find_map(|a| a.strip_prefix("--zoom="))
        .and_then(|value| value.parse().ok());

    // `--cpu` used to force the software fallback rather than the GPU
    // backend; accepted and ignored now that vieww's own rasterizer is the
    // only renderer this crate has, so an old invocation still runs.
    let _force_cpu = rest.iter().any(|a| a == "--cpu");

    let window = if small {
        vieww_foundation::Size::new(1366.0, 679.0)
    } else {
        WINDOW
    };
    let mut driver = FrameDriver::new(window);
    // Same call `main` makes. Without it the shell mounts with `Shortcuts`
    // unregistered, which prints a warning and drops the whole shortcut layer
    // out of the tree — so the picture was of a slightly different window than
    // the one that ships.
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    // The bundled screens, so the picture shows a file that exists rather than
    // a buffer invented for the screenshot.
    let workspace = Workspace::open(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/screens"
    )));
    // The real pipeline, so the picture is of a screen that actually compiled
    // rather than of a state set by hand.
    // The checkout's own target directory by default, and whatever
    // `VIEWWSTUDIO_TARGET_DIR` names when it is set — the same variable
    // `install::target_dir` honours first.
    //
    // **So that a picture can be taken of a packaged SDK.** `package.sh`
    // assembles a `lib/vieww` containing a manifest, the exact rlib the binary
    // linked and 270-odd dependencies, and until this line the only way to
    // find out whether that bundle actually compiles a preview was to install
    // it on a machine with a display and press Render. Pointing this at the
    // bundle exercises `Toolchain::discover`, the fingerprint check and a real
    // guest compile, and produces a picture of the result.
    let target = std::env::var_os(viewwstudio::install::TARGET_DIR_ENV).map_or_else(
        || std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug")),
        std::path::PathBuf::from,
    );
    let studio = Studio::with_workspace(&runtime, workspace)
        .with_toolchain(Toolchain::discover(&target), Session::new(0x5C_2EE0).ok());

    studio.dark.set(!light);
    if let Some(accent) = accent {
        studio.accent.set((*accent).to_owned());
    }
    if let Some(size) = font {
        studio.font_size.set(size);
    }
    if android {
        studio.platform.set(Platform::Android);
    }
    if let Some(zoom) = zoom {
        studio.preview_zoom.set(Some(zoom));
    }

    // Before the render, so `--open` chooses *what* is compiled rather than
    // opening a file beside a picture of a different one.
    if let Some(name) = open {
        studio.open_path(
            std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/screens")).join(name),
        );
    }

    // Render the buffer for real. `--failed` then edits it into something that
    // does not compile and renders again, which is how the stale-frame state
    // gets into a screenshot without being staged.
    studio.render();
    settle(&studio);
    if failed {
        let mut value = studio.active().expect("a buffer").value;
        value
            .text
            .push_str("\nfn broken() -> u32 { \"not a number\" }\n");
        studio.edit(value);
        studio.render();
        settle(&studio);
        studio.panel_tab.set(PanelTab::Problems);
    }
    if collapsed {
        studio.panel_open.set(false);
    }
    if picker {
        studio.open_picker();
    }
    if let Some(view) = view {
        studio.view.set(match view {
            "toolchains" | "toolchain" => viewwstudio::state::View::Toolchain,
            "search" => viewwstudio::state::View::Search,
            "problems" => viewwstudio::state::View::Problems,
            "snippets" => viewwstudio::state::View::Snippets,
            "inspector" => viewwstudio::state::View::Inspector,
            "tokens" => viewwstudio::state::View::Tokens,
            "export" => viewwstudio::state::View::Export,
            "source" | "git" => viewwstudio::state::View::Source,
            "settings" => viewwstudio::state::View::Settings,
            _ => viewwstudio::state::View::Explorer,
        });
    }

    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();

    // Scroll to the diagnostic, so `--failed` shows the squiggle rather than
    // the top of a file that has an error somewhere in it.
    //
    // **After the first frame, and needing a second.** `reveal` scrolls the
    // least distance that brings a range into view, and before layout has
    // reported extents the viewport is zero — every range looks taller than
    // the window and there is nowhere to scroll to. One frame measures, the
    // jump moves, the next frame draws it.
    if failed {
        if let Some(first) = studio.diagnostics.get().first() {
            studio.jump_to(first.line, first.column);
            driver.draw_frame();
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a window size in logical pixels is a small positive number"
    )]
    let (width, height) = (window.width as u32, window.height as u32);

    // The ground the card shell floats on, not the activity bar's colour. With
    // gutters between the regions this is genuinely visible in the picture.
    let background = if light {
        viewwstudio::StudioTheme::light().window
    } else {
        viewwstudio::StudioTheme::dark().window
    };

    let mut renderer = NativeRenderer::new();
    let (png, report) = renderer
        .render_to_png(driver.scene(), width, height, background)
        .expect("rendering the shell");
    std::fs::write(&path, png).expect("writing the PNG");

    println!(
        "{} — {width}×{height}, {} shapes",
        path.display(),
        report.shapes
    );
}

/// Run the compile the studio just started to completion.
///
/// # Why a screenshot has to do this at all
///
/// `Studio::render` starts a worker thread and a frame hook polls the result.
/// A screenshot draws one frame and exits, so without this it draws the frame
/// *before* the compile finished — and `--failed` produced a picture reading
/// "No problems. The buffer compiled clean." over a buffer that does not
/// compile. Which is worse than no picture: it is fiction that renders
/// identically to fact.
///
/// It also has to run after the **first** render, not only after the edit:
/// `render` refuses to start while a job is in flight, so a second call made
/// half a second after the first is a no-op and the screenshot then waits for
/// the *unedited* buffer's compile to finish.
fn settle(studio: &Studio) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while studio.is_compiling() && std::time::Instant::now() < deadline {
        if !studio.poll_compile() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
