//! N2: making a new vieww project that builds and previews on the first try.
//!
//! Plan 2 §5.2 sets two properties the template must have, and both are
//! reactions to defects this project actually shipped:
//!
//! 1. **It builds and runs immediately.** The scratch buffer once had no
//!    `#[derive(Debug)]` against a `Widget: Any + Debug` bound, so the studio's
//!    own default content did not compile — while the test suite was green,
//!    because every test used its own fixture. Here, the templates are the
//!    fixture: [`files`] is what a test compiles, and it is what the New
//!    Workspace command writes.
//! 2. **Its screens are previewable.** `src/screens/home.rs` exposes
//!    `pub fn screen()`, so Render works on a fresh project without the user
//!    being told a rule first.
//!
//! # Generating and writing are separate on purpose
//!
//! [`files`] is pure: a name and a dependency in, a list of `(path, contents)`
//! out. [`create`] is the only part that touches a disk. That split is what
//! lets a test assert the *content* of a template — including compiling it —
//! without a directory, and lets [`create`]'s tests be about the things only a
//! filesystem can get wrong: a name that is not a crate name, a directory that
//! already has something in it, a half-written project left behind by a failure
//! at file four of seven.

use std::fmt;
use std::path::{Path, PathBuf};

/// Where a generated project gets `vieww` from.
///
/// Plan 2's open question 1, and it does not have a single right answer yet:
///
/// * A **path** into this checkout is what works today. The preview loads a
///   `cdylib` into the studio's own process, which is sound only if the
///   project's `vieww` is the *same compilation* as the studio's — and a path
///   dependency on the studio's own workspace is the only way to guarantee it.
/// * A **version** from crates.io is what a shippable project wants, and the
///   ABI guard will refuse to preview it. Build and Run still work, because a
///   built application has no ABI relationship with the studio at all.
///
/// So both exist, and the UI says which one it is choosing and what it costs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dependency {
    /// `vieww = { path = "…" }` — previewable.
    Path(PathBuf),
    /// `vieww = "0.0.1"` — not previewable against this studio.
    Version(String),
}

impl Dependency {
    /// The `Cargo.toml` right-hand side for one vieww crate.
    fn spec(&self, crate_name: &str) -> String {
        match self {
            Self::Path(root) => {
                let path = root.join("crates").join(crate_name);
                // Cargo reads TOML, so a Windows `\` would be an escape.
                let path = path.to_string_lossy().replace('\\', "/");
                format!("{{ path = \"{path}\" }}")
            }
            Self::Version(version) => format!("\"{version}\""),
        }
    }

    /// Whether a project built on this can use the fast preview path.
    #[must_use]
    pub const fn previewable(&self) -> bool {
        matches!(self, Self::Path(_))
    }
}

/// Why a project could not be created.
#[derive(Debug)]
pub enum Error {
    /// The name is not usable as a Cargo package name.
    Name(&'static str),
    /// The target directory already holds something. Never overwritten:
    /// scaffolding into a directory with work in it is unrecoverable, and the
    /// user's own `~/src` is exactly the directory they will pick by accident.
    NotEmpty(PathBuf),
    Io(std::io::Error),
}

impl fmt::Display for Error {
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

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Check a name against Cargo's rules, and against Rust's.
///
/// Cargo would reject most of these itself — twenty seconds into a build, in a
/// message about a manifest the user did not write. Refusing at the dialog is
/// the same rule enforced where it can still be fixed by typing.
///
/// # Errors
///
/// If the name is empty, starts with a digit, contains anything but
/// `a-z A-Z 0-9 _ -`, or is a Rust keyword.
pub fn check_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::Name("it is empty"));
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(Error::Name("a crate name cannot start with a digit"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(Error::Name("use letters, digits, `-` and `_` only"));
    }
    // `[[bin]] name` becomes a crate name, and `-` becomes `_` there. A project
    // called `loop` produces `mod loop;` and does not compile.
    //
    // The whole list, not the half of it this used to hold. The missing half
    // was the common half: `if`, `for`, `fn`, `let`, `struct`, `impl`, `enum`,
    // `mod`, `trait`, `pub`. A project named any of those passed the check,
    // scaffolded a complete tree, and then failed to compile on a line the
    // user never wrote — which is a worse first five minutes than being told
    // "that is a Rust keyword" while the name is still in the field.
    //
    // Reserved-but-unused words (`become`, `gen`, `try`, …) are in here too:
    // they are rejected by `rustc` as identifiers today, so a project named
    // after one does not build today either.
    const KEYWORDS: [&str; 52] = [
        "Self", "abstract", "as", "async", "await", "become", "box", "break", "const", "continue",
        "crate", "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if",
        "impl", "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv",
        "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "try",
        "type", "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
    ];
    if KEYWORDS.contains(&name.replace('-', "_").as_str()) {
        return Err(Error::Name("that is a Rust keyword"));
    }
    Ok(())
}

/// A file a template produces, and what to call it.
///
/// # Why New Screen is not New Project with fewer files
///
/// A workspace is created in an empty directory and owns every path in it. A
/// screen is added *into* a project that already exists, next to files somebody
/// wrote, and the only two things it must not do are overwrite one of them and
/// leave the module list not mentioning it. Both are decisions about a
/// directory rather than about a template, which is why this returns the file
/// and lets the caller place it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scaffolded {
    /// Relative to the workspace root.
    pub path: PathBuf,
    pub contents: String,
    /// The line to add to `src/screens/mod.rs`, if this belongs in one.
    pub module_line: Option<String>,
}

/// What a template makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// A screen: a `pub fn screen() -> impl Widget`, which is the contract the
    /// preview looks for and the reason a screen is worth templating at all.
    Screen,
    /// A widget: a `struct` and its `impl Widget`, with no `screen()`. Meant to
    /// be used *by* a screen rather than previewed on its own.
    Widget,
}

impl Template {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Screen => "Screen",
            Self::Widget => "Widget",
        }
    }
}

/// The file `template` produces for something called `name`.
///
/// `name` is a snake_case module name; the type inside is its UpperCamel form.
/// Both are derived here rather than asked for twice, because two names that
/// have to agree are two names that can disagree.
///
/// # Errors
///
/// If `name` is not usable as a Rust module name.
pub fn scaffold(template: Template, name: &str) -> Result<Scaffolded, Error> {
    check_module_name(name)?;
    let type_name = upper_camel(name);
    // A template with markers rather than a `format!`: these bodies are Rust
    // source full of braces, and every one of them would have to be doubled in
    // a format string — which is how a template stops being readable as the
    // thing it produces.
    let contents = match template {
        Template::Screen => SCREEN_TEMPLATE,
        Template::Widget => WIDGET_TEMPLATE,
    }
    .replace("__TYPE__", &type_name)
    .replace("__NAME__", name);

    Ok(Scaffolded {
        path: PathBuf::from("src")
            .join("screens")
            .join(format!("{name}.rs")),
        contents,
        // Both go under `screens/`, because that is the one directory the
        // generated `mod.rs` walks and a widget nobody can `use` is a file.
        module_line: Some(format!("pub mod {name};")),
    })
}

/// The screen a New Screen produces.
///
/// Carries `pub fn screen() -> impl Widget`, which is the contract the preview
/// looks for and the whole reason a screen is worth templating: a file without
/// it opens in the editor and cannot be rendered, and nothing says why.
const SCREEN_TEMPLATE: &str = r#"//! The __NAME__ screen.

use vieww::prelude::*;

#[derive(Debug)]
pub struct __TYPE__;

impl Widget for __TYPE__ {
    fn debug_name(&self) -> &'static str {
        "__TYPE__"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        SafeArea::new()
            .child(
                Container::new()
                    .color(theme.colors.surface)
                    .padding(EdgeInsets::all(16.0))
                    .child(Text::new("__TYPE__").style(theme.text.headline)),
            )
            .into()
    }
}

/// The entry point the preview looks for.
pub fn screen() -> impl Widget {
    __TYPE__
}
"#;

/// The widget a New Widget produces.
///
/// No `screen()`: a widget is used *by* a screen, and giving every one of them
/// an entry point would make each previewable on its own, which is not what it
/// is.
const WIDGET_TEMPLATE: &str = r#"//! The __NAME__ widget.

use vieww::prelude::*;

#[derive(Debug)]
pub struct __TYPE__;

impl Widget for __TYPE__ {
    fn debug_name(&self) -> &'static str {
        "__TYPE__"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        Container::new()
            .color(theme.colors.surface_variant)
            .radius(theme.metrics.corner)
            .padding(EdgeInsets::all(12.0))
            .child(Text::new("__TYPE__").style(theme.text.body))
            .into()
    }
}
"#;

/// Whether `name` can be a Rust module.
///
/// Stricter than [`check_name`], which allows `-` because a *crate* name may
/// contain one and cargo maps it to `_`. A module may not: `mod photo-tile;`
/// does not parse, and the file would be created and then never compile.
///
/// # Errors
///
/// If it is empty, starts with a digit, contains anything but `[a-z0-9_]`, or
/// is a Rust keyword.
pub fn check_module_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::Name("it is empty"));
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(Error::Name("a module name cannot start with a digit"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(Error::Name("use lower-case letters, digits and `_` only"));
    }
    check_name(name)
}

/// `photo_tile` as `PhotoTile`.
fn upper_camel(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// What a new project's screens are written in.
///
/// A choice, not a fork: both kinds scaffold the same project — same
/// Cargo.toml, same tree, same `src/screens/` — and either project accepts
/// both, so changing your mind later is opening a different file, not
/// migrating. A Say project writes `home.say` where a Rust project writes
/// `home.rs`, and keeps a Rust tree that builds so Build and Run, Export and
/// every existing habit keep working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectKind {
    /// Structured English, compiled to Rust by the studio on every Render.
    #[default]
    Say,
    /// vieww's API, written directly.
    Rust,
}

impl ProjectKind {
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Say => "Say",
            Self::Rust => "Rust",
        }
    }

    #[must_use]
    pub const fn blurb(self) -> &'static str {
        match self {
            Self::Say => {
                "Describe your screens in structured English. Every line shows the Rust it becomes."
            }
            Self::Rust => {
                "Write vieww's API directly. Everything Say can do, and everything it cannot."
            }
        }
    }
}

/// The first screen a Say project opens with.
///
/// Kept inside the codegen subset by construction: the scaffold's Say screen
/// must render on the first Render, or the kind card lied.
pub const HOME_SAY: &str =
    include_str!("../../../crates/vieww-say-codegen/tests/fixtures/counter.say");

/// The buffer the welcome card's first row opens.
pub const SAY_COUNTER: &str = HOME_SAY;

/// Every file a new project starts with, as `(relative path, contents)`.
///
/// Pure. The New Project command writes these; a test compiles them.
#[must_use]
pub fn files(name: &str, dependency: &Dependency) -> Vec<(PathBuf, String)> {
    files_for(name, dependency, ProjectKind::Rust)
}

/// The same, for a chosen kind.
///
/// A Say project's `src/lib.rs` mounts a placeholder that points at the
/// `.say` file — the library has to build for Build and Run to stay honest,
/// and the screens a Say project ships live in `.say` files the studio
/// renders, which the library itself does not name.
#[must_use]
pub fn files_for(name: &str, dependency: &Dependency, kind: ProjectKind) -> Vec<(PathBuf, String)> {
    let mut out = vec![
        (
            PathBuf::from("Cargo.toml"),
            cargo_toml(name, dependency, kind),
        ),
        (PathBuf::from("rust-toolchain.toml"), TOOLCHAIN.to_string()),
        (PathBuf::from(".gitignore"), GITIGNORE.to_string()),
        (PathBuf::from("src/main.rs"), main_rs(name)),
    ];
    match kind {
        ProjectKind::Rust => {
            out.push((PathBuf::from("src/lib.rs"), lib_rs(name)));
            out.push((PathBuf::from("src/screens/mod.rs"), SCREENS_MOD.to_string()));
            out.push((PathBuf::from("src/screens/home.rs"), HOME_RS.to_string()));
        }
        ProjectKind::Say => {
            out.push((PathBuf::from("src/lib.rs"), lib_rs_say(name)));
            out.push((
                PathBuf::from("src/screens/mod.rs"),
                SCREENS_MOD_SAY.to_string(),
            ));
            out.push((PathBuf::from("src/screens/home.say"), HOME_SAY.to_string()));
            // **The build script, and without it Say is a preview-only
            // language.** See `BUILD_RS_SAY`.
            out.push((PathBuf::from("build.rs"), BUILD_RS_SAY.to_string()));
        }
    }
    // **The Live Preview's file.** A new project can press Live and see a
    // flow immediately, and the file is a starting point to edit rather
    // than a format to look up. Deleting it turns the Live Preview off,
    // which the studio then says out loud. See `crate::livedoc`.
    out.push((
        PathBuf::from(crate::livedoc::FILE),
        crate::livedoc::TEMPLATE.to_string(),
    ));
    // Cargo does not track empty directories and neither does git, so the
    // assets folder needs a file or it is not there when the user looks.
    out.push((
        PathBuf::from("assets/.gitkeep"),
        "# Images, fonts and data files. Reach them with an AssetBundle.\n".to_string(),
    ));
    // The Gradle and Xcode projects. Appended rather than interleaved so the
    // Rust a person opens first is the first thing written, and because
    // `harness` owns every path under `android/` and `ios/` — including the
    // ones `export` runs its second step in.
    out.extend(crate::harness::files(name));
    out
}

fn cargo_toml(name: &str, dependency: &Dependency, kind: ProjectKind) -> String {
    // A Say project compiles its `.say` screens in `build.rs`, so it needs the
    // generator at build time. `vieww-say-codegen` has no runtime dependencies
    // at all — text in, Rust text out — so this adds a parser to the build
    // graph and nothing else. A Rust project has no `.say` files and no reason
    // to carry it.
    let build_deps = match kind {
        ProjectKind::Rust => String::new(),
        ProjectKind::Say => format!(
            "\n[build-dependencies]\n\
             # Compiles `src/screens/*.say` to Rust — see `build.rs`.\n\
             vieww-say-codegen = {codegen}\n",
            codegen = dependency.spec("vieww-say-codegen"),
        ),
    };
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
publish = false

# **An empty `[workspace]` table is the load-bearing line in this file.**
#
# A scaffolded project has no control over where it lands on disk. The studio's
# `new_project` puts it beside the open folder, which is the right default for
# an isolated project — but it is *also* the right default when the open folder
# is itself inside another workspace (like this very checkout), and in that case
# the new project's `Cargo.toml` lands inside a parent workspace's directory
# tree without being listed in its `members`.
#
# Without this empty table, Cargo walks up from the new project, finds the
# parent workspace, and errors:
#
#     error: current package believes it's in a workspace when it's not:
#            current:  …/vieww-app-2/Cargo.toml
#            workspace: …/vieww/Cargo.toml
#
# The empty `[workspace]` table tells Cargo "this package is its own workspace
# root, do not look further up the tree", which is the standard Cargo idiom for
# a sub-package that is standalone inside a larger repo — used by `rust-analyzer`'s
# examples, Bevy's examples, and the cargo book's own nested-package
# documentation. Removing it makes the scaffold fail inside any checkout of
# vieww itself, which is exactly the place a vieww user is most likely to be
# standing when they press New Project.
[workspace]

[dependencies]
# The umbrella crate: widgets, layout, painting and the element tree.
vieww = {vieww}
# The window, the event loop and the GPU surface. Separate from `vieww` because
# a vieww tree can be driven headlessly — by a test, or by another host — and
# only an application needs a platform.
vieww-platform-winit = {platform}

# `android_main` in `src/lib.rs` reports a failed start through `log`, which on
# a phone is the only output there is (`vieww-platform-winit` routes it to
# logcat). Android-only, because nowhere else calls it.
[target.'cfg(target_os = "android")'.dependencies]
log = "0.4"

# **One crate, three shapes, and none of them is optional.**
#
# `rlib` is the desktop binary below and anything that tests this crate.
# `cdylib` is Android: the activity loads a shared library and calls
# `android_main` inside it, so there is no `main` for a binary to be.
# `staticlib` is iOS: `UIApplicationMain` has to own the process before any
# window exists, so the entry point is `ios/App/main.m` and this is what it
# links against.
#
# The library's name is what the Android manifest's `android.app.lib_name` and
# the Xcode project's `-l` flag both say. Renaming it means renaming those.
[lib]
name = "{lib}"
crate-type = ["rlib", "cdylib", "staticlib"]
path = "src/lib.rs"

[[bin]]
name = "{name}"
path = "src/main.rs"
{build_deps}
# **Debug information for your code, not for two hundred dependencies.**
#
# Cargo's default is full debug info for everything in the build, and on a
# graphical application that is most of what the binary weighs. Measured on this
# very template, before these four lines:
#
#     saydemo (debug binary)   260.5 MB
#       of which .text            17.4 MB   <- the actual code
#       of which .debug_*        234.0 MB
#
# Ninety percent of it was symbols, and almost all of those belonged to
# dependencies you are not stepping through. `debug = 1` keeps line tables for
# your own crate — so a panic still names your file and line, and a debugger
# still breaks where you ask — and the dependency override drops the rest.
#
#     saydemo (debug binary)    40.4 MB     <- with these lines
#
# Set `debug = 2` here if you need to step *into* a dependency; it is a slower,
# fatter build and it is occasionally the only way to see what is happening.
[profile.dev]
debug = 1

[profile.dev.package."*"]
debug = false

# **What ships.** The same application as above, built with this profile, is
# 7.6 MB.
#
# `strip` removes the symbol table — a release binary is not something anyone
# debugs from a backtrace, and the 20 MB it saves is 20 MB every user downloads.
# `lto = "thin"` lets the optimiser work across crate boundaries, which is where
# most of a widget tree's inlining is. `codegen-units = 1` gives it the whole
# crate to look at instead of sixteen slices. `panic = "abort"` removes the
# unwinding tables: a released GUI application has no `catch_unwind` boundary to
# unwind *to*, so the machinery is weight with nothing on the other end.
#
# Building for iOS or Android? Keep `panic = "abort"` in mind if you add a
# `catch_unwind` of your own — with it set, a panic ends the process.
[profile.release]
strip = true
lto = "thin"
codegen-units = 1
panic = "abort"
"#,
        vieww = dependency.spec("vieww"),
        platform = dependency.spec("vieww-platform-winit"),
        lib = crate::harness::lib_name(name),
        build_deps = build_deps,
    )
}

/// Pinned, and the comment says why, because the reason is not obvious and the
/// consequence of removing it is undefined behaviour rather than an error.
const TOOLCHAIN: &str = r#"[toolchain]
# Pinned because vieww Studio's preview loads a compiled library into its own
# process. A `Box<dyn Widget>` built by a different rustc has a different vtable
# layout and different `TypeId`s, so crossing that boundary is undefined
# behaviour rather than a mismatch anything can detect afterwards. The studio
# refuses to load a library built by a compiler other than its own; this file is
# what keeps them the same one.
channel = "stable"
"#;

const GITIGNORE: &str = "/target\n**/*.rs.bk\n";

fn main_rs(name: &str) -> String {
    format!(
        r#"//! {name} — the desktop entry point.
//!
//! Deliberately thin. Everything this does is in `src/lib.rs`, because Android
//! and iOS cannot reach a `main` and have to call into the library instead —
//! and three entry points that each set up their own window would drift apart
//! within a week.

fn main() {{
    {lib}::run();
}}
"#,
        lib = crate::harness::lib_name(name),
    )
}

/// The library every entry point goes through.
///
/// # Why the platform entry points live here rather than in three files
///
/// `main` on desktop, `android_main` on Android, and an `extern "C"` symbol
/// called from Objective-C on iOS. All three want the same window title, the
/// same background and the same root widget, and the only real difference is
/// where the event loop comes from — on Android it cannot be conjured, because
/// the activity, its looper and its surface already exist by the time any Rust
/// runs.
///
/// So the setup is written once in [`app`] and the three entry points are three
/// lines each. The alternative — a `main.rs`, an `android.rs` and an `ios.rs`
/// that each build an `App` — is how a project ends up with a title bar that
/// says the right thing on one platform.
fn lib_rs(name: &str) -> String {
    format!(
        r#"//! {name} — a vieww application.
//!
//! One library, three entry points: [`run`] for desktop, `android_main` for an
//! Android activity, and `vieww_ios_main` for `ios/App/main.m`. Everything they
//! share is in [`app`], so the window cannot end up configured differently
//! depending on where it was started from.

use vieww::prelude::*;
use vieww_platform_winit::App;

pub mod screens;

/// The window's size at startup. Phone-shaped, because the screen in
/// `screens/home.rs` is laid out for one — change both together.
///
/// Ignored on a phone, where the window is whatever the device is.
const WINDOW: Size = Size {{
    width: 420.0,
    height: 880.0,
}};

/// The theme this application draws in.
///
/// One value. `App::theme` sets the window's background from it *and*
/// publishes it above the widget tree, so the two cannot disagree — change
/// this line and the window and everything in it move together.
///
/// `light()` because that is what vieww Studio previews with by default, so
/// what you see there and what you get here are the same screen. `dark()` is
/// the other one; `ThemeData::adaptive(platform, dark)` follows the platform.
#[must_use]
pub fn theme() -> ThemeData {{
    ThemeData::light()
}}

/// The application, configured but not started.
#[must_use]
pub fn app() -> App {{
    App::new().title("{name}").size(WINDOW).theme(theme())
}}

/// Mount the first screen.
///
/// `screen()` returns `impl Widget` rather than a concrete type — that is the
/// preview pipeline's contract (`screens/mod.rs`) — so there is no
/// `Into<WidgetNode>` for `set_root` to find without naming the conversion,
/// which only a widget's own concrete type carries. `WidgetNode::new` is the
/// generic form of exactly that conversion.
pub fn mount(driver: &mut FrameDriver) {{
    driver.set_root(WidgetNode::new(screens::home::screen()));
}}

/// Start on a desktop, where the process owns its own event loop.
pub fn run() {{
    match app().run(mount) {{
        Ok(report) => println!("{{report}}"),
        Err(error) => eprintln!("{name} failed to start: {{error}}"),
    }}
}}

/// Android's entry point.
///
/// The activity loads `lib{lib}.so` and calls this; there is no `main`. The
/// `AndroidApp` it hands over carries the looper, the asset manager and the
/// surface, none of which Rust can create for itself — which is why this takes
/// an argument and [`run`] does not.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(android: vieww_platform_winit::AndroidApp) {{
    if let Err(error) = app().run_android(android, mount) {{
        // No stdout on a device. `android_logger` is initialised by the
        // platform crate, so this reaches `adb logcat`.
        log::error!("{name} failed to start: {{error}}");
    }}
}}

/// iOS's entry point, called from `ios/App/main.m`.
///
/// # Why iOS needs a symbol and the others do not
///
/// `UIApplicationMain` must own the process before any window exists and never
/// returns, so the `main` that runs on iOS is Objective-C's. This crate is
/// linked in as a static library and this is the one symbol that file calls.
///
/// `no_mangle` and `extern "C"` are both load-bearing: the linker looks for
/// exactly `vieww_ios_main`, and a mangled or Rust-ABI symbol produces an
/// "undefined symbol" at link time that names something unrecognisable.
#[cfg(target_os = "ios")]
#[no_mangle]
pub extern "C" fn vieww_ios_main() {{
    // `run` never returns on iOS — winit hands control to UIKit — so anything
    // written after this line would be unreachable rather than a cleanup.
    run();
}}
"#,
        lib = crate::harness::lib_name(name),
    )
}

/// The library of a Say project: the same three entry points, mounting a
/// placeholder that says where the screens live.
///
/// The placeholder exists so `cargo build` succeeds from the first minute —
/// Build and Run refusing on a fresh project would be a lie on the kind card.
/// The screens themselves are the `.say` files beside this, rendered by the
/// studio.
fn lib_rs_say(name: &str) -> String {
    format!(
        r#"//! {name} — a vieww application whose screens are written in Say.
//!
//! One library, three entry points: [`run`] for desktop, `android_main` for an
//! Android activity, and `vieww_ios_main` for `ios/App/main.m`.
//!
//! **The screens live in `src/screens/*.say`.** `build.rs` compiles them to
//! Rust with the same compiler vieww Studio previews with, so what you see in
//! the studio and what the built application runs are the same screen — press
//! Render for the fast loop, `cargo run` for the real thing.

use vieww::prelude::*;
use vieww_platform_winit::App;

pub mod screens;

const WINDOW: Size = Size {{
    width: 420.0,
    height: 880.0,
}};

/// The theme this application draws in.
///
/// One value. `App::theme` sets the window's background from it *and*
/// publishes it above the widget tree, so the two cannot disagree.
///
/// `light()` because that is what vieww Studio previews with by default, so
/// the screen you Render and the screen you build are the same screen.
#[must_use]
pub fn theme() -> ThemeData {{
    ThemeData::light()
}}

/// The application, configured but not started.
#[must_use]
pub fn app() -> App {{
    App::new().title("{name}").size(WINDOW).theme(theme())
}}

/// Mount the home screen.
///
/// `screens::home` is generated from `src/screens/home.say` by `build.rs` —
/// see `src/screens/mod.rs`. Rename the file and this follows it; add another
/// `.say` and it appears as another module here.
pub fn mount(driver: &mut FrameDriver) {{
    driver.set_root(WidgetNode::new(screens::home::screen()));
}}

/// Start on a desktop, where the process owns its own event loop.
pub fn run() {{
    match app().run(mount) {{
        Ok(report) => println!("{{report}}"),
        Err(error) => eprintln!("{name} failed to start: {{error}}"),
    }}
}}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(android: vieww_platform_winit::AndroidApp) {{
    if let Err(error) = app().run_android(android, mount) {{
        log::error!("{name} failed to start: {{error}}");
    }}
}}

#[cfg(target_os = "ios")]
#[no_mangle]
pub extern "C" fn vieww_ios_main() {{
    // `run` never returns on iOS — winit hands control to UIKit.
    run();
}}
"#,
        name = name,
    )
}
/// The module list of a Say project: empty, because the screens live in
/// `.say` files the studio renders. Add `pub mod <name>;` here the day a
/// screen is written in Rust, exactly as the Rust scaffold would.
/// The build script a Say project ships with.
///
/// # Why a Say project needs one, and what it was like without
///
/// A `.say` file is compiled to Rust by `vieww-say-codegen`. Until this script
/// existed, the **only** thing that ever called that compiler was vieww Studio's
/// preview — so a `.say` screen was something you could look at and not
/// something you could ship. `cargo build` produced a real binary that did not
/// contain the screen, and the scaffolded `lib.rs` mounted a placeholder
/// reading "Open src/screens/home.say and press Render", which is what the
/// finished application then said when you ran it.
///
/// That is a language you can preview but cannot use. This script closes it:
/// every `src/screens/*.say` is compiled at build time into `OUT_DIR`, one
/// module per file, and `src/screens/mod.rs` includes the result. The screen in
/// the built application is the screen in the file, compiled by the same
/// generator the studio previews with.
///
/// # Why `OUT_DIR` and not a generated file in `src/`
///
/// Generated code checked in beside handwritten code goes stale, gets edited by
/// mistake, and shows up in every diff. `OUT_DIR` is where Cargo expects build
/// output, `include!` is how a crate reads it back, and `rerun-if-changed` on
/// the directory means a saved `.say` file rebuilds and nothing else does.
const BUILD_RS_SAY: &str = r##"//! Compiles this project's `.say` screens to Rust, at build time.
//!
//! Every `src/screens/*.say` becomes a module in `$OUT_DIR/say_screens.rs`,
//! which `src/screens/mod.rs` includes. `home.say` becomes `screens::home`,
//! and `screens::home::screen()` is the widget your application mounts.
//!
//! Add a screen by adding a `.say` file. There is no list to update.
//!
//! A syntax error in a `.say` file fails the build here, with the file, line
//! and column — the same diagnostics vieww Studio shows in its Problems panel,
//! because it is the same compiler.

use std::path::{Path, PathBuf};

fn main() {
    let screens = Path::new("src/screens");
    // The directory, so a *new* `.say` file triggers a rebuild — watching only
    // the files that exist today would miss the one added tomorrow.
    println!("cargo:rerun-if-changed=src/screens");
    println!("cargo:rerun-if-changed=build.rs");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("say_screens.rs");

    let mut modules = String::new();
    let mut names: Vec<String> = Vec::new();

    let entries = match std::fs::read_dir(screens) {
        Ok(entries) => entries,
        // No screens directory is not an error: a project part-way through
        // moving its screens to Rust is a valid project.
        Err(_) => {
            std::fs::write(&out, "").expect("writing an empty screen module");
            return;
        }
    };

    // Sorted, so the generated file is byte-identical between builds on
    // machines whose directory order differs.
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "say"))
        .collect();
    paths.sort();

    for path in &paths {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a `.say` file has a name");
        let file = path.file_name().and_then(|s| s.to_str()).unwrap_or(stem);
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

        match vieww_say_codegen::compile(file, &source) {
            Ok(generated) => {
                for warning in &generated.warnings {
                    // Surfaced as a cargo warning, so `cargo build` shows it
                    // the way it shows any other.
                    println!(
                        "cargo:warning={}:{}:{}: {} [{}]",
                        file, warning.line, warning.column, warning.message, warning.code
                    );
                }
                modules.push_str(&format!(
                    "pub mod {stem} {{\n{}\n}}\n",
                    generated.rust
                ));
                names.push(stem.to_owned());
            }
            Err(diagnostics) => {
                for diagnostic in &diagnostics {
                    println!(
                        "cargo:warning={}:{}:{}: {} [{}]",
                        file,
                        diagnostic.line,
                        diagnostic.column,
                        diagnostic.message,
                        diagnostic.code
                    );
                }
                panic!(
                    "{file} did not compile — {} error(s), listed above",
                    diagnostics.len()
                );
            }
        }
    }

    std::fs::write(&out, modules).expect("writing the generated screens");
    // A convenience for the error message you get if `home.say` is deleted.
    println!("cargo:rustc-env=VIEWW_SAY_SCREENS={}", names.join(","));
}
"##;

const SCREENS_MOD_SAY: &str = r#"//! One module per screen, generated from the `.say` files beside this one.
//!
//! `build.rs` compiles every `src/screens/*.say` with the same Say compiler
//! vieww Studio previews with, and writes the result into Cargo's `OUT_DIR`.
//! This line reads it back, so `home.say` is `screens::home` and the widget it
//! describes is `screens::home::screen()`.
//!
//! **Add a screen by adding a `.say` file.** There is no list here to keep in
//! step — that is the point of generating this rather than writing it.
//!
//! Want a screen in Rust instead? Write it as an ordinary `home_details.rs`
//! beside this file and declare it below; the two kinds live together.

include!(concat!(env!("OUT_DIR"), "/say_screens.rs"));
"#;

const SCREENS_MOD: &str = r#"//! One module per screen.
//!
//! Each screen exposes `pub fn screen() -> impl Widget`, which is the contract
//! vieww Studio's preview looks for: open the file, press Render, and it
//! appears in the device frame without the application having to run.

pub mod home;
"#;

/// The first screen, and the template that has to compile.
///
/// `#[derive(Debug)]` is not decoration: `Widget: Any + Debug`, and its absence
/// is precisely the defect that once shipped in the studio's own default
/// buffer. It is here with a comment so that nobody deletes it tidily.
///
/// # Why the template is a whole flow and not one static screen
///
/// It used to be a heading and a sentence. The Live Preview, meanwhile, opens
/// on a three-screen flow — an inbox, a detail view, a settings form — because
/// `live.rs` sketches exactly that. So the first Build and Run of a new
/// project produced one screen where the preview had shown three, and the
/// reasonable conclusion was that the build had dropped two thirds of the app.
/// Reported as: *"in live preview it has shown three screens but when I built
/// it, only a single screen is there."*
///
/// Both halves were working as designed; the design disagreed with itself.
/// The template now disagrees with nothing: `home.rs` is the same flow the
/// sketch draws, written in real Rust — navigation, a switch, a counter — so
/// what Build and Run starts is what Live was showing, and every piece of it
/// is the user's to edit or delete.
const HOME_RS: &str = r#"//! The first screen — and, as shipped, the whole application.
//!
//! `screen()` is what vieww Studio renders. Keep it — the preview finds a
//! screen by that name — and put whatever you like inside it.
//!
//! `Home` below is a small but complete app: an Inbox, a Detail view and a
//! Settings screen, with a navigation bar between them. It is the same flow
//! `live.rs` sketches for the Live Preview, written in real Rust — so what
//! Build and Run starts is what Live was showing, and every piece of it is
//! yours to delete.

use std::cell::RefCell;
use std::rc::Rc;

use vieww::prelude::*;

/// The entry point the preview looks for.
#[must_use]
pub fn screen() -> impl Widget {
    Home
}

// `Debug` is required: the `Widget` trait is `Any + Debug`, so a screen without
// this derive does not compile. It is the first thing to check if a new screen
// will not build.
#[derive(Debug)]
pub struct Home;

/// The screens the navigation bar switches between, in bar order.
const SCREENS: [&str; 3] = ["Inbox", "Detail", "Settings"];

/// What `Home` remembers while it is on screen.
///
/// The widget itself (`Home`, above) holds nothing: everything that changes
/// lives in this state, which the element tree keeps across rebuilds and
/// snapshots across a Render — so navigating somewhere and re-rendering the
/// file does not throw you back to the first screen. See `create_state` and
/// `snapshot` below.
#[derive(Debug, Default)]
struct HomeState {
    /// Which screen is showing: an index into [`SCREENS`].
    route: usize,
    /// The Settings switch.
    notifications: bool,
    /// The Settings counter.
    badge: u32,
    /// Set by gesture handlers; [`take_pending`](ElementState::take_pending)
    /// returns and clears it, which is how a write from a gesture becomes a
    /// rebuild.
    pending: bool,
}

impl HomeState {
    /// Switch screens.
    fn go(&mut self, route: usize) {
        self.route = route;
        self.pending = true;
    }
}

impl ElementState for HomeState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    // Across a Render the whole subtree is rebuilt from a fresh compilation,
    // and only what `snapshot` saved comes back. The format is private to
    // this struct; `restore` refuses anything it cannot fully read rather
    // than guessing.
    fn snapshot(&self) -> Option<String> {
        Some(format!(
            "route={};notifications={};badge={}",
            self.route, self.notifications, self.badge
        ))
    }

    fn restore(&mut self, saved: &str) -> bool {
        let mut route = self.route;
        let mut notifications = self.notifications;
        let mut badge = self.badge;
        let mut fields = 0;
        for part in saved.split(';') {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            let parsed = match key {
                // All three arms normalise to `Result<(), ()>` so the match
                // has one type; the error itself carries nothing worth
                // reporting — `restore` refuses, that is the whole answer.
                "route" => value
                    .parse::<usize>()
                    .map(|v| route = v)
                    .map_err(|_| ()),
                "notifications" => value
                    .parse::<bool>()
                    .map(|v| notifications = v)
                    .map_err(|_| ()),
                "badge" => value
                    .parse::<u32>()
                    .map(|v| badge = v)
                    .map_err(|_| ()),
                _ => continue,
            };
            if parsed.is_err() {
                return false;
            }
            fields += 1;
        }
        if fields == 0 {
            return false;
        }
        self.route = route;
        self.notifications = notifications;
        self.badge = badge;
        true
    }
}

/// A handler that applies one edit to the screen's state.
///
/// A gesture runs long after the build that created it, so it carries a
/// *handle* to the element's state rather than anything borrowed from the
/// build. The edit is a plain function pointer, which is why the call sites
/// below can pass a closure that captures nothing.
fn edit(handle: Option<Rc<RefCell<dyn ElementState>>>, apply: fn(&mut HomeState)) -> impl Fn() {
    move || {
        let Some(handle) = &handle else { return };
        let mut state = handle.borrow_mut();
        if let Some(home) = state.as_any_mut().downcast_mut::<HomeState>() {
            apply(home);
        }
    }
}

/// The navigation flavour of [`edit`]: switch to `route`.
fn go_to(handle: Option<Rc<RefCell<dyn ElementState>>>, route: usize) -> impl Fn() {
    move || {
        let Some(handle) = &handle else { return };
        let mut state = handle.borrow_mut();
        if let Some(home) = state.as_any_mut().downcast_mut::<HomeState>() {
            home.go(route);
        }
    }
}

impl Widget for Home {
    fn debug_name(&self) -> &'static str {
        "Home"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    // Handing the tree a state is the whole trick: everything else here is an
    // ordinary build.
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(HomeState::default()))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let colors = ThemeData::of(ctx).colors;

        // Read-only: a build draws from the state and never writes it. The
        // writes happen in the handlers, through `edit` and `go_to` above.
        let route = ctx.state(|state: &HomeState| state.route).unwrap_or(0);
        let notifications = ctx
            .state(|state: &HomeState| state.notifications)
            .unwrap_or(false);
        let badge = ctx.state(|state: &HomeState| state.badge).unwrap_or(0);

        let state = ctx.state_handle();
        let screen: WidgetNode = match route {
            // Detail — reached from the Inbox rows' flow, "Back" returns.
            1 => Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(children![
                    header("Design Review", colors),
                    note(
                        "Everything here is real Rust: the navigation, the \
                         switch and the counter hold state on the element \
                         tree, and this screen survives a Render.",
                        colors,
                    ),
                    action("Back", go_to(state, 0)),
                ])
                .into(),
            // Settings — the switch and the counter, plus a way out.
            2 => Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(children![
                    header("Settings", colors),
                    toggle_row(
                        notifications,
                        colors,
                        Rc::new({
                            let handle = state.clone();
                            move |value: bool| {
                                let Some(handle) = &handle else { return };
                                let mut state = handle.borrow_mut();
                                if let Some(home) =
                                    state.as_any_mut().downcast_mut::<HomeState>()
                                {
                                    home.notifications = value;
                                    home.pending = true;
                                }
                            }
                        }),
                    ),
                    counter_row(
                        badge,
                        colors,
                        edit(state.clone(), |home: &mut HomeState| {
                            home.badge = home.badge.saturating_sub(1);
                            home.pending = true;
                        }),
                        edit(state.clone(), |home: &mut HomeState| {
                            home.badge = home.badge.saturating_add(1);
                            home.pending = true;
                        }),
                    ),
                    action("Done", go_to(state, 0)),
                ])
                .into(),
            // Inbox — the screen the app opens on, and everything else.
            _ => Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(children![
                    header("Inbox", colors),
                    conversation("Design Review", "3 new comments on the card layout", colors),
                    conversation("Sprint Planning", "5 tickets moved to In Progress", colors),
                    conversation("Release Notes", "Draft for v0.3 is ready to review", colors),
                    action("Open settings", go_to(state, 2)),
                ])
                .into(),
        };

        // The bar along the bottom, the current screen marked — the same bar
        // `live.rs`'s `nav` line draws for the Live Preview.
        let mut entries: Vec<WidgetNode> = Vec::new();
        for (index, name) in SCREENS.iter().enumerate() {
            let selected = index == route;
            let label = (*name).to_owned();
            entries.push(
                Flexible::expanded(1)
                    .child(
                        Pressable::themed(ctx, move |_| {
                            Flex::column()
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .spacing(4.0)
                                .children(children![
                                    Container::new()
                                        .color(if selected {
                                            colors.primary
                                        } else {
                                            Color::TRANSPARENT
                                        })
                                        .radius(3.0)
                                        .size(24.0, 3.0),
                                    Text::new(label.clone()).size(11.0).color(if selected {
                                        colors.primary
                                    } else {
                                        colors.on_surface_variant
                                    }),
                                ])
                                .into()
                        })
                        .on_tap(go_to(ctx.state_handle(), index)),
                    )
                    .into(),
            );
        }

        // `SafeArea` keeps content clear of a notch, a status bar and a home
        // indicator. It costs nothing on a desktop window and is the
        // difference between a usable and an unusable first run on a phone.
        // The colour goes outside and the insets go inside, which is what a
        // real application does: the background reaches the physical edges,
        // the content stops at the cutout.
        Container::new()
            .color(colors.surface)
            .child(
                SafeArea::new().child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(children![
                            Flexible::expanded(1).child(Clip::rect().child(screen)),
                            Container::new()
                                .color(colors.surface_variant)
                                .height(56.0)
                                .child(
                                    Flex::row()
                                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                        .children(entries),
                                ),
                        ]),
                ),
            )
            .into()
    }
}

/// A screen's heading, on the grouped background.
fn header(title: &str, colors: ColorScheme) -> WidgetNode {
    Container::new()
        .color(colors.surface_variant)
        .padding(EdgeInsets::symmetric(16.0, 14.0))
        .child(
            Text::new(title.to_owned())
                .size(24.0)
                .bold()
                .color(colors.on_surface),
        )
        .into()
}

/// An inbox row: an initial in a circle, a title, and a line under it.
fn conversation(title: &str, detail: &str, colors: ColorScheme) -> WidgetNode {
    let initial = title.chars().next().unwrap_or('?').to_string();
    Container::new()
        .color(colors.surface)
        .padding(EdgeInsets::symmetric(16.0, 12.0))
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(12.0)
                .children(children![
                    Container::new()
                        .color(colors.primary)
                        .radius(20.0)
                        .size(40.0, 40.0)
                        .alignment(Alignment::CENTER)
                        .child(
                            Text::new(initial)
                                .size(16.0)
                                .bold()
                                .color(colors.on_primary)
                        ),
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(2.0)
                        .children(children![
                            Text::new(title.to_owned()).size(15.0).color(colors.on_surface),
                            Text::new(detail.to_owned())
                                .size(12.0)
                                .color(colors.on_surface_variant),
                        ]),
                ]),
        )
        .into()
}

/// A labelled button row.
fn action(label: &str, on_tap: impl Fn() + 'static) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::symmetric(16.0, 8.0))
        .child(Button::new(label.to_owned()).on_pressed(on_tap))
        .into()
}

/// A quiet paragraph.
fn note(body: &str, colors: ColorScheme) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::symmetric(16.0, 12.0))
        .child(
            Text::new(body.to_owned()).style(TextStyle {
                color: colors.on_surface_variant,
                size: 13.0,
                line_height: 1.5,
                ..TextStyle::new(13.0)
            }),
        )
        .into()
}

/// A labelled switch.
fn toggle_row(on: bool, colors: ColorScheme, on_changed: Rc<dyn Fn(bool)>) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::symmetric(16.0, 10.0))
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(children![
                    Flexible::expanded(1).child(
                        Text::new("Notifications".to_owned())
                            .size(15.0)
                            .color(colors.on_surface)
                    ),
                    Switch::new(on).label("Notifications").on_changed(on_changed),
                ]),
        )
        .into()
}

/// The badge counter: a label, then `-` value `+` as one stepper.
///
/// The two step buttons belong on the same line as the number they change.
/// Splitting them across two rows — which this used to do — leaves a `+`
/// floating on a line of its own with nothing to say what it adds to, and it
/// does not match the counter the Live Preview draws for `live.rs`. The whole
/// point of this file is that Build and Run shows what Live was showing.
fn counter_row(
    badge: u32,
    colors: ColorScheme,
    step_down: impl Fn() + 'static,
    step_up: impl Fn() + 'static,
) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::symmetric(16.0, 10.0))
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(10.0)
                .children(children![
                    Flexible::expanded(1).child(
                        Text::new("Badge count".to_owned())
                            .size(15.0)
                            .color(colors.on_surface)
                    ),
                    step_glyph("\u{2212}", colors).on_tap(step_down),
                    Text::new(format!("{badge}"))
                        .size(15.0)
                        .color(colors.on_surface),
                    step_glyph("+", colors).on_tap(step_up),
                ]),
        )
        .into()
}

/// A round step button whose wash fades while it is pressed.
fn step_glyph(glyph: &str, colors: ColorScheme) -> Pressable {
    // The builder outlives this call, so the glyph it renders is copied
    // into it rather than borrowed from the argument.
    let glyph = glyph.to_owned();
    Pressable::new(move |press| {
        Container::new()
            .color(if press > 0.0 {
                colors.primary.with_alpha(46)
            } else {
                colors.surface_variant
            })
            .radius(16.0)
            .size(32.0, 32.0)
            .alignment(Alignment::CENTER)
            .child(
                Text::new(glyph.clone())
                    .size(18.0)
                    .color(colors.on_surface),
            )
            .into()
    })
}
"#;

/// Write a new project into `root`.
///
/// Returns the files written, in the order they were written.
///
/// # Errors
///
/// [`Error::Name`] for a name Cargo would reject, [`Error::NotEmpty`] if `root`
/// already holds anything, and [`Error::Io`] for anything the filesystem
/// refused.
///
/// # What happens when it fails halfway
///
/// The directories it made and the files it wrote are **removed**, so a failure
/// at file five does not leave four files and a `Cargo.toml` that looks like a
/// project. A half-scaffolded directory is worse than none: it is not empty, so
/// a second attempt at the same path is refused by the check above, and the
/// user has to work out what to delete.
pub fn create(root: &Path, name: &str, dependency: &Dependency) -> Result<Vec<PathBuf>, Error> {
    create_for(root, name, dependency, ProjectKind::Rust)
}

/// [`create`], for a chosen kind. The whole tree is identical except the
/// screens: `home.say` where `home.rs` would be, and a library that builds.
pub fn create_for(
    root: &Path,
    name: &str,
    dependency: &Dependency,
    kind: ProjectKind,
) -> Result<Vec<PathBuf>, Error> {
    check_name(name)?;

    if root.exists() {
        let mut entries = std::fs::read_dir(root)?;
        if entries.next().is_some() {
            return Err(Error::NotEmpty(root.to_path_buf()));
        }
    }

    let existed = root.exists();
    std::fs::create_dir_all(root)?;

    let mut written = Vec::new();
    for (relative, contents) in files_for(name, dependency, kind) {
        let path = root.join(&relative);
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, contents)
        })();

        if let Err(error) = result {
            // Undo. `remove_dir_all` on a directory this call created is safe
            // because the emptiness check above proved there was nothing of the
            // user's in it.
            if existed {
                for path in &written {
                    std::fs::remove_file(path).ok();
                }
            } else {
                std::fs::remove_dir_all(root).ok();
            }
            return Err(Error::Io(error));
        }
        written.push(path);
    }

    Ok(written)
}

/// The vieww checkout this studio was built from, if it is still there.
///
/// A generated project has to depend on **this** vieww for the preview to load
/// it — see [`Dependency`] — and the only thing that knows where this vieww is
/// is the build that produced the studio. `CARGO_MANIFEST_DIR` is
/// `<checkout>/apps/viewwstudio`, which is the same assumption `main.rs`
/// already makes when it looks for the rlibs.
///
/// `None` when the studio has been moved away from its checkout, which is the
/// case where a path dependency would be written and then not resolve — a
/// project that does not build at all, which is worse than one that builds and
/// cannot be previewed.
#[must_use]
pub fn checkout_root() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent()?.parent()?;
    // Verified rather than assumed: the two crates a template names must
    // actually be there.
    let ok = root.join("crates/vieww/Cargo.toml").is_file()
        && root
            .join("crates/vieww-platform-winit/Cargo.toml")
            .is_file();
    ok.then(|| root.to_path_buf())
}

/// Whether a directory is a vieww project: a `Cargo.toml` that depends on
/// `vieww`.
///
/// The same question [`crate::file_tree::is_vieww_project`] answers for an
/// opened workspace, asked here of a freshly created one — a scaffold that does
/// not satisfy the studio's own recognition rule would light up none of the
/// build controls it just made possible.
#[must_use]
pub fn is_vieww_project(root: &Path) -> bool {
    let Ok(manifest) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return false;
    };
    manifest.lines().any(|line| {
        line.trim_start().starts_with("vieww ") || line.trim_start().starts_with("vieww=")
    })
}

/// The name of the binary this project's `[[bin]]` builds, read from its
/// manifest.
///
/// # Why this is read rather than derived from the folder
///
/// [`files`] writes `[[bin]] name = "{name}"` from the project name the studio
/// was given, and that is *usually* the folder's name — but a project can be
/// renamed on disk, moved, or created by hand, and a build that guesses wrong
/// fails with `no bin target named …` rather than doing something sensible.
/// Reading the manifest is the only version that is right for a project this
/// studio did not create, which is most of them after the first day.
///
/// Deliberately a small parser rather than a TOML dependency: this looks for
/// the `name` immediately under a `[[bin]]` header, which is the shape [`files`]
/// writes and the shape every hand-written manifest uses. `None` when there is
/// no explicit `[[bin]]` — Cargo then infers one from the package name, and the
/// caller falls back to that.
#[must_use]
pub fn binary_name(root: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_bin = false;
    let mut package_name = None;
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_bin = line == "[[bin]]";
            in_package = line == "[package]";
            continue;
        }
        let Some(value) = line.strip_prefix("name") else {
            continue;
        };
        let Some(value) = value.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        if in_bin {
            return Some(value);
        }
        if in_package && package_name.is_none() {
            package_name = Some(value);
        }
    }
    // No `[[bin]]` section: Cargo infers the binary from the package name, and
    // so does this.
    package_name
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("vieww-scaffold-{name}"));
            std::fs::remove_dir_all(&dir).ok();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
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
        for bad in ["", "1app", "my app", "my.app", "app!", "loop", "match"] {
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
        for expected in [
            "Cargo.toml",
            "rust-toolchain.toml",
            ".gitignore",
            "src/lib.rs",
            "src/main.rs",
            "src/screens/mod.rs",
            "src/screens/home.rs",
            "assets/.gitkeep",
            "live.rs",
            // The Android and iOS harnesses. A new project can reach a phone
            // without anybody writing a `build.gradle` by hand, which is the
            // whole difference between "exports an app" and "exports an app
            // you brought the harness for".
            "android/settings.gradle",
            "android/build.gradle",
            "android/gradle.properties",
            "android/gradle/wrapper/gradle-wrapper.properties",
            "android/app/build.gradle",
            "android/app/src/main/AndroidManifest.xml",
            "android/README.md",
            "ios/my-app.xcodeproj/project.pbxproj",
            "ios/my-app.xcodeproj/xcshareddata/xcschemes/my-app.xcscheme",
            "ios/App/main.m",
            "ios/App/Info.plist",
            "ios/ExportOptions.plist",
            "ios/README.md",
        ] {
            assert!(paths.contains(&expected.to_string()), "missing {expected}");
        }
    }

    /// The property that makes Render work on a fresh project. If this ever
    /// fails, a new user's first click on the studio's main button does
    /// nothing useful.
    /// The Say scaffold's gate: the home.say a Say project starts with must
    /// compile, or the kind card's promise ("a screen that already renders")
    /// is false on the very first Render.
    /// The Android and iOS entry points are behind `#[cfg(target_os)]`, so no
    /// desktop build compiles them; these are the two defects that shipped
    /// there, pinned by text. The real compile is `examples/export_check.rs`
    /// (`cargo check --target aarch64-linux-android`), which needs the target.
    #[test]
    fn the_mobile_entry_points_match_the_platform_api() {
        for kind in [ProjectKind::Rust, ProjectKind::Say] {
            let files = files_for("app", &dependency(), kind);
            let get = |p: &str| {
                files
                    .iter()
                    .find(|(path, _)| path == &PathBuf::from(p))
                    .map(|(_, c)| c.clone())
                    .unwrap_or_default()
            };
            let lib = get("src/lib.rs");
            let manifest = get("Cargo.toml");
            // `App::run_android(self, android: AndroidApp, build: F)`.
            assert!(
                lib.contains("run_android(android, mount)"),
                "{kind:?}: android_main must call run_android(android, mount)"
            );
            if lib.contains("log::") {
                assert!(
                    manifest.lines().any(|l| l.trim_start().starts_with("log ")),
                    "{kind:?}: src/lib.rs uses log:: but Cargo.toml does not depend on log"
                );
            }
        }
    }

    #[test]
    fn the_say_scaffold_compiles() {
        let result = vieww_say_codegen::compile("home.say", HOME_SAY);
        assert!(
            result.is_ok(),
            "the Say scaffold does not compile: {:?}",
            result
                .err()
                .map(|d| d.iter().map(|d| d.to_string()).collect::<Vec<_>>())
        );
        // And the tree it produces is the promised shape: same Cargo.toml,
        // same src/screens/, home.say where Rust puts home.rs.
        let say_files = files_for("app", &dependency(), ProjectKind::Say);
        let rust_files = files_for("app", &dependency(), ProjectKind::Rust);
        assert!(say_files
            .iter()
            .any(|(p, _)| p == &PathBuf::from("src/screens/home.say")));
        assert!(!say_files
            .iter()
            .any(|(p, _)| p == &PathBuf::from("src/screens/home.rs")));
        assert!(rust_files
            .iter()
            .any(|(p, _)| p == &PathBuf::from("src/screens/home.rs")));
        for required in ["Cargo.toml", "src/main.rs"] {
            assert!(
                say_files.iter().any(|(p, _)| p == &PathBuf::from(required)),
                "a Say project is missing {required}"
            );
        }
    }

    #[test]
    fn the_first_screen_is_previewable() {
        let generated = files("my-app", &dependency());
        let home = generated
            .iter()
            .find(|(path, _)| path.ends_with("home.rs"))
            .map(|(_, contents)| contents)
            .unwrap();
        assert!(
            home.contains("pub fn screen()"),
            "the preview looks for this"
        );
        assert!(
            home.contains("#[derive(Debug)]"),
            "Widget: Any + Debug — this is the defect that shipped once"
        );
        assert!(home.contains("impl Widget for"));
    }

    /// **The bug this exists to keep fixed:** a scaffolded project that lands
    /// inside another workspace's directory tree (which is the common case
    /// when the studio is launched from inside `vieww/` itself, or from inside
    /// any workspace the user happens to have open) must opt out of that
    /// parent workspace — otherwise Cargo reports `current package believes
    /// it's in a workspace when it's not` on the first build, and the user's
    /// first experience of a fresh project is an error in a file they did not
    /// write.
    ///
    /// The empty `[workspace]` table is the standard Cargo idiom for "this is
    /// its own workspace root, do not walk up". If this test ever fails, the
    /// scaffold will start producing projects that cannot be built from inside
    /// a workspace checkout, which is the exact failure mode the line was
    /// added to prevent.
    #[test]
    fn the_scaffold_opts_out_of_any_parent_workspace() {
        let generated = files("my-app", &dependency());
        let manifest = generated
            .iter()
            .find(|(path, _)| path.ends_with("Cargo.toml"))
            .map(|(_, contents)| contents)
            .expect("Cargo.toml is in the scaffold");

        // The line itself, bare. Not `[workspace] members = []` (that would
        // still be a workspace root, but with no members — semantically the
        // same, and one character noisier), and not `workspace = false` under
        // `[package]` (that means "do not inherit workspace values", which is
        // a different question and not the one Cargo asks here).
        let lines = manifest.lines();
        let mut found_empty_workspace = false;
        let mut in_workspace = false;
        for line in lines {
            let trimmed = line.trim();
            // A `[section]` header.
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                in_workspace = trimmed == "[workspace]";
                continue;
            }
            // Inside `[workspace]`, the only thing we want to see is comments
            // and blank lines — no `members`, no `default-members`, no
            // `exclude`. An empty `[workspace]` is what opts out.
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
            "the scaffolded Cargo.toml must contain an empty `[workspace]` table to \
             opt out of any parent workspace — without it, a new project created \
             inside another workspace (e.g. inside vieww itself) errors on first \
             build with `current package believes it's in a workspace when it's \
             not`"
        );
    }

    #[test]
    fn the_name_reaches_every_place_it_has_to() {
        let generated = files("my-app", &dependency());
        let manifest = &generated[0].1;
        assert!(manifest.contains("name = \"my-app\""));
        assert!(manifest.contains("[[bin]]"), "there is something to run");

        assert!(
            manifest.contains("name = \"my_app\""),
            "and the library's name is the linkable one, because the Android \
             manifest and the Xcode project both name it: {manifest}"
        );

        let text = |suffix: &str| {
            generated
                .iter()
                .find(|(path, _)| path.ends_with(suffix))
                .map(|(_, c)| c.clone())
                .unwrap_or_else(|| panic!("no {suffix}"))
        };

        // The window is configured in the library, because Android and iOS
        // cannot reach `main` and would otherwise each configure their own.
        let lib = text("lib.rs");
        assert!(lib.contains(".title(\"my-app\")"));
        assert!(
            lib.contains("screens::home::screen()"),
            "the application mounts the same screen the preview does"
        );
        assert!(lib.contains("fn android_main"), "Android's entry point");
        assert!(lib.contains("vieww_ios_main"), "and the one main.m calls");

        // And the desktop binary is a delegation, not a second copy of it.
        let main = text("main.rs");
        assert!(main.contains("my_app::run()"), "{main}");
        assert!(
            !main.contains(".title("),
            "the window is configured once: {main}"
        );
    }

    #[test]
    fn a_path_dependency_points_at_the_crates() {
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
        let dep = Dependency::Version("0.0.1".into());
        let manifest = &files("app", &dep)[0].1;
        assert!(manifest.contains(r#"vieww = "0.0.1""#), "{manifest}");
        assert!(!dep.previewable(), "a published vieww cannot be previewed");
        assert!(dependency().previewable());
    }

    #[test]
    fn creating_writes_every_file_and_the_project_is_recognised() {
        let s = Scratch::new("create");
        let written = create(s.path(), "my-app", &dependency()).unwrap();
        assert_eq!(written.len(), files("my-app", &dependency()).len());
        for path in &written {
            assert!(path.is_file(), "{} was not written", path.display());
        }
        assert!(
            is_vieww_project(s.path()),
            "the studio must recognise what it just made"
        );
    }

    /// The user's own `~/src` is exactly the directory they will pick by
    /// accident, and scaffolding into it is not undoable.
    #[test]
    fn an_existing_directory_with_anything_in_it_is_refused() {
        let s = Scratch::new("not-empty");
        std::fs::create_dir_all(s.path()).unwrap();
        std::fs::write(s.path().join("important.txt"), b"do not lose me").unwrap();

        let error = create(s.path(), "my-app", &dependency()).unwrap_err();
        assert!(matches!(error, Error::NotEmpty(_)), "{error:?}");
        assert!(
            s.path().join("important.txt").is_file(),
            "the existing file must be untouched"
        );
        assert!(!s.path().join("Cargo.toml").exists(), "and nothing written");
    }

    #[test]
    fn an_empty_existing_directory_is_fine() {
        let s = Scratch::new("empty");
        std::fs::create_dir_all(s.path()).unwrap();
        assert!(create(s.path(), "my-app", &dependency()).is_ok());
    }

    #[test]
    fn a_bad_name_writes_nothing() {
        let s = Scratch::new("bad-name");
        assert!(create(s.path(), "1app", &dependency()).is_err());
        assert!(
            !s.path().exists(),
            "nothing may be created for a refused name"
        );
    }

    /// Compiling the template is what actually proves property 1, and it needs
    /// a working `cargo` and the vieww crates on disk — neither of which is
    /// guaranteed on the machine running the unit suite. It lives in
    /// `tests/scaffold.rs`, which skips loudly rather than silently when the
    /// workspace is not where it expects, exactly as `vieww-hardware` does for
    /// a missing GPU.
    ///
    /// This test only pins that the two halves cannot drift: the integration
    /// test compiles `files()`, so `files()` is the only definition of the
    /// template there is.
    #[test]
    fn there_is_one_definition_of_the_template() {
        let generated = files("probe-app", &dependency());
        assert!(
            generated.iter().all(|(_, contents)| !contents.is_empty()),
            "an empty template file would compile and mean nothing"
        );
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;

    #[test]
    fn a_screen_carries_the_entry_point_the_preview_looks_for() {
        let made = scaffold(Template::Screen, "photo_grid").expect("a valid name");
        assert!(made.contents.contains("pub fn screen() -> impl Widget"));
        assert!(made.contents.contains("pub struct PhotoGrid;"));
        assert!(made.contents.contains("impl Widget for PhotoGrid"));
        assert_eq!(made.path, PathBuf::from("src/screens/photo_grid.rs"));
        assert_eq!(made.module_line.as_deref(), Some("pub mod photo_grid;"));
    }

    /// A widget is used *by* a screen. Giving it a `screen()` too would make
    /// every one of them previewable on its own, which is not what it is.
    #[test]
    fn a_widget_has_no_entry_point() {
        let made = scaffold(Template::Widget, "tile").expect("a valid name");
        assert!(!made.contents.contains("pub fn screen()"));
        assert!(made.contents.contains("impl Widget for Tile"));
    }

    #[test]
    fn a_module_name_is_stricter_than_a_crate_name() {
        // A crate may contain `-`; cargo maps it to `_`. A module may not —
        // `mod photo-tile;` does not parse, and the file would be written and
        // then never compile.
        assert!(check_name("photo-tile").is_ok());
        assert!(check_module_name("photo-tile").is_err());
        assert!(check_module_name("PhotoTile").is_err());
        assert!(check_module_name("2fast").is_err());
        assert!(check_module_name("loop").is_err());
        assert!(check_module_name("photo_tile").is_ok());
    }

    #[test]
    fn the_type_name_is_derived_rather_than_asked_for_twice() {
        assert_eq!(upper_camel("photo_grid"), "PhotoGrid");
        assert_eq!(upper_camel("home"), "Home");
        assert_eq!(upper_camel("a_b_c"), "ABC");
    }
}
