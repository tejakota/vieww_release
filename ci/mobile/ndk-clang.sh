#!/usr/bin/env bash
#
# Link an Android target with the NDK's clang.
#
# Not called by hand. `.cargo/config.toml` points each Android target at one of
# the two shims beside this file, and each shim passes its architecture as `$1`.
#
# # What this fixes
#
# Without a linker configured, cargo links Android test binaries with the host
# `cc`, and the failure names nothing Android at all:
#
#     /usr/bin/x86_64-linux-gnu-ld.bfd: cannot find -llog
#     /usr/bin/x86_64-linux-gnu-ld.bfd: cannot find -lunwind
#     /usr/bin/x86_64-linux-gnu-ld.bfd: cannot find -landroid
#
# Those are the NDK's sysroot libraries. The host linker has never heard of
# them, so it reports them as missing rather than as wrong-toolchain — which
# reads like a broken system rather than like an unconfigured build.
#
# It only bites on things that **link**: a workspace cross-build produces rlibs
# and needs no linker at all, which is why `.github/workflows/ci.yml`'s
# `Cross-compile — Android` step passes on a runner with no NDK, and why this
# went unnoticed until somebody ran the tests.
#
# # Why a script rather than a path in the config
#
# `linker` takes one program and no arguments, and the NDK's path differs on
# every machine: it is a version directory under `$ANDROID_HOME`, and a version
# directory is not necessarily an NDK — the SDK manager leaves a partial
# download behind when an install is abandoned. `ci/mobile/android-env.sh` already
# knows how to pick the right one and how to skip the wrong one; this hands
# cargo that answer instead of a guess that goes stale on the next SDK update.
#
# # This does not change what CI does
#
# An environment variable beats `.cargo/config.toml`, and the workflow sets
# `CARGO_TARGET_<ARCH>_LINUX_ANDROID_LINKER` explicitly. `cargo apk` sets its
# own for the same reason. Both keep the linker they chose; this only fills in
# the case where nobody chose one.

set -euo pipefail

arch="${1:?ndk-clang: expected an architecture as the first argument}"
shift

root="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=ci/mobile/android-env.sh
. "$root/ci/mobile/android-env.sh"
# The summary line goes to stdout, and a linker's stdout is nobody's business.
# Diagnostics and the multiple-NDK warning are on stderr and are kept.
android_env "ndk-clang" >/dev/null || exit $?

# Globbed rather than hardcoded to `linux-x86_64`: an NDK ships exactly one
# prebuilt toolchain, named for the host it runs on, and hardcoding the Linux
# one is a macOS break that only shows up on somebody else's machine.
ndk_bin=""
for candidate in "$ANDROID_NDK_ROOT"/toolchains/llvm/prebuilt/*/bin; do
    if [ -d "$candidate" ]; then
        ndk_bin="$candidate"
        break
    fi
done

if [ -z "$ndk_bin" ]; then
    echo "ndk-clang: no prebuilt toolchain under $ANDROID_NDK_ROOT/toolchains/llvm/prebuilt." >&2
    echo "  That NDK is incomplete. Install it again, or set ANDROID_NDK_ROOT" >&2
    echo "  to one that is not." >&2
    exit 6
fi

# 24 because that is what the APK claims to run on —
# `crates/vieww-platform-winit/Cargo.toml`'s `min_sdk_version` — and what CI
# links against. Linking to a lower API than the device runs is the supported
# direction; the reverse silently produces a binary the phone cannot load.
api=24
clang="$ndk_bin/$arch-linux-android$api-clang"

if [ ! -x "$clang" ]; then
    echo "ndk-clang: $clang is missing." >&2
    echo "  The NDK at $ANDROID_NDK_ROOT has no API $api toolchain for $arch." >&2
    echo "  Available for this architecture:" >&2
    ls "$ndk_bin" | grep -E "^$arch-linux-android[0-9]+-clang$" | sed 's/^/    /' >&2 || true
    exit 6
fi

exec "$clang" "$@"
