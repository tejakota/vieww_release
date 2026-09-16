#!/usr/bin/env bash
# Fail if line coverage falls below the floor.
#
#     ci/check/coverage.sh          # check against the floor
#     ci/check/coverage.sh --html   # and write a browsable report to target/llvm-cov
#
# # The exclusions, and why each one
#
# The tested set matches the `Test` job in `.github/workflows/ci.yml` exactly,
# and for that job's reasons rather than for coverage's:
#
#   - `vieww-platform-winit` turns on `vieww-paint/gpu` for the whole workspace
#     through cargo's feature unification, which is the thing DESIGN §11 put the
#     backend behind a feature to prevent;
#   - `reload-host` drags the same feature back in through a name that line never
#     mentions, which is a hole that survived from 2026-08-13 until CI found it;
#   - `reload-guest` and `reload-android` for the same reason.
#
# If those two lists ever disagree, this gate is measuring a different program
# than the one being tested, and its number is worse than no number.
#
# # Tests and examples are excluded from the *report*
#
# A test file's own coverage is a tautology — it ran, so it is covered — and
# including it lets the number be raised by writing more tests rather than by
# testing more code. Examples are excluded because they are compiled by clippy's
# `--all-targets` gate and are meant to be *looked at*, not asserted on; counting
# them would reward a screen nobody runs.
#
# # The floor is a floor, not a target
#
# 70% of lines. High enough that deleting a test suite fails the build, low
# enough that it does not push anyone towards testing accessors. `docs/AIMS.md`
# §E is explicit that vieww's testing story is already good and its *device*
# story is the differentiator — so this gate exists to catch a regression, not to
# be optimised.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

FLOOR=70

# **`cargo-llvm-cov` sets `RUSTFLAGS`, and setting it discards
# `.cargo/config.toml`.**
#
# Cargo picks exactly one source of rustflags, in order: the `RUSTFLAGS`
# environment variable, then `target.<triple>.rustflags`, then
# `build.rustflags`. They are not merged — the first one that exists wins and
# the rest are ignored outright. `cargo llvm-cov` exports `RUSTFLAGS` to add
# `-C instrument-coverage`, so the moment it runs, the
# `[target.'cfg(...)'] rustflags` block in `.cargo/config.toml` stops applying.
#
# What that block contains is `-C prefer-dynamic`, and the root `Cargo.toml`
# spends a page on why it is load-bearing: it is what makes `viewwstudio` and
# the preview `cdylib` it `dlopen`s share one `libstd`, which is what makes
# the host's `catch_unwind` catch a panic raised in guest code. Without it the
# guest links its own `libstd` and the panic crosses as a foreign exception:
#
#     fatal runtime error: Rust cannot catch foreign exceptions, aborting
#
# which is precisely how `viewwstudio`'s `pipeline` suite —
# `a_panicking_build_on_a_later_rebuild_is_caught_by_the_host_not_aborted` —
# died with SIGABRT under coverage while passing under `cargo test`. The
# abort then took the run down and `llvm-cov` reported "below 70%" from
# partial data, which is the same lie the preflight above exists to prevent:
# a toolchain problem wearing a coverage failure's clothes.
#
# `cargo-llvm-cov` appends to an existing `RUSTFLAGS` rather than replacing
# it, so exporting it here gets both. Keep in step with `.cargo/config.toml`;
# if the two disagree, this gate is measuring a binary nobody ships.
export RUSTFLAGS="${RUSTFLAGS:-} -C prefer-dynamic -C rpath"

# **`cargo-llvm-cov` sets `RUSTFLAGS`, and setting it discards
# `.cargo/config.toml` entirely.**
#
# Cargo takes rustflags from exactly one source, in order: the `RUSTFLAGS`
# environment variable, then `target.<triple>.rustflags`, then
# `build.rustflags`. They are not merged — the first that exists wins and the
# rest are ignored. `cargo llvm-cov` exports `RUSTFLAGS` to add
# `-C instrument-coverage`, so the moment it runs, the `[target.*] rustflags`
# block in `.cargo/config.toml` stops applying.
#
# What that block carries is `-C prefer-dynamic`, and the root `Cargo.toml`
# spends a page on why it is load-bearing: it makes `viewwstudio` and the
# preview `cdylib` it `dlopen`s share one `libstd`, which is what lets the
# host's `catch_unwind` catch a panic raised in guest code. Without it the
# guest links its own `libstd` and the panic arrives as a foreign exception:
#
#     fatal runtime error: Rust cannot catch foreign exceptions, aborting
#
# which is exactly how `viewwstudio`'s `pipeline` suite —
# `a_panicking_build_on_a_later_rebuild_is_caught_by_the_host_not_aborted` —
# died with SIGABRT under coverage while passing under `cargo test`. The abort
# took the run down, and `llvm-cov` then reported "below 70%" from partial
# data: the same lie the preflight above exists to prevent, arriving from a
# direction the preflight cannot see.
#
# `cargo-llvm-cov` appends to an existing `RUSTFLAGS` rather than replacing it,
# so exporting here gets both. Keep in step with `.cargo/config.toml`; if the
# two disagree, this gate is measuring a binary nobody ships.
export RUSTFLAGS="${RUSTFLAGS:-} -C prefer-dynamic -C rpath"

# Two separate things have to be present, and they fail in ways that look
# nothing alike — so they are checked separately and reported separately.
#
# **This distinction is the whole reason there is a preflight.** The first draft
# of this script had none, and `cargo llvm-cov --fail-under-lines` exits
# non-zero for *both* "coverage is too low" and "the toolchain is missing". So a
# machine without `llvm-tools-preview` was told its coverage had fallen below the
# floor, which is a lie that sends whoever reads it looking for missing tests.
missing() {
    cat >&2 <<EOF
error: coverage cannot be measured on this machine — $1

    cargo install cargo-llvm-cov
    rustup component add llvm-tools-preview

Both are needed, and the second is the one that is usually missing. Its symptom
is not "command not found" — it is:

    raw profile version mismatch: Profile uses raw profile format version = N;
    expected version = M

which means the \`llvm-profdata\` on PATH belongs to a different LLVM than the one
rustc was built with. \`llvm-tools-preview\` installs the matching one; a system
LLVM will not do, however new it is.

**This is not a coverage failure.** Nothing has been measured.
EOF
    exit 2
}

if ! cargo llvm-cov --version >/dev/null 2>&1; then
    missing "cargo-llvm-cov is not installed."
fi

# `llvm-profdata` has to be the one matching rustc's LLVM, which is the copy
# inside the toolchain's own sysroot. A system `llvm-profdata` on PATH is *worse*
# than none: it runs, and then fails on a version mismatch a long way from here.
SYSROOT="$(rustc --print sysroot)"
if ! compgen -G "${SYSROOT}/lib/rustlib/"*"/bin/llvm-profdata" >/dev/null; then
    missing "rustc's llvm-tools are not installed (looked in ${SYSROOT})."
fi

ARGS=(
    --workspace
    # **Without this, this gate measures a different program than it says.**
    #
    # `vieww-paint`'s `native` module is its rasterizer and, since the vello
    # backends were removed, its *only* renderer — and it is behind an
    # off-by-default feature. The `Test` job passes
    # `--features vieww-paint/native` for exactly that reason; this list did
    # not, so the 224 tests covering the rasterizer were never compiled here
    # and every line they cover counted as uncovered against the 70% floor.
    #
    # This is the same class of mistake as the `--exclude` lists drifting:
    # two invocations that are supposed to describe one program, maintained
    # in two files.
    --features vieww-paint/native

    --exclude vieww-platform-winit
    --exclude reload-host
    --exclude reload-guest
    --exclude reload-android
    # Not the same thing as `--exclude`: these drop files from the *report*
    # while still running them, which is what "a test's own coverage is a
    # tautology" needs.
    --ignore-filename-regex '(tests?/|examples/)'
)

if [[ "${1:-}" == "--html" ]]; then
    cargo llvm-cov "${ARGS[@]}" --html
    echo "Report written to target/llvm-cov/html/index.html"
fi

# `--fail-under-lines` makes the tool itself enforce the floor, so the number
# this script prints and the number it judges cannot drift apart — which is what
# happens the moment a shell script starts parsing a percentage out of a summary.
if ! cargo llvm-cov "${ARGS[@]}" --summary-only --fail-under-lines "${FLOOR}"; then
    cat >&2 <<EOF

error: line coverage is below ${FLOOR}%.

Untested code is not automatically a defect, but it is always a decision. If the
uncovered lines are a deliberate gap — a platform path this machine cannot run,
a panic arm that exists to be loud — say so in TRACKER.md rather than lowering
the floor quietly.
EOF
    exit 1
fi
