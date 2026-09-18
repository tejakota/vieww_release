#!/usr/bin/env bash
#
# Ten pictures of what this machine draws: five feature examples and five
# studio screens.
#
#   ci/vieww tool screenshots.sh [OUT]   # default: target/screenshots
#
# # Why this exists
#
# The gate proves things about pixels by comparing them with each other. It
# cannot tell anybody that the product *looks right*, and
# `docs/RELEASE-CHECK.md` says so in as many words: "a person has skimmed
# index.html on each platform and the pictures look like the product — no
# missing glyphs, no clipped panels, no mismatched light/dark colours". That
# person needs pictures from each OS, and downloading a whole gate run to find
# them is why nobody does it.
#
# So this writes ten PNGs, small enough to attach to every CI run and look at
# in the browser. Not a gate row: it fails nothing, and it is the one piece of
# evidence a human reads rather than a script.
#
# The five features are chosen to be the ones a rendering problem shows up in
# first — two are mostly text, because text is what a font or scaling bug
# takes away while leaving every box and icon on the screen exactly where it
# was.
#
# Needs: a Rust toolchain. No display, no GPU (`vieww_paint::native` draws
# these; see its module doc).
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root" || exit 2

out="${1:-target/screenshots}"
mkdir -p "$out/features" "$out/studio"

# Matched as substrings of the package name by shot-suite.sh.
FEATURES="text-display,text-box,form-controls,data-table,markdown"

echo "==> feature screenshots"
# `--debug`: these are for looking at, and an optimised build of five examples
# costs more than the pictures are worth.
bash ci/certify/shot-suite.sh --debug --out "$out/features" --only "$FEATURES" ||
	echo "screenshots: the feature shots failed; carrying on" >&2

echo "==> studio screenshots"
# **The studio walkthrough writes into the checkout.** `release_check` exercises
# the "new screen" command for real, and that command creates
# `apps/viewwstudio/screens/src/screens/screen_<n>.rs` — one more pair of files
# per run, left behind in the working tree. Harmless on a throwaway CI
# checkout, and on a laptop it is a dirty tree that fails G0.5 on the next
# gate. So the new files are noted and removed afterwards; nothing already
# tracked or already untracked is touched.
scratch="apps/viewwstudio/screens/src/screens"
before=""
if git rev-parse --git-dir >/dev/null 2>&1; then
	before="$(git status --porcelain --untracked-files=all -- "$scratch" 2>/dev/null)"
fi

studio_all="$out/.studio-run"
bash ci/certify/release-check.sh --quick "$studio_all" ||
	echo "screenshots: the studio walkthrough exited non-zero; keeping whatever it wrote" >&2

# Five, spread across the run rather than the first five, which are all the
# same screen at start-up.
# A `while read` loop rather than `mapfile`: macOS ships bash 3.2, which does
# not have `mapfile`, and the macOS job produced feature shots and no studio
# ones at all the first time this ran.
shots=()
while IFS= read -r shot; do
	shots+=("$shot")
done < <(find "$studio_all" -name '*.png' | sort)
count="${#shots[@]}"
if ((count > 0)); then
	step=$((count / 5))
	((step > 0)) || step=1
	for i in 0 1 2 3 4; do
		index=$((i * step))
		((index < count)) || break
		cp "${shots[index]}" "$out/studio/$(basename "${shots[index]}")"
	done
	# The manifest names every screen and lists the labels the walkthrough
	# could not find — the difference between "this screen is wrong" and
	# "this screen was never drawn".
	find "$studio_all" -name 'manifest.json' -exec cp {} "$out/studio/" \; 2>/dev/null
fi
rm -rf "$studio_all"

# The screens the walkthrough just created, and only those.
if git rev-parse --git-dir >/dev/null 2>&1; then
	after="$(git status --porcelain --untracked-files=all -- "$scratch" 2>/dev/null)"
	while IFS= read -r line; do
		[ -n "$line" ] || continue
		case "$line" in '??'*) ;; *) continue ;; esac
		file="${line#?? }"
		case "$before" in *"$line"*) continue ;; esac
		echo "screenshots: removing $file, written by the studio walkthrough"
		rm -f "$file"
	done <<<"$after"
fi

echo
echo "screenshots: $(find "$out" -name '*.png' | wc -l | tr -d ' ') picture(s) in $out"
find "$out" -name '*.png' | sort | sed 's/^/  /'
