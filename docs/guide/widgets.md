# Writing widgets

## The three trees, in one page

You will hear "widget tree", "element tree" and "render tree". They are three
different things and knowing which is which explains most of vieww's
behaviour.

```
Widget            a cheap, immutable description       thrown away constantly
  ↓
Element           identity + state, one per widget     survives rebuilds
  ↓
Render object     size, position, painting             only for widgets that draw
```

When you rebuild, you produce a **new widget**. vieww matches it against the
**existing element** — same type, same key means the same element — and
updates it in place. The element keeps its state: a half-finished animation,
a text cursor, a scroll position, an open subscription.

That is why a widget must be immutable and hold nothing durable. Anything you
put in a widget struct is gone on the next rebuild by design. Anything that
must survive goes in element state (see [`state.md`](./state.md)).

Only widgets that actually draw get a render object. A `Container` does not —
it is composed from a `Padding` and a `DecoratedBox` that do.

## The shortest widget

```rust
use vieww::prelude::*;

#[derive(Debug)]
struct Badge {
    label: String,
}

#[widget]
impl Badge {
    fn build(&self, ctx: &BuildContext) -> impl Into<WidgetNode> {
        let theme = ThemeData::of(ctx);
        Container::new()
            .padding(EdgeInsets::symmetric(8.0, 4.0))
            .color(theme.colors.surface_variant)
            .child(Text::new(&self.label).style(theme.text.label))
    }
}
```

`#[widget]` writes three things you would otherwise write by hand:

- `debug_name()`, returning `"Badge"` — the type's own name, used in tree
  dumps and error messages.
- `kind()`, returning `WidgetKind::Composed` — "this widget describes itself
  by building others".
- `impl From<Badge> for WidgetNode` — which is what lets you write
  `.child(Badge { .. })` instead of `.child(WidgetNode::new(Badge { .. }))`.

It does **not** add `#[derive(Debug)]`. That is deliberate: an attribute that
silently derives traits makes the set of traits a type implements invisible
at the type's own definition.

### Returning `impl Into<WidgetNode>`

The `Widget` trait's `build` returns `WidgetNode`. Under `#[widget]` you may
return anything that converts into one, which removes the `.into()` that
would otherwise end every widget you write. Returning `WidgetNode` directly
still works.

### The long form

`#[widget]` is a typing shortcut and nothing else. This is the same widget:

```rust
impl Widget for Badge {
    fn debug_name(&self) -> &'static str { "Badge" }
    fn kind(&self) -> WidgetKind<'_> { WidgetKind::Composed }
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // ...
        # unimplemented!()
    }
}

widget_node_from!(Badge);
```

Write it out when you want to see it, or when the type is generic —
`#[widget]` skips generating the `WidgetNode` conversion for a generic type,
because it cannot infer the bounds that would make it correct.

## Adding the other trait methods

Anything else `Widget` offers is written in the same `impl` block and lands in
the right place:

```rust
#[widget]
impl Badge {
    // Explicit identity, so a reordered list keeps element state with the
    // right row rather than with the right position.
    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    // Durable state. See state.md.
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(BadgeState::default()))
    }

    // Extra fields in a tree dump.
    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("label", self.label.clone())]
    }

    fn build(&self, ctx: &BuildContext) -> impl Into<WidgetNode> {
        // ...
        # Text::new("")
    }
}
```

Helper methods and constructors in the same block stay ordinary inherent
methods — the attribute moves only the names `Widget` actually declares.

## Keys, and when you need one

Reconciliation matches a new widget to an existing element **by type and
position** unless you give it a key. That is right almost always and wrong in
exactly one situation: a list whose items are reordered, inserted into or
removed from the middle.

Without keys, deleting the first of five rows makes row 2's widget land on
row 1's element — and row 1's element is the one holding row 1's animation
and scroll offset. With keys, elements follow their data.

```rust
Flex::column().children(
    items
        .iter()
        .map(|item| Row { key: Key::from(item.id), item: item.clone() }.into())
        .collect::<Vec<WidgetNode>>(),
)
```

Use a key derived from your data's identity, never from the index — an index
key is the same as no key at all.

## Composition over configuration

vieww's catalogue is deliberately small and the widgets are deliberately
single-purpose. There is no `full_width` flag on `Button`; you give it tight
constraints from outside with a `SizedBox` or a stretched `Flex`. There is no
`margin` on `Container`; you wrap it in a `Padding`.

This is a deliberate trade and it has the same consequence: trees
get deeper, and each widget stays explainable. If you find yourself wanting a
flag, the answer is usually a wrapper you write once and reuse — which is a
composed widget, which is this page.

## Widget tiers

Every widget in the catalogue reports a tier: `Primitive` (`Flex`, `Stack`,
`Text`, `Clip` — the ones that own layout or painting), `Behavior`
(`Pressable`, `Draggable`, `Animated`, `Semantics` — the ones that add a
behaviour to a child), and `Pattern` (`Dialog`, `DataTable`, `Navigator` —
the ones assembled from the other two).

You mostly do not need to care. It matters when you are building a design
system on top of vieww: a `Pattern` widget can declare a simpler fallback and
degrade to it under a tight budget, so a design system built from these
primitives never has to fork the catalogue to do that. See
`vieww_widget::tiers`.

## What to read next

- [`state.md`](./state.md) — signals, element state, and loading things.
- [`layout.md`](./layout.md) — why your box is the size it is.
- `examples/features/` — one directory per widget or feature, all headless.
