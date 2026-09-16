# shellcheck shell=bash
#
# Disk-lean builds for every check, certification and release script.
#
#   . "$root/ci/lib/lean.sh"; vieww_lean_env
#
# # Why
#
# A full `ci/vieww gate` compiles the workspace in debug (tests, clippy, docs),
# in release (suites, Studio, feature examples, installers), a second release
# tree with a static std (`vieww-standard`'s allocation counter), an MSRV tree
# and three throwaway guest builds. With full debuginfo and incremental caches
# that needed around 100 GB on a beta tester's laptop, and the linker died with SIGBUS when
# the disk filled.
#
# Nearly all of that is debuginfo and incremental state, which no check reads:
#
# * `CARGO_INCREMENTAL=0` — incremental caches are per-crate copies of the
#   compiler's state, only useful for the *next* edit-compile cycle.
# * `CARGO_PROFILE_*_DEBUG=0` — no DWARF. Panics still name functions (symbols
#   are kept); only file:line in backtraces goes. Measurements are unaffected.
#
# Set `VIEWW_FULL_DEBUG=1` to keep both, e.g. to debug a crash from the gate.
#
# The environment only changes cargo's defaults, so a variable you already
# exported wins.
#
# `vieww_prune DIR...` deletes build directories under target/ once the stages
# that needed them are done — only in lean mode, and never outside target/.

vieww_lean_env() {
	if [ "${VIEWW_FULL_DEBUG:-0}" = 1 ]; then
		export VIEWW_LEAN=0
		return 0
	fi
	export VIEWW_LEAN=1
	export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
	export CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG:-0}"
	export CARGO_PROFILE_TEST_DEBUG="${CARGO_PROFILE_TEST_DEBUG:-0}"
	export CARGO_PROFILE_RELEASE_DEBUG="${CARGO_PROFILE_RELEASE_DEBUG:-0}"
	export CARGO_PROFILE_BENCH_DEBUG="${CARGO_PROFILE_BENCH_DEBUG:-0}"
}

# vieww_prune ROOT DIR... — rm -rf ROOT/target/DIR for each DIR, in lean mode.
vieww_prune() {
	local root="$1"
	shift
	[ "${VIEWW_LEAN:-0}" = 1 ] || return 0
	[ -n "$root" ] && [ -d "$root/target" ] || return 0
	local dir
	for dir in "$@"; do
		case "$dir" in
		"" | /* | *..*) continue ;;
		esac
		rm -rf "${root:?}/target/${dir:?}"
	done
}

# vieww_disk ROOT — "target=<GB> free=<GB>", for logs.
vieww_disk() {
	local root="$1" used free
	used=$(du -sk "$root/target" 2>/dev/null | awk '{printf "%.1f", $1 / 1048576}')
	free=$(df -Pk "$root" 2>/dev/null | awk 'NR==2 {printf "%.1f", $4 / 1048576}')
	echo "target=${used:-0}GB free=${free:-?}GB"
}
