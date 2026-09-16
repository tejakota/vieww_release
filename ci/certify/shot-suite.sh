#!/usr/bin/env bash
#
# Photograph every feature example on this machine, so another machine's set can
# be compared against it.
#
#   ci/certify/shot-suite.sh                          # capture into out/shots/<platform>-<stamp>
#   ci/certify/shot-suite.sh --out DIR                # capture somewhere specific
#   ci/certify/shot-suite.sh --against DIR            # capture, then diff against DIR
#   ci/certify/shot-suite.sh --only rectangle,flex    # a subset, matched on the package name
#   ci/certify/shot-suite.sh --debug                  # skip the release build
#
# # Why this exists
#
# `examples/features/` already renders headless — `VIEWW_SHOT=dir cargo run -p
# feature-rectangle` has worked since the harness was written. What did not
# exist was a reason to believe two machines produce the same pictures, and the
# gap between those two is not a small one: the fifty-six examples were run one
# at a time, by hand, on one Linux box, and nothing has ever compared a Windows
# run to a macOS run at all.
#
# `docs/RELEASE-CHECK.md` is where the claim being tested is written down: the
# pixels come from vieww's own CPU rasterizer rather than from asking the OS to
# draw anything, so the same scene **must** produce the same bytes everywhere.
# That is a strong claim and it is either true or it is a bug — there is no
# threshold of acceptable platform noise to argue about, which is what makes an
# exact comparison the right one and what makes a difference worth stopping for.
#
# # What it captures beyond the pictures
#
# `manifest.txt` records what this machine *was* — OS, architecture, libc,
# toolchain, and the fonts on the system font path. That last one is not
# decoration: font fallback is by a wide margin the most likely reason two
# platforms disagree, and a report saying "these 340 pixels differ" is answered
# in one line by two manifests listing different font files. Without it the same
# answer costs an afternoon.
#
# The per-shot counter lines the harness prints (`... -> 12 shapes, 3 glyph runs
# (41 glyphs), 1 clips, 0 layers`) are captured into `counters.txt` for the same
# reason. They are the non-pixel half of the evidence: two machines whose images
# differ but whose counters agree built the same scene and rasterised it
# differently, and two whose counters differ did not build the same scene at all.
# Those are different bugs in different layers, and the counters tell them apart
# before anybody opens an image.
#
# Needs: a Rust toolchain. No display, no GPU, no network.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

out=""
against=""
only=""
profile="--release"

while [ "$#" -gt 0 ]; do
	case "$1" in
	--out)
		out="${2:?--out needs a directory}"
		shift
		;;
	--against)
		against="${2:?--against needs a directory}"
		shift
		;;
	--only)
		only="${2:?--only needs a comma-separated list}"
		shift
		;;
	--debug) profile="" ;;
	-h | --help)
		sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
		exit 0
		;;
	*)
		echo "shot-suite: unknown argument: $1" >&2
		exit 2
		;;
	esac
	shift
done

command -v cargo >/dev/null 2>&1 || {
	echo "shot-suite: cargo not found." >&2
	exit 127
}

# The platform stamp, and it goes in the directory name rather than only in the
# manifest so that two capture directories sitting side by side can be told
# apart without opening either. `uname -s` is `Darwin`, `Linux`, or one of
# `MINGW64_NT-10.0` / `MSYS_NT-10.0` under Git Bash, which is the shell
# `docs/RELEASE-CHECK.md` already tells Windows operators to use.
case "$(uname -s)" in
Darwin) platform="macos" ;;
Linux) platform="linux" ;;
MINGW* | MSYS* | CYGWIN*) platform="windows" ;;
*) platform="$(uname -s | tr '[:upper:]' '[:lower:]')" ;;
esac
arch="$(uname -m)"
stamp="$(date +%Y%m%d-%H%M%S)"

[ -n "$out" ] || out="out/shots/$platform-$arch-$stamp"
mkdir -p "$out"

# Every feature example, by package name, read from the manifests rather than
# derived from the directory names — `00-rectangle` is the directory and
# `feature-rectangle` is the package, and a script that guesses the mapping
# breaks the first time somebody adds one that does not follow it.
packages=()
for manifest in examples/features/*/Cargo.toml; do
	case "$manifest" in
	examples/features/harness/*) continue ;;
	esac
	name="$(sed -n 's/^name = "\(.*\)"/\1/p' "$manifest" | head -1)"
	[ -n "$name" ] || continue
	if [ -n "$only" ]; then
		keep=0
		# `--only rectangle,flex` keeps anything whose package name contains one
		# of those, so a person debugging one example does not have to know
		# whether it is `feature-flex-row` or `feature-row-flex`.
		IFS=',' read -ra wanted <<<"$only"
		for want in "${wanted[@]}"; do
			case "$name" in *"$want"*) keep=1 ;; esac
		done
		[ "$keep" -eq 1 ] || continue
	fi
	packages+=("$name")
done

[ "${#packages[@]}" -gt 0 ] || {
	echo "shot-suite: no examples matched." >&2
	exit 2
}

echo "shot-suite: $platform-$arch, ${#packages[@]} example(s) -> $out"

# One build for all of them, before the loop. Fifty-six `cargo run`s each do
# their own dependency resolution and their own linker invocation; building once
# turns a run that took minutes into one that takes seconds, and — more to the
# point — means a compile error stops the run before any shot is written rather
# than half way through, leaving a directory that looks like a capture and is a
# fragment of one.
# shellcheck disable=SC2086 # $profile is a flag or empty, and must word-split.
cargo build $profile "${packages[@]/#/--package=}"

counters="$out/counters.txt"
: >"$counters"

failed=()
for package in "${packages[@]}"; do
	# `--quiet` so the counter lines are the only thing on stdout: they are the
	# evidence this file exists to keep, and cargo's progress output around them
	# would have to be filtered back out on the reading end.
	# shellcheck disable=SC2086
	if VIEWW_SHOT="$out" cargo run --quiet $profile -p "$package" >>"$counters" 2>>"$out/stderr.txt"; then
		printf '.'
	else
		printf 'x'
		failed+=("$package")
	fi
done
printf '\n'

# What this machine was, for the person reading a diff later. Every line is
# something that has plausibly caused a rendering difference somewhere; none of
# it is here to be decorative.
{
	echo "platform: $platform"
	echo "arch: $arch"
	echo "uname: $(uname -a)"
	echo "captured: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
	echo "profile: ${profile:---debug}"
	echo "rustc: $(rustc --version 2>/dev/null || echo unknown)"
	echo "cargo: $(cargo --version 2>/dev/null || echo unknown)"
	echo "commit: $(git rev-parse HEAD 2>/dev/null || echo unknown)"
	echo "dirty: $(git status --porcelain 2>/dev/null | wc -l | tr -d ' ') file(s)"
	echo "examples: ${#packages[@]}"
	echo "shots: $(find "$out" -maxdepth 1 -name '*.png' | wc -l | tr -d ' ')"
	echo
	# The font list, per the header. Truncated because a Linux box can have two
	# thousand of them and the question a diff asks is "are these two lists the
	# same", which a hash answers and a wall of paths does not — the paths are
	# kept as well because when the hashes differ, the next question is which
	# file.
	echo "fonts:"
	for dir in /usr/share/fonts /usr/local/share/fonts ~/.fonts ~/.local/share/fonts \
		/System/Library/Fonts /Library/Fonts ~/Library/Fonts \
		"${WINDIR:-/c/Windows}/Fonts"; do
		[ -d "$dir" ] || continue
		count="$(find "$dir" -type f \( -name '*.ttf' -o -name '*.otf' -o -name '*.ttc' \) 2>/dev/null | wc -l | tr -d ' ')"
		echo "  $dir: $count file(s)"
	done
} >"$out/manifest.txt"

if [ "${#failed[@]}" -gt 0 ]; then
	echo "shot-suite: ${#failed[@]} example(s) failed to run:" >&2
	printf '  %s\n' "${failed[@]}" >&2
	echo "  see $out/stderr.txt" >&2
fi

echo "shot-suite: $(find "$out" -maxdepth 1 -name '*.png' | wc -l | tr -d ' ') shot(s) in $out"

if [ -n "$against" ]; then
	[ -d "$against" ] || {
		echo "shot-suite: --against $against is not a directory." >&2
		exit 2
	}
	echo
	# shellcheck disable=SC2086
	cargo run --quiet $profile -p shot-diff -- "$against" "$out" --out "$out/diff"
	exit $?
fi

[ "${#failed[@]}" -eq 0 ]
