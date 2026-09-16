#!/usr/bin/env bash
#
# Build, install and launch the hot-reload host on an attached phone.
#
#   ci/mobile/reload-android.sh             # build, install, run, follow the log
#   ci/mobile/reload-android.sh --no-log    # build, install, run, and return
#
# Then, in another terminal, watch for edits and push them automatically:
#
#   ci/mobile/reload-watch-android.sh
#
# or do it by hand, once per edit:
#
#   ci/mobile/reload-guest-android.sh --package dev.vieww.reload
#
# The activity is not reinstalled by either — it picks the library up between
# frames. Editing `BAND` should change the band without resetting the tap count;
# adding a field to `Taps` should reset it and say why.
#
# Exists because `cargo apk` reads the SDK and NDK from the environment and a
# plain `cargo apk run -p reload-android` in a fresh shell fails with "Android
# SDK is not found" — which is a missing variable, not a missing SDK.

set -euo pipefail

package="dev.vieww.reload"
follow=1

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-log) follow=0 ;;
        -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "reload-android: unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

command -v adb >/dev/null 2>&1 || { echo "reload-android: adb not found." >&2; exit 127; }
adb shell true >/dev/null 2>&1 || { echo "reload-android: no device reachable." >&2; adb devices -l >&2; exit 3; }

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

# shellcheck source=ci/mobile/android-env.sh
. "$root/ci/mobile/android-env.sh"
android_env "reload-android" || exit $?

command -v cargo-apk >/dev/null 2>&1 || {
    echo "reload-android: cargo-apk not found." >&2
    echo "  cargo install cargo-apk" >&2
    exit 127
}

echo "reload-android: building and installing $package (release)"
# Release for the same reason the demo is: a debug build's frame times on this
# hardware are not the ones anybody should be looking at, and hot reload is
# about the guest's build, not the host's.
#
# `|| true` for the reason ci/mobile/device-suite.sh gives: cargo apk exits non-zero
# after a successful launch on this device — it parses `pm list packages -U`
# output that carries two uids and fails on "10207,1010207 is not a UID". The
# app is running by then.
cargo apk run -p reload-android --release || true

for _ in $(seq 1 20); do
    [ -n "$(adb shell pidof "$package" 2>/dev/null | tr -d '\r' || true)" ] && break
    sleep 0.5
done

pid="$(adb shell pidof "$package" 2>/dev/null | tr -d '\r' | awk '{print $1}' || true)"
if [ -z "$pid" ]; then
    echo "reload-android: the activity did not start." >&2
    echo "  adb logcat -s vieww-reload  may say why." >&2
    exit 8
fi
echo "reload-android: running as pid $pid"

echo
echo "reload-android: push a guest with"
echo "    ci/mobile/reload-guest-android.sh --package $package"

if [ "$follow" -eq 0 ]; then
    exit 0
fi

echo
echo "reload-android: following the log — ^C to stop, the activity keeps running"
# Not `-d`: the interesting lines are the ones that arrive *after* a push, so
# this follows rather than dumping what is already there.
adb logcat -s vieww-reload
