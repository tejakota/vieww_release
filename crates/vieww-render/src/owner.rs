//! Keeping the render tree in step with the element tree.

use std::fmt;

use vieww_element::{ElementId, ElementTree};
use vieww_foundation::{Constraints, Offset, Size};
use vieww_paint::Canvas;
use vieww_widget::{WidgetKind, WidgetNode};

use crate::{ChildIds, HitTestResult, RenderFactory, RenderId, RenderTree};
use std::collections::HashSet;
use vieww_foundation::FastMap;

/// What one element looked like at the last sync.
///
/// One record per element the sync has ever visited, replacing the two parallel
/// `HashMap`s this used to be — every lookup was doing two hashes of the same
/// key to answer one question.
struct Synced {
    /// The render object this element owns, if it renders at all.
    render: Option<RenderId>,
    /// The widget that render object was built from.
    ///
    /// Compared by `Rc` pointer, not by value: "is this the same description"
    /// is the question, and a widget that is the same allocation cannot
    /// describe anything different.
    ///
    /// # It is not the *only* input, and that is why
    /// [`RenderOwner::rebuild_render_objects`] exists
    ///
    /// A render object is built by the factory from the widget **and the
    /// inherited scope** — `ambient_direction`, `ambient_touch_target`,
    /// `ambient_accessibility`. So a pointer-identical widget under a *changed*
    /// scope describes something different after all, and this comparison says
    /// otherwise.
    ///
    /// It is very nearly never wrong, because almost everything that publishes
    /// into the scope is also read during `build`, which produces new widgets
    /// and breaks the pointer equality by itself. The exception is anything read
    /// **only** by the factory — which is exactly `Accessibility`, since `Text`
    /// is a `RenderLeaf` and never builds. Found by
    /// `crates/vieww/tests/accessibility_preferences.rs`, which changed the text
    /// scale on a mounted tree and watched nothing happen.
    widget: Option<WidgetNode>,
    /// The render objects this element handed its parent.
    ///
    /// One entry for a rendering element; for a transparent one — composed or
    /// inherited — whatever its children contributed. Kept so that a subtree
    /// which has not changed can be answered from here instead of walked.
    contributed: ChildIds,
    /// The element's `subtree_revision` when this record was written.
    revision: u64,
}

/// Owns an element tree's corresponding render tree and drives a frame.
///
/// This is the pipeline owner: the thing that knows the order a frame
/// happens in — rebuild pending elements, sync the render tree to match, lay out,
/// paint.
///
/// Not every element has a render object. Composed and inherited elements exist
/// only to describe and to publish; they are **transparent** here, and their
/// children attach to the nearest ancestor that does render. That is why the
/// render tree is always shallower than the element tree, often much.
pub struct RenderOwner {
    tree: RenderTree,
    factory: RenderFactory,
    /// What each element looked like at the last sync.
    synced: FastMap<ElementId, Synced>,
    /// Set for exactly one `sync`, by [`RenderOwner::rebuild_render_objects`],
    /// to suppress the subtree-revision early-out. See its docs and the use
    /// site in `sync_element`.
    rebuilding: bool,
    /// The element tree's death count at the last sweep of [`Self::synced`].
    ///
    /// Records for dead elements have to be removed, and finding them means
    /// walking the whole map — the one part of a sync that is unavoidably
    /// proportional to tree size rather than to what changed. So it is not done
    /// unless something actually died, which this number reports in O(1).
    /// Removals are rare; an idle frame, a scroll and a colour change all leave
    /// it untouched.
    swept_at_deaths: u64,
    /// How many elements the last [`Self::sync`] actually walked.
    ///
    /// The counted form of "work proportional to what changed". Read it in a
    /// test to make that an assertion rather than a claim — see
    /// [`Self::visited_last_sync`].
    visited: usize,
}

impl RenderOwner {
    /// An owner with the built-in widgets registered.
    #[must_use]
    pub fn new() -> Self {
        Self::with_factory(RenderFactory::with_builtins())
    }

    /// An owner using a custom factory.
    #[must_use]
    pub fn with_factory(factory: RenderFactory) -> Self {
        Self {
            tree: RenderTree::new(),
            factory,
            synced: FastMap::default(),
            rebuilding: false,
            swept_at_deaths: 0,
            visited: 0,
        }
    }

    /// The render tree.
    #[must_use]
    pub const fn tree(&self) -> &RenderTree {
        &self.tree
    }

    /// The render tree, mutably.
    pub const fn tree_mut(&mut self) -> &mut RenderTree {
        &mut self.tree
    }

    /// The widget-to-render-object registry.
    #[must_use]
    pub const fn factory(&self) -> &RenderFactory {
        &self.factory
    }

    /// Register a render object for a widget type. See [`RenderFactory::register`].
    pub fn register<W, R>(&mut self, create: impl Fn(&W) -> R + 'static)
    where
        W: vieww_widget::Widget,
        R: crate::RenderObject,
    {
        self.factory.register::<W, R>(create);
    }

    /// How many elements the last [`Self::sync`] walked.
    ///
    /// A sync skips any subtree the element tree has not stamped as changed, so
    /// this is the size of the changed part of the tree plus the path to it —
    /// not the size of the tree. An idle frame reports 0.
    ///
    /// Exposed because "work proportional to what changed" is the kind of claim
    /// that is true when it is written and quietly stops being true later.
    /// `tests/incremental_sync.rs` asserts on it.
    #[must_use]
    pub const fn visited_last_sync(&self) -> usize {
        self.visited
    }

    /// Force every render object to be rebuilt from its widget on the next
    /// [`sync`](Self::sync), keeping the render ids and the tree shape.
    ///
    /// For a change to something the **factory** reads out of the inherited
    /// scope rather than out of the widget — see `Synced::widget` for the
    /// full argument and for the one case that is really like this.
    ///
    /// It forgets the *comparison*, not the objects: each entry keeps its
    /// `render` id and drops only the widget it was last built from, so `sync`
    /// takes the `replace_object` path rather than the `insert_detached` one.
    /// Clearing the whole map instead would mint new `RenderId`s for a tree
    /// that has not structurally changed, orphaning the old ones and throwing
    /// away every relayout boundary in the process.
    ///
    /// Costs one full pass over the render tree, so it is for genuinely rare
    /// events. The only caller is
    /// [`FrameDriver::set_accessibility`](crate::FrameDriver::set_accessibility),
    /// which fires when a person changes an OS setting — a handful of times in
    /// the life of an application, against a mis-sized UI for the people who
    /// rely on it if it fires none.
    pub fn rebuild_render_objects(&mut self) {
        self.rebuilding = true;
        for entry in self.synced.values_mut() {
            entry.widget = None;
        }
    }

    /// The render object an element produced, if it produced one.
    #[must_use]
    pub fn render_of(&self, element: ElementId) -> Option<RenderId> {
        self.synced.get(&element).and_then(|entry| entry.render)
    }

    /// Bring the render tree in line with the element tree.
    ///
    /// # What "incremental" means here
    ///
    /// It used to mean only that render objects were not *reconstructed*: the
    /// walk still visited every element every frame, cloning a `WidgetNode`
    /// and building a child-id `Vec` per node, to discover that nothing had
    /// moved. On a tree of any size that is the dominant cost of an idle
    /// frame, and it grew with the whole application rather than with the part
    /// of it that changed.
    ///
    /// Now the walk stops at the top of any subtree whose
    /// [`Element::subtree_revision`](vieww_element::Element::subtree_revision)
    /// has not advanced since the last sync — the element tree stamps that on
    /// every mutation path, so "not advanced" means "nothing under here was
    /// touched". A frame that changed one label pays for the path from the root
    /// to that label and nothing else.
    ///
    /// Removals are the one thing a skipped subtree cannot report, so they are
    /// handled separately and only when the element tree says something died.
    pub fn sync(&mut self, elements: &ElementTree) {
        let Some(root) = elements.root() else {
            self.tree = RenderTree::new();
            self.synced.clear();
            self.swept_at_deaths = elements.deaths();
            return;
        };

        self.sweep_dead(elements);

        self.visited = 0;
        let mut roots = ChildIds::new();
        self.sync_element(elements, root, &mut roots);
        self.tree.set_root(roots.first().copied());
        // One pass, not a mode. Cleared here rather than by the caller so that
        // a forced rebuild cannot leave every later frame walking the whole
        // tree — which would silently undo the revision optimisation entirely.
        self.rebuilding = false;
    }

    /// Drop the records of elements that no longer exist.
    ///
    /// Proportional to the number of live records, so it is gated on the
    /// element tree's death count: no unmount since the last sweep means
    /// nothing in the map can have gone stale, and the whole pass is skipped.
    fn sweep_dead(&mut self, elements: &ElementTree) {
        let deaths = elements.deaths();
        if deaths == self.swept_at_deaths {
            return;
        }
        self.swept_at_deaths = deaths;

        let Self { synced, tree, .. } = self;
        synced.retain(|&element, entry| {
            if elements.is_alive(element) {
                return true;
            }
            if let Some(render) = entry.render {
                tree.remove(render);
            }
            false
        });
    }

    /// Sync one element, appending the render objects it contributes to its
    /// parent — one if it renders, otherwise whatever its children contributed.
    ///
    /// Appends into the caller's buffer rather than returning a `Vec`, so a
    /// chain of transparent elements costs no allocation at all.
    fn sync_element(&mut self, elements: &ElementTree, element: ElementId, out: &mut ChildIds) {
        let Some(node) = elements.get(element) else {
            return;
        };

        let revision = node.subtree_revision();
        if let Some(previous) = self.synced.get(&element) {
            // `!self.rebuilding` is the second half of the condition and it is
            // load-bearing: the revision answers "did the *element tree*
            // change", and a factory input that lives in the inherited scope
            // can change while every revision in the tree stays put. Without
            // this the walk returns here and `Synced::widget` is never even
            // consulted, so clearing it accomplishes nothing.
            if previous.revision == revision && !self.rebuilding {
                // Nothing under here has been touched since the record was
                // written, so it still describes the truth. This is the whole
                // point of the revision: no clone, no recursion, no
                // `set_children`.
                out.extend_from_slice(&previous.contributed);
                return;
            }
        }

        self.visited += 1;
        let widget = node.widget().clone();

        let mut child_renders = ChildIds::new();
        for &child in node.children() {
            self.sync_element(elements, child, &mut child_renders);
        }

        if !self.factory.handles(&widget) {
            // Transparent — composed or inherited — or a render widget whose
            // type was never registered, which is a bug and is reported.
            warn_if_unregistered(&widget);

            // A widget that used to render and no longer does leaves a render
            // object behind. Rare — it needs a `can_update` match across a
            // change of kind — but the object would otherwise sit detached in
            // the tree forever.
            if let Some(previous) = self.synced.get(&element) {
                if let Some(render) = previous.render {
                    self.tree.remove(render);
                }
            }

            out.extend_from_slice(&child_renders);
            self.synced.insert(
                element,
                Synced {
                    render: None,
                    widget: None,
                    contributed: child_renders,
                    revision,
                },
            );
            return;
        }

        let existing = self
            .synced
            .get(&element)
            .and_then(|entry| entry.render)
            .filter(|&id| self.tree.is_alive(id));

        let id = match existing {
            Some(id) => {
                let unchanged = self
                    .synced
                    .get(&element)
                    .and_then(|entry| entry.widget.as_ref())
                    .is_some_and(|old| old.ptr_eq(&widget));
                if !unchanged {
                    let object = self
                        .factory
                        .create_with_context(&widget, node.scope())
                        .expect("factory reported it handles this widget");
                    self.tree.replace_object(id, object);
                }
                id
            }
            None => {
                let object = self
                    .factory
                    .create_with_context(&widget, node.scope())
                    .expect("factory reported it handles this widget");
                self.tree.insert_detached(object)
            }
        };

        self.tree.set_children(id, child_renders);
        out.push(id);
        self.synced.insert(
            element,
            Synced {
                render: Some(id),
                widget: Some(widget),
                // The single-id case, inline. This used to be `vec![id]` — a
                // guaranteed heap allocation for *every* rendering element on
                // every miss, which on the studio's 536-element keystroke
                // rebuild was 536 allocations for 536 eight-byte values.
                contributed: ChildIds::one(id),
                revision,
            },
        );
    }

    /// Run a whole frame: rebuild pending elements, sync, lay out.
    ///
    /// The order is not negotiable. Layout must see the render tree the
    /// *current* widget tree describes, so rebuilding and syncing both have to
    /// finish first — laying out against a stale tree produces a frame that is
    /// one state change behind, which is the classic source of "it updates a
    /// frame late" bugs.
    pub fn draw_frame(&mut self, elements: &mut ElementTree, constraints: Constraints) -> Size {
        elements.rebuild_pending();
        self.sync(elements);
        self.tree.layout_root(constraints)
    }

    /// Paint the render tree.
    pub fn paint(&self, canvas: &mut dyn Canvas) {
        self.tree.paint(canvas);
    }

    /// Paint the render tree onto a surface at `dpr` physical pixels to the
    /// logical one. See [`RenderTree::paint_with_ratio`].
    pub fn paint_with_ratio(&self, canvas: &mut dyn Canvas, dpr: f32) {
        self.tree.paint_with_ratio(canvas, dpr);
    }

    /// Hit test the render tree.
    #[must_use]
    pub fn hit_test(&self, point: Offset) -> HitTestResult {
        self.tree.hit_test(point)
    }
}

/// Complain, once per widget type, about a render widget nobody registered.
///
/// # Why this is not silent
///
/// `sync_element` treats "the factory does not handle this widget" as
/// *transparent*, and for a composed or inherited widget that is exactly right
/// — they describe and publish, they do not draw.
///
/// For a `RenderLeaf` or `RenderSingleChild` it is not right at all. A render
/// widget whose type was never passed to [`RenderOwner::register`] produced no
/// render object, no warning and no error: it drew nothing, its children were
/// spliced silently into its parent's child list, and the only symptom was a
/// hole in the layout somewhere else. Third-party render objects are an
/// advertised extension point — `crates/vieww/tests/third_party_render_widget.rs`
/// stands where a third party stands — which makes forgetting to register one
/// the single likeliest way to be defeated by this framework.
///
/// Deduplicated by type name because it fires from inside a sync, and a sync
/// runs every frame: without it, one forgotten registration writes sixty lines
/// a second and the message that would have explained it scrolls away.
///
/// A warning rather than a panic, on the same reasoning `overflow.rs` uses: a
/// missing widget is a visible defect an author will chase, and taking the
/// process down in a released application makes a layout mistake into a crash.
///
/// # Where the warning goes
///
/// Used to be a bare `eprintln!` to a stderr nobody was reading. Now the host
/// can install a sink through [`set_unregistered_render_object_sink`], which is
/// what the studio does to surface these in the Problems panel rather than in
/// a console the user of a GUI application does not have. Without a sink the
/// `eprintln!` still happens, so a host that has not opted in (every test, every
/// example) is no worse off than before.
fn warn_if_unregistered(widget: &WidgetNode) {
    if !matches!(
        widget.kind(),
        WidgetKind::RenderLeaf | WidgetKind::RenderSingleChild(_) | WidgetKind::RenderMultiChild(_)
    ) {
        return;
    }

    let name = widget.debug_name();
    if !UNREGISTERED_REPORTED.with_borrow_mut(|seen| seen.insert(name)) {
        return;
    }

    let message = format!(
        "vieww: `{name}` describes a render object but no render object is \
         registered for it, so it drew nothing and its children were attached \
         to its parent instead. Call `RenderOwner::register::<{name}, _>(..)` \
         (or `RenderFactory::register`) before the first frame."
    );

    // If the host installed a sink, hand the name and the formatted message
    // over. If it did not, the message goes to stderr exactly as before —
    // which is what a host that has no UI to surface it in still wants.
    let sink_taken = UNREGISTERED_SINK.with_borrow(|sink| sink.is_some());
    if sink_taken {
        UNREGISTERED_SINK.with_borrow(|sink| {
            if let Some(sink) = sink.as_ref() {
                sink(name, &message);
            }
        });
    } else {
        eprintln!("{message}");
    }
}

use std::cell::RefCell;

/// A host-installed reporter for a diagnostic: `(name, detail)`.
///
/// Both sinks in this module take the same shape, and both took it spelled out
/// — `Option<Box<dyn Fn(&str, &str)>>` — in four places between the two of
/// them, which is what clippy's `type_complexity` was pointing at. The name is
/// worth more than the brevity: two bare `&str`s in a row say nothing about
/// which is which, and `DiagnosticSink` has one line to say that the first is
/// the render object's type name and the second is what happened to it.
///
/// Not `Send + Sync`, deliberately. Each of the sites below explains why: the
/// studio's `Signal` is `Rc`-backed, the sink runs on the thread that owns it,
/// and demanding thread-safety here would push every host through an
/// `Arc<Mutex<..>>` for a guarantee nothing needs.
pub type DiagnosticSink = Box<dyn Fn(&str, &str)>;

thread_local! {
    /// One name per process — see `warn_if_unregistered`'s dedup note.
    static UNREGISTERED_REPORTED: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
    /// The host-installed sink for unregistered-render-object warnings, or
    /// `None` to fall back to `eprintln!`. Set through
    /// [`set_unregistered_render_object_sink`].
    ///
    /// `Fn` rather than `FnMut` because the warning fires from inside a
    /// `sync`, which holds the render tree's borrow immutably; a `FnMut` sink
    /// would also need `&mut self`, and the studio's sink is `Fn` anyway
    /// (it pushes through a `Signal`).
    ///
    /// Not `Send + Sync` on purpose. The render tree is single-threaded, so
    /// the sink is only ever called from the thread that installed it. A
    /// studio's `Signal` is `Rc`-backed and not `Send`; requiring `Send +
    /// Sync` here would force every host through an `Arc<Mutex<..>>` for no
    /// reason — the sink runs on the same thread that owns the signal.
    static UNREGISTERED_SINK: RefCell<Option<DiagnosticSink>> = RefCell::new(None);
}

/// Install a process-wide sink for warnings about unregistered render
/// objects.
///
/// The sink is called once per widget *type* (deduplicated internally), with
/// the widget's `debug_name` and a formatted message suitable for showing to
/// a user. A host with a Problems panel — the studio — installs one at
/// startup so these land where the developer will see them, rather than on
/// stderr that a GUI application does not have.
///
/// Passing `None` clears the sink and returns to the default `eprintln!`
/// behaviour.
///
/// # Why a thread-local rather than a method on `RenderOwner`
///
/// The warning fires from inside `sync_element`, which is a free function
/// rather than a method on `RenderOwner`, because the deduplication has to be
/// per-process rather than per-tree: a second `RenderOwner` in the same
/// process that builds the same unregistered widget type would otherwise warn
/// a second time, and the studio only has one Problems panel either way. A
/// thread-local sink is the same shape as the existing dedup set, and is the
/// one place the warning has to look.
///
/// # Why not `Send + Sync`
///
/// The render tree is single-threaded, so the sink is only ever called from
/// the thread that installed it. The studio's `Signal` is `Rc`-backed and not
/// `Send`; forcing a `Send + Sync` bound would push every host through an
/// `Arc<Mutex<..>>` for a guarantee nothing needs. The dedup set the warning
/// already uses is a thread-local for the same reason.
pub fn set_unregistered_render_object_sink(sink: Option<DiagnosticSink>) {
    UNREGISTERED_SINK.with_borrow_mut(|slot| *slot = sink);
}

// ------------------------------------------------------------------ panics
// The per-node catch for layout, paint, and hit-test. See `report_render_panic`
// below for the full story.

thread_local! {
    /// The host-installed sink for render-object panics, or `None` to fall back
    /// to `eprintln!`. Set through [`set_render_panic_sink`].
    static RENDER_PANIC_SINK: RefCell<Option<DiagnosticSink>> = RefCell::new(None);
    /// Names that have already reported a panic, so the sink fires once per
    /// type rather than once per frame — a panicking `CustomPainter::paint`
    /// runs sixty times a second.
    static RENDER_PANIC_REPORTED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Install a process-wide sink for render-object panics.
///
/// The sink is called once per widget *type* (deduplicated internally), with
/// the widget's `debug_name` and a formatted message. A host with a Problems
/// panel — the studio — installs one at startup so these land where the
/// developer will see them. Without a sink, the `eprintln!` still happens.
///
/// # Why this exists
///
/// `ElementTree::build`'s `catch_unwind` covers `widget.build()`. The render
/// tree calls `RenderObject::layout`, `paint`, and `hit_test` directly, and a
/// `CustomPainter::paint` that panics — or a render object whose `layout`
/// divides by zero — used to abort the process. With both sides built
/// `-C prefer-dynamic` (see `Cargo.toml`), the host's `catch_unwind` catches
/// guest panics as written; this sink is how the catch reports what it caught.
pub fn set_render_panic_sink(sink: Option<DiagnosticSink>) {
    RENDER_PANIC_SINK.with_borrow_mut(|slot| *slot = sink);
}

/// Report a panic from a render-object method, deduplicated by widget name.
///
/// Called from `RenderTree::layout`, `paint_node`, and `hit_test_node` when
/// their `catch_unwind` catches. The `phase` is `"layout"`, `"paint"`, or
/// `"hit_test"` — the three per-node entry points that call into guest code.
pub(crate) fn report_render_panic(name: &str, phase: &str) {
    if !RENDER_PANIC_REPORTED.with_borrow_mut(|seen| seen.insert(name.to_string())) {
        return;
    }
    let message = format!(
        "vieww: `{name}` panicked during {phase}. The render object was \
         skipped, which may leave a hole in the layout or a dead spot in \
         hit-testing. Fix the panic in the screen's source to restore the \
         frame."
    );
    let sink_taken = RENDER_PANIC_SINK.with_borrow(|sink| sink.is_some());
    if sink_taken {
        RENDER_PANIC_SINK.with_borrow(|sink| {
            if let Some(sink) = sink.as_ref() {
                sink(name, &message);
            }
        });
    } else {
        eprintln!("{message}");
    }
}

impl Default for RenderOwner {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RenderOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderOwner")
            .field("render_objects", &self.tree.len())
            .field("mapped_elements", &self.synced.len())
            .finish()
    }
}
