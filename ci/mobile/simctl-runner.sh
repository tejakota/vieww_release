#!/usr/bin/env bash
#
# Cargo target runner for `aarch64-apple-ios-sim`.
#
# Cargo invokes a runner as `<runner> <binary> [harness args...]`, so all this
# has to do is hand the test binary to a simulator and get the exit status back.
# `xcrun simctl spawn` runs a Mach-O binary inside the simulator runtime, which
# is exactly what a libtest binary built for the sim target is.
#
# Wired up in `.cargo/config.toml`. See docs/DESIGN.md §10.
#
# Requires macOS with Xcode, and a booted simulator — the CI job boots one and
# exports its UDID as VIEWW_SIM_DEVICE. Falls back to whatever is already
# booted, which is what you want when running this by hand on a Mac.

set -euo pipefail

if ! command -v xcrun >/dev/null 2>&1; then
    echo "simctl-runner: xcrun not found." >&2
    echo "  iOS simulator tests need macOS with Xcode installed." >&2
    echo "  On Linux, cross-compile only: cargo build --target aarch64-apple-ios" >&2
    exit 127
fi

if [ "$#" -lt 1 ]; then
    echo "simctl-runner: expected a test binary, got no arguments." >&2
    exit 2
fi

binary="$1"
shift

device="${VIEWW_SIM_DEVICE:-booted}"

if [ "$device" = "booted" ] && ! xcrun simctl list devices booted | grep -q Booted; then
    echo "simctl-runner: no simulator is booted and VIEWW_SIM_DEVICE is unset." >&2
    echo "  Boot one first:  xcrun simctl boot 'iPhone 16'" >&2
    exit 3
fi

# `simctl spawn` drops the ambient environment; only variables prefixed with
# SIMCTL_CHILD_ reach the spawned process, with the prefix stripped. Forward the
# few that change what a Rust test prints on failure.
export SIMCTL_CHILD_RUST_BACKTRACE="${RUST_BACKTRACE:-1}"
[ -n "${RUST_LOG:-}" ] && export SIMCTL_CHILD_RUST_LOG="$RUST_LOG"
[ -n "${RUST_TEST_THREADS:-}" ] && export SIMCTL_CHILD_RUST_TEST_THREADS="$RUST_TEST_THREADS"

# `exec` so the spawned process's exit status becomes this script's, which is
# what cargo reads to decide whether the test binary passed.
exec xcrun simctl spawn "$device" "$binary" "$@"
