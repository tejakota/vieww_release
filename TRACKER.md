# ViewW status tracker

This file is the honest, current answer to "what actually works, and how do I
check that" across the workspace.

**What is *not* done lives in [`PENDING.md`](./PENDING.md)**, and nothing
belongs on both lists. That split is new. This file used to end with a "What
genuinely remains" section, which meant the record of what had been verified
and the worklist of what had not were the same document — so the worklist was
buried under a history that only grows, and every gap had to be rediscovered by
reading to the bottom. The section is still here for continuity, and
`PENDING.md` is the one to read if the question is "what should I work on".

## The most recent effort (2026-09-15): the GPU compositor, measured standards, one certification command

Scope was a review of the September 15 archive: GPU planner gaps, a
certification script that did not run most suites, a standard runner that fed
literal zeros into two clauses, and a patch-workspace archive. Evidence below
is from `ci/certify/certify.sh` on a 2-core container with lavapipe
(Mesa's software Vulkan), rustc 1.95.0 — **not** the pinned 1.98.1, which this
machine could not download. Source digest of the certified tree is in
`target/cert-linux/meta/source.txt` of that run.

**Verified, by things that ran:**

1. **The uploaded tree did not load as a workspace.** `examples/launch-animation`
   was a member with no directory; removed from `members`. `.cargo/config.toml`
   was missing (dropped with hidden directories), which aborts
   `viewwstudio`'s panic-boundary tests; reconstructed from the contract in
   `ci/check/checks.sh`, `Cargo.toml` and the mobile runner scripts.
2. **The GPU path now covers every `Command`.** Offscreen layers and group
   opacity, all 28 blend modes, layer and backdrop filters, shaped clips,
   outer/inset/rotated shadows, per-fragment gradients, rotated and mip-blended
   images, colour glyphs, dashed strokes. `vieww-hal`'s new `vulkan_compositor`
   suite (17 tests) compares every pixel with `NativeRenderer`: max channel
   difference 1 wherever geometry is pixel-aligned. Mutations (blur kernel,
   Hue formula, even-odd winding, miter limit) each fail it.
   `fixtures --census`: **23 of 23 fixtures plan complete**, 0 mismatches
   beyond the geometry-edge band. `test-gpu-work`: `unsupported_gpu_commands=0`
   measured, worst frame 0.378% of pixels over tolerance 8 — identical across
   ten consecutive runs.
3. **Bugs this found in the existing GPU path**, each fixed with a test: the
   scene shader did not parse (every Vulkan scene test failed on the uploaded
   tree); group opacity folded into child alpha; radial/sweep gradients
   interpolated per vertex; rotated images drawn as bounding boxes; atlas
   growth moving live entries; per-instance atlas versions colliding across
   planners; even-odd fill rule; device-space strokes and ignored dashes;
   lyon's miter limit at twice the CPU's; 16-bit indices silently emptying large
   paths; image identity by reusable address (also in the **CPU** `MipCache`).
   Details: `docs/GPU-RENDERER-STATUS.md`.
4. **`vieww-standard` measures all twelve clauses.** `steady_allocations` by a
   counting global allocator (first honest reading: 7 per steady frame, now
   **0** over 60 frames); `unsupported_gpu_commands` from real plans (**0**
   over 91 frames). Fixes behind the zero: scratch buffers in
   `ElementTree`/`RenderTree`/`LayerTree`, and `RenderTree::set_root` clearing
   the incremental boundary cache on every frame — the cache never reused a
   subtree in a real frame loop. `NativeRenderer::render_in_place` /
   `render_retained_in_place` / `last_frame` stop the winit presenter copying
   the frame per present. Report: startup 27.5 ms, worst frame 13.1 ms, p95
   11.4 ms, latencies 1 interval, drift/trim/a11y/typography 0, PASS.
5. **One command runs every suite.** Workspace tests (**4,225 passed, 0 failed**,
   now `--no-fail-fast`), GPU planner, shaders, paint-native, Vulkan (37
   passed), GPU workload, census, premium UI, fixtures, all six stress/fidelity
   suites, `vieww-standard`, web baseline, plus measured gates. Skips are
   `SKIPPED(reason)` and `COMPLETE=false`; `VIEWW_CERT_STRICT=1` fails on them.
6. **The web verifier lives in the repository** (`examples/test-web/verify_web.py`).
   Run against the archive's prebuilt wasm and today's native baseline: both
   pointer states **EQUAL**, byte for byte, in headless Chromium.
7. **Release hygiene.** 26 `.orig`/`.rej`/`.bak` files (every rejected hunk
   checked as already present), a truncated copy of the archive nested inside
   itself, a `cargo metadata` dump with home paths, stale `cert-linux/`
   evidence, one-off patch scripts and build outputs removed.
   `ci/check/release-clean-check.sh` fails on any of them; `ci/release/package-source.sh`
   builds an archive only from a clean tree.

**Not verified, and why:**

- `suites/animation-stress` **fails its 60 Hz budget on this machine** (p95
  24.3 ms). The pristine uploaded source measures the same (p95 23.7–24.6 ms
  on the same machine), so it is hardware, not a regression — but it is a
  failure here, and the summary says `FAILED=1` for it.
- Web build and `wasm-check` (no `wasm32-unknown-unknown` target reachable),
  the desktop suite (no display), and `ci/certify/certify-windows.ps1` (no
  Windows) did not run.
- Nothing here ran on real GPU hardware. GPU timings above are lavapipe.

## The most recent effort: the web backend compiled, run, and certified byte-for-byte

The crate below (item 8 of the previous effort) said `vieww-platform-web` had
never been compiled. On 2026-09-14, on a machine that *can* reach
`static.rust-lang.org`, that changed — and then went three steps past it.

**Verified, by things that ran:**

1. **`ci/check/wasm-check.sh` passed for the first time.** The target installed
   normally, and the first honest clippy run on `wasm32-unknown-unknown`
   found seven `-D warnings` failures — all in `vieww-paint`, the crate the
   web backend builds against, all in code that had only ever been linted for
   the host target: two `#[expect]`s that do not fire on this target
   (`f32::from(u8)` is exact, and the misattributed one guarded no cast at
   all), a `contains_key`+`insert` pair, a `len() > 0`, two
   `needless_range_loop`s, and a `too_many_arguments` on `paint_shape`. All
   fixed; the gate is green, and `test-web` (below) holds it there with the
   same `-D warnings` on both targets.

2. **The web backend *ran*, and its pixels are provably the rasteriser's.**
   `examples/test-web` (new) renders one deterministic scene — gradients, a
   shadow card, a blurred circle-clipped layer, a rotated group, Latin + CJK
   text through the embedded fallback chain, and a live tap counter — through
   `WebApp` in a headless Chromium, and compares the canvas readback byte for
   byte against a native `render_to_pixels` of the same tree at
   `devicePixelRatio` 1. **Equal: all 2,304,000 bytes, in both pointer
   states** — after a real click, the canvas matches the native render of the
   tree at `taps = 1`, which is the pointer pipeline, the reactive rebuild
   and the rasteriser all agreeing across the wasm boundary. Three more taps
   move exactly the counter's pixels again; a wheel event does not kill the
   loop; the console is clean. `apps/viewwsite` (the product page, DOM
   backend with the canvas demo island inside it) loads, scrolls, reveals,
   and taps its counter in the same browser session, also clean.

3. **The workspace's own gates hold.** `cargo clippy --workspace --all-targets
   --features "vieww-paint/native,vieww-hal/vulkan" -- -D warnings` is green
   — it found fifteen more lints in the example suites that `cargo check` had
   silently passed (unused bindings left behind by refactorings, two
   `fn new` that returned `WidgetNode`, a `let_and_return`, two unaliased
   screen-array types, a match that wanted a guard, an `as u64` to `u64`);
   all fixed. `cargo fmt --all -- --check` is green too — the submitted tree
   was not fmt-clean (the fmt stage of `ci/check/checks.sh` was already red in the
   baseline), so this is a formatting-only pass over ~30 files, no semantics
   touched, all suites re-run green after it.

**Not verified, and unchanged from below:** the GPU mesh path's gates
(`vieww-gpu` scene completeness for real screens), and every non-Linux
toolchain. The wasm certification is Chromium-on-Linux; other browsers and
OSes inherit the byte-parity claim only insofar as they implement
`put_image_data` and `requestAnimationFrame` to spec.

## The effort before that: text on the GPU, and four gates that were not gates

Six things, and as always the split between verified and not is the point.

**Verified on this machine**, by tests that ran:

1. **The GPU path draws text.** `vieww_gpu::Planner` holds a glyph atlas
   (`vieww_gpu::atlas`) and plans a glyph run into one textured quad per glyph,
   batched into the same vertex buffer as the shapes around it;
   `vieww_hal::vulkan::SceneRenderer` uploads the atlas as an `R8_UNORM` image
   and samples it in `scene.wgsl`. `ScenePlan::is_complete` no longer refuses a
   frame for having a label on it, which is the difference between a GPU path
   that can draw no real screen and one that can.

   **Seven new tests in `crates/vieww-hal/tests/vulkan_text.rs`, all passing on
   `lavapipe`**, and their tolerance is the interesting part: **every pixel,
   within 1/255, including the antialiased rim of every glyph.** The shape
   parity suite compares two independent rasterizations and so can only assert
   about interiors; there is only ever *one* glyph rasterizer here — the GPU
   samples the coverage `vieww-paint` produced — so the only permitted
   difference is the atlas holding that coverage as `u8` instead of `f32`. A
   mirrored frame, a glyph one pixel out, a wrong atlas patch and a colour
   applied before coverage all fail that bound, and all of them fit inside
   "interiors match".

   Solid geometry samples a reserved full-coverage texel in the same atlas, so
   one pipeline draws a panel and the label on it in one `draw_indexed`; the
   asserted form of that claim is
   `a_label_on_a_panel_is_one_batch_and_still_matches`.

2. **A live Y-flip in `text.wgsl`**, found while doing the above. It mapped
   `y = 0` to NDC −1, which is the bottom of the framebuffer in the WebGPU
   convention `naga` adjusts from — the same defect this file already celebrates
   catching in `solid.wgsl`, still sitting in the shader library, for the reason
   that makes this class worth naming: **nothing had ever drawn through it**, so
   no test could disagree with it. Fixed. The whole-scene path is what renders
   text, and its parity tests use asymmetric text near a frame edge precisely so
   a flip cannot pass.

3. **`vieww new` still shipped the theme bug this file records as fixed.**
   `App::theme` landed, and `apps/viewwstudio`'s two templates adopted it — but
   `crates/vieww-build/src/scaffold.rs`, which is what `vieww new` writes and
   what `docs/guide/getting-started.md` documents as the way in, was left on
   `background(ThemeData::dark().colors.surface)` with no `Theme` mounted. So
   the first vieww code most people ever run still drew near-black text on a
   near-black ground, after the defect was fixed everywhere else.

   Fixed in the scaffold and in the guide.
   `the_scaffold_mounts_a_theme_it_also_paints_the_window_from` asserts on the
   generated source — the defect is a *missing call*, and only the source says
   whether a call is there, which is why the existing scaffold tests (all of
   which passed the whole time) could not see it.

4. **Five fixtures had layouts that did not fit, and the gallery knew.**
   `vieww_render::overflow::reported` has existed all along, with a doc comment
   saying a number "is what turns that from something a reviewer might notice
   into something a script can refuse" — and nothing read it. So the gallery
   printed `RenderColumn overflowed by …` on stderr, between the pictures, on
   every run, and `fixtures-out/20-settings.png` — the reference image for what
   a settings form looks like in this framework — showed its eighth row sliced
   through and its ninth missing.

   `runner.rs`, `motion.rs` and `interaction.rs` now count overflows per
   fixture, name the fixture, and `main.rs` exits non-zero. All five are fixed
   at the fixture: 20-settings 9 rows → 7, 31-stagger 8 → 5, the interaction
   list 12 → 10, `page_behind`'s spacing 10 → 8, the shared-element card 100px
   → 124px tall, and both faces of that transition wrapped in `Offstage` while
   fully faded — which also stops a 192px detail view being laid out into a
   60px box and painted outside the card meant to contain it. **The gallery
   now reports `no fixture's layout overflows`.**

   These change the gallery's timings, because they change what it draws. That
   is the correct trade and it is written down here rather than absorbed:
   a fixture's number is only comparable across a change if the fixture is
   drawing the right picture, and these were not.

5. **Two `web-sys` types were named and not enabled.** `vieww-platform-web`
   uses `web_sys::Event` (every listener closure takes one) and
   `web_sys::EventTarget` (both listener helpers coerce the window to one) and
   neither was in its feature list — two guaranteed `cannot find type` errors,
   parked behind a target install nobody could do. `ci/check/wasm-check.sh` now
   audits that list against the crate's own source **before** touching a
   toolchain, so this class is caught on any machine, and `ci/check/checks.sh` runs
   that stage on every run. Both features added.

6. **`ci/check/checks.sh` gained the two gates above** (the web-sys audit and the
   fixture gallery), and `ci/check/platform-check.sh` is new: one command per
   platform, each of which passes, fails at the compiler, or refuses to start
   naming the SDK, target or toolchain that is missing. See below.

7. **The docs gate was not covering the Vulkan backend.** `vieww_hal::vulkan`
   is behind a feature and the docs stage enabled only `vieww-paint/native`, so
   `cargo doc` never documented the module — the largest body of new code in
   the workspace had never been through that gate, and was carrying a broken
   intra-doc link (`super::VulkanDevice`, in `vulkan/scene.rs`'s module doc)
   the whole time the stage reported green.

   This file records `cargo doc -D warnings` as clean twice, at 117 links and
   then at 36, both times noting that the cause was nothing running the check.
   This is the third version of the same thing and a different shape of it: the
   check was running, on everything except the code anyone was actually worried
   about. Feature added to the stage, link fixed.

**Not verified, and the distinction is not cosmetic:**

- **No GPU number here was measured on real graphics hardware.** `lavapipe` is
  a software Vulkan implementation. What the text pipeline has is
  *correctness*, verified against the CPU rasterizer; it has no performance
  claim at all, and this file should not acquire one until it runs on a GPU.
- **Every platform other than Linux is still unbuilt**, unchanged from the
  position below. `ci/check/platform-check.sh` is the command that changes that; it
  cannot change it from this machine.

### What was actually run for the section above

Stated precisely, because "the tests pass" is the claim this file exists to
make checkable:

| gate | result |
|---|---|
| `cargo test -p vieww-foundation -p vieww-element -p vieww-widget -p vieww-render -p vieww -p vieww-text -p vieww-scroll -p vieww-accessibility` | 2,335 passed, 0 failed (72 binaries) |
| `cargo test -p vieww-paint -p vieww-gpu -p vieww-build -p vieww-hal` | 319 passed, 0 failed (17 binaries) |
| `cargo test -p vieww-hal --features vulkan -- --ignored` | 20 passed, 0 failed — 13 pre-existing, **7 new text tests** |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings`, plus `-p vieww-hal --features vulkan` | clean |
| `cargo doc --no-deps -D warnings`, including `vieww-hal/vulkan` for the first time | clean |
| `cargo run --release -p fixtures` | 23 fixtures, 7 motion sequences, 2 interaction suites, **no layout overflows** |

**`apps/viewwstudio` was not re-run**, and neither was a single
`cargo test --workspace`. Not because either is expected to fail — nothing in
this effort touches the studio — but because this machine ran out of disk
building them (the workspace's debug artefacts exceed its allowance), and a
number this file did not observe is a number it should not print. The previous
effort's 608 studio tests stand as that effort's measurement, not as this
one's.

### `lavapipe` and concurrent devices

`crates/vieww-hal/tests/vulkan_scene.rs` began to **SIGSEGV inside
`libvulkan_lvp.so`** on roughly half of multi-threaded runs once the suite
started creating images and samplers. Every frame at the fault is lavapipe's
own; the deepest frame belonging to this workspace is an ordinary
`wait_for_fences`; the same eight tests pass every time under
`--test-threads=1`, and nothing in this crate shares a Vulkan object between
threads.

So it is serialised **in the tests**, where the constraint is, with a mutex and
the observation written beside it — rather than with synchronisation inside
`SceneRenderer` that would slow a real driver down to accommodate a software
one. Anyone adding a test to those files needs the guard; that is why the
explanation lives in the file rather than only here.

### One command per platform

`ci/check/platform-check.sh {linux|macos|windows|ios|android|web}` — or no argument
for every one this machine can attempt. Each row of the "never built on that
platform's own toolchain" status below now has exactly one command behind it,
and each does one of three things: passes (saying explicitly that compiling is
not running), fails at the compiler, or refuses to start and names the missing
prerequisite.

The third is what it is really for. A missing Rust target looks like a broken
checkout; a missing NDK looks like a Rust linker bug (see `ci/mobile/apk.sh` on
`-lunwind`); a missing Xcode looks like nothing at all until forty seconds in.
And a machine that cannot reach `static.rust-lang.org` gets `cargo --version`
failing under a twenty-six frame backtrace with a perfectly good toolchain on
disk — the same trap recorded below as costing a day in viewwstudio's own
toolchain test. The script checks for that first and tells you to set
`RUSTUP_TOOLCHAIN`.

## The effort before that one: nine things, seven verified

Kept in full rather than summarised, because the reasoning in it is still the
reasoning behind the code. Only the heading has changed: this was "the most
recent effort" until the section above it existed, and a file whose top two
sections both claim to be the newest is a file nobody can date.

Nine things were done. Seven are verified on this machine; two are not, and
which is which is the whole point of this file.

The verified half is verified by **2,873 passing tests** (2,265 across the
framework crates, 608 in the studio), `cargo fmt --all --check`, and
`cargo clippy --workspace --all-targets -- -D warnings` — all clean.

**Verified here:**

1. **The GPU path draws.** `vieww-gpu::scene` turns a `vieww_paint::Scene`
   into a batched `ScenePlan` — one vertex buffer, per-vertex colour, one
   draw call per scissor change — and `vieww_hal::vulkan::SceneRenderer`
   executes it on a real Vulkan device with a persistent pipeline (built
   once; only the target is rebuilt on resize, and the geometry buffers grow
   rather than reallocate). `lavapipe` was installed so this could be *run*
   rather than only compiled: **13 Vulkan tests pass**, five of them
   comparing the GPU's output against `NativeRenderer`'s pixel for pixel away
   from antialiased edges.
2. **A latent Y-flip in the shipped shader, found and fixed.** `solid.wgsl`
   wrote the Vulkan clip-space convention while `naga`'s SPIR-V writer was
   already flipping Y for it (`WriterFlags::ADJUST_COORDINATE_SPACE` is on in
   `Options::default()`), so every frame through it was vertically mirrored.
   It survived because the only test exercising it drew a square symmetric
   about the horizontal centre line. Both shaders now author in WebGPU's
   convention and let naga adjust per backend, and `vulkan_mesh_smoke`'s
   geometry is asymmetric in both axes so a flip cannot pass again.
   **The parity harness had the same class of flaw and it was caught the same
   way**: `interior_mismatches` originally decided "is this pixel on an edge"
   by asking whether a *neighbour also disagreed*, which returns zero when
   everything disagrees. Swapping red and blue in the planner passed three
   parity tests. It now keys off transitions in the reference image, and that
   mutation fails five.
3. **`#[widget]`.** A composed widget was four items before it said anything;
   `debug_name`, `kind` and the `widget_node_from!` call are now written by
   the attribute. 11 tests, each comparing the attribute's output against the
   hand-written long form rather than asserting about the macro in isolation.
   The scaffold and the `counter` example use it.
4. **The Apple colour schemes pass WCAG AA.** They failed seven pairs between
   them, and this file's previous version recorded that as a property of
   Apple's palette — true, and the wrong place to stop for a scheme the
   framework *ships as a default*. Both now use Apple's own published
   increased-contrast colours, plus a black label on the dark accent fills;
   see `ColorScheme::apple_light`'s doc for the numbers and
   `theme_audit.rs`'s for why pinning the failure was not good enough.
5. **The second text stack is gone.** `shaping`/`layout`/`selection`/
   `editing`/`bidi`/`fonts` — 2,262 lines reached by nothing, whose
   `layout::Paragraph` hard-coded a one-glyph-per-code-point `NaiveShaper`
   and whose crate-root re-exports shadowed `vieww-foundation`'s `FontFamily`
   and `FontWeight`. The `harfbuzz` feature went with it: its description
   claimed the default build lacked full Unicode shaping, which was never
   true of any build (`cosmic-text` shapes with `harfrust`).
6. **`task::Pool`.** `Threads`' own doc admitted it starts one OS thread per
   task; the advice was "bring a real runtime", which is wrong for an
   application whose async work is a dozen image loads. A bounded,
   dependency-free pool, with the panic-isolation and drop-joins-workers
   behaviour its doc claims each covered by a test.

7. **A test that failed for the wrong reason.**
   `viewwstudio`'s `the_host_environment_finds_the_targets_this_machine_has`
   guarded itself with `rustup --version` and then asserted on what
   `rustup target list --installed` returns. Those come apart on any machine
   that cannot reach `static.rust-lang.org`: `rustup` re-syncs channel
   metadata before answering a query, so `--version` exits 0 while
   `target list` downloads nothing — and the studio, which correctly reported
   an empty list, took the blame. The guard now runs the command the
   assertion depends on. 608 studio lib tests pass.

**Not verified, and the distinction is not cosmetic:**

8. **`vieww-platform-web` had never been compiled** — *closed 2026-09-14, see
   the top section.* This is the record as it stood: not "compiles but is
   untested" — never through `rustc` at all, because the machine it was
   written on could not install `wasm32-unknown-unknown` (`rustup target
   add` needs `static.rust-lang.org`, outside its network policy). Every
   dependency and the single module are gated to `cfg(target_arch =
   "wasm32")` so it builds to nothing everywhere else and cannot break a
   verified build, and it is `publish = false` for the same reason.
   `ci/check/wasm-check.sh` was the script that changed this; it found seven
   lints on its first honest run, then passed — and the backend has since
   been *run*, with its canvas certified byte-for-byte against the native
   rasteriser.
9. **Metal, D3D12, and every non-Linux platform** are unchanged from the
   position below: real code, unit-tested where the logic is testable on this
   host, never built on their own toolchains.

`cargo check --workspace --all-targets` and `cargo clippy --workspace
--all-targets -- -D warnings` are both clean, and the version moved to 0.1.0
— see `Cargo.toml`'s comment for what that does and does not claim.

## How to read the status column

- **Real, tested (CPU/host)** — real logic, real tests, passing on this
  machine, no further work needed to trust it on any host this workspace
  targets.
- **Real, tested (host) + GPU-pending** — the code path is real and its
  CPU-testable surface (construction, validation, CPU-side bookkeeping) is
  tested here; the GPU-facing half (a real device, a real shader compile, an
  actual frame) is not exercised by the host-only suite.

  Two corrections to what this row used to say. It named "an actual `wgpu`
  device", and **there is no `wgpu` in this workspace** — it went with vello,
  and the sentence outlived it; the device is an `ash`/Vulkan one. And it said
  the sandbox "has no GPU", which stopped being the operative limit once
  `mesa-vulkan-drivers` was installed: `lavapipe` is an ICD, the Vulkan suites
  run, and what is genuinely pending is a run on **real graphics hardware**,
  which is a performance question rather than a correctness one.
- **Real, tested (host) + other-toolchain-pending** — same idea, for a
  platform whose own toolchain (Xcode, an Android NDK, MSVC) is not present
  on this Linux sandbox: macOS/iOS/Windows/Android packaging paths, for
  instance. The code is real and unit-tested; it has not been exercised by
  an actual build on that platform.
- **Scaffolding only** — module structure and a doc comment exist; the real
  implementation is still to be written.

## Crates added or substantially extended in that effort

| Crate | What it is | Status | Verified with |
|---|---|---|---|
| `vieww-image` | Mipmaps, atlas packing, GIF sequences, color-space tagging, a residency cache | Real, tested (CPU/host) | `cargo test -p vieww-image` — 33 passed; clippy clean |
| `vieww-devtools` (extensions) | Frame timeline, render-graph/damage/memory/semantics/GPU inspectors, hand-rolled JSON export | Real, tested (host); GPU inspector's `DriverStats` is honestly `None`-always without a real driver | `cargo test -p vieww-devtools --all-features` — 63 passed; clippy clean |
| `vieww-build` / `vieww-cli` | `vieww new/doctor/build/run/test/profile/package` — real subprocess invocation, a real scaffold-and-compile integration test | Real, tested (host); macOS/Windows/Android packaging paths are real-but-unexercised on this Linux host | `cargo test -p vieww-build -p vieww-cli` — 47 + 21 passed (1 ignored by design — compiles a full scaffolded app, run with `--ignored`); clippy clean |
| `vieww-interaction` | Keyboard shortcuts, a command registry, outgoing-drag session state machine | Real, tested (CPU/host); `NullDragStarter` is an honest stub — no portable winit drag-start API exists (tracked upstream by winit issue #1550) | `cargo test -p vieww-interaction` — 28 passed; clippy clean |
| `vieww-scroll` | A draggable, proportional `Scrollbar` widget; `NestedScroll` drag-splitting between an inner and outer scroll position | Real, tested (CPU/host) | `cargo test -p vieww-scroll` — 18 passed + 1 doctest; clippy clean |
| `vieww-accessibility` | WCAG contrast math, static semantics-tree audits (missing labels, undersized touch targets, internal-consistency checks), theme contrast scanning | Real, tested (CPU/host) — and found a real property of the framework's own built-in Apple color schemes (they fail several AA pairs) rather than tuning the test to hide it | `cargo test -p vieww-accessibility` — 27 passed; clippy clean; `cargo doc` clean; example runs |
| `vieww-plugin` / `vieww-plugin-macros` | Stable, versioned, cross-`rustc` plugin ABI: `#[repr(C)]` vtables, a safe `Plugin` trait, `#[vieww_plugin]` codegen, `PluginRegistry` (`libloading`-based) | Real, tested end to end — including a real `cdylib` fixture plugin built with a genuine `cargo build` subprocess and `dlopen`ed for real across the actual compiled boundary | `cargo test -p vieww-plugin -p vieww-plugin-macros` — 15 + 5 passed; clippy clean |
| `vieww-test-harness` (extensions) | `criterion`-based benchmarks over real `TestHarness` operations; a `TestReport` with hand-rolled JSON/HTML rendering | Real, tested (CPU/host) | `cargo test -p vieww-test-harness` — 21 passed; `cargo bench -p vieww-test-harness --no-run` compiles, and a `--test` smoke run executes all three benchmarks successfully; clippy clean |
| `vieww-widget` (tokens + tiers extension) | `WindowSizeClass` (the Compact/Medium/Expanded breakpoints, published via the existing `Inherited<T>` mechanism), an open `DesignToken<T>`/`TokenSet` registry, and `WidgetTier`/`Tiered`/`TierBudget`/`TierGate` (the assessment's own Primitive/Behavior/Pattern widget classification, made actionable) | Real, tested (CPU/host); a fourth planned item — a dedicated "container query" widget — was deliberately not built, since `LayoutBuilder` (pre-existing) already is one; documented in both types' own docs rather than duplicated | `cargo test -p vieww-widget --lib` — 505 passed (25 new across both efforts); clippy clean; `cargo doc` clean; both `tiers` doctests pass |

## Naming note, not a gap

An earlier architecture sketch named separate `vieww-window`/`vieww-native`
crates. That functionality already exists under different names —
`vieww_render::window`, `vieww-platform-winit`, and
`vieww_foundation::capability::platform_view` — and a risky rename/extraction
purely to match a proposed name was deliberately not done. If the naming is
ever revisited, do it as its own tracked refactor, not folded into unrelated
feature work.

## Cross-backend parity — a deliberate non-goal, not a gap

The original architectural assessment's "cross-backend parity" suite assumed
two independent rendering backends that could disagree with each other. This
workspace does not have that shape: the vello-based second backend was
removed earlier in that effort, and `vieww`'s own `NativeRenderer` (CPU) plus
the Vulkan HAL path (there is no `wgpu` here; that word was left behind by
vello) are one implementation compiled two ways, not two implementations.
There is nothing to compare "across" — parity-testing a renderer against
itself would only restate whatever bugs it already has as if they were
agreements.

**Except in one place, and it is the exception that proves the rule.** The GPU
path's *geometry* is `lyon` tessellation and hardware sampling where the CPU
path's is analytic scanline coverage, so those genuinely are two
implementations of the same question and `tests/vulkan_scene.rs` compares them
— with a tolerance that allows the edges to disagree, because they must. Its
*text* is not: both sides use the same rasterised coverage, so
`tests/vulkan_text.rs` can and does demand every pixel within 1/255. Which
kind of comparison is available depends on whether there are really two
implementations underneath, which is the whole point of this section. What `vieww-test-harness::visual` provides instead,
and what genuinely covers the "golden suite" half of item 13, is
`assert_matches_golden` (`crates/vieww-test-harness/src/visual.rs`): a real
PNG-baseline image-diff oracle plus `assert_partial_repaint_is_complete`, a
damage-region oracle. If a second real backend is ever added, parity testing
belongs here as new golden baselines shared across both, not as a separate
mechanism.

## Widget tiers — done

Item 11 names "design-system engine + adaptive layout + widget tiers" as one
line. This effort originally built only the design-system/adaptive-layout
two thirds of it (`vieww-widget::tokens` — `WindowSizeClass`,
`DesignToken<T>`, `TokenSet`; `LayoutBuilder`'s existing container-query
behavior documented rather than duplicated) and left "widget tiers"
unaddressed, because at the time no concrete meaning for "tier" had been
pinned down against source.

It has since been built, against the actual source: the external
architectural assessment this workspace was built from names, in its own
§9, exactly three widget levels — "Primitive widgets" (`Box`/`Flex`/`Stack`/
`Scroll`/`Text`/`Image`/`Transform`/`Clip`/`CustomPaint`), "Behavior
widgets" (`Focusable`/`Hoverable`/`Draggable`/`Dismissible`/`Scrollable`/
`Animated`/`Transition`/`Portal`/`Overlay`/`Semantics`), and "Design-system
widgets" — closing with "ViewW should allow custom design systems without
needing to become forked widgets." `vieww-widget::tiers` (new) is that
classification applied to this crate's real widget catalog:
`WidgetTier::{Primitive, Behavior, Pattern}`, a `Tiered` trait each widget
type implements once, and `TierBudget`/`TierBudgetProvider`/`TierGate` — the
same ambient-publish-and-read mechanism `WindowSizeClass` already
established — so a `Pattern`-level widget a design system built from this
crate's primitives and behaviors can name a simpler fallback and degrade to
it under a tight budget, without forking anything here to do so. 14 new
tests, all passing; clippy clean; both doc examples compile and run.
`cargo test -p vieww-widget --lib` — 505 passed (was 491); clippy clean.

## The `vieww_beta_0.3` release archive, and what it did not contain

The archive this workspace was released as contains **1,224 entries and no
dotfiles at all**. That is what an archive built without them looks like, and it
is not cosmetic:

- **`.cargo/config.toml` was missing.** `Cargo.toml`'s own `[profile.release]`
  comment says, at length, that `-C prefer-dynamic` "has to be set as a
  rustflag, so it lives in `.cargo/config.toml`" — and it is what makes the
  studio and the preview `cdylib` it `dlopen`s share one `libstd`, which is what
  makes `catch_unwind` catch a panic raised inside somebody's screen. Without
  the file, every build produced a studio whose panic boundary aborts the
  process: `fatal runtime error: Rust cannot catch foreign exceptions`, SIGABRT,
  the window and the unsaved buffer gone. `apps/viewwstudio/tests/pipeline.rs`
  reproduced it exactly, and did so on the pristine archive as well as on a
  modified tree, which is how it was confirmed as shipped rather than
  introduced. The file is restored, with the reasoning written into it.

Anything else that lived in a dotfile — a CI workflow, an editor config — is
gone from the archive the same way and has not been reconstructed, because
there is no record here of what it contained.

## `ci/check/checks.sh` had not been able to run since the vello removal

The docs stage passed `--features "vieww-paint/gpu,vieww-paint/hybrid,vieww-paint/cpu"`.
Those three features were `vieww-paint`'s vello-based backends and they were
deleted with vello. Cargo does not warn about an unknown feature on the command
line; it fails resolution outright, so that stage and **every stage after it**
— MSRV, licences, the test suite, the end-to-end reload suite, the packaged
studio — had not executed since. Fixed to `vieww-paint/native`.

Separately, the test stage ran a plain `cargo test --workspace`, which does not
enable `vieww-paint/native` — so the 224 tests covering the renderer were not
failing in CI, they were not being compiled. The stage now enables it, and the
four test targets that need it declare `required-features` so that
`cargo test -p vieww-paint` skips them cleanly instead of failing to compile
with `unresolved import vieww_paint::native`.

## `ci/check/checks.sh` was failing at its second stage, not its fourth

Above, the docs stage is described as unable to run since the vello removal.
It never got that far. `cargo fmt --all -- --check` sits earlier in the same
script, and **119 files** had drifted out of `rustfmt`'s shape — so the script
stopped there, and everything after it (clippy, build, docs, MSRV, licences,
tests, the end-to-end suites, the packaged studio) had not executed either.

The workspace is now formatted and the gate passes. Note for anyone re-running
it: `examples/fixtures/src/screens.rs` needs **two** `cargo fmt` passes to reach
a fixed point, because it contains a `children![]` macro invocation whose
contents `rustfmt` re-flows only once the surrounding call has been collapsed.
That is a `rustfmt` behaviour, not a repository one, and `cargo fmt --all` twice
is the whole workaround.

## The size budget was recorded on a laptop and first measured on CI

`ci/check/size-budget.txt` held **544880 bytes** for `x86_64-unknown-linux-gnu`. The
first time the gate ran on CI it measured **593128** — 8.9%, past the 5%
ceiling — and it measured the same figure on every subsequent run, to the byte.

That reproducibility is what settles what it means. A budget is per-toolchain
and per-target by construction, and the script's own header says so: "a
different rustc, a different libc, or a cross build produces a different size
for identical source". The recorded number came from a developer machine; this
one comes from CI's `rustc 1.98.1` on the runner's libc. The two were never
comparable, and the gate had no way to say so because the *triple* matched —
which is the one thing it does check.

So the figure is re-recorded at **593128** against the machine that will be
doing the measuring from now on, and the 8.9% is not attributed to anything,
because there is nothing yet to attribute it to. What that buys is a gate whose
next red is a real one: a second CI figure that moves 5% off this baseline is a
change in this repository, not a change of laptop.

**What this does not do** is establish that the framework did not grow. If that
question matters, `cargo bloat --release -p vieww --example counter` on a runner
answers it, and this section should be replaced with the answer rather than
extended.

## The 56 feature examples had silently opted out of the workspace MSRV

`Cargo.toml` declares `rust-version = "1.85"` under `[workspace.package]`, and
every crate under `crates/` and `apps/` inherits it with
`rust-version.workspace = true`. The 56 crates under `examples/features/` did
not. They inherited `version` and `edition` and stopped there.

Nothing announced that, and two gates were quietly weaker for it.

**Clippy suggests APIs that do not exist on the declared minimum.** Clippy reads
`package.rust-version` to decide whether an MSRV-gated lint may fire. With no
`rust-version`, it assumes none, and
`examples/features/54-image-mipmaps-atlas/src/main.rs:45` was told to replace
`chunks_exact_mut(4)` with `as_chunks_mut::<4>().0` — an API stabilised in Rust
**1.88**, three releases after the minimum this workspace promises. Taking that
suggestion would have passed the clippy stage and then failed the MSRV stage of
the same script, in a different crate, for a reason the error message would not
have connected to it.

The tell was that the workspace has **seventeen** `chunks_exact` call sites and
exactly one was reported. The other sixteen are in crates that inherit the MSRV,
where clippy correctly held the lint back.

**And `ci/check/checks.sh`'s MSRV stage was checking them without meaning to.** It runs
`cargo check --workspace --all-targets` on 1.85, so the examples were compiled
against the minimum all along — but by accident of `--workspace` rather than by
a claim in their manifests, and with no lint respecting it.

Fixed by adding `rust-version.workspace = true` to all 56. No code changed; the
lint stops firing because it should never have fired.

## `ttf-parser` is unmaintained, and there are two font parsers in the binary

`cargo deny check advisories` reports RUSTSEC-2026-0192: the author of
`ttf-parser` has said it will not receive further fixes, and the advisory states
plainly that there is no safe upgrade. The named alternative is `skrifa`.

It is a **direct** dependency, not an inherited one: `vieww-text` takes it
unconditionally, and `vieww-paint` takes it behind `native` — the only renderer
this framework has, so on every real build.

**It is also unreachable to remove.** Two paths arrive without anyone here
choosing them:

    ttf-parser <- fontdb           <- cosmic-text <- vieww-text
    ttf-parser <- owned_ttf_parser <- ab_glyph <- sctk-adwaita <- winit

The second is winit's Wayland client-side decorations. A complete port of our
own code would leave both standing and the advisory still firing, which is why
`deny.toml` carries an `ignore` for it rather than a TODO. The entry says to
delete it the day either path goes away.

**Worth doing anyway, for a different reason.** `skrifa` 0.44 is *already* in
the dependency graph — `cosmic-text` and `swash` both use it. So this workspace
currently links two independent font parsers: skrifa, through the text stack,
and ttf-parser, through `vieww-paint`'s glyph outlines. Moving `vieww-paint`
onto skrifa adds no dependency and deletes one parser's worth of code from every
binary.

That is an open question against the size budget recorded in the section above.
The counter example measured 8.9% over a figure taken on another machine, and it
was re-recorded rather than attributed. Two font parsers is the first place to
look if anyone attributes it later: `cargo bloat --release -p vieww --example
counter` will say whether ttf-parser is in the counter's binary at all.

## `rust-version = "1.85"` was a promise nothing could keep

`[workspace.package]` declared 1.85. The workspace has never built on it. The
first time `ci/check/checks.sh`'s MSRV stage actually ran — it had been unreachable
behind earlier stages for months — cargo refused before compiling a line:

    cosmic-text@0.19.0 requires rustc 1.89
    smol_str@0.3.6     requires rustc 1.89
    image@0.25.10      requires rustc 1.88.0
    naga@29.0.4        requires rustc 1.87
    zbus / zvariant / zcheapstr  require rustc 1.87

`cosmic-text` is the text stack: not optional, not a dev-dependency, not behind
a feature. So the figure on crates.io was wrong for every consumer, in the
direction that costs them a build failure rather than a warning.

Now **1.89**, the maximum over the tree. Lowering it later is real work — pinning
`cosmic-text`, `image` and `naga` back to releases predating their bumps — and
worth doing only if someone needs it. Nobody has asked.

**The number now lives in one place.** `ci/check/checks.sh` already read it from
`Cargo.toml`; `.github/workflows/ci.yml` hard-coded `1.85` in two steps and would
have gone on installing a toolchain the workspace no longer claimed. Both steps
now read it with the same `sed`, so the three cannot disagree again.

### The lints the false MSRV was suppressing

`slice::as_chunks` stabilised in 1.88, so at a declared 1.85 clippy held
`chunks_exact_to_as_chunks` back everywhere. At 1.89 it fires — on **seventeen**
`chunks_exact` sites, every one of them a per-pixel loop: `vieww-effects`' blend
and matrix passes, `vieww-paint`'s target readback, `vieww-image`'s atlas and
mipmap chains, `vieww-hal`'s swapchain copy.

It is allowed workspace-wide for now, in `[workspace.lints.clippy]`, with the
reasoning at the site. Briefly: the rewrite is not the substitution the lint
suggests — `as_chunks_mut::<4>()` returns a `(&mut [[u8; 4]], &mut [u8])` pair,
so a zip over two of them changes shape as well as type and the remainder slice
has to be used or explicitly dropped. Seventeen of those at once, in the code
that writes every pixel, on a style lint, is how a blend bug gets in. Do them
deliberately, with the parity and retained-repaint suites as the check.

**`manual_is_multiple_of` is the same story**, from `is_multiple_of`'s
stabilisation in 1.87: **thirty-nine** `% n == 0` sites, from the leap-year rule
in `vieww-foundation` and the blur kernel's odd-size rounding, through
`vieww-paint`'s dash phase, to a long tail of `index % 2 == 0` row striping in
tests and gallery screens. Also allowed, also its own change — and only the
*unsigned* sites can move, because the method does not exist for signed
integers.

**Expect others.** Clippy holds back every lint that suggests an API newer than
the declared minimum, so four editions' worth of them switched on together when
the figure was corrected. The way to find the rest is one local run, not one
push per lint:

    cargo clippy --workspace --all-targets --keep-going \
      --features "vieww-paint/native,vieww-hal/vulkan" 2>&1 \
      | grep -oP 'clippy::\K[a-z_]+' | sort -u

Note the absent `-- -D warnings`: they stay warnings, nothing aborts, and the
whole set arrives at once.

### One warning left standing, on purpose

    warning: output filename collision at target/doc/vieww/index.html
    the bin target `vieww` in `vieww-cli` has the same output filename as
    the lib target `vieww` in `vieww`

`vieww-cli`'s binary is named `vieww`, which is the right name for it to have.
This is cargo#6313 and it affects the generated docs directory only. It is a
cargo warning rather than a rustdoc one, so `RUSTDOCFLAGS="-D warnings"` does
not turn it into an error and the stage is green. Renaming the binary to silence
it would be the tail wagging the dog.

## The gate could go red without anyone touching the repository

`rust-toolchain.toml` said `channel = "stable"`. That is not a version; it is
whatever rustup resolves on the morning the job runs. `ci/check/checks.sh`'s clippy
stage passes `-D warnings`. Together those two lines mean a new clippy release —
every six weeks, on somebody else's schedule — turns unchanged code into a build
failure.

That is not a hypothetical. It is the entire recent history of this gate. Once
the earlier blockers cleared and clippy could finally reach the workspace, it
failed in sequence on `manual_clamp`, `chunks_exact_to_as_chunks`,
`unused_variables`, `unused_imports`, and `manual_is_multiple_of` — **not one of
them from a change to the code they fired on**. Each cost a full CI run to learn
a single line, because `-D warnings` also stops the build at the first finding.

Three separate things made that a loop, and all three are now closed:

1. **The toolchain floated.** `channel` is pinned to `1.98.1`. The lint set is
   now a property of this checkout, not of the calendar. Upgrading is a commit:
   bump the number, run the gate, deal with the new lints in one deliberate
   change.

2. **The gate stopped at the first finding.** `--keep-going` on the clippy,
   build and docs stages. One run now names everything it can, instead of one
   thing per push.

3. **The declared MSRV was false, which silently suppressed lints.** 1.85 was a
   figure nothing could build on; correcting it to 1.89 switched on every lint
   that suggests a 1.86–1.89 API, all at once. See the section above.

**And the gate was being run in the wrong place.** `ci/check/checks.sh` is a shell
script with no CI dependencies — the loop above was expensive only because each
iteration went through GitHub Actions. It costs nothing locally:

    cargo fmt --all && ci/check/checks.sh --quick

Push when that prints `all quick checks passed`. CI is there to catch what a
machine other than yours disagrees about, not to tell you about an unused
import.

## The studio's panic boundary is configured in an untracked file

`viewwstudio`'s `tests/pipeline.rs` aborted CI with

    fatal runtime error: Rust cannot catch foreign exceptions, aborting
    process didn't exit successfully (signal: 6, SIGABRT)

in `a_panicking_build_on_a_later_rebuild_is_caught_by_the_host_not_aborted` —
taking all twenty-one tests in the file down with it, since an abort is not a
test failure.

The mechanism is the one `loaded.rs` documents at length, arriving from the one
direction it does not: **the host half was missing.** Guest and host must share
one `libstd` or a panic raised in a loaded preview is a foreign exception to the
host's unwinder. `apps/viewwstudio/src/compile.rs` passes `-C prefer-dynamic`
for the guest explicitly, in code, and that half works. The host half comes from
`.cargo/config.toml`, and on CI it was not applying.

Two things ruled out first: the workflow sets `rustflags: ""` on
`actions-rust-lang/setup-rust-toolchain`, which that action documents as leaving
`RUSTFLAGS` *unset* precisely so `target.*.rustflags` still applies; and the
guest is compiled with the flag regardless of any config, from `run_rustc`. What
is left is the file itself.

**`.cargo/` is a common `.gitignore` entry.** That is exactly how a file ends up
present on the machine it was written on and absent from every clone — including
the one CI makes. It is worth confirming with `git ls-files .cargo/`; an empty
answer is the whole explanation.

**It is now written and committed**, reconstructed from the scripts and module
docs that describe it, because CI proved the absence rather than suggesting it:
the preflight below tripped on a clean checkout. It carries three things —

  * `-C prefer-dynamic -C rpath` for the Linux-gnu and macOS hosts, `cfg`-scoped
    so it cannot reach an Android or iOS cross-build, where a dynamic `libstd`
    is a link error against a file the NDK does not ship. Windows is absent on
    purpose: MSVC ships no dynamic `std`, which is why `ci/check/platform-check.sh`'s
    Windows branch says the boundary there needs an out-of-process preview.
  * the Android linker shims (`ci/mobile/ndk-clang-{aarch64,x86_64}.sh`) and the `adb`
    test runner, per `ci/mobile/ndk-clang.sh` and `ci/mobile/adb-runner.sh`.
  * the iOS simulator runner, per `ci/mobile/simctl-runner.sh`.

Verified rather than assumed: with the file in place, `cargo build -v` passes
`-C prefer-dynamic -C rpath` to rustc, and `ldd` on the result resolves
`libstd-<hash>.so` through the recorded runpath.

**If an untracked copy exists on a working machine, merge — do not overwrite.**
This reconstruction covers what the checked-in scripts document; a local file
may have accumulated more.

**`ci/check/checks.sh` now checks for it before the test stage.** Not because a check
script should second-guess a checkout, but because of how this failure presents:
a SIGABRT with a message about the unwinder, in the middle of the suite that
exists to prove the unwinder works. It reads as a bug in the panic-catching code
rather than as three missing lines of configuration, and it costs whoever meets
it an afternoon in the wrong file.

**The same rule bites from a second direction.** Cargo takes rustflags from
exactly one source, so *setting `RUSTFLAGS` at all* — even to an empty value —
discards `target.*.rustflags` wholesale and removes this boundary. That is what
`ci/check/coverage.sh` had to work around for `cargo-llvm-cov`, which exports
`RUSTFLAGS` to add its instrumentation, and it is worth knowing before adding a
`RUSTFLAGS` to any shell profile or CI step.

## Documentation links

`cargo doc` with `-D warnings` — the gate that had not run — reported **36**
broken intra-doc links across ten crates. Most were links into the vello-era
`gpu`, `hybrid` and `cpu` modules and into items that moved or became private
with them; the rest were `[`Thing`](Thing)` forms that `rustdoc` now flags as
redundant. All 36 are fixed: a link whose target still exists was repointed, and
one whose target is genuinely gone became plain code formatting rather than a
link that renders on docs.rs as literal brackets.

This is the second time this crate has accumulated a batch of these — the
stage's own comment records the first, at 117 — and both times the cause was the
same, which is that nothing was running the check. That is now true again only
if somebody breaks the script again.

## Rendering performance

The CPU rasterizer is the renderer for everything, and it was costing about
100 ms for a full studio frame. Measured against the pristine archive on the
same machine, interleaved runs, minimum of four, over `examples/fixtures`:

| fixture | before | after | |
|---|---|---|---|
| `22-editor` (an IDE shell) | 20.41 ms | 11.51 ms | 1.77x |
| `11-typography` | 7.95 ms | 4.10 ms | 1.94x |
| `11-code-block` | 6.72 ms | 3.85 ms | 1.75x |
| `21-dashboard` | 10.41 ms | 6.29 ms | 1.66x |
| `20-settings` | 12.33 ms | 7.77 ms | 1.59x |
| `23-editor-glass` | 43.53 ms | 29.26 ms | 1.49x |
| **whole gallery** | **304.69 ms** | **238.86 ms** | **1.28x** |

`00-strokes` and `00-curves` measure 0.97x and 0.99x — untouched code paths
coming out flat is what says the rest of the table is the change and not the
machine.

> **This table is a before/after of a renderer change, and three of its
> fixtures have since been edited.** `20-settings` lost two rows, and the
> interaction screens lost rows too — see the layout-overflow entry at the top
> of this file, which is why. A fixture that draws less is faster for a reason
> that has nothing to do with the rasterizer, so the "after" column above is
> **not** comparable with what the gallery prints today and must not be
> updated in place to look like it is; the pairs in it were measured against
> each other, on one machine, on identical content.
>
> The current absolute numbers, from a run after those edits, are in the next
> section. Anyone repeating this experiment should re-measure both columns.

For a noise-free figure on the studio's own frame, `callgrind` over
`apps/viewwstudio/examples/frame_cost` (605 commands, 323 shapes, 1,447 glyphs,
13 shadows, 3 layers at 1440x900): **7,000,560,220 instructions before,
4,035,509,557 after — 1.73x fewer**, and 62.9 ms to 48.9 ms of wall clock for a
full repaint of the whole window. Wall clock improves by less than that,
because what is left is increasingly memory-bound rather than instruction-bound:
after the change the frame's largest remaining costs are `paint_shape` (35%),
`composite_layer` (11%), `Premul::to_straight_u8` (10%) and clearing buffers
(5%), and the last two are proportional to the *window*, not to the scene. Both
of those also mostly disappear on a damaged repaint, which converts and clears
only the rows it redrew — the live window's path, and not what this benchmark
measures.

What changed, in order of what it was worth:

- **`[profile.dev]` did not optimise the rasterizer.** `[profile.dev.package."*"]`
  covers non-workspace dependencies; the rule was written when vello and wgpu
  did the pixel work, and `vieww-paint` is a workspace member. Every debug build
  since the migration ran the framework's hottest loop at `opt-level = 0`.
  Measured on `vieww-paint`'s own `perf_probe`: **54.5 ms → 7.3 ms, 7.5x.** This
  is what a debug `viewwstudio` was actually suffering from, and it presented as
  a studio that appears to hang on a click, an animation that plays two of its
  frames, and a splash screen that never fades.
- **A rasterised-glyph cache** (`native/glyph_raster.rs`). A studio frame
  contains 1,447 glyphs and every one of them was re-outlined, re-placed,
  re-flattened and re-scanline-rasterised every frame. Keyed on exact bits
  rather than a quantised phase; see the module doc for the 57-pixels-by-1/255
  difference this does introduce and why it is written down rather than rounded
  to "none".
- **Gradients resolved once per shape** rather than per pixel — 8.7% of a whole
  frame, by callgrind, on a mostly flat interface.
- **A closed-form path for axis-aligned rectangles**, reproducing the scanline
  pass's own vertical quantisation exactly so a rectangle is the same bytes
  either way, and asserted as such over a sweep of sub-pixel offsets.
- **Layers composite only what they inked**, not their declared bounds.
- **Blurs run over the inked band** and the vertical pass carries a strip of
  columns. See `native/effects.rs`'s `vertical` for the measurement — and for
  the warning attached to it, which is that a single before/after sample on this
  machine gave the opposite answer and was nearly acted on.
- **The blend mode is decided once per draw, not once per pixel.** Every
  composite loop already computed a `normal` flag from the mode and the colour
  pipeline, and then went on to call `blend_with_pipeline` — which matches on
  the pipeline, then on twenty-eight blend modes — for each pixel anyway. A
  callgrind profile of `23-editor-glass` put **22% of the whole frame** in that
  dispatch, for arithmetic that is four multiplies and four adds. See
  `native/color.rs`'s `over`.
- **The gradient's device mapping is folded into its ramp.** The inverse
  transform, the shape's bounds, the two divisions and the geometry `match` were
  all per pixel and all constant per shape; for a linear gradient the whole
  composition is affine, so it collapses to one multiply and one add with the
  row's term hoisted. See `native/gradient.rs`'s `DeviceRamp`.
- **`-C target-cpu=x86-64-v2`.** The default `x86-64` baseline is SSE2 and has
  no `roundss`, so `f32::floor` and `f32::ceil` were *calls into
  `compiler-builtins`*. Worth **27.4 ms to 22.7 ms** on `23-editor-glass` by
  itself. See `.cargo/config.toml` for why v2 and not v3.

### The two fixtures still over budget, and what that does and does not mean

`00-layers-nested` (24 isolated layers, a deliberate compositor-depth stress)
and `23-editor-glass` (the IDE shell plus a blurred palette, a full-surface
scrim and real elevation) render a **full** frame in about 25 ms and 22 ms
against a 16.7 ms budget. Both were about 45 ms.

Those two are the only ones left: the most recent full run reports **23
fixtures, 208 ms of rendering in total**, and every other fixture inside the
budget. (`20-settings` reads 7.2 ms rather than the 7.77 ms in the table above
because it now draws seven rows instead of nine — see the note there.)

The gallery measures full repaints, and that is the right alarm for a first
frame, a resize and a theme change — all of which genuinely redraw everything.
It is not what a live window does when something changes, and for as long as it
was the only number it stood in for a claim nobody had checked: that these
screens cannot hold 60 Hz.

So `examples/fixtures`' interaction suite now measures the glass screen too —
the same screen, one signal-driven control over it, changed:

```
interaction                       damage  full frame    retained  speed-up  identical?
glass: highlight moves             0.72%     21.25ms      9.85ms      2.2x  yes  within 60Hz
glass: highlight clears            0.72%     21.88ms      9.99ms      2.2x  yes  within 60Hz
```

(Re-measured after the layout-overflow fixes. The full-frame column moved
because the screen behind the palette changed — `page_behind`'s spacing and the
list's row count — not because the renderer did. The *claim* is unchanged and
is the one asserted on every run: byte-identical, and inside the budget.)

Byte-identical to the full repaint, and inside the frame budget. That is the
measurement that was missing, and it is asserted on every run rather than
described here — a retained repaint of a screen with a blurred backdrop, real
shadows and a clip stack is exactly where a region repaint goes subtly wrong.

**What would take the full-repaint numbers further** is explicit SIMD in the
compositing loops, or a GPU. The scalar loops are close to their limit: the
frame is roughly 200 million instructions for about four million blended
pixels, and the remaining costs are spread across compositing (22%), the
output conversion (16%), layer resolve (8%) and blur (7.5%) rather than
concentrated anywhere a single change would reach.

### Four optimisations that were tried, measured, and removed

Written down because each is the obvious next idea, and because the pattern in
them is the point.

- **A reduced-resolution blur.** Downsample, blur small, bilinear upsample —
  what Core Animation, Skia and Chrome all do. Implemented in full, including
  the variance-corrected sigma. Blur turned out to be 7.5% of the frame it was
  supposed to rescue; end to end it moved `23-editor-glass` from 32.21 ms to
  32.46 ms, and its upsample's `f32::floor` calls put 2.9% of the frame into
  `compiler-builtins`. Removed; see `native/effects.rs`.
- **An opaque-row `copy_from_slice` in `composite_layer`**, with a matching
  full-coverage skip in `composite_flat`. Made `23-editor-glass` 1.7% *worse* —
  the alpha scan is a second pass over the row and the blend it replaces is
  already a few instructions. Removed; see `native/target.rs`.
- **`as_chunks_mut::<4>()` instead of `chunks_exact_mut(4)` and
  `copy_from_slice`** in the output conversion, to drop a length check per
  pixel. 1,203,539,448 instructions against 1,203,539,565 — a difference of 117
  across a whole render. LLVM had already done it. Reverted.
- **A strip-wise vertical blur pass**, which *was* kept — but only after a
  second look. One run of each said it was slower and it was reverted on that
  basis; six runs of each said it was 26% faster. See `native/effects.rs`'s
  `vertical`, and `examples/blur_bench.rs`, which exists so the next person
  does not repeat it.

The common thread: on this machine the run-to-run spread on the same binary is
wider than most of these effects. A single before-and-after is not evidence, and
`callgrind`'s instruction count — deterministic, and the only reason the
`copy_from_slice` and blur results were unambiguous — is not the whole story
either. `00-fills-rrect-clip` regressed 20% in wall clock while executing **15%
fewer instructions**, because building the coverage mask with
`Vec::with_capacity` and pushing writes zeros that `vec![0.0; n]` gets from the
allocator for free. Both numbers, every time.

## Baked-in choices removed

A framework is generic to the extent that an application can replace what it
does. Four places decided something on the application's behalf that the
application had every right to decide, and in each case the mechanism for
deciding it already existed and was simply not reachable.

**Text layout constructed its own shaper.** `vieww-text`'s crate doc calls
`shaping`/`layout` "a shaper-agnostic interface" and `layout.rs`'s own header
says the bidi step "is what makes Arabic work, and it cannot be skipped".
`Paragraph::layout` built a `NaiveShaper` — one glyph per code point, monotonic
advance — inside itself, so no caller could pass a real one and
`HarfBuzzShaper` was unreachable from the layout pass however it was
configured. A trait with one hard-coded implementation is not an abstraction.
Now `Paragraph::layout_with(shaper, fonts, …)` takes it, and `layout` is that
call with the naive default named at the call site.

Two smaller versions of the same thing went with it, both silent:

- Every run was shaped as `FontHandle(0)` regardless of the span's
  `font_family`, while `FontCache::resolve` had existed the whole time to
  answer exactly that question.
- Every run was shaped with `&[]` for its OpenType features regardless of the
  span's `features`, so a span asking for `"liga" off` was given ligatures
  anyway.

Four new tests cover these, each of which could not previously have failed
*or* passed: the supplied shaper is the one that shapes, a span's features
reach it, a span's family is resolved through the cache, and the size an
unsized span gets comes from `LayoutOptions` rather than from `unwrap_or(16.0)`
in the middle of the pass. `1.2` and `0.8` in the line-box arithmetic are now
`NORMAL_LINE_HEIGHT` and `BASELINE_FRACTION`, the second of which says out loud
that it is an approximation standing in for font metrics the `Shaper` trait
does not expose.

`Paragraph` also now stores its `LayoutOptions` whole rather than three of the
five fields, because `relayout` had to invent the rest — it passed
`line_height: None`, so re-laying a paragraph out at a new width silently reset
its line spacing.

**The window's rasterizer was fixed at construction.**
`vieww-platform-winit`'s `NativeRenderer::for_window` built
`CpuRenderer::new()`, and that was the only rasterizer a windowed application
could have. `vieww-paint` offers `with_color_pipeline` — the linear-light path
documented at length in `native/linear.rs` as the colorimetrically correct way
to blend a gradient or a translucent overlay — and
`with_glyph_outline_budget_bytes` for a memory-constrained target. Neither
could be reached from a real window: configurable in a unit test, fixed in
every application, which is worse than not offering them.
`for_window_with(…, cpu)` takes the rasterizer; `for_window` is the short form
with the default.

**Two controls drew their own palette.** The carousel's page indicators were
`rgb(58, 122, 246)` and `rgb(203, 213, 225)`, and the colour picker's labels,
track and hex readout were five more hex literals — so the one control on the
screen whose entire subject is colour was the one that ignored the theme, and
on a dark scheme drew near-black text on a dark ground. Both now read
`ColorScheme` roles (`primary`, `outline`, `on_surface`, `on_surface_variant`,
`surface_variant`). The picker's *preset swatches* are deliberately still the
caller's own colours: those are its content, not its chrome.

### What was looked at and deliberately left

- **`ModalBarrier`'s scrim** and the colour picker's **default presets** are
  documented defaults with builder overrides (`.color(…)`, `.presets(…)`). A
  default a caller can replace in one call is not a baked-in choice.
- **The switch's thumb shadow** is `rgba(0, 0, 0, 0x38)` and stays that way. It
  is the only baked shadow colour left in the framework, occlusion is
  achromatic rather than a brand decision, and `ColorScheme` says of itself
  that it is "trimmed to what the controls here actually use — a role nothing
  reads is a role nobody can be sure is right". Adding an elevation role for a
  single reader would be the same mistake in the other direction. If a second
  control ever needs one, that is when it earns its place.
- **`cfg(target_os = …)`** appears 65 times and is not hard-coding: it is how a
  cross-platform crate says which platform a path belongs to.

## The preview and the built application disagreed

A scaffolded project ran with a **dark window** and **light-themed widgets**.
The heading and the body text were drawn in near-black ink on a near-black
ground — present, laid out, and invisible — while the filled button drew its own
accent colour and was the only thing on the screen.

Two settings and nothing tying them together:

* `App::background(ThemeData::dark().colors.surface)` painted the window.
* The widget tree got whatever `Theme` the application mounted, and the
  template mounted none, so `ThemeData::of` returned its
  `ThemeData::light()` fallback.

The template's own comment claimed the invariant it did not have — "the theme's
own surface rather than a colour chosen here, so the window's background cannot
drift from what the widgets draw on" — beside the line that let it drift.

**It survived review because vieww Studio's preview was right.** The preview
wraps the screen in `Theme::new(ThemeData::adaptive(platform, dark))` of its own,
so the studio showed the screen correctly the whole time and only the built
application was wrong. A preview that disagrees with the build is worse than no
preview: it is the one failure the feature exists to prevent, and it makes every
other thing the preview says untrustworthy.

Fixed at the framework rather than in the template, because a template fix
leaves the trap set for everyone who writes an `App` by hand:

* `FrameDriver::set_theme` publishes a theme above the root, the same way the
  view metrics and the accessibility preferences are already published — so an
  application does not have to remember to wrap its own tree. A `Theme` *inside*
  the tree still wins, so a dark section of a light screen is unaffected.
* `App::theme(data)` sets the window background **and** calls that, from one
  value. `App::background` remains, and its doc now says what it is for and
  what it cannot do alone.
* Both project templates use `App::theme(theme())`, where `theme()` is one
  function returning `ThemeData::light()` — light because that is what the
  studio previews with by default, so the screen you Render and the screen you
  build are the same screen. Changing that one line moves the window and every
  widget together.

Two tests in `vieww-render`: a widget that mounts no theme of its own reads the
application's, and a `Theme` inside the tree still overrides it.

## Say was a language you could preview but not ship

`vieww-say-codegen` had exactly one caller in the whole workspace: vieww Studio's
preview. Nothing in a *user's* build ever touched it.

So a Say project built cleanly and produced an application that did not contain
its own screen. The scaffolded `lib.rs` mounted a placeholder — literally
`Text::new("Open src/screens/home.say and press Render")` — and that is what the
finished, runnable, shipped application said when you ran it. The screen the
author wrote existed only inside the studio.

A Say project now scaffolds with a `build.rs` that compiles every
`src/screens/*.say` through the same generator the preview uses, into Cargo's
`OUT_DIR`; `src/screens/mod.rs` includes the result, so `home.say` becomes
`screens::home` and `mount` mounts `screens::home::screen()`. Adding a screen
means adding a `.say` file — there is no list to keep in step, which is the
point of generating it. A syntax error fails the build with file, line and
column, from the same diagnostics the Problems panel renders.

`a_scaffolded_say_project_builds_and_contains_its_screen` is the test, and it
does not stop at "it built": it greps the binary for `Tapped`, a string that
exists only in `home.say`, and asserts `press Render` is *absent*. The old
scaffold built perfectly and shipped the wrong screen, so compiling was never
the property worth asserting.

Generated Say code also emitted `warning: method 'mark' is never used` into a
file under `target/` that the author did not write and cannot edit. A warning
nobody can act on is worse than none — it teaches the person that this build is
expected to be noisy, and the next warning, one that *is* theirs, arrives into
a build they have stopped reading. The generator now scopes an `allow` to the
fixed scaffold it emits, and leaves every other lint in the file live.

## A hello-world weighed 1.3 GB, and 90% of the binary was other people's symbols

Measured on a freshly scaffolded project, `cargo build`, debug:

| artifact | before | after |
|---|---|---|
| `libapp.a` (iOS staticlib) | 813.5 MB | not built |
| `libapp.so` (Android cdylib) | 228.4 MB | not built |
| `libapp.rlib` | 41.7 MB | 7.0 MB |
| the binary | 260.5 MB | **40.6 MB** |
| the binary, `--release` | — | **7.6 MB** |

Three independent causes, and only the first is obvious once seen:

- **A desktop build was building the Android and iOS libraries.** The scaffold
  declares `crate-type = ["rlib", "cdylib", "staticlib"]` and all three are
  genuinely needed — the rlib for the desktop binary, the cdylib because an
  Android activity loads a shared library, the staticlib because
  `UIApplicationMain` links one. None of them is needed *on a desktop build*,
  and a plain `cargo build` builds every declared crate type. `Builds::spec` now
  passes `--bin <name>` for the desktop targets, which stops cargo at the rlib
  the binary links; the mobile targets keep the plain `build`.
- **Full debug info for ~380 dependencies.** `.text` in that 260 MB binary was
  17.4 MB; `.debug_info` and `.debug_str` alone were 186 MB. The scaffold now
  sets `debug = 1` for the author's crate — a panic still names their file and
  line — and `debug = false` for `[profile.dev.package."*"]`.
- **No release profile at all.** The scaffold had none, so a shipped build got
  cargo's defaults. It now sets `strip`, `lto = "thin"`,
  `codegen-units = 1` and `panic = "abort"`, each with the reason written beside
  it.

`binary_name` reads the `[[bin]]` name out of the manifest rather than guessing
it from the folder, because a project can be renamed, moved, or written by hand,
and a build that guesses wrong fails with `no bin target named …`.

## What genuinely remains

> **Superseded by [`PENDING.md`](./PENDING.md)**, which is complete where this
> section is partial — it covers the platform, text, tooling and product gaps
> this section never listed, and gives each one a starting point. What follows
> is kept because its *reasoning* is referenced from elsewhere in this file;
> where the two disagree, `PENDING.md` is current.

- **GPU coverage, not GPU existence.** This section once said no GPU code had
  ever run. Then it said the GPU could draw shapes but not text. Both are now
  out of date: `lavapipe` is installed, `vieww_hal::vulkan::SceneRenderer`
  executes a whole batched scene on it, and **text is drawn and verified** —
  see the top of this file, and `PENDING.md` for the full remaining list.

  What `vieww_gpu::ScenePlan` still cannot express is **images, shadows,
  gradients, shaped clips and layers**. The practical consequence has narrowed
  rather than gone away: a screen of text and flat panels now plans complete,
  and any screen with a photograph, an elevation shadow, a gradient or a
  rounded clip does not — which is still most real screens, and
  `ScenePlan::is_complete` still sends every one of them to the CPU rasterizer.

  A second gap sits behind the first: `lavapipe` is a *software* Vulkan
  implementation, so what has been verified is correctness, not performance.
  **No number in this file was measured on real graphics hardware.**

  The two fixtures still over a 60 Hz full-repaint budget are unchanged by
  any of this: they have blurs, shadows and shaped clips on them, so
  `is_complete` refuses them and they stay on the CPU path whatever the GPU
  can do with their text. Explicit SIMD in the composite loops remains the other
  real option and remains undone.
- **Non-Linux packaging.** `vieww-build`'s macOS/Windows/Android packaging
  paths are real code, unit-tested for their logic (argument construction,
  tool detection, error naming), but have not been exercised by an actual
  build on those platforms' own toolchains.
- ~~**`docs/architecture/*.md`** — not written.~~ **Done.** See
  [`docs/architecture/README.md`](./docs/architecture/README.md) and its four
  companion files: the UI tree (widget/element/render), the rendering
  pipeline (scene/render-graph/render-planner/CPU-GPU-Hybrid), the
  cross-cutting crates, and tooling/distribution. Written from the crates as
  they actually exist, citing each crate's own module doc rather than
  restating it.
- ~~**A full stale-doc sweep**~~ — **partly done, and the mechanical half is
  finished.** `cargo doc` under `-D warnings` now passes across the workspace,
  which is 36 broken intra-doc links fixed (see above) and is the part a
  machine can check. Two specific stale *claims* were also found and corrected
  by reading them against the code: the `rustybuzz` note below, and
  `ci/check/checks.sh`'s comment about backends that no longer exist. A prose comment
  can still be out of date without any tool noticing, so this is "no known
  stale docs" rather than "proven none".
- **Naming note on item 7 ("rustybuzz"), corrected.** The previous wording
  here said the shaping stack was "not literally the `rustybuzz` crate". That
  is half right and it hid the more interesting half, so, from the lockfile
  and the call sites rather than from memory:

  - **What actually shapes the text a widget draws** is `cosmic-text 0.19`,
    reached through `vieww_text::Paragraph` (`src/paragraph.rs`) — and
    cosmic-text shapes with **`harfrust`**, a HarfBuzz port built on
    `read-fonts`/`skrifa`. So the functional requirement is met, by a
    HarfBuzz-lineage shaper, and the named crate is indeed not the one doing
    it.
  - **`rustybuzz` is nonetheless a real direct dependency** of `vieww-text`,
    optional behind the `harfbuzz` feature, and `shaping::HarfBuzzShaper`
    genuinely wraps it and is genuinely tested. Nothing in this workspace
    enables that feature, so in a default build it is not compiled — which is
    what an optional dependency is for, and not a gap.
  - **`shaping`, `layout`, `selection`, `editing` and `bidi` are a second,
    in-progress stack** and `vieww-text`'s own crate doc says so in as many
    words. `layout::Paragraph` hard-codes `NaiveShaper` — one glyph per code
    point — and is reached only through its module; it is not what any widget
    uses. It is worth knowing that before reading `NaiveShaper` and concluding
    that the framework has no shaping, which is a conclusion this file's
    previous wording made easy to reach.
- **Everything else this tracker lists** is real, has tests that ran and
  passed on this machine, and has been clippy-clean under
  `-D warnings` at the point it was last touched.