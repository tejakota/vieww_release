//! The whole pipeline, wired together.
//!
//! [`FrameDriver`] is the thing that actually connects the three trees to the
//! paint layer: it implements [`FrameSink`], so a
//! [`FrameScheduler`](vieww_paint::FrameScheduler) drives it at vsync and the
//! phases run in the one correct order without any application having to remember
//! what that order is.
//!
//! This lives in the render crate rather than the paint crate because it has to
//! name all three trees, and the paint layer sits below them.

use std::time::Duration;

use vieww_animation::{Lerp, Tickers, Tween};
use vieww_element::{Animation, ElementTree, Runtime};
use vieww_foundation::{
    Capture, Constraints, Cursor, DroppedFiles, FileDrag, ImeEvent, KeyEvent, LogicalKey, NamedKey,
    Offset, PointerEvent, PointerPhase, Rect, ScrollEvent, Size, ViewMetrics,
};
use vieww_paint::{
    Damage, DamageCullStats, FlattenStats, FrameInfo, FrameScheduler, FrameSink, FrameStats,
    LayerTree, Scene, SceneFlattener,
};
use vieww_widget::{Inherited, WidgetNode};

use crate::{
    Announcement, Dispatched, FocusManager, HitTestResult, PointerRouter, RenderId, RenderObject,
    RenderOwner, RenderTree, SemanticAction, SemanticsTree,
};

/// The broadest invalidation observed while producing the current frame.
///
/// This is intentionally renderer-neutral. It describes *why* work may be
/// needed, not how GPU, Hybrid, or CPU should execute it. The driver upgrades
/// the value as later phases discover stronger requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InvalidationKind {
    /// No visual work was discovered. The frame may still have run for input,
    /// semantics, focus, or another non-visual reason.
    None,
    /// State changed and caused a rebuild, but no layout or paint work has yet
    /// been established. This is useful for keeping state-only work visible
    /// in diagnostics without pretending it is a rendering invalidation.
    State,
    /// Recorded visual content changed without requiring layout.
    Paint,
    /// Geometry or layout work changed and may require repainting.
    Layout,
    /// The surface contents cannot be trusted and must be repainted completely.
    Full,
}

impl InvalidationKind {
    fn upgrade(&mut self, next: Self) {
        if next > *self {
            *self = next;
        }
    }
}

/// Owns every tree and turns vsync into a painted frame.
///
/// # What a frame does
///
/// | phase | work |
/// |---|---|
/// | animate | animations advance and pending whatever displays them |
/// | build | pending elements rebuild, then the render tree is synced to them |
/// | layout | pending render objects measure and place |
/// | paint | the layers whose subtree changed re-record; the rest are reused |
/// | composite | the layers are flattened, and their changes become damage |
///
/// The ordering is the part that matters, and it is not negotiable — see
/// [`RenderOwner::draw_frame`] for why layout must not run before the sync.
///
/// # What a frame costs
///
/// Not the size of the screen. A change repaints the layer containing it and
/// damages the pixels that actually differ, and a backend that honours
/// `damage` — see `GpuRenderer::render_damaged` — rasterises only
/// those. A frame where nothing changed reports [`Damage::is_clean`] and can be
/// skipped outright.
/// A panic caught in the layout or paint phase of a frame.
///
/// # What this closes
///
/// `ElementTree` has guarded **build** since it was written: a widget whose
/// `build` panics is replaced by a placeholder, the rest of the screen keeps
/// drawing, and the failure is recorded as a `BuildError` the application can
/// show. Layout and paint had no such guard, and the studio's own source said
/// so out loud: "`RenderOwner::draw_frame` guards nothing, and a `layout` that
/// divides by a zero-height constraint still takes the process down".
///
/// For an editor whose entire purpose is running UI code that is being written
/// — code that does not compile half the time and divides by a zero constraint
/// the other half — that is the difference between a magenta box and every
/// unsaved buffer in the window being gone.
///
/// A caught layout or paint panic **abandons the rest of that phase and the
/// frame**: the previously composited scene stays on screen. That is
/// deliberate. A render tree that panicked partway through layout has some
/// objects sized and some not, and painting it would draw a frame assembled
/// from two different states — which looks like a rendering bug rather than
/// like the error it is. Keeping the last good frame and reporting the panic is
/// the honest pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameError {
    /// Which phase it happened in.
    pub phase: FramePhase,
    /// The panic message, when it was a `&str` or a `String`.
    pub message: String,
}

/// The phase a [`FrameError`] was caught in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePhase {
    Layout,
    Paint,
}

impl FramePhase {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::Paint => "paint",
        }
    }
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a render object panicked during {}: {}",
            self.phase.name(),
            self.message
        )
    }
}

/// Run `work`, turning a panic into a [`FrameError`] rather than an unwind.
///
/// `AssertUnwindSafe` because the render tree is `&mut` across the boundary and
/// cannot be otherwise. The assertion it makes is real and is the price of the
/// guard: a tree whose layout panicked partway may hold objects in an
/// inconsistent state. What makes that acceptable is the rule above — the frame
/// is abandoned, nothing composited from it reaches the screen, and the next
/// frame lays the tree out again from the top.
fn guarded<R>(phase: FramePhase, work: impl FnOnce() -> R) -> Result<R, FrameError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).map_err(|payload| FrameError {
        phase,
        message: panic_message(&*payload),
    })
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&'static str>().map_or_else(
        || {
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "<non-string panic payload>".to_owned())
        },
        |message| (*message).to_owned(),
    )
}

#[derive(Debug)]
pub struct FrameDriver {
    elements: ElementTree,
    owner: RenderOwner,
    constraints: Constraints,
    surface: Rect,
    /// The repaint boundaries and their recordings.
    ///
    /// The render tree records *into* this rather than into one flat scene, which
    /// is what lets a frame re-record only the layers whose subtree changed.
    layers: LayerTree,
    /// This frame's layers flattened, for backends that take a command list.
    ///
    /// Retained across frames and rebuilt span by span rather than wholesale —
    /// see [`SceneFlattener`]. The flattener owns the commands; `scene()` hands
    /// out its view.
    flattener: SceneFlattener,
    /// Number of times the retained layer tree has actually been flattened
    /// into `scene`. A clean frame must not increment this. Kept as a cheap
    /// framework-level counter so performance tests can assert work avoided,
    /// not merely pixels produced.
    scene_rebuilds: u64,
    /// Set when the retained scene was discarded while the layer tree stayed
    /// clean, so the next frame must flatten anyway. See the use site in
    /// `draw_frame` and [`Self::trim_memory`], which is the only thing that
    /// sets it.
    needs_flatten: bool,
    /// How many commands damage culling would skip in the current scene.
    ///
    /// Computed lazily from the retained scene and the current damage, because
    /// not every consumer needs it — a backend that does its own culling per
    /// tile does not need a frame-level number. Tests do, and a performance
    /// overlay does.
    damage_cull: DamageCullStats,
    /// Broad invalidation classification for the current frame.
    invalidation: InvalidationKind,
    /// Panics caught in the layout or paint phase, newest last.
    ///
    /// Drained by the application. See [`Self::take_frame_errors`] and the
    /// guards in `layout` and `paint` for what this closes.
    frame_errors: Vec<FrameError>,
    /// Live pointers, their routes, and the arena they contest in.
    pointers: PointerRouter,
    /// What the cursor is over, innermost first.
    ///
    /// Kept as the *previous* answer so that a move can be diffed against it:
    /// hover is delivered as enter and leave rather than as a position, and
    /// there is nothing else that knows what has already been entered.
    hovered: Vec<RenderId>,
    /// Which render object the keyboard is talking to.
    focus: FocusManager,
    /// Animations driven explicitly — an [`Animation`] the application started.
    ///
    /// Implicit animations are *not* here: they live on the elements that own
    /// them and are advanced through [`ElementTree::tick_states`]. Two
    /// registries rather than one because they are reached differently — one by
    /// a handle the application holds, the other by the element that created it
    /// — and merging them would mean the element tree handing out `Rc`s to its
    /// own state for the driver to hold.
    animations: Tickers,
    damage: Damage,
    /// `true` when the surface's current contents cannot be trusted, so the next
    /// frame must repaint all of it rather than only what changed.
    ///
    /// Diffing answers "what changed since the last frame", which is the wrong
    /// question when the surface was never painted, or was just resized, or the
    /// device was lost. In those cases there is no last frame to have changed
    /// *from*, and a diff would leave uninitialised pixels wherever this frame
    /// happens not to draw.
    full_repaint: bool,
    size: Size,
    /// What the platform says the surface is like. Published above the root, so
    /// `SafeArea` and anything else platform-shaped can read it.
    metrics: ViewMetrics,
    /// Who is reading the surface. Published above the root beside the metrics,
    /// so [`Sensitive`](vieww_widget::Sensitive) masks without any application
    /// code running at the moment a screenshot is taken.
    ///
    /// A published state rather than a callback, for the reason
    /// [`Capture`] gives: on some platforms the notification arrives *after* the
    /// pixels were read, so anything reacting to an event has already lost.
    capture: Capture,
    /// System accessibility preferences, published as a fourth provider — see
    /// [`Accessibility`]'s module docs for why it is its own rather than a
    /// field on `ViewMetrics`.
    accessibility: vieww_foundation::Accessibility,
    /// The theme published above the root, when the application set one.
    ///
    /// See [`FrameDriver::set_theme`].
    theme: Option<std::rc::Rc<vieww_widget::ThemeData>>,
    /// Whether a file from outside is over the window, published above the tree.
    file_drag: FileDrag,
    /// Files dropped since the last take, gathered into one delivery.
    dropped: DroppedFiles,
    /// The semantics tree as of the last [`take_announcements`](Self::take_announcements).
    ///
    /// The only state a live region needs, and it is one tree rather than a log:
    /// "what should be said now" is entirely a function of what changed since
    /// what was said last.
    announced: SemanticsTree,
    /// The application's own root, without the metrics wrapper.
    ///
    /// Kept so that a notch, a rotation or a keyboard can re-publish without the
    /// application being asked to hand its tree over again.
    root: Option<WidgetNode>,
    /// Consecutive frames that ended with something pending.
    ///
    /// Layout is the only phase that can pending an element after the build has
    /// run, so this counts consecutive frames whose *layout* wrote a signal.
    /// [`drive`](Self::drive) asks for the frame that rebuilds the reader, and
    /// this is the bound that stops it asking for ever if the write never
    /// settles — the memory two reverted attempts did not have.
    layout_pending_streak: u32,
}

impl FrameDriver {
    /// How many times a frame may rebuild and re-lay out to settle what layout
    /// discovered, and — as a backstop — how many consecutive frames may still
    /// end pending before [`drive`](Self::drive) stops asking for another.
    ///
    /// **Primarily a bound on passes within one frame.** `FrameSink::layout`
    /// settles in place so that a `LayoutBuilder` or a viewport never puts a
    /// frame on screen built against the previous answer. The cross-frame streak
    /// below it remains for the tree that will not converge even then.
    ///
    /// A legitimate run is a **chain**, and its length is bounded by how deeply
    /// render objects that write from layout are nested — one frame per link,
    /// since each link needs a rebuild before the next one can measure. There is
    /// exactly one such render object today, `RenderViewport`, so eight is far
    /// more nesting than any real tree has and still cheap enough to be
    /// invisible if it is ever spent: eight frames is 133ms at 60Hz, against a
    /// pegged core for the unbounded version.
    ///
    /// Beyond it the writes are not converging, and no number of frames will fix
    /// that — so the loop stops, says which elements, and leaves the content
    /// stale. Stale is the right direction to fail in; the spin is what took two
    /// reverts.
    pub const MAX_LAYOUT_PENDING_FRAMES: u32 = 8;

    /// A driver for a surface of the given size.
    ///
    /// The root is laid out under *tight* constraints: a window is a fixed size,
    /// and the root filling it is what makes a full-screen background possible
    /// without every application wrapping its tree in something that stretches.
    #[must_use]
    pub fn new(surface: Size) -> Self {
        Self::with_runtime(surface, Runtime::new())
    }

    /// A driver for a surface, sharing an existing reactive graph.
    ///
    /// **This is what a second window is.** A [`FrameDriver`] already owns
    /// everything that is per-surface and nothing that is not — its own element
    /// tree, render tree, layers, scene, pointer router, focus and tickers — so
    /// two windows are two drivers rather than one driver taught to be two.
    ///
    /// What must *not* be duplicated is the state the application is about. A
    /// [`Runtime`] is an `Rc` to one reactive graph, so handing the same one to
    /// two drivers means a signal written by either marks its readers in both:
    /// a document open in two windows is one document, and closing one window
    /// drops one tree and leaves the other's subscriptions alone.
    ///
    /// ```no_run
    /// use vieww_element::Runtime;
    /// use vieww_foundation::Size;
    /// use vieww_render::FrameDriver;
    ///
    /// let shared = Runtime::new();
    /// let main = FrameDriver::with_runtime(Size::new(800.0, 600.0), shared.clone());
    /// let inspector = FrameDriver::with_runtime(Size::new(320.0, 600.0), shared);
    /// ```
    ///
    /// Each driver still runs its own frame — vsync, damage and repainting are
    /// per-surface, and a window that nothing moved in should not repaint
    /// because another one animated.
    ///
    /// # What had to be fixed to make this sound
    ///
    /// `ElementId` used to be an arena slot index within one tree, so two trees
    /// minted the same ids and the runtime's shared pending set could not tell
    /// them apart: drawing one window discarded the other's pending marks, and
    /// `is_alive` could answer true for the other tree's element. The defect
    /// predated this constructor — `ElementTree::with_runtime` had always had
    /// it, and nothing had ever shared a runtime, so nothing had exercised it.
    ///
    /// Ids now carry the tree they belong to. See
    /// `a_window_drawing_does_not_consume_another_windows_pending_work`.
    #[must_use]
    pub fn with_runtime(surface: Size, runtime: Runtime) -> Self {
        let bounds = Rect::from_origin_size(Offset::ZERO, surface);
        Self {
            elements: ElementTree::with_runtime(runtime),
            owner: RenderOwner::new(),
            constraints: Constraints::tight(surface),
            surface: bounds,
            layers: LayerTree::new(),
            flattener: SceneFlattener::new(),
            needs_flatten: false,
            scene_rebuilds: 0,
            damage_cull: DamageCullStats::default(),
            invalidation: InvalidationKind::None,
            frame_errors: Vec::new(),
            pointers: PointerRouter::new(),
            hovered: Vec::new(),
            focus: FocusManager::new(),
            animations: Tickers::new(),
            // The first frame has no previous contents, so all of it is damage.
            damage: Damage::everything(bounds),
            full_repaint: true,
            size: Size::ZERO,
            metrics: ViewMetrics::plain(surface),
            capture: Capture::default(),
            accessibility: vieww_foundation::Accessibility::default(),
            theme: None,
            file_drag: FileDrag::default(),
            dropped: DroppedFiles::default(),
            announced: SemanticsTree::default(),
            root: None,
            layout_pending_streak: 0,
        }
    }

    /// The element tree, for reading state and driving it directly.
    ///
    /// # Do not mount the application's root through this
    ///
    /// [`ElementTree::set_root`] is reachable from here and is **not** the same
    /// as [`FrameDriver::set_root`]. The driver keeps the root widget so it can
    /// re-publish it wrapped in an [`Inherited<ViewMetrics>`](Inherited)
    /// whenever the surface changes; a root mounted straight onto the element
    /// tree leaves the driver with nothing to re-publish, so every
    /// [`SafeArea`](vieww_widget::SafeArea) in the application reads zero
    /// insets for the rest of the process.
    ///
    /// Nothing about that fails loudly — it is a screen that draws under the
    /// notch — so [`set_view_metrics`](Self::set_view_metrics) complains in
    /// debug instead. See its docs.
    /// Take the layout and paint panics caught since the last call.
    ///
    /// Drained rather than read, for the same reason
    /// `ElementTree::take_build_errors` is: an application shows each failure
    /// once, and a list that only grows turns one broken widget into an
    /// unbounded log.
    pub fn take_frame_errors(&mut self) -> Vec<FrameError> {
        std::mem::take(&mut self.frame_errors)
    }

    /// Whether the last frame gave up partway through layout or paint.
    ///
    /// What tells a caller that the composited scene is the *previous* frame's
    /// rather than this one's.
    #[must_use]
    pub fn frame_failed(&self) -> bool {
        !self.frame_errors.is_empty()
    }

    pub fn elements(&mut self) -> &mut ElementTree {
        &mut self.elements
    }

    /// The render tree owner, for hit testing and inspection.
    #[must_use]
    pub const fn owner(&self) -> &RenderOwner {
        &self.owner
    }

    /// The render tree owner, mutably — for the questions that cannot be asked
    /// through a shared borrow.
    ///
    /// [`RenderTree::intrinsic`] is the reason this exists: measuring a subtree
    /// takes the object out of its slot while the query runs, so it needs
    /// `&mut` even though it changes nothing an observer can see. An inspector
    /// that wants to ask a node how big it would like to be needs this, and so
    /// does any test of the intrinsic protocol.
    ///
    /// # This is a sharp tool
    ///
    /// Everything else reachable from here mutates the tree the framework is
    /// about to draw from. It is exposed because the alternative is the failure
    /// this repository already made once and wrote down: `RenderFactory::register`
    /// was public, correct and unit-tested, and no application could reach it,
    /// because `owner` handed out a shared borrow and `App` built the driver.
    /// A mechanism nothing outside can call is not a feature. See
    /// `crates/vieww/tests/third_party_render_widget.rs`.
    pub const fn owner_mut(&mut self) -> &mut RenderOwner {
        &mut self.owner
    }

    /// Teach this driver to realise a widget type of your own.
    ///
    /// The application-facing end of `RenderFactory::register`. `create` turns
    /// a widget into the render object that draws it, and after this call a tree
    /// containing `W` lays out and paints like any built-in.
    ///
    /// ```no_run
    /// # use vieww_render::FrameDriver;
    /// # use vieww_foundation::Size;
    /// # fn example<MyWidget, MyRender>(driver: &mut FrameDriver, make: fn(&MyWidget) -> MyRender)
    /// # where MyWidget: vieww_widget::Widget, MyRender: vieww_render::RenderObject {
    /// driver.register::<MyWidget, MyRender>(make);
    /// # }
    /// ```
    ///
    /// # Why this exists, given the registry was already public
    ///
    /// Because it was not **reachable**. `RenderFactory::register` and
    /// `RenderOwner::register` have been public since the factory was written,
    /// and two doc comments told third parties to use them — but the only route
    /// to an owner from a running application was
    /// [`owner`](Self::owner), which hands out `&RenderOwner`, and
    /// [`App::run`](../../vieww_platform_winit/struct.App.html) constructs the
    /// driver itself. So the seam was documented, built, tested at its own
    /// level, and impossible to use from where an application stands. The gap
    /// was a missing accessor rather than a missing mechanism.
    ///
    /// # And why this rather than `owner_mut`
    ///
    /// `RenderOwner` also exposes `sync` and `tree_mut`, which drive the
    /// pipeline. Handing those to an application to reach a registry would make
    /// "register a widget" and "reach inside the frame you are in the middle of"
    /// the same permission. This is the one operation that is safe between
    /// frames, so it is the one that is exposed.
    pub fn register<W, R>(&mut self, create: impl Fn(&W) -> R + 'static)
    where
        W: vieww_widget::Widget,
        R: RenderObject,
    {
        self.owner.register::<W, R>(create);
    }

    /// The last frame, flattened into one scene.
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        self.flattener.scene()
    }

    /// What the last flatten actually rebuilt, in commands.
    ///
    /// [`scene_rebuilds`](Self::scene_rebuilds) answers "did we flatten?"; this
    /// answers "how much of the screen did flattening cost?" — which is the
    /// question a repaint boundary is supposed to bound, and which was
    /// unanswerable while the scene was rebuilt whole.
    #[must_use]
    pub const fn flatten_stats(&self) -> FlattenStats {
        self.flattener.stats()
    }

    /// Number of scene flattening passes performed by this driver.
    ///
    /// This is deliberately a count rather than a stopwatch: it is stable in
    /// tests and answers the architectural question "did we rebuild?" without
    /// pretending a headless timing result is a device benchmark.
    #[must_use]
    pub const fn scene_rebuilds(&self) -> u64 {
        self.scene_rebuilds
    }

    /// How many commands damage culling would skip for the current scene
    /// and damage.
    ///
    /// Computed lazily at the end of the composite phase, so this is
    /// always the current answer without a per-frame allocation. A test uses
    /// it to assert that a small damage region skipped most of the screen,
    /// following the framework rule that every optimization needs a regression
    /// test where the work is countable.
    #[must_use]
    pub const fn damage_cull_stats(&self) -> DamageCullStats {
        self.damage_cull
    }

    /// The damage-culled scene: only commands whose bounds intersect the
    /// current damage region.
    ///
    /// A backend that does not do its own per-region culling can take this
    /// instead of [`scene`](Self::scene) and skip the filtering work. The
    /// stats are the same as [`damage_cull_stats`](Self::damage_cull_stats).
    #[must_use]
    pub fn damage_culled_scene(&self) -> (Scene, DamageCullStats) {
        self.flattener.scene().damage_cull(&self.damage)
    }

    /// The broadest invalidation observed while producing the current frame.
    ///
    /// This is a diagnostic/architecture seam, not a promise that a backend can
    /// skip every phase for a given value. It is deliberately conservative: a
    /// stronger invalidation always wins.
    #[must_use]
    pub const fn invalidation(&self) -> InvalidationKind {
        self.invalidation
    }

    /// The repaint boundaries and their recordings.
    ///
    /// A backend that composites layers natively should walk this instead of
    /// taking [`scene`](Self::scene), which flattens them.
    #[must_use]
    pub const fn layers(&self) -> &LayerTree {
        &self.layers
    }

    /// What changed in the last frame.
    #[must_use]
    pub const fn damage(&self) -> &Damage {
        &self.damage
    }

    /// The size the root settled on in the last layout.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// The surface being painted into.
    #[must_use]
    pub const fn surface(&self) -> Rect {
        self.surface
    }

    /// Resize the surface, invalidating everything on it.
    ///
    /// A resize discards the old contents, so there is nothing worth diffing
    /// against and the next frame repaints in full.
    pub fn resize(&mut self, surface: Size) {
        self.surface = Rect::from_origin_size(Offset::ZERO, surface);
        self.constraints = Constraints::tight(surface);
        if self.metrics.size != surface {
            self.metrics.size = surface;
            self.republish();
        }
        self.invalidate();
    }

    // ----------------------------------------------------------- view metrics

    /// Mount `root`, with the view metrics published above it.
    ///
    /// Use this rather than `elements().set_root` in an application: it wraps
    /// the tree in the [`ViewMetrics`] provider that [`SafeArea`](vieww_widget::SafeArea)
    /// and anything else platform-shaped reads. The root is kept, so the wrapper
    /// can be rebuilt when a notch, a rotation or a keyboard changes the
    /// metrics without the application being asked to re-supply its tree.
    pub fn set_root(&mut self, root: impl Into<WidgetNode>) {
        // Every live pointer's route points into the tree that is being thrown
        // away. Delivering to those ids afterwards would reach nothing, or —
        // worse — whatever has since been given the same slot. This is the
        // automatic `clear` that used to be the caller's job to remember.
        self.pointers.clear();
        self.hovered.clear();
        self.root = Some(root.into());
        self.republish();
    }

    /// Throw the tree away and mount `root` fresh.
    ///
    /// [`set_root`](Self::set_root) reconciles, so an element whose widget still
    /// matches keeps its state — which is the whole point of it, and exactly
    /// wrong after a hot reload in which a state type changed shape. The old
    /// allocation has the old layout and the new code would read it with the
    /// new one; that is undefined behaviour rather than a glitch, so this exists
    /// to make "keep nothing" expressible.
    ///
    /// Costs a full rebuild and every animation and scroll offset in the tree.
    /// Use `set_root` unless something makes reuse unsound.
    pub fn remount(&mut self, root: impl Into<WidgetNode>) {
        self.elements.clear();
        self.set_root(root);
    }

    /// `true` once a root has been mounted through [`set_root`](Self::set_root).
    ///
    /// Distinct from "the element tree has something in it": a tree mounted
    /// through [`elements`](Self::elements) is live and drawable but invisible
    /// to view metrics.
    #[must_use]
    pub const fn has_root(&self) -> bool {
        self.root.is_some()
    }

    /// Where the cursor is, or `None` when it has left the window.
    ///
    /// Hover is not a gesture and does not go near the arena: it has no press to
    /// be routed from, so every move hit tests afresh. What is delivered is the
    /// *change* — each object is told once when the cursor arrives and once when
    /// it leaves — which is what keeps a mouse crossing a screen from calling a
    /// handler per pixel.
    ///
    /// Returns whether anything's hover state changed, so a caller can skip
    /// asking for a frame when nothing did.
    pub fn handle_hover(&mut self, position: Option<Offset>) -> bool {
        let now: Vec<RenderId> = match position {
            Some(position) => {
                let tree = self.owner.tree();
                self.owner
                    .hit_test(position)
                    .entries()
                    .iter()
                    .map(|entry| entry.id)
                    .filter(|&id| tree.object(id).is_some_and(|o| o.wants_hover()))
                    .collect()
            }
            None => Vec::new(),
        };
        if now == self.hovered {
            return false;
        }

        let tree = self.owner.tree();
        // Left first, entered second — the same order the gesture layer
        // delivers losers before winners, and for the same reason: something
        // un-highlighting after its replacement lit up looks wrong on screen.
        for &id in &self.hovered {
            if !now.contains(&id) {
                if let Some(object) = tree.object(id) {
                    object.handle_hover(false);
                }
            }
        }
        for &id in &now {
            if !self.hovered.contains(&id) {
                if let Some(object) = tree.object(id) {
                    object.handle_hover(true);
                }
            }
        }
        self.hovered = now;
        true
    }

    /// The pointer shape for whatever is under `position`.
    ///
    /// Innermost-first: the deepest object with an opinion wins, so a text
    /// field inside a card gets its I-beam rather than the card's arrow. An
    /// object with no opinion is skipped rather than treated as asking for the
    /// default — which is why [`RenderObject::cursor`] returns an `Option`.
    ///
    /// A separate query rather than something folded into
    /// [`handle_hover`](Self::handle_hover), because the two ask different
    /// questions of different objects: hover is about *state* and only objects
    /// that opted in receive it, while the cursor is about *appearance* and
    /// every object under the pointer is a candidate. Piggybacking would mean a
    /// field had to claim it wanted hover events it does not use in order to
    /// get an I-beam.
    ///
    /// Cheap enough to call on every pointer move: one hit test, then a walk
    /// that stops at the first answer.
    #[must_use]
    pub fn cursor_at(&self, position: Offset) -> Cursor {
        let tree = self.owner.tree();
        self.owner
            .hit_test(position)
            .entries()
            .iter()
            .find_map(|entry| {
                tree.object(entry.id)
                    .and_then(|object| object.cursor(entry.local))
            })
            .unwrap_or_default()
    }

    /// Feed one wheel or trackpad scroll in.
    ///
    /// Hit tested where it lands rather than routed, and offered outward from
    /// there until something consumes it — so a wheel over a list inside a page
    /// scrolls the list, and goes on to scroll the page once the list has
    /// nothing left to give.
    ///
    /// Returns whether anything took it.
    pub fn handle_scroll(&mut self, event: &ScrollEvent) -> bool {
        let hits = self.owner.hit_test(event.position);
        let tree = self.owner.tree();
        // Bound rather than returned straight out: the iterator borrows `hits`,
        // and a tail expression's temporaries outlive the locals it borrows.
        let taken = hits
            .bubble()
            .filter_map(|entry| tree.object(entry.id))
            .any(|object| object.handle_scroll(event));
        taken
    }

    /// The objects the cursor is currently over, innermost first.
    #[must_use]
    pub fn hovered(&self) -> &[RenderId] {
        &self.hovered
    }

    /// What the platform says the surface is like.
    #[must_use]
    pub const fn view_metrics(&self) -> ViewMetrics {
        self.metrics
    }

    /// Update what the platform says about the surface.
    ///
    /// Returns whether anything changed. A change rebuilds the tree — a
    /// keyboard appearing is a layout event — so a caller should not send
    /// unchanged metrics every frame, and this returning `false` makes it
    /// harmless if it does.
    ///
    /// The size is owned by [`resize`](Self::resize) and is overwritten here
    /// with the surface's, so that two sources of truth cannot disagree about
    /// how big the window is.
    ///
    /// # Panics
    ///
    /// In debug builds, if the metrics changed and no root has been mounted
    /// through [`set_root`](Self::set_root) — because there is then nothing to
    /// re-publish them to and the new insets are silently discarded.
    ///
    /// The mistake this catches is mounting the application through
    /// [`elements`](Self::elements) instead: the tree draws perfectly, and
    /// every [`SafeArea`](vieww_widget::SafeArea) in it reads zero for the life
    /// of the process. That cost a real debugging session on a real phone, with
    /// the correct insets sitting one function away from where they were
    /// needed. It fails loudly here rather than looking like a device with no
    /// notch.
    pub fn set_view_metrics(&mut self, metrics: ViewMetrics) -> bool {
        let metrics = ViewMetrics {
            size: self.surface.size(),
            ..metrics
        };
        if self.metrics == metrics {
            return false;
        }
        self.metrics = metrics;
        let published = self.republish();
        debug_assert!(
            published,
            "view metrics changed but no root has been mounted through \
             FrameDriver::set_root, so they reach nothing — every SafeArea will \
             read zero. Mounting through FrameDriver::elements().set_root() is \
             the usual cause; use FrameDriver::set_root instead."
        );
        true
    }

    // -------------------------------------------------------------- capture

    /// Who is reading the surface.
    #[must_use]
    pub const fn capture(&self) -> Capture {
        self.capture
    }

    /// Whether a file from outside is over the window.
    #[must_use]
    pub const fn file_drag(&self) -> FileDrag {
        self.file_drag
    }

    /// Tell the tree that a file is, or is no longer, being dragged over it.
    ///
    /// Called by the platform layer from its drag-hover callbacks. Returns
    /// `true` if this changed anything, so a caller can tell a repeated
    /// notification from a real one — the platform reports one hover event per
    /// file, so repeats are the normal case rather than the exception.
    pub fn set_file_drag(&mut self, drag: FileDrag) -> bool {
        if self.file_drag == drag {
            return false;
        }
        self.file_drag = drag;
        self.republish();
        true
    }

    /// Gather one more dropped file into the current delivery.
    ///
    /// The platform reports one event per file; this is where they become one
    /// drop. See [`DroppedFiles`].
    pub fn push_dropped_file(&mut self, path: std::path::PathBuf) {
        self.dropped.push(path);
    }

    /// Take whatever the user dropped, leaving nothing behind.
    ///
    /// A take rather than a read, the same as
    /// [`take_announcements`](Self::take_announcements) and for the same reason:
    /// a drop happens once, and two readers each acting on it is a worse failure
    /// than one of them missing it.
    pub fn take_dropped_files(&mut self) -> DroppedFiles {
        self.dropped.take()
    }

    /// Tell the tree that something other than the screen is reading it — or has
    /// stopped.
    ///
    /// Called by the platform layer from the screenshot, screen-recording and
    /// lifecycle callbacks. Returns `true` if this changed anything, so a caller
    /// can tell a redundant notification from a real one.
    ///
    /// # Ordering is the whole difficulty, and this is the safe half
    ///
    /// The masked frame has to be on the surface *before* the pixels are read,
    /// and on some platforms the notification arrives after. A platform layer
    /// that cannot get ahead of the capture should mask on the way *into* the
    /// background and unmask on the way out, which is what
    /// [`Capture::Recorded`] on `onPause` amounts to — pessimistic, and the only
    /// direction in which being wrong is harmless.
    ///
    /// Unlike [`set_view_metrics`](Self::set_view_metrics) this does not
    /// complain when there is no root: a screenshot of an application that has
    /// not mounted anything is a screenshot of nothing.
    pub fn set_capture(&mut self, capture: Capture) -> bool {
        if self.capture == capture {
            return false;
        }
        self.capture = capture;
        self.republish();
        true
    }

    /// The system accessibility preferences currently in force.
    #[must_use]
    pub const fn accessibility(&self) -> vieww_foundation::Accessibility {
        self.accessibility
    }

    /// Publish new accessibility preferences, re-mounting the root under them.
    ///
    /// The platform layer calls this at startup and whenever the OS reports the
    /// settings changed — which it does *while the application is running*, on
    /// every platform: a person can turn Reduce Motion on from Control Centre
    /// without leaving the app. Treating these as read-once-at-startup is the
    /// common bug, and it is invisible to anyone who does not use the settings.
    ///
    /// Returns `false` when there was no root to publish to, the same as
    /// [`set_capture`](Self::set_capture) — and like it, this does not complain
    /// about that: preferences arriving before the first mount is ordinary.
    pub fn set_accessibility(&mut self, accessibility: vieww_foundation::Accessibility) -> bool {
        if self.accessibility == accessibility {
            return false;
        }
        self.accessibility = accessibility;
        // **Motion is honoured here, not by any widget.**
        //
        // The accessibility module has published `reduce_motion` since it was
        // written and the framework honoured it in none of its own animations,
        // for a structural reason: `Widget::create_state` takes no
        // `BuildContext`, so an `AnimationController` is built where nothing
        // ambient can be read. The registry every framework animation is
        // already advanced through once per frame needs none of that context —
        // it just has to be told, and it settles instead of stepping. See
        // `Tickers::set_reduce_motion` and `Ticker::settle`.
        //
        // Whatever is mid-flight is still animating, so the loop is already
        // awake and the very next frame is the one that lands them.
        self.animations
            .set_reduce_motion(accessibility.reduce_motion);
        // Republishing alone is not enough, and the reason is worth reading
        // before this line is removed as redundant: the reconciler treats a
        // pointer-identical widget as describing an unchanged render object,
        // and `Text` is a `RenderLeaf` that never rebuilds — so the tree comes
        // back byte-identical and every `RenderText` keeps the size it was
        // built with. `Accessibility` is the one thing the factory reads that
        // nothing reads during `build`. See `RenderOwner::rebuild_render_objects`.
        self.republish();
        self.owner.rebuild_render_objects();
        true
    }

    /// Release the derived data this driver holds, on an OS memory warning.
    ///
    /// The platform layer calls this from `onTrimMemory` /
    /// `didReceiveMemoryWarning`; see
    /// [`MemoryPressure`](vieww_foundation::MemoryPressure) for the mapping.
    ///
    /// # What this reaches, and what the caller must still trim itself
    ///
    /// A driver owns the **text** caches and the **retained scene**. It does
    /// *not* own the renderer — a `GpuRenderer` or `CpuRenderer` lives in the
    /// platform layer, above this — so the backend's font, image and atlas
    /// caches are not reachable from here and the platform must trim those
    /// too. Both implement [`Trim`](vieww_foundation::Trim); this is
    /// deliberately not a hidden fan-out to a registry, because a driver that
    /// silently trimmed a renderer it does not own would be reaching across the
    /// one layering boundary this crate exists to keep.
    ///
    /// # Why the retained scene goes with the caches
    ///
    /// [`SceneFlattener`] holds a full command list for the current frame so
    /// the next one can be rebuilt span by span. Under pressure that retention
    /// is exactly the wrong bet: it is proportional to what is on screen, it is
    /// pure derived data, and the cost of losing it is one full flatten. It is
    /// dropped only at [`trims_everything`](vieww_foundation::MemoryPressure::trims_everything),
    /// because that flatten lands on the very next frame.
    ///
    /// Trimming never changes what is drawn — see the [`Trim`] contract — so
    /// this is safe to call at any point, including between frames.
    ///
    /// [`Trim`]: vieww_foundation::Trim
    pub fn trim_memory(&mut self, pressure: vieww_foundation::MemoryPressure) {
        use vieww_foundation::Trim as _;

        if !pressure.trims_unused() {
            return;
        }
        self.owner.tree_mut().fonts_mut().trim(pressure);
        if pressure.trims_everything() {
            // `reset` is already the documented "forget everything retained"
            // call, and its own docs make the point this relies on: the scene
            // is still *correct* afterwards, because the layer tree is the
            // source of truth and this is only its cache. The next frame pays
            // one full flatten.
            self.flattener.reset();
            self.needs_flatten = true;
        }
    }

    /// Publish `theme` above the root, so every widget in the tree reads it.
    ///
    /// # The bug this exists to make impossible
    ///
    /// [`ThemeData::of`](vieww_widget::ThemeData::of) falls back to
    /// [`ThemeData::light`](vieww_widget::ThemeData::light) when no
    /// [`Theme`](vieww_widget::Theme) is above the widget asking. An
    /// application that painted its window dark — with
    /// `App::background(ThemeData::dark().colors.surface)`, which is what the
    /// project template did — and mounted its screen without a `Theme` got a
    /// **dark window** whose widgets were all styled for a **light** one:
    /// near-black body text on a near-black ground, invisible, beside a filled
    /// button that drew its own accent and was therefore the only thing on the
    /// screen.
    ///
    /// It is a particularly bad shape of bug because it survives review: the
    /// studio's preview wraps the screen in a `Theme` of its own, so the
    /// preview was correct and only the built application was wrong — the two
    /// disagreed, which is the one thing a preview exists not to do.
    ///
    /// The cause is two sources of truth. The window's background colour and
    /// the widget tree's theme were set separately and nothing tied them
    /// together, so they could — and did — disagree.
    /// [`App::theme`](../../vieww_platform_winit/struct.App.html#method.theme)
    /// sets both from one value and calls this.
    ///
    /// Published like the view metrics and the accessibility preferences, and
    /// for the same reason: it belongs above the application's tree rather than
    /// inside it, so an application does not have to remember to wrap its own
    /// root.
    pub fn set_theme(&mut self, theme: impl Into<std::rc::Rc<vieww_widget::ThemeData>>) {
        self.theme = Some(theme.into());
        self.republish();
    }

    /// Re-mount the root under the current metrics.
    ///
    /// Only when they actually changed: `set_root` is a full reconciliation, and
    /// running one every frame would defeat every relayout boundary in the tree.
    ///
    /// Returns `false` when there was no root to publish, which
    /// [`set_view_metrics`](Self::set_view_metrics) turns into a complaint.
    fn republish(&mut self) -> bool {
        let Some(root) = self.root.clone() else {
            return false;
        };
        // The application's theme, innermost, so a `Theme` *inside* the tree
        // still wins — a dark section of a light screen is a thing people
        // write, and publishing this outermost would not change that, but
        // keeping it adjacent to the root is what makes the nesting obvious.
        let root = match &self.theme {
            Some(theme) => vieww_widget::Theme::shared(std::rc::Rc::clone(theme), root).into(),
            None => root,
        };
        // Two providers rather than one combined value: they change on
        // wholly different clocks — a rotation is rare and a screenshot is
        // rarer — and a widget reading one must not rebuild for the other.
        self.elements.set_root(Inherited::new(
            self.metrics,
            Inherited::new(
                self.capture,
                Inherited::new(self.file_drag, Inherited::new(self.accessibility, root)),
            ),
        ));
        true
    }

    /// Declare the surface's contents unusable, forcing the next frame to
    /// repaint all of it.
    ///
    /// Needed whenever something outside the framework has scribbled on the
    /// surface or thrown it away: a lost GPU device, a recreated swapchain, a
    /// window coming back from the background.
    pub fn invalidate(&mut self) {
        self.full_repaint = true;
        self.damage = Damage::everything(self.surface);
    }

    /// Hit test the last laid-out frame.
    #[must_use]
    /// The render tree, to read.
    ///
    /// # The last link of the inspector seam
    ///
    /// `RenderTree::describe` and `describe_subtree` answer what is on screen,
    /// and until this existed nothing outside the crate could reach a tree to
    /// ask them: an application holds a `FrameDriver`, the driver holds a
    /// `RenderOwner`, and the owner was private. That is the whole of why the
    /// studio's Inspector was five lines of buffer statistics — not a missing
    /// query, a missing accessor.
    ///
    /// Read-only on purpose. Mutating the render tree from outside the frame
    /// loop is how a layout ends up disagreeing with the elements that produced
    /// it, and nothing that wants to look at the tree needs to change it.
    pub const fn renders(&self) -> &RenderTree {
        self.owner.tree()
    }

    /// Replace the font store, marking every text object for relayout.
    ///
    /// The driver-level route to [`RenderTree::set_fonts`], which is otherwise
    /// unreachable: [`renders`](Self::renders) is deliberately read-only, and
    /// fonts are the one thing outside the frame loop that legitimately has to
    /// change — an application shipping a CJK face has nowhere else to put it.
    pub fn set_fonts(&mut self, fonts: vieww_text::FontStore) {
        self.owner.tree_mut().set_fonts(fonts);
    }

    /// Load the platform's fonts as a fallback behind the embedded faces.
    ///
    /// **What a shipped application should call, and what
    /// `vieww_platform_winit::App` now calls for it.** The default store is
    /// embedded-only — Latin, Hebrew and Arabic — which keeps test metrics
    /// identical on every machine and renders every other script as nothing at
    /// all. See [`FontStore::with_system_fallback`](vieww_text::FontStore::with_system_fallback)
    /// for why the order matters and what it still cannot fix.
    ///
    /// Costs a system font scan, so call it once at start-up rather than per
    /// frame.
    pub fn use_system_fonts(&mut self) {
        self.set_fonts(vieww_text::FontStore::with_system_fallback());
    }

    /// The topmost object under `point`, for an inspector's click-to-pick.
    ///
    /// The driver-level shorthand for `renders().hit_test_identify(..)`, beside
    /// `hit_test` because a caller that wants *the thing you clicked* should
    /// not have to know that the result list is ordered topmost-first.
    #[must_use]
    pub fn hit_test_identify(&self, point: Offset) -> Option<RenderId> {
        self.owner.tree().hit_test_identify(point)
    }

    #[must_use]
    pub fn hit_test(&self, point: Offset) -> HitTestResult {
        self.owner.hit_test(point)
    }

    /// Feed one pointer event in, and deliver whatever gestures come out.
    ///
    /// Returns what was dispatched, so a caller can see the disambiguation rather
    /// than only its side effects.
    ///
    /// Handlers usually set a signal, which marks an element pending; the *next*
    /// frame is what shows the result. Input is not a frame phase and does not
    /// try to be one — a gesture arriving mid-frame would otherwise change the
    /// tree between layout and paint.
    pub fn handle_pointer(&mut self, event: &PointerEvent) -> Vec<Dispatched> {
        // Focus follows the press, before the gesture is routed. Doing it here
        // rather than in a widget's tap handler is deliberate: a tap that lands
        // on a field has to move focus whether or not any recogniser goes on to
        // *win* the arena, and a drag that starts in a field and turns into a
        // scroll has still put the caret there.
        //
        // Only a press. A move or a release changing focus would take the
        // keyboard away mid-drag.
        if event.phase == PointerPhase::Down {
            let hits = self.owner.hit_test(event.position);
            // Scope-aware: with a modal open, a press outside its subtree moves
            // no focus at all. See `FocusManager::reachable_focusable_in`.
            let target = self.focus.reachable_focusable_in(self.owner.tree(), &hits);
            // A press on nothing focusable dismisses focus, which is what makes
            // clicking the background put a caret away — unless a trap is open,
            // where the press was outside it and dismissing would let the modal
            // lose the keyboard to nothing.
            if target.is_some() || !self.focus.is_trapped() {
                self.focus.focus(target);
            }
        }

        let dispatched = self.pointers.handle(self.owner.tree(), event);
        self.deliver(&dispatched);
        dispatched
    }

    // ------------------------------------------------------------------- focus

    /// The object with the keyboard, if anything has it.
    #[must_use]
    pub const fn focused(&self) -> Option<RenderId> {
        self.focus.focused()
    }

    /// The focus manager, for traversal and inspection.
    #[must_use]
    pub const fn focus(&self) -> &FocusManager {
        &self.focus
    }

    /// Give the keyboard to a particular object, or to nothing.
    ///
    /// Returns whether anything changed. A change is visible — a caret appears —
    /// so a `true` here is a reason to ask for a frame.
    pub fn set_focus(&mut self, id: Option<RenderId>) -> bool {
        self.focus.focus(id)
    }

    /// Move focus to the next focusable object, wrapping at the end.
    pub fn focus_next(&mut self, forward: bool) -> bool {
        self.focus.focus_next(self.owner.tree(), forward)
    }

    /// Confine focus to `root`'s subtree until [`pop_focus_scope`] closes it.
    ///
    /// What a modal dialog, an alert or a bottom sheet calls when it opens.
    /// Tab then wraps inside the subtree instead of walking out into the screen
    /// behind, and a press outside it moves no focus.
    ///
    /// [`pop_focus_scope`]: Self::pop_focus_scope
    pub fn push_focus_scope(&mut self, root: RenderId, trapped: bool) {
        self.focus.push_scope(root, trapped);
    }

    /// Close the innermost focus scope, restoring what had focus before it.
    ///
    /// Returns whether focus moved, which is a reason to ask for a frame.
    pub fn pop_focus_scope(&mut self) -> bool {
        self.focus.pop_scope(self.owner.tree())
    }

    /// Feed one key event in.
    ///
    /// Returns whether anything handled it. An unhandled [`NamedKey::Tab`]
    /// moves focus, which is why traversal is here and not in the platform
    /// bridge: whether Tab is a character or a traversal depends on what has
    /// focus, and only this layer knows that.
    ///
    /// Like [`handle_pointer`](Self::handle_pointer), this is not a frame phase.
    /// A handler sets a signal, and the *next* frame shows the result.
    pub fn handle_key(&mut self, event: &KeyEvent) -> bool {
        // A focused object can have been removed by a rebuild since the last
        // key, so this is checked here rather than trusted from when it was set.
        self.focus.prune(self.owner.tree());

        if self.focus.dispatch(self.owner.tree(), event) {
            return true;
        }

        if event.is_down() && event.key == LogicalKey::Named(NamedKey::Tab) {
            return self.focus_next(!event.modifiers.shift());
        }
        false
    }

    /// Feed one input method event in.
    ///
    /// Returns whether anything handled it.
    pub fn handle_ime(&mut self, event: &ImeEvent) -> bool {
        self.focus.prune(self.owner.tree());
        self.focus.dispatch_ime(self.owner.tree(), event)
    }

    /// Whether the focused object wants an input method open on it.
    ///
    /// A platform bridge reads this whenever focus changes: it decides whether
    /// the IME is enabled at all, and on a phone whether the soft keyboard comes
    /// up. Asking for one when nothing accepts text covers half the screen for
    /// no reason; not asking when something does makes the field untypeable.
    #[must_use]
    pub fn accepts_text(&self) -> bool {
        self.focus.accepts_text(self.owner.tree())
    }

    /// The tree as a screen reader sees it.
    ///
    /// Built on demand rather than kept: an accessibility adapter asks for it
    /// when something structural changed, which is far rarer than a frame — a
    /// caret blinking twice a second changes no semantics at all. Keeping one
    /// up to date every frame would be work nobody had asked for on every
    /// machine with no screen reader running, which is most of them.
    #[must_use]
    pub fn semantics(&self) -> SemanticsTree {
        SemanticsTree::build(self.owner.tree(), self.focus.focused())
    }

    /// What a screen reader should say now, and nothing it has already said.
    ///
    /// The consuming half of `Liveness`, **for a backend that has no live
    /// regions of its own**. Such a bridge calls this once per frame — it is
    /// cheap when nothing is live, which is almost always — and speaks whatever
    /// comes back.
    ///
    /// # Whether your bridge wants this at all
    ///
    /// Probably not, and the check is one question: *does the accessibility
    /// backend below you diff the tree itself?*
    ///
    /// AccessKit does. Publishing `Liveness` onto the node it hands the
    /// platform is enough on its own — its adapters emit the announcement when
    /// a live node appears or its name changes, which is this same diff computed
    /// one layer down. A bridge that publishes liveness **and** calls this
    /// speaks every announcement twice. `vieww-platform-winit`'s `a11y::live`
    /// carries the reading that established it, and this repository therefore
    /// has no caller for this method — deliberately.
    ///
    /// What this is for is the other kind of backend: one driving a
    /// text-to-speech engine directly, or a platform whose accessibility API has
    /// no live-region concept. There, the diff has to happen here because
    /// nothing below will do it, and such a bridge must not also publish
    /// liveness to something that would.
    ///
    /// # Why this is a `take` rather than a read
    ///
    /// Because an announcement is a thing that happens once. Leaving it readable
    /// would mean every caller has to remember what it already spoke, which is
    /// the bookkeeping this exists to remove; and two bridges on one driver
    /// double-announcing is a worse failure than one of them missing a message.
    ///
    /// # Why the previous tree is kept here rather than by the caller
    ///
    /// [`semantics`](Self::semantics) is deliberately built on demand and not
    /// retained, so there is nothing for a diff to be against unless something
    /// holds one. This is the one place that has to, and it holds exactly one
    /// tree rather than a log.
    pub fn take_announcements(&mut self) -> Vec<Announcement> {
        let current = self.semantics();
        let announcements = current.announcements(&self.announced);
        self.announced = current;
        announcements
    }

    /// Carry out a screen reader's request against the node it named.
    ///
    /// Returns whether anything did it. The action is routed to `id` if `id`
    /// offers it and to the nearest ancestor that does otherwise — a screen
    /// reader's cursor sits on what it is *reading*, which for "scroll down" is
    /// a row rather than the list around it.
    ///
    /// The semantics tree is rebuilt to answer this rather than kept: an action
    /// arrives when a human presses a key, which is many orders of magnitude
    /// rarer than a frame.
    pub fn handle_semantic_action(&mut self, id: RenderId, action: SemanticAction) -> bool {
        let Some(target) = self.semantics().action_target(id, action) else {
            return false;
        };
        // Down from there until something takes it. A control declares its
        // actions on the annotation that names it, and the object that knows
        // how to be pressed is inside — so "what does activate mean here" is
        // answered by "the first thing under this that knows", which is the only
        // policy that does not need a table of widget types.
        offer(self.owner.tree(), target, action)
    }

    /// Where the focused field's caret is on screen, for placing a candidate
    /// window.
    ///
    /// In the surface's own coordinates. A platform bridge scales it into
    /// physical pixels before handing it to the OS.
    #[must_use]
    pub fn ime_cursor_area(&self) -> Option<Rect> {
        self.focus.ime_cursor_area(self.owner.tree())
    }

    /// Let the recognisers see that time has passed.
    ///
    /// A long press fires from here and nowhere else, so an application that
    /// never calls this has a long press that never happens. The scheduler calls
    /// it as part of [`build`](FrameSink::build).
    pub fn tick_pointers(&mut self, now: std::time::Duration) -> Vec<Dispatched> {
        let dispatched = self.pointers.tick(now);
        self.deliver(&dispatched);
        dispatched
    }

    /// The pointer router, for inspecting live routes.
    #[must_use]
    pub const fn pointers(&self) -> &PointerRouter {
        &self.pointers
    }

    fn deliver(&self, dispatched: &[Dispatched]) {
        let tree = self.owner.tree();
        for entry in dispatched {
            // The tree can have been rebuilt since the pointer went down — a
            // handler on an earlier gesture in this same batch may have done it —
            // so a target that no longer exists is ordinary, not a bug.
            if let Some(object) = tree.object(entry.target) {
                object.handle_gesture(&entry.gesture, entry.local);
            }
        }
    }

    // --------------------------------------------------------------- animation

    /// The registry of explicitly driven animations.
    ///
    /// Usually reached through [`animation`](Self::animation) instead; this is
    /// for attaching something that built its own [`Animation`].
    pub const fn tickers(&mut self) -> &mut Tickers {
        &mut self.animations
    }

    /// Create an animation over `tween` on this driver's runtime, already
    /// attached to its frames.
    ///
    /// The two things that are easy to get wrong when doing it by hand are both
    /// handled here: the signal is created on the runtime of *this* tree, so a
    /// write actually marks something pending, and the animation is registered, so
    /// frames actually advance it.
    pub fn animation<T: Lerp + 'static>(
        &mut self,
        tween: Tween<T>,
        duration: Duration,
    ) -> Animation<T> {
        let animation = Animation::new(self.elements.runtime(), tween, duration);
        animation.attach(&mut self.animations);
        animation
    }

    /// `true` while anything on screen is still moving.
    ///
    /// The signal an event loop uses to keep asking for frames — see
    /// [`drive`](Self::drive), which does it for you. It covers both kinds of
    /// animation: the ones the application drives and the ones widgets started
    /// for themselves.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.animations.is_animating() || self.elements.has_animating_states()
    }

    /// When the next frame is owed, when nothing needs one sooner than that.
    ///
    /// `None` means "as soon as possible" — the caller should draw — and is the
    /// answer whenever anything wanting a frame cannot name a time:
    ///
    /// * a **live pointer** waiting on a gesture deadline, which
    ///   [`needs_frame`](Self::needs_frame) explains;
    /// * an **implicit animation** on an element, which is sampled from a curve
    ///   and therefore differs on every frame;
    /// * an explicit [`Ticker`](vieww_animation::Ticker) that does not override
    ///   `next_deadline`.
    ///
    /// `Some(t)` means every reason for a frame is a ticker that will not change
    /// until `t`, so a loop can sleep until then. The one that matters is a text
    /// caret: it blinks on a schedule, animates for ever, and used to keep an
    /// otherwise idle window's event loop spinning at 100% of a core because
    /// "something is animating" was all it could say.
    ///
    /// Note that this says nothing about whether a frame is *requested* — that
    /// is the scheduler's question, and `event_loop::next_action` asks both.
    #[must_use]
    pub fn frame_deadline(&self, now: Duration) -> Option<Duration> {
        if self.pointers.wants_tick() || self.elements.has_animating_states() {
            return None;
        }
        self.animations.next_deadline(now)
    }

    /// `true` while another frame would change something.
    ///
    /// Animation is the obvious half. The other half is a **live pointer**: a
    /// gesture can be waiting on a deadline rather than on input — a long press
    /// firing, a press becoming visible — and a finger resting on the screen
    /// sends nothing at all, so only a frame can bring
    /// [`tick_pointers`](Self::tick_pointers) forward.
    ///
    /// Leaving that out is why a long press was reachable in every test here and
    /// in no real window: a test draws frames in a row, and an event loop draws
    /// the one frame the `Down` asked for and then goes to sleep. The router
    /// answers precisely, so a finger held for a minute costs the two frames its
    /// deadlines fall on rather than sixty a second.
    ///
    /// # An element marked pending after the build phase is *not* the third half
    ///
    /// It is handled, but not here — [`drive`](Self::drive) asks for that frame
    /// directly, and the reason the answer lives there rather than in this
    /// predicate is the rule this method has to obey. See below.
    ///
    /// Input marks things pending and then asks for a frame itself, so almost
    /// everything is covered before this is consulted. **Layout is the
    /// exception.** [`RenderViewport`](crate::RenderViewport) reports its
    /// extents out of layout, and the handler that receives them writes a
    /// signal — which marks its reader pending at a point where `poll_states` and
    /// `rebuild_pending` are both already behind us, and where nothing else in the
    /// system is going to ask for the frame that would show it.
    ///
    /// The same bug as the long press, one layer up, and it hid the same way: on
    /// a desktop a mouse crossing the window requests a frame and the correction
    /// lands invisibly, so only a phone shows it — as content that is wrong
    /// until you touch it.
    ///
    /// **This makes report-on-change load-bearing for the frame loop, not just
    /// for the rebuild loop.** A handler called from layout that writes
    /// unconditionally used to cost a wasted rebuild whenever something else
    /// asked for a frame; it now costs sixty frames a second, for ever. See
    /// `RenderViewport::report_extents`, which is the only such handler and
    /// already guards. Anything joining it must guard too.
    /// # The rule, and the clause that broke it
    ///
    /// **Every clause here must describe a condition a frame will clear.**
    /// Animation does. A pending pointer deadline does. `pending_count() > 0` does
    /// *not*, and adding it here cost a working window: `about_to_wait` does
    /// `if pending { request_frame() }` and then sets `ControlFlow::Poll`, so a
    /// predicate that stays true until a frame clears it re-requests a frame
    /// every iteration and pins the loop into a spin it cannot leave — **2923
    /// iterations, 2 frames**, measured on a desktop window that rendered once
    /// and never again.
    ///
    /// The distinction is not academic. This is read as a *standing state* by a
    /// loop deciding whether to sleep; asking for one frame is a different
    /// operation, and it belongs where one frame can be asked for. Anything
    /// tempted to add a clause here should check it against the spin above
    /// first: if a frame would not falsify it, it goes in `drive`.
    #[must_use]
    pub fn needs_frame(&self) -> bool {
        self.is_animating() || self.pointers.wants_tick()
    }

    /// `true` when a write has left elements in this tree waiting to rebuild.
    ///
    /// **Deliberately not a clause in [`needs_frame`](Self::needs_frame)**, and
    /// the rule above says why: this is a standing state, so a loop that treats
    /// it as a reason to draw re-requests every iteration and spins. It is here
    /// for the one question a single window could never ask.
    ///
    /// # The multi-window case
    ///
    /// Input marks things pending and then asks for a frame itself — but it asks
    /// for **its own window's** frame. A signal written by a handler in one
    /// window marks readers pending in every tree that shares the
    /// [`Runtime`](vieww_element::Runtime), and nothing asked on their behalf,
    /// so a second window showing the same document would sit stale until
    /// something unrelated woke it. The same shape as the layout write, one tree
    /// over.
    ///
    /// [`wake_if_pending`](crate::event_loop::wake_if_pending) is what a loop
    /// calls with it, once per frame that actually ran, rather than once per
    /// iteration.
    #[must_use]
    pub fn owes_rebuild(&self) -> bool {
        self.elements.pending_count() > 0
    }

    /// Drive one vsync, and ask for another while anything still needs one.
    ///
    /// # Why this is not the scheduler's job
    ///
    /// [`FrameScheduler`] deliberately knows nothing about what a frame
    /// contains — it turns "something changed" into "one frame, at the next
    /// vsync" and stops there. Animation is the one case where the *previous*
    /// frame implies the next one, and this is the seam where that is known:
    /// the driver can see whether anything is still moving, and the scheduler
    /// cannot.
    ///
    /// Without this, an animation runs for exactly one frame and stops — which
    /// looks, from the outside, like the animation working and then the app
    /// freezing.
    ///
    /// # And why the pending request is here rather than in `needs_frame`
    ///
    /// [`FrameScheduler::pulse`] clears `requested` *before* running the phases,
    /// so a phase that marks something pending schedules the next frame instead of
    /// having it swallowed by this one — and that works for every phase except
    /// the two that run after the build. `RenderViewport` reports its extents out
    /// of **layout**, the handler writes a signal, and its reader is left pending
    /// at a point where `poll_states` and `rebuild_pending` are both already
    /// behind us. Nothing else is in a position to notice.
    ///
    /// The distinction that matters, and the one both reverted attempts got
    /// wrong: this request is **bounded**. An event loop draws while anything is
    /// pending and then asks again immediately, so an unbounded condition is a
    /// livelock by construction wherever it is read from —
    /// [`needs_frame`](Self::needs_frame) spun 2923 iterations for 2 frames, and
    /// the same predicate here froze a window at 1. Asking for at most
    /// [`MAX_LAYOUT_PENDING_FRAMES`](Self::MAX_LAYOUT_PENDING_FRAMES) frames in a row
    /// cannot, whatever the tree does: the worst case is that many wasted frames
    /// and one diagnostic. Stale content rather than a spin is the direction
    /// this has to fail in, because a spinning loop is worse than the bug.
    ///
    /// Two guards keep it that narrow. It only fires when a frame **actually
    /// ran** — `pulse` returning `None` means nothing was requested and nothing
    /// happened, and an idle driver holding a stale pending element must not wake
    /// the display. And `report_extents` reporting only *changes* is what makes
    /// the common case *one* extra frame rather than the budget: an
    /// unconditional handler called from layout would spend the whole streak
    /// every time and then give up, which is a diagnostic pointing straight at
    /// it. That obligation is documented at both ends, and it is load-bearing
    /// here too.
    ///
    /// Termination is proved without a window by
    /// [`LoopHarness::settle`](crate::event_loop::LoopHarness::settle), which
    /// runs this decision against the real `next_action` under a bound. Both
    /// reverted fixes passed the whole suite; neither could have passed that.
    pub fn drive<C>(
        &mut self,
        scheduler: &mut FrameScheduler,
        now: Duration,
        elapsed: C,
    ) -> Option<FrameStats>
    where
        C: FnMut() -> Duration,
    {
        let stats = scheduler.pulse(now, self, elapsed);
        if self.needs_frame() {
            scheduler.request_frame();
        }

        // **After `pulse`, and that is not the obvious choice.** `self.damage`
        // is *assigned* inside `draw_frame` — which `pulse` runs — and then
        // left alone until the next frame overwrites it. So before the pulse it
        // still describes the *previous* frame, and on the very first one it
        // describes nothing at all. The first version of this read it before
        // and reported zero damage for the first frame of every application.
        //
        // Annotated here rather than inside the scheduler, which is
        // deliberately ignorant of damage — see `FrameStats::damage_area`.
        let stats = stats.map(|mut stats| {
            let area = self.damage.covered_area();
            // `repaint_regions`, not `regions`: the latter is deliberately
            // **empty** for a full repaint — the flag is stored instead of a
            // rectangle — so counting it would report "0 regions" for the
            // frame that repaints the entire screen, which is precisely
            // backwards. `covered_area` already handles the same case
            // internally, which is why only one of the two numbers was wrong.
            let regions = self.damage.repaint_regions().len();
            stats.damage_area = area;
            stats.damage_regions = regions;
            scheduler.annotate_last_damage(area, regions);
            stats
        });

        // **The pending request, bounded.**
        //
        // `RenderViewport` reports its extents out of *layout*, which writes a
        // signal at a point where `poll_states` and `rebuild_pending` are both
        // already behind us. Nothing else is in a position to notice, so without
        // this the element stays pending and the loop sleeps on top of it —
        // measured on `examples/grid.rs` as `pending=1` at 7.181s with no frame
        // until an unrelated wheel notch 48 seconds later, with `Swatches#10v0`
        // named as the element by the trace below.
        //
        // # Why a bounded streak and not `pending > 0`, nor a rising edge
        //
        // The two reverted attempts both asked "is anything pending" with no
        // bound, and `next_action` turns an unbounded condition into a livelock
        // by construction: it draws while anything is pending and asks again
        // immediately, so a condition the frame does not clear re-decides `Draw`
        // for ever. One put it in `needs_frame` and pinned the loop at 2923
        // iterations for 2 frames; the other put it here and froze a window at 1
        // presented frame.
        //
        // The third shipped a **rising edge** — `pending > 0 && was == 0` — which
        // terminates, and misses a case. `rebuild_pending` drains the whole pending
        // set every frame, so pending work never carries across a frame boundary: a
        // non-zero count *is* "layout wrote a signal during this frame", a
        // per-frame event rather than a standing state. A **chain** therefore
        // reads as a plateau. Frame 1 marks `A` pending; frame 2 rebuilds `A` and its
        // layout marks `B` pending, because `A`'s new size is what an outer viewport
        // measures. The count is 1 both times, no edge is detected, and `B` is
        // never rebuilt. Nested viewports are where that lives.
        //
        // Since the condition is a per-frame event, it can simply be **counted**.
        // Ask for the next frame while layout keeps writing, and stop after
        // [`MAX_LAYOUT_PENDING_FRAMES`](Self::MAX_LAYOUT_PENDING_FRAMES) consecutive
        // frames of it. That terminates without any claim about the tree at all:
        // the worst case is a fixed, small number of wasted frames and one
        // diagnostic, where the unbounded version's worst case is a pegged core.
        // It is the discipline `ElementTree::rebuild_pending` already applies to
        // the identical feedback shape one phase down, where a build writing a
        // signal it reads is bounded by `MAX_REBUILD_PASSES` and then fails
        // loudly rather than hanging.
        //
        // # Only when a frame actually ran
        //
        // `stats` is `None` when nothing was requested and nothing happened. An
        // idle driver holding pending work from before must not wake the display on an
        // iteration that did no work — that is how a sleeping loop becomes a
        // polling one without any single line looking wrong.
        if stats.is_some() {
            if self.elements.pending_count() > 0 {
                self.layout_pending_streak += 1;
                if self.layout_pending_streak <= Self::MAX_LAYOUT_PENDING_FRAMES {
                    scheduler.request_frame();
                } else if self.layout_pending_streak == Self::MAX_LAYOUT_PENDING_FRAMES + 1 {
                    self.report_unsettled_layout();
                }
            } else {
                // The streak is consecutive frames, so any clean frame ends it —
                // including the clean frame that ends a run which gave up. An
                // application that oscillated once is not held in the degraded
                // state afterwards.
                self.layout_pending_streak = 0;
            }
        }
        // # What the two reverted attempts actually hit, corrected by a trace
        //
        // The second revert concluded that *something in the scrolling path
        // leaves an element pending every frame*, and set finding it as the
        // precondition for this attempt. **That is not what happens.** With the
        // trace naming pending elements, `examples/grid.rs` scrolled through 204
        // wheel events reporting `pending=0` for every one of them: nothing in the
        // scroll path marks anything pending again. The only pending work in a 12-second session
        // was one element at startup, held for 1.8 seconds because the loop went
        // to sleep on top of it.
        //
        // What both attempts really hit was the event loop, not the predicate.
        // They were measured before `about_to_wait` was fixed: it asked the
        // window for a `RedrawRequested` and drew when one arrived, and
        // `request_redraw` no-ops while one is pending — so a dropped redraw
        // stranded the request and *no frame ever ran to clear the condition*.
        // Any pending flag stayed pending, and the loop polled forever. That is
        // the spin, and it was a lost redraw wearing a predicate's clothes.
        //
        // The streak above is safe whether or not that reading is complete,
        // which is the point of bounding it rather than reasoning about it — and
        // `LoopHarness::settle` proves termination without a window, which is
        // what no previous attempt had.
        //
        // See `VIEWW_TRACE_FRAMES` below for the instrument that answers it.
        self.trace(now, stats.is_some());
        stats
    }

    /// The pending elements as sorted `Widget#id` names.
    ///
    /// Sorted because the pending set is a `HashSet`: without it the same standing
    /// element reorders between calls and reads as a changing one, which is the
    /// exact distinction both readers below exist to draw. Allocating, so it is
    /// called only when something is already wrong.
    fn pending_names(&self) -> String {
        let mut names: Vec<String> = self
            .elements
            .pending_widgets()
            .into_iter()
            // `ElementId`'s `Display` supplies its own `#`, so this is
            // `Scrollable#3v1` rather than `Scrollable##3v1`.
            .map(|(id, name)| format!("{name}{id}"))
            .collect();
        names.sort_unstable();
        names.join(", ")
    }

    /// Say once that layout is not settling, and name what is still pending.
    ///
    /// # Why this is not an assert
    ///
    /// `ElementTree::rebuild_pending` panics when *its* budget is exhausted, and
    /// that is right there: a build writing a signal it reads is a contract
    /// violation with a stack to point at, and it is reachable from a test.
    /// This is not the same thing. The equivalent here is a *layout* whose
    /// handler reports something that does not converge, and the frame that
    /// exhausts the budget is one the user is looking at. Killing a shipped
    /// application over a stale row would be a worse outcome than the stale row,
    /// and — unlike a build cycle — the app keeps running correctly in every
    /// respect but that content.
    ///
    /// Unconditional rather than behind `VIEWW_TRACE_FRAMES`, because this only
    /// prints when something is already broken, and the person who needs it has
    /// no reason to have set an environment variable first. Once per streak: it
    /// is a state that lasts until a clean frame, and repeating it every frame
    /// would be a write syscall per frame in exactly the situation where the
    /// loop is already struggling.
    fn report_unsettled_layout(&self) {
        eprintln!(
            "vieww: layout marked an element pending for {} frames running without \
             settling, so the driver has stopped asking for more. The content \
             below is stale. Still pending: {}. A handler called from layout must \
             report only *changes* — see RenderViewport::report_extents.",
            Self::MAX_LAYOUT_PENDING_FRAMES,
            self.pending_names(),
        );
    }

    /// Print what the driver decided this iteration, when `VIEWW_TRACE_FRAMES`
    /// is set in the environment.
    ///
    /// # Why this exists at all
    ///
    /// Two fixes for the same bug were written, verified against the whole suite
    /// and reverted, and both times the diagnosis came from reading the source.
    /// There was no way to ask a *running window* what the driver was doing —
    /// `FrameLog` counts presented frames, and every failure here is an iteration
    /// that presents nothing, so the one instrument in the tree is blind to
    /// exactly this class of fault by construction.
    ///
    /// It prints per **iteration**, not per presented frame, because the
    /// difference between those two numbers is the whole subject: a window that
    /// looks frozen while pegging a core is `ran=false` or an undamaged
    /// `ran=true` repeating without end.
    ///
    /// `stderr` and not the `FrameLog`, deliberately — this has to work when the
    /// loop is too busy to present, which is when a report gathered for the end
    /// of the run never arrives.
    /// The timestamp leads every line, because the fault being chased is a
    /// **gap** — 5.7 seconds between two frames — and a log with no clock cannot
    /// say where one is.
    /// # Only on a change, and that is not an optimisation
    ///
    /// The first version of this printed every iteration. `stderr` is unbuffered,
    /// so that is a write syscall per iteration of a `ControlFlow::Poll` loop —
    /// and it took `examples/grid.rs` from 80 presented frames and working
    /// scrolling to **one frame and an unresponsive window**. The instrument
    /// became the fault.
    ///
    /// So the state is compared against the last line and printed only when it
    /// differs. The whole 7-second session that produced 82 iterations' worth of
    /// noise is a handful of lines this way, and none of the questions it was
    /// built to answer needed the repeats: a gap is found from the *timestamps of
    /// the transitions either side of it*, not from counting identical lines.
    fn trace(&self, now: Duration, ran: bool) {
        use std::cell::RefCell;
        use std::sync::OnceLock;

        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| std::env::var_os("VIEWW_TRACE_FRAMES").is_some()) {
            return;
        }

        let state = (
            ran,
            self.elements.pending_count(),
            self.is_animating(),
            self.pointers.wants_tick(),
        );

        // **Which elements, not how many** — and part of the compared state, not
        // an extra line under it.
        //
        // The count is all the last two `needs_frame` attempts had, and it
        // cannot answer the question the second revert leaves as the
        // precondition for a third: *something in the scrolling path leaves an
        // element pending every frame*. A single element standing pending and a
        // different element going pending each frame produce the same `pending=1` for
        // ever, and they want opposite fixes — one is a handler that should
        // guard, the other is a rebuild cycle. Comparing the **names** is what
        // tells them apart, and comparing only the count would print the first
        // frame of a rotating set and then go quiet, which is worse than not
        // printing at all because it looks settled.
        //
        // Computed only while something is pending. It allocates, and an
        // allocation per iteration of a `ControlFlow::Poll` loop is how this
        // instrument once became the fault it was built to find — so a settled
        // window pays nothing, and a spinning one pays a `Vec` per iteration to
        // report the spin it is already in. That trade is the right way round.
        let pending_names = (state.1 > 0).then(|| self.pending_names());

        type Line = ((bool, usize, bool, bool), Option<String>);
        thread_local! {
            static LAST: RefCell<Option<Line>> = const { RefCell::new(None) };
        }
        // A run of identical states is one line plus its duration, which the next
        // transition's timestamp gives for free.
        let line = (state, pending_names);
        if LAST.with_borrow(|last| last.as_ref() == Some(&line)) {
            return;
        }
        LAST.with_borrow_mut(|last| *last = Some(line.clone()));

        let ((ran, pending, animating, wants_tick), pending_names) = line;
        eprintln!(
            "{:>8.3} ran={ran} pending={pending} animating={animating} wants_tick={wants_tick}",
            now.as_secs_f64(),
        );
        if let Some(names) = pending_names {
            eprintln!("         pending: {names}");
        }
    }

    /// Run one frame directly, without a scheduler.
    ///
    /// For tests and for one-shot rendering. An application should go through a
    /// [`FrameScheduler`] instead, so that many changes coalesce into one frame.
    ///
    /// `now` is the timestamp the frame is drawn for, and animations are sampled
    /// at it: `draw_frame(Duration::ZERO)` twice runs two frames' worth of
    /// pipeline and zero milliseconds' worth of animation, which is exactly what
    /// a test asserting about layout wants and never what one asserting about
    /// motion does.
    pub fn draw_frame_at(&mut self, now: Duration) {
        self.invalidation = InvalidationKind::None;
        let frame = FrameInfo {
            number: 0,
            timestamp: now,
            delta: Duration::ZERO,
        };
        FrameSink::animate(self, &frame);
        self.build(&frame);
        self.layout(&frame);
        self.paint(&frame);
        self.composite(&frame);
    }

    /// Run one frame directly at time zero.
    pub fn draw_frame(&mut self) {
        self.draw_frame_at(Duration::ZERO);
    }
}

impl FrameSink for FrameDriver {
    fn animate(&mut self, frame: &FrameInfo) {
        self.invalidation = InvalidationKind::None;
        // Both registries, and both before the build: an animation's new value
        // is picked up by *this* frame's rebuild rather than trailing it by one,
        // which is the whole reason animate is the first phase.
        //
        // Neither return value is kept. Whether another frame is worth producing
        // is asked at the end of the frame instead, through `is_animating`,
        // because a build can start an animation that the animate phase has not
        // seen yet — that is exactly what an implicit animation noticing a new
        // target *is*.
        if self.animations.advance(frame.timestamp) {
            self.invalidation.upgrade(InvalidationKind::State);
        }
        if self.elements.tick_states(frame.timestamp) {
            self.invalidation.upgrade(InvalidationKind::State);
        }
    }

    fn build(&mut self, frame: &FrameInfo) {
        // Before the rebuild, so a long press firing this frame is one of the
        // changes the rebuild picks up rather than one it misses by a frame.
        let _ = self.tick_pointers(frame.timestamp);
        // And between the two, because that is where a gesture handler's writes
        // are: a press reported by the tick above has just set a flag on some
        // widget's state, and this is what turns that into a rebuild. Polling
        // before the tick would show every press one frame late.
        let marked = self.elements.poll_states();
        if marked > 0 || self.elements.pending_count() > 0 {
            self.invalidation.upgrade(InvalidationKind::State);
        }
        self.elements.rebuild_pending();
        self.owner.sync(&self.elements);
        // The sync can have removed whatever had the keyboard. Dropping it here
        // rather than on the next keypress means `focused()` never reports an id
        // that is no longer in the tree, which anything drawing a focus ring
        // would otherwise read.
        self.focus.prune(self.owner.tree());
    }

    fn layout(&mut self, _frame: &FrameInfo) {
        let before = self.owner.tree().layout_runs();
        // Guarded. See `FrameError`: a `layout` that divides by a zero-height
        // constraint used to take the whole process down, taking every unsaved
        // buffer in an editor built on this with it.
        let (owner, constraints) = (&mut self.owner, self.constraints);
        match guarded(FramePhase::Layout, move || {
            owner.tree_mut().layout_root(constraints)
        }) {
            Ok(size) => self.size = size,
            Err(error) => {
                self.frame_errors.push(error);
                return;
            }
        }
        if self.owner.tree().layout_runs() > before {
            self.invalidation.upgrade(InvalidationKind::Layout);
        }

        // **Settle what layout discovered inside this frame, not the next one.**
        //
        // Layout is the only phase that learns something the build could not
        // know: how much room a subtree was actually given. A `LayoutBuilder`
        // chooses its tree from that, and `RenderViewport` reports its extents
        // from it. Both used to land one frame late — the reader was marked
        // after the build that would have read it, so the frame that went to the
        // screen was built against the previous answer.
        //
        // One frame of the wrong tree is visible on a rotation and continuous
        // during a window drag, and the classic design does not have it: `LayoutBuilder`
        // there re-enters the build pipeline from inside layout, through
        // `invokeLayoutCallback`. A render object here cannot do that — it holds
        // `&mut RenderTree` and has no handle on the element tree, and DESIGN §7
        // keeps that direction closed on purpose.
        //
        // It does not need to. **This** is the level where both trees are owned,
        // so the loop belongs here rather than inside a render object: rebuild
        // and re-lay out until quiet, then paint once. No re-entrancy, no
        // aliasing, and no debug-only guard of the kind a re-entrant design needs because it
        // really does re-enter.
        //
        // Bounded by the same constant and for the same reason as the
        // cross-frame version it replaces — see
        // [`MAX_LAYOUT_PENDING_FRAMES`](Self::MAX_LAYOUT_PENDING_FRAMES). The
        // streak counter in [`drive`](Self::drive) stays as the backstop for a
        // tree that does not converge even here.
        for _ in 0..Self::MAX_LAYOUT_PENDING_FRAMES {
            // **Both channels, or the loop half works.** A signal written from
            // layout marks its readers as it is written, so `pending_count`
            // already sees it. An `ElementState` flag — which is how
            // `LayoutBuilder` reports — is inert until something polls it.
            // Asking `pending_count` without polling first would settle
            // viewports and leave layout-builders trailing by a frame, which is
            // the bug this loop exists to remove, still present in half the
            // cases and therefore much harder to see.
            self.elements.poll_states();
            if self.elements.pending_count() == 0 {
                return;
            }

            self.elements.rebuild_pending();
            self.owner.sync(&self.elements);
            // For the reason the build phase gives: the sync can have removed
            // whatever held the keyboard.
            self.focus.prune(self.owner.tree());
            self.size = self.owner.tree_mut().layout_root(self.constraints);
        }
        if self.owner.tree().layout_runs() > before {
            self.invalidation.upgrade(InvalidationKind::Layout);
        }
    }

    fn paint(&mut self, _frame: &FrameInfo) {
        if !self.layers.is_clean() {
            self.invalidation.upgrade(InvalidationKind::Paint);
        }
        // Guarded for the same reason layout is, and with the same consequence:
        // a half-recorded layer tree is not composited, so what stays on screen
        // is the last frame that finished.
        //
        // Read out before the closure, which moves both borrows: the ratio the
        // leaves snap to is the surface's, and `self.metrics` is where the
        // platform published it.
        let dpr = self.metrics.device_pixel_ratio;
        let (owner, layers) = (&mut self.owner, &mut self.layers);
        if let Err(error) = guarded(FramePhase::Paint, move || {
            owner.tree_mut().paint_layers_with_ratio(layers, dpr);
        }) {
            self.frame_errors.push(error);
        }
    }

    fn composite(&mut self, _frame: &FrameInfo) {
        // Damage comes from the layers that were re-recorded, moved or removed —
        // each contributing where it sat at the end of the last frame *and* where
        // it sits now, the first to erase and the second to draw. It has to be
        // read before `end_frame` settles them.
        //
        // On the first frame, after a resize, or after the surface was
        // invalidated, the surface does not hold what the last frame drew, and a
        // report of what changed *since* it is the wrong question: the untouched
        // pixels would keep whatever was already there.
        let full_repaint = std::mem::take(&mut self.full_repaint);
        if full_repaint {
            self.invalidation.upgrade(InvalidationKind::Full);
        }
        self.damage = if full_repaint {
            Damage::everything(self.surface)
        } else {
            self.layers.damage(self.surface)
        };

        // A requested frame does not necessarily mean the visual layer tree
        // changed. State, semantics, focus, input, or another pipeline phase
        // can legitimately ask for a frame after paint has already settled.
        // Flattening the same retained layers again would turn an O(1) idle
        // visual update into an O(scene) walk. Keep the previous Scene exactly
        // as it is when the layer tree is clean.
        // `full_repaint` does not force a re-flatten: the scene describes the
        // layer tree, and the layer tree has not changed. What a full repaint
        // invalidates is the *surface*, which `self.damage` above already says.
        // Rebuilding the same commands to send them to a different destination
        // was the one case where an untouched tree still cost a full walk.
        // `needs_flatten` is a *second* reason to flatten, and it exists
        // because "the layer tree changed" and "the retained scene is still
        // there" are two different facts. Everything above reasons about the
        // first; `trim_memory` invalidates the second, by throwing the scene
        // away while the tree stays perfectly clean. Without this flag that
        // combination flattens never and draws a blank frame — which is what
        // `tests/memory_pressure.rs` caught.
        if !self.layers.is_clean() || self.needs_flatten {
            self.flattener.flatten(&self.layers);
            self.scene_rebuilds += 1;
            self.needs_flatten = false;
        }
        self.layers.end_frame();

        // Compute damage cull stats so `damage_cull_stats()` has a fresh
        // answer after every composite, without an allocation on the hot
        // path — `Scene::damage_cull` returns a new Scene, which we don't
        // need here. Walk the retained scene's bounds against the damage
        // instead.
        if !self.damage.is_clean() {
            let damage_bounds = self.damage.bounds();
            let total = self.flattener.scene().len();
            let copied = self
                .flattener
                .scene()
                .commands()
                .iter()
                .filter(|c| c.bounds().overlaps(damage_bounds))
                .count();
            self.damage_cull = DamageCullStats {
                copied_commands: copied,
                skipped_commands: total - copied,
            };
        } else {
            self.damage_cull = DamageCullStats::default();
        }
    }
}

/// Offer `action` to `id` and then to its subtree, innermost last.
///
/// Depth-first and parent-first: the outermost object that can do the thing
/// wins, so a row inside a button does not steal the button's activation.
fn offer(tree: &crate::RenderTree, id: RenderId, action: SemanticAction) -> bool {
    if let Some(object) = tree.object(id) {
        if object.handle_semantic_action(action, tree.size(id)) {
            return true;
        }
    }
    tree.children(id)
        .iter()
        .any(|&child| offer(tree, child, action))
}

#[cfg(test)]
mod tests {
    use vieww_animation::Tween;
    use vieww_foundation::Color;
    use vieww_paint::{FramePhase, FrameScheduler};
    use vieww_widget::prelude::*;

    use super::*;

    fn ms(millis: u64) -> std::time::Duration {
        std::time::Duration::from_millis(millis)
    }

    fn driver() -> FrameDriver {
        FrameDriver::new(Size::new(200.0, 200.0))
    }

    /// A widget that records the theme its build sees.
    #[derive(Debug)]
    struct ThemeProbe(std::rc::Rc<std::cell::RefCell<Option<Color>>>);

    impl Widget for ThemeProbe {
        fn debug_name(&self) -> &'static str {
            "ThemeProbe"
        }

        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }

        fn build(&self, ctx: &BuildContext) -> WidgetNode {
            *self.0.borrow_mut() = Some(vieww_widget::ThemeData::of(ctx).colors.surface);
            Container::new().into()
        }
    }

    vieww_widget::widget_node_from!(ThemeProbe);

    /// A widget mounted without a `Theme` of its own reads the one the
    /// application set.
    ///
    /// # The bug
    ///
    /// `ThemeData::of` falls back to `light()` when nothing above it published
    /// a theme. An application that painted its window with
    /// `ThemeData::dark().colors.surface` and mounted its screen bare therefore
    /// got a dark window full of light-themed widgets: body text in near-black
    /// on near-black, invisible, next to a filled button drawing its own accent
    /// and looking like the only thing on the screen. That is exactly what a
    /// scaffolded project did, and vieww Studio's preview — which wraps the
    /// screen in a `Theme` of its own — showed it correctly the whole time.
    #[test]
    fn the_applications_theme_reaches_a_widget_that_did_not_mount_one() {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
        let mut driver = driver();
        let dark = vieww_widget::ThemeData::dark();
        let surface = dark.colors.surface;

        driver.set_theme(dark);
        driver.set_root(ThemeProbe(std::rc::Rc::clone(&seen)));
        driver.draw_frame();

        assert_eq!(
            *seen.borrow(),
            Some(surface),
            "the widget saw the light fallback, not the application's theme"
        );
    }

    /// A `Theme` inside the tree still wins, so a dark section of a light
    /// screen keeps working.
    #[test]
    fn a_theme_inside_the_tree_overrides_the_applications() {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
        let mut driver = driver();
        let inner = vieww_widget::ThemeData::light();
        let inner_surface = inner.colors.surface;

        driver.set_theme(vieww_widget::ThemeData::dark());
        driver
            .set_root(vieww_widget::Theme::new(inner).child(ThemeProbe(std::rc::Rc::clone(&seen))));
        driver.draw_frame();

        assert_eq!(*seen.borrow(), Some(inner_surface));
    }

    #[test]
    fn a_frame_lays_out_and_paints_a_mounted_tree() {
        let mut driver = driver();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(50.0)));

        driver.draw_frame();

        assert_eq!(
            driver.size(),
            Size::new(200.0, 200.0),
            "tight to the surface"
        );
        let fills = driver.scene().fills();
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].1.color, Color::RED);
    }

    #[test]
    fn the_first_frame_damages_the_whole_surface() {
        let driver = driver();
        assert!(
            driver.damage().is_everything(),
            "there are no previous contents to keep"
        );
    }

    /// A tree that paints a small part of the surface and leaves the rest alone.
    ///
    /// The distinction matters: a root that happens to fill the surface pushes
    /// damage past the repaint-everything threshold by accident, which would hide
    /// a driver that failed to force a full repaint.
    fn small_tree() -> impl Into<vieww_widget::WidgetNode> {
        Center::new().child(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)))
    }

    #[test]
    fn the_first_painted_frame_still_damages_the_whole_surface() {
        let mut driver = driver();
        driver.elements().set_root(small_tree());

        driver.draw_frame();

        assert!(
            driver.damage().is_everything(),
            "the surface has never been painted, so a diff against nothing is not \
             enough — the untouched 99% would keep whatever garbage was there: {}",
            driver.damage()
        );
    }

    #[test]
    fn the_frame_after_a_resize_damages_the_whole_new_surface() {
        let mut driver = driver();
        driver.elements().set_root(small_tree());
        driver.draw_frame();
        driver.draw_frame(); // settle, so damage would otherwise be clean

        driver.resize(Size::new(400.0, 400.0));
        driver.draw_frame();

        assert!(
            driver.damage().is_everything(),
            "a resized surface holds nothing worth keeping: {}",
            driver.damage()
        );
        assert_eq!(driver.damage().surface(), driver.surface());
    }

    #[test]
    fn invalidating_forces_one_full_repaint_and_then_stops() {
        let mut driver = driver();
        driver.elements().set_root(small_tree());
        driver.draw_frame();
        driver.draw_frame();

        driver.invalidate();
        driver.draw_frame();
        assert!(
            driver.damage().is_everything(),
            "the frame after invalidate"
        );

        driver.draw_frame();
        assert!(
            driver.damage().is_clean(),
            "and then back to diffing — a full repaint every frame would defeat \
             the point of tracking damage: {}",
            driver.damage()
        );
    }

    #[test]
    fn invalidation_classification_is_conservative_and_frame_local() {
        let mut driver = driver();
        driver.elements().set_root(small_tree());

        driver.draw_frame();
        assert_eq!(driver.invalidation(), InvalidationKind::Full);

        driver.draw_frame();
        assert_eq!(driver.invalidation(), InvalidationKind::None);

        driver
            .elements()
            .set_root(ColoredBox::new(Color::BLUE).child(SizedBox::square(20.0)));
        driver.draw_frame();
        assert_eq!(driver.invalidation(), InvalidationKind::Layout);
    }

    #[test]
    fn an_unchanged_frame_produces_no_damage() {
        let mut driver = driver();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(50.0)));

        driver.draw_frame();
        driver.draw_frame();

        assert!(
            driver.damage().is_clean(),
            "nothing changed, so nothing should be repainted: {}",
            driver.damage()
        );
    }

    #[test]
    fn an_unchanged_frame_reuses_the_previous_scene() {
        let mut driver = driver();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(50.0)));

        driver.draw_frame();
        assert_eq!(driver.scene_rebuilds(), 1);

        driver.draw_frame();

        assert_eq!(
            driver.scene_rebuilds(),
            1,
            "a clean layer tree must not be flattened again"
        );
        assert!(driver.damage().is_clean());
    }

    #[test]
    fn a_colour_change_damages_the_frame_without_relaying_out() {
        let mut driver = driver();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(50.0)));
        driver.draw_frame();

        let layouts = driver.owner().tree().total_layouts();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::BLUE).child(SizedBox::square(50.0)));
        driver.draw_frame();

        assert_eq!(
            driver.owner().tree().total_layouts(),
            layouts,
            "colour is a paint property"
        );
        assert_eq!(driver.scene().fills()[0].1.color, Color::BLUE);
        assert!(!driver.damage().is_clean(), "but the pixels did change");
    }

    #[test]
    fn a_resize_invalidates_the_surface_and_relayouts() {
        let mut driver = driver();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(50.0)));
        driver.draw_frame();

        driver.resize(Size::new(400.0, 100.0));
        assert!(driver.damage().is_everything());

        driver.draw_frame();
        assert_eq!(driver.size(), Size::new(400.0, 100.0));
    }

    #[test]
    fn a_scheduler_drives_the_driver_through_a_whole_frame() {
        let mut scheduler = FrameScheduler::sixty_hz();
        let mut driver = driver();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(50.0)));

        assert!(
            scheduler.pulse(ms(0), &mut driver, || ms(0)).is_none(),
            "no frame until one is asked for"
        );

        scheduler.request_frame();
        let stats = scheduler
            .pulse(ms(16), &mut driver, || ms(0))
            .expect("a frame was requested");

        assert_eq!(stats.number, 1);
        assert_eq!(scheduler.phase(), FramePhase::Idle);
        assert_eq!(
            driver.scene().fills()[0].1.color,
            Color::RED,
            "the frame reached the scene through the scheduler"
        );
    }

    /// A composed widget whose colour comes from a signal, so that setting the
    /// signal rebuilds *only this element* — which is what lets a test tell a
    /// boundary doing its job from a rebuild that happened to touch one subtree.
    #[derive(Debug)]
    struct Swatch {
        color: vieww_element::Signal<Color>,
        size: f32,
    }

    impl Widget for Swatch {
        fn debug_name(&self) -> &'static str {
            "Swatch"
        }

        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }

        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            ColoredBox::new(self.color.get())
                .child(SizedBox::square(self.size))
                .into()
        }
    }

    vieww_widget::widget_node_from!(Swatch);

    /// How many times each layer has been re-recorded, root first, then its
    /// children in composite order.
    fn paint_counts(driver: &FrameDriver) -> Vec<u32> {
        let layers = driver.layers();
        let mut counts = Vec::new();
        if let Some(root) = layers.root() {
            collect_counts(layers, root, &mut counts);
        }
        counts
    }

    fn collect_counts(
        layers: &vieww_paint::LayerTree,
        id: vieww_paint::LayerId,
        out: &mut Vec<u32>,
    ) {
        out.push(layers.layer(id).paint_count());
        for &child in layers.layer(id).children() {
            collect_counts(layers, child, out);
        }
    }

    /// A static band above a small signal-driven swatch behind a boundary.
    fn layered_tree(color: &vieww_element::Signal<Color>) -> WidgetNode {
        Flex::column()
            .children(children![
                ColoredBox::new(Color::RED).child(SizedBox::from_size(Size::new(200.0, 120.0))),
                RepaintBoundary::new().child(Swatch {
                    color: color.clone(),
                    size: 20.0,
                }),
            ])
            .into()
    }

    #[test]
    fn a_repaint_boundary_gets_its_own_layer() {
        let mut driver = driver();
        let color = driver.elements().runtime().signal(Color::BLUE);
        driver.elements().set_root(layered_tree(&color));
        driver.draw_frame();

        assert_eq!(
            driver.layers().len(),
            2,
            "the root owns one, the boundary the other"
        );
        assert_eq!(
            driver.scene().fills().len(),
            2,
            "compositing puts both layers' content in the frame: {}",
            driver.layers().len()
        );
    }

    #[test]
    fn a_change_behind_a_boundary_repaints_only_that_layer() {
        let mut driver = driver();
        let color = driver.elements().runtime().signal(Color::BLUE);
        driver.elements().set_root(layered_tree(&color));
        driver.draw_frame();
        driver.draw_frame();

        let before = paint_counts(&driver);
        color.set(Color::GREEN);
        driver.draw_frame();
        let after = paint_counts(&driver);

        assert_eq!(
            after[0], before[0],
            "the root layer must not re-record for a change below a boundary — \
             that absence is the entire point of the boundary"
        );
        assert_eq!(after[1], before[1] + 1, "the boundary's layer must");
        assert_eq!(
            driver
                .scene()
                .fills()
                .iter()
                .filter(|(_, paint)| paint.color == Color::GREEN)
                .count(),
            1,
            "and the new colour still reaches the frame, from the reused composite"
        );
    }

    #[test]
    fn damage_is_proportional_to_the_change_not_to_the_screen() {
        let mut driver = driver();
        let color = driver.elements().runtime().signal(Color::BLUE);
        driver.elements().set_root(layered_tree(&color));
        driver.draw_frame();
        driver.draw_frame();

        color.set(Color::GREEN);
        driver.draw_frame();

        let damage = driver.damage();
        let surface = driver.surface().area();
        // The swatch is 20x20 on a 200x200 surface: 1% of it, plus the
        // antialiasing bleed on each edge.
        assert!(
            !damage.is_everything() && damage.covered_area() < surface * 0.05,
            "a 20x20 change on a 200x200 surface damaged {:.1}% of it: {damage}",
            100.0 * damage.covered_area() / surface
        );
        // A column fills its cross axis and centres in it, so the 20-wide swatch
        // sits in the middle of the 200-wide surface, under the 120-tall band.
        assert!(
            damage.intersects(Rect::new(90.0, 120.0, 110.0, 140.0)),
            "and it has to cover where the swatch actually is: {damage}"
        );
    }

    #[test]
    fn removing_a_boundary_removes_its_layer_and_damages_what_it_drew() {
        let mut driver = driver();
        let color = driver.elements().runtime().signal(Color::BLUE);
        driver.elements().set_root(layered_tree(&color));
        driver.draw_frame();
        driver.draw_frame();
        assert_eq!(driver.layers().len(), 2);

        driver.elements().set_root(
            ColoredBox::new(Color::RED).child(SizedBox::from_size(Size::new(200.0, 120.0))),
        );
        driver.draw_frame();

        assert_eq!(driver.layers().len(), 1, "the boundary's layer is gone");
        assert!(
            driver
                .damage()
                .intersects(Rect::new(0.0, 120.0, 20.0, 140.0)),
            "a deleted subtree stays on screen until something paints over it: {}",
            driver.damage()
        );
    }

    /// A composed widget that is nothing but a signal-driven vertical gap.
    ///
    /// It exists so a test can move a *sibling* without rebuilding it. Putting
    /// the signal on an ancestor instead would rebuild the whole subtree — a
    /// composed widget's `build` constructs new widgets for everything below it —
    /// and a re-recorded boundary would then be correct rather than a bug.
    #[derive(Debug)]
    struct Spacer {
        height: vieww_element::Signal<f32>,
    }

    impl Widget for Spacer {
        fn debug_name(&self) -> &'static str {
            "Spacer"
        }

        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }

        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            SizedBox::height(self.height.get()).into()
        }
    }

    vieww_widget::widget_node_from!(Spacer);

    #[test]
    fn a_boundary_that_only_moves_does_not_re_record() {
        let mut driver = driver();
        let gap = driver.elements().runtime().signal(0.0_f32);

        driver
            .elements()
            .set_root(Flex::column().children(children![
                Spacer {
                    height: gap.clone()
                },
                RepaintBoundary::new()
                    .child(ColoredBox::new(Color::BLUE).child(SizedBox::square(20.0))),
            ]));
        driver.draw_frame();
        driver.draw_frame();

        let before = paint_counts(&driver);
        gap.set(40.0);
        driver.draw_frame();

        assert_eq!(
            paint_counts(&driver)[1],
            before[1],
            "the boundary records in its own space, so moving it re-records \
             nothing — only where the layer lands changed"
        );
        assert!(
            driver
                .damage()
                .intersects(Rect::new(90.0, 40.0, 110.0, 60.0)),
            "but the pixels it moved to are damaged: {}",
            driver.damage()
        );
        assert!(
            driver
                .damage()
                .intersects(Rect::new(90.0, 0.0, 110.0, 20.0)),
            "and so are the ones it vacated: {}",
            driver.damage()
        );
    }

    #[test]
    fn a_signal_change_reaches_the_scene_on_the_next_frame() {
        #[derive(Debug)]
        struct Swatch {
            color: Color,
        }

        impl Widget for Swatch {
            fn debug_name(&self) -> &'static str {
                "Swatch"
            }

            fn kind(&self) -> WidgetKind<'_> {
                WidgetKind::Composed
            }

            fn build(&self, _ctx: &BuildContext) -> WidgetNode {
                ColoredBox::new(self.color)
                    .child(SizedBox::square(20.0))
                    .into()
            }
        }

        let mut driver = driver();
        driver
            .elements()
            .set_root(WidgetNode::new(Swatch { color: Color::RED }));
        driver.draw_frame();
        assert_eq!(driver.scene().fills()[0].1.color, Color::RED);

        driver.elements().set_root(WidgetNode::new(Swatch {
            color: Color::GREEN,
        }));
        driver.draw_frame();
        assert_eq!(
            driver.scene().fills()[0].1.color,
            Color::GREEN,
            "a rebuild must reach the pixels in the same frame, not the next one"
        );
    }

    // ------------------------------------------------------------- animation

    /// Runs vsyncs at 60Hz through `drive`, which is what an event loop does,
    /// until nothing is animating. Returns how many frames were produced.
    fn run_animation(driver: &mut FrameDriver, scheduler: &mut FrameScheduler) -> u32 {
        let mut frames = 0;
        let mut now = ms(0);
        while scheduler.is_frame_requested() {
            now += ms(16);
            if driver.drive(scheduler, now, || ms(0)).is_some() {
                frames += 1;
            }
            assert!(frames < 1000, "the animation never stopped");
        }
        frames
    }

    #[test]
    fn an_animation_moves_the_pixels_frame_by_frame() {
        let mut driver = driver();
        let tint = driver.animation(Tween::new(Color::RED, Color::BLUE), ms(160));
        let color = tint.signal();
        driver.elements().set_root(Swatch {
            color: color.clone(),
            size: 20.0,
        });
        driver.draw_frame();
        assert_eq!(driver.scene().fills()[0].1.color, Color::RED);

        tint.forward(ms(0));
        driver.draw_frame_at(ms(80));

        let midway = driver.scene().fills()[0].1.color;
        assert!(
            midway.r > 0 && midway.b > 0 && midway != Color::RED,
            "half way through, the pixels have to be half way: {midway}"
        );

        driver.draw_frame_at(ms(160));
        assert_eq!(driver.scene().fills()[0].1.color, Color::BLUE);
    }

    #[test]
    fn the_driver_keeps_asking_for_frames_until_the_animation_is_over() {
        let mut scheduler = FrameScheduler::sixty_hz();
        let mut driver = driver();
        let tint = driver.animation(Tween::new(Color::RED, Color::BLUE), ms(160));
        driver.elements().set_root(Swatch {
            color: tint.signal(),
            size: 20.0,
        });

        tint.forward(ms(0));
        scheduler.request_frame();
        let frames = run_animation(&mut driver, &mut scheduler);

        assert!(
            frames >= 9,
            "160ms at 60Hz is ten frames; one frame and a stop is what an \
             animation that forgot to re-request looks like: {frames}"
        );
        assert!(!driver.is_animating(), "and then it stops");
        assert_eq!(driver.scene().fills()[0].1.color, Color::BLUE);
    }

    #[test]
    fn a_signal_written_from_layout_is_shown_by_the_frame_that_wrote_it() {
        // `RenderViewport` reports its extents out of **layout**, and the
        // obvious thing to do with them is write a signal — which is exactly
        // what `ScrollController` does. `poll_states` and `rebuild_pending` both
        // ran in the *build* phase, which is behind us by then.
        //
        // **This test used to assert that the driver asks for a second frame.**
        // It did, and that second frame was the first correct one — so the frame
        // actually shown had the previous extents in it. `FrameSink::layout` now
        // rebuilds and re-lays out until quiet before painting, so the extents
        // reach their reader inside the frame that measured them and there is
        // nothing left to ask for.
        let mut scheduler = FrameScheduler::sixty_hz();
        let mut driver = driver();
        let content = driver.elements().runtime().signal(0.0_f32);
        let sink = content.clone();

        driver.set_root(Flex::column().children(children![
            Viewport::new(Axis::Vertical)
                .on_extents(std::rc::Rc::new(move |extents: ScrollExtents| sink
                    .set(extents.content)))
                .child(SizedBox::from_size(Size::new(50.0, 500.0))),
            Spacer {
                height: content.clone()
            },
        ]));

        scheduler.request_frame();
        driver.drive(&mut scheduler, ms(16), || ms(0));

        assert_eq!(
            driver.elements().pending_count(),
            0,
            "layout reported extents into a signal and the settle loop rebuilt \
             the reader before this frame painted, so nothing is left waiting"
        );
        assert!(
            !scheduler.is_frame_requested(),
            "and no frame is owed, because none is needed. The bug this guards \
             against was a list that is wrong until you touch it — a desktop \
             hides it, since a mouse crossing the window requests a frame, and \
             a phone shows it plainly. It is now impossible rather than \
             corrected: the stale frame is never painted in the first place"
        );
        assert!(
            !driver.needs_frame(),
            "and it asks by *requesting a frame*, not by answering `true` here. \
             An event loop reads `needs_frame` to decide whether to sleep, and \
             the version of this fix that put the condition there spun 2923 \
             iterations for 2 frames"
        );

        // **And then it stops.** This is the assertion the reverted attempt did
        // not have, and it is exactly the property a spinning loop violates: the
        // old one asserted a frame was wanted and nothing asserted one ever
        // stopped being wanted.
        //
        // Driven to a fixed point rather than for a fixed count, because the
        // count is not the interesting part and pinning it would make this test
        // fail on a legitimate change. **The loop below now runs zero times** —
        // the frame settled in place, so nothing was requested. It is kept
        // because the property it states is the one that matters, and it is the
        // assertion that would catch the settle loop being removed or capped
        // wrongly: any return to asking for follow-up frames re-enters here and
        // still has to terminate.
        //
        // What makes it terminate at all, wherever the loop lives, is the
        // change-only guard in `RenderViewport` — `Signal::set` marks its reader
        // pending whether or not the value moved, so an unconditional report
        // would mark something every pass for ever and spend the budget instead
        // of settling.
        //
        // That dedupe is the only reason there is no third — `Signal::set`
        // marks its reader pending whether or not the value moved, so an unconditional
        // report here would pending something every frame for ever. The edge in
        // `drive` degrades to one wasted frame rather than a spin if it ever
        // does, but the loop would still be sleeping on stale content, so the
        // guard in `RenderViewport` stays load-bearing.
        let mut frames = 1;
        let mut now = 32;
        while scheduler.is_frame_requested() {
            assert!(
                frames < 8,
                "settled after {frames} frames, or rather did not — a frame that \
                 asks for the next one for ever is the spin this fix exists to \
                 avoid, and it is what an unconditional handler called from \
                 layout would cost"
            );
            driver.drive(&mut scheduler, ms(now), || ms(0));
            frames += 1;
            now += 16;
        }

        assert_eq!(
            driver.elements().pending_count(),
            0,
            "the frames it asked for are the frames that rebuilt the reader"
        );
        assert!(
            !driver.needs_frame(),
            "and the driver is idle at the end of it, which is what lets the \
             event loop go back to `ControlFlow::Wait`"
        );
        assert_eq!(
            content.peek(),
            500.0,
            "and the value that travelled from layout to the reader is the \
             content extent the viewport measured. `peek`, not `get`, so the \
             assertion does not subscribe anything and leave the tree pending \
             behind itself"
        );
    }

    #[test]
    fn the_first_painted_frame_already_has_the_geometry_layout_discovered() {
        // **The claim the settle loop actually makes, asserted in pixels.**
        //
        // `a_signal_written_from_layout_is_shown_by_the_frame_that_wrote_it`
        // asserts the bookkeeping — nothing pending, no frame owed — and that
        // would still pass if the *painted* frame had been recorded before the
        // rebuild. This asserts the thing a user would see: the scene the very
        // first frame produced already reflects a value that did not exist
        // until layout ran.
        //
        // Written because the settle loop changed `RenderViewport` as much as it
        // changed `LayoutBuilder` — scroll extents used to reach their reader a
        // frame late — and every scrolling test stayed green through it, which
        // says nothing broke rather than saying the improvement holds.
        // The marker is **sized** from the measured value rather than pushed
        // down by it, and sits first so it starts at the origin. A position
        // further down the column would be off a 200pt surface and could be
        // culled, which would fail this test for a reason that is not the one
        // it is asking about.
        let mut driver = driver();
        let measured = driver.elements().runtime().signal(0.0_f32);
        let sink = measured.clone();

        driver.set_root(Flex::column().children(children![
            ColoredBox::new(Color::BLUE).child(Spacer {
                height: measured.clone()
            }),
            // Measures a 120-tall child and reports it out of layout.
            Viewport::new(Axis::Vertical)
                .on_extents(std::rc::Rc::new(move |extents: ScrollExtents| sink
                    .set(extents.content)))
                .child(SizedBox::from_size(Size::new(50.0, 120.0))),
        ]));

        driver.draw_frame();

        let blue = driver
            .scene()
            .fills()
            .into_iter()
            .find(|(_, paint)| paint.color == Color::BLUE)
            .map(|(rect, _)| rect)
            .expect(
                "the marker is painted on the first frame — on a frame recorded \
                 before the settle its height is zero, and a zero-height box may \
                 not be recorded at all, which is this same failure",
            );

        assert_eq!(
            blue.height(),
            120.0,
            "the marker was painted {}pt tall. Its height is the extent the \
             viewport measured during *this* frame's layout, so anything other \
             than 120 means the scene was recorded before that value reached \
             its reader — a stale frame on screen, which is the whole thing the \
             settle loop removes",
            blue.height()
        );
        assert_eq!(
            measured.peek(),
            120.0,
            "and the value itself travelled. `peek`, so that asserting does not \
             subscribe this test and leave the tree pending behind itself"
        );
    }

    #[test]
    fn two_windows_share_one_reactive_graph_and_nothing_else() {
        // **The architectural question behind multi-window, answered in a
        // test rather than in a paragraph.**
        //
        // A second window must be a second `FrameDriver`, because everything a
        // window owns — element tree, render tree, layers, scene, pointers,
        // focus, tickers — is already exactly this type's fields. What it must
        // *not* be is a second copy of the application's state, or the two
        // windows are two applications that happen to look alike.
        //
        // `Runtime` is an `Rc` to one reactive graph, so the sharing costs
        // nothing and the isolation is structural rather than maintained.
        let shared = Runtime::new();
        let mut main = FrameDriver::with_runtime(Size::new(200.0, 200.0), shared.clone());
        let mut inspector = FrameDriver::with_runtime(Size::new(120.0, 200.0), shared.clone());

        let gap = shared.signal(10.0_f32);

        main.set_root(Spacer {
            height: gap.clone(),
        });
        inspector.set_root(Spacer {
            height: gap.clone(),
        });
        main.draw_frame();
        inspector.draw_frame();
        assert_eq!(main.elements().pending_count(), 0);
        assert_eq!(inspector.elements().pending_count(), 0);

        // One write, from outside either window.
        gap.set(40.0);

        assert_eq!(
            main.elements().pending_count(),
            1,
            "the window that read the signal has to rebuild"
        );
        assert_eq!(
            inspector.elements().pending_count(),
            1,
            "**and so does the other one.** A document open in two windows is \
             one document; two runtimes would make it two, and the second \
             window would quietly show stale state for ever"
        );
    }

    /// **The defect this test was written to record, and now the one it guards.**
    ///
    /// Two element trees sharing a [`Runtime`] corrupt each other's pending
    /// work, and the cause is that [`ElementId`](vieww_element::ElementId) is
    /// `{ index, generation }` where `index` is an **arena slot within one
    /// tree**. Both trees start at slot 0, so the first element of window A and
    /// the first element of window B are the same key in the runtime's shared
    /// `pending: HashSet<ElementId>`.
    ///
    /// Two consequences, and the second is worse than the first:
    ///
    /// 1. `ElementTree::next_pending` sweeps ids it cannot find — when nothing
    ///    pending is alive in *this* tree it clears the whole set — so drawing
    ///    one window discards the other's pending marks and that window never
    ///    rebuilds. This is what the assertion below catches.
    /// 2. `is_alive` can answer **true** for the other tree's id, so window A
    ///    can rebuild *its* element because window B's was written. Silent,
    ///    and not caught here.
    ///
    /// The bug predates multi-window: `ElementTree::with_runtime` has been
    /// public and has always had it. Nothing shared a runtime until now, so
    /// nothing had ever exercised it.
    ///
    /// **Fixed 2026-08-14**: `ElementId` gained a `tree`
    /// field minted per `ElementTree` from a thread-local counter, so ids are
    /// unique across trees; `next_pending` considers and sweeps only
    /// `id.tree() == self.tree`; and `ElementTree::pending_count` reports this
    /// tree's share rather than the runtime's total.
    #[test]
    fn a_window_drawing_does_not_consume_another_windows_pending_work() {
        let shared = Runtime::new();
        let mut main = FrameDriver::with_runtime(Size::new(200.0, 200.0), shared.clone());
        let mut inspector = FrameDriver::with_runtime(Size::new(120.0, 200.0), shared.clone());

        let gap = shared.signal(10.0_f32);
        main.set_root(Spacer {
            height: gap.clone(),
        });
        inspector.set_root(Spacer {
            height: gap.clone(),
        });
        main.draw_frame();
        inspector.draw_frame();

        gap.set(40.0);
        main.draw_frame();

        assert_eq!(
            main.elements().pending_count(),
            0,
            "the window that drew is settled"
        );
        assert_eq!(
            inspector.elements().pending_count(),
            1,
            "and the one that did not draw still owes a frame — a window is \
             repainted because *it* changed, not because a sibling did. This \
             reported 0 before the `tree` field, because main's rebuild swept \
             the shared set"
        );
    }

    #[test]
    fn a_pending_unbuilt_element_does_not_make_needs_frame_true() {
        // **The shape no test in this file had**, and the reason a spin shipped.
        // Three tests guard against an idle application asking for frames
        // (`nothing_asks_for_frames_once_the_card_has_settled`,
        // `a_settled_control_asks_for_no_more_frames`, and
        // `an_idle_driver_asks_for_nothing` below) and every one of them settles
        // the tree *first* and then asks. None holds an element pending and asks
        // the same question — which is the state a scrolling window is in
        // permanently, and the state in which `needs_frame` answering `true`
        // pins the event loop into a spin it cannot leave.
        //
        // So this is a test about the *rule* rather than about a behaviour: a
        // pending element is a real reason to draw a frame, and it is still not
        // something `needs_frame` may say `true` to, because an event loop reads
        // it to decide whether to sleep and the frame that would clear it is the
        // thing being blocked. `drive` asks for that frame instead.
        let mut scheduler = FrameScheduler::sixty_hz();
        let mut driver = driver();
        let height = driver.elements().runtime().signal(10.0_f32);

        driver.set_root(Flex::column().children(children![Spacer {
            height: height.clone()
        }]));
        scheduler.request_frame();
        driver.drive(&mut scheduler, ms(16), || ms(0));
        assert!(
            !scheduler.is_frame_requested(),
            "the tree is settled before the interesting part starts"
        );

        // Pending, and deliberately not built.
        height.set(40.0);

        assert!(
            driver.elements().pending_count() > 0,
            "the write landed and its reader is waiting to rebuild"
        );
        assert!(
            !driver.needs_frame(),
            "and `needs_frame` still says no. Adding `pending_count() > 0` here \
             cost 2923 event-loop iterations for 2 frames and a window that \
             never drew again — if this assertion is what is failing, that \
             clause is back and the loop will spin"
        );
    }

    #[test]
    fn an_idle_driver_asks_for_nothing() {
        let mut scheduler = FrameScheduler::sixty_hz();
        let mut driver = driver();
        let _unused = driver.animation(Tween::new(0.0_f32, 1.0), ms(200));
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        scheduler.request_frame();
        driver.drive(&mut scheduler, ms(16), || ms(0));

        assert!(!driver.is_animating());
        assert!(
            !scheduler.is_frame_requested(),
            "an animation that was created but never started must not keep the \
             display awake"
        );
    }

    #[test]
    fn an_implicit_animation_runs_itself_through_the_driver() {
        let mut scheduler = FrameScheduler::sixty_hz();
        let mut driver = driver();
        let wide = driver.elements().runtime().signal(false);

        // A composed widget so that flipping the signal rebuilds it — which is
        // how a real screen retargets an `AnimatedContainer`: by describing it
        // differently, with no controller anywhere in sight.
        #[derive(Debug)]
        struct Card {
            wide: vieww_element::Signal<bool>,
        }

        impl Widget for Card {
            fn debug_name(&self) -> &'static str {
                "Card"
            }

            fn kind(&self) -> WidgetKind<'_> {
                WidgetKind::Composed
            }

            fn build(&self, _ctx: &BuildContext) -> WidgetNode {
                // Centred, because the root is laid out under tight constraints
                // and a container that inherits them fills the surface however
                // wide it asked to be.
                Center::new()
                    .child(
                        AnimatedContainer::new()
                            .duration(ms(160))
                            .curve(vieww_animation::Curve::Linear)
                            .color(Color::RED)
                            .width(if self.wide.get() { 100.0 } else { 20.0 })
                            .height(20.0)
                            .child(SizedBox::shrink()),
                    )
                    .into()
            }
        }

        vieww_widget::widget_node_from!(Card);

        driver.elements().set_root(Card { wide: wide.clone() });
        driver.draw_frame();
        let painted = |driver: &FrameDriver| driver.scene().fills()[0].0.width();
        assert_eq!(painted(&driver), 20.0);

        wide.set(true);
        scheduler.request_frame();
        let frames = run_animation(&mut driver, &mut scheduler);

        assert!(
            frames >= 9,
            "a description that changed has to animate across frames rather \
             than snapping: {frames}"
        );
        assert_eq!(painted(&driver), 100.0);
        assert!(!driver.is_animating());
    }

    #[test]
    fn an_animation_dropped_mid_flight_stops_the_frames() {
        let mut scheduler = FrameScheduler::sixty_hz();
        let mut driver = driver();
        let tint = driver.animation(Tween::new(Color::RED, Color::BLUE), ms(1600));
        driver.elements().set_root(Swatch {
            color: tint.signal(),
            size: 20.0,
        });
        tint.forward(ms(0));
        scheduler.request_frame();
        driver.drive(&mut scheduler, ms(16), || ms(0));
        assert!(driver.is_animating());

        // The screen holding it went away.
        drop(tint);

        driver.drive(&mut scheduler, ms(32), || ms(0));
        assert!(
            !driver.is_animating(),
            "nothing owns the animation any more, and nothing had to unregister it"
        );
    }

    #[test]
    fn a_root_mounted_through_the_element_tree_is_not_the_drivers_root() {
        // The distinction the notch bug turned on. Both mount a drawable tree;
        // only one of them can be re-published under new view metrics.
        let mut driver = driver();
        assert!(!driver.has_root());

        driver.elements().set_root(ColoredBox::new(Color::RED));
        assert!(
            !driver.has_root(),
            "the element tree has a root and the driver was never told"
        );

        driver.set_root(ColoredBox::new(Color::BLUE));
        assert!(driver.has_root());
    }

    /// Debug-only, because the guard is a `debug_assert`. Stated explicitly so
    /// the test does not quietly change meaning under `cargo test --release`.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "no root has been mounted")]
    fn changing_metrics_with_no_root_fails_loudly_rather_than_dropping_them() {
        let mut driver = driver();
        driver.elements().set_root(ColoredBox::new(Color::RED));

        // Correct insets, computed at real cost, with nowhere to go.
        driver.set_view_metrics(ViewMetrics {
            size: Size::new(200.0, 200.0),
            device_pixel_ratio: 3.0,
            safe_area: EdgeInsets::only(0.0, 47.0, 0.0, 34.0),
            view_insets: EdgeInsets::ZERO,
        });
    }

    // ------------------------------------------------------ damage cull stats

    /// The count, not the picture: a flattener that retained a stale scene
    /// and rebuilt nothing would score perfectly.
    #[test]
    fn an_unchanged_frame_has_no_damage_cull_work() {
        let mut driver = driver();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(50.0)));

        driver.draw_frame();
        driver.draw_frame();

        // A second clean frame: damage is clean so no cull stats.
        assert_eq!(
            driver.damage_cull_stats(),
            DamageCullStats::default(),
            "no damage, no cull work"
        );
    }

    /// A small change in one corner should leave the opposite corner
    /// unculled.
    #[test]
    fn damage_culling_skips_commands_outside_the_damaged_region() {
        // Build a tree with things spread across the surface.
        let mut driver = driver();
        driver.elements().set_root(
            Flex::row().children([
                ColoredBox::new(Color::RED)
                    .child(SizedBox::square(50.0))
                    .into(),
                ColoredBox::new(Color::BLUE)
                    .child(SizedBox::square(50.0))
                    .into(),
                ColoredBox::new(Color::GREEN)
                    .child(SizedBox::square(50.0))
                    .into(),
                ColoredBox::new(Color::rgb(255, 255, 0))
                    .child(SizedBox::square(50.0))
                    .into(),
            ]),
        );

        driver.draw_frame();
        driver.draw_frame(); // settle
        let total_commands = driver.scene().len();

        // Change the first box's colour. Damage should be bounded to its area.
        driver.elements().set_root(
            Flex::row().children([
                ColoredBox::new(Color::rgb(255, 0, 255))
                    .child(SizedBox::square(50.0))
                    .into(),
                ColoredBox::new(Color::BLUE)
                    .child(SizedBox::square(50.0))
                    .into(),
                ColoredBox::new(Color::GREEN)
                    .child(SizedBox::square(50.0))
                    .into(),
                ColoredBox::new(Color::rgb(255, 255, 0))
                    .child(SizedBox::square(50.0))
                    .into(),
            ]),
        );
        driver.draw_frame();

        let stats = driver.damage_cull_stats();
        assert!(
            stats.skipped_commands > 0,
            "something should be skipped with a localised change \
             ({total_commands} total, {stats:?})"
        );
        assert_eq!(
            stats.total(),
            total_commands,
            "copied + skipped must cover all commands"
        );
    }

    /// A full-repaint frame culls nothing.
    #[test]
    fn a_full_repaint_culls_nothing() {
        let mut driver = driver();
        driver
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(50.0)));

        driver.draw_frame();
        driver.draw_frame();

        // Force full repaint and draw.
        driver.invalidate();
        driver.draw_frame();

        let stats = driver.damage_cull_stats();
        assert_eq!(
            stats.skipped_commands, 0,
            "everything is damage, nothing can be skipped"
        );
    }
}
