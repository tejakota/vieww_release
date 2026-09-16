#!/usr/bin/env bash
#
# Write a distribution-clean source archive.
#
#   ci/release/package-source.sh [OUT.tar.gz]     # default: target/vieww-source.tar.gz
#
# Runs `ci/check/release-clean-check.sh` first and refuses to package a tree that
# fails it. The archive never contains `target/`, build outputs, or
# certification evidence: evidence is produced by running
# `ci/certify/certify.sh` against the unpacked archive, and carries the
# source digest that ties it to that tree.

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${1:-$ROOT/target/vieww-source.tar.gz}"
cd "$ROOT"

bash ci/check/release-clean-check.sh "$ROOT"
mkdir -p "$(dirname "$OUT")"
name="$(basename "$OUT" .tar.gz)"
# GNU tar (Linux, Git Bash) renames with --transform; bsdtar (macOS) with -s.
if tar --version 2>/dev/null | grep -q GNU; then rename=(--transform "s,^\.,$name,"); else rename=(-s ",^\.,$name,"); fi
tar --exclude=./target --exclude='*/target' --exclude=.git --exclude=./out \
	"${rename[@]}" -czf "$OUT" .
echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
