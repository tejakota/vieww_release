#!/usr/bin/env bash
# Build and run an Android example with the toolchain resolved first.
#
#     ci/mobile/apk.sh --example android
#     ci/mobile/apk.sh --example android --release
#
# # Why this exists
#
# `cargo apk` reads the NDK from the environment, and when nothing tells it
# otherwise it falls back to `$ANDROID_HOME/ndk-bundle`. On this machine that
# is **NDK r22.1**, which predates `libunwind` — so the build runs for a couple
# of minutes and then dies at link time with:
#
#     ld: error: unable to find library -lunwind
#
# That error names a missing C library and looks like a broken checkout or a
# Rust problem. It is neither: it is the wrong NDK, chosen silently by a
# fallback, and `$ANDROID_HOME/ndk/27.3.13750724` was sitting there the whole
# time. It cost two builds before anyone read the toolchain path in the
# command line rather than the error at the end of it.
#
# `ci/mobile/android-env.sh` already knew which NDK to use. The only thing missing was
# something that made *not* asking it impossible, which is all this is: it adds
# to that file rather than beside it, per the rule in `docs/HANDOFF.md`.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=ci/mobile/android-env.sh
. "$root/ci/mobile/android-env.sh"
android_env "apk" || exit 1

cd "$root"
exec cargo apk run -p vieww-platform-winit "$@"
