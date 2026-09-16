//! `vieww new <name>`: a minimal, real, compilable desktop app skeleton.
//!
//! # Why this is a second scaffolder rather than a shared one
//!
//! `apps/viewwstudio/src/scaffold.rs` already does this — for real, tested by
//! actually compiling what it writes — and is the more capable of the two: it
//! generates a Say-or-Rust choice, an Android Gradle project, an iOS Xcode
//! project, a Live Preview buffer, and a `Cargo.toml` shaped so the studio's
//! `dlopen`-based preview can load the result. None of that is duplicated
//! here.
//!
//! What *is* duplicated, in miniature, is the part both scaffolders must get
//! right regardless: a name check that keeps a bad crate or module name from
//! ever reaching disk, a `Cargo.toml` with the empty `[workspace]` table that
//! keeps a project created inside another workspace from failing its first
//! build (see that module's own comment on the bug this fixes), and the
//! atomic "write everything or clean up" semantics of [`create`].
//!
//! This is not shared with `viewwstudio::scaffold` because doing so honestly
//! would mean one of two things, and this delivery's scope allows neither:
//! either lifting the shared parts out of `apps/viewwstudio` into this crate
//! and pointing the studio's own module at them — a real refactor of a file
//! this task was explicitly told not to modify — or reaching *into*
//! `apps/viewwstudio` from a library crate, which inverts the workspace's
//! actual dependency direction (an app may depend on a library; a library
//! reaching into an app does not make sense as a build graph and `apps/*` is
//! not even a path other crates can depend on by convention here). So the
//! honest thing is two scaffolders that agree on principle and differ on
//! product: the studio's writes a project shaped for its own GUI and preview
//! pipeline, and this one writes the plainest thing `vieww new` can hand
//! somebody at a terminal — a desktop binary with one window and one widget,
//! nothing that needs a studio to open.
//!
//! **Known duplication, left for future work to unify**: the crate-name
//! keyword list below and the "write everything or roll it all back" logic in
//! [`create`] are close enough to `apps/viewwstudio`'s that a shared
//! `vieww-build::scaffold::naming` module — depended on by both — is the
//! obvious next step, the same way `vieww-gestures` re-exports `Spring` and
//! `Fling` from `vieww-animation` rather than keeping a second copy once the
//! sharing became easy to do safely.
//!
//! # What the generated project actually is
//!
//! ```text
//! my-app/
//!   Cargo.toml       — an empty [workspace] table; see check_name's sibling doc
//!   .gitignore
//!   src/main.rs      — one window, one widget, real vieww/vieww-platform-winit calls
//! ```
//!
//! No `[lib]`, no Android or iOS harness, no asset bundle: this is
//! `cargo new`'s scope, not `viewwstudio`'s New Project. `src/main.rs` opens a
//! window titled after the project and shows one piece of text — small enough
//! to read in full the moment somebody opens it, and real enough that
//! `tests/scaffold_build.rs`(../../tests/scaffold_build.rs) compiles it with
//! an actual `cargo build`, not merely renders it as a string that looks like
//! Rust.

use std::fmt;
use std::path::{Path, PathBuf};

/// Where a generated project's `vieww` and `vieww-platform-winit` come from.
///
/// The same two-shape question `viewwstudio::scaffold::Dependency` answers,
/// for the same reason: a preview needs the *exact* compilation the tool
/// itself was built from and can only get that from a path into this
/// checkout, while a project meant to leave this checkout needs a version
/// crates.io can resolve. `vieww new` cannot preview anything — there is no
/// `dlopen` here — so the distinction matters less than it does for the
/// studio, but writing a `vieww = "0.0.1"` into a project scaffolded from
/// inside this very checkout, when `crates/vieww` sits three directories up
/// and is not published anywhere, would hand somebody a project that never
/// builds. [`Dependency::detect`] is what picks correctly by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dependency {
    /// `vieww = { path = "…" }` — resolves inside a checkout of this
    /// workspace, including this one.
    Path(PathBuf),
    /// `vieww = "…"` — resolves against a published crate.
    Version(String),
}

impl Dependency {
    /// Look for this checkout and use it if it is there, falling back to the
    /// workspace's own version string — the version `vieww` itself carries,
    /// since every crate in this workspace shares one (`[workspace.package]`
    /// `version`).
    ///
    /// This is a judgement call answered the same way `checkout_root` answers
    /// it: verified against the filesystem, not merely assumed because this
    /// code happens to live inside the checkout it is looking for. A copy of
    /// `vieww-build`'s source vendored somewhere else — unlikely today, real
    /// the day this crate is published — must not claim a path that is not
    /// there.
    #[must_use]
    pub fn detect() -> Self {
        checkout_root().map_or_else(
            || Self::Version(env!("CARGO_PKG_VERSION").to_owned()),
            Self::Path,
        )
    }

    fn spec(&self, crate_name: &str) -> String {
        match self {
            Self::Path(root) => {
                let path = root.join("crates").join(crate_name);
                // Cargo.toml is TOML; a Windows backslash in the string would
                // be read as an escape sequence rather than a path separator.
                let path = path.to_string_lossy().replace('\\', "/");
                format!("{{ path = \"{path}\" }}")
            }
            Self::Version(version) => format!("\"{version}\""),
        }
    }
}

/// Why a project could not be scaffolded.
#[derive(Debug)]
pub enum ScaffoldError {
    /// The name is not usable as a Cargo package name.
    Name(&'static str),
    /// The target directory already has something in it. Never overwritten —
    /// see [`create`]'s docs for why refusing outright is the only safe
    /// answer here.
    NotEmpty(PathBuf),
    Io(std::io::Error),
}

impl fmt::Display for ScaffoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(why) => write!(f, "that name will not work: {why}"),
            Self::NotEmpty(path) => write!(
                f,
                "{} already has files in it — pick an empty directory, or a new name",
                path.display()
            ),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ScaffoldError {}

impl From<std::io::Error> for ScaffoldError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Check `name` against Cargo's rules and Rust's, before anything is written.
///
/// # Errors
///
/// If the name is empty, starts with a digit, contains anything but
/// `a-z A-Z 0-9 _ -`, or is a Rust keyword — the same keyword list
/// `apps/viewwstudio`'s scaffolder carries, and for the same reason: a project
/// named `loop` or `match` scaffolds a complete tree whose `[[bin]] name`
/// becomes a module path (`-` maps to `_`), and `mod loop;` does not compile.
/// Finding that out from a name check beats finding it out from `rustc`
/// twenty seconds into the first build.
pub fn check_name(name: &str) -> Result<(), ScaffoldError> {
    if name.is_empty() {
        return Err(ScaffoldError::Name("it is empty"));
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(ScaffoldError::Name(
            "a crate name cannot start with a digit",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ScaffoldError::Name("use letters, digits, `-` and `_` only"));
    }
    const KEYWORDS: [&str; 52] = [
        "Self", "abstract", "as", "async", "await", "become", "box", "break", "const", "continue",
        "crate", "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if",
        "impl", "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv",
        "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "try",
        "type", "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
    ];
    if KEYWORDS.contains(&name.replace('-', "_").as_str()) {
        return Err(ScaffoldError::Name("that is a Rust keyword"));
    }
    Ok(())
}

/// Every file a new project starts with, as `(relative path, contents)`.
///
/// Pure — nothing here touches a disk — so [`create`] and the compile test in
/// `tests/scaffold_build.rs` are exercising the same definition of "what a new
/// project is" rather than two that could drift apart.
#[must_use]
pub fn files(name: &str, dependency: &Dependency) -> Vec<(PathBuf, String)> {
    vec![
        (PathBuf::from("Cargo.toml"), cargo_toml(name, dependency)),
        (PathBuf::from(".gitignore"), GITIGNORE.to_owned()),
        (PathBuf::from("src/main.rs"), main_rs(name)),
    ]
}

fn cargo_toml(name: &str, dependency: &Dependency) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
publish = false

# An empty `[workspace]` table, and load-bearing.
#
# `vieww new` has no control over where it lands on disk, and the common case
# for anyone scaffolding a project *from inside a checkout of vieww* — which
# is exactly what running `vieww-build`'s own tests does — is landing inside
# another Cargo workspace's directory tree without being one of its members.
# Without this table, Cargo walks up from this manifest, finds that outer
# workspace, and refuses:
#
#     error: current package believes it's in a workspace when it's not
#
# The empty table is the standard idiom for "this manifest is its own
# workspace root, stop walking up" — the same line
# `apps/viewwstudio/src/scaffold.rs` writes and documents at greater length.
[workspace]

[dependencies]
vieww = {vieww}
vieww-platform-winit = {platform}

[[bin]]
name = "{name}"
path = "src/main.rs"
"#,
        vieww = dependency.spec("vieww"),
        platform = dependency.spec("vieww-platform-winit"),
    )
}

const GITIGNORE: &str = "/target\n";

/// The whole application: one window, one widget, real calls throughout.
///
/// `#[derive(Debug)]` on `Home` is not decoration — `Widget: Any + Debug`, and
/// its absence is precisely the defect `apps/viewwstudio`'s own template
/// comment warns future editors not to delete. It is the first thing to check
/// if a generated project fails to build.
///
/// `#[widget]` writes the `impl Widget` block's mechanical half — `debug_name`,
/// `kind`, and `From<Home> for WidgetNode`. It is used here rather than the
/// long form deliberately: the scaffold is the first vieww code most people
/// read, and what it shows is taken as the house style. Showing four methods
/// where one is the real one teaches that a widget is expensive to declare.
/// The long form still works and is what the attribute expands to; see
/// `vieww-widget-macros`.
///
/// # Why this emits `App::theme` and not `App::background`
///
/// It used to emit `background(ThemeData::dark().colors.surface)` and mount no
/// `Theme` at all. Those are two settings with nothing tying them together:
/// the first painted the window dark, and `ThemeData::of` — finding no `Theme`
/// above it — handed the widgets `ThemeData::light()`. A scaffolded
/// application therefore ran with a **dark window** and **light-themed
/// widgets**, drawing near-black body text on a near-black ground.
///
/// `App::theme` was added to close exactly that gap, and
/// `apps/viewwstudio/src/scaffold.rs`'s two templates adopted it — but this
/// one, which is what `vieww new` writes and what `docs/guide/getting-started.md`
/// documents as the way in, was left behind. So the first vieww code most
/// people ever run still shipped the defect after it was fixed everywhere
/// else.
///
/// `the_scaffold_mounts_a_theme_it_also_paints_the_window_from` is the test
/// that stops it happening a third time; it asserts on this string rather than
/// on a built application, because the failure is a *missing* call and only
/// the source says whether the call is there.
fn main_rs(name: &str) -> String {
    format!(
        r#"//! {name} — a vieww application.
//!
//! Generated by `vieww new`. Everything below is real: `App` opens an actual
//! window, and `Home` is an actual [`Widget`](vieww::prelude::Widget) — there
//! is nothing here to fill in before this runs.

use vieww::prelude::*;
use vieww_platform_winit::App;

/// The theme this application draws in.
///
/// One value. `App::theme` sets the window's background from it *and*
/// publishes it above the widget tree, so the two cannot disagree — change
/// this line and the window and everything in it move together.
///
/// `light()` because that is what vieww Studio previews with by default, so
/// the screen you preview and the screen you build are the same screen.
/// `dark()` is the other one; `ThemeData::adaptive(platform, dark)` follows
/// the platform.
fn theme() -> ThemeData {{
    ThemeData::light()
}}

fn main() -> Result<(), Box<dyn std::error::Error>> {{
    let report = App::new()
        .title("{name}")
        .size(Size::new(420.0, 720.0))
        .theme(theme())
        .run(|driver| {{
            driver.set_root(WidgetNode::new(Home));
        }})?;
    println!("{{report}}");
    Ok(())
}}

#[derive(Debug)]
struct Home;

#[widget]
impl Home {{
    fn build(&self, ctx: &BuildContext) -> impl Into<WidgetNode> {{
        let theme = ThemeData::of(ctx);
        Container::new()
            .color(theme.colors.surface)
            .alignment(Alignment::CENTER)
            .child(Text::new("Hello, {name}!").style(theme.text.headline))
    }}
}}
"#,
        name = name,
    )
}

/// Write a new project into `target_dir`.
///
/// The dependency is chosen automatically by [`Dependency::detect`] — see its
/// docs for when that is a path into this checkout and when it is a version.
/// Use [`create`] directly to force one or the other, which the tests below
/// and `tests/scaffold_build.rs` both do to make sure the generated project
/// actually compiles.
///
/// # Errors
///
/// [`ScaffoldError::Name`] for a name Cargo would reject, and
/// [`ScaffoldError::NotEmpty`] if `target_dir` already holds anything — see
/// [`create`] for the rest.
pub fn scaffold(name: &str, target_dir: &Path) -> Result<Vec<PathBuf>, ScaffoldError> {
    create(target_dir, name, &Dependency::detect())
}

/// [`scaffold`], with the dependency spelled out rather than detected.
///
/// # Errors
///
/// [`ScaffoldError::Name`] for a name Cargo would reject, [`ScaffoldError::NotEmpty`]
/// if `target_dir` already holds anything, and [`ScaffoldError::Io`] for
/// anything the filesystem refused.
///
/// # What happens when it fails halfway
///
/// Every directory and file this call created is removed, so a failure while
/// writing file two of three does not leave a directory that is not empty but
/// is not a project either — which is worse than either extreme, because a
/// second attempt at the same path is then refused by the emptiness check
/// above and somebody has to go work out what to delete by hand.
pub fn create(
    target_dir: &Path,
    name: &str,
    dependency: &Dependency,
) -> Result<Vec<PathBuf>, ScaffoldError> {
    check_name(name)?;

    let existed = target_dir.exists();
    if existed {
        let mut entries = std::fs::read_dir(target_dir)?;
        if entries.next().is_some() {
            return Err(ScaffoldError::NotEmpty(target_dir.to_path_buf()));
        }
    }
    std::fs::create_dir_all(target_dir)?;

    let mut written = Vec::new();
    for (relative, contents) in files(name, dependency) {
        let path = target_dir.join(&relative);
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, contents)
        })();

        if let Err(error) = result {
            if existed {
                for path in &written {
                    std::fs::remove_file(path).ok();
                }
            } else {
                std::fs::remove_dir_all(target_dir).ok();
            }
            return Err(ScaffoldError::Io(error));
        }
        written.push(path);
    }

    Ok(written)
}

/// The vieww checkout this crate was built from, if it is still there.
///
/// Mirrors `viewwstudio::scaffold::checkout_root` exactly in spirit:
/// `CARGO_MANIFEST_DIR` is `<checkout>/crates/vieww-build`, walking up twice
/// reaches the workspace root, and the two crates a template names are
/// checked for on disk rather than assumed — a copy of this source moved
/// elsewhere, or vendored into another project, must not claim a path that
/// is not there.
#[must_use]
pub fn checkout_root() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent()?.parent()?;
    let ok = root.join("crates/vieww/Cargo.toml").is_file()
        && root
            .join("crates/vieww-platform-winit/Cargo.toml")
            .is_file();
    ok.then(|| root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("vieww-build-scaffold-{name}"));
            std::fs::remove_dir_all(&dir).ok();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn dependency() -> Dependency {
        Dependency::Path(PathBuf::from("/w/vieww"))
    }

    #[test]
    fn a_name_is_checked_before_anything_is_written() {
        for bad in [
            "", "1app", "my app", "my.app", "app!", "loop", "match", "Self",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?} should be refused");
        }
        for good in ["app", "my-app", "my_app", "App2", "a"] {
            assert!(check_name(good).is_ok(), "{good:?} should be accepted");
        }
    }

    #[test]
    fn the_template_is_the_whole_project() {
        let generated = files("my-app", &dependency());
        let paths: Vec<String> = generated
            .iter()
            .map(|(path, _)| path.to_string_lossy().replace('\\', "/"))
            .collect();
        for expected in ["Cargo.toml", ".gitignore", "src/main.rs"] {
            assert!(paths.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn the_scaffold_opts_out_of_any_parent_workspace() {
        let manifest = &files("my-app", &dependency())[0].1;
        let mut in_workspace = false;
        let mut found_empty_workspace = false;
        for line in manifest.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                in_workspace = trimmed == "[workspace]";
                continue;
            }
            if in_workspace && !trimmed.is_empty() && !trimmed.starts_with('#') {
                found_empty_workspace = false;
                break;
            }
            if in_workspace {
                found_empty_workspace = true;
            }
        }
        assert!(
            found_empty_workspace,
            "the scaffolded Cargo.toml must contain an empty [workspace] table: {manifest}"
        );
    }

    #[test]
    fn the_binary_carries_the_project_name_throughout() {
        let generated = files("my-app", &dependency());
        let manifest = &generated[0].1;
        assert!(manifest.contains(r#"name = "my-app""#));
        assert!(manifest.contains("[[bin]]"));

        let main = &generated
            .iter()
            .find(|(p, _)| p.ends_with("main.rs"))
            .unwrap()
            .1;
        assert!(main.contains(r#".title("my-app")"#));
        assert!(main.contains("Hello, my-app!"));
        assert!(main.contains("#[derive(Debug)]"), "Widget: Any + Debug");
        // The attribute, not the long form: `#[widget]` is what writes the
        // `impl Widget` block now, and a scaffold that quietly reverted to
        // four hand-written methods is a regression this catches.
        assert!(main.contains("#[widget]"));
        assert!(main.contains("impl Home"));
        assert!(main.contains("fn main()"));
    }

    /// The window and the widgets have to come from one value.
    ///
    /// This asserts on the generated *source* rather than on a running
    /// application because the defect it guards is a call that is not there:
    /// a scaffold that paints the window with `App::background` and mounts no
    /// `Theme` compiles, runs, opens a window, and draws near-black text on a
    /// near-black ground — `ThemeData::of` falls back to `ThemeData::light()`
    /// while the window is `dark()`. Nothing about that is observable to a
    /// test that only checks the project builds, which is why the previous
    /// scaffold test suite passed the whole time the defect shipped.
    ///
    /// It also asserts `background` is *absent*, not merely that `theme` is
    /// present: reintroducing `background` beside `theme` is how the two
    /// settings drift apart again, and `theme` sets the background itself.
    #[test]
    fn the_scaffold_mounts_a_theme_it_also_paints_the_window_from() {
        let generated = files("my-app", &dependency());
        let main = &generated
            .iter()
            .find(|(p, _)| p.ends_with("main.rs"))
            .unwrap()
            .1;

        assert!(
            main.contains(".theme(theme())"),
            "the scaffold must configure the window from the same ThemeData the \
             widget tree gets — see this module's `main_rs` doc for what \
             happens when it does not:\n{main}"
        );
        assert!(
            main.contains("fn theme() -> ThemeData"),
            "the theme belongs in one named function the author can change in \
             one place:\n{main}"
        );
        assert!(
            !main.contains(".background("),
            "`App::background` sets the window without publishing a theme, so a \
             scaffold that calls it has two settings that can disagree; \
             `App::theme` already sets the background:\n{main}"
        );
    }

    #[test]
    fn a_path_dependency_points_at_the_crates_directory() {
        let manifest = &files("app", &Dependency::Path(PathBuf::from("/w/vieww")))[0].1;
        assert!(
            manifest.contains(r#"vieww = { path = "/w/vieww/crates/vieww" }"#),
            "{manifest}"
        );
        assert!(manifest.contains(
            r#"vieww-platform-winit = { path = "/w/vieww/crates/vieww-platform-winit" }"#
        ));
    }

    #[test]
    fn a_version_dependency_is_written_as_a_version() {
        let dep = Dependency::Version("0.0.1".to_owned());
        let manifest = &files("app", &dep)[0].1;
        assert!(manifest.contains(r#"vieww = "0.0.1""#), "{manifest}");
    }

    #[test]
    fn detect_finds_this_very_checkout() {
        // This crate's own tests run from inside the checkout they describe,
        // so `detect` must find it — the fallback branch is what a published
        // copy of this crate would hit, which this test cannot be.
        let dependency = Dependency::detect();
        assert!(
            matches!(dependency, Dependency::Path(_)),
            "expected a path dependency when run from inside the checkout: {dependency:?}"
        );
    }

    #[test]
    fn creating_writes_every_file() {
        let s = Scratch::new("create");
        let written = create(&s.0, "my-app", &dependency()).unwrap();
        assert_eq!(written.len(), files("my-app", &dependency()).len());
        for path in &written {
            assert!(path.is_file(), "{} was not written", path.display());
        }
    }

    #[test]
    fn an_existing_directory_with_anything_in_it_is_refused() {
        let s = Scratch::new("not-empty");
        std::fs::create_dir_all(&s.0).unwrap();
        std::fs::write(s.0.join("important.txt"), b"do not lose me").unwrap();

        let error = create(&s.0, "my-app", &dependency()).unwrap_err();
        assert!(matches!(error, ScaffoldError::NotEmpty(_)), "{error:?}");
        assert!(
            s.0.join("important.txt").is_file(),
            "existing file untouched"
        );
        assert!(!s.0.join("Cargo.toml").exists(), "nothing written");
    }

    #[test]
    fn an_empty_existing_directory_is_fine() {
        let s = Scratch::new("empty");
        std::fs::create_dir_all(&s.0).unwrap();
        assert!(create(&s.0, "my-app", &dependency()).is_ok());
    }

    #[test]
    fn a_bad_name_writes_nothing() {
        let s = Scratch::new("bad-name");
        assert!(create(&s.0, "1app", &dependency()).is_err());
        assert!(!s.0.exists(), "nothing may be created for a refused name");
    }
}
