# The preview

The preview pane renders your buffer's `screen()` inside a
device frame you can switch, so a layout gets judged at the
size it will actually be seen at rather than at whatever
your editor window happens to be.

## Platforms

Three platforms, in the switcher above the frame:

```text
iOS
Android
Desktop
```

Switching platform changes the frame's size and safe-area
insets, and it changes which theme conventions apply — an
Android screen defaults to a different motion and elevation
feel than an iOS one, and `ThemeData::of(ctx)` reflects
that difference back into your widgets without you writing
a single `if platform ==` anywhere in `screen()`.

## What is simulated, and what is real

The frame, the safe area, and the theme are simulated: they
are numbers this studio applies, chosen to match each
platform's real conventions closely enough that a layout
decision made here holds up on the device. They are not the
platform itself.

What is real is everything your widget tree does with those
numbers — the actual layout algorithm, the actual paint, the
actual text shaping and font fallback. A `Flex` that
overflows here overflows on the device too; a `Text` that
wraps at the wrong width here wraps the same way there. The
preview is not a mockup of your screen, it is your screen,
looked at through a simulated window.

## What the preview cannot show you

A handful of things only exist once real platform services
are behind the widget tree: a camera feed, a GPS fix, a
push notification, a background task that keeps running
after the app is backgrounded. `screen()` can still be
built and looked at with these missing — pass in sample
data the way `LandingScreen` does above — but the preview
will not exercise the real service underneath. That is what
**Build and Run**, on the next page, is for.
