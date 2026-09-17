#!/usr/bin/env bash
#
# Fail if the tree carries anything that must not ship in a framework source
# archive.
#
#   ci/check/release-clean-check.sh [DIR]     # default: the repository root
#
# # Why this exists
#
# A September 2026 review of the published archive found `Cargo.lock.orig`,
# `Cargo.lock.rej`, `ci/*.rej`, `*.vieww-fix.bak`, `*.vieww-validate.bak`, a
# truncated copy of the archive nested inside itself, a `cargo metadata` dump
# with a developer's home paths, and a `cert-linux/` directory whose `fmt.txt`
# recorded a failed run against an older tree — sitting next to source that
# claimed the opposite. None of it broke a build. All of it made the archive
# read as a patch workspace, and the stale evidence was actively misleading.
#
# Every one of those classes is a rule below, and `ci/release/package-source.sh` runs
# this before it writes an archive.

set -euo pipefail
DIR="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$DIR"

problems=0
report() {
	echo "release-clean: $1" >&2
	problems=$((problems + 1))
}

# Patch leftovers.
while IFS= read -r -d '' file; do
	report "patch leftover: $file"
done < <(find . -path ./target -prune -o \( -name '*.orig' -o -name '*.rej' -o -name '*.bak' \) -print0)

# Archives inside the source tree.
while IFS= read -r -d '' file; do
	report "archive inside the source tree: $file"
done < <(find . -path ./target -prune -o \( -name '*.tar' -o -name '*.tar.gz' -o -name '*.tgz' -o -name '*.zip' \) -print0)

# Certification evidence and logs outside target/.
for path in cert-linux cert-windows certification-linux.log certification-windows.log; do
	[[ -e "$path" ]] && report "certification evidence in the source tree: $path (it belongs to a run; see ci/certify/certify.sh)"
done

# Machine-specific dumps.
[[ -e vieww-metadata.json ]] && report "cargo metadata dump with local paths: vieww-metadata.json"
if grep -rIl --exclude-dir=target --exclude-dir=.git -e '/home/[a-z][a-z0-9_-]*/Documents' -e '/home/[a-z][a-z0-9_-]*/Music' . 2>/dev/null | grep -v '^./ci/check/release-clean-check.sh$' >/tmp/release-clean-paths.$$; then
	while IFS= read -r file; do report "developer home path in: $file"; done </tmp/release-clean-paths.$$
fi
rm -f /tmp/release-clean-paths.$$

# Generated test and suite output (images written for a person to look at,
# fixture/census renders). Regenerated on every run; never source.
for path in crates/vieww-paint/tests/output fixtures-out census-out gpu-test-out vieww-standard-out out; do
	[[ -e "$path" ]] && report "generated output in the source tree: $path"
done

# Build outputs.
while IFS= read -r -d '' dir; do
	report "build output directory: $dir"
done < <(find . -path ./target -prune -o -type d -name dist -print0)

# Workspace members that do not exist (the archive shipped with one).
while IFS= read -r member; do
	case "$member" in
	*'*'*) continue ;;
	esac
	[[ -f "$member/Cargo.toml" ]] || report "workspace member without a manifest: $member"
done < <(sed -n '/^members = \[/,/^\]/p' Cargo.toml | grep -o '"[^"]*"' | tr -d '"')

# The build directory must be ignored by git. Hidden files are the ones that go
# missing when a tree is copied or uploaded by hand — `.cargo/config.toml` did
# once, and `.gitignore` did on the first CI run, where every checkout showed
# `?? target/` as soon as anything was built.
if [[ ! -f .gitignore ]] || ! grep -qE '^/?target/?$' .gitignore; then
	report "missing .gitignore, or it does not ignore /target (hidden files are easy to lose when a tree is uploaded; commit it)"
fi

# Required configuration that packagers tend to drop with hidden directories.
if ! grep -q 'prefer-dynamic' .cargo/config.toml 2>/dev/null; then
	report "missing .cargo/config.toml (or it lacks -C prefer-dynamic): viewwstudio's panic boundary and the mobile runners depend on it"
fi

if ((problems > 0)); then
	echo "release-clean: $problems problem(s)" >&2
	exit 1
fi
echo "release-clean: OK"
