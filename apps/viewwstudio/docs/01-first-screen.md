# Your first screen

Every buffer the studio previews needs exactly one thing:
a function named `screen` that returns something to draw.

```rust
use vieww::prelude::*;

pub fn screen() -> impl Widget {
    Text::new("Hello, vieww!")
}
```

Press **Render**, or just stop typing for a moment, and
the preview pane on the right rebuilds from this function.
That is the whole loop this studio exists to shorten: edit
a line, see the picture change, edit the next line.

## Why `screen`, specifically

Render compiles this buffer on its own, as a standalone
crate, and calls `fn screen() -> impl Widget` to get
something to draw. It cannot call an arbitrary widget's
constructor for you, because it has no way to know what
to pass it — a `Card` might need a title, a list of items,
a callback. Writing `screen()` is how you tell the preview
what to build and with which values, the same way `main`
tells a real binary where to start.

If a buffer defines exactly one `Widget` and that widget
takes no fields, the Problems panel offers to paste the
missing `screen()` for you. Otherwise it shows you the
widget's name and asks you to fill in whatever it needs.

## What is in scope for one buffer

A single file compiles as a single crate, so `screen()` and
everything it calls have to live in that one file. Split a
screen into helper functions or smaller widgets freely —
the file below is a common shape:

```rust
use vieww::prelude::*;

#[derive(Debug)]
struct Greeting {
    name: String,
}

impl Widget for Greeting {
    fn debug_name(&self) -> &'static str {
        "Greeting"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(
        &self,
        ctx: &BuildContext,
    ) -> WidgetNode {
        Text::new(format!(
            "Hi, {}",
            self.name
        ))
        .into()
    }
}

pub fn screen() -> impl Widget {
    Greeting {
        name: "there".to_string(),
    }
}
```

Buffers besides the one you are looking at are not compiled
into this one — reach across files with ordinary Rust
modules once a project grows past a single screen, which
the next page covers.
