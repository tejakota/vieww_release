# The shapes

A handful of widgets cover most of what a screen is made
of. This page is not the full list — `cargo doc` has that —
just the ones you reach for first.

## Container

Colour, padding, and rounded corners around one child.

```rust
Container::new()
    .color(theme.colors.surface)
    .padding(EdgeInsets::all(16.0))
    .radius(theme.metrics.corner)
    .child(Text::new("Card"))
```

## Flex

A row or a column. `spacing` sits between children, and
`cross_axis_alignment` decides how they line up on the
axis they are not stretching along.

```rust
Flex::column()
    .spacing(12.0)
    .cross_axis_alignment(
        CrossAxisAlignment::Stretch,
    )
    .children(children![
        Text::new("First"),
        Text::new("Second"),
    ])
```

## Stack

Children on top of one another, each positioned relative
to the stack's own edges rather than flowing in a line.

```rust
Stack::new().children(children![
    Image::asset("background.png"),
    Positioned::new()
        .bottom(16.0)
        .right(16.0)
        .child(Text::new("Overlay")),
])
```

## Text

A run of styled text. `theme.text` carries the studio's
own scale — headline, title, body — so a label matches
everything around it without a hand-picked size.

```rust
Text::new("Balance due")
    .style(theme.text.title)
    .bold()
```

## Button

A pressable label that calls back on tap.

```rust
Button::new("Save")
    .on_pressed(|| {
        // handle the tap
    })
```

## ListView

A scrolling column that only builds the rows near the
viewport, so a list of thousands stays cheap.

```rust
ListView::builder(items.len(), move |index| {
    Text::new(items[index].clone()).into()
})
```

## Reaching for more

Beyond these six, `vieww-widget` has controls for most of
what a form or a settings screen needs — checkboxes,
switches, dropdowns, dialogs, tables, and more — under the
same `Widget` trait these six implement. If you have used
one, you already know the shape of the rest.
