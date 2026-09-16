# Release check: the visual walkthrough

This is the script to run on Linux, Windows and macOS before a beta ships. It
drives the studio the way a person actually would — real clicks, drags,
hovers, right-clicks and typed text, aimed at whatever it finds by its
accessible label rather than a hardcoded pixel — and photographs every step.

## Run it

```console
git pull                       # or check out the tag being tested
ci/certify/release-check.sh            # full run: every theme, both window sizes
ci/certify/release-check.sh --quick    # release_check only, one theme, one size
```

On Windows, run it from Git Bash (the shell `ci/*.sh` has always assumed).
On each machine this writes `out/release-check/<platform>-<timestamp>/index.html`
— open it in a browser. Nothing else needs installing; a Python 3 on `PATH`
is used to build that page if present and skipped (with the raw PNGs and
`manifest.json` still there to read) if not.

Do this on all three platforms before a release. The pixels are produced by
vieww's own CPU rasterizer, not by asking the OS to draw anything (see
`vieww_paint::native`'s module doc), so the *same* scene renders identical
bytes everywhere — a difference between the three galleries is either a real
platform difference (font fallback, a keyboard shortcut's label, a path
separator visible somewhere it shouldn't be) or a bug specific to how that
platform's runner behaves, not rendering noise. That is what makes three
screenshots worth taking instead of one.

## What's actually running

`ci/certify/release-check.sh` runs three headless examples from `apps/viewwstudio/examples/`,
none of which open a window or touch a GPU:

| Script | What it covers | What it can't tell you |
|---|---|---|
| `release_check` | Every command in the palette (`Command::ALL`), fired for real. Every sidebar view, panel tab, preview platform and inspector tab. Every menu, opened by clicking the title bar. A real session on top: open a project, type text through the same IME-commit path a keystroke uses, drag-select it, cut/copy/paste, find & replace by typing into the boxes, drag every resizable seam, right-click a file for its context menu, hover tabs and icons for their tooltips, the two destructive dialogs (cancelled), the unsaved-changes-at-quit prompt. | Nothing it can't find by label — a relabelled or newly hidden control shows up as a warning, not silently. |
| `tour` | Every *state* the shell can draw, by setting the signal directly. Cheap and exhaustive — every one of the ~50 named scenarios is a picture. | Whether a person can actually reach that state by clicking — it doesn't click anything. |
| `walkthrough` | One twenty-minute session, forward only, nothing reset between steps. Built to catch state that leaks: a badge that never clears, a panel that doesn't come back. | Breadth — it's one path through the app, not every command. |

`release_check` is the one that answers "is this release-ready" most
directly, because it is the only one of the three that interacts the way a
beta user will. Run all three; `release_check`'s warnings are the first
thing to read.

## Reading the report

Each variant section in `index.html` shows a pass/fail badge, a warning
count, and every screenshot grouped by the phase of the run that took it
(`commands`, `views`, `menus`, `session`, …). A screenshot with a red border
had zero shapes drawn (almost certainly a blank frame) or a frame error on
that step.

- **Any frame error is a real bug**, not a judgement call — it means a caught
  panic during layout or paint (see `FrameDriver`'s doc). `release_check`
  exits non-zero when this happens, and `ci/certify/release-check.sh` reports FAIL.
- **Warnings** ("no element labelled like … was found") mean the script went
  looking for something by its visible text and didn't find it. Sometimes
  that's the script's own guess going stale (a button's copy changed);
  sometimes it's the actual regression — the button is gone, or covered, or
  never rendered. Either way, look at the screenshot immediately before it.
- **Everything else is a person's judgement**, and there is no way around
  actually looking: does the layout hold at both window sizes, does light and
  dark both look intentional, is there a glyph missing, does a panel clip its
  own content. This is exactly what three still-mostly-empty projects'
  screenshots (`screens/*.rs`) exist to exercise consistently across runs.

## The other two scripts to run on all three

`release-check` drives the studio's UI and photographs it. That leaves two
questions it does not ask, and each has a script of its own. Run all three on
each platform; they are independent and can run in any order.

### `ci/certify/desktop-suite.sh` — does the platform layer behave here

The desktop sibling of `ci/mobile/device-suite.sh`, and it works the same way: the
application asserts what must hold on any desktop and reports on
`__VIEWW_SUITE__` lines, and the script asserts what only the operator knows.

```console
ci/certify/desktop-suite.sh                      # build, run, verify
ci/certify/desktop-suite.sh --expect-hidpi       # on the machine that has such a display
ci/certify/desktop-suite.sh --expect-multi-monitor
ci/certify/desktop-suite.sh --headless           # a Linux CI box, under Xvfb
```

It opens a real window and runs the same demo tree the Android and iOS entry
points run, so a report from a Mac and a report from a phone carry the same
check names against the same widgets. What it adds on top of the shared checks
is everything a phone would have to answer `Unsupported` to: a second window
and a dialog that closes with its parent, the shortcut modifier (Command on
macOS, Control elsewhere), the pasteboard round-trip, and whether `data_dir`
returned this platform's conventional location rather than the no-home
fallback that silently loses settings.

**It needs a Vulkan loader with a driver**, which surprises people: the
rasterising is vieww's own and runs on the CPU, but presentation goes through
`vieww_hal::vulkan`, so a machine with no ICD fails at startup before any check
runs. On a headless Linux box `mesa-vulkan-drivers` (lavapipe) is enough.

### `ci/certify/shot-suite.sh` — do all three platforms draw the same pixels

The claim at the top of this file — that the same scene renders identical bytes
everywhere, because the pixels are vieww's own rasterizer's and not the OS's —
was true and unchecked. This is the check.

```console
# on each machine
ci/certify/shot-suite.sh

# then, with two capture directories in reach
cargo run -p shot-diff -- out/shots/linux-x86_64-… out/shots/macos-aarch64-…
# or, in one step, on the second machine
ci/certify/shot-suite.sh --against /path/to/the/first/capture
```

It photographs all 56 `examples/features/*` headlessly — no window, no GPU, no
Vulkan — and writes a manifest recording what the machine was, including how
many fonts are on each system font path. `shot-diff` then compares two captures
and writes `index.html`: per shot a verdict, and for anything that differs, how
many pixels, the worst per-channel delta, the bounding box, and a diff image
with the differences painted magenta over a washed-out baseline.

The shape of a difference is what names its cause — a delta of 1 along glyph
rims is font fallback, a delta of 255 is a font that is absent, one rectangular
region is a layout difference, and the whole image differing by a constant is a
colour-pipeline difference. `shot-diff --help` has the table.

## Before this replaces a human running the real executable

These are fast, thorough mechanical passes, not a substitute for launching the
actual packaged build once on each platform by hand. `desktop-suite` now covers
a real window, real DPI scaling and real window management, and `shot-suite`
covers font substitution across platforms — which is most of what used to be
listed here as uncheckable — but what remains is judgement: does the layout hold,
does light and dark both look intentional, does a panel clip its own content.
`TRACKER.md` is explicit about the other boundary, what is verified on this
machine vs. what needs a GPU-backed one. Treat a clean run of all three as "safe
to spend the ten minutes doing that by hand", not as a replacement for it.

## Building the executables

`packaging/package.sh` builds the installable tree/installer for whichever
platform it runs on (`--installers` for the native package — `.deb`/`.AppImage`
on Linux, `.dmg` on macOS, `.msi` on Windows — plain archive otherwise). See
that script's own header comment for what each format needs installed on the
machine that builds it, and what it deliberately does not do (no code signing
— there's no certificate to put in this repository).

`.github/workflows/release.yml` runs `ci/certify/release-check.sh --quick` and then
`packaging/package.sh --installers` on all three OSes in GitHub Actions,
uploading both as artifacts on every run and, on a pushed `vX.Y.Z` tag,
publishing them to a GitHub Release. Trigger it manually from the Actions tab
to try a build before tagging, or push a tag to cut a release.
