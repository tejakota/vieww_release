//! M3: the buffer, through `rustc`, into a loadable library.
//!
//! One `rustc` invocation per Render click — not `cargo`, which would resolve a
//! dependency graph the studio already has resolved — against the `.rlib`s the
//! host itself was built from. That is the whole reason a compile here takes a
//! fraction of a second rather than the twenty a cold `cargo build` takes: the
//! only crate being compiled is the one in the editor.
//!
//! # The toolchain has to be the *same* toolchain
//!
//! A `cdylib` built by a different `rustc`, or against a different build of
//! vieww, is not merely likely to misbehave — `Box<dyn Widget>` crossing that
//! boundary is undefined behaviour, because the vtable layout and the `TypeId`s
//! are compilation-session artefacts. [`Toolchain::discover`] therefore records
//! what the host was built with and refuses to load anything compiled against
//! anything else (plan §8, "toolchain mismatch guard").
//!
//! # Where the files go
//!
//! One temp directory per session, one numbered `.rs`/`.so` pair per Render,
//! removed when the process exits. Numbered rather than reused because a
//! library that has been `dlopen`ed is never unloaded (see [`crate::loaded`]),
//! so overwriting the file it was loaded from is asking for the loader to
//! disagree with the filesystem.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::state::{Diagnostic, Severity};

/// How long a compile may run before it is killed.
///
/// # This was ten seconds, and ten seconds was wrong
///
/// The plan's open question (§8) asked for a kill threshold, and the answer
/// written down was ten — chosen against a measured "~0.2s for a screen here".
/// That measurement was taken with a **warm page cache**. Measured properly on
/// this workspace, the same compile costs:
///
/// | | |
/// |---|---|
/// | first compile of a session (cold cache, ~100 rlibs) | **10.6s** |
/// | every compile after it | **0.8s** |
///
/// So the threshold sat almost exactly on top of the one compile that every
/// session is guaranteed to perform. The first Render after launch was a coin
/// flip, and when it lost, the studio blamed the user's code: *"rustc was still
/// running after 10s and was killed — a const-eval loop or a very large macro
/// expansion can do this"*, against a nine-line hello-world.
///
/// # Why sixty, and why it can afford to be generous
///
/// The original argument for a tight bound was that `rustc` ran on the UI
/// thread, so a long compile *was* a hung window. That is no longer true: the
/// compile runs on a worker (see [`Job`]), the window keeps painting, and
/// [`Command::CancelRender`] is on the toolbar and on ⇧⌘⏎. The timeout stopped
/// being the thing that protects responsiveness and became what it should
/// always have been — a backstop against a process that will never finish.
///
/// A backstop should sit far past anything legitimate. Sixty seconds is roughly
/// six times the worst honest compile measured here and still far short of any
/// reading of "this is never coming back".
///
/// [`Command::CancelRender`]: crate::command::Command::CancelRender
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// What the host was built with, and where to find it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// The `rustc` that compiles a preview.
    pub rustc: PathBuf,
    /// The umbrella crate the buffer writes against.
    pub vieww_rlib: PathBuf,
    /// Every `vieww` rlib in the target directory, best guess first.
    ///
    /// There is usually more than one, and picking the wrong one renders a
    /// blank preview rather than failing — see [`vieww_candidates`].
    pub candidates: Vec<PathBuf>,
    /// Where its transitive `.rlib`s live.
    pub deps: PathBuf,
    /// `rustc --print sysroot`'s `lib/` directory, if the compiler would say.
    ///
    /// # What this is for
    ///
    /// A preview compiled `-C prefer-dynamic` depends on the toolchain's
    /// `libstd-<hash>.so`, and `dlopen` has to be able to *find* that file.
    /// Under `cargo run` it can — cargo puts the sysroot's `lib/` on
    /// `LD_LIBRARY_PATH` — but a studio started any other way (a desktop
    /// entry, a double click, a shell without the toolchain exported) gives
    /// the loader nowhere to look, and the Render that compiled clean dies
    /// with `libstd-<hash>.so: cannot open shared object file`.
    ///
    /// Two things use it: `run_rustc` bakes the directory into the preview
    /// itself with `-C rpath`, and [`Toolchain::prime_libstd`] maps the file
    /// into the process before the first `dlopen` so a preview compiled
    /// before the rpath existed still loads. Neither is reachable through
    /// the other, because one acts on libraries not yet compiled and the
    /// other on ones already on disk.
    ///
    /// See `sysroot_lib` for which directory this is on a rustup toolchain
    /// versus a flat one.
    pub sysroot_lib: Option<PathBuf>,
    /// `rustc --version`, verbatim.
    pub version: String,
}

/// The open project's own compiled library, when the preview can link it.
///
/// # The limitation this exists to remove
///
/// The preview compiles the buffer as a standalone `cdylib` with exactly one
/// `--extern`: `vieww`. That is right for the three screens the studio ships
/// with, and it is why a screen out of a *real* application cannot be
/// previewed at all — `snapsearch/src/screens/library.rs` opens with six
/// `use crate::…` lines, and none of them can resolve in a crate that consists
/// of one file. The studio could open any project, edit any file in it and
/// build it, and could preview only files that happened to depend on nothing.
///
/// With this, the buffer is compiled with the project's own rlib on the extern
/// list, and `crate::` is rewritten to point at it — see [`with_project`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLib {
    /// The library target's crate name, as the linker sees it: `my-app` is
    /// `my_app`.
    pub crate_name: String,
    /// The `.rlib` itself.
    pub rlib: PathBuf,
    /// The project's own `target/<profile>/deps`, so the rlib's dependencies
    /// resolve.
    pub deps: PathBuf,
    /// The `vieww` rlib **the project itself was built against**, if there is
    /// one in its dependency directory.
    ///
    /// # Why this has to be checked rather than assumed
    ///
    /// Linking the project's library into a preview brings the project's whole
    /// dependency graph with it, `vieww` included. If that `vieww` is a
    /// different compilation from the one the studio forces on the guest — a
    /// different metadata hash, which is what a project with its own
    /// `target/` directory always has — then rustc refuses the project rlib
    /// outright, and says `can't find crate for \`snapsearch\`` while the crate
    /// is sitting on the command line. That error is unreadable: it names the
    /// wrong crate and gives no hint that two builds of a third one are the
    /// reason.
    ///
    /// So the studio compares the two before compiling and explains the real
    /// problem. See `Studio::project_lib_refusal`.
    pub vieww_rlib: Option<PathBuf>,
}

impl ProjectLib {
    /// Find the most recently built rlib for the library target in `root`.
    ///
    /// # Why the newest rather than the only one
    ///
    /// Cargo leaves every rlib it has ever built in `deps/`, each with its own
    /// metadata hash — `libsnapsearch-3f2a….rlib` beside
    /// `libsnapsearch-91bc….rlib`. Only one of them matches the current
    /// dependency graph, and the newest is that one: any change that produces a
    /// different hash also produces a fresh file. Picking an older one links a
    /// stale library and fails with type errors that make no sense against the
    /// code on screen.
    #[must_use]
    pub fn discover(root: &Path, crate_name: &str, studio_deps: &Path) -> Option<Self> {
        let lib_name = crate_name.replace('-', "_");
        // **The studio's own dependency directory is searched first**, and that
        // ordering is the difference between a preview that works and one that
        // refuses.
        //
        // A project built with `CARGO_TARGET_DIR` pointing at the studio's
        // target directory puts its rlib *there*, beside the very `vieww` the
        // preview links — which is the one arrangement where the screen, the
        // project and the studio are one build of vieww and the preview can
        // actually mount the result. A project also has its own `target/` from
        // ordinary `cargo build`s, holding an rlib built against a different
        // vieww; finding that one first means refusing a project that had
        // already been set up correctly.
        let mut directories = vec![studio_deps.to_path_buf()];
        directories.push(root.join("target").join("debug").join("deps"));
        directories.push(root.join("target").join("release").join("deps"));

        for deps in directories {
            let Ok(entries) = std::fs::read_dir(&deps) else {
                continue;
            };
            // Two shapes, because cargo writes both: `libfoo-<hash>.rlib` for a
            // dependency it may hold several versions of, and a plain
            // `libfoo.rlib` for the project's own library target. Matching only
            // the hashed form finds nothing for exactly the crate this is for.
            let hashed = format!("lib{lib_name}-");
            let plain = format!("lib{lib_name}.rlib");
            let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "rlib") {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !name.starts_with(&hashed) && name != plain {
                    continue;
                }
                let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
                    continue;
                };
                if best.as_ref().is_none_or(|(best, _)| modified > *best) {
                    best = Some((modified, path));
                }
            }
            if let Some((_, rlib)) = best {
                let vieww_rlib = newest_matching(&deps, "libvieww-");
                return Some(Self {
                    crate_name: lib_name,
                    rlib,
                    deps,
                    vieww_rlib,
                });
            }
        }
        None
    }
}

/// The newest file in `dir` whose name starts with `prefix` and ends `.rlib`.
fn newest_matching(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rlib") {
            continue;
        }
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix))
        {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(best, _)| modified > *best) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

/// The directory holding the `libstd-<hash>.so` the preview will depend on.
///
/// Asked once, at [`Toolchain::discover`] time, because it costs a pair of
/// subprocesses and nothing about it changes while the studio runs. A bundled
/// compiler answers with the bundle's own directory (`<sdk>/toolchain/lib`,
/// exactly where `packaging/package.sh` put it) and a `PATH` compiler answers
/// with the rustup sysroot — either way, the directory holding the
/// `libstd-<hash>.so` that `-C prefer-dynamic` makes every preview depend on.
///
/// # Why the host triple, and why both directories
///
/// The preview is compiled *without* `--target`, so the `std` it links is the
/// one rustc keeps for its own host triple — on a rustup toolchain that is
/// `<sysroot>/lib/rustlib/<host>/lib`, and the plain `<sysroot>/lib` holds no
/// `libstd` at all (only `librustc_driver` and the compiler's own libraries).
/// A distro or hand-built toolchain may still use the flat layout, so both are
/// tried, rustlib first because that is where every rustup install — the
/// common case by far — actually keeps it.
///
/// The triple comes from `rustc -vV` rather than from this studio's build
/// script, because the question is "where does *that compiler* keep the std it
/// links for host builds", and the compiler is the one thing that answers it
/// without being wrong across a cross-compile.
///
/// `None` when the compiler will not say or neither directory holds a libstd:
/// a refusal here should degrade to the old failure mode rather than panic
/// about a directory nobody promised.
fn sysroot_lib(rustc: &Path) -> Option<PathBuf> {
    let version_verbose = Command::new(rustc).args(["-vV"]).output().ok()?;
    if !version_verbose.status.success() {
        return None;
    }
    let host = String::from_utf8_lossy(&version_verbose.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)?;

    let sysroot = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()?;
    if !sysroot.status.success() {
        return None;
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());

    let rustlib = sysroot
        .join("lib")
        .join("rustlib")
        .join(host.trim())
        .join("lib");
    let flat = sysroot.join("lib");
    [rustlib, flat].into_iter().find(|lib| holds_libstd(lib))
}

/// Whether `dir` holds a `libstd-<hash>.so` (or `.dylib`).
///
/// The check rather than the bare `is_dir`: a directory that exists but holds
/// no libstd is not a place the preview's dependency can come from, and
/// recording it would make [`Toolchain::prime_libstd`] a no-op that looks like
/// a fix.
fn holds_libstd(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().starts_with("libstd-"))
        })
        .unwrap_or(false)
}

/// Rewrite a buffer so it can be compiled on its own against the project's
/// library.
///
/// Two substitutions, and both are textual on purpose — a real parse would mean
/// carrying a Rust front end to answer a question that a substitution answers
/// correctly for every line of Rust anybody writes in a screen file:
///
/// * `crate::` becomes `<crate_name>::`, because in a one-file `cdylib`
///   `crate::` means *this file*, and every `use crate::` in a project file
///   means the project.
/// * `use super::` becomes `use <crate_name>::` for the same reason, which
///   covers the sibling-module imports a `screens/` directory is full of.
///
/// **Line numbers are preserved**, which is the property diagnostics depend
/// on: neither substitution adds or removes a newline, so an error rustc
/// reports on line 40 is on line 40 of what the user is looking at. Columns can
/// shift on a rewritten line, and that is the stated cost.
///
/// It will also rewrite `crate::` inside a string literal or a comment. In a
/// comment that is invisible; in a string it would change a `&'static str`
/// nobody previews. Named here rather than discovered later.
#[must_use]
pub fn with_project(source: &str, crate_name: &str) -> String {
    source
        .replace("use super::", &format!("use {crate_name}::"))
        .replace("crate::", &format!("{crate_name}::"))
}

/// Why a toolchain could not be put together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainError {
    /// No `rustc` on `PATH`, or it would not run.
    NoRustc(String),
    /// The vieww `.rlib`s are not where the studio expected.
    NoRlibs(PathBuf),
    /// The host and the compiler disagree about what `rustc` this is.
    Mismatch { host: String, found: String },
    /// The bundle the studio ships does not describe itself consistently.
    ///
    /// Separate from [`Self::Mismatch`] because the fix is different: a
    /// mismatch is "your `PATH` has the wrong compiler on it", which the user
    /// can put right, and this is "the thing you installed is not intact",
    /// which they cannot.
    Sdk(crate::sdk::SdkError),
}

impl std::fmt::Display for ToolchainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRustc(why) => write!(f, "no usable rustc: {why}"),
            Self::NoRlibs(path) => write!(
                f,
                "vieww was not built where the studio expected it ({}). \
                 Run `cargo build -p vieww` first.",
                path.display()
            ),
            Self::Mismatch { host, found } => write!(
                f,
                "toolchain mismatch: the studio was built with {host}, \
                 and rustc on PATH is {found}. Loading a library built by a \
                 different compiler is undefined behaviour, so the preview is \
                 refused rather than risked."
            ),
            Self::Sdk(error) => write!(f, "the bundled vieww SDK is not usable: {error}"),
        }
    }
}

/// The `rustc` version the *host* was compiled by, recorded at build time.
///
/// `option_env!` rather than `env!` so the studio still builds outside a Cargo
/// invocation that sets it; an unknown host version disables the guard and says
/// so rather than pretending to check.
const HOST_RUSTC: Option<&str> = option_env!("VIEWWSTUDIO_RUSTC");

impl Toolchain {
    /// Find `rustc` and the vieww `.rlib`s, and check they belong together.
    ///
    /// `target` is the directory holding `libvieww.rlib` and `deps/` — the
    /// workspace's `target/debug` in a development checkout.
    ///
    /// # Errors
    ///
    /// If `rustc` will not run, if the `.rlib`s are missing, or if the two do
    /// not match what the host was built with.
    pub fn discover(target: &Path) -> Result<Self, ToolchainError> {
        // **The bundled compiler first.** A packaged studio ships the `rustc`
        // it was built by, and using it is what makes host and guest one
        // compilation *by construction* rather than by coincidence. `PATH` is
        // the fallback for a development checkout, where the coincidence is
        // real and `HOST_RUSTC` is what checks it.
        let bundled = crate::sdk::rustc_path(target);
        let rustc = bundled.clone().unwrap_or_else(|| PathBuf::from("rustc"));

        let output = Command::new(&rustc)
            .arg("--version")
            .output()
            .map_err(|error| ToolchainError::NoRustc(error.to_string()))?;

        if !output.status.success() {
            return Err(ToolchainError::NoRustc(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // **The bundle is asked before `PATH` is judged.** Both checks can
        // catch a wrong compiler, and they differ only in the sentence they
        // produce — so the more specific one goes first. "The SDK says it was
        // built by 1.95 but the compiler in it reports 1.93" tells somebody
        // their install is damaged; `Mismatch` would have told them to change
        // their `PATH`, which for a bundled compiler is not the problem and
        // not the fix.
        //
        // A missing manifest is not an error: a development checkout has none,
        // and its absence is the statement that there is nothing here to
        // verify rather than a claim that failed.
        let manifest = match crate::sdk::Manifest::read(target) {
            Ok(manifest) => {
                manifest
                    .verify(
                        crate::sdk::host_triple(),
                        bundled.as_ref().map(|_| version.as_str()),
                    )
                    .map_err(ToolchainError::Sdk)?;
                Some(manifest)
            }
            Err(crate::sdk::SdkError::Missing(_)) => None,
            Err(error) => return Err(ToolchainError::Sdk(error)),
        };

        if let Some(host) = HOST_RUSTC {
            if host.trim() != version {
                return Err(ToolchainError::Mismatch {
                    host: host.trim().to_string(),
                    found: version,
                });
            }
        }

        let deps = target.join("deps");
        let mut candidates = vieww_candidates(target);

        // **The manifest ends the guessing.** `vieww_candidates` orders by
        // mtime because, scavenging somebody else's target directory, that is
        // the best available signal. A bundle *knows* which rlib the studio
        // binary linked, so when it says so that one goes first and the
        // fallback list stops being load-bearing.
        if let Some(named) = manifest.as_ref().map(|m| target.join(&m.rlib)) {
            if named.is_file() {
                candidates.retain(|path| path != &named);
                candidates.insert(0, named);
            }
        }

        let Some(vieww_rlib) = candidates.first().cloned() else {
            return Err(ToolchainError::NoRlibs(target.join("libvieww.rlib")));
        };
        if !deps.exists() {
            return Err(ToolchainError::NoRlibs(deps));
        }

        Ok(Self {
            sysroot_lib: sysroot_lib(&rustc),
            rustc,
            vieww_rlib,
            candidates,
            deps,
            version,
        })
    }

    /// Map the toolchain's `libstd` into this process, before a preview needs it.
    ///
    /// The belt to `run_rustc`'s braces: a preview already on disk —
    /// compiled by an older studio, or by a hand-run `rustc` — carries no
    /// runpath, and loading it still needs `libstd-<hash>.so` findable.
    /// Mapping the file into this process first satisfies the dependency
    /// without searching: the loader matches the preview's `DT_NEEDED`
    /// against the names already loaded before it looks at any directory.
    ///
    /// Leaked rather than closed for the same reason every preview is (see
    /// the module header), and idempotent — the second call is a `dlopen` of
    /// a name already in the link map, which refcounts and returns.
    pub fn prime_libstd(&self) {
        if let Some(lib) = self.sysroot_lib.as_deref() {
            crate::loaded::prime_libstd(lib);
        }
    }

    /// The same toolchain pointed at a different candidate `vieww` rlib.
    ///
    /// What [`crate::state::Studio`] does when the fingerprint says the one it
    /// tried belongs to a different compilation than the host links.
    #[must_use]
    pub fn with_candidate(&self, index: usize) -> Option<Self> {
        let rlib = self.candidates.get(index)?.clone();
        Some(Self {
            vieww_rlib: rlib,
            ..self.clone()
        })
    }

    /// The same toolchain, forced to one specific `vieww` rlib.
    ///
    /// # Why the project gets to choose
    ///
    /// The candidate list exists because a workspace produces several `vieww`
    /// rlibs and only one of them shares a `TypeId` space with the studio — see
    /// [`vieww_candidates`]. When the buffer also links the *project's*
    /// library, there is a second constraint and it is a hard one: the project
    /// was compiled against exactly one `vieww`, and a guest compiled against
    /// any other cannot link it at all. rustc reports that as `can't find crate
    /// for <project>`, which names the wrong crate.
    ///
    /// So the project's choice wins, and the ABI fingerprint in [`ENTRY`] is
    /// what then reports the remaining question — whether the studio agrees
    /// with the two of them — as the one thing it can be: a clear refusal
    /// rather than a preview that mounts nothing.
    #[must_use]
    pub fn with_vieww(&self, rlib: PathBuf) -> Self {
        Self {
            vieww_rlib: rlib,
            ..self.clone()
        }
    }

    /// Whether the compiler being used is the one the studio shipped.
    ///
    /// The difference between a guarantee and a coincidence, so it is worth
    /// being able to see: bundled means host and guest are one compilation by
    /// construction, and anything else means the studio checked that `PATH`
    /// happened to hold the right compiler and will keep checking.
    #[must_use]
    pub fn is_bundled(&self) -> bool {
        self.rustc.is_absolute()
            && self
                .rustc
                .parent()
                .and_then(Path::parent)
                .is_some_and(|dir| dir.ends_with(crate::sdk::TOOLCHAIN_DIR))
    }

    /// A one-line stamp for the status bar.
    #[must_use]
    pub fn stamp(&self) -> String {
        let version = self
            .version
            .split_whitespace()
            .nth(1)
            .map_or_else(|| self.version.clone(), |v| format!("rustc {v}"));
        if self.is_bundled() {
            format!("{version} · bundled")
        } else {
            version
        }
    }
}

/// Every `vieww` rlib worth trying, best first.
///
/// # Why there is a *list* rather than a path
///
/// A workspace routinely holds more than one compilation of a crate: cargo
/// rebuilds a dependency whenever feature resolution differs, so
/// `target/debug/deps` ends up with `libvieww_widget-<a>.rlib` **and**
/// `libvieww_widget-<b>.rlib`. Both are valid; they are different types.
///
/// The studio was linking one and compiling the buffer against the other, and
/// the symptom was not an error — `TypeId`s differed, the host's factory missed
/// every widget the guest built, and the preview drew nothing while the status
/// bar said `Rendered`. Six months of "why is my screen blank".
///
/// So: gather the candidates, try the most likely, and **verify** what comes
/// back ([`crate::loaded::Preview::load`] compares fingerprints). Newest first,
/// because the studio's own graph is rebuilt whenever the studio is, and
/// `target/debug/libvieww.rlib` last, because that one is produced by a
/// separate `cargo build -p vieww` under its own feature resolution and is the
/// least likely to match.
#[must_use]
pub fn vieww_candidates(target: &Path) -> Vec<PathBuf> {
    let mut from_deps: Vec<(std::time::SystemTime, PathBuf)> =
        std::fs::read_dir(target.join("deps"))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                let Some(name) = path.file_name().map(|n| n.to_string_lossy()) else {
                    return false;
                };
                name.starts_with("libvieww-") && name.ends_with(".rlib")
            })
            .map(|path| {
                let at = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (at, path)
            })
            .collect();
    // Newest first: `sort_by_key` then reverse, because the key is the
    // timestamp and clippy is right that a comparator here says less.
    from_deps.sort_by_key(|(at, _)| *at);
    from_deps.reverse();

    let mut out: Vec<PathBuf> = from_deps.into_iter().map(|(_, path)| path).collect();
    let umbrella = target.join("libvieww.rlib");
    if umbrella.exists() {
        out.push(umbrella);
    }
    out
}

/// What a Render click produced.
#[derive(Debug)]
pub struct Compiled {
    /// The library, if `rustc` produced one.
    pub library: Option<PathBuf>,
    /// Every JSON record `rustc` emitted, verbatim.
    pub json: Vec<String>,
    /// Everything `rustc` said, errors and warnings alike.
    pub diagnostics: Vec<Diagnostic>,
    /// The command line, and the outcome, for the Output panel.
    pub log: Vec<String>,
    /// How long `rustc` took.
    pub duration: Duration,
}

impl Compiled {
    /// Whether anything `rustc` said was fatal.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.library.is_none()
    }
}

/// The temp directory a session compiles into, and the counter that names its
/// files.
#[derive(Debug)]
pub struct Session {
    /// Kept so `Drop` can release the claim `new` took on it.
    seed: u64,
    directory: PathBuf,
    next: std::cell::Cell<u32>,
}

/// The seeds of every `Session` alive in this process.
///
/// See [`Session::new`] for what sharing one costs. A `Mutex` rather than a
/// thread-local: the collision this exists to catch is *between* threads.
fn live_seeds() -> &'static std::sync::Mutex<std::collections::HashSet<u64>> {
    static LIVE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u64>>> =
        std::sync::OnceLock::new();
    LIVE.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

impl Session {
    /// Make a fresh directory for this run of the studio.
    ///
    /// # Errors
    ///
    /// If the directory cannot be created.
    pub fn new(seed: u64) -> std::io::Result<Self> {
        // Named from a seed the caller supplies rather than from a clock,
        // because `SystemTime::now` in a constructor is the thing that makes a
        // type impossible to test twice with the same result.
        let directory = std::env::temp_dir().join(format!("vieww-studio-{seed:x}"));

        // **Two live sessions may not share a seed.** The directory is named
        // from the seed and nothing else, and `Drop` does `remove_dir_all` on
        // it — so a second session on the same seed does not get its own space,
        // it gets a co-tenant, and the first one to finish deletes the other's
        // sources while `rustc` is still reading them.
        //
        // That is not hypothetical. Two tests in `tests/pipeline.rs` were both
        // on `0x3333`; under `cargo test`'s default parallelism one of them
        // failed about half the time, with `library: None` and a message about
        // a clean buffer that pointed nowhere near the cause, and both passed
        // whenever either was run alone.
        //
        // Refused here rather than left to the filesystem, because the
        // filesystem's answer to "this directory already exists" is yes, and by
        // the time the damage shows up it looks like a compiler error. A
        // *sequential* reuse of a seed is still fine: the guard is keyed on
        // live sessions, and `Drop` releases it.
        if !live_seeds().lock().is_ok_and(|mut live| live.insert(seed)) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "a session on seed {seed:#x} is already live at {} — two \
                     sessions on one seed share a directory and delete each \
                     other's work; give this one its own seed",
                    directory.display()
                ),
            ));
        }

        if let Err(error) = std::fs::create_dir_all(&directory) {
            // Released, or the seed stays claimed by a session that was never
            // built and `Drop` will therefore never run for.
            if let Ok(mut live) = live_seeds().lock() {
                live.remove(&seed);
            }
            return Err(error);
        }

        Ok(Self {
            seed,
            directory,
            next: std::cell::Cell::new(1),
        })
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Every library this session has produced, newest first.
    ///
    /// What the explorer's session list is built from. Read from the directory
    /// rather than kept in a `Vec` beside the counter, so the list cannot claim
    /// a file that a failed compile never wrote.
    #[must_use]
    pub fn artefacts(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .collect();
        // The names are zero-padded and monotonic, so a lexical sort is a
        // chronological one, and reversing it puts the newest render at the top
        // where somebody looking for "the one I just made" expects it.
        paths.sort();
        paths.reverse();
        paths
    }

    /// The `(source, library)` pair for the next Render.
    fn next_pair(&self) -> (PathBuf, PathBuf) {
        let index = self.next.get();
        self.next.set(index + 1);
        (
            self.directory.join(format!("preview-{index:04}.rs")),
            self.directory.join(format!("libpreview-{index:04}.so")),
        )
    }
}

#[cfg(test)]
mod session_seed_tests {
    use super::Session;

    /// Two live sessions on one seed is refused, not quietly shared.
    #[test]
    fn a_second_live_session_on_one_seed_is_refused() {
        let first = Session::new(0x5EED_0001).expect("the first session");
        let second = Session::new(0x5EED_0001);
        assert!(
            second.is_err(),
            "the second session was handed the first one's directory, and \
             whichever dropped first would have deleted the other's sources"
        );
        drop(first);
    }

    /// And reusing a seed *after* the first session is gone still works — the
    /// guard is about overlap, not about the number ever being seen again.
    #[test]
    fn a_seed_is_reusable_once_its_session_has_been_dropped() {
        let first = Session::new(0x5EED_0002).expect("the first session");
        let directory = first.directory().to_path_buf();
        drop(first);
        assert!(!directory.exists(), "Drop removes the directory");
        let second = Session::new(0x5EED_0002).expect("the seed is free again");
        assert_eq!(second.directory(), directory);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Best effort: a leftover temp directory is untidy, and a panic in a
        // destructor over one would be worse.
        std::fs::remove_dir_all(&self.directory).ok();
        // Released last, so the seed is only reusable once the directory this
        // session owned is actually gone.
        if let Ok(mut live) = live_seeds().lock() {
            live.remove(&self.seed);
        }
    }
}

/// A compile running on a worker thread.
///
/// # Why `rustc` moved off the UI thread
///
/// It ran there until now, and the comment on [`Studio::render`] said what that
/// cost: the window was frozen for the length of a compile, and the `Compiling`
/// state the state machine so carefully maintained was **never once visible**,
/// because nothing painted between setting it and finishing. A spinner that
/// cannot be seen is a spinner that is not there.
///
/// [`Studio::render`]: crate::state::Studio::render
///
/// # The shape this takes, and why it is this one
///
/// One thread per Render, joined by dropping the receiver, and no thread pool.
/// The plan allows exactly one compile in flight (§4.5), so a pool would be a
/// pool of one — and `std::thread::spawn` costs microseconds against a compile
/// that costs a fifth of a second.
///
/// The result comes back down an `mpsc` channel that the UI polls once per
/// frame. Polling rather than a callback because a callback would run on the
/// worker and have to reach signals that are `Rc` and single-threaded by
/// design; the poll happens on the UI thread, where writing them is allowed.
///
/// **A finished compile has to cause a frame.** On an idle window nothing
/// draws, so the result would sit in the channel until the user moved the
/// mouse. [`Job::spawn`] takes a `wake` callback for exactly this — the studio
/// hands it the platform's waker.
#[derive(Debug)]
pub struct Job {
    results: Receiver<std::io::Result<Compiled>>,
    /// Set by [`cancel`](Job::cancel). The worker checks it while waiting on
    /// `rustc` and kills the child if it is set.
    cancelled: Arc<AtomicBool>,
    /// What the job is compiling, for the status bar.
    pub file_name: String,
    /// When it started, so the UI can say how long it has been going.
    pub started: Instant,
}

/// What a poll of an in-flight job found.
#[derive(Debug)]
pub enum Progress {
    /// Still compiling.
    Running,
    /// It finished; here is what came out.
    Done(std::io::Result<Compiled>),
    /// The worker went away without answering. Only reachable if it panicked,
    /// which would be a bug here rather than in the buffer — reported rather
    /// than swallowed, so it cannot look like a compile that never ends.
    Lost,
}

impl Job {
    /// Start compiling `source` on a worker thread.
    ///
    /// `wake` is called once, from the worker, when the result is ready.
    ///
    /// # Errors
    ///
    /// If the buffer cannot be written to the session directory. Everything
    /// after that point is reported through [`poll`](Job::poll) instead, because
    /// it happens on the other thread.
    pub fn spawn(
        toolchain: &Toolchain,
        session: &Session,
        source: &str,
        file_name: &str,
        project: Option<ProjectLib>,
        wake: impl Fn() + Send + 'static,
    ) -> std::io::Result<Self> {
        // The pair is minted here, on the UI thread, because `Session`'s
        // counter is a `Cell` — deliberately single-threaded, like everything
        // else the studio holds. Only the paths cross to the worker.
        let (source_path, library_path) = session.next_pair();
        let source = match project.as_ref() {
            Some(project) => with_project(source, &project.crate_name),
            None => source.to_owned(),
        };
        std::fs::write(&source_path, format!("{source}\n{ENTRY}"))?;

        let (sender, results) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));

        let toolchain = toolchain.clone();
        let file_name_owned = file_name.to_string();
        let flag = Arc::clone(&cancelled);
        std::thread::Builder::new()
            .name("viewwstudio-rustc".into())
            .spawn(move || {
                let outcome = run_rustc(
                    &toolchain,
                    &source_path,
                    &library_path,
                    &file_name_owned,
                    project.as_ref(),
                    &flag,
                );
                // A send that fails means the studio dropped the job — the user
                // pressed Cancel, or closed the window. Nothing to report to.
                sender.send(outcome).ok();
                wake();
            })?;

        Ok(Self {
            results,
            cancelled,
            file_name: file_name.to_string(),
            started: Instant::now(),
        })
    }

    /// Ask the worker to stop.
    ///
    /// Advisory: `rustc` is killed at the next check rather than instantly, so
    /// the UI must treat the job as gone the moment this is called rather than
    /// waiting for a confirmation that a killed process will never send.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation has been asked for.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// How long this job has been running.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Non-blocking. Called once a frame from the studio's frame hook.
    pub fn poll(&self) -> Progress {
        match self.results.try_recv() {
            Ok(result) => Progress::Done(result),
            Err(TryRecvError::Empty) => Progress::Running,
            Err(TryRecvError::Disconnected) => Progress::Lost,
        }
    }
}

/// Whether `source` gives the appended entry point something to call.
///
/// # Why this is checked before `rustc` rather than left to it
///
/// The preview appends [`ENTRY`], which calls a zero-argument `screen()`. A
/// buffer without one compiles to `cannot find function \`screen\` in this
/// scope` — pointing at a line the user cannot see, in code the studio wrote
/// and hid from them, about a convention nobody has told them about yet. That
/// is the single most confusing thing a person can be shown in the first five
/// minutes of this application, and it happens on the most natural first
/// action there is: open a real project's screen file and press Render.
///
/// Opening `snapsearch/src/screens/library.rs` — a real vieww application whose
/// screens take an `AppState` — is exactly that case, and it is the case this
/// was written against.
///
/// The check is textual rather than a parse. `screen` could be behind a `cfg`,
/// or generated by a macro, and this would miss it; the cost of a false
/// negative is a *hint* on a build that then succeeds anyway, and the cost of
/// the false positive it replaces is the error above.
#[must_use]
pub fn entry_point(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        let line = line.strip_prefix("pub ").unwrap_or(line);
        line.starts_with("fn screen(")
            || line.starts_with("fn screen (")
            || line.starts_with("fn screen<")
    })
}

/// The widget this file defines, if it defines exactly one.
///
/// Used to turn the missing-entry message from a rule into a line the user can
/// paste. A file with two widgets in it gets the general advice instead: the
/// studio does not know which one was meant, and guessing produces a snippet
/// that compiles and previews the wrong thing.
///
/// Textual, for the reason [`entry_point`] is.
#[must_use]
pub fn sole_widget(source: &str) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    for line in source.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("impl Widget for ") else {
            continue;
        };
        // `impl Widget for Card {`, and also `impl Widget for Card<'_> {`.
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    match names.len() {
        1 => names.pop(),
        _ => None,
    }
}

/// Whether `name` is declared as a unit struct — `struct Card;` — and so can be
/// written into a suggested `screen()` with nothing filled in.
#[must_use]
fn is_unit_struct(source: &str, name: &str) -> bool {
    let unit = format!("struct {name};");
    source
        .lines()
        .any(|line| line.trim_start().ends_with(&unit))
}

/// The diagnostic shown instead of a compiler error about hidden code.
///
/// Written as a `Diagnostic` so it lands in the Problems panel beside real
/// errors, with a file and a line, and can be clicked like any other.
#[must_use]
pub fn missing_entry(file_name: &str, lines: u32) -> Diagnostic {
    missing_entry_for(file_name, lines, "")
}

/// The same, with the buffer's own text so the suggestion can name its widget.
#[must_use]
pub fn missing_entry_for(file_name: &str, lines: u32, source: &str) -> Diagnostic {
    // The best suggestion this file supports, in descending order of how much
    // work it leaves the user: a unit struct can be pasted as-is, a struct with
    // fields needs them filled in, and a file with two widgets or none gets the
    // rule stated generally.
    let help = match sole_widget(source) {
        Some(name) if is_unit_struct(source, &name) => format!(
            "Render compiles this buffer on its own and calls `fn screen() -> impl Widget` \
             to get something to draw. This file defines `{name}`, so adding this to the \
             end of it is enough:\n\n    \
             pub fn screen() -> impl Widget {{\n        {name}\n    }}"
        ),
        Some(name) => format!(
            "Render compiles this buffer on its own and calls `fn screen() -> impl Widget` \
             to get something to draw. This file defines `{name}`; add a `screen()` that \
             builds one with whatever it needs:\n\n    \
             pub fn screen() -> impl Widget {{\n        \
             {name} {{ /* the fields this widget takes */ }}\n    }}\n\n\
             The preview cannot call a widget's constructor for you, because it has no way \
             to know what to pass it — which is why the entry point is a function you write."
        ),
        None => "Render compiles this buffer on its own and calls \
                 `fn screen() -> impl Widget` to get something to draw. Add one that builds \
                 the widget you want to look at, with any state it needs filled \
                 in:\n\n    \
                 pub fn screen() -> impl Widget {\n        \
                 MyScreen { state: AppState::sample() }\n    }"
            .to_owned(),
    };

    Diagnostic {
        file: file_name.to_owned(),
        severity: Severity::Error,
        code: "no-screen".to_owned(),
        message: "the preview needs a `screen()` function in this file".to_owned(),
        help: Some(help),
        // The end of the file, which is where the function would go and where
        // the appended entry point is looking for it.
        line: lines.max(1),
        column: 1,
        end_line: lines.max(1),
        end_column: 1,
    }
}

/// The function the tool appends, never shown in the editor.
///
/// `catch_unwind` is not optional: unwinding across `extern "C"` is undefined
/// behaviour, so a panicking `build()` has to become a null pointer *before* it
/// crosses the boundary. The pointer is a double box — `Box<Box<dyn Widget>>` —
/// because a trait object is a fat pointer and cannot be handed back as one
/// `*mut c_void`.
pub const ENTRY: &str = r#"
// ---- appended by vieww Studio; not part of the buffer -----------------------

// **The ABI check that actually checks the ABI.**
//
// The host resolves widgets through a factory keyed by `TypeId`. If the buffer
// was compiled against a different *compilation* of vieww than the host links —
// two rlibs of one crate, which is what a workspace produces the moment feature
// resolution differs — then every `TypeId` differs, the factory misses every
// widget, and the preview renders **nothing at all** while reporting success.
//
// Comparing `rustc --version` does not catch that: both sides had the same
// compiler. This does, by asking the guest what it thinks a well-known vieww
// type's identity is and comparing it with the host's answer.
#[no_mangle]
pub extern "C" fn vieww_preview_fingerprint() -> u64 {
    use ::std::hash::{Hash, Hasher};
    let mut hasher = ::std::collections::hash_map::DefaultHasher::new();
    ::std::any::TypeId::of::<::vieww::prelude::Text>().hash(&mut hasher);
    hasher.finish()
}

#[no_mangle]
pub extern "C" fn vieww_preview_entry() -> *mut ::std::ffi::c_void {
    let built = ::std::panic::catch_unwind(|| {
        ::std::boxed::Box::new(screen()) as ::std::boxed::Box<dyn ::vieww::Widget>
    });
    match built {
        Ok(widget) => ::std::boxed::Box::into_raw(::std::boxed::Box::new(widget)).cast(),
        Err(_) => ::std::ptr::null_mut(),
    }
}

// **The first `build()`, run on this side of the boundary.**
//
// The host used to do this itself, with a `catch_unwind` around a call into
// this image. It read as if it worked and it did not: a `cdylib` links its own
// copy of `std`, so a panic raised in here is a *foreign* exception to the
// host's unwinder, and the host does not catch foreign exceptions — it prints
// "fatal runtime error: Rust cannot catch foreign exceptions" and aborts. One
// `panic!` in a user's `build()` took the whole studio down, unsaved buffers
// with it, and `LoadError::PanicInBuild` was unreachable code.
//
// A panic can only be caught by the runtime that raised it, so the catch has to
// live where the panic does. Returns 0 for "built without panicking" and 1 for
// "panicked"; the host turns the 1 into the diagnostic it always meant to show.
//
// The pointer is borrowed, never taken: the host still owns that box.
#[no_mangle]
pub extern "C" fn vieww_preview_probe(handle: *mut ::std::ffi::c_void) -> u8 {
    if handle.is_null() {
        return 1;
    }
    let outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
        let widget: &::std::boxed::Box<dyn ::vieww::Widget> =
            unsafe { &*handle.cast::<::std::boxed::Box<dyn ::vieww::Widget>>() };
        if ::std::matches!(widget.kind(), ::vieww::WidgetKind::Composed) {
            let _ = widget.build(&::vieww::BuildContext::root());
        }
    }));
    u8::from(outcome.is_err())
}
"#;

/// Compile `source` into a loadable library, on this thread.
///
/// Never returns `Err` for a *compile* failure — that is a `Compiled` with no
/// library and a list of diagnostics, which is a normal outcome the UI shows.
/// `Err` is reserved for the studio's own machinery failing.
///
/// The studio itself goes through [`Job::spawn`] instead, so that the window
/// keeps painting. This stays because it is what the pipeline tests drive: a
/// test wants the answer, not a thread to wait on.
///
/// # Errors
///
/// If the source cannot be written, or `rustc` cannot be spawned or waited on.
pub fn compile(
    toolchain: &Toolchain,
    session: &Session,
    source: &str,
    file_name: &str,
) -> std::io::Result<Compiled> {
    let (source_path, library_path) = session.next_pair();
    std::fs::write(&source_path, format!("{source}\n{ENTRY}"))?;
    run_rustc(
        toolchain,
        &source_path,
        &library_path,
        file_name,
        None,
        &AtomicBool::new(false),
    )
}

/// Invoke `rustc` and turn what it says into a [`Compiled`].
///
/// Split out from [`compile`] so that [`Job`] can call the same code on a
/// worker thread. Everything it touches is owned or borrowed from the caller —
/// no `Session`, no signals — which is what makes it safe to run over there.
///
/// `cancelled` is checked on the same beat as the timeout: both end in killing
/// the child, and having one loop rather than two means a cancel cannot be
/// missed because the timeout branch was taken first.
fn run_rustc(
    toolchain: &Toolchain,
    source_path: &Path,
    library_path: &Path,
    file_name: &str,
    project: Option<&ProjectLib>,
    cancelled: &AtomicBool,
) -> std::io::Result<Compiled> {
    // **The command as it is actually run**, rather than a sentence describing
    // one. The two had already drifted — the line said `--edition 2021
    // --crate-type cdylib --error-format=json <file>` and the real invocation
    // carried four more flags, including the `--extern` that decides whether
    // the preview can see the project at all. A log of what the studio *meant*
    // to run is worth nothing on the day it runs something else.
    let mut args: Vec<String> = vec![
        "--edition".into(),
        "2021".into(),
        "--crate-type".into(),
        "cdylib".into(),
        "--error-format=json".into(),
        "-C".into(),
        "opt-level=0".into(),
        // **`-C prefer-dynamic`: share `libstd` with the studio.**
        //
        // A `cdylib` statically links its own copy of `std`, so a panic raised
        // inside the guest is a *foreign* exception to the host's unwinder —
        // the host's `catch_unwind` does not catch it and the process aborts.
        // With both the studio and the preview built `prefer-dynamic`, both
        // link against `libstd-*.so` from the same toolchain, and the host's
        // existing `catch_unwind` in `ElementTree::build` works for guest
        // widgets too. See `loaded.rs`'s panic section for the full history
        // and `Cargo.toml`'s `[profile.dev]` for the host side.
        "-C".into(),
        "prefer-dynamic=true".into(),
        // **`-C rpath`: the preview carries the toolchain's `lib/` with it.**
        //
        // `prefer-dynamic` is only half a plan: it records
        // `libstd-<hash>.so` as a dependency without saying where the file
        // is. `dlopen` then searches `LD_LIBRARY_PATH` and the system paths,
        // which hold the toolchain only when cargo launched this process —
        // a studio started from a desktop entry, a double click or a bare
        // shell fails here with `libstd-<hash>.so: cannot open shared object
        // file` against a compile that exited 0. `tests/pipeline.rs` never
        // saw it because `cargo test` sets the very variable the desktop
        // launch does not.
        //
        // `-C rpath` makes rustc record the sysroot's `lib/` in the `.so`'s
        // own runpath, so the loader finds libstd without anybody's
        // environment being right. Unix only: the flag is a no-op elsewhere
        // and Windows resolves dependencies by sitting them next to the exe,
        // which is the packager's job.
        //
        // This and the host-side `.cargo/config.toml` are complementary
        // rather than redundant: the config makes the studio share libstd so
        // the panic boundary holds, and this makes the preview loadable even
        // when the studio was built without it — a studio binary someone
        // built with their own cargo config, say.
        "--extern".into(),
        format!("vieww={}", toolchain.vieww_rlib.display()),
        "-L".into(),
        format!("dependency={}", toolchain.deps.display()),
    ];
    // `-C rpath`, unix only (see the comment above): the flag is a no-op
    // elsewhere and Windows resolves dependencies by sitting them next to the
    // exe, which is the packager's job. A runtime `cfg!` rather than an
    // attribute on the element, because this is one list built in one place
    // and a conditional push reads as what it is.
    if cfg!(unix) {
        args.push("-C".into());
        args.push("rpath".into());
    }
    if let Some(project) = project {
        // The project's own rlib and its dependency directory, so a screen
        // file's `use crate::…` — rewritten to `use <crate>::…` — resolves.
        args.push("--extern".into());
        args.push(format!("{}={}", project.crate_name, project.rlib.display()));
        args.push("-L".into());
        args.push(format!("dependency={}", project.deps.display()));
    }
    args.push(source_path.display().to_string());
    args.push("-o".into());
    args.push(library_path.display().to_string());

    let mut log = vec![
        format!("write buffer -> {}", source_path.display()),
        format!("{} {}", toolchain.rustc.display(), args.join(" ")),
    ];

    let started = Instant::now();
    let mut child = Command::new(&toolchain.rustc)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Wait with a deadline rather than `wait()`, so a pathological compile is
    // killed instead of hanging the Render button for ever — and so that a
    // cancel from the UI thread is acted on rather than noted.
    let stopped = loop {
        match child.try_wait()? {
            Some(_) => break None,
            None if cancelled.load(Ordering::Relaxed) => {
                child.kill().ok();
                child.wait().ok();
                break Some(Stopped::Cancelled);
            }
            None if started.elapsed() >= TIMEOUT => {
                child.kill().ok();
                child.wait().ok();
                break Some(Stopped::TimedOut);
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };

    let output = child.wait_with_output()?;
    let duration = started.elapsed();

    if let Some(stopped) = stopped {
        log.push(stopped.log_line());
        return Ok(Compiled {
            library: None,
            json: Vec::new(),
            diagnostics: stopped
                .diagnostic(file_name)
                .map_or_else(Vec::new, |d| vec![d]),
            log,
            duration,
        });
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostics = parse_diagnostics(&stderr, file_name);
    let json: Vec<String> = stderr
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
        .map(str::to_string)
        .collect();
    let succeeded = output.status.success() && library_path.exists();

    log.push(if succeeded {
        format!("exit 0 — {:.2}s", duration.as_secs_f32())
    } else {
        let errors = diagnostics
            .iter()
            .filter(|d| matches!(d.severity, Severity::Error))
            .count();
        format!("exit 1 — {errors} error(s), {:.2}s", duration.as_secs_f32())
    });

    Ok(Compiled {
        library: succeeded.then(|| library_path.to_path_buf()),
        json,
        diagnostics,
        log,
        duration,
    })
}

/// Why a compile ended without `rustc` finishing.
///
/// The two are told apart because only one of them is a problem with the
/// buffer. A timeout is something the code did and belongs in the Problems
/// panel; a cancel is something the *user* did and must not leave a red mark
/// on a file that may be perfectly fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stopped {
    Cancelled,
    TimedOut,
}

impl Stopped {
    fn log_line(self) -> String {
        match self {
            Self::Cancelled => "cancelled".to_string(),
            Self::TimedOut => format!("killed after {}s", TIMEOUT.as_secs()),
        }
    }

    fn diagnostic(self, file_name: &str) -> Option<Diagnostic> {
        match self {
            Self::Cancelled => None,
            Self::TimedOut => Some(Diagnostic {
                file: file_name.to_string(),
                severity: Severity::Error,
                code: "timeout".into(),
                message: format!(
                    "rustc was still running after {}s and was killed",
                    TIMEOUT.as_secs()
                ),
                help: Some("a const-eval loop or a very large macro expansion can do this".into()),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            }),
        }
    }
}

/// Turn `rustc --error-format=json` into the studio's own diagnostics.
///
/// # Parsed, not scanned
///
/// This used to find `"level":"` in the line and read forward to the next
/// quote. It said so, and named the case that defeats it: a `"` inside a
/// message, which `rustc` escapes and the scan unescaped "only for the common
/// cases". The messages that contain quotes are not exotic — every
/// `expected `&str`, found `String`` and every `unknown field "name"` has
/// them — and the failure mode was a Problems panel showing half a sentence,
/// or a span read out of the middle of a string.
///
/// It also could not tell *which* object a key belonged to. `"line_start"`
/// was found by searching the whole record, so a diagnostic whose primary span
/// was second in the list was reported at the first span's line — pointing the
/// user at code that was not the problem.
///
/// [`crate::json`] already existed for exactly this, and `cargo.rs` already
/// used it on the same stream. So this walks a parsed value: the primary span
/// is the one flagged `is_primary`, the code is `code.code`, and a quote in a
/// message is a quote in a message.
#[must_use]
pub fn parse_diagnostics(stderr: &str, file_name: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    for line in stderr.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(record) = crate::json::Json::parse(line) else {
            continue;
        };

        let level = record.str_field("level").unwrap_or_default();
        let severity = match level {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            // "note", "help" and "failure-note" arrive as their own records and
            // duplicate what the parent already said.
            _ => continue,
        };

        let Some(message) = record.str_field("message") else {
            continue;
        };
        if message.starts_with("aborting due to") || message.starts_with("For more information") {
            continue;
        }

        let code = record
            .path(["code", "code"])
            .and_then(crate::json::Json::as_str)
            .unwrap_or(level)
            .to_owned();
        let (line_no, column, end_line, end_column) = primary_span(&record);

        out.push(Diagnostic {
            file: file_name.to_string(),
            severity,
            code,
            message: message.to_owned(),
            help: suggestion(&record),
            line: line_no,
            column,
            end_line,
            end_column,
        });
    }

    out
}

/// The `suggested_replacement` of the first span that offers one.
fn suggestion(record: &crate::json::Json) -> Option<String> {
    record
        .get("spans")?
        .as_array()?
        .iter()
        .find_map(|span| span.str_field("suggested_replacement"))
        .map(|text| format!("try `{text}`"))
}

/// The primary span of a record, start and end.
///
/// The **primary** one, not the first one: `rustc` puts secondary spans in the
/// same list (the "expected because of this" locations), and jumping to one of
/// those puts the caret on code that is correct. `is_primary` is the flag
/// `rustc` sets for the span the error is actually at; when no span carries it,
/// the first span stands, which is what the old scan did for every record.
///
/// The end defaults to the start rather than to the end of the line: a span
/// that could not be read is a *point*, and a mark from a point to the line's
/// end is a confident claim about text the compiler said nothing about.
fn primary_span(record: &crate::json::Json) -> (u32, u32, u32, u32) {
    let spans = record.get("spans").and_then(crate::json::Json::as_array);
    let span = spans.and_then(|spans| {
        spans
            .iter()
            .find(|span| matches!(span.get("is_primary"), Some(crate::json::Json::Bool(true))))
            .or_else(|| spans.first())
    });
    let Some(span) = span else {
        return (1, 1, 1, 1);
    };
    let number = |key: &str| span.get(key).and_then(crate::json::Json::as_u32);
    let line = number("line_start").unwrap_or(1);
    let column = number("column_start").unwrap_or(1);
    let end_line = number("line_end").unwrap_or(line);
    let end_column = number("column_end").unwrap_or(column);
    (line, column, end_line, end_column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sysroot_is_discovered_and_holds_libstd() {
        // `Toolchain::discover` needs the workspace's rlibs; from the unit-test
        // vantage point they are where `cargo test` put them, two levels up
        // from this binary — the same dance `tests/pipeline.rs` does, minus
        // the skip, because *these* assertions are about the compiler rather
        // than the graph and `rustc` alone is enough to satisfy them.
        let exe = std::env::current_exe().expect("a test binary");
        let target = exe
            .parent()
            .and_then(Path::parent)
            .expect("target/debug")
            .to_path_buf();
        let Ok(toolchain) = Toolchain::discover(&target) else {
            // No built rlibs on this machine; the sysroot questions below can
            // still be asked of the `rustc` on PATH, which is the same one
            // discover would have used.
            let sysroot = sysroot_lib(Path::new("rustc"));
            assert!(
                sysroot.is_some(),
                "rustc --print sysroot refused to answer, so nothing can \
                 prime libstd or bake a runpath"
            );
            return;
        };

        let lib = toolchain
            .sysroot_lib
            .as_deref()
            .expect("a compiler that ran for discover can name its sysroot");
        assert!(
            lib.is_dir(),
            "discover recorded a sysroot lib directory that is not there: {}",
            lib.display()
        );
        let holds_libstd = std::fs::read_dir(lib)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("libstd-"));
        assert!(
            holds_libstd,
            "the sysroot lib directory {} holds no libstd, so a preview \
             compiled prefer-dynamic could never load",
            lib.display()
        );
    }

    #[test]
    fn a_real_rustc_record_becomes_a_diagnostic() {
        // Trimmed from an actual run against this workspace.
        let stderr = r#"{"$message_type":"diagnostic","message":"mismatched types","code":{"code":"E0308"},"level":"error","spans":[{"file_name":"preview-0001.rs","line_start":24,"line_end":24,"column_start":38,"column_end":42,"is_primary":true}],"children":[]}"#;

        let parsed = parse_diagnostics(stderr, "landing_screen.rs");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].severity, Severity::Error);
        assert_eq!(parsed[0].code, "E0308");
        assert_eq!(parsed[0].message, "mismatched types");
        assert_eq!((parsed[0].line, parsed[0].column), (24, 38));
        assert_eq!(
            parsed[0].file, "landing_screen.rs",
            "reported against the buffer, not the temp file rustc saw"
        );
    }

    #[test]
    fn the_summary_records_are_dropped() {
        let stderr = concat!(
            r#"{"message":"aborting due to 1 previous error","level":"error","spans":[]}"#,
            "\n",
            r#"{"message":"For more information about this error, try `rustc --explain E0308`.","level":"failure-note","spans":[]}"#,
        );
        assert!(
            parse_diagnostics(stderr, "x.rs").is_empty(),
            "neither is a place in the buffer anybody can jump to"
        );
    }

    #[test]
    fn warnings_survive_and_errors_are_told_apart() {
        let stderr = r#"{"message":"unused import: `vieww::controls::Chip`","code":{"code":"unused_imports"},"level":"warning","spans":[{"line_start":2,"column_start":44,"is_primary":true}]}"#;
        let parsed = parse_diagnostics(stderr, "x.rs");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].severity, Severity::Warning);
        assert_eq!(parsed[0].line, 2);
    }

    #[test]
    fn the_appended_entry_catches_a_panic_before_it_crosses_the_boundary() {
        assert!(
            ENTRY.contains("catch_unwind"),
            "unwinding across extern \"C\" is UB; this is the guard"
        );
        assert!(ENTRY.contains("null_mut"), "and null is how it reports it");
    }

    #[test]
    fn a_session_cleans_up_after_itself() {
        let session = Session::new(0xDEAD_BEEF).expect("a temp directory");
        let directory = session.directory().to_path_buf();
        assert!(directory.exists());

        let (source, library) = session.next_pair();
        assert!(source.to_string_lossy().ends_with("preview-0001.rs"));
        assert!(library.to_string_lossy().ends_with("libpreview-0001.so"));

        let (second, _) = session.next_pair();
        assert!(
            second.to_string_lossy().ends_with("preview-0002.rs"),
            "never reused: the first one may already be dlopen'd"
        );

        drop(session);
        assert!(!directory.exists(), "the session took its files with it");
    }

    #[test]
    fn a_quote_inside_a_message_no_longer_truncates_it() {
        // The case the old scan named and could not handle: rustc escapes the
        // quotes it puts around a type name, and reading to "the next quote"
        // stopped in the middle of the sentence.
        let stderr = r#"{"message":"unknown field \"nmae\", expected \"name\"","code":{"code":"E0560"},"level":"error","spans":[{"line_start":7,"line_end":7,"column_start":5,"column_end":9,"is_primary":true}],"children":[]}"#;
        let parsed = parse_diagnostics(stderr, "screen.rs");
        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].message,
            "unknown field \"nmae\", expected \"name\""
        );
        assert_eq!((parsed[0].line, parsed[0].column), (7, 5));
    }

    #[test]
    fn the_primary_span_is_used_rather_than_the_first_one() {
        // A secondary span is "expected because of this" — correct code the
        // user should not be sent to.
        let stderr = r#"{"message":"mismatched types","code":{"code":"E0308"},"level":"error","spans":[{"line_start":3,"line_end":3,"column_start":1,"column_end":4,"is_primary":false},{"line_start":41,"line_end":41,"column_start":9,"column_end":13,"is_primary":true}],"children":[]}"#;
        let parsed = parse_diagnostics(stderr, "screen.rs");
        assert_eq!((parsed[0].line, parsed[0].column), (41, 9));
        assert_eq!((parsed[0].end_line, parsed[0].end_column), (41, 13));
    }

    #[test]
    fn a_line_that_is_not_json_is_skipped_rather_than_guessed_at() {
        let stderr = "   Compiling preview v0.0.1\n{ not json at all\n";
        assert!(parse_diagnostics(stderr, "screen.rs").is_empty());
    }

    #[test]
    fn a_suggestion_becomes_the_help_line() {
        let stderr = r#"{"message":"help: a similar name exists","level":"warning","spans":[{"line_start":2,"line_end":2,"column_start":1,"column_end":2,"is_primary":true,"suggested_replacement":"width"}],"children":[]}"#;
        let parsed = parse_diagnostics(stderr, "screen.rs");
        assert_eq!(parsed[0].help.as_deref(), Some("try `width`"));
    }
}
