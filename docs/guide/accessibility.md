# Accessibility

## What you get without doing anything

Every control in the catalogue publishes a semantics node: a role, a label, a
state, and the actions it accepts. That tree is handed to
[AccessKit](https://accesskit.dev), which is what actually talks to the
platform — UI Automation on Windows, AT-SPI on Linux, NSAccessibility on
macOS, and (through a hand-rolled path around an upstream limitation) the
Android accessibility framework.

So a `Button` announces itself as a button with its label. A `Switch`
announces its checked state and updates it when toggled. A `Dialog` gets
`Role::Alert`, which every platform screen reader treats as something to read
immediately. `InlineError` reaches the implicit live region. A `DataTable`
row reports selection the way AccessKit models it.

You also get, without asking:

- **Touch targets** never smaller than the platform's own number — 44 points
  on Apple, 48 on Android — on either axis, for every control in the
  catalogue.
- **Focus order** following the tree, with `FocusTrap` for modals so tab does
  not escape an open dialog.
- **Reduced motion** honoured centrally, through `theme.motion`, rather than
  per widget.
- **High contrast** applied at `ThemeData::of` — one funnel every control
  already passes through — so it reaches everything.
- **Colour is never the only signal** for anything the catalogue draws.
- **Right-to-left** throughout, if you use the directional variants (see
  [`layout.md`](./layout.md)).

## What you still have to do

### Label anything that is only an icon

An icon button has no text, so it has no label, and the audit will tell you
so:

```rust
Semantics::new()
    .label("Delete")
    .child(FloatingActionButton::new().icon(icons::TRASH))
```

This is the single most common real accessibility bug in an application built
on any framework, and the only one the framework cannot fix for you.

### Say what a decorative image is not

An image that carries meaning needs a label. An image that is decoration
should be excluded, so a screen reader does not read out a filename:

```rust
ExcludeSemantics::new().child(background_flourish())
```

### Announce what changed, when it is not on screen

A toast that appears, a result count that updated, a save that finished:

```rust
ctx.announce("7 results");
```

`Semantics::live` marks a region whose changes should be announced as they
happen — a live region — for things that update in place.

### Group what should be read as one thing

A card with a title, a subtitle and a price is three nodes and probably wants
to be one:

```rust
Semantics::new()
    .label(format!("{title}, {subtitle}, {price}"))
    .child(BlockSemantics::new().child(card))
```

## Auditing

Two audits, both mechanical, both worth putting in CI.

**The semantics tree**, for missing labels, undersized touch targets and
internal inconsistencies:

```rust
let findings = vieww_accessibility::audit(&harness.driver().semantics());
assert!(findings.is_empty(), "{findings:#?}");
```

**The theme**, for contrast:

```rust
let report = vieww_accessibility::audit_color_scheme(&my_scheme);
assert!(report.passes(), "{:#?}", report.failures);
```

Every documented foreground/background pair against AA's 4.5:1, and `outline`
against 1.4.11's non-text 3:1. This is how the framework's own schemes are
checked — and it is how the Apple ones were found to be failing seven pairs,
which is why they now use Apple's increased-contrast colours (see
[`theming.md`](./theming.md)).

```sh
cargo run -p feature-accessibility-audit
```

runs both over a sample screen, headless.

## What the audit cannot tell you

This matters, because a green audit is easy to over-read.

It checks that a label **exists**, never that it is **right**. `label("Button
2")` passes every check here and is useless to the person relying on it. It
checks contrast arithmetic, not whether a colour is distinguishable to
someone with a specific colour vision deficiency. It checks that the
semantics tree is internally consistent, not that its reading order makes
sense.

And it checks what reaches AccessKit — **not** what a screen reader says.
Nothing in this repository has been listened to with VoiceOver or TalkBack
switched on. The conversion into AccessKit's types is tested; the last mile
is not, and until somebody does it, treat "works with a screen reader" as
designed rather than as known.

The honest way to close that gap is thirty minutes with the screen reader on
your own platform, tabbing through your application with the display off.
There is no substitute and no test that replaces it.
