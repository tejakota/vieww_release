#!/usr/bin/env bash
#
# Build this host's release artifacts and make them publishable.
#
#   ci/vieww artifacts [package.sh args]   # default: --installers
#   ci/vieww sums DIR [DIR...]             # merge per-OS SHA256SUMS into ./SHA256SUMS
#
# Writes into target/package/:
#   viewwstudio-<os>-<arch>.*   from packaging/package.sh --installers
#   SHA256SUMS                  one line per artifact, `sha256sum -c` format
#   MANIFEST.txt                version, source digest, git rev, toolchain,
#                               host, signing status — checklist rows G8.*
#
# The download page (packaging/site/index.html) links a single `SHA256SUMS`,
# so after building on all three OSes, merge them with `ci/vieww sums`.
#
# On Windows run from Git Bash, as packaging/package.sh requires.

set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if command -v sha256sum >/dev/null; then SHA256=(sha256sum); else SHA256=(shasum -a 256); fi

if [[ "${1:-}" == --merge ]]; then
	shift
	(($# > 0)) || { echo "usage: ci/vieww sums DIR [DIR...]" >&2; exit 2; }
	cat "${@/%//SHA256SUMS}" | sort -k2 | uniq >SHA256SUMS
	dupes=$(awk '{print $2}' SHA256SUMS | sort | uniq -d)
	[[ -z "$dupes" ]] || { echo "conflicting checksums for: $dupes" >&2; exit 1; }
	echo "wrote $(pwd)/SHA256SUMS ($(wc -l <SHA256SUMS | tr -d ' ') artifacts)"
	exit 0
fi

cd "$root"
bash ci/check/release-clean-check.sh "$root"

args=("$@")
((${#args[@]} > 0)) || args=(--installers)
bash packaging/package.sh "${args[@]}"

out="$root/target/package"
cd "$out"
shopt -s nullglob
files=(viewwstudio-*.deb viewwstudio-*.AppImage viewwstudio-*.tar.gz viewwstudio-*.zip viewwstudio-*.msi viewwstudio-*.dmg)
((${#files[@]} > 0)) || { echo "artifacts: package.sh produced no artifacts in $out" >&2; exit 1; }
"${SHA256[@]}" "${files[@]}" | sed 's/ \*/  /' >SHA256SUMS

case "$(uname -s)" in
Darwin) os=macos signing="UNSIGNED (no Developer ID signature, not notarized)" ;;
MINGW* | MSYS* | CYGWIN*) os=windows signing="UNSIGNED (no Authenticode signature)" ;;
*) os=linux signing="unsigned packages; integrity via SHA256SUMS" ;;
esac
{
	echo "version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")"
	echo "git_rev=$(git -C "$root" rev-parse HEAD 2>/dev/null || echo none)"
	echo "git_dirty=$(git -C "$root" status --porcelain 2>/dev/null | wc -l | tr -d ' ')"
	echo "built=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
	echo "host_os=$os"
	echo "host=$(uname -a)"
	echo "rustc=$(rustc --version)"
	echo "cargo=$(cargo --version)"
	echo "toolchain_file=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$root/rust-toolchain.toml")"
	echo "signing=$signing"
	echo "reproducible=no (timestamps and bundled toolchain paths are not normalised)"
	echo "# artifacts"
	cat SHA256SUMS
} >MANIFEST.txt

cat MANIFEST.txt
echo "==> $out/SHA256SUMS"
