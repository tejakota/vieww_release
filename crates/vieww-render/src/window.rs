//! A window, as a capability an application asks for rather than a thing only
//! `main` can make.
//!
//! # Why this is a service and not a method on `App`
//!
//! A second window is opened by a **menu item**, in a tree that has no idea what
//! `App` is and cannot be handed one — the platform crate sits above the widget
//! crate, not below it. So opening a window arrives the way the camera and the
//! clipboard arrive: through
//! [`Services`](vieww_foundation::service::Services), looked up by its own
//! trait, with the platform providing the implementation and the framework
//! providing only the shape.
//!
//! `docs/AIMS.md` §A states the test this is written against — *nothing in the
//! platform list needs a framework change to add* — and windows are the first
//! capability where vieww itself is the third party. If opening a window had
//! needed a new field somewhere, the seam would have been the wrong shape.
//!
//! # Nothing here names a `winit` type, and that is load-bearing
//!
//! The desktop a Rust UI framework actually meets is **Wayland**, where the
//! interesting window vocabulary is not "a window" but `xdg_toplevel` with a
//! parent, `xdg_popup` with a real grab, and `wlr-layer-shell` for a panel.
//! `winit` 0.30 has none of it: one flat kind of window, and parenting only on
//! some backends. A seam shaped like `winit`'s capabilities would therefore have
//! to be **broken** the day a Wayland-native backend implements it — and there
//! is now a whole Rust desktop worth implementing it for, since System76's
//! COSMIC (Rust, Wayland-native, `cosmic-comp` on Smithay) shipped 1.0 with
//! Pop!_OS 24.04 LTS on 2025-12-11.
//!
//! So [`WindowSpec`] says what an application wants and stops, and every field
//! it does not yet carry — a role, a parent, an anchor — is a field it can gain
//! without any caller changing. Nothing in this module refers to a window
//! manager, a compositor, or a handle.
//!
//! # A request, not a window
//!
//! Every implementation here **queues**. `winit` can only create a window from
//! inside an event-loop callback, holding an `ActiveEventLoop` that exists for
//! the duration of that callback and cannot be stored — so a handler running
//! mid-frame has nowhere to create one from even in principle. The request is
//! recorded, the loop drains it on its next pass, and the window appears one
//! iteration later.
//!
//! This is the same shape [`DeepLinks::take_pending`](vieww_foundation::DeepLinks::take_pending)
//! has and for a related reason: what the platform will do and when it will do
//! it are the platform's business, and pretending otherwise puts a lie in the
//! signature.
//!
//! # What one window shares with the next
//!
//! Everything the application is about, and nothing the surface owns. A window
//! is a [`FrameDriver`], which already holds exactly the per-surface things —
//! element tree, render tree, layers, scene, pointers, focus, tickers — and a
//! [`Runtime`] is an `Rc` to one reactive graph that every driver shares. A
//! document open in two windows is one document, and that is asserted rather
//! than asserted-to:
//! `two_windows_share_one_reactive_graph_and_nothing_else` in `frame.rs`.
//!
//! For contrast, and dated because upstream facts move: the shipping way to get
//! a second window in most toolkits as of 2026 is a second engine — starting
//! an isolate with its own heap, so the two windows
//! exchange state over IPC and every plugin is registered twice. The shape
//! below — one engine, windows sharing one data model — is where the
//! industry is heading. The difference worth
//! keeping is not that vieww got somewhere first; it is that here there is no
//! second engine for a second copy of the state to live in.

use std::cell::{Cell, RefCell};
use std::fmt;

use vieww_element::{Runtime, Signal};
use vieww_foundation::{Color, ServiceError, Size};

use crate::FrameDriver;

/// Which window.
///
/// Minted by whoever implements [`Windows`], and opaque on purpose: it is a
/// name for a window in this process and nothing about the platform's own
/// handle should leak through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowKey(u64);

impl WindowKey {
    /// The window an application starts with.
    ///
    /// Fixed rather than minted, so a tree can name it without having been told
    /// — which is what makes "bring the main window forward" writable in a
    /// widget that was built inside a different one.
    pub const PRIMARY: Self = Self(0);

    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn is_primary(self) -> bool {
        self.0 == Self::PRIMARY.0
    }
}

impl fmt::Display for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_primary() {
            f.write_str("window#primary")
        } else {
            write!(f, "window#{}", self.0)
        }
    }
}

/// What a window is *for*, which is not the same as what the compositor calls
/// it.
///
/// # Why this describes behaviour rather than a platform object
///
/// A desktop application is cross-OS, and all three desktops have the idea of a
/// window that belongs to another one — `GWL_HWNDPARENT` on Windows, an
/// `NSWindow` child or sheet on macOS, `xdg_toplevel.set_parent` on Wayland,
/// `WM_TRANSIENT_FOR` on X11 — and every one of them spells it differently.
/// `winit` 0.30 exposes almost none of it portably: `with_parent_window` is
/// `unsafe` and honoured on Windows and X11 only, and there is no modal at all.
///
/// So a role here is **a promise about behaviour that vieww keeps itself**.
/// Closing a parent closes its dialogs, a parent with a modal child takes no
/// input, and focus returns to the parent when its dialog goes — enforced by
/// [`WindowSet`] on every platform, with no per-OS code, because one process
/// owns both windows' trees. A backend that can *also* tell the window manager
/// makes the OS's own alt-tab and window shading agree with that, which is
/// polish on top rather than the thing that makes it true.
///
/// **This is the difference worth having.** Designs that hand an application
/// a window and leave modality to it, per platform, share the same gap.
///
/// # What is deliberately not here
///
/// `Popup` and `Layer` — a menu with a real compositor grab, and a panel with an
/// exclusive zone. Both need an object no backend here can create yet, and
/// layer-shell is Linux-only, which is exactly the wrong thing to bake into a
/// cross-OS vocabulary. A variant that silently means nothing is the concession
/// `docs/AIMS.md` refuses; these arrive with a backend that can honour them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WindowRole {
    /// A window in its own right. Closes alone, takes input whenever it has
    /// focus, appears in the task switcher.
    #[default]
    Regular,
    /// A window that belongs to another one.
    ///
    /// Closes when its parent does, returns focus to its parent, and — while it
    /// is open — makes its parent inert: no pointers, no keys, and nothing for a
    /// screen reader to walk into. That last part is the half every other
    /// framework leaves to the application, and the half that is invisible until
    /// somebody navigates the window behind a modal with a screen reader.
    Dialog {
        /// The window this one belongs to.
        parent: WindowKey,
    },
}

impl WindowRole {
    /// The window this one belongs to, if any.
    #[must_use]
    pub const fn parent(self) -> Option<WindowKey> {
        match self {
            Self::Regular => None,
            Self::Dialog { parent } => Some(parent),
        }
    }
}

/// What a window should look like when it appears.
///
/// The same three things `App` itself takes plus its [`WindowRole`], and
/// deliberately no more: a spec that can express less than the primary window
/// can would make the second window a lesser kind of window, which is the
/// concession `docs/AIMS.md` §B exists to refuse.
///
/// Built with methods rather than public fields so that the anchoring and
/// exclusive zones a Wayland backend can express — see [`WindowRole`] — arrive
/// as new methods rather than as a breaking change to every construction site.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    title: String,
    size: Size,
    background: Color,
    role: WindowRole,
}

impl WindowSpec {
    /// An 800x600 window on white.
    ///
    /// The same defaults `App::new` uses, so that
    /// `WindowSpec::new("Inspector")` and a plain `App` differ only in title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            size: Size::new(800.0, 600.0),
            background: Color::WHITE,
            role: WindowRole::Regular,
        }
    }

    #[must_use]
    pub const fn with_size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn with_background(mut self, background: Color) -> Self {
        self.background = background;
        self
    }

    /// Make this a dialog belonging to `parent`.
    ///
    /// See [`WindowRole::Dialog`] for what the framework then guarantees, on
    /// every desktop, whatever the window manager was willing to be told.
    #[must_use]
    pub const fn as_dialog(mut self, parent: WindowKey) -> Self {
        self.role = WindowRole::Dialog { parent };
        self
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    #[must_use]
    pub const fn background(&self) -> Color {
        self.background
    }

    #[must_use]
    pub const fn role(&self) -> WindowRole {
        self.role
    }
}

/// Which windows are open, what each belongs to, and what follows from that.
///
/// **The policy, kept out of the event loop on purpose.** Every rule a role
/// implies — what closes with what, which window is inert, where focus goes — is
/// bookkeeping over keys, so it belongs somewhere a test can reach without a
/// compositor. The platform's loop applies the answers; it does not decide them.
///
/// That split is the same one `next_action` is written for, and for the same
/// reason: a rule that only exists inside an event-loop callback is a rule
/// nothing can test.
#[derive(Debug, Clone, Default)]
pub struct WindowSet {
    /// In the order they opened, which is the order a window menu lists them
    /// and the order [`keys`](Self::keys) reports.
    entries: Vec<(WindowKey, WindowRole)>,
}

impl WindowSet {
    /// No windows at all.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Just the primary window, as an application starts.
    #[must_use]
    pub fn with_primary() -> Self {
        let mut set = Self::new();
        set.insert(WindowKey::PRIMARY, WindowRole::Regular);
        set
    }

    /// Record a window. Re-inserting a key replaces its role.
    pub fn insert(&mut self, key: WindowKey, role: WindowRole) {
        match self.entries.iter_mut().find(|(open, _)| *open == key) {
            Some(entry) => entry.1 = role,
            None => self.entries.push((key, role)),
        }
    }

    /// Remove `key` **and everything that belongs to it**, innermost first.
    ///
    /// Transitive, because a dialog can open a dialog: closing the document
    /// window has to take the settings sheet and the confirmation on top of it,
    /// and leaving either behind is an orphan window an application cannot close
    /// and a user cannot explain.
    ///
    /// Innermost first so a caller destroying surfaces in the returned order
    /// never destroys a parent while a child still points at it.
    pub fn remove(&mut self, key: WindowKey) -> Vec<WindowKey> {
        let mut doomed = vec![key];
        let mut index = 0;
        while index < doomed.len() {
            let parent = doomed[index];
            for (open, role) in &self.entries {
                if role.parent() == Some(parent) && !doomed.contains(open) {
                    doomed.push(*open);
                }
            }
            index += 1;
        }
        self.entries.retain(|(open, _)| !doomed.contains(open));
        doomed.reverse();
        doomed
    }

    /// `true` if `key` is open.
    #[must_use]
    pub fn contains(&self, key: WindowKey) -> bool {
        self.entries.iter().any(|(open, _)| *open == key)
    }

    /// Every open window, in the order they opened.
    #[must_use]
    pub fn keys(&self) -> Vec<WindowKey> {
        self.entries.iter().map(|(key, _)| *key).collect()
    }

    /// What `key` belongs to, if anything.
    #[must_use]
    pub fn parent(&self, key: WindowKey) -> Option<WindowKey> {
        self.entries
            .iter()
            .find(|(open, _)| *open == key)
            .and_then(|(_, role)| role.parent())
    }

    /// `true` when `key` has a dialog open on it and must therefore take no
    /// input and offer nothing to a screen reader.
    ///
    /// The claim that makes [`WindowRole::Dialog`] worth having. It is answered
    /// here rather than by the window itself because a window cannot see its own
    /// children, and answered *live* rather than as a flag because a flag is
    /// what goes stale when a dialog closes during a drag.
    #[must_use]
    pub fn is_inert(&self, key: WindowKey) -> bool {
        self.entries
            .iter()
            .any(|(_, role)| role.parent() == Some(key))
    }

    /// Where focus should go when `key` closes.
    ///
    /// Its parent, if it had one and that parent is still open. `None` for a
    /// regular window, because the platform decides which of several unrelated
    /// windows comes forward and guessing would fight it.
    #[must_use]
    pub fn focus_after_closing(&self, key: WindowKey) -> Option<WindowKey> {
        let parent = self.parent(key)?;
        self.contains(parent).then_some(parent)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// How a new window's tree is built.
///
/// Handed the window's own [`WindowKey`] as well as its driver, because the
/// alternative is a tree that cannot close the window it is in: the key does not
/// exist until [`Windows::open_boxed`] returns, and by then the closure has
/// already been built and moved. One parameter removes the ordering problem
/// entirely.
///
/// `FnOnce`, matching `App::run`'s own builder: a tree is mounted once, and
/// everything after that is the element tree's business.
pub type WindowBuild = Box<dyn FnOnce(WindowKey, &mut FrameDriver)>;

/// Opening and closing windows, and knowing which are open.
///
/// # Errors, and what they mean here
///
/// [`ServiceError::Unsupported`] is the honest answer on a platform where an
/// application does not own its windows — Android and iOS, where the activity
/// or the scene is the window and a second one is not something a UI framework
/// may conjure. An application that hides its "New Window" item when this
/// service is absent behaves correctly on a phone with no code of its own.
pub trait Windows: 'static {
    /// Ask for a window, and get the name it will have.
    ///
    /// The key is minted **now** and the window appears later, which is what
    /// lets a caller keep the key without waiting: the request has been
    /// accepted, and the only thing outstanding is the platform's own work.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where an application cannot open windows,
    /// and [`ServiceError::Failed`] if the request could not be recorded.
    fn open_boxed(&self, spec: WindowSpec, build: WindowBuild) -> Result<WindowKey, ServiceError>;

    /// Ask for a window to close.
    ///
    /// Closing the last one ends the application, exactly as closing the only
    /// window always has. Closing a key that is not open is **not** an error:
    /// two paths racing to close the same window is ordinary, and the second one
    /// has nothing to fix.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where an application cannot close windows.
    fn close(&self, key: WindowKey) -> Result<(), ServiceError>;

    /// Which windows are on screen, as state the tree can read.
    ///
    /// **A [`Signal`], not a `Vec`, and that is the whole point.** Reading it
    /// during a build subscribes that element, so a window menu, a "close all"
    /// item, or a button that must not open the inspector twice rebuild when a
    /// window opens or closes — without the application keeping a second copy of
    /// the truth and remembering to update it.
    ///
    /// The second copy is what an application has to maintain against every
    /// other multi-window API, and it is wrong in exactly the cases nobody
    /// tests: the window the *user* closed, the window that failed to open, the
    /// window closed while a dialog was up.
    ///
    /// Always contains [`WindowKey::PRIMARY`] until the primary window closes.
    /// Ordering is the order they opened.
    fn open_windows(&self) -> Signal<Vec<WindowKey>>;
}

/// [`Windows::open_boxed`] without the box.
///
/// A separate trait rather than a method with `where Self: Sized`, because the
/// service is reached as `Rc<dyn Windows>` and a `Sized` method is exactly what
/// such a handle cannot call. Blanket-implemented for every `Windows`, `?Sized`
/// included, so the ergonomic form is the one that works on the handle callers
/// actually hold.
pub trait WindowsExt {
    /// Ask for a window, passing the builder directly.
    ///
    /// # Errors
    ///
    /// As [`Windows::open_boxed`].
    fn open(
        &self,
        spec: WindowSpec,
        build: impl FnOnce(WindowKey, &mut FrameDriver) + 'static,
    ) -> Result<WindowKey, ServiceError>;

    /// Whether `key` is on screen right now.
    ///
    /// Subscribes the caller, like any other read of
    /// [`open_windows`](Windows::open_windows).
    fn is_open(&self, key: WindowKey) -> bool;
}

impl<W: Windows + ?Sized> WindowsExt for W {
    fn open(
        &self,
        spec: WindowSpec,
        build: impl FnOnce(WindowKey, &mut FrameDriver) + 'static,
    ) -> Result<WindowKey, ServiceError> {
        self.open_boxed(spec, Box::new(build))
    }

    fn is_open(&self, key: WindowKey) -> bool {
        self.open_windows().with(|open| open.contains(&key))
    }
}

/// [`Windows`] that opens nothing and remembers everything.
///
/// A real implementation rather than a panicking stub, for the reason
/// [`MemoryStorage`](vieww_foundation::MemoryStorage) is one: an application
/// running against this behaves correctly right up to the point where a window
/// would have appeared, and a test can then do the interesting half itself —
/// [`take_opened`](Self::take_opened) hands back the builders, and running one
/// against a [`FrameDriver`] mounts the second window's tree with no platform,
/// no compositor and no display.
///
/// That is the whole point of it: "the New Window button builds the right tree"
/// is a claim about the application, and it should not need a screen.
///
/// [`open_windows`](Windows::open_windows) is maintained **as though every
/// request succeeded**, which is what a test of an application's own menu wants
/// and is not a claim about any platform.
pub struct RecordingWindows {
    opened: RefCell<Vec<(WindowKey, WindowSpec, WindowBuild)>>,
    closed: RefCell<Vec<WindowKey>>,
    /// The same policy a real platform applies, so that a test written against
    /// this double is not written against different rules — closing a parent
    /// here takes its dialogs with it, exactly as it will on a desktop.
    set: RefCell<WindowSet>,
    open: Signal<Vec<WindowKey>>,
    /// The next key to hand out. Starts at 1: 0 is [`WindowKey::PRIMARY`],
    /// which nothing may be given.
    next: Cell<u64>,
}

impl fmt::Debug for RecordingWindows {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The builders are closures, which have nothing printable about them.
        f.debug_struct("RecordingWindows")
            .field("opened", &self.opened.borrow().len())
            .field("closed", &self.closed.borrow().len())
            .finish_non_exhaustive()
    }
}

impl RecordingWindows {
    /// Recording into `runtime`'s graph.
    ///
    /// The runtime matters: a signal minted from a *different* graph than the
    /// trees reading it marks nothing pending, so the menu would never rebuild
    /// and every test of it would pass. Hand it the same one the drivers share.
    #[must_use]
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            opened: RefCell::new(Vec::new()),
            closed: RefCell::new(Vec::new()),
            set: RefCell::new(WindowSet::with_primary()),
            open: runtime.signal(vec![WindowKey::PRIMARY]),
            next: Cell::new(1),
        }
    }

    /// Publish the set as the signal a tree reads.
    ///
    /// One write, from one place: the set is the truth and the signal is how the
    /// tree sees it, and two updates that could disagree is exactly the second
    /// copy this whole design exists to remove.
    fn publish(&self) {
        self.open.set(self.set.borrow().keys());
    }

    /// Every window asked for since the last call, in order, with its builder.
    pub fn take_opened(&self) -> Vec<(WindowKey, WindowSpec, WindowBuild)> {
        std::mem::take(&mut *self.opened.borrow_mut())
    }

    /// Every key asked to close, in order.
    #[must_use]
    pub fn closed(&self) -> Vec<WindowKey> {
        self.closed.borrow().clone()
    }
}

impl Windows for RecordingWindows {
    fn open_boxed(&self, spec: WindowSpec, build: WindowBuild) -> Result<WindowKey, ServiceError> {
        let key = WindowKey::new(self.next.get());
        self.next.set(key.raw() + 1);
        self.set.borrow_mut().insert(key, spec.role());
        self.opened.borrow_mut().push((key, spec, build));
        self.publish();
        Ok(key)
    }

    fn close(&self, key: WindowKey) -> Result<(), ServiceError> {
        // Everything that belonged to it, innermost first — the same answer a
        // platform gets from the same type.
        let closed = self.set.borrow_mut().remove(key);
        self.closed.borrow_mut().extend(closed);
        self.publish();
        Ok(())
    }

    fn open_windows(&self) -> Signal<Vec<WindowKey>> {
        self.open.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use vieww_foundation::service::Services;
    use vieww_widget::prelude::*;

    use super::*;

    fn recording() -> (Runtime, RecordingWindows) {
        let runtime = Runtime::new();
        let windows = RecordingWindows::new(&runtime);
        (runtime, windows)
    }

    #[test]
    fn a_window_is_reached_through_the_same_seam_as_the_camera_would_be() {
        // The claim `docs/AIMS.md` §A makes about every platform capability,
        // checked on the one where vieww is its own third party. If this ever
        // needs a change to `Services` to compile, windows have become special.
        let runtime = Runtime::new();
        let mut services = Services::new();
        services.provide::<dyn Windows>(Rc::new(RecordingWindows::new(&runtime)));

        let windows = services.get::<dyn Windows>().expect("registered");
        let key = windows
            .open(WindowSpec::new("Inspector"), |_, _| {})
            .expect("recorded");

        assert!(!key.is_primary(), "a new window is never the primary one");
    }

    #[test]
    fn the_builder_mounts_a_tree_with_no_platform_underneath_it() {
        // **What `RecordingWindows` is for.** The half of "New Window" that is
        // the application's — does the second window build the right thing —
        // needs no window at all, and this is the test an application writes.
        let (_runtime, windows) = recording();
        windows
            .open(WindowSpec::new("Inspector"), |_, driver| {
                driver.set_root(ColoredBox::new(Color::BLUE));
            })
            .expect("recorded");

        let mut opened = windows.take_opened();
        assert_eq!(opened.len(), 1);
        let (key, spec, build) = opened.remove(0);
        assert_eq!(spec.title(), "Inspector");

        let mut driver = FrameDriver::new(Size::new(100.0, 100.0));
        build(key, &mut driver);
        driver.draw_frame();

        assert!(
            driver.has_root(),
            "the builder ran against a real driver, and the tree it mounted is \
             the one the window would have shown"
        );
    }

    #[test]
    fn a_window_learns_its_own_key_on_the_way_in() {
        // The reason `WindowBuild` takes two parameters rather than one: a tree
        // that cannot name its own window cannot close it, and the key does not
        // exist until `open` has returned — by which time the closure is built.
        let (_runtime, windows) = recording();
        let told = Rc::new(Cell::new(None));
        let sink = Rc::clone(&told);
        let key = windows
            .open(WindowSpec::new("Inspector"), move |key, _| {
                sink.set(Some(key));
            })
            .expect("recorded");

        let mut opened = windows.take_opened();
        let (given, _, build) = opened.remove(0);
        let mut driver = FrameDriver::new(Size::new(10.0, 10.0));
        build(given, &mut driver);

        assert_eq!(told.get(), Some(key), "and it is the key the caller holds");
    }

    #[test]
    fn keys_are_unique_and_never_collide_with_the_primary() {
        let (_runtime, windows) = recording();
        let first = windows.open(WindowSpec::new("a"), |_, _| {}).expect("a");
        let second = windows.open(WindowSpec::new("b"), |_, _| {}).expect("b");

        assert_ne!(first, second);
        assert!(!first.is_primary() && !second.is_primary());
    }

    #[test]
    fn closing_is_recorded_rather_than_refused() {
        // Closing a window nothing opened is ordinary — two paths racing to
        // close the same one — and the second caller has nothing to fix.
        let (_runtime, windows) = recording();
        assert!(windows.close(WindowKey::new(41)).is_ok());
        assert_eq!(windows.closed(), vec![WindowKey::new(41)]);
    }

    #[test]
    fn which_windows_are_open_is_one_truth_rather_than_the_applications_copy() {
        // The state an application has to keep for itself against every other
        // multi-window API, and gets wrong in the cases nobody tests.
        let (_runtime, windows) = recording();
        assert_eq!(windows.open_windows().get(), vec![WindowKey::PRIMARY]);

        let inspector = windows
            .open(WindowSpec::new("Inspector"), |_, _| {})
            .expect("open");
        assert!(windows.is_open(inspector));
        assert!(windows.is_open(WindowKey::PRIMARY));

        windows.close(inspector).expect("close");
        assert!(!windows.is_open(inspector));
        assert_eq!(
            windows.open_windows().get(),
            vec![WindowKey::PRIMARY],
            "and the primary is untouched by another window closing"
        );
    }

    #[test]
    fn a_tree_that_reads_the_open_windows_rebuilds_when_one_opens() {
        // **The reason it is a signal.** A menu listing open windows is stale
        // for ever if this does not mark its reader pending — and stale in a way
        // that looks like a rendering bug rather than a state one.
        let runtime = Runtime::new();
        let windows = RecordingWindows::new(&runtime);
        let open = windows.open_windows();

        let mut driver = FrameDriver::with_runtime(Size::new(100.0, 100.0), runtime);
        driver.set_root(WindowCount { open: open.clone() });
        driver.draw_frame();
        assert_eq!(driver.elements().pending_count(), 0);

        windows
            .open(WindowSpec::new("Inspector"), |_, _| {})
            .expect("open");

        assert_eq!(
            driver.elements().pending_count(),
            1,
            "the element that read the set has to rebuild, or the menu shows \
             yesterday's windows"
        );
    }

    /// A tree whose shape depends on which windows are open.
    #[derive(Debug)]
    struct WindowCount {
        open: Signal<Vec<WindowKey>>,
    }

    impl Widget for WindowCount {
        fn debug_name(&self) -> &'static str {
            "WindowCount"
        }

        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }

        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            // A shape that differs by *whether* a second window is open, rather
            // than a size derived from the count: the claim under test is the
            // subscription, and a cast would only add a lint to argue with.
            let alone = self.open.with(|open| open.len() <= 1);
            SizedBox::square(if alone { 10.0 } else { 20.0 }).into()
        }
    }

    vieww_widget::widget_node_from!(WindowCount);

    #[test]
    fn the_double_closes_dialogs_with_their_parent_because_a_platform_will() {
        // A test double that applies different rules than the thing it stands
        // for is worse than no double: the application's own test passes and the
        // behaviour it asserted never happens.
        let (_runtime, windows) = recording();
        let sheet = windows
            .open(
                WindowSpec::new("Settings").as_dialog(WindowKey::PRIMARY),
                |_, _| {},
            )
            .expect("open");
        assert!(windows.is_open(sheet));

        windows.close(WindowKey::PRIMARY).expect("close");

        assert!(!windows.is_open(sheet), "the sheet went with its parent");
        assert_eq!(
            windows.closed(),
            vec![sheet, WindowKey::PRIMARY],
            "innermost first"
        );
    }

    #[test]
    fn a_spec_says_only_what_the_primary_window_can_say() {
        let spec = WindowSpec::new("Inspector")
            .with_size(Size::new(320.0, 640.0))
            .with_background(Color::BLACK);
        assert_eq!(spec.title(), "Inspector");
        assert_eq!(spec.size(), Size::new(320.0, 640.0));
        assert_eq!(spec.background(), Color::BLACK);
        assert_eq!(spec.role(), WindowRole::Regular, "unless it says otherwise");
    }

    // ---------------------------------------------------------- window set

    /// Three windows: the primary, a dialog on it, and a dialog on that.
    fn nested() -> (WindowSet, WindowKey, WindowKey) {
        let sheet = WindowKey::new(1);
        let confirm = WindowKey::new(2);
        let mut set = WindowSet::with_primary();
        set.insert(
            sheet,
            WindowRole::Dialog {
                parent: WindowKey::PRIMARY,
            },
        );
        set.insert(confirm, WindowRole::Dialog { parent: sheet });
        (set, sheet, confirm)
    }

    #[test]
    fn closing_a_window_closes_everything_that_belongs_to_it() {
        // A dialog can open a dialog, so this has to be transitive. Anything
        // left behind is a window the application cannot close and the user
        // cannot explain.
        let (mut set, sheet, confirm) = nested();

        let closed = set.remove(WindowKey::PRIMARY);

        assert_eq!(
            closed,
            vec![confirm, sheet, WindowKey::PRIMARY],
            "innermost first, so a caller destroying surfaces in this order \
             never destroys a parent while a child still points at it"
        );
        assert!(set.is_empty());
    }

    #[test]
    fn closing_a_dialog_leaves_its_parent_alone() {
        let (mut set, sheet, confirm) = nested();

        assert_eq!(set.remove(confirm), vec![confirm]);
        assert!(set.contains(sheet));
        assert!(set.contains(WindowKey::PRIMARY));
    }

    #[test]
    fn a_window_with_a_dialog_on_it_is_inert() {
        // **The claim `WindowRole::Dialog` exists for.** Every other framework
        // hands you two windows and leaves this to the application, per
        // platform — and the half nobody remembers is the screen reader, which
        // walks into the window behind the modal without a sound.
        let (mut set, sheet, _confirm) = nested();

        assert!(set.is_inert(WindowKey::PRIMARY), "its sheet is up");
        assert!(
            set.is_inert(sheet),
            "and the sheet has a confirmation on it"
        );

        set.remove(sheet);

        assert!(
            !set.is_inert(WindowKey::PRIMARY),
            "and it takes input again the moment the sheet goes — answered live \
             rather than held as a flag, which is what goes stale when a dialog \
             closes mid-drag"
        );
    }

    #[test]
    fn a_regular_second_window_does_not_make_the_first_inert() {
        // The distinction that makes the inert rule usable at all: an inspector
        // beside a document is two windows, not a modal.
        let mut set = WindowSet::with_primary();
        set.insert(WindowKey::new(1), WindowRole::Regular);

        assert!(!set.is_inert(WindowKey::PRIMARY));
        assert_eq!(set.remove(WindowKey::new(1)), vec![WindowKey::new(1)]);
        assert!(set.contains(WindowKey::PRIMARY));
    }

    #[test]
    fn focus_returns_to_the_parent_a_dialog_belonged_to() {
        let (set, sheet, _confirm) = nested();
        assert_eq!(set.focus_after_closing(sheet), Some(WindowKey::PRIMARY));
        assert_eq!(
            set.focus_after_closing(WindowKey::PRIMARY),
            None,
            "a regular window has no parent to hand focus back to, and picking \
             one of several unrelated windows is the platform's decision to make"
        );
    }

    #[test]
    fn the_order_windows_opened_in_is_the_order_they_are_listed() {
        // What a window menu shows. Reinserting a key must not reorder it, or
        // the menu shuffles whenever a role changes.
        let mut set = WindowSet::with_primary();
        set.insert(WindowKey::new(1), WindowRole::Regular);
        set.insert(WindowKey::new(2), WindowRole::Regular);
        set.insert(
            WindowKey::new(1),
            WindowRole::Dialog {
                parent: WindowKey::PRIMARY,
            },
        );

        assert_eq!(
            set.keys(),
            vec![WindowKey::PRIMARY, WindowKey::new(1), WindowKey::new(2)]
        );
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn removing_a_window_that_is_not_open_reports_only_itself() {
        // Two paths racing to close the same window is ordinary.
        let mut set = WindowSet::with_primary();
        assert_eq!(set.remove(WindowKey::new(9)), vec![WindowKey::new(9)]);
        assert!(set.contains(WindowKey::PRIMARY));
    }
}
