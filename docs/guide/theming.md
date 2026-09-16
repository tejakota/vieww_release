# Theming

## Roles, not colours

A control never asks for "dark grey". It asks for `on_surface`, and the
scheme decides what that is. That is the whole design, and it is what makes
swapping a theme swap every control with it rather than most of them.

```rust
pub struct ColorScheme {
    pub primary: Color,             // the accent: a filled button, a checked switch
    pub on_primary: Color,          // text and icons on top of primary
    pub surface: Color,             // the background a screen is built on
    pub on_surface: Color,          // text and icons on top of surface
    pub surface_variant: Color,     // a raised or recessed area: a card, a chip
    pub on_surface_variant: Color,  // secondary text: a subtitle, a caption
    pub outline: Color,             // borders and dividers
    pub error: Color,
    pub on_error: Color,
    pub success: Color,
    pub on_success: Color,
}
```

Eleven roles, and no more, because a role nothing reads is a role nobody can
be sure is right. Every one of them is used by the catalogue.

Note that `success` is a *role*, not "green". Green is the hardest colour to
see for the most common form of colour blindness, which is why nothing in the
catalogue uses colour as the only signal for anything — and why a scheme that
wants to change it can.

## Setting a theme

```rust
Theme::new(ThemeData::dark()).child(app)
```

or, on the window:

```rust
App::new().theme(ThemeData::dark()).run(|driver| { .. })
```

Prefer `App::theme` over `App::background`: it sets the window's background
*and* the theme the widgets read, from one value. Setting only the background
is how a window ends up painted for one theme and populated for another.

## Changing one colour

`ThemeData` and `ColorScheme` are plain structs with public fields, not
builders, so overriding one thing does not require knowing which builder
method sets it:

```rust
ThemeData {
    colors: ColorScheme {
        primary: Color::hex(0x00_5F73),
        ..ColorScheme::light()
    },
    ..ThemeData::light()
}
```

That works for every field that will ever be added.

## What a theme carries besides colour

```rust
pub struct ThemeData {
    pub colors: ColorScheme,
    pub text: Typography,     // display, headline, title, body, label
    pub metrics: Metrics,     // corner, gap, touch_target
    pub motion: Motion,       // durations and curves
    pub platform: TargetPlatform,
}
```

`motion` is worth knowing about: it is where durations and curves live, so an
application can slow every transition down or speed it up in one place, and
so reduced-motion can be honoured centrally rather than per widget.

`metrics.touch_target` is 44 on Apple platforms and 48 on Android, because
those are the two platforms' own numbers. No control in the catalogue is ever
smaller than it on either axis.

## Platform-adaptive themes

```rust
ThemeData::adaptive(TargetPlatform::current(), dark)
```

picks the platform's colours, metrics, motion **and shapes**. The controls
that differ visibly between iOS and Android — the switch, the slider, the
checkbox and radio, the button's corner, the activity indicator, the alert's
button row, the segmented control — read `theme.platform` and draw the shape
that platform's users recognise. Everything else ignores it, which is why
this is still one catalogue and not two.

### A note on the Apple schemes

`ColorScheme::apple_light` and `apple_dark` use Apple's **increased-contrast**
system colours rather than its default ones — `#0040DD` for `systemBlue`
rather than `#007AFF`, and so on.

The reason is measured rather than aesthetic. Apple's default palette is
tuned for how a filled iOS control looks under a 17pt semibold label, which
WCAG judges as *large* text at 3:1. A framework cannot assume that on an
application's behalf: a `Button` here takes whatever size the type scale
gives it, and `outline` is a text field's border as well as a divider. Under
the default palette, `vieww-accessibility`'s audit failed seven pairs across
the two schemes. Under Apple's own accessible variants it passes all of them.

In dark mode the labels on the accent fills are **black**, not white, for the
same reason — see `ColorScheme::apple_dark`'s own documentation for why a
lighter fill with a dark label beats a darker fill with a light one.

## High contrast

```rust
let scheme = ColorScheme::light().high_contrast();
```

Foregrounds go to the end of the lightness axis; grounds move a quarter of
the way, not all of it, because a surface driven to pure white is a glare
source. `outline` moves most, because it is the role that fails first — a
hairline divider at 1.4:1 is invisible to the people the setting exists for.

**Hue is preserved.** `primary`, `error` and `success` keep their identity;
they are not collapsed to black and white. A high-contrast mode that throws
away the difference between "destructive" and "confirm" has traded one
accessibility failure for another.

You do not normally call this yourself: `ThemeData::of` reads the platform's
high-contrast preference and applies it, once per build, at the one funnel
every control already passes through.

## Design tokens

For values that are yours rather than the framework's:

```rust
let tokens = TokenSet::new()
    .with(DesignToken::<Elevation>::new("card"), Elevation(2.0));
```

An open registry, published through the same `Inherited<T>` mechanism as
everything else. Use it when your design system has values the eleven colour
roles and three metrics do not cover — and resist adding them to
`ColorScheme`, which stays small on purpose.

## Checking a theme

```rust
let report = vieww_accessibility::audit_color_scheme(&scheme);
assert!(report.passes(), "{:#?}", report.failures);
```

Put that in your test suite. It checks every documented foreground/background
pair against WCAG AA (4.5:1), and `outline` against 1.4.11's non-text
threshold (3:1). It is how the framework's own schemes are checked, and it is
how the Apple ones were found to be failing.
