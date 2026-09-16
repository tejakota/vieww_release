#!/usr/bin/env bash
#
# Make room on a GitHub-hosted runner before `ci/vieww gate`.
#
#   bash ci/tools/ci-runner-prep.sh
#
# Hosted runner images ship tens of GB of SDKs this repository never uses
# (.NET, Haskell, CodeQL, extra simulators), and the space left over can be less
# than a gate needs even in lean mode. The Android SDK and NDK are KEPT: Studio's
# Android export (G2.10) builds with them. This deletes the rest, and on Windows
# points `target/` at whichever drive has the most room.
#
# Only for throwaway CI machines: it deletes system SDKs. It refuses to run
# anywhere `CI` and `GITHUB_ACTIONS` are not both set.

set -uo pipefail
if [[ "${CI:-}" != true || "${GITHUB_ACTIONS:-}" != true ]]; then
	echo "ci-runner-prep: refusing to run outside GitHub Actions (it deletes SDKs)" >&2
	exit 2
fi
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

show() {
	echo "--- disk ($1)"
	df -h 2>/dev/null | grep -vE '^(tmpfs|devfs|map |overlay)' || true
}
show before

gone() {
	local path
	for path in "$@"; do
		[[ -e "$path" ]] || continue
		echo "removing $path"
		rm -rf "$path" 2>/dev/null || sudo rm -rf "$path" 2>/dev/null || true
	done
}

case "$(uname -s)" in
Darwin)
	gone "$HOME/.dotnet" \
		/usr/local/share/dotnet "$HOME/.ghcup" /usr/local/lib/node_modules \
		"$HOME/Library/Developer/CoreSimulator/Caches"
	# Xcode and its iOS simulator platform stay: Studio's iOS simulator export
	# (G2.11) builds against them, and hdiutil packages the .dmg.
	;;
MINGW* | MSYS* | CYGWIN*)
	gone /c/hostedtoolcache/windows/go /c/hostedtoolcache/windows/CodeQL \
		/c/ghcup /c/tools/ghc* /c/Strawberry
	# The checkout lives on D:, which is small on some images; C: usually has
	# far more room. A junction keeps `target/` at the path every script
	# expects (packaging/package.sh hard-codes $root/target) while the bytes
	# land on the bigger drive.
	free_of() { df -Pk "$1" 2>/dev/null | awk 'NR==2 {print $4}'; }
	here_free=$(free_of "$root")
	c_free=$(free_of /c)
	if [[ -n "$c_free" && -n "$here_free" && "$c_free" -gt "$here_free" && ! -e "$root/target" ]]; then
		mkdir -p /c/vieww-target
		cmd //c mklink /J "$(cygpath -w "$root/target")" 'C:\vieww-target' >/dev/null &&
			echo "target/ -> C:\\vieww-target ($((c_free / 1048576)) GB free there)"
	fi
	;;
Linux)
	gone /usr/share/dotnet /opt/ghc /usr/local/.ghcup /opt/hostedtoolcache/CodeQL
	sudo docker image prune --all --force >/dev/null 2>&1 || true
	;;
esac

show after
