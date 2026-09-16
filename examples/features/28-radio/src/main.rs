//! **Each control is given a handler, and that is not decoration.** A
//! value-bearing control in this framework is *disabled* until it has an
//! `on_changed` — `Switch::on_changed`'s own doc says so, and that is this framework's
//! rule. These examples showed the widget without one, so the picture in the
//! gallery was of the **disabled** control in both of its states, which is not
//! what anybody comes here to look at. The handlers are empty because this
//! page is a picture rather than a form; what they change is which of the two
//! appearances is drawn.

use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("28 — radio", Size::new(480.0, 200.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(Flex::row().spacing(24.0).children(children![
                    Radio::new(true).on_selected(std::rc::Rc::new(|()| {})),
                    Radio::new(false).on_selected(std::rc::Rc::new(|()| {})),
                ])),
        );
    })
}
