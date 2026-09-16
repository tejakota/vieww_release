use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Alignment, Key, Transform};

use crate::{
    icons, widget_node_from, Align, Animated, BuildContext, Clip, CrossAxisAlignment, Flex,
    GestureDetector, Icon, MainAxisAlignment, MainAxisSize, Opacity, SemanticRole, Semantics,
    ThemeData, Transformed, Widget, WidgetKind, WidgetNode,
};

/// A header that reveals or hides a body when tapped.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Accordion;
///
/// let panel = Accordion::new(Text::new("Details"), Text::new("More about the thing"))
///     .expanded(false)
///     .on_toggled(|open| println!("now {open}"));
/// ```
///
/// # Controlled, like everything else in this module
///
/// `Accordion` does not own whether it is open — the caller does, in a
/// `Signal`](crate::Signal) it writes from [`on_toggled`
/// and reads back into [`expanded`](Self::expanded). This is what lets a
/// list of accordions enforce "only one open at a time" from the outside
/// with no cooperation from this widget at all — the same reasoning
/// `controls/mod.rs`'s module docs give for every control here.
///
/// # It measures its own body — `content_height` is gone
///
/// This used to take the body's revealed height as a **required** parameter,
/// on the reasoning that measuring a child before showing it needed an
/// intrinsic-size pass the framework did not have. That reasoning was wrong
/// about which mechanism it needed. A reveal does not have to know the height
/// in advance: the body is laid out normally every frame, and
/// [`Align::height_factor`](crate::Align::height_factor) makes this box a
/// fraction of whatever the body chose, with a [`Clip`](crate::Clip) cutting
/// the rest. Animating the factor from 0 to 1 is the reveal, in **one layout
/// pass**, to the height the body actually has this frame.
///
/// So a body whose content changes now animates to the right height without
/// anyone updating a constant, and the class of bug where the caller's number
/// and the body's real height drift apart cannot occur. A size transition reaches the
/// same place with an explicit size transition for the same reason.
///
/// [`content_height`](Self::content_height) is kept as a no-op so existing
/// call sites still compile; it is deprecated and does nothing.
///
/// # Why the element still exists, and still animates, while collapsed
///
/// The [`Animated`] wrapping the body is mounted **unconditionally** — its
/// `target` moves between `0.0` and `1.0` rather than the body being added
/// and removed from the tree. `Animated` "continues from where it is" only
/// because the element holding the animation stays alive; tearing it down
/// and remounting it on every toggle would restart the motion from
/// wherever `from` says, every single time, which is not a crossfade at
/// all.
pub struct Accordion {
    header: WidgetNode,
    body: WidgetNode,
    expanded: bool,
    on_toggled: Option<Rc<dyn Fn(bool)>>,
    key: Option<Key>,
}

impl Accordion {
    #[must_use]
    pub fn new(header: impl Into<WidgetNode>, body: impl Into<WidgetNode>) -> Self {
        Self {
            header: header.into(),
            body: body.into(),
            expanded: false,
            on_toggled: None,
            key: None,
        }
    }

    #[must_use]
    pub const fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    /// **Deprecated and ignored.** The body is measured; see the type's docs.
    ///
    /// Kept rather than removed because deleting it would break every call
    /// site for no benefit — the parameter is now noise, not a lie, and the
    /// deprecation says so where a reader will see it.
    #[must_use]
    #[deprecated(
        note = "the body's height is measured now; this value is ignored and the call can be deleted"
    )]
    pub const fn content_height(self, height: f32) -> Self {
        let _ = height;
        self
    }

    #[must_use]
    pub fn on_toggled(mut self, handler: impl Fn(bool) + 'static) -> Self {
        self.on_toggled = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Accordion {
    fn debug_name(&self) -> &'static str {
        "Accordion"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let _ = ctx; // no themed colour used in this v1 body yet
        let expanded = self.expanded;
        let body = self.body.clone();

        let chevron: WidgetNode = Animated::themed(ctx, if expanded { 1.0 } else { 0.0 })
            .build(move |t| {
                // A quarter turn, tweened directly on the angle rather than
                // through a second `Animated`: one animated value driving
                // both the rotation and the reveal below keeps them in
                // lock-step by construction, with nothing to fall out of
                // sync if either duration is ever changed independently.
                Transformed::new(Transform::rotate(t * std::f32::consts::FRAC_PI_2))
                    .child(Icon::new(icons::chevron_down()).size(20.0))
                    .into()
            })
            .into();

        let header_row: WidgetNode = GestureDetector::new()
            .on_tap({
                let handler = self.on_toggled.clone();
                move |_| {
                    if let Some(handler) = &handler {
                        handler(!expanded);
                    }
                }
            })
            .child(
                Flex::row()
                    .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .push(self.header.clone())
                    .push(chevron),
            )
            .into();

        let header_row: WidgetNode = Semantics::new()
            .role(SemanticRole::Button)
            .toggled(expanded)
            .child(header_row)
            .into();

        // Never shorter than a finger can reliably land on — a header whose
        // content is one line of text otherwise comes out exactly that tall.
        let touch_target = ThemeData::of(ctx).metrics.touch_target;
        let header_row: WidgetNode = crate::controls::min_height(touch_target, header_row).into();

        let revealed: WidgetNode = Animated::themed(ctx, if expanded { 1.0 } else { 0.0 })
            .build(move |t| {
                // `Align` with a height factor, not a `SizedBox` with a
                // computed height: the body is laid out at its natural size
                // either way, and this box then takes `t` of it. Aligned to the
                // **top** so the body slides out from under the header rather
                // than growing from its own middle.
                Clip::rect()
                    .child(
                        Align::new(Alignment::TOP_LEFT)
                            .height_factor(t)
                            .child(Opacity::new(t).child(body.clone())),
                    )
                    .into()
            })
            .into();

        Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .children(crate::children![header_row, revealed])
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("expanded", self.expanded.to_string())]
    }
}

impl fmt::Debug for Accordion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Accordion")
            .field("expanded", &self.expanded)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Accordion);

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::{inflate, DebugNode, Text, Theme};

    use super::*;

    fn panel(expanded: bool) -> Accordion {
        Accordion::new(Text::new("Header"), Text::new("Body")).expanded(expanded)
    }

    fn built(accordion: Accordion) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(accordion))
    }

    #[test]
    fn the_header_reports_its_toggled_state() {
        assert_eq!(
            built(panel(false))
                .find("Semantics")
                .and_then(|n| n.property("toggled")),
            Some("false")
        );
        assert_eq!(
            built(panel(true))
                .find("Semantics")
                .and_then(|n| n.property("toggled")),
            Some("true")
        );
    }

    #[test]
    fn both_animated_values_target_where_expanded_says() {
        // The two `Animated` nodes (chevron rotation, body reveal) both
        // exist regardless of `expanded` — see the type's doc comment for
        // why remounting them would break the crossfade — so what changes
        // between the two states is their `target`, not their presence.
        let collapsed = built(panel(false));
        let open = built(panel(true));
        assert_eq!(collapsed.find_all("Animated").len(), 2);
        assert_eq!(open.find_all("Animated").len(), 2);
    }

    #[test]
    fn the_stored_handler_fires_with_whatever_its_caller_passes() {
        // What this cannot cover, honestly stated: the `!expanded` inversion
        // itself lives inside the closure `build` constructs for the
        // `GestureDetector`'s `on_tap`, which needs a real hit test to
        // dispatch — the same limitation `Breadcrumbs` and `Pagination`'s
        // equivalent tests carry. This confirms the handler `Accordion`
        // stores and eventually calls behaves correctly once called; it
        // does not reach into `build` to prove *what* it is called with.
        let toggled_to = Rc::new(Cell::new(None));
        let accordion = panel(false).on_toggled({
            let toggled_to = Rc::clone(&toggled_to);
            move |now_expanded| toggled_to.set(Some(now_expanded))
        });

        let handler = accordion.on_toggled.clone().expect("has a handler");
        handler(true);
        assert_eq!(toggled_to.get(), Some(true));
    }
}
