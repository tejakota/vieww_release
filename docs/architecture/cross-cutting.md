# Cross-cutting crates

These crates sit beside the widget → element → render → scene → render-graph
→ render-planner → CPU/GPU/hybrid spine rather than on it. Each depends on
`vieww-foundation` and little else, which is what lets them be used, and
tested, without pulling in the render stack at all — most of them are pure
logic over data the caller already has, with no tree, no frame, and no window
in sight.

## `vieww-foundation`: the vocabulary

Every other layer speaks in this crate's types, and it speaks in nobody
else's: it knows nothing about widgets, elements, or pixels. Its only
external dependency is `unicode-segmentation`, used for the grapheme-cluster
boundaries `TextEditingValue` moves a caret over — a Unicode table, not an
algorithm, and hand-rolling it is the specific failure mode where backspace
leaves half an emoji behind. The one module here that isn't pure vocabulary
is `crash`: a panic hook is process-global, so the type describing a panic
has to be visible from every layer, and this is the one crate all of them
already depend on. It also carries the capability system this workspace uses
for camera/location/biometrics/sensors/notifications/platform-view access
(`capability::*`) — a permission model deliberately kept in the foundation
layer rather than the platform bridge, since every layer above needs to be
able to *ask* about a capability even where only `vieww-platform-winit`
answers it for real.

## `vieww-text`: strings to positioned glyphs

Text gets its own layer rather than being a corner of the canvas because it's
the hardest part of a UI toolkit and the usual weak point of Rust ones. What
it owns: font loading and fallback, shaping, line breaking, and
bidirectional-run ordering. What comes out is `GlyphRun`s — positioned glyph
IDs, ready to draw — and nothing downstream ever sees a character again; if
the paint layer received raw strings it would have to shape them itself to
draw them, and any disagreement with what layout measured would surface as
text silently overflowing its box. Bidirectionality is built in from the
start rather than retrofitted, because "a line is a string plus an x
position" cannot represent Hebrew with an embedded English phrase, and a run
here carries its own direction so a single line can hold runs of both.

**A naming note worth being direct about:** the original task list named
`rustybuzz` as the shaping library. What is actually implemented and tested
here (162 unit tests, 10 shaping-property tests, 3 doctests, all passing) is
built on `cosmic-text` / `harfrust` / `skrifa` / `swash` / `read-fonts` — a
different, real, modern pure-Rust shaping stack, not the named crate. The
functional requirement (real shaping — ligatures, script reordering, correct
advances — not a stand-in) is met; the specific library named was not the one
used, and that substitution is recorded here and in
[`TRACKER.md`](../../TRACKER.md) rather than left unstated.

## `vieww-asset` and `vieww-image`: files to pixels, and pixels to more pixels

`vieww-asset` closes a specific gap: `Image` used to have exactly one
constructor (`from_rgba8`), so every application brought its own decoder, its
own cache, and its own answer to "where does a file live on this platform" —
three reinvented wheels, the worst of which is that an APK is a zip and an
iOS bundle is a directory, so "open a file" isn't even one operation across
platforms. The shape: `AssetBundle` is *where bytes come from* and is a
`vieww_foundation` service, so the platform registers one implementation and
a test registers a different one without either side knowing; `decode` is
*bytes to pixels* and is pure, needing no platform and no files, which is why
it's testable everywhere; `ImageCache` exists because decoding a 2MB
photograph on the frame that wants to draw it is a dropped frame every time.

`vieww-image` picks up from there — operating on and producing the same
`Image` pixels — with what a renderer or a long-running app needs *after*
decode: `mipmap` (straight-alpha-aware box-filter mip chains, correct at
non-power-of-two sizes), `atlas` (shelf-packing layout plus a real
pixel-copying compositor), `sequence` (animated GIF playback via `image`'s
own `AnimationDecoder`), `profile` (color-space tagging and bulk sRGB
transfer-function conversion — deliberately *not* full ICC profile parsing or
gamut mapping, which its own module doc scopes out explicitly rather than
leaving as a silent gap), and `residency` (a byte-budgeted LRU cache for
decoded images, for an application that wants bounded memory on purpose).

## `vieww-gestures` and `vieww-animation`: pointers over time, values over time

`vieww-gestures` exists because a finger landing on the screen is
*permanently* ambiguous at the moment of contact — a touch on a button inside
a scrollable list might be a press or the start of a scroll, and nothing
about that instant says which. Rather than guessing, every recognizer under
the pointer joins a gesture `arena`, and the arena holds the ambiguity open
until something resolves it: a drag passing its slop threshold and declaring
itself, or the finger lifting with nobody having claimed the gesture, in
which case the innermost waiting recognizer wins. This crate depends on
`vieww-foundation` alone, so every recognizer is testable by handing it
synthetic pointer events with no tree, no frame, and no window — routing a
resolved gesture to an actual widget is `vieww-render`'s job, since that's
what owns the hit test.

`vieww-animation`'s one governing idea is that every animation is a
**function of time**, sampled at the frame's vsync timestamp — nothing here
integrates a velocity by a frame delta, and nothing reads a wall clock
directly. That single constraint is what makes an animation behave the same
at 60 Hz and 120 Hz, survive a dropped frame by skipping ahead rather than
stretching, and be testable by handing it a `Duration` instead of sleeping in
a test. `Curve` is shape (ease-in, ease-out, a cubic bézier), `Tween`/`Lerp`
is "half way between these two values" per type, `Spring`/`Fling` is
physically-described motion for continuing a gesture, `AnimationController`
is one `f32` moving over time under any of the above, and `Tickers` is the
registry a frame actually advances.

## `vieww-accessibility`: verification, deliberately downstream

`vieww-render`'s `SemanticsTree` and the AccessKit bridge in
`vieww-platform-winit` are the plumbing that gets an application in front of
a screen reader at all. Neither can tell an author that a button forgot its
label, that a tap target shrank to ten pixels, or that a theme's error color
is unreadable on its own background — that's a question about the *content*
of a tree or a palette, asked after the fact, and belongs next to a test
suite rather than inside the render loop that rebuilds the tree every frame.
Keeping this a separate, downstream crate is also what keeps it honest:
`vieww-render` and `vieww-widget` don't know this crate exists, so nothing in
it can quietly become load-bearing for either — an audit that stopped
compiling would fail exactly this crate's own tests, never the framework's
core render path. Everything here is a pure function: data in, findings out,
no window, no GPU, no screen reader in the room. `contrast` hand-computes the
published WCAG relative-luminance and ratio formulas, reusing
`vieww-foundation`'s own sRGB linearization rather than duplicating it; the
audit modules found and pinned a real property of the framework's own
built-in Apple color schemes — several of their role pairs fail AA — rather
than tuning a test to hide it.

## `vieww-interaction` and `vieww-scroll`: input and scrolling that don't need a tree

`vieww-render`'s own `pointer` and `focus` modules already answer
"which render object does this event belong to" — that needs the tree, so it
lives there. `vieww-interaction` is deliberately smaller and sits beside
`vieww-gestures`: keyboard-accelerator matching (`shortcut`), a named-action
command registry built on it so a menu item, a toolbar button, and an
accelerator can all point at the same action instead of each holding its own
copy (`command`), and the portable half of dragging content *out* of the app
to the OS (`drag_out` — whose own doc is explicit that its `NullDragStarter`
is an honest stub, not a fake success path, because no portable winit
drag-start API exists yet, tracked upstream as winit issue #1550).

`vieww-scroll` gathers what didn't already have a home: physics
(`vieww_gestures::ScrollPosition`), the element-layer controller
(`vieww_element::ScrollController`), and virtualized lists
(`vieww_widget::ListView`) already existed and aren't duplicated here. What
this crate adds: a real, draggable, proportionally-sized `Scrollbar` widget
with auto-computed thumb geometry, and `NestedScroll` — pure arithmetic
splitting one drag delta between an inner and an outer scroll position, for a
sub-list inside a page or a collapsing header above one.

## `vieww-hardware` and the platform crates: asking the machine, then acting on the answer

`vieww-hardware` turns "we cannot test that here" from a line in a document
into a value a test can check at runtime: `skip_without!(display)` runs a
test on a machine with a screen and prints *why* it didn't, and passes,
on one without — the gap becomes a check instead of an assumption nobody
verifies.

`vieww-platform` is the seam between the framework and the OS for everything
that isn't rendering, windowing, or input: clipboard, notifications, the
share sheet, deep links, opening URLs, key/value storage. It is deliberately
just traits, a registry, and a recording implementation for tests — no FFI,
no `#[cfg(target_os)]`, no dependencies — so a widget can call
`ctx.services()` in a headless CI container against a test recorder and
assert on what was *requested*, with the exact same widget code that runs
against the real platform crate in production.

`vieww-platform-winit` is the one crate allowed to talk to the operating
system at all. Everything below it is arithmetic — `vieww-render` can lay out
a tree, paint it, hit test it, and hand back a scene without a window, which
is why hundreds of tests in this workspace run without one — but a scene
nobody sees is not a frame, and the things that make it one (a window, a
swapchain, a vsync tick, a finger) belong to the OS. This is deliberately the
only place they come in.

## `vieww-effects`

Blur, color-matrix filters, and blend modes, split the same way the rest of
the paint stack is: pure CPU pixel operations (`cpu`) that are the
correctness reference — slow, but they run everywhere including CI, and
they're what any future GPU implementation is tested against — and widget
wrappers (`BackdropBlur`, `FilterChain`, `Blend`) that compose with the
existing widget tree above them.
