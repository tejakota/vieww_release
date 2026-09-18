//! A real guest, rebuilt, reloaded — with the element tree still standing.
//!
//! # Why this file exists
//!
//! Everything else in this crate's suite tests a *part*. `fingerprint.rs`
//! compares two fingerprints that the test itself wrote down; `watch.rs` moves
//! file timestamps around and checks what `poll` makes of them. Both are honest
//! unit tests and neither of them ever calls `dlopen`.
//!
//! So the claim the crate is *for* — "the guest is rebuilt, the host loads the
//! new one, and the element tree with every signal and scroll offset in it stays
//! where it was" — was checked by a person running `examples/reload-host` and
//! looking at a window, and by nothing in CI at all. The README states it in
//! bold. `docs/HOT-RELOAD.md` is a design document, not a test. A regression in
//! `Guest::load`, in the `guest!` macro's symbol names, or in the ABI between
//! host and guest would have gone out with a green suite.
//!
//! This builds the real `reload-guest` crate with cargo, loads the real
//! `cdylib`, edits it, rebuilds it, and asserts on what came back across the
//! library boundary. No window, so it runs where CI runs.
//!
//! # It is slow, and it is `#[ignore]`d for it
//!
//! Two cargo builds of a crate that depends on `vieww-widget`. That is minutes,
//! not milliseconds, and putting it in the default set would make `cargo test`
//! something people stop running.
//!
//! ```console
//! cargo test -p vieww-reload --test end_to_end -- --ignored --test-threads=1
//! ```
//!
//! **`--test-threads=1` is not optional, and leaving it off is why this
//! command was flaky.** Both tests below shell out to cargo to build the same
//! guest crate into the same target directory. Run in parallel they race for
//! cargo's build lock: one blocks for minutes, or — worse — both succeed and
//! one loads a `cdylib` the other was midway through rewriting, which fails as
//! a wrong symbol rather than as a lock error. Every place in this repository
//! that names this command names the flag; `ci/check/checks.sh` always had it and
//! the README and this header did not, which is the whole of the flake.
//!
//! `ci/check/checks.sh` runs it. A test nobody runs is worth nothing, which is why
//! `#[ignore]` here comes with the line above and an entry in the check script
//! rather than on its own.

use std::path::{Path, PathBuf};
use std::process::Command;

use vieww_reload::{Reloaded, Reloader};

/// The workspace root, from this crate's manifest directory.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/<name>/ is two below the root")
        .to_path_buf()
}

/// Build `reload-guest` into `target_dir`, and return the `cdylib`.
///
/// A target directory of its own, passed explicitly: the test rewrites the
/// guest's source between builds, and sharing the workspace's `target/` would
/// mean this test's second build invalidating whatever else was compiled there
/// — including, on a developer's machine, the thing they were about to run.
fn build_guest(target_dir: &Path) -> PathBuf {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(workspace())
        .env("CARGO_TARGET_DIR", target_dir)
        // Inherited so a machine with no default toolchain — a CI image that
        // pins one per invocation — resolves the same compiler this test was
        // built by rather than none at all.
        .args(["build", "-p", "reload-guest"])
        .output()
        .expect("cargo runs");
    assert!(
        output.status.success(),
        "building the guest failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let library = target_dir
        .join("debug")
        .join(if cfg!(target_os = "windows") {
            "reload_guest.dll"
        } else if cfg!(target_os = "macos") {
            "libreload_guest.dylib"
        } else {
            "libreload_guest.so"
        });
    assert!(
        library.exists(),
        "cargo reported success but there is no cdylib at {}",
        library.display()
    );
    library
}

/// The guest's source, and a scope that puts it back however the test ends.
///
/// The file is a checked-in example that a person may have open. Restoring it
/// on `Drop` rather than at the end of the test body means a failed assertion
/// — which unwinds — does not leave the working tree edited.
struct GuestSource {
    path: PathBuf,
    original: String,
}

impl GuestSource {
    fn open() -> Self {
        let path = workspace()
            .join("examples")
            .join("reload-guest")
            .join("src")
            .join("lib.rs");
        let original = std::fs::read_to_string(&path).expect("the guest example is checked in");
        Self { path, original }
    }

    /// Rewrite the guest with `from` replaced by `to`.
    ///
    /// **The search runs against a `\n`-normalised copy.** The landmarks below
    /// span two lines, and a Windows checkout can hold the guest with `\r\n`
    /// — `.gitattributes` asks for LF, but a clone made before it existed
    /// still has CRLF — so the landmark was not found and the test failed
    /// saying the example had changed when it had not. `Drop` still restores
    /// the file's original bytes, whichever they were.
    fn replace(&self, from: &str, to: &str) {
        let normalised = self.original.replace("\r\n", "\n");
        assert!(
            normalised.contains(from),
            "the guest example no longer contains `{from}` — this test edits it \
             to force a rebuild, and needs a landmark that is actually there"
        );
        std::fs::write(&self.path, normalised.replace(from, to)).expect("writing the guest");
    }
}

impl Drop for GuestSource {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.path, &self.original);
    }
}

/// **The thesis, end to end.** A cosmetic edit to the guest produces a
/// `Reloaded::Root` — the verdict that means "swap the root, keep the tree".
///
/// The colour constant is the edit `examples/reload-guest`'s own documentation
/// tells a reader to make first, so this test is the automated form of the
/// thing the example asks a person to try.
#[test]
#[ignore = "builds the guest crate twice; run with --ignored"]
fn a_rebuilt_guest_reloads_and_the_tree_is_kept() {
    let scratch = tempdir("reload-e2e-keep");
    let source = GuestSource::open();

    let library = build_guest(&scratch);
    let mut reloader = Reloader::new(&library).expect("the first load");

    // Nothing has been rebuilt, so nothing is reported. This also proves the
    // watcher does not treat its own first sighting as a change — the bug
    // `watch::tests::the_first_poll_after_start_up_is_not_a_rebuild` pins in
    // the unit suite, here against a real file cargo actually wrote.
    assert!(
        matches!(reloader.poll(), Reloaded::Nothing),
        "a guest nobody has touched must not report a reload"
    );

    // The edit the example's docs suggest: a different colour, same state.
    source.replace(
        "const BAND: Color = Color::rgb(186, 217, 212);",
        "const BAND: Color = Color::rgb(240, 120, 60);",
    );
    build_guest(&scratch);

    match reloader.poll() {
        Reloaded::Root(root) => {
            // It came across the boundary as a real tree, not a null pointer
            // wearing one's name.
            assert!(
                !root.debug_name().is_empty(),
                "the reloaded root has no debug name, so it did not survive the \
                 trip across the library boundary intact"
            );
        }
        Reloaded::Restart(_) => panic!(
            "a colour constant changed and the host tore the tree down — the \
             fingerprint is reporting a state change that did not happen, and \
             every reload would silently reset the application's state"
        ),
        Reloaded::Failed(error) => panic!("the rebuilt guest would not load: {error}"),
        Reloaded::Nothing => panic!(
            "cargo rewrote the library and the watcher did not notice — the \
             whole loop hangs on this"
        ),
    }
}

/// And the other half: a guest whose **state changed shape** must come back as
/// `Restart`, not `Root`.
///
/// This is the assertion that separates a reload from undefined behaviour. If
/// the fingerprint ever stops noticing, the host reads an old allocation with a
/// new layout and the failure is memory corruption in someone's editor — so it
/// is worth paying a third cargo build to check against a real library rather
/// than against two fingerprints a test made up.
#[test]
#[ignore = "builds the guest crate twice; run with --ignored"]
fn a_guest_whose_state_changed_shape_forces_a_restart() {
    let scratch = tempdir("reload-e2e-restart");
    let source = GuestSource::open();

    let library = build_guest(&scratch);
    let mut reloader = Reloader::new(&library).expect("the first load");

    // The second edit the example's docs suggest: a new field in `Taps`, which
    // changes its size and therefore its fingerprint.
    source.replace(
        "pub struct Taps {\n    count: u32,",
        "pub struct Taps {\n    count: u32,\n    added_by_the_end_to_end_test: u64,",
    );
    build_guest(&scratch);

    match reloader.poll() {
        Reloaded::Restart(_) => {}
        Reloaded::Root(_) => panic!(
            "a state type grew by eight bytes and the host said keep the tree — \
             that is an old allocation read with a new layout, which is the one \
             outcome this crate's fingerprint exists to prevent"
        ),
        Reloaded::Failed(error) => panic!("the rebuilt guest would not load: {error}"),
        Reloaded::Nothing => panic!("the watcher missed a rebuild"),
    }
}

/// **A guest that panics while building leaves the previous one running.**
///
/// The claim `Reloaded::Failed` has always made — *"the old one is still
/// running"* — and the one case where it was not true. `__vieww_guest_root` is
/// `extern "C"`, and a panic reaching an `extern "C"` boundary aborts the
/// process: not an error the host could report, not a screen it could keep, but
/// the whole window gone. A developer mid-edit is exactly the person whose
/// `unwrap` is about to fail, and losing the session they had built up to reach
/// the screen they were editing is the opposite of what a hot reloader is for.
///
/// The abort is not observable from inside a test — it takes the test runner
/// with it — so what this asserts is the *absence* of it: the poll returns, it
/// returns `Failed`, and the reloader still hands back the previous build's
/// root afterwards. Before the guard in `guest!`, this test did not fail; the
/// process died and the suite reported a signal.
#[test]
#[ignore = "builds the guest crate twice; run with --ignored"]
fn a_guest_that_panics_while_building_does_not_take_the_host_down() {
    let scratch = tempdir("reload-e2e-panic");
    let source = GuestSource::open();

    let library = build_guest(&scratch);
    let mut reloader = Reloader::new(&library).expect("the first load");
    let before = reloader
        .root()
        .expect("the first build is fine")
        .debug_name();

    // A build that compiles and panics on its way to a widget tree — the shape
    // of an ordinary mistake, not a contrived one: an index into an empty
    // collection while wiring up a screen.
    source.replace(
        "fn screen() -> impl Into<WidgetNode> {",
        "fn screen() -> impl Into<WidgetNode> {\n    let empty: Vec<u8> = Vec::new();\n             assert!(!empty.is_empty(), \"a panic on the way to a widget tree\");",
    );
    build_guest(&scratch);

    // Reaching this line at all is most of the test.
    match reloader.poll() {
        Reloaded::Failed(error) => {
            let message = error.to_string();
            assert!(
                message.contains("panicked"),
                "the failure has to name what happened, so a developer reads \
                 `panicked while building` rather than a symbol error: {message}"
            );
        }
        Reloaded::Root(_) | Reloaded::Restart(_) => panic!(
            "a guest that panicked while building produced a root — the null \
             the guard returns is being read as a widget tree"
        ),
        Reloaded::Nothing => panic!("the watcher missed a rebuild"),
    }

    // And the previous build is still the one loaded.
    assert_eq!(
        reloader
            .root()
            .expect("the previous build still builds")
            .debug_name(),
        before,
        "the failed reload replaced the running guest"
    );
}

/// A scratch directory that is this test's alone, removed if it was left over
/// **and removed again when the test ends**, pass or fail.
///
/// Each one is a complete cargo target directory for `reload-guest` and its
/// dependencies — gigabytes with debuginfo — and the process id in the name
/// means no later run ever reused, let alone deleted, an earlier one. Three per
/// run of this suite accumulated in the system temp directory, which on a
/// laptop is the same disk the release gate was already running out of.
fn tempdir(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("vieww-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    Scratch(dir)
}

/// Removes its directory on drop, including when a test panics.
struct Scratch(PathBuf);

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
