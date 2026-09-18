//! Development-time hot reload for vieww.
//!
//! Splits a development build in two: a **host** owning the event loop, the GPU
//! and the element tree, and a **guest** `cdylib` holding the application's
//! widgets. When the guest is rebuilt, the host loads the new one and swaps the
//! root — and the element tree, with every signal, scroll offset and
//! half-finished animation in it, stays where it was.
//!
//! The design, the three approaches weighed, and the one blocker that had to be
//! cleared first are in `docs/HOT-RELOAD.md`. This is the implementation of the
//! approach that document chose.
//!
//! # The whole loop
//!
//! ```console
//! # one terminal: rebuild the guest whenever a file changes
//! cargo watch -w app/src -x 'build -p app'
//!
//! # the other: run the host, which notices and reloads
//! cargo run -p host
//! ```
//!
//! # Development only, and not for the reason people assume
//!
//! Not because it is unstable. Because **every reload leaks a library**, on
//! purpose and unavoidably — see [`Guest`]. A long session leaks steadily, which
//! is fine at a desk and is never something to ship.
//!
//! # Where it works, which is a property of the platform rather than of this
//! crate
//!
//! - **Desktop** — yes, and it is what `examples/reload-host` demonstrates.
//! - **Android** — yes, measured. A real guest `cdylib` was loaded *and
//!   executed* on a Redmi Note 7 Pro at `target_sdk_version = 34`, in a
//!   non-debuggable release build. The library has to be staged into the app's
//!   private directory first, because `adb` can only write somewhere that is
//!   generally `noexec` — see [`Watch::stage_in`] and `examples/reload-android`.
//! - **iOS device** — no, and no amount of staging changes it. Every executable
//!   page must be backed by a signed binary from the app bundle, so `dlopen` of
//!   a library delivered at runtime is refused. The same policy that withholds
//!   JIT.
//! - **iOS Simulator** — plausible, untried. A simulated app is a macOS process,
//!   so the desktop rules apply; [`Reloader::staged_in`] is already the right
//!   shape for it. Nobody here has a Mac to find out — see `docs/ROADMAP.md`.
//!
//! An earlier revision of this paragraph said hot reload "does not work on iOS
//! at all", which would talk a reader out of the one iOS path that could work.
//! `docs/HOT-RELOAD.md` had it right the whole time.
//!
//! # What survives a reload, and what does not
//!
//! | | |
//! |---|---|
//! | element state, signals, scroll offsets, animations | **survive** |
//! | anything held by the host — GPU, fonts, the window | never reloaded |
//! | a state type whose *shape* changed | detected, and the tree is rebuilt instead — see [`Verdict`] |
//! | a state type reordered at the same size | **not** detected; the documented sharp edge |
//!
//! Survival depends on the `hot-reload` feature of `vieww-widget` being on, which
//! is what makes reconciliation identify widgets by type path rather than by
//! `TypeId`. Without it a reload still works and simply keeps nothing, because
//! every `TypeId` changed — a slow restart wearing a reload's name.

mod fingerprint;
mod guest;
mod watch;

pub use fingerprint::{Fingerprint, Verdict};
pub use guest::{Guest, LoadError, FINGERPRINT_SYMBOL, ROOT_SYMBOL};
pub use watch::{Change, Watch};

// Re-exported so `guest!` can name it without the application depending on
// `vieww-widget` under that name.
pub use vieww_widget::WidgetNode;

use std::path::PathBuf;
use std::time::Duration;

/// A guest library, watched and reloaded.
///
/// The whole of the host side: hold one, call [`poll`](Self::poll) when
/// convenient, and act on what it returns.
///
/// ```no_run
/// # use vieww_reload::{Reloader, Reloaded};
/// # fn example(driver: &mut impl FnMut(vieww_reload::WidgetNode)) {
/// let mut reloader = Reloader::new("target/debug/libapp.so").expect("the guest builds first");
/// # let mut set_root = driver;
/// // ... each frame, or on a timer:
/// match reloader.poll() {
///     Reloaded::Nothing => {}
///     Reloaded::Root(root) => set_root(root),
///     Reloaded::Restart(root) => set_root(root), // and drop the tree first
///     Reloaded::Failed(error) => eprintln!("reload failed, still running the old build: {error}"),
/// }
/// # }
/// ```
#[derive(Debug)]
pub struct Reloader {
    watch: Watch,
    guest: Guest,
}

/// What a [`Reloader::poll`] produced.
#[derive(Debug)]
pub enum Reloaded {
    /// The guest has not been rebuilt.
    Nothing,
    /// A new build, and its state is compatible: swap the root and keep the
    /// tree.
    Root(WidgetNode),
    /// A new build whose state changed shape. Drop the tree, then mount this.
    ///
    /// Separate from [`Root`](Self::Root) because the host has to do something
    /// *different*, and folding them together would make the dangerous case the
    /// silent one.
    Restart(WidgetNode),
    /// The new build could not be loaded. **The old one is still running.**
    ///
    /// A compile error in the guest leaves the library missing or stale, and
    /// carrying on with what is already loaded is the only useful answer — the
    /// developer is mid-edit and about to fix it.
    Failed(LoadError),
}

impl Reloader {
    /// Load `path` now, and watch it from here.
    ///
    /// Fails only if the *first* load fails: after that a broken build is
    /// reported through [`Reloaded::Failed`] and the running one is kept.
    ///
    /// # On Windows the first load is a copy too
    ///
    /// Windows locks a loaded DLL against deletion, and cargo's next build of
    /// the guest starts by removing the old one — so opening the path cargo
    /// writes made every rebuild after the first fail with `failed to remove
    /// file ... reload_guest.dll: Access is denied`, and no reload could ever
    /// happen. Loading a copy beside it leaves cargo's file free, exactly as
    /// every later reload already does.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, LoadError> {
        let path = path.into();
        let watch = Watch::new(path);
        let guest = if cfg!(windows) {
            let staged = watch
                .stage()
                .map_err(|error| LoadError::Stage(watch.staged_path(), error))?;
            Guest::load(&staged)?
        } else {
            Guest::load(watch.path())?
        };
        Ok(Self { watch, guest })
    }

    /// Load `path`, but copy it into `dir` first — **including the first load**.
    ///
    /// # When the watched path cannot be loaded from at all
    ///
    /// [`new`](Self::new) opens the watched library directly, which is right
    /// wherever one directory is both writable and executable. On Android it is
    /// not: the rebuilt library arrives over `adb`, which can only write the
    /// app's external directory or `/data/local/tmp`, and neither can be
    /// `dlopen`ed — external storage is a FUSE mount and generally `noexec`.
    /// The load has to happen from `internal_data_path()`, which `adb` cannot
    /// write.
    ///
    /// So the first load is staged too, and that is the whole difference. A
    /// constructor rather than a builder because the copy has to happen *before*
    /// the load that `new` already performs, and a method called afterwards
    /// would be too late to matter.
    pub fn staged_in(path: impl Into<PathBuf>, dir: impl Into<PathBuf>) -> Result<Self, LoadError> {
        let path = path.into();
        let watch = Watch::new(path).stage_in(dir);
        let staged = watch
            .stage()
            .map_err(|error| LoadError::Stage(watch.staged_path(), error))?;
        let guest = Guest::load(&staged)?;
        Ok(Self { watch, guest })
    }

    /// The root of the currently loaded build.
    ///
    /// # Errors
    ///
    /// [`LoadError::Panicked`] if the guest panicked while building. See
    /// [`Guest::root`].
    pub fn root(&self) -> Result<WidgetNode, LoadError> {
        self.guest.root()
    }

    /// How many reloads have been accepted.
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.watch.generation()
    }

    /// Ask `wake` for a frame whenever the library changes.
    ///
    /// **Without this, a reload is invisible until something else asks for a
    /// frame.** The swap lands in a `before_frame` hook, and an idle window
    /// draws nothing — so the developer rebuilds, sees no change, and moves the
    /// mouse to find out it worked all along. A hot reload loop that needs the
    /// mouse jiggled is not one anybody keeps using.
    ///
    /// Spawns a thread that watches the same path and calls `wake` on a change.
    /// It does no loading of its own — [`poll`](Self::poll) still does that, on
    /// the loop's thread, between frames. This only makes the frame *happen*.
    ///
    /// Takes a plain callback rather than a platform type so this crate stays
    /// independent of the platform layer: a `vieww-platform-winit` host passes
    /// `move || FrameWaker::wake(&waker)`.
    ///
    /// The thread runs until the process ends. It holds nothing but a path.
    pub fn wake_on_change<W>(&self, every: Duration, wake: W)
    where
        W: Fn() + Send + 'static,
    {
        // Its own watcher, deliberately: this one only decides *when to ask for
        // a frame*, and `poll` decides what to do about it. Sharing one across
        // the two threads would need a lock on the hot path to save a `stat`
        // every few hundred milliseconds.
        let mut watch = Watch::new(self.watch.path().to_path_buf());
        std::thread::Builder::new()
            .name("vieww-reload-watch".to_owned())
            .spawn(move || loop {
                std::thread::sleep(every);
                if watch.poll() == Change::Rebuilt {
                    wake();
                }
            })
            .expect("a watcher thread");
    }

    /// Look for a rebuild, and load it if there is one.
    ///
    /// Cheap when nothing changed — one `stat`. Call it per frame, or on a
    /// timer; the cadence belongs to the host rather than to a thread in here,
    /// so a reload always lands between frames rather than during one.
    pub fn poll(&mut self) -> Reloaded {
        match self.watch.poll() {
            Change::Unchanged | Change::Missing => return Reloaded::Nothing,
            Change::Rebuilt => {}
        }

        // Copied before opening: cargo writes the library in place, so opening
        // the path it is still linking gets a half-written file.
        let staged = match self.watch.stage() {
            Ok(staged) => staged,
            // Mid-write. The next poll sees the same timestamp and tries again,
            // because `generation` already moved past it.
            Err(_) => return Reloaded::Nothing,
        };

        let incoming = match Guest::load(&staged) {
            Ok(guest) => guest,
            Err(error) => return Reloaded::Failed(error),
        };

        let verdict = Verdict::between(self.guest.fingerprint(), incoming.fingerprint());
        // Built *before* the swap, so a guest that panics while building leaves
        // the previous one loaded and running — which is now true rather than
        // merely intended: until the unwind guard went into `guest!`, the panic
        // crossed an `extern "C"` boundary and aborted the process before this
        // line could matter. See `Guest::root`.
        let root = match incoming.root() {
            Ok(root) => root,
            Err(error) => return Reloaded::Failed(error),
        };
        self.guest = incoming;

        match verdict {
            Verdict::Reload => Reloaded::Root(root),
            Verdict::Restart => Reloaded::Restart(root),
        }
    }
}
