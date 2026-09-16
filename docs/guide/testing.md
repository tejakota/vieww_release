# Testing

vieww is built so that almost all of it runs without a screen. That is not a
convenience — it is why the framework has a test suite at all, and it is
available to your application for the same reason.

## The harness

```rust
use vieww_test_harness::TestHarness;
use vieww_foundation::{Offset, Size};

#[test]
fn pressing_the_button_increments_the_counter() {
    let mut harness = TestHarness::new(Size::new(400.0, 600.0));
    harness.mount(Counter::default());

    harness.tap(Offset::new(60.0, 40.0));

    assert!(harness.driver().elements().debug_tree().contains("Count: 1"));
}
```

`TestHarness` owns a `FrameDriver` and a clock you control. Nothing is real
except the framework: no window, no GPU, no compositor, and no waiting.

What it gives you:

- `mount`, `driver()`, `runtime()`
- input — `tap`, `pointer`, `drag`, `scroll`, `key`, `type_char`, `press`,
  `press_with`
- time — `tick(duration)` advances the clock and runs a frame; `settle(limit)`
  runs frames until nothing is pending; `now()` says where the clock is
- several windows at once — `with_windows`, `window(index)`, `mount_on`

## Time is yours

An animation test that sleeps is slow and flaky. Advance the clock instead:

```rust
harness.mount(Animated::new(...));
harness.tick(Duration::from_millis(150));
// The animation is exactly 150ms in — not "about", and not on a good day.
```

This is also how you test a timeout, a debounce, or anything that would
otherwise need a real second to pass.

## Async, deterministically

`Spawn` is a trait, so a test substitutes one:

```rust
use vieww_foundation::task::Inline;

// Runs the work immediately, on the calling thread, so `poll` is true on the
// very next call. Exactly what a test wants and exactly what a UI must not do.
AsyncBuilder::new(Arc::new(Inline), Arc::new(NoWaker), work, view)
```

No sleeping, no polling loop, no flake. When you want to test the *pending*
state, use a spawner you control that holds the work until you release it.

## Pixels

For things that are only wrong when you look at them:

```rust
use vieww_test_harness::visual;

let frame = visual::render(harness.driver(), Size::new(400.0, 600.0), Color::WHITE);
assert_eq!(frame.at(10, 10), (255, 0, 0, 255));
```

`Frame` can be compared, diffed with a tolerance, and written or read as PNG,
so a golden-image test is a few lines:

```rust
let expected = visual::Frame::read_png(Path::new("tests/golden/settings.png"))?;
assert!(frame.differences(&expected, 2).is_empty());
```

### Two oracles worth knowing about

`assert_partial_repaint_is_complete` renders a frame twice — once in full,
once through the damage-tracking path — and asserts the results are identical.
This is how you catch the specific bug where something changed but was not
marked dirty, which shows up as a stale rectangle on screen and as nothing at
all in an ordinary test.

`assert_matches_golden` is the PNG-baseline version, with the diff and the
failure message already written.

## Accessibility, in a test

The semantics tree is a data structure, so assert on it:

```rust
let semantics = harness.driver().semantics();
assert!(semantics.iter().any(|node| node.label == "Save"));
```

and audit it for the mistakes that are mechanical:

```rust
let findings = vieww_accessibility::audit(&semantics);
assert!(findings.is_empty(), "{findings:#?}");
```

That catches missing labels, undersized touch targets and internal
inconsistencies. It does not catch a label that is wrong, which is why it is
an audit and not a guarantee — see [`accessibility.md`](./accessibility.md).

## What a headless test cannot tell you

Worth being honest about, because the coverage here is good enough to be
misleading:

- **That a screen reader reads it correctly.** The harness checks what
  reaches AccessKit. Whether VoiceOver says something sensible needs
  VoiceOver, a Mac, and a person listening.
- **That it is fast on a phone.** Frame times here are this machine's.
- **That the GPU path agrees.** It does, for what it can draw — see
  `vieww-hal`'s `vulkan_scene` suite, which compares against the CPU
  rasterizer — but that suite needs a Vulkan ICD and is `#[ignore]`d without
  one.
- **That your application updates while idle.** The one async behaviour with
  no test: a value arriving at a genuinely idle application. A desktop under
  test is never idle — a mouse crossing the window produces the frame that
  hides the bug.

## Running the framework's own suite

```sh
./ci/check/checks.sh              # everything, in order
./ci/check/checks.sh --quick      # skip the slow end-to-end suites

cargo test -p vieww-widget                       # one crate
cargo test -p vieww-paint --features native      # the rasterizer needs its feature
cargo test -p vieww-hal --features vulkan -- --ignored   # needs a Vulkan ICD
```

`cargo run --release -p fixtures` renders every rendering feature as a picture
and a number into `fixtures-out/`. That is where a rendering change is
reviewed — by looking at it.

It is also a gate, not only a gallery: it **exits non-zero if any fixture's
own layout overflows**, naming the fixture. A reference image whose column
paints past its box is not a picture of what this framework does, and five of
them were doing exactly that — reporting themselves to stderr, between the
pictures, on every run, for as long as they existed. If you add a fixture and
this fails, the fixture is what to fix.

```sh
./ci/check/platform-check.sh          # does this compile for each platform, here?
./ci/check/platform-check.sh macos    # or one of: linux macos windows ios android web
```

`platform-check.sh` answers a different question from `checks.sh`: not "is
this correct" but "can this machine even build it for that target". Each row
passes, fails at the compiler, or names the SDK, target or toolchain that is
missing. A pass means the code compiles — it never means a frame has reached
a screen there.
