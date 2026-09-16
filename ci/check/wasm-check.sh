#!/usr/bin/env bash
#
# Compile the web backend.
#
# # History, in one line each
#
# * Written on a machine that could not install `wasm32-unknown-unknown`
#   (`rustup target add` needs `static.rust-lang.org`, which its network
#   policy did not reach) — so this script was the plan for a machine that
#   could, and its header said so.
# * First passed on 2026-09-14, on a machine with the target: it found seven
#   clippy `-D warnings` failures in `vieww-paint` (the crate the web backend
#   builds against), all in code that `ci/check/checks.sh` had never linted for
#   this target — target-dependent lints, mostly, which is exactly the class
#   of thing a second target exists to catch.
# * The same session then ran it: `examples/test-web` builds the crate's
#   scene as wasm, mounts it in a headless Chromium, and compares the canvas
#   readback byte for byte against a native render of the same scene. Equal,
#   in both pointer states — the comparison and its evidence live in that
#   example's docs.
#
# # What this script checks, and what it does not
#
# `crates/vieww-platform-web`'s dependencies and its single module are gated
# to `cfg(target_arch = "wasm32")`, so on every other target this crate
# builds to nothing. `ci/check/checks.sh` running green says *nothing at all* about
# this crate; checking it needs a different target, so it needs this script.
#
# The stage-0 audit below needs no toolchain at all: `web-sys` gates every
# binding behind a cargo feature named after the type, so naming
# `web_sys::Event` without listing `"Event"` is a compile error — and that
# comparison runs first so a machine that cannot install the target still
# finds that class of mistake. (It found exactly two, once.)
#
# Usage:
#   ci/check/wasm-check.sh
#
# Requires: rustup, and network access to `static.rust-lang.org` for the
# target install.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

target="wasm32-unknown-unknown"
crate="crates/vieww-platform-web"

# ── stage 0: the check that needs no toolchain at all ──────────────────────
#
# `web-sys` gates every binding behind a cargo feature named after the type, so
# naming `web_sys::Event` without listing `"Event"` is a compile error — and it
# is the first thing this script's own doc predicts it will find. That check
# does not need the wasm target, or a compiler, or a network: it is a
# comparison between two lists of names in two files in this repository.
#
# It runs first, and unconditionally, because a machine that cannot install the
# target should still be able to find this class of mistake — which is exactly
# the machine this crate was written on, and exactly why two of them
# (`Event`, `EventTarget`) sat in it unnoticed from the day it was written.
echo "==> auditing web-sys features against the types the crate names"
# The names the crate uses, from both spellings: a `web_sys::Thing` path, and a
# `use web_sys::{Thing, Other as Alias}` list. An alias is resolved to the name
# on the left of `as`, because that is the one `web-sys` gates on — matching the
# alias instead would report `DomPointerEvent`, a type that does not exist.
used=$(
        {
                grep -oh 'web_sys::[A-Z][A-Za-z0-9_]*' "$crate/src/lib.rs" | sed 's/web_sys:://'
                sed -n '/^use web_sys::{/,/};/p' "$crate/src/lib.rs" |
                        tr ',' '\n' |
                        sed 's/.*{//; s/}.*//; s/ as .*//; s/[^A-Za-z0-9_]//g' |
                        grep -E '^[A-Z]'
        } | sort -u
)
enabled=$(sed -n '/^features = \[/,/^]/p' "$crate/Cargo.toml" |
        grep -o '"[A-Za-z0-9_]*"' | tr -d '"' | sort -u)
missing=$(comm -23 <(echo "$used") <(echo "$enabled"))
if [ -n "$missing" ]; then
        echo "error: $crate names web-sys types that its Cargo.toml does not enable:" >&2
        echo "$missing" | sed 's/^/    /' >&2
        echo >&2
        echo "Add them to the web-sys 'features' list. Each one is a 'cannot find" >&2
        echo "type' error waiting for the first machine with the target installed," >&2
        echo "and there is no reason to make that machine be the one to find it." >&2
        exit 1
fi
echo "    every web-sys type the crate names is enabled"

# ── stage 1: the target ────────────────────────────────────────────────────
#
# `rustup target list` is asked with stderr discarded, deliberately. On a
# machine that cannot reach `static.rust-lang.org`, rustup re-syncs the channel
# manifest before answering *any* query and prints a twenty-six frame Rust
# backtrace when that fails — while still answering the question correctly from
# what is already on disk. `TRACKER.md` records the same trap costing a day
# once already, in `viewwstudio`'s toolchain test: a query that exits non-zero
# for a reason that has nothing to do with what was asked.
#
# So: read the list, ignore the noise, and decide from the answer.
installed=$(rustup target list --installed 2>/dev/null || true)

if ! echo "$installed" | grep -qx "$target"; then
        # Probed before `rustup target add` is run, so that an unreachable host is
        # reported as an unreachable host in one line rather than as a download
        # failure under a backtrace that reads like a crash in rustup.
        if ! curl -fsS -m 15 -o /dev/null https://static.rust-lang.org/dist/channel-rust-stable.toml; then
                echo >&2
                echo "error: $target is not installed, and static.rust-lang.org is not" >&2
                echo "       reachable from this machine, so it cannot be installed." >&2
                echo >&2
                echo "       This is a network policy rather than a fault in this" >&2
                echo "       workspace — and it is the exact reason" >&2
                echo "       crates/vieww-platform-web has never been compiled. Run this" >&2
                echo "       script on a machine that can reach that host." >&2
                echo >&2
                echo "       Stage 0 above still ran. It is the half of this check that" >&2
                echo "       needs no toolchain, and it passed." >&2
                exit 2
        fi
        echo "==> installing $target"
        rustup target add "$target"
fi

echo "==> cargo check -p vieww-platform-web --target $target"
cargo check -p vieww-platform-web --target "$target" --all-targets

echo "==> cargo clippy -p vieww-platform-web --target $target"
cargo clippy -p vieww-platform-web --target "$target" --all-targets -- -D warnings

echo
echo "vieww-platform-web compiles for $target."
echo "It has also *run*: examples/test-web mounts this crate's WebApp in a"
echo "headless browser and compares the canvas against a native render,"
echo "byte for byte. Re-certify with that example's build-web.sh + the"
echo "certification runner whenever the web-facing code changes."
