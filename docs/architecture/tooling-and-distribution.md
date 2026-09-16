# Tooling and distribution

These crates ship *around* a `vieww` application rather than inside one — an
app never links against `vieww-cli`, and the inspector in `vieww-devtools` is
read-only by construction so a debugging session can never be the thing that
corrupts a tree.

## `vieww-test-harness`: a window-free event loop

`vieww_render::LoopHarness` is the real thing underneath this crate: it runs
the exact same `next_action` decision a winit event loop makes, but against a
synthetic clock with no window. `vieww-test-harness` wraps that with what a
test actually reaches for on every use: event injection (`tap`, `key`,
`scroll` — one call, one frame request), time control (`tick` advances by
exact vsync intervals and reports what happened), and frame assertions
(`FrameReport`). Its `visual` module is the "golden suite" piece of this
workspace's testing story: `assert_matches_golden` is a real PNG-baseline
image-diff oracle (write the baseline once, compare byte-for-byte or with a
tolerance thereafter), and `assert_partial_repaint_is_complete` is a damage
oracle checking that a partial repaint actually covers what changed and
nothing more. See [`TRACKER.md`](../../TRACKER.md) for why this workspace
treats *cross-backend* parity testing as a deliberate non-goal rather than a
gap — there is currently one real renderer, not two to compare.

## `vieww-devtools`: an inspector that can't corrupt what it inspects

`Inspector` walks the element tree and reports what it finds — widget types,
build counts, signal subscriptions, layout bounds — and never mutates it,
because a devtools that can change the tree it's inspecting is a devtools
that can corrupt it. Snapshot testing renders a widget to an image and
compares it against a stored golden, pixel-exact by default and perceptual
with a threshold since anti-aliasing differs enough between backends to make
exact-match flaky across machines. Beyond the inspector, a set of focused
sub-inspectors each report on one real data source that already exists
elsewhere in the workspace rather than inventing a new one: a frame timeline,
a render-graph/damage/memory/semantics view, and a GPU inspector whose
`DriverStats` is honestly `None`-always on a machine with no real driver to
query, rather than a plausible-looking fake.

## `vieww-plugin` and `vieww-plugin-macros`: a plugin ABI that survives a different `rustc`

The problem this solves is specific and harder than it sounds: a `cdylib`
plugin is built once, shipped as a binary, and loaded by a host whose `rustc`
version, build flags, and even standard library revision it cannot assume
match its own. A Rust trait object's vtable layout is not guaranteed stable
across any of those variables, so it cannot cross this boundary. The `abi`
module's answer is a `#[repr(C)]` vtable of plain function pointers — the one
shape the C ABI itself guarantees stays laid out the same way regardless of
compiler version on either side. `host` provides a ready-to-use
`HostVTable` for the host side (`stderr_host_vtable`); `registry` loads a
compiled plugin with `libloading`, validates its ABI version before touching
anything else, and drives its lifecycle (`init`, command dispatch, `shutdown`
before the library itself is unloaded — the wrong order there is a
use-after-unload waiting to happen). `#[vieww_plugin]` in
`vieww-plugin-macros` is the codegen that saves a plugin author from writing
the `extern "C" fn vieww_plugin_entry()` trampoline by hand.

This is deliberately a different problem from `vieww-reload`, covered next —
worth being explicit about the distinction since the two crates sound
similar: a plugin's binary compatibility constraint is permanent (it ships
once, separately, to users running an unknown host build), while
`vieww-reload`'s host and guest are compiled together, moments apart, by the
same `rustc` invocation, and that coupling is required for it to work at all
rather than a limitation to route around.

The whole chain is exercised end to end, not simulated: a real fixture plugin
is built via an actual `cargo build` subprocess and `dlopen`ed across the
genuine compiled boundary in `crates/vieww-plugin/tests/example_plugin.rs`.

## `vieww-build` and `vieww-cli`: packaging and diagnostics

Three questions a `vieww` project owner asks, each its own module in
`vieww-build`: `doctor` — "can a build even be attempted on this machine,"
answered with real probes (`rustc --version`, `cargo --version`, the
`RUSTUP_TOOLCHAIN` environment variable, and `vieww-hardware`'s own
`Capability::Display` probe reused rather than re-implemented); `scaffold` —
`vieww new`'s template, real enough that its own integration test compiles
the generated project with an actual `cargo build`; and `package` — turning a
built project into a platform artifact for each of the five targets its
`Cargo.toml` names, with an honest error (never a panic, never a silent
no-op) wherever a required tool or host platform is missing. `vieww-cli` is
the thin command surface over all three (`new` / `doctor` / `build` / `run` /
`test` / `profile` / `package`). The macOS, Windows, and Android packaging
paths are real, unit-tested code — argument construction, tool detection,
error naming — that has not yet been exercised by an actual build on those
platforms' own toolchains, since this workspace was built on Linux; see
[`TRACKER.md`](../../TRACKER.md) for exactly which category of "pending"
that is.

## `vieww-reload`: development-time hot reload

Splits a development build in two — a host owning the event loop, the GPU,
and the element tree, and a guest `cdylib` holding just the application's
widgets. When the guest is rebuilt, the host loads the new one and swaps the
root, and the element tree underneath (every signal, scroll offset, and
half-finished animation) stays exactly where it was, because reconciliation
is what `vieww-element` already does on every ordinary rebuild — a reloaded
guest is just an unusually large one. It's development-only, and its own doc
is specific about *why*: not instability, but that every reload leaks a
library, which is an acceptable cost for a session that ends when you stop
`cargo watch` and an unacceptable one for anything shipped.
