#!/usr/bin/env bash
#
# Check the parts of ci/mobile/reload-watch-android.sh that do not need a phone.
#
#   ci/mobile/reload-watch-android-test.sh
#
# Exits non-zero on the first failing expectation, and prints every result.
#
# # Why this exists
#
# The watcher's happy path needs an attached device, an installed host, and a
# cross-compile — so nothing about it can run in CI, and in practice it gets
# exercised by using it, which is how a change-detector's *silence* becomes
# invisible. A watcher that never fires looks exactly like a watcher with
# nothing to do.
#
# It has already earned its place. `fingerprint` was first written with `%Y`,
# whole seconds, plus the file size — and `changing BAND to another colour`,
# which is the literal first instruction in `examples/reload-guest`, keeps the
# byte count and usually lands in the same second. The fingerprint did not move
# and no rebuild happened. That is the case pinned below.
#
# The functions are read out of the real script rather than copied, on the rule
# this directory has paid for twice: a copy inherits none of the original's
# fixes.

set -uo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
src="$root/ci/mobile/reload-watch-android.sh"

[ -f "$src" ] || { echo "reload-watch-test: $src is not there." >&2; exit 2; }

eval "$(awk '/^fingerprint\(\) \{/,/^\}/' "$src")"
eval "$(awk '/^changed_paths\(\) \{/,/^\}/' "$src")"

if ! declare -f fingerprint >/dev/null || ! declare -f changed_paths >/dev/null; then
    echo "reload-watch-test: could not lift the functions out of the script." >&2
    echo "  Were they renamed, or re-indented off column zero?" >&2
    exit 2
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Shadows the script's own value. `fingerprint` reads it as a global.
watched="$tmp/guest"
mkdir -p "$watched/src"
printf 'const BAND: u32 = 1;\n' >"$watched/src/lib.rs"
printf '[package]\nname = "g"\n' >"$watched/Cargo.toml"
printf 'notes\n' >"$watched/src/README.txt"

passed=0
failed=0

check() {
    if [ "$2" = "$3" ]; then
        passed=$((passed + 1))
        echo "  ok   $1"
    else
        failed=$((failed + 1))
        echo "  FAIL $1"
        echo "         expected [$3]"
        echo "         got      [$2]"
    fi
}

first="$(fingerprint)"
check "watches .rs and Cargo.toml" "$(printf '%s\n' "$first" | wc -l)" "2"
check "and nothing else" "$(printf '%s\n' "$first" | grep -c 'README' || true)" "0"

# **The regression this file was written for.** Same byte count, same second.
printf 'const BAND: u32 = 2;\n' >"$watched/src/lib.rs"
same_length="$(fingerprint)"
check "an edit of identical length, in the same second, is still an edit" \
    "$([ "$first" != "$same_length" ] && echo detected || echo missed)" "detected"
check "and the changed file is named" \
    "$(changed_paths "$first" "$same_length")" "$watched/src/lib.rs"

check "nothing moving is not a change" \
    "$(changed_paths "$same_length" "$(fingerprint)")" ""

printf 'more notes\n' >"$watched/src/README.txt"
check "an untracked file is not a change" \
    "$(changed_paths "$same_length" "$(fingerprint)")" ""

printf '[package]\nname = "g"\nversion = "0.0.1"\n' >"$watched/Cargo.toml"
check "the manifest counts too" \
    "$(changed_paths "$same_length" "$(fingerprint)")" "$watched/Cargo.toml"

echo
if [ "$failed" -eq 0 ]; then
    echo "reload-watch-test: PASS — $passed checks."
else
    echo "reload-watch-test: FAIL — $failed of $((passed + failed)) checks." >&2
fi
[ "$failed" -eq 0 ]
