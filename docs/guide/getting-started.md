# Getting started

From an empty directory to a window you can click on.

## Before you start

vieww needs Rust 1.85 or newer (`rust-toolchain.toml` pins it) and, on Linux,
the usual windowing development packages — `libx11`, `libxkbcommon`,
`libwayland` and a Vulkan loader. On Debian and Ubuntu:

```sh
sudo apt install libx11-dev libxkbcommon-dev libwayland-dev libvulkan1
```

If you only want to run headless — tests, fixtures, the feature examples —
you need none of that. Quite a lot of this framework is usable without a
screen, deliberately; see [`testing.md`](./testing.md).

## A new project

```sh
cargo run -p vieww-cli -- new my-app
cd my-app
cargo run
```

That is a real window with a real widget in it. `vieww new` writes a
`Cargo.toml`, a `.gitignore` and a `src/main.rs` that is about thirty lines,
and there is nothing in it to fill in before it runs.

Check your toolchain first if anything looks wrong:

```sh
cargo run -p vieww-cli -- doctor
```

## What the generated `main.rs` says

```rust
use vieww::prelude::*;
use vieww_platform_winit::App;

/// The theme this application draws in.
///
/// One value. `App::theme` sets the window's background from it *and*
/// publishes it above the widget tree, so the two cannot disagree.
fn theme() -> ThemeData {
    ThemeData::light()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report = App::new()
        .title("my-app")
        .size(Size::new(420.0, 720.0))
        .theme(theme())
        .run(|driver| {
            driver.set_root(WidgetNode::new(Home));
        })?;
    println!("{report}");
    Ok(())
}

#[derive(Debug)]
struct Home;

#[widget]
impl Home {
    fn build(&self, ctx: &BuildContext) -> impl Into<WidgetNode> {
        let theme = ThemeData::of(ctx);
        Container::new()
            .color(theme.colors.surface)
            .alignment(Alignment::CENTER)
            .child(Text::new("Hello, my-app!").style(theme.text.headline))
    }
}
```

Five things are worth pointing at.

**`App::theme` is one value, not two.** It sets the window's background *and*
publishes the `ThemeData` above the root, so what the window is painted with
and what the widgets draw on cannot drift apart. `App::background` still
exists and still only does the first half — a window painted dark over a tree
that never got a `Theme` gets `ThemeData::light()` from `ThemeData::of`, which
is near-black text on a near-black ground. Change `theme()` and both move
together.

**`App::run` takes the thread and does not return** until the window closes.
That is what a native event loop is. The closure it takes runs once, before
the first frame, and `driver` is your handle on the tree — `set_root` is
almost always the only thing you do with it at startup.

**`#[derive(Debug)]` is required, not decorative.** `Widget: Any + Debug`,
because a tree you cannot print is a tree you cannot debug. Leave it off and
the compiler will tell you, but it will tell you at the `impl`, which is one
step removed from the missing line.

**`#[widget]` writes the mechanical half of the `Widget` impl** — the type's
name, the fact that it is a composed widget, and the `From<Home> for
WidgetNode` conversion that lets you pass a `Home` anywhere a `WidgetNode` is
wanted. You can write all of that out by hand and some widgets do; see
[`widgets.md`](./widgets.md).

**`ThemeData::of(ctx)` is how anything reads the theme.** Not a global, not a
parameter you thread through: it walks up from this widget's position in the
tree to the nearest `Theme` above it. That is the same mechanism every
inherited value uses. See [`theming.md`](./theming.md).

## Something that responds

A counter, which is the shortest program that involves state:

```rust
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
        // Reading the signal here is what subscribes this widget to it.
        // There is no dependency array to keep in sync — the read *is* the
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
```

Press the button and **only `Counter` rebuilds**. Not the `Container` above
it, not the window, not the `Text` — the element that read the signal, and
whatever its `build` returns. That is the whole point of the element layer,
and you can watch it happen:

```sh
cargo run -p vieww --example counter
```

which prints the tree with a `builds=` count on every node before and after a
write.

## Where to go next

- Writing your own widgets, and what the three trees are for:
  [`widgets.md`](./widgets.md)
- State that outlives a rebuild — animations, text fields, subscriptions,
  loading things: [`state.md`](./state.md)
- `cargo run -p vieww-cli -- --help` for `build`, `run`, `test`, `profile`
  and `package`.
- `examples/features/` is 57 numbered directories, one feature each, all
  headless. `00-rectangle` upward is roughly increasing order of difficulty
  and each is small enough to read in a sitting.
