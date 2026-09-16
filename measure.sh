#!/usr/bin/env bash
# Launch-film measurements, on a machine that is actually yours.
#
#   ./measure.sh 2>&1 | tee measurements.txt
#
# Run it from the repository root. Everything below was already run in a
# two-core cloud container with no GPU; the point of running it again is that
# your numbers are the ones going in the film, and mine are conservative.
#
# ABOUT THE GPU. You offered a GPU measurement and the honest answer is that
# there is nothing to measure there yet: this tree has no `vieww_paint::gpu`
# module. `vieww-platform-winit` rasterises with the CPU renderer and uses
# vieww-hal only to *present* the finished buffer. Several comments refer to a
# `GpuRenderer` that does not exist. So step 2 below checks which adapter your
# Vulkan present path picks — worth knowing, and it catches the case where a
# software ICD enumerates first — but it is not a rasterisation benchmark, and
# no number from this script is a GPU number.

set -u

hr() { printf '\n\033[1m── %s ─────────────────────────────────────\033[0m\n' "$1"; }

hr "0 · toolchain"
# Unlike my run, yours should resolve the pinned 1.98.1. If this prints
# something else, everything below is measured against a different compiler
# than the one you ship, which is worth knowing before you quote it.
rustc --version
cargo --version
echo "pinned by rust-toolchain.toml: $(grep -m1 '^channel' rust-toolchain.toml)"

hr "1 · build"
cargo build --release \
  -p viewwstudio --bin viewwstudio \
  --example bench --example coldstart --example raster_real || exit 1
cargo build --release -p vieww-paint --features native --example edge_probe || exit 1

hr "2 · which adapter presents (NOT a raster benchmark)"
# Exits non-zero if there is no working device, and shouts if it picked a
# software rasteriser — in which case ignore whatever it says about "GPU".
cargo run --release -p vieww-hal --features vulkan --example adapter \
  || echo "  (no Vulkan device, or the feature is unavailable — this does not affect anything below)"

hr "3 · frame cost"
# The keynote numbers: idle, and a keystroke broken into its four phases.
# --only=typing skips the full-surface rasterise, which step 4 replaces.
./target/release/examples/bench --frames=400
echo
echo "  second run, for the spread:"
./target/release/examples/bench --frames=400 --only=typing | head -12

hr "4 · what a keystroke really rasterises"
# bench.rs above rasterises the whole 1440x900 window. The platform does not:
# it repaints the damage. In my run that was an 86x23 region and the
# full-surface figure overstated a keystroke by 462x. This prints both.
./target/release/examples/raster_real

hr "5 · cold start"
# Run twice on purpose. Inside the repo, rust-toolchain.toml pins a toolchain;
# if yours is installed this is fast, and if it is not, rustup tries to fetch
# it on the studio's startup path — which is the case the timeout in
# toolchains.rs now bounds at 150 ms.
echo "  (a) from inside this repo — the pinned-toolchain path:"
for i in 1 2 3; do ./target/release/examples/coldstart | grep -E 'Studio::new|first complete'; done
echo
echo "  (b) from a directory with no toolchain pin:"
cp target/release/examples/coldstart "${TMPDIR:-/tmp}/vieww-coldstart"
( cd "${TMPDIR:-/tmp}" && for i in 1 2 3; do ./vieww-coldstart | grep -E 'Studio::new|first complete'; done )

hr "6 · is the pale edge ours or the film's"
# 64 configurations of a light rounded rect composited over a near-black stage.
# PASS means the rasteriser is correct and the rim in the master is authored.
cargo run --release -p vieww-paint --features native --example edge_probe | tail -6

hr "7 · what ships"
cp target/release/viewwstudio "${TMPDIR:-/tmp}/vs-stripped" && strip "${TMPDIR:-/tmp}/vs-stripped"
printf 'unstripped  %s\n' "$(du -h target/release/viewwstudio | cut -f1)"
printf 'stripped    %s\n' "$(du -h "${TMPDIR:-/tmp}/vs-stripped" | cut -f1)"
echo
echo "shared libraries:"
ldd target/release/viewwstudio 2>/dev/null || otool -L target/release/viewwstudio 2>/dev/null

hr "8 · tests"
# One known failure in this container that I could not attribute:
# `a_panicking_build_on_a_later_rebuild_is_caught_by_the_host_not_aborted`
# aborts with "Rust cannot catch foreign exceptions". It is a panic-ABI
# question across the cdylib boundary and nothing to do with the toolchains
# change — but I was on 1.95.0, not your 1.98.1, so please confirm.
cargo test --release -p viewwstudio 2>&1 | grep -E '^test result|FAILED|^error' | head -20

hr "done"
echo "Send me the whole output. The four figures I most want are:"
echo "  · idle frame and keystroke frame medians          (step 3)"
echo "  · the damaged-region raster figure                (step 4)"
echo "  · cold start (b), the unpinned case               (step 5)"
echo "  · stripped binary size and the ldd list           (step 7)"
