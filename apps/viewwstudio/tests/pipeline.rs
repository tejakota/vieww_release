//! M3 end to end: a buffer, through `rustc`, into a widget tree.
//!
//! These tests run the real compiler against the real `.rlib`s. They skip —
//! loudly — when the workspace has not been built, because the alternative is a
//! suite that goes green on a machine where the pipeline cannot work at all.

use std::path::{Path, PathBuf};

use viewwstudio::compile::{compile, Job, Progress, Session, Toolchain, TIMEOUT};
use viewwstudio::loaded::{LoadError, Preview};
use viewwstudio::state::Severity;

/// `target/debug` of this workspace, where `libvieww.rlib` lands.
fn target() -> PathBuf {
    // The test binary is at target/debug/deps/<name>-<hash>; two levels up is
    // the profile directory, which is what the studio itself passes.
    let exe = std::env::current_exe().expect("a test binary");
    exe.parent()
        .and_then(Path::parent)
        .expect("target/debug")
        .to_path_buf()
}

/// The smallest screen that compiles, for probing which `vieww` candidate this
/// test binary actually shares types with.
const PROBE: &str = r#"
use vieww::prelude::*;
#[derive(Debug)]
pub struct Probe;
impl Widget for Probe {
    fn debug_name(&self) -> &'static str { "Probe" }
    fn kind(&self) -> WidgetKind<'_> { WidgetKind::Composed }
    fn build(&self, _ctx: &BuildContext) -> WidgetNode { Container::new().into() }
}
pub fn screen() -> impl Widget { Probe }
"#;

/// Which of `Toolchain::discover`'s candidates is the compilation of `vieww`
/// that *this* test binary links.
///
/// # Why the first one is not necessarily it
///
/// `discover` returns the candidate rlibs it finds, newest first, and a
/// well-used `target/` holds several: cargo produces a fresh
/// `libvieww-<hash>.rlib` whenever feature resolution or a codegen flag
/// changes, and it keeps the old ones. This checkout accumulated **thirteen**.
///
/// A preview built against the wrong one hands back widgets whose `TypeId`s the
/// host has never seen — which is exactly what `loaded::host_fingerprint`
/// exists to catch, and it catches it: `LoadError::AbiMismatch`. `state::Studio`
/// responds by advancing to the next candidate and compiling again, so the
/// *studio* works in a directory like that.
///
/// These tests did not: every one of them took the first candidate and
/// `expect`ed the load, so they failed in precisely the situation the product
/// handles. That is a false failure, and a loud one — eight tests reporting
/// that the compile pipeline is broken when the compile pipeline is doing its
/// job.
///
/// So the probe is run once per test binary and every test is handed the
/// candidate that answered. If none does, the tests skip rather than fail:
/// there is then no compilation of `vieww` here matching this binary, which is
/// a fact about the checkout and not about the code under test.
fn matching_candidate(base: &Toolchain) -> Option<usize> {
    static RESOLVED: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *RESOLVED.get_or_init(|| {
        let session = Session::new(0xABCD).ok()?;
        for candidate in 0.. {
            let toolchain = base.with_candidate(candidate)?;
            let Ok(compiled) = compile(&toolchain, &session, PROBE, "probe.rs") else {
                continue;
            };
            let Some(library) = compiled.library.as_ref() else {
                continue;
            };
            match Preview::load(library) {
                Ok(_) => return Some(candidate),
                Err(LoadError::AbiMismatch { .. }) => continue,
                Err(_) => continue,
            }
        }
        None
    })
}

/// The toolchain exactly as [`Toolchain::discover`] returns it, candidate
/// ordering and all.
///
/// [`toolchain`] resolves past the head of that list when the head is a
/// different compilation of `vieww` (see [`matching_candidate`]), which is what
/// every test that *loads* a preview wants and what a test about the ordering
/// itself must not have.
fn discovered() -> Option<Toolchain> {
    match Toolchain::discover(&target()) {
        Ok(toolchain) => Some(toolchain),
        Err(error) => {
            eprintln!("skipping: {error}");
            None
        }
    }
}

fn toolchain() -> Option<Toolchain> {
    let base = match Toolchain::discover(&target()) {
        Ok(toolchain) => toolchain,
        Err(error) => {
            eprintln!("skipping: {error}");
            return None;
        }
    };
    // See `matching_candidate`: newest-first is a good guess and not a
    // guarantee, and every test below assumes the preview it compiles will
    // load.
    let Some(candidate) = matching_candidate(&base) else {
        eprintln!(
            "skipping: no `vieww` rlib in {} matches this test binary's own \
             compilation — build the workspace the way this binary was built",
            target().display()
        );
        return None;
    };
    base.with_candidate(candidate)
}

const GOOD: &str = r#"
use vieww::prelude::*;

#[derive(Debug)]
pub struct Screen;

impl Widget for Screen {
    fn debug_name(&self) -> &'static str { "Screen" }
    fn kind(&self) -> WidgetKind<'_> { WidgetKind::Composed }
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Container::new().color(Color::hex(0x22_88FF)).into()
    }
}

pub fn screen() -> impl Widget { Screen }
"#;

#[test]
fn a_buffer_compiles_loads_and_becomes_a_tree() {
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x1111).expect("a temp directory");

    let compiled = compile(&toolchain, &session, GOOD, "screen.rs").expect("rustc ran");
    assert!(
        !compiled.failed(),
        "the sample compiles: {:?}",
        compiled.diagnostics
    );
    assert!(compiled.duration < TIMEOUT);

    let library = compiled.library.expect("a library");
    let preview = Preview::load(&library).expect("it loads");

    // The proof it is a real tree and not a handle: build it and read the dump.
    let dump = vieww_widget::debug_tree(preview.node());
    assert!(
        dump.contains("Screen"),
        "the loaded widget names itself: {dump}"
    );
}

#[test]
fn a_type_error_comes_back_as_a_diagnostic_at_the_right_line() {
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x2222).expect("a temp directory");

    // Line 4 passes an f32 where `padding` wants `EdgeInsets`.
    let source = "\nuse vieww::prelude::*;\n\npub fn screen() -> impl Widget {\n    Container::new().padding(16.0)\n}\n";
    let compiled = compile(&toolchain, &session, source, "screen.rs").expect("rustc ran");

    assert!(compiled.failed(), "no library comes out of a broken buffer");
    let error = compiled
        .diagnostics
        .iter()
        .find(|d| matches!(d.severity, Severity::Error))
        .expect("an error");

    assert_eq!(
        error.line, 5,
        "the line in the buffer, not in the temp file"
    );
    assert_eq!(error.file, "screen.rs");
    assert!(
        error.code.starts_with('E'),
        "a real rustc code: {}",
        error.code
    );
}

#[test]
fn a_panicking_screen_is_caught_at_the_boundary_rather_than_taking_the_host_down() {
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x3333).expect("a temp directory");

    let source = r#"
use vieww::prelude::*;

pub fn screen() -> impl Widget {
    panic!("this is what user code does at three in the morning");
    #[allow(unreachable_code)]
    Container::new()
}
"#;

    let compiled = compile(&toolchain, &session, source, "screen.rs").expect("rustc ran");
    let library = compiled.library.expect("it compiles — it just panics");

    // The process surviving this call *is* the assertion.
    let result = Preview::load(&library);
    assert_eq!(
        result.err(),
        Some(LoadError::PanicInScreen),
        "the panic became a value, and the studio is still running to read it"
    );
}

#[test]
fn a_panicking_build_is_caught_too_rather_than_aborting_the_process() {
    // **The case the test above did not cover, and the studio did not survive.**
    //
    // `screen()` panicking was always caught, because that catch is written
    // into the buffer by `compile::ENTRY` and runs inside the image. A panic in
    // the widget's `build()` was supposed to be caught by `Preview::probe`,
    // which called `build()` from the host inside a `catch_unwind` — and could
    // not work: a cdylib links its own `std`, so a guest panic is a *foreign*
    // exception to the host's unwinder, which does not catch those. It printed
    // "fatal runtime error: Rust cannot catch foreign exceptions" and aborted,
    // taking every unsaved buffer with it, and `LoadError::PanicInBuild` was
    // unreachable code that read like a safety net.
    //
    // As above, the process still being alive to run the assertion is most of
    // the assertion.
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x3334).expect("a temp directory");

    let source = r#"
use vieww::prelude::*;

#[derive(Debug)]
pub struct Screen;

impl Widget for Screen {
    fn debug_name(&self) -> &'static str { "Screen" }
    fn kind(&self) -> WidgetKind<'_> { WidgetKind::Composed }
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        panic!("this is what user code does at three in the morning")
    }
}

pub fn screen() -> impl Widget { Screen }
"#;

    let compiled = compile(&toolchain, &session, source, "screen.rs").expect("rustc ran");
    let library = compiled.library.expect("it compiles — it just panics");

    assert_eq!(
        Preview::load(&library).err(),
        Some(LoadError::PanicInBuild),
        "a panicking build() has to come back as a diagnostic, not as an abort"
    );
}

///
/// **Ignored on Windows, where the boundary does not exist yet.** The catch
/// below needs the host and the guest to share one `libstd`, and
/// `.cargo/config.toml` deliberately does not set `-C prefer-dynamic` for MSVC
/// (see `ci/check/platform-check.sh`'s Windows note: that toolchain's dynamic
/// `std` is not something a checkout can rely on, and the answer there is an
/// out-of-process preview, which is not written). So on Windows the guest
/// panic really is a foreign exception, `catch_unwind` cannot catch it, and
/// the process fails fast — the run showed it as
/// `STATUS_STACK_BUFFER_OVERRUN`, a crashed test binary rather than a failed
/// assertion. Ignoring it records that gap where somebody will read it;
/// `packaging/package.sh` gives the *shipped* studio a dynamic `std`, so this
/// is about a checkout, not about the installed product.
#[test]
#[cfg_attr(
    windows,
    ignore = "needs one shared libstd; MSVC has no dynamic std in a checkout"
)]
fn a_panicking_build_on_a_later_rebuild_is_caught_by_the_host_not_aborted() {
    // **The boundary the panic section in `loaded.rs` describes as the third
    // catch.** `screen()` panicking and the *first* `build()` panicking are
    // caught inside the guest, because unwinding across `extern "C"` is UB.
    // A panic on a *later* rebuild — when `ElementTree::build` calls
    // `widget.build` again after the widget has already been mounted — runs in
    // the host's frame, and is caught by `ElementTree::build`'s `catch_unwind`.
    //
    // For that catch to fire on guest code, the host and the guest have to
    // share one `libstd`: a `cdylib` statically links its own `std`, so a
    // guest panic is a *foreign* exception to the host's unwinder, and
    // `catch_unwind` does not catch foreign exceptions — it prints "fatal
    // runtime error: Rust cannot catch foreign exceptions" and aborts. Both
    // sides are built `-C prefer-dynamic` (see `Cargo.toml`'s `[profile.dev]`
    // and `compile.rs`'s `run_rustc`), which puts both on the same
    // `libstd-*.so` and makes the host's catch fire.
    //
    // The process surviving this test *is* the assertion. The shape of the
    // recovered error — a recorded build error rather than an abort — is the
    // secondary one.
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x3355).expect("a temp directory");

    // A widget that panics on the *second* build: the first build sets a
    // `Cell`, the second build reads it and panics. The probe (which fires
    // once, before mounting) takes the first-build path and does not panic;
    // the host-side `ElementTree::build` then takes the second-build path
    // and is what has to catch the panic.
    let source = r#"
use vieww::prelude::*;
use std::cell::Cell;

thread_local! { static BUILDS: Cell<u32> = Cell::new(0); }

#[derive(Debug)]
pub struct Screen;

impl Widget for Screen {
    fn debug_name(&self) -> &'static str { "Screen" }
    fn kind(&self) -> WidgetKind<'_> { WidgetKind::Composed }
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let n = BUILDS.with(|b| { let n = b.get(); b.set(n + 1); n });
        if n >= 1 {
            panic!("a panic on a later rebuild");
        }
        Text::new("first").into()
    }
}

pub fn screen() -> impl Widget { Screen }
"#;

    let compiled = compile(&toolchain, &session, source, "screen.rs").expect("rustc ran");
    let library = compiled.library.expect("it compiles — it just panics");
    let preview = Preview::load(&library).expect("the probe fires once and does not panic");

    let mut driver = vieww_render::FrameDriver::new(vieww_foundation::Size::new(400.0, 300.0));
    // **The studio's own error policy, because that is what is being tested.**
    //
    // `ErrorPolicy::default()` is `Placeholder` under `debug_assertions` and
    // `Propagate` otherwise, and `viewwstudio::install` overrides it to
    // `Custom(panicked_widget)` for the reason its own comment gives — the code
    // that panics is the user's, and the build they run is the release one.
    //
    // A bare `FrameDriver` inherits the default instead, so this test used to
    // assert the boundary held while running with the boundary switched off.
    // Under `cargo test` (debug) it passed anyway, on the debug default; under
    // `cargo test --release` the panic propagated and took the test process
    // with it. Installing what the studio installs is the only version of this
    // test that checks the thing it says it checks, in either profile.
    viewwstudio::install(&mut driver);
    driver
        .set_root(vieww_widget::Theme::new(vieww_widget::ThemeData::light()).child(preview.node()));

    // First frame mounts the widget and runs `build()` for the first time —
    // the same call the probe already made. The `BUILDS` counter is now 2
    // (probe + mount), so a rebuild past this point is the second build and
    // panics.
    driver.draw_frame();

    // Force a rebuild of the root and draw again. The host's `ElementTree::build`
    // catches the panic, records it as a build error, and replaces the widget
    // with the studio's error placeholder. The process is still alive.
    if let Some(root) = driver.elements().root() {
        driver.elements().mark_pending(root);
    }
    driver.draw_frame();

    // The frame produced something rather than aborting — that is the
    // assertion. Whether the error placeholder is what was drawn or whether the
    // previous frame is what is showing is a finer detail than this test
    // should pin, because both are recovery and both are correct outcomes.
    let _ = driver.owner().tree().debug_tree();
    // The process reaching this line is the assertion that the host-side catch
    // worked rather than aborting with a foreign-exception error.
}

#[test]
fn a_buffer_with_no_screen_function_is_told_so() {
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x4444).expect("a temp directory");

    let compiled =
        compile(&toolchain, &session, "pub fn nothing() {}", "screen.rs").expect("rustc ran");

    assert!(compiled.failed());
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|d| d.message.contains("screen")),
        "rustc's own message names the missing function: {:?}",
        compiled.diagnostics
    );
}

#[test]
fn the_toolchain_guard_refuses_a_different_compiler() {
    // The guard compares the recorded host version against `rustc --version`.
    // Both come from the same machine here, so what is checkable is that the
    // stamp exists and is the version this test is running under.
    let Some(toolchain) = toolchain() else {
        return;
    };
    assert!(toolchain.version.starts_with("rustc "));
    assert!(toolchain.stamp().starts_with("rustc "));
    assert!(toolchain.vieww_rlib.exists());
}

#[test]
fn a_missing_toolchain_says_what_to_do_about_it() {
    let error =
        Toolchain::discover(Path::new("/nowhere/at/all")).expect_err("there are no rlibs there");
    let message = error.to_string();
    assert!(
        message.contains("cargo build -p vieww"),
        "the error tells you the fix: {message}"
    );
}

// ===================== the worker thread ================================

#[test]
fn a_job_compiles_on_a_worker_and_reports_back() {
    // The async path, end to end: spawn, poll while it runs, and pick up the
    // same `Compiled` the synchronous call would have produced. This is what
    // moving `rustc` off the UI thread bought, so it is what has to be checked
    // — the state machine above it is only as good as this.
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x5555).expect("a temp directory");
    let woken = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let flag = std::sync::Arc::clone(&woken);
    let job = Job::spawn(&toolchain, &session, GOOD, "screen.rs", None, move || {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    })
    .expect("the buffer was written and a thread started");

    // It has to actually be concurrent: if `spawn` blocked, this would be
    // `Done` on the very first look and the whole exercise would be theatre.
    let mut polls = 0u32;
    let compiled = loop {
        match job.poll() {
            Progress::Running => {
                polls += 1;
                assert!(
                    job.elapsed() < std::time::Duration::from_secs(60),
                    "the worker never answered"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Progress::Done(result) => break result.expect("rustc ran"),
            Progress::Lost => panic!("the worker went away without answering"),
        }
    };

    assert!(
        polls > 0,
        "the compile returned before it could have started"
    );
    assert!(
        !compiled.failed(),
        "diagnostics: {:?}",
        compiled.diagnostics
    );
    assert!(
        woken.load(std::sync::atomic::Ordering::SeqCst),
        "without the wake, a finished compile sits in its channel until the \
         user happens to move the mouse"
    );

    let library = compiled.library.as_ref().expect("a library");
    Preview::load(library).expect("it loads and builds");
}

#[test]
fn a_cancelled_job_leaves_no_diagnostic_behind() {
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x6666).expect("a temp directory");
    let job = Job::spawn(&toolchain, &session, GOOD, "screen.rs", None, || {}).expect("spawned");

    job.cancel();
    assert!(job.is_cancelled());

    // Whatever the race, the outcome must never be a red mark on the buffer:
    // the user stopped it, the code did not fail. A timeout is the other case
    // and does get a diagnostic — see `compile::Stopped`.
    loop {
        match job.poll() {
            Progress::Running => std::thread::sleep(std::time::Duration::from_millis(5)),
            Progress::Done(result) => {
                let compiled = result.expect("rustc ran or was killed");
                if compiled.failed() {
                    assert!(
                        compiled.diagnostics.is_empty(),
                        "a cancel is not a problem with the file: {:?}",
                        compiled.diagnostics
                    );
                }
                break;
            }
            Progress::Lost => panic!("the worker went away"),
        }
    }
}

#[test]
fn the_session_lists_what_it_has_actually_written() {
    let session = Session::new(0x7777).expect("a temp directory");
    assert!(
        session.artefacts().is_empty(),
        "a session that has compiled nothing lists nothing — the explorer used \
         to show three invented filenames here"
    );

    std::fs::write(session.directory().join("preview-0001.rs"), "//").expect("write");
    std::fs::write(session.directory().join("preview-0002.rs"), "//").expect("write");
    let listed: Vec<String> = session
        .artefacts()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        listed,
        ["preview-0002.rs", "preview-0001.rs"],
        "newest first, which is where somebody looks for the one they just made"
    );
}

// ===================== what the studio ships with ========================

#[test]
fn the_scratch_buffer_the_studio_opens_with_actually_compiles() {
    // **The test whose absence let a broken default ship.** Every fresh studio
    // opens on this buffer, and pressing Render on it was the first thing
    // anybody would do — `pub struct Screen;` had no `#[derive(Debug)]`, and
    // `Widget: Any + Debug`, so it did not compile. The pipeline tests all used
    // their own `GOOD` constant, which did derive it, so the suite was green
    // and the product was broken.
    //
    // Anything the studio *hands* the user has to be compiled by something.
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0x8888).expect("a temp directory");
    let scratch = viewwstudio::Workspace::scratch();
    let buffer = scratch.buffers.first().expect("the scratch buffer");

    let compiled =
        compile(&toolchain, &session, &buffer.value.text, &buffer.name).expect("rustc ran");

    assert!(
        !compiled.failed(),
        "the buffer every new studio opens on does not compile: {:#?}",
        compiled.diagnostics
    );
    Preview::load(compiled.library.as_ref().expect("a library")).expect("and it mounts");
}

#[test]
fn every_screen_the_repository_ships_compiles() {
    // The same argument, for the three example screens in `screens/`. They are
    // what the studio is pointed at as a workspace, so they are equally part of
    // what it hands somebody.
    let Some(toolchain) = toolchain() else {
        return;
    };
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("screens");
    let Ok(entries) = std::fs::read_dir(&directory) else {
        eprintln!("skipping: no screens directory at {}", directory.display());
        return;
    };

    let session = Session::new(0x9999).expect("a temp directory");
    let mut checked = 0u32;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // **`live.rs` is data, not code.** Its own header says so — "It is NOT
        // compiled and not part of your app: the studio parses it and draws
        // it" — and its body opens with a `live! {` macro that exists only in
        // the studio's Live Preview, not in anything `rustc` can see. The
        // studio writes this file into whatever workspace it has open the
        // first time somebody accepts the Live Preview, so a checkout that
        // has ever run the walkthrough or pressed Live holds it in `screens/`
        // — and this test, sweeping every `.rs` in the directory, used to
        // compile it, fail on the macro, and turn the whole suite red against
        // a file that was never supposed to be here at all.
        if name == "live.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable");
        let compiled = compile(&toolchain, &session, &text, &name).expect("rustc ran");
        assert!(
            !compiled.failed(),
            "{name} does not compile: {:#?}",
            compiled.diagnostics
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "no screens were checked, so nothing was proved"
    );
}

#[test]
fn the_timeout_is_far_past_an_honest_compile() {
    // Measured on this workspace: 10.6s for the first compile of a session with
    // a cold page cache, 0.8s for every one after it. The threshold was ten,
    // which sat on top of the one compile every session is guaranteed to make —
    // and a killed first Render was reported to the user as a const-eval loop
    // in their nine-line hello-world.
    assert!(
        TIMEOUT >= std::time::Duration::from_secs(30),
        "a backstop this tight kills honest compiles and blames the buffer"
    );
}

// ===================== the pane never blanks =============================

#[test]
fn a_failure_after_a_success_keeps_the_frame_and_admits_it_is_stale() {
    // Plan §2.2's actual promise, checked where it can be checked honestly:
    // with a *real* screen mounted, because that is the thing that is supposed
    // to survive. `tests/shell.rs` used to assert this with no screen at all,
    // which is how the pane came to dim an empty phone under "showing last
    // successful render" — a badge claiming a render that never happened.
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0xAAAA).expect("a temp directory");
    let compiled = compile(&toolchain, &session, GOOD, "screen.rs").expect("rustc ran");
    let preview = Preview::load(compiled.library.as_ref().expect("a library")).expect("loads");

    let runtime = vieww_element::Runtime::new();
    let studio = viewwstudio::Studio::new(&runtime);

    // A success: the frame is there and nothing claims it is old.
    studio.preview_screen.set(Some(preview));
    studio.preview.set(viewwstudio::PreviewState::Rendered);
    let good = vieww_widget::debug_tree(viewwstudio::Shell {
        studio: studio.clone(),
    });
    assert!(!good.contains("showing last successful render"));
    assert!(
        !good.contains("No render yet"),
        "the compiled screen is what the frame shows now"
    );

    // Then a failure. The screen stays; the badge appears.
    studio.preview.set(viewwstudio::PreviewState::Failed);
    let stale = vieww_widget::debug_tree(viewwstudio::Shell { studio });
    assert!(
        stale.contains("showing last successful render"),
        "the pane never blanks — the previous frame is still on screen"
    );
    assert!(
        stale.contains("Opacity"),
        "and it is faded, so a glance says stale without reading the badge"
    );
}

// ===================== the preview actually renders ======================

#[test]
fn a_compiled_screen_reaches_the_render_tree() {
    // **The test that was missing, and its absence hid the worst defect in the
    // application.** Everything else here checked that a buffer *compiles* and
    // *loads*. Nothing checked that what it built ever reached a render object
    // — and it did not. The host resolves widgets through a `TypeId`-keyed
    // factory; the guest was linking a different compilation of vieww, so every
    // widget missed, the tree came out empty, and the preview drew nothing
    // while the status bar said `Rendered`.
    //
    // A widget tree that is right and a render tree that is empty look
    // identical from `debug_tree`. This looks at the render tree.
    let Some(toolchain) = toolchain() else {
        return;
    };
    let session = Session::new(0xB0B0).expect("a temp directory");
    let scratch = viewwstudio::Workspace::scratch();
    let buffer = scratch.buffers.first().expect("the scratch buffer");

    let compiled =
        compile(&toolchain, &session, &buffer.value.text, "scratch.rs").expect("rustc ran");
    assert!(!compiled.failed(), "{:#?}", compiled.diagnostics);
    let preview = Preview::load(compiled.library.as_ref().expect("a library"))
        .expect("the chosen candidate is the one this process shares types with");

    let mut driver = vieww_render::FrameDriver::new(vieww_foundation::Size::new(400.0, 300.0));
    // The studio's error policy, for the reason the other panic test gives.
    viewwstudio::install(&mut driver);
    driver
        .set_root(vieww_widget::Theme::new(vieww_widget::ThemeData::light()).child(preview.node()));
    driver.draw_frame();

    let dump = driver.owner().tree().debug_tree();
    assert!(
        dump.contains("RenderText"),
        "the screen's `Text` never became a render object — the preview is \
         blank. Render tree was:\n{dump}"
    );
    assert!(
        dump.contains("RenderColoredBox"),
        "nor did its Container. Render tree was:\n{dump}"
    );
}

#[test]
fn a_preview_built_against_the_wrong_vieww_is_refused_not_shown_blank() {
    // The guard. A workspace holds more than one compilation of vieww, and only
    // one shares this process's types. Loading the wrong one used to succeed
    // and render nothing; it now fails with a sentence saying why.
    let Some(toolchain) = toolchain() else {
        return;
    };
    if toolchain.candidates.len() < 2 {
        eprintln!("skipping: only one vieww rlib in this target directory");
        return;
    }

    let mut refused = 0u32;
    let mut loaded = 0u32;
    for index in 0..toolchain.candidates.len() {
        let candidate = toolchain.with_candidate(index).expect("in range");
        let session = Session::new(0xB100 + index as u64).expect("a temp directory");
        let Ok(compiled) = compile(&candidate, &session, GOOD, "screen.rs") else {
            continue;
        };
        if compiled.failed() {
            continue;
        }
        match Preview::load(compiled.library.as_ref().expect("a library")) {
            Ok(_) => loaded += 1,
            Err(LoadError::AbiMismatch { host, guest }) => {
                assert_ne!(host, guest, "a mismatch that matches is not a mismatch");
                refused += 1;
            }
            Err(other) => panic!("unexpected: {other}"),
        }
    }

    assert!(
        loaded >= 1,
        "no candidate produced a loadable preview, so the studio cannot render \
         at all"
    );
    // **Relaxed, on this test's own instruction.** It used to require at least
    // one refusal, and its own message said that a workspace whose rlibs have
    // all converged is good news and should relax it. That is what happened:
    // several full rebuilds in a row left three `libvieww-*.rlib` files that
    // are the same compilation, so every candidate loads and there is no
    // mismatch left to catch.
    //
    // What still holds unconditionally is the half that is a *correctness*
    // claim rather than an environment one: a refusal, when there is one, is
    // between two genuinely different fingerprints — asserted at the match arm
    // above, where it belongs. Requiring a mismatch to exist made this test a
    // report on the state of `target/`, which is not a property of the studio.
    if refused == 0 {
        eprintln!(
            "note: every candidate loaded — this target directory holds only \
             matching rlibs, so the ABI guard had nothing to refuse"
        );
    }
}

#[test]
fn the_best_candidate_is_tried_first() {
    // `discovered`, not `toolchain`: this is a test *about* the candidate
    // ordering, so it has to see the order `discover` chose rather than the
    // ABI-matched candidate the loading tests are handed.
    let Some(toolchain) = discovered() else {
        return;
    };
    assert!(
        !toolchain.candidates.is_empty(),
        "discover succeeded, so it found at least one"
    );
    assert_eq!(
        toolchain.vieww_rlib, toolchain.candidates[0],
        "the chosen rlib is the head of the candidate list"
    );
    assert!(
        toolchain.vieww_rlib.starts_with(target().join("deps")),
        "the studio's own dependency graph is a better guess than the artifact \
         a separate `cargo build -p vieww` left at the top level"
    );
}

// ---------------------------------------------------------------------------
// The snippet library
// ---------------------------------------------------------------------------

/// Every item snippet the Snippets view offers, through the real compiler.
///
/// # Why this is worth a slow test
///
/// The library is a promise: press this and you get code that works. A snippet
/// that does not compile is worse than a missing one — it puts an error in a
/// file the user did not write and cannot yet read, on their first day. The
/// eleven that were here before this test were never compiled by anything;
/// they were right by inspection, which is a way of saying nobody checked.
///
/// **Items only.** An item is a whole thing — a `fn`, a `struct`, an `impl` —
/// and stands on its own. An expression snippet is a fragment that names
/// `theme`, `items`, `child`: free variables that only mean something at the
/// caret it is inserted at, so compiling one in isolation would assert a
/// context that does not exist. Those are checked for shape instead, in
/// `tests/snippets.rs`.
#[test]
fn every_item_snippet_compiles() {
    let Some(toolchain) = toolchain() else { return };
    let session = Session::new(0x5E_11EE).expect("a session directory");

    for snippet in viewwstudio::edit_ops::SNIPPETS
        .iter()
        .filter(|snippet| snippet.kind == viewwstudio::edit_ops::SnippetKind::Item)
    {
        // Every previewable buffer has to define `screen()` — `compile` writes
        // an entry point that calls it — so one is added for the snippets that
        // are not themselves a `screen`. Detected rather than always appended,
        // because the "Screen entry point" snippet *is* one and two would be a
        // duplicate definition.
        let entry = if snippet.body.contains("fn screen(") {
            String::new()
        } else {
            "\npub fn screen() -> impl Widget {\n    Text::new(\"x\")\n}\n".to_owned()
        };
        let source = format!("use vieww::prelude::*;\n\n{}\n{entry}", snippet.body);
        let compiled = compile(&toolchain, &session, &source, "snippet.rs")
            .unwrap_or_else(|error| panic!("rustc did not run for {}: {error}", snippet.name));

        assert!(
            !compiled.failed(),
            "the {} snippet does not compile:\n{}",
            snippet.name,
            compiled
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .map(|d| format!("  {} {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// Every lesson is supposed to be a complete `pub fn screen() -> impl Widget`
/// source that compiles in the studio's preview. A lesson that does not compile
/// is a broken promise — somebody clicks it, hits Render, and gets errors
/// instead of the picture they were promised — so this runs each one through
/// the real `compile` path the Render button uses.
#[test]
fn every_lesson_compiles() {
    let Some(toolchain) = toolchain() else { return };
    let session = Session::new(0x1E_5100).expect("a session directory");

    for lesson in viewwstudio::lessons::LESSONS {
        let compiled = compile(&toolchain, &session, lesson.body, "lesson.rs")
            .unwrap_or_else(|error| panic!("rustc did not run for {}: {error}", lesson.name));
        assert!(
            !compiled.failed(),
            "the {} lesson does not compile:\n{}",
            lesson.name,
            compiled
                .diagnostics
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .map(|d| format!("  {} {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

// ----- the launch that was never tested --------------------------------------
//
// The suite above runs under `cargo test`, which puts the toolchain's `lib/`
// on `LD_LIBRARY_PATH` for every test binary — so a preview's dependency on
// `libstd-<hash>.so` resolved without anything in the studio earning it. The
// studio launched by a desktop entry or a bare shell has no such variable, and
// the preview that compiled clean died at `dlopen`. This asks the question the
// failing launch asks: with the variable *removed*, does the preview still
// resolve everything it needs?

/// The preview carries the toolchain's `lib/` in its own runpath.
///
/// Two assertions, one file: the sysroot directory appears in the library's
/// bytes (the runpath rustc baked in with `-C rpath`), and the loader itself —
/// asked through `ldd`, with `LD_LIBRARY_PATH` scrubbed — resolves every
/// dependency of the compiled preview. The first is portable; the second is
/// the end-to-end proof on the platform that has `ldd`.
#[test]
#[cfg(unix)]
fn a_preview_resolves_its_libstd_without_the_environment_cargo_sets() {
    let Some(toolchain) = toolchain() else {
        return;
    };
    // **Not `0x3333`.** That is
    // `a_panicking_screen_is_caught_at_the_boundary_rather_than_taking_the_host_down`'s
    // seed, and a `Session`'s directory is named from its seed alone while its
    // `Drop` does `remove_dir_all`. Two tests holding the same seed therefore
    // share one directory, and whichever finishes first deletes the other's
    // sources mid-compile — `rustc` then reports a missing input, `library`
    // comes back `None`, and the `expect` below fails with a message about a
    // clean buffer that has nothing to do with the cause. It failed roughly one
    // run in two under `cargo test`'s default parallelism and passed every time
    // either test was run alone, which is the shape of bug that gets filed as
    // "flaky" and then ignored — on a test whose job is to prove the studio
    // survives a guest panic.
    let session = Session::new(0x33AA).expect("a temp directory");
    let compiled = compile(&toolchain, &session, GOOD, "screen.rs").expect("rustc ran");
    let library = compiled.library.expect("a library from a clean buffer");

    let sysroot = toolchain
        .sysroot_lib
        .as_deref()
        .expect("the compiler that just ran can name its sysroot");
    let needle = sysroot.display().to_string();
    let bytes = std::fs::read(&library).expect("the compiled preview on disk");
    assert!(
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes()),
        "the preview does not carry {needle} in its runpath — a studio \
         launched outside cargo will refuse to dlopen it, exactly as reported"
    );

    // The loader's own answer, with the variable cargo sets for its tests
    // removed. `ldd` is not everywhere; its absence skips this half rather
    // than failing it, because the byte assertion above has already said the
    // runpath is there.
    #[cfg(target_os = "linux")]
    if let Ok(output) = std::process::Command::new("ldd")
        .arg(&library)
        .env_remove("LD_LIBRARY_PATH")
        .output()
    {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !text.contains("not found"),
            "with no LD_LIBRARY_PATH, the preview's dependencies do not \
             resolve: {text}"
        );
    }
}
