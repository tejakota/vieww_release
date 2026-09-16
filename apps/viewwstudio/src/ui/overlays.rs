//! Damage regions and the semantics tree, drawn over the window.
//!
//! # Why these are over the *window* and not inside the preview pane
//!
//! Both are measured in window coordinates — `Damage::regions` is what the
//! compositor was asked to repaint, and `SemanticsNode::bounds` is where a
//! screen reader will draw its cursor. Translating either into the preview
//! stage's local space would mean knowing where the stage is during a build,
//! which is a layout fact and not available there. Drawing them where they
//! already are is both simpler and more honest: the damage overlay shows the
//! *whole* frame's repaint, including the studio's own chrome, which is the
//! interesting part — a studio that repaints the entire window on every caret
//! blink is a thing worth seeing.
//!
//! # Why they are worth having at all
//!
//! The framework's central performance claim is damage-driven repaint: *"a
//! 20×20 change on a 200×200 surface touches under 5% of it"*. No application
//! could see it. A number in a panel is a number somebody believes or does not;
//! a rectangle over the part that actually repainted is the claim itself.
//!
//! The semantics overlay is the same argument for accessibility. `docs/
//! PRODUCTION-GAPS.md` calls the screen-reader gap the one item with a
//! multi-year shape; this is how somebody building a screen checks their half
//! of it without waiting for the other.

use vieww_foundation::{Color, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, GestureDetector, SizedBox};

use crate::state::Studio;
use crate::ui::chrome::label;

/// Damage: the framework's own warning colour, at an alpha that leaves the
/// content underneath readable.
const DAMAGE: Color = Color::rgba(0xE0, 0xA9, 0x3C, 0x38);
const DAMAGE_EDGE: Color = Color::rgba(0xE0, 0xA9, 0x3C, 0xCC);
/// Semantics: a colour nothing in the shell uses, so a box is never mistaken
/// for a selection or a focus ring.
const SEMANTIC_EDGE: Color = Color::rgba(0x57, 0xC5, 0x96, 0xE0);

#[derive(Debug)]
pub struct Overlays {
    pub studio: Studio,
}

impl Widget for Overlays {
    fn debug_name(&self) -> &'static str {
        "Overlays"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Read so the layer rebuilds when the capture changes. The snapshots
        // are `RefCell`s beside a generation signal, like the tree capture and
        // the job queue: two vectors that would otherwise be cloned into
        // signals every frame, when what the layer needs is only that they
        // differ.
        let _generation = self.studio.overlay_generation.get();
        let damage = self.studio.damage_snapshot.borrow();
        let semantics = self.studio.semantics_snapshot.borrow();

        let picking = self.studio.pick_mode.get();

        if damage.is_empty() && semantics.is_empty() && !picking {
            // Absent, not transparent. An invisible full-window layer that is
            // still in the tree still takes every pointer event under it —
            // which, for a layer over the whole studio, is all of them.
            return SizedBox::shrink().into();
        }

        let mut layers: Vec<WidgetNode> = Vec::with_capacity(damage.len() + semantics.len());

        for region in damage.iter() {
            layers.push(
                Positioned::new()
                    .left(region.left)
                    .top(region.top)
                    .width(region.width())
                    .height(region.height())
                    .child(
                        Container::new()
                            .color(DAMAGE)
                            .border(vieww_foundation::Border {
                                color: DAMAGE_EDGE,
                                width: 1.0,
                            }),
                    )
                    .into(),
            );
        }

        for (bounds, description) in semantics.iter() {
            // Outline only, with the name in the corner. A filled box over
            // every semantic node would cover the interface it is describing,
            // and the whole question being asked is "does *this control* have
            // the right role".
            layers.push(
                Positioned::new()
                    .left(bounds.left)
                    .top(bounds.top)
                    .width(bounds.width())
                    .height(bounds.height())
                    .child(
                        Container::new()
                            .border(vieww_foundation::Border {
                                color: SEMANTIC_EDGE,
                                width: 1.0,
                            })
                            .alignment(vieww_foundation::Alignment::TOP_LEFT)
                            .child(
                                Container::new()
                                    .color(Color::rgba(0x12, 0x28, 0x1F, 0xE6))
                                    .padding(EdgeInsets::symmetric(3.0, 1.0))
                                    .child(label(description, 9.0, SEMANTIC_EDGE)),
                            ),
                    )
                    .into(),
            );
        }

        // **Pick mode is the one thing here that does take a click**, and it
        // takes *every* click — that is what picking means. On top of the
        // boxes, and only while it is on, so the overlays go back to being a
        // picture the moment a pick lands.
        //
        // `position` rather than `local`: `hit_test_identify` walks the tree
        // from the root, in window coordinates, and this layer is the window.
        if picking {
            let studio = self.studio.clone();
            layers.push(
                Positioned::new()
                    .left(0.0)
                    .top(0.0)
                    .right(0.0)
                    .bottom(0.0)
                    .child(
                        GestureDetector::new()
                            .on_tap(move |details: vieww_foundation::TapDetails| {
                                // Recorded, not resolved. Hit-testing needs a
                                // `FrameDriver` and a handler has a position and
                                // no tree; `Studio::capture_overlays` picks this
                                // up on the next frame.
                                studio
                                    .pick_request
                                    .set(Some((details.position.dx, details.position.dy)));
                            })
                            .child(Container::new().color(Color::rgba(0x5B, 0x4F, 0xD6, 0x14))),
                    )
                    .into(),
            );
        }

        // `Passthrough` otherwise, so the overlay never takes a click. It is a
        // picture of the window, and a picture that swallowed the controls it
        // is drawn over would make the thing it describes unusable while on.
        Stack::new()
            .fit(StackFit::Passthrough)
            .children(layers)
            .into()
    }
}

widget_node_from!(Overlays);
