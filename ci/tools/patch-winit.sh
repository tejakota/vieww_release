#!/usr/bin/env bash
#
# Vendor winit and teach its Android backend to surface hover events.
#
#     ci/tools/patch-winit.sh          # vendor and patch
#     ci/tools/patch-winit.sh --check  # does the patch still apply? changes nothing
#     ci/tools/patch-winit.sh --clean  # remove the vendored copy
#
# # What this is for
#
# **winit 0.30.13 throws away every hover event on Android.**
# `src/platform_impl/android/mod.rs` maps `Down`, `PointerDown`, `Up`,
# `PointerUp`, `Move` and `Cancel`, and everything else falls into
# `_ => { None // TODO mouse events }`. With TalkBack's explore-by-touch on, a
# finger dragged over the screen arrives as `HoverEnter`/`HoverMove`/`HoverExit`
# and as nothing else — so it is discarded a layer below vieww, before any code
# in this repository runs.
#
# That is why touch exploration does nothing on this app, and it is **not
# fixable in this repository**: three attempts at choosing a different host view
# changed nothing and never could have. Measured on 2026-08-15 with a patched
# `accesskit_android` logging both sides of its hover decision, and raw
# `getevent` capture running alongside so the run could prove a finger was on the
# screen rather than assume it:
#
#     raw input lines : 1955     <- the screen was unambiguously touched
#     a11y-probe lines: 0        <- hover never reached Rust
#
# # Why a patch file and not a fork
#
# Because the point is the diff. `ci/tools/winit-hover.patch` is written against
# upstream winit and is the change to send them — keeping it as a patch means it
# stays reviewable and stays honest about being a local deviation, where a
# vendored fork quietly becomes permanent. It also keeps ~100k lines of somebody
# else's source out of this repository's history.
#
# # This is only half of the fix
#
# Once the events arrive, something has to hand them to AccessKit.
# `accesskit_android::InjectingAdapter` registers `onHoverEvent` as a **native
# method on its Java delegate**, so it cannot be called from Rust — the
# `adapter_handle` it needs is private. `docs/HANDOFF.md` predicted no
# accesskit change would be required, on the assumption that vieww could call
# the Java `Delegate.onHover(View, MotionEvent)` directly. That is still true and
# is what `a11y_android::AndroidAdapter::on_hover` does: it synthesises a Java
# `MotionEvent` with `MotionEvent.obtain` and hands it to the delegate reached
# through `View.getAccessibilityDelegate()`.
#
# So: this script for the events, `a11y_android.rs` for the delivery, and a
# finger on a phone for the verdict.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
patch_file="$root/ci/tools/winit-hover.patch"
vendor="$root/vendor/winit"
# Pinned, and it has to match what `Cargo.lock` resolves winit to. A patch
# applied to a different version is the kind of thing that appears to work and
# then silently stops mapping one of the three actions.
version="0.30.13"

mode="apply"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --check) mode="check" ;;
        --clean) mode="clean" ;;
        -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "patch-winit: unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

if [ "$mode" = "clean" ]; then
    rm -rf "$vendor"
    echo "patch-winit: removed $vendor"
    echo "  Remember to comment out the [patch.crates-io] block in Cargo.toml."
    exit 0
fi

# The registry, not a download: cargo has already fetched winit to build this
# workspace, so vendoring from there needs no network and cannot get a different
# source than the one the build used.
registry="$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -maxdepth 1 -type d -name 'index.crates.io-*' | head -1)"
source_dir="$registry/winit-$version"

if [ ! -d "$source_dir" ]; then
    echo "patch-winit: no winit-$version in the cargo registry." >&2
    echo "  Build the workspace once so cargo fetches it, then re-run:" >&2
    echo "    cargo check -p vieww-platform-winit" >&2
    exit 3
fi

if [ "$mode" = "check" ]; then
    # Against a scratch copy, so a check never leaves a half-patched tree behind.
    scratch="$(mktemp -d)"
    trap 'rm -rf "$scratch"' EXIT
    cp -r "$source_dir/." "$scratch/"
    if patch -p1 --dry-run --silent -d "$scratch" < "$patch_file"; then
        echo "patch-winit: the patch still applies to winit $version."
        exit 0
    fi
    echo "patch-winit: the patch NO LONGER APPLIES to winit $version." >&2
    echo "  winit moved underneath it. Rebase ci/tools/winit-hover.patch, and while" >&2
    echo "  you are there check whether upstream fixed this themselves — the" >&2
    echo "  whole point of keeping it as a patch is that it can go away." >&2
    exit 4
fi

rm -rf "$vendor"
mkdir -p "$(dirname "$vendor")"
cp -r "$source_dir" "$vendor"
# The registry copy is read-only, and `patch` cannot edit a read-only file.
chmod -R u+w "$vendor"

# `.cargo-ok` and the checksum file mark a registry checkout; a path dependency
# with them present confuses nothing today but is misleading to read.
rm -f "$vendor/.cargo-ok" "$vendor/.cargo-checksum.json"

patch -p1 -d "$vendor" < "$patch_file"

cat <<EOF

patch-winit: winit $version vendored and patched at
  vendor/winit

**One manual step is left**, and it is deliberate rather than automated: adding
a [patch.crates-io] section rewires every build in this workspace, including
CI's, and a script that did it silently would be a local deviation nobody
remembers making.

Uncomment these two lines in Cargo.toml, just below [workspace.dependencies]:

    [patch.crates-io]
    winit = { path = "vendor/winit" }

    sed -n '60,72p' Cargo.toml    # to see them in place

Then, on a phone, with TalkBack on by hand:

    cargo apk run -p vieww-platform-winit --example android --release
    ci/mobile/a11y-android.sh --activate

and **touch the screen**. What changes if this works: a focus rectangle follows
your finger and TalkBack speaks what is under it. That is the one step nothing
in this repository can check.
EOF
