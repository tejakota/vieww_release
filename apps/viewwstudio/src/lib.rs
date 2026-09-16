//! vieww Studio — an editor and a device-framed preview, built with vieww.
//!
//! A library rather than a binary, so the whole shell can be built, laid out
//! and rasterised in a test without opening a window. `main.rs` is thirty lines
//! of "find a toolchain, open a window, hand it a [`Shell`]".
//!
//! # Where the milestones stand
//!
//! `viewwstudioplan.md`'s M0 through M8 are built, with one deliberate
//! departure that is worth stating here rather than leaving to be discovered:
//!
//! **M2 and M4 do not use an external texture.** The plan called for the
//! preview to be a second, headless `RenderTree`/`FrameDriver` rendered to a
//! `wgpu` texture and blitted by a new leaf render object, with pointer events
//! remapped into the embedded tree's coordinate space. What is built instead
//! mounts the loaded screen as an ordinary subtree of the host tree, inside
//! `Inherited<ViewMetrics>` and a `Theme` carrying the simulated
//! [`Platform`]'s `TargetPlatform` — see [`ui::preview`]. It reaches the same
//! end (a screen laid out at device metrics, under device platform rules,
//! inside a device frame) with no texture round-trip, and it gets pointer
//! interactivity for free rather than by writing a coordinate remap. The cost
//! is stated in [`loaded`]: a panic on a *later* rebuild of the guest is not
//! caught, because there is no second `FrameDriver` to throw away.
//!
//! # Shape
//!
//! - [`state::Studio`] — every signal, held above the tree. No widget here owns
//!   state.
//! - [`command`] — every action, named once, so the menus, the palette, the
//!   shortcuts and the toolbar cannot disagree.
//! - [`ui`] — one module per region, each reading the signals it needs.
//! - [`compile`] and [`loaded`] — the buffer through `rustc` and back as a
//!   widget.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use vieww_platform_winit::App;
//! use viewwstudio::{Shell, Studio};
//!
//! App::new().title("vieww Studio").run(|driver| {
//!     let runtime = driver.elements().runtime().clone();
//!     viewwstudio::install(driver);
//!     driver.set_root(Shell {
//!         studio: Studio::new(&runtime),
//!     });
//! })?;
//! # Ok(()) }
//! ```

pub mod about;
pub mod assets;
pub mod buffer;
pub mod builds;
pub mod caret;
pub mod cargo;
pub mod command;
pub mod compile;
pub mod customise;
pub mod docs;
pub mod edit_ops;
pub mod export;
pub mod file_tree;
pub mod find;
pub mod folding;
pub mod git;
pub mod harness;
pub mod highlight;
pub mod history;
pub mod install;
pub mod jobs;
pub mod json;
pub mod language;
pub mod lessons;
pub mod lint;
pub mod live;
pub mod livedoc;
pub mod loaded;
pub mod lsp;
pub mod picker;
pub mod png;
pub mod recovery;
pub mod scaffold;
pub mod sdk;
pub mod settings;
pub mod setup;
pub mod state;
pub mod task;
pub mod theme;
pub mod tokens;
pub mod toolchains;
pub mod ui;

pub use buffer::{Buffer, Workspace};
pub use command::{Chord, Command, Menu};
pub use file_tree::{FileTree, WatchEvent, Watcher};
pub use find::Query;
pub use history::History;
pub use state::{PanelTab, Platform, PreviewState, Studio, View};
pub use theme::StudioTheme;
pub use ui::{Shell, Shortcuts};

/// The window the studio opens at. Big enough to hold both panes at their
/// minimum widths plus the sidebar, which is the smallest useful shell.
pub const WINDOW: vieww_foundation::Size = vieww_foundation::Size {
    width: 1440.0,
    height: 900.0,
};

/// Teach a driver about the studio's own render objects.
///
/// # Why this is a free function rather than something `Shell` does
///
/// A widget cannot register a render object: registration is a property of the
/// *driver*, and a widget never sees one — `FrameDriver::register` is reachable
/// only from where the application stands, which is `main` and a test harness.
/// Both call this, so neither has to remember it separately, and a shell built
/// without it fails loudly at mount rather than quietly losing its shortcuts.
pub fn install(driver: &mut vieww_render::FrameDriver) {
    driver.register::<Shortcuts, ui::shortcuts::RenderShortcuts>(|widget| {
        ui::shortcuts::RenderShortcuts::for_studio(widget.studio.clone())
    });

    // **The panic policy, set on purpose rather than inherited.**
    //
    // `ErrorPolicy` defaults to `Placeholder` in a debug build and `Propagate`
    // in a release one, and that default is right for an *application*: a
    // magenta box on a stranger's phone is worth less than the crash report it
    // hides. A studio is the other case in every respect. The code that panics
    // is the user's own, it is the thing they are sitting there editing, and
    // the build they will actually run is the release one — which, on the
    // default, takes the whole window and the unsaved buffer down with it.
    //
    // `Custom` rather than `Placeholder` so the substitute says what happened.
    // The built-in is a 48-point magenta square, which is exactly right for
    // somebody debugging the framework and unreadable to somebody debugging
    // their screen.
    //
    // The error itself reaches the Problems panel through
    // `Studio::drain_build_errors`; a caught panic nobody reports is a screen
    // that silently shows a coloured box.
    driver
        .elements()
        .set_error_policy(vieww_element::ErrorPolicy::Custom(panicked_widget));
}

/// What is mounted where a widget's `build` panicked.
///
/// Deliberately small and deliberately not expanding: a placeholder that filled
/// its space would be laid out under whatever constraints the broken widget had,
/// and inside a scrollable that is infinity — which turns one caught panic into
/// a layout failure. `ErrorPlaceholder`'s own docs make the argument; this
/// keeps the size and changes the message.
///
/// No text, for the reason that type gives as well: drawing a string needs a
/// font store, and a font store with no faces loads the system's — 33 seconds
/// in a debug build, in the exact situation this exists to make survivable.
/// The message travels to the Problems panel instead.
fn panicked_widget(_error: &vieww_element::BuildError) -> vieww_widget::WidgetNode {
    use vieww_widget::{Container, SizedBox};
    Container::new()
        .color(vieww_foundation::Color::rgba(0xC0, 0x33, 0x4A, 0xB0))
        .radius(4.0)
        .child(SizedBox::from_size(vieww_foundation::Size::new(48.0, 48.0)))
        .into()
}

/// Put focus somewhere, if it is nowhere.
///
/// Keys in this framework bubble outward **from whatever has focus**, so a
/// window nobody has clicked has nothing to bubble from and no shortcut would
/// ever arrive. Pointing focus at the outermost focusable object — which is the
/// shortcut layer, since it wraps everything — is what makes ⌘P work before the
/// first click. See [`ui::shortcuts`].
///
/// Called every frame from the platform hook. A no-op on all but the first, and
/// on any frame after a rebuild removed whatever had focus, which is exactly
/// when the shortcuts would otherwise go quiet.
pub fn seed_focus(driver: &mut vieww_render::FrameDriver) {
    if driver.focused().is_none() {
        driver.focus_next(true);
    }
}
