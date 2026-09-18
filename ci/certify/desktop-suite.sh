#!/usr/bin/env bash
#
# Run the on-desktop suite on this machine and report pass/fail.
#
# Two halves, and the split is the one `ci/mobile/device-suite.sh` already uses. The
# application asserts what must be true on *any* desktop — see
# `crates/vieww-platform-winit/examples/shared/desktop_tests.rs` — and reports on
# `__VIEWW_SUITE__` lines. This script asserts what only somebody sitting at the
# machine can know: that there really are two displays, that this one really is
# HiDPI, and that the pixels came from a driver rather than from a software
# fallback.
#
#   ci/certify/desktop-suite.sh                      # build, run, verify
#   ci/certify/desktop-suite.sh --expect-multi-monitor
#   ci/certify/desktop-suite.sh --expect-hidpi       # require a density above 1
#   ci/certify/desktop-suite.sh --no-build           # use whatever is already built
#   ci/certify/desktop-suite.sh --debug              # skip the release build
#   ci/certify/desktop-suite.sh --headless           # under Xvfb, for a Linux CI box
#
# Run it on **Linux, macOS and Windows**, the same three
# `docs/RELEASE-CHECK.md` asks for, and for a complementary reason: that script
# drives the studio's UI and photographs it, and this one asks the platform layer
# underneath whether it is behaving. A control that is reachable in a screenshot
# on a Mac says nothing about whether the pasteboard works there.
#
# On Windows, run it from Git Bash — the shell `ci/*.sh` has always assumed.
#
# # Prerequisites, and the one that surprises people
#
# A Rust toolchain, and a **Vulkan loader with a driver**. The rasterising is
# vieww's own and runs on the CPU, so it is tempting to assume a window needs
# nothing from the GPU — but `NativeRenderer::for_window` presents through
# `vieww_hal::vulkan`, so a machine with no ICD fails at startup with
# `Unable to find a Vulkan driver` before a single check runs. On a headless
# Linux box `mesa-vulkan-drivers` (lavapipe) is enough and is what `--headless`
# expects; see `PENDING.md` §2.6 for why a *GPU-rendered* frame is still a
# separate, unclosed question.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

build=1
profile="--release"
headless=0
expect_multi_monitor=0
expect_hidpi=0
# Long enough for the in-app report, which lands at frame 600 — ten seconds at
# 60Hz, and longer on a machine that is slower than its refresh rate. The run is
# stopped the moment the verdict line appears, so this is a ceiling and not a
# wait.
timeout_seconds=90
# Where the app's own output goes. Kept after the run (it is the only record of
# a crash), default under target/.
keep_log=""

while [ "$#" -gt 0 ]; do
	case "$1" in
	--no-build) build=0 ;;
	--debug) profile="" ;;
	--headless) headless=1 ;;
	--expect-multi-monitor) expect_multi_monitor=1 ;;
	--expect-hidpi) expect_hidpi=1 ;;
	--log)
		keep_log="${2:?--log needs a file}"
		shift
		;;
	--timeout)
		timeout_seconds="${2:?--timeout needs seconds}"
		shift
		;;
	-h | --help)
		sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*)
		echo "desktop-suite: unknown argument: $1" >&2
		exit 2
		;;
	esac
	shift
done

command -v cargo >/dev/null 2>&1 || {
	echo "desktop-suite: cargo not found." >&2
	exit 127
}

case "$(uname -s)" in
Darwin) platform="macos" ;;
Linux) platform="linux" ;;
MINGW* | MSYS* | CYGWIN*) platform="windows" ;;
*) platform="$(uname -s | tr '[:upper:]' '[:lower:]')" ;;
esac

# ------------------------------------------------------------------ the machine
#
# Asked before the app runs, so that a failure to *start* still leaves the
# operator with the description of the machine it would not start on. A blank
# answer here is not an error: every one of these is a nice-to-have, and a
# missing `xrandr` is not a reason to refuse to test.

monitors=""
scales=""
case "$platform" in
linux)
	if [ -n "${DISPLAY:-}" ] && command -v xrandr >/dev/null 2>&1; then
		monitors="$(xrandr --listmonitors 2>/dev/null | sed -n 's/^Monitors: \([0-9]*\).*/\1/p')"
	fi
	;;
macos)
	if command -v system_profiler >/dev/null 2>&1; then
		monitors="$(system_profiler SPDisplaysDataType 2>/dev/null | grep -c 'Resolution:' || true)"
		# "Retina" is the user-facing word for a backing scale above 1, and it is
		# what `system_profiler` prints; the app reports the number itself.
		scales="$(system_profiler SPDisplaysDataType 2>/dev/null | grep -c 'Retina' || true)"
	fi
	;;
windows)
	if command -v powershell >/dev/null 2>&1; then
		monitors="$(powershell -NoProfile -Command \
			'(Get-CimInstance -ClassName Win32_VideoController | Measure-Object).Count' 2>/dev/null | tr -d '\r')"
	fi
	;;
esac

echo "desktop-suite: $platform, $(uname -m)"
[ -n "$monitors" ] && echo "desktop-suite: the OS reports $monitors display(s)"
[ -n "$scales" ] && echo "desktop-suite: $scales of them look HiDPI to the OS"

# ------------------------------------------------------------------------- run

if [ "$build" -eq 1 ]; then
	# shellcheck disable=SC2086 # $profile is a flag or empty, and must word-split.
	cargo build $profile -p vieww-platform-winit --example desktop
fi

runner=()
if [ "$headless" -eq 1 ]; then
	command -v xvfb-run >/dev/null 2>&1 || {
		echo "desktop-suite: --headless needs xvfb-run." >&2
		exit 127
	}
	# A real size rather than the 640x480 default: the demo asks for an 880-tall
	# window, and a virtual screen shorter than that measures a window the WM
	# squashed rather than the one the suite meant to open.
	runner=(xvfb-run -a --server-args="-screen 0 1280x1024x24")
elif [ "$platform" = "linux" ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
	echo "desktop-suite: no DISPLAY and no WAYLAND_DISPLAY." >&2
	echo "  This suite opens real windows. On a headless box use --headless." >&2
	exit 3
fi

# The log is evidence, not scratch: a segfault's only trace is in it, and the
# previous `mktemp` + `rm` on exit threw that away.
log="${keep_log:-$root/target/desktop-suite/app-$(date +%Y%m%dT%H%M%S).log}"
mkdir -p "$(dirname "$log")"
: >"$log"
echo "desktop-suite: app output -> $log"

binary="target/release/examples/desktop"
[ -n "$profile" ] || binary="target/debug/examples/desktop"
[ -x "$binary" ] || {
	echo "desktop-suite: $binary is not built. Drop --no-build." >&2
	exit 3
}

echo "desktop-suite: running (up to ${timeout_seconds}s; a window will open)"

# `timeout` is the ceiling and not the plan. The app owns its window until
# somebody closes it, and nothing here can click its close button — so the run is
# ended by this script once the report is out, and the ceiling only matters when
# the report never comes, which is itself the answer to "did it hang".
#
# Backgrounded with its output to a file, rather than piped: a pipeline's exit
# status belongs to the reader, and `set -o pipefail` on a `head` that stops
# early would kill this script instead of the app.
${runner[@]+"${runner[@]}"} "$binary" >"$log" 2>&1 &
app=$!

waited=0
while [ "$waited" -lt "$timeout_seconds" ]; do
	if grep -q '__VIEWW_SUITE__ result' "$log" 2>/dev/null; then
		break
	fi
	if ! kill -0 "$app" 2>/dev/null; then
		break
	fi
	sleep 1
	waited=$((waited + 1))
done

# `kill` the process group's leader and let it go; `wait` swallows the status,
# which is meaningless here — the verdict is on the sentinel line, for the reason
# `ci/mobile/device-suite.sh`'s header gives about exit codes that have to survive a
# transport.
kill "$app" 2>/dev/null || true
app_status=0
wait "$app" 2>/dev/null || app_status=$?

echo
sed -n 's/^__VIEWW_SUITE__ //p' "$log" || true
echo

if ! grep -q '__VIEWW_SUITE__ result' "$log"; then
	echo "desktop-suite: FAIL — no report after ${timeout_seconds}s." >&2
	if [ "$app_status" -gt 128 ] && [ "$app_status" -ne 143 ]; then
		echo "  The app died on signal $((app_status - 128)) ($(kill -l "$((app_status - 128))" 2>/dev/null))." >&2
		echo "  For a backtrace: gdb -batch -ex run -ex bt --args $binary" >&2
	fi
	echo "  The last thing it said (full log: $log):" >&2
	tail -40 "$log" >&2
	exit 1
fi

failures=0
grep -q '__VIEWW_SUITE__ result PASS' "$log" || {
	echo "desktop-suite: the in-app suite reported failures." >&2
	failures=1
}

# --------------------------------------------------- what only the operator knows
#
# Each of these is asserted *only* when asked for, because the honest default on
# an unknown machine is silence. A suite that fails on a laptop with one screen
# teaches people to ignore it, which is the failure `device_tests.rs` opens by
# warning about.

density="$(sed -n 's/.*got [0-9.]*x[0-9.]* logical at \([0-9.]*\)x.*/\1/p' "$log" | head -1)"
[ -n "$density" ] && echo "desktop-suite: the window reported a density of ${density}x"

if [ "$expect_hidpi" -eq 1 ]; then
	if [ -z "$density" ]; then
		echo "desktop-suite: FAIL — --expect-hidpi, but the app never reported a density." >&2
		failures=1
	elif awk "BEGIN{exit !($density > 1)}"; then
		echo "desktop-suite: ok — HiDPI, as expected (${density}x)"
	else
		echo "desktop-suite: FAIL — --expect-hidpi, but the window is at ${density}x." >&2
		echo "  Either this display is not the HiDPI one, or the scale factor is not" >&2
		echo "  reaching ViewMetrics. Check which window the app opened on." >&2
		failures=1
	fi
fi

if [ "$expect_multi_monitor" -eq 1 ]; then
	if [ -z "$monitors" ]; then
		echo "desktop-suite: FAIL — --expect-multi-monitor, but this platform's" >&2
		echo "  display query found nothing to count." >&2
		failures=1
	elif [ "$monitors" -gt 1 ]; then
		echo "desktop-suite: ok — $monitors displays, as expected"
	else
		echo "desktop-suite: FAIL — --expect-multi-monitor, but the OS reports $monitors." >&2
		failures=1
	fi
fi

if [ "$failures" -eq 0 ]; then
	echo "desktop-suite: PASS"
else
	echo "desktop-suite: FAIL" >&2
fi
exit "$failures"
