#!/usr/bin/env bash
#
# Build the web half of the certification: the scene as wasm, the page that
# mounts it, and nothing else.
#
# Usage:
#   examples/test-web/build-web.sh              # -> examples/test-web/dist/
#   examples/test-web/build-web.sh /tmp/out
#
# Requires the `wasm32-unknown-unknown` target and a `wasm-bindgen` matching
# the lockfile — the same two things `apps/viewwsite/build-site.sh` checks,
# for the same reasons. See that script for the version-mismatch trap.
#
# Then verify it: `examples/test-web/verify_web.py <out> <baseline-dir>` does
# the serving, the headless browsing, the canvas readback and the byte
# comparison in one run (it used to live outside this repository).

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

out="${1:-examples/test-web/dist}"
target="wasm32-unknown-unknown"

wanted="$(sed -n '/^name = "wasm-bindgen"$/,/^version/ s/^version = "\(.*\)"/\1/p' Cargo.lock | head -1)"
if ! command -v wasm-bindgen >/dev/null; then
        echo "error: wasm-bindgen is not on PATH." >&2
        echo "       cargo install wasm-bindgen-cli --version $wanted" >&2
        exit 1
fi
have="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$have" != "$wanted" ]; then
        echo "error: wasm-bindgen $have, but Cargo.lock pins wasm-bindgen $wanted." >&2
        echo "       cargo install wasm-bindgen-cli --version $wanted --force" >&2
        exit 1
fi

# Release, without the size tuning `viewwsite`'s page takes: this is a
# verification build, and what it optimises for is a fast, boring build. The
# rasteriser's loops still need `--release` — a debug wasm build of a CPU
# renderer over 960x600 is seconds per frame.
cargo build --release -p test-web --target "$target"

mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" \
        "target/$target/release/test_web.wasm"
cp examples/test-web/index.html "$out/index.html"

raw="$(stat -c%s "$out/test_web_bg.wasm" 2>/dev/null || stat -f%z "$out/test_web_bg.wasm")"
echo
echo "$out — index.html, test_web.js, test_web_bg.wasm ($raw bytes)"
echo "Serve with application/wasm for the .wasm — python's http.server does,"
echo "and examples/test-web/verify_web.py starts one."
