# The UI tree: widget → element → render

Three crates, three trees, one direction: `vieww-widget` → `vieww-element` →
`vieww-render`. Each is real, independently testable, and — this is the part
worth dwelling on, because it's the part a one-tree framework doesn't have to
answer — each exists because collapsing it into its neighbor would break
something specific.

## Why three trees and not one

A naive UI tree conflates three different lifetimes into one node: what the
developer wrote, what persists across a rebuild, and what actually measured
and painted. `vieww` keeps them apart on purpose, the same split mature toolkits make
for the same reason:

- **The widget tree** (`vieww-widget`) is a description. `Container::new()`,
  `Flex::column()`, `Text::new(...)` — none of this holds mutable state or a
  position on screen. It is built constantly and thrown away every frame; the
  crate's own doc puts it plainly: "a widget must never hold mutable state. If
  you find yourself wanting either, the thing you want is an element." This
  crate has no dependency on `vieww-render` at all — not "avoids reaching into
  it," *cannot*, because the edge doesn't exist in `Cargo.toml`. A widget
  cannot query layout, cannot read a `RenderId`, cannot see a pixel.
- **The element tree** (`vieww-element`) is what persists. Rebuilding the
  widget tree is cheap specifically because the element tree underneath is
  *reconciled* against the new widget tree rather than recreated — matching
  old elements to new widgets by type and key, and keeping the elements that
  match. Identity, local state, and signal subscriptions all live here, in
  `Element`, and survive a rebuild that looks, from above, like the whole UI
  was just re-described from scratch. `Signal`/`Runtime` are the fine-grained
  reactivity layer this crate owns: reading a signal during a build subscribes
  that element, writing marks exactly those elements pending and nothing
  else — the reason a `vieww` app doesn't need a virtual-DOM diff pass to
  figure out what to redraw.
- **The render tree** (`vieww-render`) is where layout and paint actually
  happen. `RenderObject`s measure (`Constraints` in, `Size` out — see below)
  and place their children, and their `paint` method draws onto a `Canvas`
  from `vieww-paint`. This is also the layer where hit testing and focus
  live, because both need the tree's actual geometry, which is exactly the
  thing the widget and element trees never had.

The one-line version, from `vieww-render`'s own doc: "Widgets describe,
elements persist, render objects measure and place."

## Constraints down, sizes up

`vieww-render`'s layout model is a single top-down pass, not a fixed point,
and the rule that makes that possible is worth stating precisely because it's
easy to violate by accident: a parent hands each child a `Constraints`
(`vieww_foundation::Constraints`); the child returns the `Size` it chose
*within* those constraints; the parent then decides where to place it. A
child never learns its own position during layout, and never reads its
parent. This is why a child cannot make itself "as wide as its parent" by
asking — the parent has to say so, by handing down tight constraints — and
why layout never needs to run more than once per node per frame.

## Where this hands off

`vieww-render`'s own module doc draws a distinction worth repeating, because
the naming makes it easy to get backwards: this crate is **not**
`vieww_paint::native`. `vieww-render` is the layout/paint *tree* — layout, hit
testing, `RenderObject::paint` — and it sits *above* `vieww-paint`, which it
calls into to record a `Scene`. `vieww_paint::native::NativeRenderer` is the
pixel-producing *backend*, this workspace's one CPU rasterizer, and sits
*below* `vieww-paint`, consuming the `Scene` that `vieww-render` built.
Neither crate depends on the other. From here the pipeline continues into
`vieww-scene` and onward — see [`rendering-pipeline.md`](./rendering-pipeline.md).

## The design-system engine: tokens, size classes, and widget tiers

Alongside the three trees, `vieww-widget` carries the vocabulary a design
system is built from, in two modules: `tokens` (`WindowSizeClass` — the
Compact/Medium/Expanded breakpoints, published ambiently the same way
`ThemeData` is; and an open `DesignToken<T>`/`TokenSet` registry for
vocabulary `ThemeData`'s fixed fields have no place for) and `tiers`
(`WidgetTier`). The latter is a direct answer to the external architectural
assessment's §9, which names three widget levels by example — "Primitive
widgets" (`Box`/`Flex`/`Stack`/`Text`/`Image`/`Clip`/`CustomPaint`),
"Behavior widgets" (`Focusable`/`Hoverable`/`Draggable`/`Scrollable`/
`Animated`/`Semantics`), and "Design-system widgets" — closing with the
point that matters: "ViewW should allow custom design systems without
needing to become forked widgets." `WidgetTier::{Primitive, Behavior,
Pattern}` classifies this crate's real widgets against exactly that
three-level split, and `TierBudget`/`TierGate` make it actionable: a
`Pattern`-level widget a design system composed from this crate's own
primitives and behaviors can name a simpler fallback and degrade to it
under a tight budget, without forking anything in this crate to do so —
which is the mechanism that makes the assessment's closing line true rather
than aspirational.

## Testing without a window

Because none of layout, hit testing, or scene recording needs an actual
window, all three trees are testable headlessly — `vieww-test-harness` builds
directly on this (see
[`tooling-and-distribution.md`](./tooling-and-distribution.md)) — which is
why several hundred of this workspace's tests run in a CI container with no
display and no GPU.
