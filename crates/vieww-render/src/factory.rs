//! Mapping widgets to render objects, without the widget layer knowing that
//! render objects exist.
//!
//! Putting `createRenderObject` on the widget — as some designs do — makes `widgets/`
//! depend on `rendering/`. `docs/DESIGN.md` §7 forbids that direction here: the
//! widget layer is a cheap description that knows nothing about pixels, and
//! keeping it that way is what lets Phase 1 be tested with no render deps at
//! all.
//!
//! So the lookup is inverted. A [`RenderFactory`] maps a widget's [`TypeId`] to
//! a constructor, and the render layer registers the built-ins it knows about.
//! The cost is that a *user's* render widget has to be registered rather than
//! just implementing a trait — an explicit call in one place, in exchange for a
//! widget layer that stays portable.
//!
//! **That call is [`FrameDriver::register`](crate::FrameDriver::register)**, and
//! naming it here is not decoration. This paragraph promised the extension for a
//! long time while the only public routes to a registry were this type and
//! `RenderOwner` — neither of which an application can reach, since
//! `FrameDriver::owner` returns `&RenderOwner` and the platform crate builds the
//! driver itself. The mechanism was built, public and unit-tested, and unusable
//! from outside; `crates/vieww/tests/third_party_render_widget.rs` is the test
//! that now stands where an application stands and would have caught it.

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Axis, Clipboard, SharedServices, TextDirection, TextStyle};

// `Image` is aliased because `vieww_foundation::Image` — the pixels — is what
// `RenderImage` actually holds. Both names in scope unaliased reads as a mistake
// even when it compiles.
use vieww_widget::{
    Align, AspectRatio, BlockSemantics, CircularProgressArc, Clip, ClipShape as WidgetClipShape,
    ColoredBox, Constrained, CursorArea, CustomScrollView, DecoratedBox, ExcludeSemantics,
    Filtered, Fitted, Flex, Flexible, FocusTrap, GestureDetector, Icon, IgnorePointer,
    Image as ImageWidget, InheritedScope, IntrinsicSize, Measured, MeasuredConstraints, Metrics,
    Offstage, Opacity, Padding, Painting, PerformanceOverlay, Positioned, RepaintBoundary,
    RichText, SemanticLiveness, SemanticRole, Semantics, SensitiveMask, SliderBar, SliverAppBar,
    SliverList, SliverRefresh, Stack, Svg, Text, TextField, ThemeData, Transformed, Viewport,
    Widget, WidgetNode,
};

use crate::objects::{
    ClipShape, HeaderBehaviour, RenderAlign, RenderAspectRatio, RenderBlockSemantics,
    RenderCircularProgress, RenderClip, RenderColoredBox, RenderConstrainedBox, RenderCursorArea,
    RenderDecoratedBox, RenderEditableText, RenderExcludeSemantics, RenderFilter, RenderFitted,
    RenderFlex, RenderFlexible, RenderFocusTrap, RenderGestureDetector, RenderIcon,
    RenderIgnorePointer, RenderImage, RenderIntrinsicSize, RenderMeasured,
    RenderMeasuredConstraints, RenderOffstage, RenderOpacity, RenderPadding, RenderPainting,
    RenderPerformanceOverlay, RenderPositioned, RenderRepaintBoundary, RenderRichText,
    RenderScrollView, RenderSemantics, RenderSensitive, RenderSlider, RenderSliverAppBar,
    RenderSliverFixedList, RenderSliverRefresh, RenderStack, RenderText, RenderTransform,
    RenderVectorImage, RenderViewport,
};
use crate::{Liveness, RenderObject, Role};

/// Every creator takes the scope, and [`RenderFactory::register`] discards it.
///
/// One stored shape rather than two, so `create` has a single path. The cost is
/// that a context-free creator is handed something it ignores; the alternative
/// is an enum and a branch on every render object realised.
type CreateFn = Box<dyn Fn(&WidgetNode, &InheritedScope) -> Box<dyn RenderObject>>;

/// The reading direction published to this position, or [`TextDirection::Ltr`].
///
/// Shared by every built-in that reads it — [`Flex`], [`Stack`], [`Viewport`]
/// and [`CustomScrollView`] — so that "absent means left-to-right" is decided
/// once. A second copy of this that picked a different fallback would be a bug
/// visible only in a subtree with no `Directionality` above it, which is most
/// tests and no real application.
fn ambient_direction(scope: &InheritedScope) -> TextDirection {
    scope
        .get::<TextDirection>()
        .map(|direction| *direction)
        .unwrap_or_default()
}

/// The pasteboard published to this position, if the application provided one.
///
/// Through the same `SharedServices` an application uses for storage and deep
/// links, rather than a channel of its own — a text field is a *consumer* of a
/// platform service exactly as anything else is, and inventing a second route
/// for it would mean a third when the next widget needs one.
///
/// `None` where nothing was provided. That is an ordinary tree — every test in
/// this repository, for one — and the field simply reports cut, copy and paste
/// unhandled.
fn ambient_clipboard(scope: &InheritedScope) -> Option<Rc<dyn Clipboard>> {
    scope.get::<SharedServices>()?.get::<dyn Clipboard>()
}

/// The accessibility preferences in force here, or the defaults.
///
/// Falls back to [`Accessibility::default`](vieww_foundation::Accessibility::default)
/// for exactly the reason
/// [`ambient_touch_target`] falls back to `Metrics::new()`: a tree built
/// without a driver above it — every unit test in this repository — must still
/// produce text, and the default is `text_scale: 1.0`, which changes nothing.
fn ambient_accessibility(scope: &InheritedScope) -> vieww_foundation::Accessibility {
    scope
        .get::<vieww_foundation::Accessibility>()
        .map_or_else(Default::default, |a| *a)
}

/// `style` with the OS text-size setting applied.
///
/// # Why this is in the factory rather than in `Text`
///
/// Because it must catch **every** path that produces a text render object,
/// including the ones written later by somebody who has never read this file.
/// A `Text` widget that scaled its own size would be one call site; a helper
/// that a widget is expected to call would be a rule to remember. This is the
/// single funnel every `Text` already passes through, which is the same
/// argument `docs/AIMS.md` §J makes for deriving announcements from the tree
/// rather than asking each widget to post one.
/// The ambient theme's body style, or the default one when there is no theme.
///
/// Falls back for the reason [`ambient_touch_target`] does: a tree built without
/// a `Theme` — every unit test here, and any application that has not got round
/// to theming — must still render readable text rather than silently opting out.
fn ambient_body(scope: &InheritedScope) -> TextStyle {
    scope
        .get::<ThemeData>()
        .map_or_else(TextStyle::default, |theme| theme.text.body)
}

fn scaled(style: TextStyle, scope: &InheritedScope) -> TextStyle {
    let mut style = style;
    style.size = ambient_accessibility(scope).scale_text(style.size);
    style
}

/// The theme's minimum touch target, or the default one when there is no theme.
///
/// Falling back to `Metrics::new()` rather than to zero is the decision that
/// makes `docs/AIMS.md` §I's aim true rather than nearly true: a tree built
/// without a `Theme` — every unit test in this repository, and any application
/// that has not got round to theming yet — would otherwise silently opt out of
/// the whole feature. The safe direction for reach is to have it.
fn ambient_touch_target(scope: &InheritedScope) -> f32 {
    scope.get::<ThemeData>().map_or_else(
        || Metrics::new().touch_target,
        |theme| theme.metrics.touch_target,
    )
}

/// Maps widget types to the render objects that realise them.
pub struct RenderFactory {
    creators: HashMap<TypeId, CreateFn>,
}

impl RenderFactory {
    /// An empty factory that knows about nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            creators: HashMap::new(),
        }
    }

    /// A factory with every built-in widget registered.
    #[must_use]
    pub fn with_builtins() -> Self {
        let mut factory = Self::empty();

        factory.register::<Padding, _>(|widget| RenderPadding::new(widget.insets()));
        factory.register::<IntrinsicSize, _>(|widget| RenderIntrinsicSize::new(widget.axis()));
        factory.register::<Align, _>(|widget| {
            let mut object = RenderAlign::new(widget.alignment());
            object.width_factor = widget.width_factor_value();
            object.height_factor = widget.height_factor_value();
            object
        });
        factory
            .register::<Constrained, _>(|widget| RenderConstrainedBox::new(widget.constraints()));
        factory.register::<ColoredBox, _>(|widget| RenderColoredBox::new(widget.color()));
        factory.register::<ImageWidget, _>(|widget| {
            // The pixels are an `Rc` inside `Image`, so this clone is a refcount
            // bump rather than a copy of the buffer — which matters, because
            // this runs every time the widget is rebuilt.
            let mut object = RenderImage::new(widget.data().clone());
            object.fit = widget.box_fit();
            object.alignment = widget.image_alignment();
            object.label = widget.image_label().map(str::to_owned);
            object.width = widget.image_width();
            object.height = widget.image_height();
            object
        });
        factory.register::<PerformanceOverlay, _>(|widget| {
            let mut object =
                RenderPerformanceOverlay::new(widget.samples().to_vec(), widget.budget());
            if let Some(height) = widget.overlay_height() {
                object.height = height;
            }
            object
        });
        factory.register::<DecoratedBox, _>(|widget| RenderDecoratedBox::new(widget.decoration()));
        factory.register_with_context::<Text, _>(|widget, scope| {
            RenderText::new(widget.data(), scaled(*widget.text_style(), scope))
                .align(widget.text_align())
                .direction(widget.text_direction())
                .overflow(widget.text_overflow())
                .max_lines(widget.line_limit())
        });
        factory.register_with_context::<RichText, _>(|widget, scope| {
            // The base a span inherits what it does not override: the caller's
            // if it gave one, otherwise the ambient theme's body style — and
            // scaled, so a rich paragraph honours the OS text-size setting for
            // the same reason and through the same funnel every `Text` does.
            let base = scaled(
                widget.base_style().unwrap_or_else(|| ambient_body(scope)),
                scope,
            );
            let spans = widget
                .spans()
                .iter()
                .map(|span| vieww_text::TextSpan::new(span.text().to_owned(), span.resolved(base)))
                .collect();
            let links = widget
                .spans()
                .iter()
                .map(vieww_widget::Span::handler)
                .collect();
            RenderRichText::new(spans)
                .with_links(links)
                .align(widget.alignment())
                .direction(widget.base_direction())
        });
        factory.register_with_context::<TextField, _>(|widget, scope| {
            let mut editable = RenderEditableText::new(
                widget.editing_value().clone(),
                scaled(*widget.text_style(), scope),
            )
            .align(widget.text_align())
            .direction(widget.text_direction())
            .cursor(widget.cursor_color(), widget.cursor_width())
            .show_cursor(widget.cursor_shown())
            .obscure(widget.obscuring())
            .clipboard(ambient_clipboard(scope))
            .wrap(widget.wraps())
            .spans(widget.text_spans().map(|spans| Rc::new(spans.to_vec())));
            if let Some((lines, color)) = widget.diagnostic_markers() {
                editable = editable.diagnostics(Rc::clone(lines), color);
            }
            if let Some(decorations) = widget.text_decorations() {
                editable = editable.decorations(Rc::clone(decorations));
            }
            if let Some(probe) = widget.layout_probe() {
                editable = editable.probe(probe.clone());
            }
            if !widget.placeholder_text().is_empty() {
                // The fallback is the theme's, resolved by the widget before it
                // gets here — a placeholder is a hint, and a colour chosen in
                // the render layer could not match the theme it lands in.
                let color = widget
                    .placeholder_shade()
                    .unwrap_or_else(|| widget.text_style().color);
                editable = editable.placeholder(widget.placeholder_text(), color);
            }
            if let Some(color) = widget.selection_highlight() {
                editable = editable.selection_color(color);
            }
            if widget.is_single_line() {
                // Passed even when the handler is `None`: being single-line is
                // what stops `Enter` opening a line, and a field that declared
                // itself one but grew anyway because nobody was listening would
                // be the worst of both.
                editable = editable.single_line(widget.submit_handler().map(Rc::clone));
            }
            if let Some(handler) = widget.changed_handler() {
                // The two inputs report different things. A key can change the
                // text, the selection and the composing region at once, so it
                // reports the whole value and there is nothing to assemble. A
                // pointer can only move the caret, so it reports a *selection*.
                //
                // **Where that selection goes depends on what the caller asked
                // for, and the difference is not cosmetic.**
                //
                // With `on_selection` set, it goes there — carrying no text,
                // so the pointer path cannot express a text change at all.
                //
                // Without it, the widget assembles a value around the selection
                // out of *its own* text, which is the tree as of the last
                // build. `RenderEditableText` notes in its own docs that
                // several drag updates can arrive within one frame, and every
                // one of them assembles from that same string — so a drag can
                // hand the caller text the caller has already moved past, and a
                // caller that applies what it is given has just had a selection
                // gesture edit its document. That is kept as the fallback
                // because every existing field relies on it and none of them
                // holds a value that moves under a drag; anything that does
                // should set `on_selection`.
                if let Some(selection_handler) = widget.selection_handler() {
                    editable = editable.on_selection_changed(Rc::clone(selection_handler));
                } else {
                    let widget = widget.clone();
                    let handler = Rc::clone(handler);
                    editable = editable.on_selection_changed(Rc::new(move |selection| {
                        handler(widget.with_selection(selection));
                    }));
                }
                // And a modified click, when the caller has somewhere to put it.
                // Left unset otherwise, which leaves `RenderEditableText`
                // treating a ⌘-click as a plain one.
                if let Some(add) = widget.add_caret_handler() {
                    editable = editable.on_add_caret(Rc::clone(add));
                }
                editable = editable.on_changed(Rc::clone(handler));
            }
            editable
        });
        factory.register::<Icon, _>(|widget| {
            let mut object = RenderIcon::new(
                widget.data().clone(),
                widget.icon_size(),
                widget.icon_color(),
            );
            object.stroke = widget.stroke_width();
            match widget.icon_label() {
                Some(label) => object.label(label),
                None => object,
            }
        });
        factory.register::<Painting, _>(|widget| {
            // `Rc::clone`, not a copy: a painter can carry a whole scene, and
            // it is rebuilt on every rebuild of the region that shows it.
            RenderPainting::new(Rc::clone(widget.painter()), widget.requested_size())
        });
        factory.register::<Svg, _>(|widget| {
            // The shapes are plain `Vec`/`String` data behind `VectorImage`,
            // not an `Rc` — this clone is a real copy, same cost `Icon` would
            // pay if `IconData` were not `Rc`-backed. Left as-is rather than
            // wrapping `VectorImage` in an `Rc` pre-emptively: nothing yet
            // measures this as a hot path, and `docs/AIMS.md`'s own rule for
            // *Cheaper* is a counted measurement, not an argument.
            let mut object = RenderVectorImage::new(widget.data().clone());
            object.width = widget.svg_width();
            object.height = widget.svg_height();
            object.label = widget.svg_label().map(str::to_owned);
            object
        });
        // The one built-in that reads ambient state. A flex mirrors its
        // horizontal axis, and which way that runs is a property of the screen
        // rather than of this widget — so an explicit setting on the `Flex`
        // wins, and otherwise the nearest `Directionality` above it decides.
        factory.register_with_context::<Flex, _>(|widget, scope| {
            let mut flex = RenderFlex::new(widget.direction());
            flex.main_axis_alignment = widget.main_axis();
            flex.cross_axis_alignment = widget.cross_axis();
            flex.main_axis_size = widget.axis_size();
            flex.spacing = widget.child_spacing();
            flex.text_direction = widget
                .reading_direction()
                .unwrap_or_else(|| ambient_direction(scope));
            flex
        });
        factory.register::<Fitted, _>(|_| RenderFitted::new());
        // Layout-transparent; the value exists for `RenderStack` to read
        // back off the child, the same way `Flexible` carries a factor.
        factory.register::<Positioned, _>(|widget| RenderPositioned::new(widget.position()));
        // The widget keeps whatever ratio it was handed, so a dump can show a
        // mistake; `RenderAspectRatio::new` is where a non-positive or
        // non-finite one is substituted, once, rather than on every layout.
        factory.register::<AspectRatio, _>(|widget| RenderAspectRatio::new(widget.ratio()));
        factory.register::<Flexible, _>(|widget| RenderFlexible::new(widget.factor()));
        factory.register::<Transformed, _>(|widget| RenderTransform::new(widget.matrix()));
        factory.register::<Measured, _>(|widget| {
            RenderMeasured::new().on_measured(widget.measured_handler())
        });
        // `LayoutBuilder` is not here, and does not belong here: it is
        // `Composed`, so it builds this around whatever its closure returned.
        factory.register::<MeasuredConstraints, _>(|widget| {
            RenderMeasuredConstraints::new().on_constraints(widget.constraints_handler())
        });
        factory.register::<Offstage, _>(|widget| RenderOffstage::new(widget.is_offstage()));
        factory.register::<SensitiveMask, _>(|widget| {
            RenderSensitive::new(widget.is_masked(), widget.cover_color())
                .with_label(widget.mask_label())
        });
        factory.register::<ExcludeSemantics, _>(|widget| {
            RenderExcludeSemantics::new(widget.is_excluded())
        });
        factory.register::<BlockSemantics, _>(|widget| {
            RenderBlockSemantics::new(widget.is_blocking())
        });
        factory.register::<FocusTrap, _>(|widget| RenderFocusTrap::new(widget.is_trapping()));
        factory.register::<CursorArea, _>(|widget| RenderCursorArea::new(widget.shape()));
        factory
            .register::<IgnorePointer, _>(|widget| RenderIgnorePointer::new(widget.is_ignoring()));
        factory.register::<Opacity, _>(|widget| {
            RenderOpacity::new(widget.alpha()).blended(widget.blend_mode())
        });
        factory.register::<Filtered, _>(|widget| RenderFilter::new(widget.filter()));
        factory.register::<Clip, _>(|widget| RenderClip::new(to_clip_shape(widget.shape())));
        // The sliver scroller, on the same rule as `Viewport`: a horizontal one
        // in a right-to-left interface runs from the right edge.
        factory.register_with_context::<CustomScrollView, _>(|widget, scope| {
            RenderScrollView::new(widget.axis())
                .offset(widget.scroll_offset())
                .reversed(
                    widget.axis() == Axis::Horizontal
                        && ambient_direction(scope) == TextDirection::Rtl,
                )
        });
        factory.register::<SliverList, _>(|widget| {
            RenderSliverFixedList::new(widget.count(), widget.item_extent())
                .starting_at(widget.first_index())
        });
        factory.register::<SliverAppBar, _>(|widget| {
            let (max, min) = widget.extents();
            let behaviour = if widget.is_pinned() {
                HeaderBehaviour::Pinned
            } else if widget.is_floating() {
                HeaderBehaviour::Floating
            } else {
                HeaderBehaviour::Scrolling
            };
            RenderSliverAppBar::new(max, min).behaviour(behaviour)
        });
        factory.register::<SliverRefresh, _>(|widget| {
            RenderSliverRefresh::new(widget.trigger_extent()).refreshing(widget.is_refreshing())
        });
        factory.register::<SliderBar, _>(|widget| {
            let bar = RenderSlider::new(widget.value, widget.min, widget.max)
                .divisions(widget.divisions)
                .height(widget.height)
                .press(widget.press)
                .geometry(
                    widget.track_thickness,
                    widget.thumb_radius,
                    widget.thumb_ring,
                )
                .colors(widget.active, widget.inactive, widget.thumb);
            match &widget.on_changed {
                Some(handler) => bar.on_changed(Rc::clone(handler)),
                None => bar,
            }
        });
        factory.register::<CircularProgressArc, _>(|widget| {
            RenderCircularProgress::new(widget.value, widget.diameter, widget.thickness)
                .phase(widget.phase)
                .spokes(widget.spokes)
                .colors(widget.track, widget.indicator)
        });
        factory.register::<RepaintBoundary, _>(|_| RenderRepaintBoundary::new());
        // A horizontal viewport in a right-to-left interface starts at the right
        // edge. Only the anchor is decided here — the scroll offset stays a
        // distance into the content, so the physics never learn about direction.
        //
        // A *vertical* viewport never reverses: no script this framework targets
        // runs bottom to top, which is the same rule `RenderFlex` follows for a
        // column's main axis.
        factory.register_with_context::<Viewport, _>(|widget, scope| {
            let viewport = RenderViewport::new(widget.axis(), widget.scroll_offset()).reversed(
                widget.axis() == Axis::Horizontal && ambient_direction(scope) == TextDirection::Rtl,
            );
            match widget.extents_handler() {
                Some(handler) => viewport.on_extents(Rc::clone(handler)),
                None => viewport,
            }
        });
        // The minimum comes from the *theme*, read here rather than baked in,
        // because it is a platform figure — 48 on Android, 44 on Apple — and a
        // constant in the render layer could not follow the platform the tree
        // is being drawn for.
        factory.register_with_context::<GestureDetector, _>(|widget, scope| {
            RenderGestureDetector::new(widget.handlers().clone())
                .focusable(widget.is_focusable())
                .min_touch_target(
                    widget
                        .touch_target_override()
                        .unwrap_or_else(|| ambient_touch_target(scope)),
                )
        });
        // Reads ambient state only when the alignment was written directionally
        // — `resolved_alignment` ignores the direction for a physical corner,
        // which is what keeps every `Stack` already written where it was.
        factory.register_with_context::<Stack, _>(|widget, scope| {
            RenderStack::new(
                widget.resolved_alignment(ambient_direction(scope)),
                widget.stack_fit(),
            )
        });
        factory.register::<Semantics, _>(|widget| {
            let mut object = RenderSemantics::new(match widget.semantic_role() {
                SemanticRole::Group => Role::Group,
                SemanticRole::Label => Role::Label,
                SemanticRole::TextField => Role::TextField,
                SemanticRole::Button => Role::Button,
                SemanticRole::ScrollView => Role::ScrollView,
                SemanticRole::CheckBox => Role::CheckBox,
                SemanticRole::Radio => Role::Radio,
                SemanticRole::ProgressBar => Role::ProgressBar,
                SemanticRole::Tab => Role::Tab,
                SemanticRole::Switch => Role::Switch,
                SemanticRole::Slider => Role::Slider,
                SemanticRole::Custom(name) => Role::Custom(name),
                // Both enums are `#[non_exhaustive]`, so a role added to one
                // must not stop the other compiling. Announcing an unknown role
                // as a plain group is the safe direction: a screen reader says
                // less than it could, rather than something untrue.
                _ => Role::Group,
            });
            if let Some(label) = widget.semantic_label() {
                object = object.label(label);
            }
            if let Some(value) = widget.semantic_value() {
                object = object.value(value);
            }
            if let Some(toggled) = widget.semantic_toggled() {
                object = object.toggled(toggled);
            }
            if let Some(live) = widget.semantic_live() {
                object = object.live(match live {
                    SemanticLiveness::Off => Liveness::Off,
                    SemanticLiveness::Polite => Liveness::Polite,
                    SemanticLiveness::Assertive => Liveness::Assertive,
                    // Both enums are `#[non_exhaustive]`; an unknown liveness
                    // must not stop this compiling, and silence is the safe
                    // direction — a screen reader that says less is recoverable,
                    // one that interrupts for the wrong reason is trained out of
                    // the user's attention.
                    _ => Liveness::Off,
                });
            }
            object
                .enabled(widget.semantic_enabled())
                .merge(widget.merges_descendants())
        });

        factory
    }

    /// Teach the factory how to realise a widget type.
    ///
    /// Registering the same type twice replaces the earlier entry, so an
    /// application can override a built-in.
    pub fn register<W, R>(&mut self, create: impl Fn(&W) -> R + 'static)
    where
        W: Widget,
        R: RenderObject,
    {
        self.register_with_context(move |widget, _scope| create(widget));
    }

    /// The same, for a render object that depends on what ancestors published.
    ///
    /// The scope is the one a [`BuildContext`](vieww_widget::BuildContext)
    /// reads through `inherit`, and it exists here because **a render widget
    /// never runs a `build`**: `Flex` cannot ask which way the interface reads,
    /// `Padding` cannot ask for a locale, and wrapping each in a composed widget
    /// that exists only to look one up costs an element per node in every
    /// screen. The classic answer agrees — its creation hook takes a
    /// `BuildContext` for exactly this.
    ///
    /// Separate from [`register`](Self::register) rather than replacing it, so
    /// that every existing registration — including one written outside this
    /// crate — keeps compiling. Most render objects depend on nothing ambient
    /// and should keep using the simpler form.
    ///
    /// ```
    /// # use vieww_render::{RenderFactory, RenderConstrainedBox};
    /// # use vieww_widget::SizedBox;
    /// # use vieww_foundation::{Constraints, Size, TextDirection};
    /// let mut factory = RenderFactory::empty();
    /// factory.register_with_context::<SizedBox, _>(|_widget, scope| {
    ///     // Ambient state is readable here, unlike in `register`.
    ///     let _direction = scope.get::<TextDirection>();
    ///     RenderConstrainedBox::new(Constraints::tight(Size::square(1.0)))
    /// });
    /// ```
    pub fn register_with_context<W, R>(
        &mut self,
        create: impl Fn(&W, &InheritedScope) -> R + 'static,
    ) where
        W: Widget,
        R: RenderObject,
    {
        self.creators.insert(
            TypeId::of::<W>(),
            Box::new(move |node, scope| {
                let widget = node
                    .downcast_ref::<W>()
                    .expect("factory entry was keyed by this widget's TypeId");
                Box::new(create(widget, scope))
            }),
        );
    }

    /// `true` if this widget type has a render object.
    #[must_use]
    pub fn handles(&self, widget: &WidgetNode) -> bool {
        self.creators.contains_key(&widget.widget_type_id())
    }

    /// Build the render object for a widget, if one is registered.
    ///
    /// Ambient state reads as absent: anything registered through
    /// [`register_with_context`](Self::register_with_context) sees an empty
    /// scope and must fall back to a default. That is right for the callers that
    /// have no tree to hand — a unit test, a hand-built render tree — and wrong
    /// for the pipeline, which uses
    /// [`create_with_context`](Self::create_with_context).
    #[must_use]
    pub fn create(&self, widget: &WidgetNode) -> Option<Box<dyn RenderObject>> {
        self.create_with_context(widget, &InheritedScope::new())
    }

    /// Build the render object for a widget, against what its ancestors
    /// published.
    #[must_use]
    pub fn create_with_context(
        &self,
        widget: &WidgetNode,
        scope: &InheritedScope,
    ) -> Option<Box<dyn RenderObject>> {
        self.creators
            .get(&widget.widget_type_id())
            .map(|create| create(widget, scope))
    }

    /// How many widget types are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.creators.len()
    }

    /// `true` if nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.creators.is_empty()
    }
}

impl Default for RenderFactory {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl fmt::Debug for RenderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderFactory")
            .field("registered", &self.creators.len())
            .finish()
    }
}

/// A widget's clip shape as the render layer's.
///
/// Two types for one idea, and the duplication is the price of the dependency
/// rule this module exists to enforce: `vieww-widget` may not name a render
/// type. The translation is total and mechanical, which is what makes it safe
/// to have written twice.
fn to_clip_shape(shape: &WidgetClipShape) -> ClipShape {
    match shape {
        WidgetClipShape::Rect => ClipShape::Rect,
        WidgetClipShape::RRect { radius } => ClipShape::RRect { radius: *radius },
        WidgetClipShape::Oval => ClipShape::Oval,
        WidgetClipShape::Path(path) => ClipShape::Path(path.clone()),
    }
}
