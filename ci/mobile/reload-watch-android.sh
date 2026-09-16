#!/usr/bin/env bash
#
# Watch examples/reload-guest and push every edit to the running phone.
#
#   ci/mobile/reload-watch-android.sh                 # push once, then watch until Ctrl-C
#   ci/mobile/reload-watch-android.sh --once          # push once and stop
#   ci/mobile/reload-watch-android.sh --interval 250  # how often to look, in ms
#   ci/mobile/reload-watch-android.sh --no-initial    # watch only; do not push on start
#
# This is the outer loop of the Android hot-reload cycle. The inner half is
# ci/mobile/reload-android.sh, which builds, installs and launches the host — **start
# that first, in another terminal.** This script drives the host that is
# already running and never installs anything.
#
# Every rebuild goes through ci/mobile/reload-guest-android.sh, which owns the NDK, the
# staging paths and the two-directory dance. Nothing about any of that is
# repeated here, on the rule this directory has already paid for twice: a script
# written by copying prose from another script inherits none of its fixes.
#
# Needs adb and cargo-ndk, both checked by the script this one calls.

set -euo pipefail

# Only the guest. **Never the framework.**
#
# `reload-guest` depends on `vieww-widget` with `hot-reload` on, which is the
# feature that changes `WidgetNode`'s layout — so a guest built from edited
# framework sources and pushed into a host APK built from the old ones is not a
# reload, it is two different definitions of the same struct in one address
# space. That mismatch has already happened here once; it compiled perfectly and
# was undefined behaviour. Editing anything under crates/ means reinstalling the
# host with ci/mobile/reload-android.sh, and this script deliberately cannot do it for
# you.
watched="examples/reload-guest"

package="dev.vieww.reload"
tag="vieww-reload"
interval_ms=500
once=0
initial=1

while [ "$#" -gt 0 ]; do
    case "$1" in
        --once) once=1 ;;
        --no-initial) initial=0 ;;
        --interval)
            shift
            [ "$#" -gt 0 ] || { echo "reload-watch: --interval needs a value in ms" >&2; exit 2; }
            case "$1" in
                ''|*[!0-9]*) echo "reload-watch: --interval takes milliseconds: $1" >&2; exit 2 ;;
            esac
            interval_ms="$1"
            ;;
        -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "reload-watch: unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

[ -d "$watched" ] || { echo "reload-watch: $watched is not there." >&2; exit 2; }

command -v adb >/dev/null 2>&1 || { echo "reload-watch: adb not found." >&2; exit 127; }
adb shell true >/dev/null 2>&1 || { echo "reload-watch: no device reachable." >&2; adb devices -l >&2; exit 3; }

interval="$(awk "BEGIN { printf \"%.3f\", $interval_ms / 1000 }")"

# **The host has to be running, and this is the check that says so.**
#
# Pushing at a package that is installed but not running succeeds at every step
# — the file lands, the size verifies — and nothing reloads, because the thing
# that copies the library in and calls `dlopen` is a live process. The watcher
# would sit there reporting successful pushes for ever. That is this project's
# oldest recurring failure in a new place: a tool that cannot observe the thing
# under test returns a clean, confident, wrong answer.
# `|| true` because **"not running" is the answer, not an error.** `pidof` exits
# non-zero when it finds nothing, and under `set -o pipefail` that took the
# whole script down at the assignment below — before the check that prints the
# advice, so the first version of this exited 1 in silence on the one case it
# was written to explain.
host_pid() {
    adb shell pidof "$package" 2>/dev/null | tr -d '\r' | awk '{print $1}' || true
}

pid="$(host_pid)"
if [ -z "$pid" ]; then
    echo "reload-watch: $package is not running on this device." >&2
    echo "  It is the half that loads what this script pushes. Start it first:" >&2
    echo "      ci/mobile/reload-android.sh" >&2
    exit 4
fi

# What the sources look like right now: path, mtime and size, one file per line.
#
# **`%.9Y` — nanoseconds — and not `%Y`.** This is the difference between a
# watcher that works and one that ignores the first thing anybody tries.
#
# `%Y` is whole seconds, so a fingerprint of second-plus-size cannot see an edit
# that keeps the byte count inside the same second. That is not a corner case
# here: `reload-guest`'s own documentation opens by telling you to change `BAND`
# to another colour, and one colour constant is usually the same length as the
# next. Written with `%Y` first, and the test that pinned it — changing
# `= 1;` to `= 2;` — produced a byte-identical fingerprint and no rebuild.
#
# Size stays because it costs nothing and catches a truncated write.
fingerprint() {
    find "$watched" -type f \( -name '*.rs' -o -name 'Cargo.toml' \) \
        -exec stat -c '%n %.9Y %s' {} + 2>/dev/null | sort
}

# The paths that differ between two fingerprints.
#
# `|| true` for the same reason `host_pid` needs it, one step further on: `diff`
# exits 1 when the files differ, which here is not a failure but the entire
# reason the function was called.
changed_paths() {
    diff <(printf '%s\n' "$1") <(printf '%s\n' "$2") 2>/dev/null |
        awk '/^>/ { print $2 }' | sort -u || true
}

# The device's clock, in the format `logcat -T` wants.
#
# Read from the device rather than from here: logcat stamps lines with the
# phone's clock, and a laptop that is a few seconds ahead would cut off the
# verdict it is waiting for.
# The whole command is one quoted argument on purpose. `adb shell` hands its
# arguments to a shell on the device, so writing this the natural way —
# `adb shell date '+%m-%d %H:%M:%S.000'` — has the *local* shell eat the quotes
# and the phone receive `date` with two arguments, which it rejects with
# `date: Max 1 argument`. That failure goes to stderr and the cursor comes back
# empty, which `logcat -T` then rejects in turn: every push would have waited
# the full ten seconds and reported that the host said nothing, on a reload that
# had already worked. Measured on the attached device on 2026-08-14.
device_now() {
    adb shell "date '+%m-%d %H:%M:%S.000'" 2>/dev/null | tr -d '\r' || true
}

# The generation number of the last reload the host reported, or empty.
#
# The second half of "is this line new". `logcat -T` rounds down to the second,
# so a verdict from just before the push can still be inside the window; the
# generation number is what tells the two apart, because it only ever goes up.
last_generation() {
    adb logcat -d -s "$tag" 2>/dev/null |
        grep -o 'reloaded #[0-9]\+' | tail -1 | grep -o '[0-9]\+' || true
}

# Wait for the host to say what it made of the push.
#
# Three outcomes, all of them reported, because "pushed" is not an outcome: the
# state survived, the state changed shape and the tree was rebuilt, or the guest
# would not load and the phone is still running the previous build.
await_verdict() {
    _since="$1"
    _was="$2"
    # A real deadline off the shell's own clock, not a count of sleeps. Each
    # pass also shells out to adb, so "100 × 0.1s" would have been somewhere
    # north of twenty seconds while the message claimed ten.
    _deadline=$((SECONDS + 10))

    while [ "$SECONDS" -lt "$_deadline" ]; do
        _lines="$(adb logcat -d -T "$_since" -s "$tag" 2>/dev/null || true)"

        _reload="$(printf '%s\n' "$_lines" | grep 'reloaded #' | tail -1 || true)"
        if [ -n "$_reload" ]; then
            _gen="$(printf '%s\n' "$_reload" | grep -o 'reloaded #[0-9]\+' | grep -o '[0-9]\+' || true)"
            if [ "$_gen" != "$_was" ]; then
                case "$_reload" in
                    *'CHANGED SHAPE'*)
                        echo "reload-watch: reloaded #$_gen — state changed shape, tree rebuilt"
                        ;;
                    *) echo "reload-watch: reloaded #$_gen — state kept" ;;
                esac
                return 0
            fi
        fi

        if printf '%s\n' "$_lines" | grep -q 'the first guest arrived'; then
            echo "reload-watch: the first guest arrived — the window is live"
            return 0
        fi

        _kept="$(printf '%s\n' "$_lines" | grep 'keeping the running build' | tail -1 || true)"
        if [ -n "$_kept" ]; then
            echo "reload-watch: the host refused it and kept the running build."
            echo "  ${_kept#*: }"
            return 1
        fi

        sleep 0.1
    done

    # Ten seconds against a host that polls every 250ms. Not "probably fine".
    echo "reload-watch: pushed, and the host said nothing about it in 10s." >&2
    echo "  It may have died — check with: adb logcat -d -s $tag" >&2
    return 1
}

# One rebuild, one push, one answer.
sync_once() {
    _pid="$(host_pid)"
    if [ -z "$_pid" ]; then
        echo "reload-watch: $package is no longer running — start it again with ci/mobile/reload-android.sh" >&2
        return 1
    fi

    _since="$(device_now)"
    # **An empty cursor is not a harmless empty cursor.** `logcat -T ""` does not
    # fail; it prints `WARNING: -T invalid, setting to 1` and replays the buffer
    # from the beginning, so the verdict would be read out of ancient history —
    # a green "the first guest arrived" from some previous session, reported
    # against a push that may not have loaded at all. Refuse instead.
    if [ -z "$_since" ]; then
        echo "reload-watch: could not read the clock off the device; not pushing blind." >&2
        echo "  Check: adb shell \"date '+%m-%d %H:%M:%S.000'\"" >&2
        return 1
    fi
    _was="$(last_generation)"

    # Tested rather than allowed to fail: a compile error in the guest is the
    # most ordinary thing that happens in an edit loop, and it must not take the
    # watcher down with it. The phone carries on running the last good build,
    # which is exactly what someone mid-edit wants.
    if ! ci/mobile/reload-guest-android.sh --package "$package"; then
        echo "reload-watch: the build did not produce a library; nothing was pushed."
        return 1
    fi

    await_verdict "$_since" "$_was"
}

echo "reload-watch: watching $watched, every ${interval_ms}ms"
echo "reload-watch: host $package is up (pid $pid)"
echo

previous="$(fingerprint)"

if [ "$initial" -eq 1 ]; then
    echo "reload-watch: pushing what is on disk now, so the loop starts from a known state"
    sync_once || true
    echo
    if [ "$once" -eq 1 ]; then
        exit 0
    fi
elif [ "$once" -eq 1 ]; then
    echo "reload-watch: --once with --no-initial has nothing to do." >&2
    exit 2
fi

while :; do
    sleep "$interval"
    current="$(fingerprint)"
    [ "$current" = "$previous" ] && continue

    # Settle before building. An editor writes a save as several operations, and
    # a formatter on save is a second burst a moment later — building into the
    # middle of that compiles half a file and reports an error that has already
    # stopped being true.
    while :; do
        sleep "$interval"
        settled="$(fingerprint)"
        [ "$settled" = "$current" ] && break
        current="$settled"
    done

    for path in $(changed_paths "$previous" "$current"); do
        echo "reload-watch: $path"
    done

    previous="$current"
    sync_once || true
    echo
done
