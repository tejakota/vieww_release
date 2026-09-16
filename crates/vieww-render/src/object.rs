use std::any::Any;
use std::fmt::{self, Debug};

use vieww_foundation::{
    Constraints, Cursor, ImeEvent, KeyEvent, Offset, Rect, ScrollEvent, Size, Transform,
};
use vieww_widget::{FlexFactor, StackPosition};

use vieww_gestures::{GestureRecognizer, Recognized};
use vieww_paint::{Canvas, LayerEffect};

use crate::sliver::{SliverConstraints, SliverGeometry};
use crate::{RenderId, RenderTree, SemanticAction, Semantics};

/// A node that knows how to size itself and draw itself.
///
/// # The layout protocol
///
/// Constraints travel **down**, sizes come **up**. A parent hands each child a
/// [`Constraints`], the child returns the [`Size`] it chose within them, and the
/// parent then decides where to *put* it. A child never learns its own position
/// from layout, and never reads its parent — those two rules together are what
/// make layout a single pass instead of a fixed-point iteration.
///
/// Consequences worth internalising:
///
/// - A child cannot be "as wide as its parent" by asking. The parent has to
///   pass tight constraints saying so.
/// - A parent cannot position a child before laying it out, because position
///   usually depends on the size it comes back with.
/// - Layout is O(n) with one visit per node, provided nobody lays a child out
///   twice. Where a parent genuinely needs to (measure, then place), that is a
///   deliberate second pass and should be commented as such.
pub trait RenderObject: Any + Debug + 'static {
    /// Choose a size that satisfies `constraints`, laying out children as
    /// needed.
    ///
    /// The returned size **must** satisfy the constraints. Returning one that
    /// does not is a bug in the render object; `RenderTree::layout` asserts it
    /// in debug builds rather than letting a bad size propagate somewhere far
    /// away and surface as a mystery.
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size;

    /// Lay out under the **sliver** protocol, if this object speaks it.
    ///
    /// `None` — the default, and what every box render object returns — means
    /// "I only speak box layout". A viewport handed one of those wraps it: the
    /// child is measured once with box constraints and reported as a sliver of
    /// exactly that size. So `Text` and `Container` drop into a scrolling
    /// viewport with no adapter widget, which is the part other toolkits make you
    /// write by hand as an explicit adapter.
    ///
    /// # Why a defaulted method rather than a second trait
    ///
    /// The tree stores `Box<dyn RenderObject>`, and a second trait would mean
    /// either a second storage path or a downcast at every layout. This costs
    /// one `None` per box object and leaves the tree untouched — which is what
    /// made adding a second layout protocol to twenty-three existing render
    /// objects a change none of them noticed.
    ///
    /// An object implementing this **must** still implement
    /// [`layout`](Self::layout), because it may be placed outside a viewport. A
    /// sliver's box layout should report the size it would have at
    /// `scroll_offset` zero.
    ///
    /// See [`sliver`](crate::sliver) for what the two halves mean.
    fn layout_sliver(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        constraints: &SliverConstraints,
    ) -> Option<SliverGeometry> {
        let (_, _) = (ctx, constraints);
        None
    }

    /// How big this object *wants* to be, without laying anything out.
    ///
    /// The default is `None` — **"I do not know"** — and that is the whole
    /// design decision. A `0.0` default, as some toolkits choose, means a custom
    /// render object that never implements it reports that it wants no space,
    /// and the bug surfaces as something collapsing several levels away.
    /// `None` propagates instead: a parent that cannot measure a child returns
    /// `None`, the widget that asked falls back to the behaviour it had before
    /// intrinsics existed, and implementing this on one more object strictly
    /// improves the answer without being able to make anything worse.
    ///
    /// # The contract
    ///
    /// - **Do not lay children out.** [`IntrinsicCtx`] cannot, which is why it
    ///   is a different type from [`LayoutCtx`]. An intrinsic query that laid
    ///   its children out would leave them sized against constraints their
    ///   parent never chose, and the corruption surfaces a frame later.
    /// - **Answer for content, not for constraints.** `RenderConstrained`
    ///   clamps its child's answer; it does not return its own maximum. The
    ///   caller applies constraints itself.
    /// - **Be a pure function of this subtree's configuration.** The answer is
    ///   cached until something in the subtree marks itself for layout, so an
    ///   implementation that reads a clock or a scroll offset will go stale.
    ///
    /// See `crate::intrinsics` for why this is opt-in and what it costs.
    ///
    /// [`IntrinsicCtx`]: crate::IntrinsicCtx
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        let (_, _) = (ctx, query);
        None
    }

    /// Where the first line of text in this subtree sits, measured **down from
    /// the top of this object's own box**.
    ///
    /// Answered after layout, and only asked by a parent that needs it —
    /// today that is a [`RenderFlex`](crate::RenderFlex) under
    /// [`CrossAxisAlignment::Baseline`](vieww_widget::CrossAxisAlignment::Baseline).
    ///
    /// The default is `None`: *"there is no text in here."* That is not the
    /// same as zero, and the difference is visible — a row aligning to a
    /// `None` child falls back to top alignment for that child, where a row
    /// aligning to a fabricated `0.0` would hang it from its top edge and look
    /// like a bug somewhere else. See [`crate::baseline`] for the rest of the
    /// rules, and for the pass-through macro that most single-child objects
    /// want instead of writing this out.
    fn baseline(&self, ctx: &mut crate::BaselineCtx<'_>, size: Size) -> Option<f32> {
        let (_, _) = (ctx, size);
        None
    }

    /// Draw this object — **not** its children.
    ///
    /// The tree paints children afterwards, in order. That covers everything
    /// here; a render object that needs to interleave or reorder child painting
    /// wants a layer, which is Phase 4.
    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let _ = ctx;
    }

    /// Called once this object's children have all been painted.
    ///
    /// The only way to *pop* something [`paint`](Self::paint) pushed — a clip, a
    /// transform, a layer — because the tree paints the children in between the
    /// two. A viewport clipping its content is the case this exists for.
    ///
    /// Whatever `paint` pushed must be popped here; a `save` without its
    /// `restore` leaks into every sibling drawn afterwards, which shows up as
    /// unrelated widgets mysteriously clipped.
    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        let _ = ctx;
    }

    /// Whether this object's subtree keeps its own recording.
    ///
    /// A boundary gets a [`Layer`](vieww_paint::Layer): repaints inside it stop
    /// there instead of reaching the root, so the cost of a change is bounded by
    /// the subtree containing it rather than by the size of the screen.
    ///
    /// Defaults to `false`, because a layer is not free — it costs a recording to
    /// keep and a step in every composite, and most objects repaint only when
    /// their parent does anyway. The same line is drawn with
    /// `RepaintBoundary`, and the guidance is the same: too many layers is as
    /// slow as too few.
    fn is_repaint_boundary(&self) -> bool {
        false
    }

    /// How this object composites the subtree beneath it, if it does.
    ///
    /// `None` — the default — means "draws its children as they are", which is
    /// almost every object. Returning `Some` says the subtree is a *group*: it
    /// is composited once, as a unit, rather than primitive by primitive.
    ///
    /// # Why this is a hook and not a check for known types
    ///
    /// The paint walk stops at a nested repaint boundary, because that subtree
    /// records into its own layer. So an object that groups its children has no
    /// way to reach a boundary below it by recording commands — the commands go
    /// into a scene the boundary's content never enters. The effect has to
    /// travel to the *layer*, and the layer tree has to be told.
    ///
    /// It would have been shorter to have [`RenderTree`] look for a
    /// `RenderOpacity` and read its alpha. That closes the framework: a blur, a
    /// colour filter, a mask or anything else somebody writes outside this
    /// repository would compose correctly over ordinary children and silently
    /// fail over a scrollable, with no way to opt in short of editing vieww.
    /// Declaring it here costs one defaulted method and keeps the door open.
    ///
    /// # What it must not be used for
    ///
    /// This does not replace recording. An object returning `Some` still paints
    /// its own `push_layer`/`pop_layer` for the ordinary children it encloses —
    /// see [`RenderOpacity`](crate::RenderOpacity). This answers a second
    /// question: what should happen to the children that are *not* recorded
    /// here. Returning `Some` while recording nothing fades boundaries and
    /// leaves everything else alone, which is the original bug inverted.
    fn layer_effect(&self) -> Option<LayerEffect> {
        None
    }

    /// What this object confines its subtree to, if it clips.
    ///
    /// `bounds` is this object's box **already in the space the clip will be
    /// applied in** — the enclosing repaint boundary's — so an implementation
    /// that clips to itself returns `Some(bounds)` and one that clips to part of
    /// itself derives the rectangle from it (`bounds.deflate(8.0)`). Returning
    /// `None`, the default, means "does not clip", which is almost every object.
    ///
    /// # Why this exists next to `layer_effect` rather than inside it
    ///
    /// The same hole, and an object that clips usually has no opinion about
    /// alpha. [`Canvas::clip_rect`] folds a clip into commands **as they are
    /// recorded**, so an ancestor's clip lives in the ancestor's scene and a
    /// descendant repaint boundary records into a different one. The clip has to
    /// travel to the *layer* to reach it.
    ///
    /// This is not hypothetical and not rare: [`RenderViewport`] both clips and
    /// is a repaint boundary, so before this hook every `RepaintBoundary` inside
    /// a scrollable drew outside the scrollable — including the one the
    /// framework inserts by itself for an indeterminate `CircularProgress`.
    ///
    /// # It does not replace the `save`/`clip_rect`/`restore` in `paint`
    ///
    /// Both are needed and they cover disjoint sets, exactly as with
    /// [`layer_effect`](Self::layer_effect): `paint` clips what this scene
    /// records, and this clips what it cannot reach. Unlike an alpha, applying
    /// both to the same pixel twice is harmless — a clip is idempotent — so the
    /// failure mode here is a missing clip and never a doubled one.
    ///
    /// # A rectangle, where the shape may not be one
    ///
    /// A rounded or path clip has to report its **bounding box**, because a
    /// layer carries a rectangle. That is looser than the recorded clip: content
    /// in a corner that the shape would have cut can still show, for a boundary
    /// child only. Strictly better than the whole clip going missing, and the
    /// exact fix is a shape on the layer rather than a different hook here.
    ///
    /// [`RenderViewport`]: crate::RenderViewport
    fn layer_clip(&self, _bounds: Rect) -> Option<Rect> {
        None
    }

    /// The recognisers this object contests a pointer with.
    ///
    /// Built fresh per pointer by the [`PointerRouter`](crate::PointerRouter),
    /// rather than held here: a recogniser carries the state of one gesture in
    /// progress, and two fingers on the same widget are two gestures. Building
    /// them per touch also means a render object replaced mid-gesture does not
    /// silently inherit half of one.
    ///
    /// Returned innermost-first within an object, since that is the order they
    /// enter the arena.
    fn gesture_recognizers(&self) -> Vec<Box<dyn GestureRecognizer>> {
        Vec::new()
    }

    /// A gesture this object's recognisers won.
    ///
    /// `&self`, not `&mut self`: a gesture handler's job is to set a signal, and
    /// the render object is not where the state lives. Taking `&mut` here would
    /// mean the tree is mutably borrowed while callbacks run, which is exactly
    /// when a callback wants to read it.
    fn handle_gesture(&self, gesture: &Recognized, local: Offset) {
        let _ = (gesture, local);
    }

    /// Whether this object can hold the keyboard.
    ///
    /// Focusable objects are what Tab visits and what a tap gives focus to, in
    /// the order [`FocusManager::traversal_order`](crate::FocusManager::traversal_order)
    /// produces. Defaults to `false`: a layout box is not a keyboard target, and
    /// a tree where everything is focusable makes Tab useless.
    ///
    /// Being focusable is not the same as being hit-testable. A field is both; a
    /// button that reacts to Enter is focusable and opaque; a decorative image
    /// is opaque and not focusable.
    fn is_focusable(&self) -> bool {
        false
    }

    /// Whether focus is confined to this object's subtree while it is mounted.
    ///
    /// Declared by the render object rather than pushed and popped imperatively,
    /// because the alternative is a stack that can go out of balance: a dialog
    /// dismissed by a route pop from a button inside it never reaches its own
    /// close path, and the trap it pushed outlives it. A trap that *is* the
    /// subtree cannot survive the subtree — see
    /// [`FocusManager::sync_scopes`](crate::FocusManager::sync_scopes).
    ///
    /// Defaults to `false`. The things that answer `true` are the modals:
    /// dialogs, alerts, sheets and drawers, all through
    /// [`FocusTrap`](vieww_widget::FocusTrap).
    fn traps_focus(&self) -> bool {
        false
    }

    /// A key pressed while this object — or something inside it — has focus.
    ///
    /// Returns whether it was handled. `false` lets the key continue outward to
    /// the ancestors, which is what allows a field to take every ordinary
    /// character while Escape still reaches the dialog around it.
    ///
    /// `&self` for the same reason [`handle_gesture`](Self::handle_gesture) is:
    /// a handler's job is to report the change it wants, not to be where the
    /// state lives. A field turns the key into a new
    /// [`TextEditingValue`](vieww_foundation::TextEditingValue) and hands it
    /// out; whatever owns the value applies it and hands it back down.
    fn handle_key(&self, event: &KeyEvent) -> bool {
        let _ = event;
        false
    }

    /// A transform this object applies to everything beneath it.
    ///
    /// Declared rather than only applied in [`paint`](Self::paint), because
    /// painting is not the only thing that has to know. Hit testing has to undo
    /// it — a finger lands in screen coordinates and the child was drawn
    /// somewhere else — and a subtree that painted itself somewhere its parent
    /// does not expect would otherwise be untouchable.
    ///
    /// Relative to this object's own origin, so a rotation turns about its
    /// top-left rather than the window's.
    fn transform(&self) -> Option<Transform> {
        None
    }

    /// A screen reader asked this object to do something.
    ///
    /// Returns whether it did. The object that *declares* an action is usually
    /// not the one that carries it out — a control's semantics live on the
    /// annotation wrapping it, and the thing that knows how to be pressed is
    /// inside — so this is offered to the declaring object and then to its
    /// subtree until something says yes.
    ///
    /// `&self` for the reason [`handle_gesture`](Self::handle_gesture) is: the
    /// job is to report the change, not to be where the state lives.
    /// `size` is this object's own, because two of the actions are measured in
    /// screens and only the tree knows how big one is.
    fn handle_semantic_action(&self, action: SemanticAction, size: Size) -> bool {
        let _ = (action, size);
        false
    }

    /// The cursor entered or left this object's bounds.
    ///
    /// Only called on objects that answer `true` to
    /// [`wants_hover`](Self::wants_hover), and only when the answer *changes* —
    /// so a control is told once on the way in and once on the way out, rather
    /// than on every pixel the cursor crosses.
    ///
    /// `&self` for the reason [`handle_gesture`](Self::handle_gesture) is.
    fn handle_hover(&self, inside: bool) {
        let _ = inside;
    }

    /// Whether the cursor crossing this object is worth being told about.
    ///
    /// `false` by default and deliberately opt-in: hover is tracked by diffing
    /// the set of objects under the cursor against the last one, and a tree
    /// where every box wanted it would make that diff the size of the tree on
    /// every mouse move.
    fn wants_hover(&self) -> bool {
        false
    }

    /// A wheel or trackpad scroll over this object.
    ///
    /// Returns whether it was consumed. `false` lets it continue outward, which
    /// is what makes a wheel over a list inside a page scroll the list until it
    /// reaches its end — the same bubbling a key gets, and for the same reason.
    ///
    /// Not a gesture: a scroll has no press to be routed from, so it is hit
    /// tested where it lands. See [`ScrollEvent`](vieww_foundation::ScrollEvent).
    fn handle_scroll(&self, event: &ScrollEvent) -> bool {
        let _ = event;
        false
    }

    /// Whether this object's children are in the tree but out of the picture.
    ///
    /// `true` means: do not paint them, do not hit test them, do not give them
    /// layers, and do not announce them to a screen reader. They stay *mounted*,
    /// so their state survives — see
    /// [`RenderOffstage`](crate::RenderOffstage) for why that is the point.
    ///
    /// The tree asks rather than the object acting, because the tree is what
    /// walks children: paint order, repaint boundaries, hit testing and reading
    /// order are all its recursion, not the object's. Layout is the exception
    /// and needs no hook — an object that does not want its child laid out
    /// simply does not call `layout_child`.
    ///
    /// Defaults to `false`. An object that returns `true` and still lays its
    /// children out has built a subtree that costs layout and shows nothing.
    fn skips_children(&self) -> bool {
        false
    }

    /// Whether this object and its subtree can be reached by the pointer.
    ///
    /// `false` means: lay it out and paint it exactly as usual, and let every
    /// click, drag and hover pass straight through to whatever is behind it.
    /// The narrow sibling of [`skips_children`](Self::skips_children), which
    /// takes the subtree out of paint and semantics as well.
    ///
    /// The tree asks rather than the object acting, for the same reason as
    /// `skips_children`: refusing input on behalf of a whole subtree is the
    /// tree's recursion to stop, and a node deep inside it cannot decline for
    /// its ancestors.
    ///
    /// Defaults to `true`. [`RenderIgnorePointer`](crate::RenderIgnorePointer)
    /// is the object that returns `false`, and the case it exists for is a
    /// widget that *explains* a control — a tooltip — which must never be able
    /// to take the pointer away from the control it is explaining.
    fn takes_pointers(&self) -> bool {
        true
    }

    /// Whether this object's subtree is hidden from a screen reader *only*.
    ///
    /// `true` means: lay it out, paint it, hit test it — and publish none of it
    /// to the semantics tree. The narrow sibling of
    /// [`skips_children`](Self::skips_children), which hides a subtree from
    /// everything.
    ///
    /// # Why the two are not the same hook
    ///
    /// Because "visible" and "reachable" come apart, and a navigator is where.
    /// [`Navigator`](vieww_widget::Navigator) keeps the screen one below the top
    /// **onstage**, deliberately: it is what a push slides over and a pop
    /// reveals, and animating against a blank surface instead would be visibly
    /// wrong. That screen must therefore keep painting.
    ///
    /// It must not keep *announcing*. A screen reader walks the semantics tree
    /// rather than the pixels, so a covered screen left in that tree is one a
    /// user can swipe to and activate without ever being able to see it — and
    /// `handle_semantic_action` dispatches by id, so hit testing is not in the
    /// way. Touch is fine; the screen on top absorbs it. This is the path that
    /// is not.
    ///
    /// Defaults to `false`.
    fn hides_semantics(&self) -> bool {
        false
    }

    /// Whether everything painted *before* this object is hidden from a screen
    /// reader.
    ///
    /// The sibling-scoped counterpart of
    /// [`hides_semantics`](Self::hides_semantics), which is subtree-scoped. A
    /// modal barrier is the reason both exist: the screen a dialog covers is
    /// not *under* the dialog in the tree, it is beside it and painted earlier,
    /// so there is no node that wraps the screen and not the dialog. Paint
    /// order is the only relationship the two actually have, and this hook is
    /// how a barrier names it.
    ///
    /// [`ModalBarrier`](vieww_widget::ModalBarrier) already stops taps reaching
    /// that screen. Nothing stopped a screen reader, which walks the semantics
    /// tree rather than the pixels and dispatches by id — so a user could swipe
    /// to a button behind a dialog and press it.
    ///
    /// The flag is reported upward as well as acted on locally: the barrier
    /// sits several levels inside the route that owns it, and the screen it
    /// covers is a sibling of that route. See
    /// [`RenderBlockSemantics`](crate::RenderBlockSemantics).
    ///
    /// Defaults to `false`.
    fn blocks_semantics(&self) -> bool {
        false
    }

    /// This object's share of the main-axis space its parent has left over.
    ///
    /// `None` — the default — means inflexible: the object is laid out against
    /// its natural size and takes no part in the distribution.
    ///
    /// Read by [`RenderFlex`](crate::RenderFlex) off each direct child. This is
    /// where the framework departs from the classic design, which keeps the same
    /// information in a parent-owned slot on each node. See
    /// [`Flexible`](vieww_widget::Flexible) for the trade-off; the short version
    /// is that a transparent wrapper costs one node and avoids a `dyn Any` on
    /// every node in the tree.
    fn flex(&self) -> Option<FlexFactor> {
        None
    }

    /// Where this object should sit within the stack above it, if it says.
    ///
    /// `None` — the default — means an ordinary stack child: it sizes the stack
    /// and is placed by the stack's alignment. `Some` means the opposite on both
    /// counts, and [`RenderStack`](crate::RenderStack) lays it out only after
    /// the unpositioned children have settled the size its edges are measured
    /// against.
    ///
    /// The same seam as [`flex`](Self::flex), for the same reason: a wrapper
    /// carrying one value costs a node, where a parent-owned slot costs a
    /// `dyn Any` slot on every node in the tree. See
    /// [`Positioned`](vieww_widget::Positioned).
    fn stack_position(&self) -> Option<StackPosition> {
        None
    }

    /// What this object is, to a screen reader.
    ///
    /// `None` — the default — means it contributes nothing and is skipped, and
    /// its semantic children are re-parented onto whatever above it does
    /// contribute. That is the right answer for every layout box: a blind user
    /// gains nothing from being told about a `Padding`, and a tree that reported
    /// them all would take a dozen swipes to cross.
    ///
    /// Declared rather than derived, because the information is not in the
    /// geometry. A button is a coloured box with a label in it and a gesture
    /// detector around it; nothing about that shape says "button", and only the
    /// object that meant it knows.
    fn semantics(&self) -> Option<Semantics> {
        None
    }

    /// Whether this object speaks for everything beneath it.
    ///
    /// A button whose label is its own child should be *one* stop for a screen
    /// reader, not two — otherwise every button in the application is announced
    /// twice, once as the button and once as the text inside it. An object that
    /// merges keeps its own [`semantics`](Self::semantics) and discards its
    /// descendants'.
    ///
    /// Only meaningful on an object that declares semantics of its own; an
    /// object declaring nothing has nothing to merge into.
    fn merges_descendant_semantics(&self) -> bool {
        false
    }

    /// An input method event, while this object has focus.
    ///
    /// Returns whether it was handled, and bubbles outward exactly like
    /// [`handle_key`](Self::handle_key) when it is not.
    fn handle_ime(&self, event: &ImeEvent) -> bool {
        let _ = event;
        false
    }

    /// Whether this object wants an input method open on it.
    ///
    /// What makes a platform enable the IME and raise the soft keyboard. A
    /// button is focusable and answers `false`; a text field answers `true`.
    /// Separate from [`is_focusable`](Self::is_focusable) because focus is about
    /// keys and this is about *text*, and on a phone the difference is whether
    /// half the screen is covered.
    fn accepts_text(&self) -> bool {
        false
    }

    /// Where the caret is, in this object's own coordinates.
    ///
    /// Handed to the platform so a candidate window opens next to the text
    /// being composed rather than in the corner of the screen. `None` when
    /// there is nothing to point at.
    fn ime_cursor_area(&self) -> Option<Rect> {
        None
    }

    /// Whether a point inside this object's bounds counts as hitting *it*,
    /// as opposed to merely passing through to a child.
    ///
    /// Defaults to `false`: a plain layout box like padding or a column is
    /// transparent to input, and only its children are targets. Objects that
    /// draw something opaque override it.
    fn hit_test_self(&self, point: Offset, size: Size) -> bool {
        let _ = (point, size);
        false
    }

    /// The area a finger has to land in to reach this object, which is normally
    /// its layout box.
    ///
    /// Overridden by objects that must be *reachable* somewhere they do not
    /// *draw*. `docs/AIMS.md` §I's aim is that a control smaller than the
    /// platform's minimum touch target still takes taps across that minimum —
    /// and the load-bearing decision is that this expands the **hit area and
    /// not the layout box**.
    ///
    /// # Why not simply make small controls bigger
    ///
    /// Because that is a visual change, and an accessibility fix that moves
    /// pixels is one that gets reverted the first time somebody looks at the
    /// screen. A 20×20 icon button that starts occupying 48×48 of a toolbar has
    /// rearranged the toolbar; the same button taking taps within 48×48 while
    /// still drawing 20×20 has cost nothing anybody can see. Reach and layout
    /// are different questions, and this is the one that only concerns reach.
    ///
    /// Returned in this object's own coordinates, so an expanded area has a
    /// negative origin. Overlapping a sibling is allowed and resolves the way
    /// everything else in hit testing does — topmost child first, first hit
    /// wins — so an expansion can never take a tap from something drawn over it.
    fn hit_bounds(&self, size: Size) -> Rect {
        Rect::from_origin_size(Offset::ZERO, size)
    }

    /// Whether replacing `self` with `new` could change geometry.
    ///
    /// Rebuilding a composed widget produces an entirely new widget subtree, so
    /// the sync hands every render object below it a freshly constructed
    /// replacement — even where nothing meaningful changed. Without this check
    /// each rebuild would mark the whole subtree for relayout and the relayout
    /// boundary would never get the chance to do its job.
    ///
    /// This is the classic padding-setter early return — a setter returning early when
    /// the value is unchanged, expressed as one method instead of a setter per
    /// field. Most implementations are [`layout_differs_by_eq`]; a render
    /// object whose configuration only affects *painting* returns `false`.
    ///
    /// Defaults to `true`, which is always correct and merely slow.
    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        let _ = new;
        true
    }

    /// `true` if this object's configuration is read by its **parent** during
    /// the parent's layout, rather than used by the object itself.
    ///
    /// The parent-data idea, and the reason it needs saying: layout dirtying
    /// walks *upward* only as far as the nearest relayout boundary, and a child
    /// under tight constraints is one. So a `Positioned` whose offset changed
    /// marked itself, re-laid itself out against the same constraints, and
    /// produced the same box — while the `Stack` that decides *where* to put it
    /// kept the placement it had cached. The pixels did not move.
    ///
    /// That is exactly what happened to a segmented control whose selection
    /// slid off the right edge of its own track: the widget rebuilt with the
    /// right number, was instrumented and printed the right number, and drew in
    /// the old place. Nothing failed, nothing logged, and the geometry was
    /// correct everywhere except on screen.
    ///
    /// An object that answers `true` here has its parent marked for layout
    /// whenever its own configuration changes. Two implement it —
    /// [`RenderPositioned`](crate::RenderPositioned), whose position a
    /// [`RenderStack`](crate::RenderStack) reads, and
    /// [`RenderFlexible`](crate::RenderFlexible), whose factor and fit a
    /// [`RenderFlex`](crate::RenderFlex) reads. Both are layout-transparent by
    /// design, which is why neither could dirty anything by itself.
    fn parent_reads_configuration(&self) -> bool {
        false
    }

    /// Take over whatever the object being replaced had computed during layout.
    ///
    /// The counterpart to [`layout_differs`](RenderObject::layout_differs), and
    /// required by it. A rebuild constructs a brand new object and swaps it in;
    /// when `layout_differs` says the geometry cannot have moved, nothing lays
    /// the replacement out — so anything the old object had *cached from layout*
    /// is gone and will not be recomputed. For [`RenderText`](crate::RenderText)
    /// that cache is the shaped paragraph, and losing it means the text is simply
    /// not drawn: a rebuild anywhere above a label makes the label vanish until
    /// something else forces a relayout.
    ///
    /// Only objects that cache layout results need this. Implementations must
    /// check that the replacement is *equivalent* before adopting anything —
    /// otherwise a changed object draws with the old one's results.
    ///
    /// `self` is the incoming object and `old` the one being replaced, which is
    /// the opposite way round from `layout_differs`, because this one mutates.
    fn adopt_layout_cache(&mut self, old: &dyn RenderObject) {
        let _ = old;
    }

    /// Carry across a record of something the old object already **did**.
    ///
    /// The sibling of [`adopt_layout_cache`](Self::adopt_layout_cache), and the
    /// distinction is which way the value flows. A layout cache is an *input*
    /// that layout would otherwise recompute, so it is adopted only when nothing
    /// will lay the replacement out. This is an *output* — a note that a side
    /// effect has already happened — and losing it does not cause a recompute,
    /// it causes the side effect to happen twice. So it is adopted on **every**
    /// replacement, geometry changed or not.
    ///
    /// [`RenderViewport`](crate::RenderViewport) is the reason it exists. It
    /// reports its extents out of layout and remembers what it last sent, so
    /// that unchanged extents are never reported twice — the handler on the
    /// other end writes a signal, and `Signal::set` marks its readers pending
    /// whether or not the value changed. Carrying that note only when the
    /// geometry agreed meant dropping it on precisely the frames that matter,
    /// the scrolled ones, and a scrolling application could never go idle.
    ///
    /// `self` is the incoming object and `old` the one being replaced.
    fn adopt_reports(&mut self, old: &dyn RenderObject) {
        let _ = old;
    }

    /// The pointer shape to draw while the cursor is over this object.
    ///
    /// `None` means "no opinion", which is the right answer for almost
    /// everything: a column has nothing to say about the cursor, and one that
    /// answered [`Cursor::Default`] would *override* the text field inside it.
    /// The distinction between "no opinion" and "explicitly the arrow" is the
    /// whole reason this returns an `Option`.
    ///
    /// Resolved innermost-first from the hit test, so the deepest object with
    /// an opinion wins — the same rule focus uses.
    ///
    /// `local` is the pointer in this object's own coordinates, the same value
    /// [`hit_test_self`](Self::hit_test_self) and
    /// [`handle_gesture`](Self::handle_gesture) receive. Almost nothing needs
    /// it — a text field is an I-beam everywhere — but an object whose cursor
    /// varies *within* it does:
    /// [`RenderRichText`](crate::RenderRichText) shows a hand over a link and an
    /// I-beam over the prose around it, and a paragraph that showed a hand
    /// everywhere because one word in it is a link tells the reader the wrong
    /// thing about the other six.
    fn cursor(&self, local: Offset) -> Option<Cursor> {
        let _ = local;
        None
    }

    /// Name used in tree dumps.
    fn debug_name(&self) -> &'static str;

    /// The properties a reader would want to see for this object.
    ///
    /// # What this is for
    ///
    /// A widget inspector. An application holds a *widget* tree and never a
    /// render one, so until this existed there was no way for a studio, a
    /// debugger or a test to ask what the thing on screen actually is — the
    /// studio's Inspector view was five lines of buffer statistics with the
    /// word "Inspector" over them, and could not have been more, because
    /// nothing in the framework would answer.
    ///
    /// Pairs with [`RenderTree::describe`](crate::RenderTree::describe), which
    /// adds the facts every object has — name, size, offset, child count — so
    /// this only has to carry what is particular to *this* object.
    ///
    /// # Why strings
    ///
    /// Because the consumer is a person reading a list. A typed property bag
    /// would need a vocabulary covering every field of every render object in
    /// the framework and every one anybody adds, which is a maintenance burden
    /// paid for a formatting decision the caller then makes anyway. The classic
    /// diagnostics node is the same shape for the same reason.
    ///
    /// Empty by default, so an object that adds nothing costs nothing and no
    /// existing implementation had to change.
    fn describe(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }
}

/// [`RenderObject::layout_differs`] for a render object whose layout is decided
/// entirely by its own `PartialEq` fields.
///
/// A different concrete type always counts as differing — there is nothing
/// sensible to compare.
#[must_use]
pub fn layout_differs_by_eq<T>(old: &T, new: &dyn RenderObject) -> bool
where
    T: RenderObject + PartialEq,
{
    let any: &dyn Any = new;
    any.downcast_ref::<T>().is_none_or(|new| old != new)
}

/// Handed to [`RenderObject::layout`]; the only way to lay out a child.
///
/// It borrows the tree mutably, which is what stops a render object holding a
/// reference to a child across its own layout — the borrow checker enforcing
/// the protocol rather than a convention doing it.
#[derive(Debug)]
pub struct LayoutCtx<'a> {
    pub(crate) tree: &'a mut RenderTree,
    pub(crate) id: RenderId,
}

impl LayoutCtx<'_> {
    /// This object's children, in paint order.
    ///
    /// Borrowed from the tree rather than copied out of it. Most of the thirty-odd
    /// call sites only want the first child or the count, and every one of them
    /// used to allocate a `Vec` per node per layout pass — paid on every frame a
    /// scroll dirties, for a list that is thrown away three lines later.
    ///
    /// The borrow lasts as long as the slice, so an object that lays its children
    /// out *while* walking them cannot use this: `layout_child` needs the tree
    /// mutably. Those call sites — the multi-child ones, [`RenderFlex`], `Stack`,
    /// the sliver list and the scroll view — take [`children_owned`] instead, and
    /// they are the only ones that still allocate.
    ///
    /// [`RenderFlex`]: crate::RenderFlex
    /// [`children_owned`]: Self::children_owned
    #[must_use]
    pub fn children(&self) -> &[RenderId] {
        self.tree.children(self.id)
    }

    /// This object's children, copied out.
    ///
    /// For laying children out in a loop: the copy is what releases the tree so
    /// each iteration can borrow it mutably. Prefer [`children`](Self::children)
    /// anywhere the list is only read.
    #[must_use]
    pub fn children_owned(&self) -> Vec<RenderId> {
        self.tree.children(self.id).to_vec()
    }

    /// How many children this object has.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.tree.children(self.id).len()
    }

    /// Lay a child out and get the size it chose.
    pub fn layout_child(&mut self, child: RenderId, constraints: Constraints) -> Size {
        self.tree.layout(child, constraints)
    }

    /// Place a child, relative to this object's own top-left.
    pub fn place_child(&mut self, child: RenderId, offset: Offset) {
        self.tree.set_offset(child, offset);
    }

    /// Lay a child out under the **sliver** protocol and get its geometry.
    ///
    /// A child that does not speak the protocol is measured with box
    /// constraints and adapted — see
    /// [`RenderObject::layout_sliver`]. So a viewport never has to ask what
    /// kind of child it has.
    pub fn layout_sliver_child(
        &mut self,
        child: RenderId,
        constraints: &SliverConstraints,
    ) -> SliverGeometry {
        self.tree.layout_sliver(child, constraints)
    }

    /// The size a child settled on. Only meaningful after laying it out.
    #[must_use]
    pub fn child_size(&self, child: RenderId) -> Size {
        self.tree.size(child)
    }

    /// Where a child's first line of text sits, measured down from that
    /// child's own top-left. Only meaningful after laying it out.
    ///
    /// `None` means the child holds no text to align to; see
    /// [`crate::baseline`] for what a caller should do about that.
    pub fn child_baseline(&mut self, child: RenderId) -> Option<f32> {
        self.tree.baseline(child)
    }

    /// A child's flex factor, if it declared one.
    ///
    /// Only meaningful for a direct child — see
    /// [`Flexible`](vieww_widget::Flexible).
    #[must_use]
    pub fn child_flex(&self, child: RenderId) -> Option<FlexFactor> {
        self.tree.object(child).and_then(RenderObject::flex)
    }

    /// A child's stack position, if it declared one.
    ///
    /// Only meaningful for a direct child — see
    /// [`Positioned`](vieww_widget::Positioned).
    #[must_use]
    pub fn child_stack_position(&self, child: RenderId) -> Option<StackPosition> {
        self.tree
            .object(child)
            .and_then(RenderObject::stack_position)
    }

    /// Ask a child how big it *wants* to be, without laying it out.
    ///
    /// The one bridge from layout into the intrinsic protocol, for the render
    /// objects — `RenderIntrinsicSize`, and any layout whose own constraint
    /// depends on a child's natural size — that need to measure before they
    /// decide. `None` means the subtree cannot answer; see
    /// `crate::intrinsics` for why that is not zero.
    ///
    /// Calling this for every child of a large list is how an O(n) layout
    /// becomes an O(n·depth) one. It is here because some layouts cannot be
    /// written without it, not because it is cheap.
    pub fn child_intrinsic(
        &mut self,
        child: crate::RenderId,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        self.tree.intrinsic(child, query)
    }

    /// The fonts to shape text against.
    ///
    /// Text is the one thing whose size cannot be computed from its
    /// configuration alone — it takes a font to know how wide a word is.
    pub fn fonts_mut(&mut self) -> &mut vieww_text::FontStore {
        self.tree.fonts_mut()
    }
}

/// Handed to [`RenderObject::paint`].
pub struct PaintCtx<'a> {
    pub(crate) canvas: &'a mut dyn Canvas,
    pub(crate) origin: Offset,
    pub(crate) size: Size,
    /// How many physical pixels one logical one is worth, on the surface this
    /// frame is destined for.
    ///
    /// A leaf that wants to sit on the pixel grid — a stroked icon, a hairline,
    /// a caret — cannot do it in logical points alone: at a fractional display
    /// scale a whole logical pixel is a fractional number of physical ones, and
    /// a line whose edges straddle two physical pixels each is what "smeared"
    /// means. The ratio is what turns "round to the nearest pixel" into a
    /// statement about the pixels that actually exist. It defaults to one, so a
    /// test that does not care snaps exactly as a 1:1 display would.
    pub(crate) dpr: f32,
}

impl fmt::Debug for PaintCtx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `dyn Canvas` is not Debug and should not have to be — a backend is
        // free to be a GPU handle.
        f.debug_struct("PaintCtx")
            .field("origin", &self.origin)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl PaintCtx<'_> {
    /// This object's top-left in global coordinates.
    #[must_use]
    pub const fn origin(&self) -> Offset {
        self.origin
    }

    /// This object's size, as decided by layout.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// This object's bounds in global coordinates.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        Rect::from_origin_size(self.origin, self.size)
    }

    /// Physical pixels per logical pixel on this frame's surface.
    ///
    /// See the field's docs for why a paint context carries it at all.
    #[must_use]
    pub const fn device_pixel_ratio(&self) -> f32 {
        self.dpr
    }

    /// The surface to draw on.
    pub fn canvas(&mut self) -> &mut dyn Canvas {
        self.canvas
    }
}

/// The chain of render objects under a point, **innermost first**.
///
/// Children are visited in reverse paint order, so the topmost thing under the
/// point is found first; entries are recorded while unwinding, so the list runs
/// from that target outwards to the root. Phase 6 dispatches along it in order,
/// which is how a tap on a button inside a scrollable reaches the button before
/// the scroller.
#[derive(Debug, Clone, Default)]
pub struct HitTestResult {
    entries: Vec<HitTestEntry>,
}

/// One render object hit by a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitTestEntry {
    /// The object that was hit.
    pub id: RenderId,
    /// The point, expressed in that object's local coordinates.
    pub local: Offset,
}

impl HitTestResult {
    /// An empty result.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, id: RenderId, local: Offset) {
        self.entries.push(HitTestEntry { id, local });
    }

    /// Every object hit, innermost first.
    #[must_use]
    pub fn entries(&self) -> &[HitTestEntry] {
        &self.entries
    }

    /// The topmost object under the point — the first that should be offered
    /// the gesture.
    #[must_use]
    pub fn target(&self) -> Option<HitTestEntry> {
        self.entries.first().copied()
    }

    /// `true` if nothing was hit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of objects hit.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The hit objects from the topmost outwards, which is the order a gesture
    /// should be offered to them.
    pub fn bubble(&self) -> impl Iterator<Item = &HitTestEntry> {
        self.entries.iter()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use vieww_foundation::{Constraints, Offset, Size};

    use super::*;
    use crate::RenderConstrainedBox;

    /// Reads its children, then lays them out through the same context.
    ///
    /// The pair is the point: `children()` hands back a borrow of the tree, and
    /// `layout_child` needs the tree mutably, so a probe that only read would
    /// not exercise the case every multi-child object is.
    #[derive(Debug)]
    struct Probe(Rc<RefCell<Vec<RenderId>>>);

    impl RenderObject for Probe {
        fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
            self.0.borrow_mut().clear();
            self.0.borrow_mut().extend_from_slice(ctx.children());
            for child in ctx.children_owned() {
                ctx.layout_child(child, constraints);
                ctx.place_child(child, Offset::ZERO);
            }
            constraints.smallest()
        }

        fn debug_name(&self) -> &'static str {
            "Probe"
        }
    }

    #[test]
    fn an_object_sees_its_children_in_the_order_the_tree_holds_them() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut tree = RenderTree::new();
        let parent = tree.insert(None, Box::new(Probe(Rc::clone(&seen))));
        let kids: Vec<RenderId> = (0..4)
            .map(|_| {
                tree.insert(
                    Some(parent),
                    Box::new(RenderConstrainedBox::new(Constraints::tight(Size::square(
                        10.0,
                    )))),
                )
            })
            .collect();

        let _ = tree.layout_root(Constraints::tight(Size::square(100.0)));

        assert_eq!(*seen.borrow(), kids, "in paint order, all of them");
        assert_eq!(tree.children(parent), kids.as_slice());
    }

    #[test]
    fn the_borrowed_children_and_the_copied_ones_are_the_same_list() {
        let mut tree = RenderTree::new();
        let parent = tree.insert(None, Box::new(Probe(Rc::new(RefCell::new(Vec::new())))));
        for _ in 0..3 {
            tree.insert(
                Some(parent),
                Box::new(RenderConstrainedBox::new(Constraints::tight(Size::square(
                    10.0,
                )))),
            );
        }

        let ctx = LayoutCtx {
            tree: &mut tree,
            id: parent,
        };
        assert_eq!(ctx.children(), ctx.children_owned().as_slice());
        assert_eq!(ctx.children().len(), ctx.child_count());
    }
}
