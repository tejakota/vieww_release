#!/usr/bin/env bash
#
# Cross-compile the hot-reload guest for Android and stage it where the demo
# can reach it.
#
# This is the confirming half of the Android hot-reload question. The demo's
# `dlopen_from_a_writable_path` probe already answered PERMITTED, but it did so
# against 28 bytes of text that the linker rejected at `e_ident` — which proves
# the file was opened and no W^X policy stood in the way, and proves nothing
# about any check that happens after an ELF header parses. This script supplies
# a genuine `cdylib` so `dlopen_a_real_guest_library` can answer the rest.
#
#   ci/mobile/reload-guest-android.sh            # build, stage, relaunch, and report
#   ci/mobile/reload-guest-android.sh --no-run   # build and stage only
#   ci/mobile/reload-guest-android.sh --remove   # remove the staged copies
#
#   # stage for examples/reload-android, which reloads live from the push.
#   # Implies --no-run: the suite only knows how to drive the demo.
#   ci/mobile/reload-guest-android.sh --package dev.vieww.reload
#
# That last line is one turn of an edit loop. `ci/mobile/reload-watch-android.sh` runs
# it for you on every save, and reports what the phone made of each push.
#
# Needs adb, one device attached, and cargo-ndk (`cargo install cargo-ndk`).
#
# This script only *stages* the library somewhere readable. The app copies it
# into its own private directory and loads it from there — see the long comment
# on the staging paths below for why that is the honest way round rather than a
# workaround.

set -euo pipefail

package="dev.vieww.demo"
# Must match `GUEST_LIBRARY` in examples/android.rs.
library="libreload_guest.so"
# The one Adreno/Mali phone target. Kept explicit rather than derived from the
# device, because a wrong guess here produces a library that fails to load and
# reads exactly like a policy refusal — the confusion this whole script exists
# to remove.
abi="arm64-v8a"
target="aarch64-linux-android"
run=1
remove=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-run) run=0 ;;
        --remove) remove=1 ;;
        # Which app's external directory to stage into. `dev.vieww.demo` is the
        # suite's demo, which only *probes* the library; `dev.vieww.reload` is
        # `examples/reload-android`, which actually reloads from it. They are
        # separate packages because one is built with hot-reload and the other
        # must not be.
        --package)
            shift
            [ "$#" -gt 0 ] || { echo "reload-guest: --package needs a value" >&2; exit 2; }
            package="$1"
            # The suite only knows how to drive the demo, so targeting anything
            # else implies staging alone. Silently running it against the wrong
            # app would report on a package this push never touched.
            [ "$package" = "dev.vieww.demo" ] || run=0
            ;;
        -h|--help) sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "reload-guest: unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

command -v adb >/dev/null 2>&1 || { echo "reload-guest: adb not found." >&2; exit 127; }
adb shell true >/dev/null 2>&1 || { echo "reload-guest: no device reachable." >&2; adb devices -l >&2; exit 3; }

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

# shellcheck source=ci/mobile/android-env.sh
. "$root/ci/mobile/android-env.sh"

# The app must have been installed at least once, or its external directory
# cannot be created and there is no process to copy the library in anyway.
if ! adb shell pm list packages 2>/dev/null | tr -d '\r' | grep -q "^package:$package$"; then
    echo "reload-guest: $package is not installed on this device." >&2
    # The advice has to name the app that was asked for. It used to say "run
    # ci/mobile/device-suite.sh" whatever the package was, which for the reload host is
    # advice to install a different application.
    if [ "$package" = "dev.vieww.reload" ]; then
        echo "  Install it first — it starts without a guest and waits for one:" >&2
        echo "      ci/mobile/reload-android.sh" >&2
    else
        echo "  Run ci/mobile/device-suite.sh first — it builds and installs the demo." >&2
    fi
    exit 4
fi

# **The app copies the library in; this script only stages it.**
#
# The private directory the experiment is about — /data/data/$package/files —
# cannot be written by `adb push`, and the tool that can write it, `run-as`,
# requires `android:debuggable`. Building a debuggable APK to satisfy it would
# destroy the measurement: debuggable is the flag the platform relaxes policy
# for, so the answer would be about an app nobody ships, and it would not be the
# release APK the rest of the suite reports on. That was the first failure of
# this script — `run-as: package not debuggable` — and the fix is to stop
# needing it.
#
# So two staging locations, and the app takes whichever it can read:
#
#   1. the app's own external directory, which adb writes freely and the app can
#      always read;
#   2. /data/local/tmp, world-readable by mode, though SELinux may refuse an
#      `untrusted_app` reading `shell_data_file` on newer releases.
#
# Both are written, because which of them works is a property of the device and
# not worth guessing. The app reports which one it used.
external="/sdcard/Android/data/$package/files"
staging="/data/local/tmp/$library"

if [ "$remove" -eq 1 ]; then
    adb shell rm -f "$external/$library" "$staging" 2>/dev/null || true
    echo "reload-guest: removed the staged copies."
    echo "  The app's private copy cannot be reached without run-as; clear it with"
    echo "  'adb shell pm clear $package' if you need it gone."
    exit 0
fi

command -v cargo-ndk >/dev/null 2>&1 || {
    echo "reload-guest: cargo-ndk not found." >&2
    echo "  cargo install cargo-ndk" >&2
    exit 127
}

# The SDK and NDK, resolved by the one place that knows how.
#
# This block used to be forty lines of bespoke discovery here, and it was written
# in ignorance of `ci/mobile/device-suite.sh` having already solved the identical
# problem — including the partial `ndk/30.x` install on this machine. See the
# header of ci/mobile/android-env.sh.
android_env "reload-guest" || exit $?

echo "reload-guest: building examples/reload-guest for $target"
# `-p reload-guest` and not the workspace: this is the only crate that must link
# for Android, and it is excluded from the mobile CI jobs precisely because the
# Linux runner's `cc` cannot. cargo-ndk supplies the NDK linker that `cargo
# check` does not.
#
# Release, because a debug cdylib carries enough symbols to make the push slow
# and the answer is not affected by the opt level.
cargo ndk --target "$abi" --platform 24 -- build -p reload-guest --release

built="target/$target/release/$library"
if [ ! -f "$built" ]; then
    echo "reload-guest: expected $built and it is not there." >&2
    echo "  Is examples/reload-guest still crate-type = [\"cdylib\"]?" >&2
    exit 5
fi

size="$(wc -c <"$built" | tr -d ' ')"
echo "reload-guest: built $built ($size bytes)"

# Neither push is required to succeed on its own — one working staging location
# is enough, and which one depends on the device's SELinux policy.
landed=0

# The external directory may not exist until the app has asked for it once, so
# create it rather than relying on a previous launch.
adb shell mkdir -p "$external" >/dev/null 2>&1 || true
if adb push "$built" "$external/$library" >/dev/null 2>&1; then
    echo "reload-guest: staged in $external"
    landed=$((landed + 1))
fi

if adb push "$built" "$staging" >/dev/null 2>&1; then
    # Readable by the app: /data/local/tmp is 0771, so the directory can be
    # traversed but not listed, and an exact path opens only if the file itself
    # permits it.
    adb shell chmod 644 "$staging" >/dev/null 2>&1 || true
    echo "reload-guest: staged at $staging"
    landed=$((landed + 1))
fi

if [ "$landed" -eq 0 ]; then
    echo "reload-guest: could not stage the library anywhere adb can write." >&2
    exit 7
fi

# Verify a staged copy rather than trusting it: a truncated library fails to
# load and is, from the app's side, indistinguishable from one the linker
# refused.
pushed="$(adb shell wc -c "$external/$library" 2>/dev/null | tr -d '\r' | awk '{print $1}' || true)"
if [ -z "$pushed" ]; then
    pushed="$(adb shell wc -c "$staging" 2>/dev/null | tr -d '\r' | awk '{print $1}' || true)"
fi
if [ -n "$pushed" ] && [ "$pushed" != "$size" ]; then
    echo "reload-guest: staged $pushed bytes of $size — the copy is short." >&2
    exit 7
fi
echo "reload-guest: $size bytes verified on device"

if [ "$run" -eq 0 ]; then
    echo "reload-guest: --no-run, so stopping here."
    echo "  Run ci/mobile/device-suite.sh — the full one, not --no-build — and read"
    echo "  dlopen_a_real_guest_library."
    exit 0
fi

echo
echo "reload-guest: rebuilding the demo, then reading the answer"
# **A full `ci/mobile/device-suite.sh`, not `--no-build`, and this script shipped the
# other version once.**
#
# The probe that looks for a staged library lives in the *demo APK*, not in the
# guest. `--no-build` runs whatever is already installed, so the first working
# version of this script staged 751,344 bytes correctly and then asked a
# stale observer about them. It answered "not pushed", about a file sitting on
# the device, in wording that no longer existed anywhere in the source — which is
# how the staleness was spotted at all. Nothing in that output looked like a
# failure: the suite said PASS, 16 checks, 0 failed.
#
# `--no-build` was justified here as "rebuilding the APK would not change the
# guest that was just staged". True, and beside the point — it changes the
# *observer*. This project already has the rule written down from the other
# direction: a tool that cannot see the thing under test returns a clean,
# confident, wrong answer.
#
# The full run installs and launches too, so no separate force-stop or `am start`
# is needed here — and that fresh process is what performs the copy.
ci/mobile/device-suite.sh || true

echo
echo "reload-guest: the line that answers this script's question"
# `|| true` on every grep: a grep that matches nothing is fatal under
# `set -o pipefail`, and matching nothing is one of the outcomes being reported.
# Three earlier scripts in this directory died exactly here, mid-report, and read
# like clean finishes.
adb logcat -d -s vieww 2>/dev/null | grep "dlopen_a_real_guest_library" | tail -1 || true
