#!/usr/bin/env bash
#
# Run the on-device suite on an attached Android phone and report pass/fail.
#
# Two halves, and the split is deliberate. The application asserts what must be
# true on *any* device — see `examples/shared/device_tests.rs` — and reports on
# `__VIEWW_SUITE__` lines. This script asserts what only somebody looking at the
# device can know: that a tap reaches a handler, and that a phone with a cutout
# reports one.
#
#   ci/mobile/device-suite.sh                   # build, install, run, verify
#   ci/mobile/device-suite.sh --expect-cutout   # also require non-zero safe-area insets
#   ci/mobile/device-suite.sh --expect-keyboard # also require the soft keyboard to inset
#   ci/mobile/device-suite.sh --no-build        # use whatever is already installed
#
# Needs adb, one device attached, and the Android SDK/NDK env the README names.

set -euo pipefail

package="dev.vieww.demo"
build=1
expect_cutout=0
expect_keyboard=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-build) build=0 ;;
        --expect-cutout) expect_cutout=1 ;;
        --expect-keyboard) expect_keyboard=1 ;;
        -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "device-suite: unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

command -v adb >/dev/null 2>&1 || { echo "device-suite: adb not found." >&2; exit 127; }
adb shell true >/dev/null 2>&1 || { echo "device-suite: no device reachable." >&2; adb devices -l >&2; exit 3; }

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

# shellcheck source=ci/mobile/android-env.sh
. "$root/ci/mobile/android-env.sh"

# Whether the app is up. Its own function because it is asked twice, and because
# `set -o pipefail` turns a bare `pid=$(... | ...)` on a missing process into a
# silent exit — the script dies before it can say what went wrong, which is the
# one thing a diagnostic script must never do.
running_pid() {
    adb shell pidof "$package" 2>/dev/null | tr -d '\r' | awk '{print $1}' || true
}

if [ "$build" -eq 1 ]; then
    # The SDK, the NDK and the release keystore, from the one place that knows
    # how to find them. This was sixty lines here, and `ci/mobile/reload-guest-android.sh`
    # and `ci/mobile/reload-android.sh` were later written without them — each
    # rediscovering the partial-NDK trap, the environment, and the keystore in
    # turn. See the header of ci/mobile/android-env.sh.
    android_env "device-suite" || exit $?

    # Release, not debug: a debug vello is ~20x slower, and every frame-time
    # assertion in the suite would then be measuring `rustc -O0`.
    echo "device-suite: building and installing (release)…"
    if ! cargo apk run -p vieww-platform-winit --example android --release; then
        # A non-zero status here is genuinely ambiguous. `cargo apk run`
        # installs, launches, and *then* dies attaching logcat when the device
        # has a second user profile — `String '10207,1010207' is not a UID` —
        # by which point the app is already running. But a compile or install
        # error lands here too, and treating that as success would run the suite
        # against whatever was installed last week.
        #
        # The process itself is the only honest arbiter.
        if [ -z "$(running_pid)" ]; then
            echo >&2
            echo "device-suite: the build or install failed — see the output above." >&2
            echo "  (If it had merely failed to attach logcat, the app would be running.)" >&2
            exit 7
        fi
        echo "device-suite: cargo apk failed after launching; continuing." >&2
    fi
fi

pid="$(running_pid)"
if [ -z "$pid" ]; then
    echo "device-suite: $package is not running." >&2
    echo "  Launch it first, or drop --no-build so this script installs it." >&2
    exit 4
fi
echo "device-suite: $package is running as pid $pid"

# Tap before the report goes out — it is emitted once, at 600 frames, and never
# revised. The app deliberately waits ten seconds partly to give this time; an
# earlier version reported at two and the tap never made it into a passing run.
#
# Where the button is comes from the app itself — see the tap-target wait below.
# It used to be a constant here, and a constant that suits one screen is a suite
# that only verifies one screen.
# A dark screen draws no frames, and the suite needs 600 of them.
adb shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true

size="$(adb shell wm size 2>/dev/null | tr -d '\r' | awk -F': ' '/Physical/ {print $2}')"
echo "device-suite: display is ${size:-unknown}"

# The activity has to be *drawing* before a tap means anything. `cargo apk run`
# returns the moment `am start` does, and the first frame on a phone can take
# half a second on its own — 480ms of shader compilation on an Adreno 612. A tap
# delivered into that gap goes to whatever is still on screen and is simply lost,
# which is exactly how a correct button produced a missing check on a
# device whose coordinates were right all along.
sleep 3

# So tap repeatedly rather than once. There is no point waiting for confirmation
# between taps: the app reports once, at the end, so the log says nothing about
# the tap until it is far too late to send another. Retrying blind is what makes
# this independent of how long a given phone takes to start, which is the thing
# the script cannot know. Extra taps cost nothing — the counter is meant to be
# tapped — and the app latches the first one it feels.
#
# ## Where to tap comes from the app, not from this file
#
# This used to be `tap_x=190; tap_y=536; field_y=798` — three constants correct
# for one 1080x2340 phone at 2.75x and silently wrong everywhere else, with a miss
# showing up as a check *absent* from the report rather than failing. That is not a
# rough edge in a cross-platform framework, it is a verification suite that only
# works on one handset, and it is why a second device could never have been added
# to the matrix: it would have reported a pass that checked less.
#
# The app now says where its own widgets ended up. `DeviceSuite::tap_targets`
# runs on `App::after_frame`, finds the button by its label and the field by its
# semantic role — the same way a screen reader would — and reports their centres in
# physical pixels. Nothing here knows a resolution or a density.
#
# **No fallback to constants.** If the app cannot say where to tap, this fails.
# Guessing would produce exactly the silent partial pass the constants did.
echo "device-suite: waiting for the app to report its tap targets…"
taps_line=""
taps_deadline=$(( $(date +%s) + 30 ))
while [ "$(date +%s)" -lt "$taps_deadline" ]; do
    taps_line="$(adb logcat -d --pid="$pid" 2>/dev/null | tr -d '\r' \
        | grep -o "__VIEWW_TAPS__ button=[0-9-]*,[0-9-]* field=[0-9-]*,[0-9-]*" \
        | tail -1 || true)"
    [ -n "$taps_line" ] && break
    if [ -z "$(running_pid)" ]; then
        echo "device-suite: the app died before reporting its tap targets." >&2
        adb logcat -d --pid="$pid" 2>/dev/null | tail -30 >&2
        exit 5
    fi
    sleep 1
done

if [ -z "$taps_line" ]; then
    echo "device-suite: the app never reported its tap targets." >&2
    echo "  Expected a logcat line like:" >&2
    echo "    __VIEWW_TAPS__ button=523,1474 field=523,2194" >&2
    echo "  emitted by DeviceSuite::tap_targets on App::after_frame. Without it" >&2
    echo "  this script does not know where anything is, and tapping a guessed" >&2
    echo "  coordinate is how a suite passes while checking nothing." >&2
    adb logcat -d --pid="$pid" 2>/dev/null | tail -30 >&2
    exit 7
fi

tap_x="${taps_line#*button=}"; tap_x="${tap_x%%,*}"
tap_y="${taps_line#*button=*,}"; tap_y="${tap_y%% *}"
field_x="${taps_line#*field=}"; field_x="${field_x%%,*}"
field_y="${taps_line#*field=*,}"
echo "device-suite: the app reports button ${tap_x},${tap_y} and field ${field_x},${field_y} (physical)"

taps=0
max_taps=6

# Poll rather than sleep. How long the app needs depends on the device, and a
# fixed sleep is either too short on a slow phone or wasted on a fast one.
deadline=$(( $(date +%s) + 45 ))
log=""
while [ "$(date +%s)" -lt "$deadline" ]; do
    if [ "$taps" -lt "$max_taps" ]; then
        adb shell input tap "$tap_x" "$tap_y" >/dev/null 2>&1 || true
        # And the field, so the keyboard comes up. Harmless if it misses, and
        # harmless if it lands: a focused field changes nothing else the suite
        # asserts on.
        adb shell input tap "$field_x" "$field_y" >/dev/null 2>&1 || true
        taps=$(( taps + 1 ))
    fi

    log="$(adb logcat -d --pid="$pid" 2>/dev/null | tr -d '\r' | grep "__VIEWW_SUITE__" || true)"
    if echo "$log" | grep -q "__VIEWW_SUITE__ result "; then
        break
    fi
    if [ -z "$(running_pid)" ]; then
        echo "device-suite: the app died before reporting." >&2
        adb logcat -d --pid="$pid" 2>/dev/null | tail -30 >&2
        exit 5
    fi
    sleep 2
done
echo "device-suite: sent $taps tap(s) each at ${tap_x},${tap_y} and ${field_x},${field_y}"

if ! echo "$log" | grep -q "__VIEWW_SUITE__ result "; then
    echo "device-suite: the app never finished reporting." >&2
    echo "  It needs 600 frames, so it must be drawing continuously — if the" >&2
    echo "  screen went to sleep or the app was backgrounded, it never gets" >&2
    echo "  there. Recent output:" >&2
    adb logcat -d --pid="$pid" 2>/dev/null | tail -30 >&2
    exit 5
fi

echo
echo "$log" | sed 's/.*__VIEWW_SUITE__ /  /'
echo

status=0

# The application's own verdict.
if ! echo "$log" | grep -q "__VIEWW_SUITE__ result PASS"; then
    echo "device-suite: FAIL — the in-app suite reported a failure." >&2
    status=1
fi

# A tap has to have reached a handler. This used to be blamed on the coordinates,
# and that excuse is gone: the app reported them itself, from its own laid-out
# semantics tree, so a miss now means something real — the tap was swallowed, the
# button was covered, or the input chain is broken.
if ! echo "$log" | grep -q "a_tap_reached_a_handler"; then
    echo "device-suite: FAIL — $taps taps at $tap_x,$tap_y reached no handler." >&2
    echo "  The app itself reported that coordinate from its semantics tree on a" >&2
    echo "  ${size:-unknown} display, so this is not a guess that missed." >&2
    echo "  Check what is actually on screen:" >&2
    echo "    adb exec-out screencap -p > /tmp/vieww.png" >&2
    status=1
fi

# Only when the operator says the device has a cutout, because a phone without
# one legitimately reports zeros and the app cannot tell the difference.
if [ "$expect_cutout" -eq 1 ]; then
    if echo "$log" | grep -q "safe_area: l0 t0 r0 b0"; then
        echo "device-suite: FAIL — --expect-cutout, but every inset is zero." >&2
        status=1
    fi
fi

# The one thing on this list that is not yet known to work at all. Off by
# default, because a run where the field tap missed reports exactly the same
# absence as a run where `WindowInsets.Type.ime()` came back empty, and only
# somebody watching the screen can tell those apart.
if [ "$expect_keyboard" -eq 1 ]; then
    if ! echo "$log" | grep -q "the_keyboard_inset_appears_when_the_keyboard_does"; then
        echo "device-suite: FAIL — the keyboard never showed up in the insets." >&2
        echo "  Either the tap at $tap_x,$field_y missed the text field, or" >&2
        echo "  getCurrentWindowMetrics() does not carry ime() insets on this" >&2
        echo "  device — which is the open question in src/insets.rs." >&2
        echo "  Watch the screen during the run: if the keyboard came up and" >&2
        echo "  this still failed, it is the second one, and the fallback is a" >&2
        echo "  View-level query marshalled onto the UI thread." >&2
        status=1
    fi
fi

if [ "$status" -eq 0 ]; then
    echo "device-suite: PASS"
fi
exit "$status"
