#!/usr/bin/env bash
#
# Run the studio's headless visual walkthroughs on this machine and build one
# browsable report out of them.
#
#     ci/certify/release-check.sh                    # everything, out/release-check/<platform>-<date>/
#     ci/certify/release-check.sh out/my-run          # a chosen output directory
#     ci/certify/release-check.sh --quick             # release_check only, one theme, one size
#
# Run this once on each of Linux, Windows and macOS before a beta goes out.
# Each run writes a self-contained folder with every screenshot and an
# `index.html` gallery; there is nothing to install and nothing to compare
# against a server — copy the three folders anywhere and open the three
# `index.html` files side by side.
#
# # What this runs, and why three scripts and not one
#
# `apps/viewwstudio/examples/`:
#   - `release_check` — every command the palette exposes, fired for real,
#     plus the interactions the other two never touch (right-click, hover,
#     drag-select, typed find/replace). This is the new one, and the one
#     closest to "does a release-ready build survive a person clicking
#     everything".
#   - `tour` — every *state* the shell can draw, screenshotted directly.
#     Cheap, and it catches "this screen doesn't render" even for states a
#     person reaches ten menus deep.
#   - `walkthrough` — one held-together twenty-minute session. Catches state
#     that leaks between steps, which independent screenshots cannot see.
#
# None of them touch a display or a GPU — see `vieww_paint::native`'s module
# doc — so this produces the same pixels on a laptop, a CI runner and a
# machine with no monitor plugged in at all. It is not a substitute for
# running the actual built executable by hand once; it is what makes that
# final pass fast, because everything mechanical has already been checked.
#
# # What "release ready" means from this script's output
#
#   1. It exited 0. `release_check` exits 1 if the renderer itself reported a
#      frame error (a caught panic in layout or paint — see `FrameDriver`'s
#      module doc) on any step; that is a real bug, not a matter of taste.
#   2. `manifest.json`'s "warnings" array is empty, or every warning in it is
#      understood. A warning means a label this script went looking for was
#      not found — either the script's guess at the label is stale, or the
#      button/menu/tab it describes is genuinely gone or renamed, which is
#      itself worth knowing before a release.
#   3. A person has skimmed `index.html` on each platform and the pictures
#      look like the product: no missing glyphs, no clipped panels, no
#      mismatched light/dark colours, no visibly wrong layout at the window
#      sizes tested. This is the step nothing here can automate — the
#      rasteriser is pixel-correct by construction (`docs/architecture`), but
#      "correct" and "looks right to a person" are different questions.
#
# Establishing a baseline: the first clean run on each platform is worth
# keeping. `crates/vieww-test-harness/src/visual.rs` already has the
# machinery for exact pixel diffing (`Frame::differences`) if this ever grows
# into a golden-image gate; today it is a report for a person, on purpose,
# because the rendering is still moving fast enough that a strict pixel gate
# would be red more often than it was useful.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

quick=0
out=""
for arg in "$@"; do
    case "$arg" in
        --quick) quick=1 ;;
        -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) out="$arg" ;;
    esac
done

platform="linux"
case "$(uname -s 2>/dev/null || echo unknown)" in
    Darwin) platform="macos" ;;
    Linux) platform="linux" ;;
    MINGW*|MSYS*|CYGWIN*) platform="windows" ;;
esac
stamp="$(date +%Y%m%d-%H%M%S 2>/dev/null || echo run)"
out="${out:-out/release-check/${platform}-${stamp}}"
mkdir -p "$out"

echo "release-check: platform=$platform  out=$out"
{
    echo "platform: $platform"
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || echo unknown)"
    echo "uname: $(uname -a 2>/dev/null || echo unknown)"
    command -v rustc >/dev/null 2>&1 && echo "rustc: $(rustc --version)"
    command -v cargo >/dev/null 2>&1 && echo "cargo: $(cargo --version)"
    echo "commit: $(git rev-parse HEAD 2>/dev/null || echo unknown)"
} > "$out/environment.txt"

echo "release-check: building release binaries once (used by every run below)…"
cargo build --release -p viewwstudio --example release_check --example tour --example walkthrough

runs=()

run_variant() {
    local example="$1"; shift
    local name="$1"; shift
    local dir="$out/$name"
    mkdir -p "$dir"
    echo "release-check: $example -> $name"
    if ! cargo run --release -q -p viewwstudio --example "$example" -- "$dir" "$@" 2>"$dir/stderr.log"; then
        echo "release-check: $example ($name) exited non-zero — see $dir/stderr.log" >&2
        echo "FAILED" > "$dir/status.txt"
    else
        echo "OK" > "$dir/status.txt"
    fi
    runs+=("$name")
}

run_variant release_check "release_check-dark-normal"
if [ "$quick" -eq 0 ]; then
    run_variant release_check "release_check-light-normal" --light
    run_variant release_check "release_check-dark-small" --small
    run_variant tour "tour-dark-normal"
    run_variant tour "tour-light-normal" --light
    run_variant walkthrough "walkthrough-dark-normal"
    run_variant walkthrough "walkthrough-light-normal" --light
fi

echo "release-check: building $out/index.html…"
if command -v python3 >/dev/null 2>&1; then
    python3 "$root/ci/certify/release-check-report.py" "$out" "${runs[@]}"
else
    echo "release-check: python3 not found — skipping index.html; the PNGs and manifest.json under $out are still there." >&2
fi

status=0
for name in "${runs[@]}"; do
    if [ -f "$out/$name/status.txt" ] && grep -q FAILED "$out/$name/status.txt"; then
        status=1
    fi
done

echo
if [ "$status" -eq 0 ]; then
    echo "release-check: PASS — open $out/index.html"
else
    echo "release-check: FAIL — one or more runs exited non-zero, see the logs under $out" >&2
fi
exit "$status"
