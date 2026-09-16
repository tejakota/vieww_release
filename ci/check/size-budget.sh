#!/usr/bin/env bash
# Fail if the framework's compiled size has grown more than it is allowed to.
#
#     ci/check/size-budget.sh            # check against the recorded figure
#     ci/check/size-budget.sh --record   # write today's figure as the new budget
#
# # What is measured, and why it is this and not the windowed example
#
# `crates/vieww/examples/counter.rs` — a real widget tree, laid out and painted,
# with **no GPU backend and no window**. That is the whole framework this
# repository actually wrote: elements, render objects, layout, paint, text,
# gestures, semantics.
#
# The obvious alternative is one of the `vieww-platform-winit` examples, and it
# is the wrong one. vello, wgpu and naga together dwarf everything here, so a
# budget they dominate moves whenever upstream releases and says nothing at all
# about whether *vieww* grew. A gate that fires for somebody else's reasons is a
# gate people learn to re-record without reading.
#
# `docs/AIMS.md` §E argues the binary-size item is worth taking seriously rather
# than dismissing: a Rust UI framework producing smaller binaries than a mainstream
# engine is a *Cheaper* win that is measurable today, and one of the few places
# the language choice shows up directly to a user. This is the measurement that
# makes the claim checkable later; it is not the claim.
#
# # Release, stripped, and one target
#
# Debug binaries carry symbols that swamp the signal, and a stripped release
# build is what ships. `--locked` so a dependency resolving differently does not
# silently change the number.
#
# The figure is inherently per-toolchain and per-target: a different rustc, a
# different libc, or a cross build produces a different size for identical
# source. So the recorded budget names the target it was taken on, and a
# mismatch is reported rather than compared — comparing across targets is how a
# gate turns into noise and then into `--record`.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

BUDGET_FILE="ci/check/size-budget.txt"
EXAMPLE="counter"
# How much growth is allowed before this fails. Generous on purpose: a gate that
# fires on ordinary work is one that gets deleted, and the thing worth catching
# is a dependency or a monomorphisation blow-up, which is tens of percent.
TOLERANCE_PERCENT=5

TARGET_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"

echo "Building ${EXAMPLE} (release, stripped) for ${TARGET_TRIPLE}…"
RUSTFLAGS="-C strip=symbols" cargo build \
    --locked \
    --release \
    -p vieww \
    --example "${EXAMPLE}" >&2

BINARY="target/release/examples/${EXAMPLE}"
if [[ ! -f "${BINARY}" ]]; then
    echo "error: ${BINARY} was not produced — did the example get renamed?" >&2
    exit 1
fi

# `stat -c` on GNU, `stat -f` on BSD/macOS. The macOS runner is a real target
# for this repository, so both.
if stat -c%s "${BINARY}" >/dev/null 2>&1; then
    SIZE="$(stat -c%s "${BINARY}")"
else
    SIZE="$(stat -f%z "${BINARY}")"
fi

if [[ "${1:-}" == "--record" ]]; then
    printf '%s %s\n' "${TARGET_TRIPLE}" "${SIZE}" > "${BUDGET_FILE}"
    echo "Recorded ${SIZE} bytes for ${TARGET_TRIPLE} in ${BUDGET_FILE}."
    exit 0
fi

if [[ ! -f "${BUDGET_FILE}" ]]; then
    echo "error: no recorded budget."
    echo
    echo "  ci/check/size-budget.sh --record"
    echo
    echo "Today's figure is ${SIZE} bytes on ${TARGET_TRIPLE}."
    exit 1
fi

RECORDED_TRIPLE="$(awk '{print $1}' "${BUDGET_FILE}")"
RECORDED_SIZE="$(awk '{print $2}' "${BUDGET_FILE}")"

if [[ "${RECORDED_TRIPLE}" != "${TARGET_TRIPLE}" ]]; then
    echo "Recorded on ${RECORDED_TRIPLE}; this machine is ${TARGET_TRIPLE}."
    echo "Sizes are not comparable across targets, so this is not a failure."
    echo "Today's figure here: ${SIZE} bytes."
    exit 0
fi

CEILING=$(( RECORDED_SIZE + (RECORDED_SIZE * TOLERANCE_PERCENT / 100) ))
DELTA=$(( SIZE - RECORDED_SIZE ))
PERCENT=$(awk -v d="${DELTA}" -v r="${RECORDED_SIZE}" 'BEGIN { printf "%.1f", (d / r) * 100 }')

echo "recorded: ${RECORDED_SIZE} bytes"
echo "today:    ${SIZE} bytes (${PERCENT}%)"
echo "ceiling:  ${CEILING} bytes (+${TOLERANCE_PERCENT}%)"

if (( SIZE > CEILING )); then
    cat >&2 <<EOF

error: ${EXAMPLE} grew ${PERCENT}%, past the ${TOLERANCE_PERCENT}% budget.

This is not automatically a bug. Growth is expected when the framework gains
something; what it must not be is *unnoticed*. Work out which of these it is:

  - a new dependency, or an existing one pulling in more;
  - a generic that is now instantiated for many more types;
  - the framework genuinely gaining a feature.

If it is the third, record the new figure deliberately:

    ci/check/size-budget.sh --record

and say in TRACKER.md what bought the bytes.
EOF
    exit 1
fi

echo "Within budget."
