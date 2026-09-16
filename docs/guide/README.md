# The vieww guide

Documentation for people **building an application** with vieww.

That is the distinction worth stating up front, because this workspace had a
lot of documentation and none of it was this. [`docs/architecture/`](../architecture/README.md)
explains why the crates are shaped the way they are; [`TRACKER.md`](../../TRACKER.md)
says what is tested; every non-trivial public item has a doc comment
explaining its own reasoning. All of that is written for somebody working
*on* vieww. Somebody working *with* it had a README, a `cargo run` line, and
57 example directories to read in whatever order they found them.

These pages are the missing half. They assume you know Rust and do not
assume you have read anything else here.

| Page | What it answers |
|---|---|
| [`getting-started.md`](./getting-started.md) | I have an empty directory. How do I get a window with something in it? |
| [`widgets.md`](./widgets.md) | How do I write my own widget, and what are the three trees? |
| [`state.md`](./state.md) | Where does state live, what re-renders, and how do I load something? |
| [`layout.md`](./layout.md) | How does sizing actually work, and why is my box the wrong size? |
| [`theming.md`](./theming.md) | How do I make it look like my product instead of like the default? |
| [`testing.md`](./testing.md) | How do I test a UI without a screen? |
| [`accessibility.md`](./accessibility.md) | What do I get for free, and what do I still have to do? |

## The shortest possible orientation

vieww is a **retained-mode** UI framework. You describe what the UI should
look like right now; vieww works out what changed and redraws only that.

```rust
use vieww::prelude::*;

#[derive(Debug)]
struct Greeting {
    name: String,
}

#[widget]
impl Greeting {
    fn build(&self, ctx: &BuildContext) -> impl Into<WidgetNode> {
        let theme = ThemeData::of(ctx);
        Container::new()
            .padding(EdgeInsets::all(16.0))
            .child(Text::new(format!("Hello, {}!", self.name)).style(theme.text.headline))
    }
}
```

Three ideas carry almost everything:

- **A widget is a description, not a thing.** `Greeting` above is a struct
  that is thrown away and rebuilt constantly. It holds no state and owns no
  pixels. Building one is cheap on purpose.
- **An element is the identity behind it.** When `Greeting` is rebuilt, the
  element that represents it in the tree stays the same object — which is
  what lets an animation keep running and a text field keep its cursor across
  a rebuild. You rarely name elements directly; you feel them when state
  survives.
- **A signal is what makes something rebuild.** Reading a `Signal<T>` during
  `build` subscribes that one widget to it. Writing it marks that one widget
  for rebuild — not its parent, not the tree.

If you have used another retained-mode toolkit, this is the same shape with
different names, and [`widgets.md`](./widgets.md) says where it deliberately
differs. The mental model that transfers is "describe, don't mutate"; the
one that does not is callback state — see [`state.md`](./state.md).

## What vieww is good at, and what it is not, today

Worth knowing before you invest a weekend. All of this is measured rather
than aspirational — [`TRACKER.md`](../../TRACKER.md) is the crate-by-crate
version.

**Good at:** desktop applications on Linux; dense, text-heavy UI (the text
stack is `cosmic-text`, so real shaping, real bidi, real font fallback);
anything where you want the framework to stay out of the way of a custom
design system; accessibility, which is a first-class crate rather than an
afterthought.

**Not there yet:** GPU rendering exists, is real, and now draws **text** as
well as shapes — but it still cannot draw images, shadows, gradients, shaped
clips or layers, so any frame containing one of those falls back to the CPU
rasterizer. That is most real screens. The CPU rasterizer is fast and is what
every screenshot in this repository was drawn with, but it is a software
rasterizer and has a software rasterizer's ceiling.

The GPU path has also only ever run on `lavapipe`, a *software* Vulkan
implementation — so it is verified correct and entirely unmeasured for speed.

macOS, Windows, Android and iOS are real code that has never been built on its
own toolchain; `ci/check/platform-check.sh` is one command each that says so
precisely. The web backend has never been compiled at all.

If you need a browser target or a hard 120 Hz guarantee today, this is not the
framework yet, and it is better to know that now.
[`PENDING.md`](../../PENDING.md) is the full list, kept so that this paragraph
never has to be the full list.
