#!/usr/bin/env bash
#
# Build the product page: Rust to wasm, wasm to a directory a static host can
# serve.
#
# Usage:
#   apps/viewwsite/build-site.sh                 # -> apps/viewwsite/dist/
#   apps/viewwsite/build-site.sh /tmp/out        # somewhere else
#   VIEWW_REPO=owner/name apps/viewwsite/build-site.sh
#
# `VIEWW_REPO` replaces the `__REPO__` placeholder in every download URL, in
# both the wasm page and the HTML fallback. Without it the links point at
# `github.com/__REPO__/...`, which is deliberate: a placeholder that 404s is
# better than a plausible-looking wrong repository.
#
# # What it needs
#
#   * the `wasm32-unknown-unknown` target — `rustup target add
#     wasm32-unknown-unknown`, or, on a machine that cannot reach
#     `static.rust-lang.org`, the build-std recipe below;
#   * `wasm-bindgen` **exactly matching** the `wasm-bindgen` version in
#     `Cargo.lock` — `cargo install wasm-bindgen-cli --version <that>`. A
#     mismatch is a runtime error in the browser about a schema version, not a
#     build failure, which is why this script checks it.
#
# # The build-std path, for a machine without the target
#
# `rustup target add` fetches from `static.rust-lang.org`. Where that host is
# unreachable, build `std` from source instead — it needs github.com only:
#
#   git clone --depth 1 --branch "$(rustc -V | cut -d' ' -f2)" \
#       --filter=blob:none --sparse https://github.com/rust-lang/rust.git
#   cd rust && git sparse-checkout set library
#   # library/backtrace is a submodule; clone rust-lang/backtrace-rs at the
#   # commit `git ls-tree HEAD library/backtrace` names, into that path.
#   cp -r library Cargo.lock "$(rustc --print sysroot)/lib/rustlib/src/rust/"
#
# then run this script with VIEWW_BUILD_STD=1.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

out="${1:-apps/viewwsite/dist}"
repo="${VIEWW_REPO:-__REPO__}"
target="wasm32-unknown-unknown"

# ── the toolchain ──────────────────────────────────────────────────────────

wanted="$(sed -n '/^name = "wasm-bindgen"$/,/^version/ s/^version = "\(.*\)"/\1/p' Cargo.lock | head -1)"
if ! command -v wasm-bindgen >/dev/null; then
	echo "error: wasm-bindgen is not on PATH." >&2
	echo "       cargo install wasm-bindgen-cli --version $wanted" >&2
	exit 1
fi
have="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$have" != "$wanted" ]; then
	# Worth stopping for: a mismatch builds fine and fails in the browser with
	# a message about schema versions that names neither of these numbers.
	echo "error: wasm-bindgen $have, but Cargo.lock pins wasm-bindgen $wanted." >&2
	echo "       cargo install wasm-bindgen-cli --version $wanted --force" >&2
	exit 1
fi

flags=(--release -p viewwsite --target "$target")
if [ -n "${VIEWW_BUILD_STD:-}" ]; then
	export RUSTC_BOOTSTRAP=1
	flags+=(-Z build-std=std,panic_abort)
fi

# ── build ──────────────────────────────────────────────────────────────────

echo "==> cargo build ${flags[*]}"
# The profile, and why each switch is here — all measured on this page:
#
#   -O3, debug off              2 770 716 raw   955 229 gzip
#   -Oz + fat LTO               3 470 004 raw   952 566 gzip
#   -Oz + fat LTO + strip       1 955 937 raw   765 089 gzip   <- this
#
# Those three are the ratios, measured on a shorter version of the page; the
# page has roughly doubled in content since and the current build is 2 387 232
# raw / 976 698 gzip under the same switches. What the rows are here to say is
# the *shape* of the trade, which has not changed.
#
# `strip` is not a rounding error and is easy to leave out: `debug = false`
# stops *debug info* being emitted and leaves every symbol name in the binary,
# which here is a megabyte and a half of Rust paths nobody loading the page
# will ever read. Note the middle row — without it, `-Oz` is bigger than `-O3`.
#
# A fifth off the wire for no observable difference in how the page runs: this
# is a page, and the rasteriser spends its time in tight loops that `opt-level
# = "z"` does not meaningfully slow. On an *application* — the studio in a
# browser, say — measure before assuming the same trade holds.
#
# `wasm-opt -Oz` is deliberately not run. It takes the raw file down to
# 1 770 509, and gzip *up* to 792 521: it trades the repetition gzip feeds on
# for smaller instructions, and the wire is what a visitor waits for.
#
# Debug info is off because it is ten times the payload for something nobody
# loading the page can read.
CARGO_PROFILE_RELEASE_DEBUG=false \
CARGO_PROFILE_RELEASE_STRIP=symbols \
CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
CARGO_PROFILE_RELEASE_LTO=fat \
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
	cargo build "${flags[@]}"

echo "==> wasm-bindgen --target web"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" \
	"target/$target/release/viewwsite.wasm"

echo "==> index.html, fonts and figures"
sed "s|__REPO__|$repo|g" apps/viewwsite/index.html > "$out/index.html"
# The faces the DOM page is set in, and the two renderer figures it shows.
# Files rather than bytes in the wasm: the browser caches them on their own,
# and a repeat visit pays for neither.
mkdir -p "$out/fonts"
for face in Geist-Regular Geist-Medium Geist-Bold GeistMono-Regular GeistMono-Medium; do
	cp "apps/viewwsite/assets/fonts/vw-$face.ttf" "$out/fonts/$face.ttf"
done
cp apps/viewwsite/assets/fonts/Geist-OFL.txt "$out/fonts/OFL.txt"
cp apps/viewwsite/assets/studio.png apps/viewwsite/assets/og.png \
	apps/viewwsite/assets/figure-blend.png apps/viewwsite/assets/figure-depth.png \
	apps/viewwsite/assets/figure-easing.png "$out/"

raw="$(stat -c%s "$out/viewwsite_bg.wasm" 2>/dev/null || stat -f%z "$out/viewwsite_bg.wasm")"
gz="$(gzip -9 -c "$out/viewwsite_bg.wasm" | wc -c | tr -d ' ')"
echo
echo "$out — index.html, viewwsite.js, viewwsite_bg.wasm"
echo "wasm: $raw bytes, $gz gzipped — the second number is what a visitor waits for"
echo "repository in the download links: $repo"
echo
echo "Serve it with any static host. Two headers matter:"
echo "  .wasm  ->  Content-Type: application/wasm   (or the browser refuses to"
echo "             stream-compile it and falls back, slowly, or not at all)"
echo "  .wasm  ->  Content-Encoding: br or gzip     (2 MB -> about 765 KB)"
