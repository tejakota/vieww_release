#!/usr/bin/env bash
#
# Cargo target runner for the Android targets.
#
# Cargo invokes a runner as `<runner> <binary> [harness args...]`. This pushes
# the test binary to the device with adb, runs it, streams its output, and
# propagates its exit status.
#
# Wired up in `.cargo/config.toml`. See docs/DESIGN.md §10.
#
# Requires adb on PATH and exactly one device or emulator attached.

set -euo pipefail

if ! command -v adb >/dev/null 2>&1; then
    echo "adb-runner: adb not found." >&2
    echo "  Android tests need platform-tools and an attached device/emulator." >&2
    echo "  Without one, cross-compile only: cargo build --target aarch64-linux-android" >&2
    exit 127
fi

if [ "$#" -lt 1 ]; then
    echo "adb-runner: expected a test binary, got no arguments." >&2
    exit 2
fi

if ! adb shell true >/dev/null 2>&1; then
    echo "adb-runner: no device is reachable." >&2
    echo "  Attached:" >&2
    adb devices -l >&2
    exit 3
fi

binary="$1"
shift

# An ABI mismatch is the likeliest way to get here with everything else set up
# correctly — a phone plugged in while the build targets the CI emulator, or the
# reverse. The device's own error for it is `not executable: 64-bit ELF file`,
# which names neither architecture and reads like a corrupt push.
#
# The binary's architecture is read from its ELF header rather than inferred
# from its path. A doctest is built into a temp directory rather than under
# `target/<triple>/`, so the path is not a proxy for anything — checking it
# rejects a perfectly good cross-compiled doctest, which is a worse failure than
# the one this guard exists to catch.
#
# `e_machine` is two bytes at offset 18, in the ELF's own byte order. Every host
# this runs on is little-endian, as is every Android target, so `od`'s native
# order is the right one.
elf_machine="$(od -An -tu2 -j18 -N2 "$binary" 2>/dev/null | tr -d '[:space:]')"
device_abi="$(adb shell getprop ro.product.cpu.abi | tr -d '\r')"
case "$device_abi" in
    arm64*) want_machine=183; want_target="aarch64" ;;   # EM_AARCH64
    x86_64) want_machine=62;  want_target="x86_64" ;;    # EM_X86_64
    armeabi*) want_machine=40; want_target="arm" ;;      # EM_ARM
    x86) want_machine=3; want_target="i686" ;;           # EM_386
    *) want_machine=""; want_target="" ;;
esac

# Only complain when both are known and they genuinely disagree. An unreadable
# header or an unrecognised ABI means we do not know, and guessing would block a
# run that would have worked.
if [ -n "$want_machine" ] && [ -n "$elf_machine" ] && [ "$elf_machine" != "$want_machine" ]; then
    linker_var="CARGO_TARGET_$(echo "$want_target" | tr '[:lower:]' '[:upper:]')_LINUX_ANDROID_LINKER"
    echo "adb-runner: the device is $device_abi, but this binary is not." >&2
    echo "  $binary" >&2
    echo "  Build for it instead: cargo test --target $want_target-linux-android" >&2
    echo "  (and set $linker_var to the NDK clang)" >&2
    exit 4
fi

remote_dir="/data/local/tmp/vieww"
remote="$remote_dir/$(basename "$binary").$$"

cleanup() { adb shell "rm -f '$remote'" >/dev/null 2>&1 || true; }
trap cleanup EXIT

adb shell "mkdir -p '$remote_dir'" >/dev/null
adb push "$binary" "$remote" >/dev/null
adb shell "chmod 755 '$remote'" >/dev/null

# Single-quote each argument so that test filters containing spaces or globs
# survive the trip through the device's shell.
quote() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

command_line="'$remote'"
for arg in "$@"; do
    command_line="$command_line $(quote "$arg")"
done

# `adb shell` exit-status propagation depends on the adb/device version, and a
# silently-zero status would report a failing suite as green — the worst
# possible failure mode. So the status is carried back in-band on a sentinel
# line and re-raised here, rather than trusted from adb.
#
# The leading `echo` guarantees the sentinel starts its own line even when the
# test harness leaves output without a trailing newline.
remote_script="cd '$remote_dir' && $command_line; __status=\$?; echo; echo __VIEWW_EXIT__\$__status"

# `tr` strips the CRs the Android shell adds. awk swallows the sentinel, passes
# everything else through as it arrives, and exits with the captured status.
adb shell "RUST_BACKTRACE=${RUST_BACKTRACE:-1} ${RUST_LOG:+RUST_LOG=$RUST_LOG} sh -c \"$remote_script\"" \
    | tr -d '\r' \
    | awk '
        BEGIN { status = 255; seen = 0 }
        /^__VIEWW_EXIT__/ {
            status = $0
            sub(/^__VIEWW_EXIT__/, "", status)
            seen = 1
            next
        }
        { print }
        END {
            if (!seen) {
                print "adb-runner: no exit sentinel — the process died before reporting." > "/dev/stderr"
                exit 255
            }
            exit status + 0
        }
    '
