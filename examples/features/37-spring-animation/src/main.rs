//! A spring settling onto a snap point after a fling.
//!
//! `PhysicsDrag` is the drag-and-release piece: while a finger is down it
//! follows it, and on release it picks the snap point the *velocity* is heading
//! for and springs there. This example performs the release itself so the shot
//! has something to be a picture of; in a window the same code is what a
//! gesture handler would call.
//!
//! # Why the fling starts before the first frame
//!
//! A spring is integrated rather than sampled, so it only exists once it is in
//! flight. Releasing during setup means frame one is already mid-flight and the
//! strip shows the overshoot and the settle rather than five copies of a
//! resting handle.

use std::cell::RefCell;
use std::rc::Rc;

use vieww_animation::SpringPreset;
use vieww_element::{PhysicsDrag, Signal};
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

const DETENTS: [f32; 5] = [0.0, 100.0, 200.0, 300.0, 400.0];
const TRACK_TOP: f32 = 60.0;

#[derive(Debug)]
struct Track {
    position: Signal<f32>,
    /// Held because the frame driver's registration is weak — see
    /// `05-rectangle-animation`. Also what the button flings.
    drag: Rc<RefCell<PhysicsDrag>>,
}

/// One fling: a short drag released fast, from wherever the handle is.
///
/// 40pt of movement but 900pt/s of velocity, which is what carries it *past*
/// the nearest detent to the next one — a slow release from the same place
/// settles back. Reversed at the far end so the button always has somewhere to
/// throw it.
fn fling(drag: &Rc<RefCell<PhysicsDrag>>) {
    let mut physics = drag.borrow_mut();
    let from = physics.position();
    let forwards = from < DETENTS[DETENTS.len() - 1] - 1.0;
    physics.drag_begin();
    physics.drag_to(if forwards { from + 40.0 } else { from - 40.0 });
    physics.drag_end(if forwards { 900.0 } else { -900.0 });
}

impl vieww_widget::Widget for Track {
    fn debug_name(&self) -> &'static str {
        "Track"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let x = self.position.get();
        let mut stack = Stack::new();
        // The detents, so where the spring chose to stop is legible rather than
        // being a position the reader has to take on trust.
        for detent in DETENTS {
            stack = stack.push(
                Positioned::new()
                    .left(detent + 34.0)
                    .top(TRACK_TOP + 12.0)
                    .child(
                        Container::new()
                            .color(Color::rgb(214, 221, 231))
                            .radius(4.0)
                            .child(SizedBox::square(8.0)),
                    ),
            );
        }
        let again = Rc::clone(&self.drag);
        Container::new()
            .color(Color::rgb(247, 248, 250))
            .padding(EdgeInsets::all(20.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(12.0)
                    .children(children![
                        // A spring settles and then, correctly, stops. The button
                        // is how a window gets to see it move more than once.
                        Button::new("fling").on_pressed(move || fling(&again)),
                        SizedBox::from_size(Size::new(460.0, 140.0)).child(
                            stack.push(
                                Positioned::new().left(x).top(TRACK_TOP - 6.0).child(
                                    Container::new()
                                        .color(Color::rgb(58, 122, 246))
                                        .radius(16.0)
                                        .child(SizedBox::square(32.0)),
                                ),
                            )
                        ),
                    ]),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(Track);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("37 — spring", Size::new(520.0, 220.0), |d| {
        let runtime = d.elements().runtime().clone();
        let drag = Rc::new(RefCell::new(PhysicsDrag::new(
            &runtime,
            0.0,
            DETENTS.to_vec(),
            SpringPreset::Expressive,
        )));
        let position = drag.borrow().position_signal();
        d.tickers().add(&drag);

        // Flung during setup so the first frame is already mid-flight: a spring
        // is integrated rather than sampled, so it only exists once it is moving.
        // The integration is driven by tick *deltas*, so unlike a tween it does
        // not care what the absolute clock says.
        fling(&drag);

        feature_harness::set_page(d, Track { position, drag });
    })
}
