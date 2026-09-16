//! The counter from `docs/guide/getting-started.md`, as a compiling crate.
//!
//! # Why this exists as a workspace member rather than as a fenced block
//!
//! A guide's code samples are the part readers copy, and they are also the
//! part that goes stale first — a builder method gets renamed, a signature
//! grows an argument, and the prose keeps confidently showing the old one.
//! Nothing in a Markdown file is checked by anything.
//!
//! So the guide's central example lives here, in the workspace, where
//! `cargo check --workspace` compiles it like any other crate and `ci/check/checks.sh`
//! fails if it stops compiling. The guide quotes this file. If you change one,
//! change the other — and the compiler will tell you when you have not.
//!
//! It is deliberately *not* run in CI: it opens a window, and CI has no
//! display. Compiling is the assertion worth having here; a counter's
//! behaviour is covered by `vieww`'s own element tests.

use vieww::prelude::*;
use vieww_platform_winit::App;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    App::new().title("Counter").run(|driver| {
        // Signals live outside the tree, on the runtime. This one is created
        // before the root is mounted and handed to the widget that reads it.
        let count = driver.elements().runtime().signal(0i32);
        driver.set_root(Counter { count });
    })?;
    Ok(())
}

#[derive(Debug)]
struct Counter {
    count: Signal<i32>,
}

#[widget]
impl Counter {
    fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
        // Reading the signal here is what subscribes this widget to it. There
        // is no dependency array to keep in sync — the read *is* the
        // subscription, so it cannot be out of date.
        let value = self.count.get();
        let count = self.count.clone();

        Container::new()
            .padding(EdgeInsets::all(24.0))
            .child(Flex::column().children(children![
                Text::new(format!("Count: {value}")),
                SizedBox::height(12.0),
                Button::new("Add one").on_pressed(move || count.update(|n| *n += 1)),
            ]))
    }
}
