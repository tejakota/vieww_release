//! The platform's half of multi-window: a request queue the event loop drains.
//!
//! [`vieww_render::window`] built the framework's half on 2026-08-14 — the
//! [`Windows`] service, [`WindowSet`]'s policy, and the rule that a window owes
//! a rebuild when another one's input marked it pending. What it deliberately
//! left out was this: something that turns a request into an actual surface.
//!
//! # Why a queue rather than opening the window there and then
//!
//! [`Windows::open_boxed`] takes `&self` and is called from wherever an
//! application happens to be — a button's handler, a menu item, a keyboard
//! shortcut. Creating a winit window needs `&ActiveEventLoop`, which exists only
//! inside the loop's own callbacks and cannot be stored. The two are hours apart
//! in the API and microseconds apart in wall time, and a queue is the only thing
//! that bridges them without handing an `ActiveEventLoop` to application code
//! that could outlive it.
//!
//! So `open` records and returns, the loop drains in `about_to_wait`, and the
//! key is minted **at request time** — which is what lets a caller keep the key
//! without waiting, exactly as the trait promises.
//!
//! # The wake is not optional
//!
//! An idle vieww application sleeps in `ControlFlow::Wait` and costs no CPU.
//! That is the property [`crate::App`] is built around, and it means a request
//! recorded while nothing else is happening would sit in the queue **until the
//! user moved the mouse**. So every request wakes the loop, through the same
//! [`Waker`] a background task uses to say it has finished. Without it, "New
//! Window" does nothing until you jiggle something — which is not a defect
//! anybody would find in a test, because a test never idles.
//!
//! # One [`WindowSet`], not two
//!
//! The policy lives here and the loop reads it through this handle rather than
//! keeping its own copy. That is the same argument [`Windows::open_windows`]
//! makes about applications: a second copy of which windows are open is wrong in
//! exactly the cases nobody tests — the window the user closed, the window that
//! failed to open, the window closed while a dialog was up.

use std::cell::RefCell;
use std::rc::Rc;
// `next` is the only `Cell` here, and it is desktop-only — see the field.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::cell::Cell;

use vieww_foundation::task::FrameWaker;
use vieww_foundation::ServiceError;
// Through `vieww_render`'s re-export rather than a direct dependency:
// `vieww-element` is a dev-dependency of this crate and promoting it would put
// a second path to the same crate in the manifest for no gain. `Runtime` and
// `Signal` are already in `Windows`'s own signature, so this crate cannot
// implement that trait without naming them either way.
use vieww_render::element::{Runtime, Signal};
use vieww_render::{WindowBuild, WindowKey, WindowSet, WindowSpec, Windows};
// Only the desktop arm of `open_boxed` inspects a role — a phone refuses every
// request before it gets that far — so on Android and iOS this import would be
// unused and `-D warnings` would stop the build.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use vieww_render::WindowRole;

use crate::app::Waker;

/// What the loop has been asked for and has not yet done.
///
/// Shared by [`PlatformWindows`], which writes it, and the runner, which drains
/// it. `Rc` rather than a channel because both live on the UI thread by
/// construction — a `Windows` handle reached through `Services` is reached from
/// a build or a handler, and both of those are the loop.
#[derive(Debug)]
pub(crate) struct Requests {
    /// Requested, not yet created. Drained by the runner.
    opens: RefCell<Vec<Pending>>,
    /// Requested to close, not yet closed.
    closes: RefCell<Vec<WindowKey>>,
    /// Which windows are open and what belongs to what. **The** copy.
    set: RefCell<WindowSet>,
    /// How the tree sees [`set`](Self::set).
    open: Signal<Vec<WindowKey>>,
    /// The next key to hand out. Starts at 1: 0 is [`WindowKey::PRIMARY`].
    ///
    /// Absent on a phone rather than merely unused: `open_boxed` refuses there
    /// before it reaches the point of minting anything, so a counter would be a
    /// field that can never change — and saying so with `cfg` is honest where
    /// an `allow(dead_code)` would just be quiet.
    ///
    /// **Not dead, and audited as such**: `open_boxed` reads it, bumps it and
    /// puts the result in the set and in the `Pending` the loop drains, and
    /// exactly one `Requests` exists per `App`, so no two windows can be minted
    /// the same key. It reads like an allocator that hands out nothing because
    /// the read is inside the desktop `cfg` arm, several screens below the
    /// declaration. `every_window_gets_a_key_of_its_own_and_a_closed_ones_is_not_reissued`
    /// pins the property, because a duplicate key is not a compile error: it
    /// shows up as one window's events driving another.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    next: Cell<u64>,
    waker: Waker,
}

/// One window that has been asked for.
pub(crate) struct Pending {
    pub(crate) key: WindowKey,
    pub(crate) spec: WindowSpec,
    pub(crate) build: WindowBuild,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The builder is a closure and has nothing printable about it.
        f.debug_struct("Pending")
            .field("key", &self.key)
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}

impl Requests {
    fn new(runtime: &Runtime, waker: Waker) -> Self {
        Self {
            opens: RefCell::new(Vec::new()),
            closes: RefCell::new(Vec::new()),
            // The primary window is in the set from the start, before it has a
            // surface. An application reading `open_windows` during its own
            // first build otherwise sees an empty list and concludes it has no
            // windows, which is true of the compositor and false of the app.
            set: RefCell::new(WindowSet::with_primary()),
            open: runtime.signal(vec![WindowKey::PRIMARY]),
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            next: Cell::new(1),
            waker,
        }
    }

    /// Publish the set as the signal a tree reads.
    ///
    /// One write, from one place. Two updates that could disagree is the second
    /// copy this design exists to remove.
    fn publish(&self) {
        self.open.set(self.set.borrow().keys());
    }

    /// Everything asked for since the last call.
    pub(crate) fn take_opens(&self) -> Vec<Pending> {
        std::mem::take(&mut *self.opens.borrow_mut())
    }

    /// Every key asked to close since the last call.
    pub(crate) fn take_closes(&self) -> Vec<WindowKey> {
        std::mem::take(&mut *self.closes.borrow_mut())
    }

    /// Record that a window has genuinely gone, and return everything that goes
    /// with it.
    ///
    /// The returned keys include `key` itself and every dialog that belonged to
    /// it, deepest first — [`WindowSet::remove`]'s order, which is the order
    /// they have to be destroyed in.
    pub(crate) fn forget(&self, key: WindowKey) -> Vec<WindowKey> {
        let doomed = self.set.borrow_mut().remove(key);
        self.publish();
        doomed
    }

    /// `true` when `key` has a dialog on it and must take no input.
    pub(crate) fn is_inert(&self, key: WindowKey) -> bool {
        self.set.borrow().is_inert(key)
    }

    /// Where focus goes when `key` closes.
    pub(crate) fn focus_after_closing(&self, key: WindowKey) -> Option<WindowKey> {
        self.set.borrow().focus_after_closing(key)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.set.borrow().is_empty()
    }
}

/// [`Windows`], backed by a winit event loop.
///
/// Taken from [`App::windows`](crate::App::windows) **before** the loop runs and
/// registered into the application's [`Services`](vieww_foundation::Services),
/// the same shape [`App::waker`](crate::App::waker) already uses and for the
/// same reason: the tree needs the service before there is a loop to back it.
///
/// ```no_run
/// # use std::rc::Rc;
/// # use vieww_foundation::SharedServices;
/// # use vieww_platform_winit::{services, App};
/// # use vieww_render::Windows;
/// # use vieww_widget::Inherited;
/// # fn root() -> vieww_widget::WidgetNode { unimplemented!() }
/// let app = App::new();
///
/// let mut registry = services::platform();
/// registry.provide::<dyn Windows>(app.windows());
/// let services = SharedServices::new(registry);
///
/// app.run(move |driver| {
///     driver.set_root(Inherited::new(services, root()));
/// })?;
/// # Ok::<(), vieww_platform_winit::PlatformError>(())
/// ```
#[derive(Debug, Clone)]
pub struct PlatformWindows {
    inner: Rc<Requests>,
}

impl PlatformWindows {
    /// Backed by `runtime`'s reactive graph and waking through `waker`.
    ///
    /// The runtime matters and is not interchangeable: a signal minted from a
    /// different graph than the trees reading it marks nothing pending, so a
    /// window menu would never rebuild and every test of it would pass. Every
    /// window's driver is built with this same runtime — see
    /// [`FrameDriver::with_runtime`](vieww_render::FrameDriver::with_runtime).
    pub(crate) fn new(runtime: &Runtime, waker: Waker) -> Self {
        Self {
            inner: Rc::new(Requests::new(runtime, waker)),
        }
    }

    pub(crate) fn requests(&self) -> Rc<Requests> {
        Rc::clone(&self.inner)
    }
}

impl Windows for PlatformWindows {
    fn open_boxed(&self, spec: WindowSpec, build: WindowBuild) -> Result<WindowKey, ServiceError> {
        // An application does not own its windows on a phone: the activity or
        // the scene *is* the window, and a second one is not something a UI
        // framework may conjure. `Unsupported` rather than `Failed`, because no
        // amount of retrying or asking differently will change it — which is
        // exactly the distinction that lets a tree hide its "New Window" item on
        // a phone with no `cfg` of its own.
        //
        // No `return`: the two blocks below are consecutive statements of which
        // exactly one survives `cfg`, and the survivor is the tail expression. A
        // `return` in the first would leave the function body typed `()` on a
        // phone, where the second block does not exist to supply a value.
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let _ = (spec, build);
            Err(ServiceError::unsupported_on(
                "Windows",
                vieww_foundation::TargetPlatform::current(),
            ))
        }

        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            // A dialog whose parent has already gone is not a dialog, it is an
            // orphan that nothing will ever close — `WindowSet::remove` reaches
            // children through their parent, so a parentless one would outlive
            // every rule in this module. Refused rather than opened, and `Failed`
            // rather than `Unsupported` because asking again with a live parent is
            // a thing that works.
            if let WindowRole::Dialog { parent } = spec.role() {
                if !self.inner.set.borrow().contains(parent) {
                    return Err(ServiceError::failed(
                        "the parent window is not open, so its dialog could never be \
                     closed with it",
                    ));
                }
            }

            let key = WindowKey::new(self.inner.next.get());
            self.inner.next.set(self.inner.next.get() + 1);

            // In the set **now**, not when the surface appears. The key has been
            // minted and handed back, so an application that immediately reads
            // `open_windows` has to see it — otherwise "did my window open" is
            // answered `false` for a window that is on its way, and the obvious
            // application code retries for ever.
            self.inner.set.borrow_mut().insert(key, spec.role());
            self.inner
                .opens
                .borrow_mut()
                .push(Pending { key, spec, build });
            self.inner.publish();
            // See the module docs: without this an idle loop services the request
            // whenever the user next moves the mouse.
            self.inner.waker.wake();
            Ok(key)
        }
    }

    fn close(&self, key: WindowKey) -> Result<(), ServiceError> {
        // Not an error when it is not open: two paths racing to close one
        // window is ordinary and the second has nothing to fix. The queue is
        // still appended to, because a key requested and not yet created has to
        // be cancellable — otherwise the window appears *after* the close.
        self.inner.closes.borrow_mut().push(key);
        self.inner.waker.wake();
        Ok(())
    }

    fn open_windows(&self) -> Signal<Vec<WindowKey>> {
        self.inner.open.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Size;
    use vieww_render::WindowsExt;

    fn windows() -> (PlatformWindows, Rc<Requests>) {
        let runtime = Runtime::new();
        // `default` is the detached state — no loop attached — which is exactly
        // where an application's own handle sits before `run`. A wake on it is a
        // documented no-op rather than a panic, and that is under test here as
        // much as anywhere: a window asked for before the loop exists must not
        // bring the process down.
        let waker = Waker::default();
        let windows = PlatformWindows::new(&runtime, waker);
        let requests = windows.requests();
        (windows, requests)
    }

    fn spec() -> WindowSpec {
        WindowSpec::new("Second").with_size(Size::new(400.0, 300.0))
    }

    #[test]
    fn a_key_is_minted_before_the_window_exists() {
        // The trait's promise: the request has been accepted and the only thing
        // outstanding is the platform's own work.
        let (windows, requests) = windows();
        let key = windows.open(spec(), |_, _| {}).expect("recorded");

        assert_ne!(key, WindowKey::PRIMARY, "the primary is never handed out");
        assert!(
            windows.is_open(key),
            "an application that asks whether its new window is open has to be \
             told yes — it is on its way, and answering no makes the obvious \
             retry loop spin for ever"
        );
        assert_eq!(requests.take_opens().len(), 1);
        assert!(
            requests.take_opens().is_empty(),
            "a drain is a take: the loop must not create the same window twice"
        );
    }

    #[test]
    fn every_window_gets_a_key_of_its_own_and_a_closed_ones_is_not_reissued() {
        // The counter behind `Requests::next` is the whole of window identity on
        // a desktop: two windows sharing a key would not fail to compile, or even
        // fail to open — one window's events would simply drive the other, and
        // the closest thing to a symptom is a menu item acting on the wrong
        // window. So the property is pinned here rather than inferred from the
        // counter being incremented somewhere.
        let (windows, requests) = windows();
        let first = windows.open(spec(), |_, _| {}).expect("recorded");
        let second = windows.open(spec(), |_, _| {}).expect("recorded");
        requests.forget(first);
        let third = windows.open(spec(), |_, _| {}).expect("recorded");

        let keys = [WindowKey::PRIMARY, first, second, third];
        for (at, key) in keys.iter().enumerate() {
            assert!(
                !keys[at + 1..].contains(key),
                "{key} was handed out twice: {keys:?}"
            );
        }
    }

    #[test]
    fn the_signal_a_window_menu_reads_follows_both_directions() {
        let (windows, requests) = windows();
        let open = windows.open_windows();
        assert_eq!(open.peek(), vec![WindowKey::PRIMARY]);

        let key = windows.open(spec(), |_, _| {}).expect("recorded");
        assert_eq!(open.peek(), vec![WindowKey::PRIMARY, key]);

        requests.forget(key);
        assert_eq!(
            open.peek(),
            vec![WindowKey::PRIMARY],
            "the signal is how a window menu learns, and a menu that only ever \
             grows is worse than none"
        );
    }

    #[test]
    fn closing_a_parent_takes_its_dialog_with_it() {
        // The policy `WindowSet` owns, reached through the handle the loop uses,
        // so the loop and the application cannot disagree about it.
        let (windows, requests) = windows();
        let parent = windows.open(spec(), |_, _| {}).expect("recorded");
        let dialog = windows
            .open(WindowSpec::new("Sheet").as_dialog(parent), |_, _| {})
            .expect("recorded");

        assert!(
            requests.is_inert(parent),
            "a parent under a dialog is inert"
        );

        let doomed = requests.forget(parent);
        assert!(doomed.contains(&dialog), "the dialog goes with its parent");
        assert!(doomed.contains(&parent));
        assert_eq!(
            doomed.first(),
            Some(&dialog),
            "deepest first, because that is the order they can be destroyed in"
        );
        assert!(!windows.is_open(dialog));
    }

    #[test]
    fn a_dialog_on_a_window_that_is_not_open_is_refused() {
        // Otherwise it is an orphan: `WindowSet::remove` reaches children
        // through their parent, so nothing would ever close it.
        let (windows, _requests) = windows();
        let error = windows
            .open(
                WindowSpec::new("Sheet").as_dialog(WindowKey::new(99)),
                |_, _| {},
            )
            .expect_err("no such parent");
        assert!(
            !error.is_permanent(),
            "asking again with a live parent works, so this must not read as \
             Unsupported"
        );
    }

    #[test]
    fn focus_returns_to_the_parent_when_a_dialog_closes() {
        let (windows, requests) = windows();
        let parent = windows.open(spec(), |_, _| {}).expect("recorded");
        let dialog = windows
            .open(WindowSpec::new("Sheet").as_dialog(parent), |_, _| {})
            .expect("recorded");

        assert_eq!(requests.focus_after_closing(dialog), Some(parent));
        requests.forget(dialog);
        assert!(
            !requests.is_inert(parent),
            "and the parent takes input again"
        );
    }

    #[test]
    fn a_close_is_queued_even_for_a_window_that_has_not_appeared_yet() {
        // The race that would otherwise leave a window on screen with nothing
        // holding it: asked for, closed before the loop drained, then created.
        let (windows, requests) = windows();
        let key = windows.open(spec(), |_, _| {}).expect("recorded");
        windows.close(key).expect("closing is never an error");

        assert_eq!(requests.take_closes(), vec![key]);
    }

    #[test]
    fn closing_something_that_was_never_open_is_not_an_error() {
        let (windows, _requests) = windows();
        windows
            .close(WindowKey::new(42))
            .expect("two paths racing to close one window is ordinary");
    }
}
