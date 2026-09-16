//! Markdown, laid out as widgets rather than as a string.
//!
//! Headings, emphasis, lists, code and a rule all come out as the same widgets
//! anything else in the library builds from — so a heading that is only bigger,
//! or a list that lost its bullets, is visible here rather than in a diff.

use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

const SOURCE: &str = "\
# Release notes

The **feature examples** each render one capability, and *nothing else*.

- a window when there is a display
- a PNG when there is not
- the same tree either way

Run one with `cargo run -p feature-markdown`.

---

Anything not covered by a widget goes through `CustomPaint`.
";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("49 — markdown", Size::new(600.0, 480.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(28.0))
                .child(Markdown::new(SOURCE)),
        );
    })
}
