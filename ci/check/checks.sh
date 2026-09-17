#!/usr/bin/env bash
#
# Everything that has to be green before a release is cut.
#
# # Why this exists
#
# `ci/` held sixteen scripts and not one of them was "run the checks". Every
# script here does something specific and expensive — install an APK on an
# attached phone, drive `uiautomator`, measure a binary against a size budget —
# and the ordinary business of "does this workspace build, is it formatted, do
# the tests pass" was left to whatever each person typed. That is the state in
# which a release goes out with a test suite nobody ran in full, and it is how
# `vieww-reload`'s end-to-end test would have stayed unrun: it is `#[ignore]`d
# for taking five minutes, and an ignored test with no script naming it is a
# test that does not exist.
#
# Nothing here needs a device. The device suites stay where they are —
# `ci/mobile/device-suite.sh`, `ci/mobile/a11y-android.sh` — because they need hardware
# plugged in and this must be runnable by anyone, on anything, before pushing.
#
# Usage:
#   ci/check/checks.sh              # everything below
#   ci/check/checks.sh --quick      # skip the slow end-to-end suites
#
# Exits non-zero on the first failure, and says which stage failed.

set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root" || exit 1
# Lean builds (no debuginfo, no incremental) and pruning; see ci/lib/lean.sh.
# shellcheck source=ci/lib/lean.sh
. "$root/ci/lib/lean.sh"
vieww_lean_env

quick=""
for arg in "$@"; do
	case "$arg" in
	--quick) quick=1 ;;
	*)
		echo "unknown argument: $arg" >&2
		echo "usage: ci/check/checks.sh [--quick]" >&2
		exit 2
		;;
	esac
done

# **Every stage runs, and the summary names every failure.**
#
# This used to be `set -e`: the first failing stage ended the run. A beta test
# on a real laptop went clippy failure -> fix -> 25-minute rerun -> docs failure
# -> fix -> rerun, one stage per round trip, exactly the pattern the
# `--keep-going` notes below describe for a single cargo command. Now each
# stage's failure is recorded and the run continues; the exit status is still
# non-zero if anything failed. `VIEWW_CHECKS_FAIL_FAST=1` restores the old
# behaviour.
current_stage=""
failures=()
stage() {
	current_stage="$1"
	printf '\n==> %s\n' "$1"
}
check() {
	"$@"
	local rc=$?
	if [ "$rc" -ne 0 ]; then
		failures+=("$current_stage (exit $rc)")
		printf '    FAILED: %s (exit %s)\n' "$current_stage" "$rc" >&2
		if [ "${VIEWW_CHECKS_FAIL_FAST:-0}" = 1 ]; then
			exit "$rc"
		fi
	fi
	return 0
}

# The workspace pins a toolchain in `rust-toolchain.toml`, but a machine with no
# default toolchain at all fails inside cargo with a message about no such
# directory, which reads like a broken checkout rather than a missing rustup
# default. Caught here instead.
if ! cargo --version >/dev/null 2>&1; then
	echo "cargo will not run. If rustup is installed with no default toolchain," >&2
	echo "set one: rustup default stable" >&2
	exit 1
fi

stage "formatting"
# `--check` rather than a rewrite: a check script that edits the tree makes the
# diff it was run to protect.
check cargo fmt --all -- --check

# **`--features` here, for the same reason the tests stage has it.**
#
# `vieww-paint`'s `native` module is `#[cfg(feature = "native")]` and is the
# only renderer this framework has. A plain `cargo clippy --workspace` does
# not compile it, so this stage has never linted the rasterizer — not one
# line of it — while reporting green about the whole workspace.
#
# That is not hypothetical. `native/image.rs` carried a dead `pub(crate) fn
# sample_device_pixel` from the day its callers were changed to the
# pre-inverted `sample_device_pixel_with`, and the only gate that ever found
# it was `ci/check/wasm-check.sh` — because `vieww-platform-web` turns the feature
# on, so clippy finally saw the module. A lint gate that skips the largest
# and hottest module in the workspace is reporting on the part nobody was
# worried about.
#
# `vieww-hal/vulkan` for the same reason it is on the docs stage below.
#
# **`--keep-going`, or this stage reports one lint per run.**
#
# `-D warnings` makes the first warning a hard error, and cargo stops
# scheduling new units as soon as one fails — so a workspace with five
# unrelated lints in five crates takes five runs to reveal them, one per push,
# each costing a full CI cycle to learn a single line. That is exactly how this
# stage behaved once it could finally run: an unused import in one example, an
# unused loop variable in another, a `manual_clamp` in a third, each found only
# after the one before it was fixed.
#
# `--keep-going` tells cargo to finish every unit it can rather than stopping
# at the first failure. The stage still fails, and for the same reasons; it
# just names all of them at once.
stage "clippy (workspace, all targets, warnings are errors)"
check cargo clippy --workspace --all-targets --keep-going \
	--features "vieww-paint/native,vieww-hal/vulkan" -- -D warnings

stage "build (workspace, all targets)"
# `--keep-going` for the same reason as the clippy stage above.
check cargo check --workspace --all-targets --keep-going

# **The docs are a shipped artefact, and until this stage they were the only one
# nothing checked.** A broken intra-doc link does not fail a build, a test or
# clippy; it renders on docs.rs as literal `[`Thing`]` — brackets and all — on
# the front page of a crate somebody is deciding whether to depend on. The
# workspace had accumulated 117 of them, in every published crate, because
# nothing had ever asked.
#
# `--features` mirrors `vieww-paint`'s `[package.metadata.docs.rs]`: `native` is
# off by default and docs.rs turns it on, so a link into it is only checkable —
# and only *matters* — with it enabled. Keep the two in step; a link that
# resolves here and not on docs.rs is exactly the failure this stage exists to
# stop.
#
# **This named `gpu`, `hybrid` and `cpu` until now, and those features do not
# exist.** They were `vieww-paint`'s three vello-based backends and they were
# removed with vello (`docs/RENDERER-MIGRATION.md`); nothing updated this line,
# and cargo does not warn about an unknown feature on the command line — it
# fails resolution outright:
#
#     error: failed to select a version for `vieww-paint`
#
# So this stage, and therefore every stage after it in this script, had not run
# since the migration. A check script that cannot start is worse than no check
# script, because the repository still looks like it has one.
# **`vieww-hal/vulkan` is in the feature list, and its absence was a hole.**
#
# `vieww_hal::vulkan` is behind a feature, so a `cargo doc` that does not enable
# it does not document the module — and this stage did not enable it. The whole
# Vulkan backend, which is the largest body of new code in the workspace, had
# therefore never been through `cargo doc` at all, and was carrying a broken
# intra-doc link (`super::VulkanDevice`, from `vulkan/scene.rs`'s module doc) the
# entire time this stage was passing.
#
# A gate that runs on everything except the newest code is worse than no gate:
# it reports green about the part nobody was worried about.
stage "docs (workspace, broken intra-doc links are errors)"
check env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --keep-going \
	--features "vieww-paint/native,vieww-hal/vulkan"
# The rendered docs were the check's by-product, not its result.
vieww_prune "$root" doc

# **The declared minimum is a promise to every consumer, and nothing was keeping
# it.** Every stage above builds on whatever `stable` currently is, so the
# workspace could start using a feature stabilised last month and stay green
# while `rust-version = "1.85"` went on telling crates.io that 1.85 was enough.
#
# Skipped rather than failed when the toolchain is not installed: this script has
# to be runnable by anyone before pushing, and `rustup toolchain install` is not
# something a check script should do behind somebody's back. CI installs it and
# does not skip.
msrv="$(sed -n 's/^rust-version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"
if [ -n "$msrv" ] && rustup toolchain list 2>/dev/null | grep -q "^$msrv"; then
	stage "minimum supported Rust ($msrv)"
	# Its own target directory, deleted afterwards: a different compiler shares
	# nothing with target/debug and would otherwise sit beside it for good.
	check env RUSTUP_TOOLCHAIN="$msrv" CARGO_TARGET_DIR="$root/target/msrv" cargo check --workspace --all-targets
	vieww_prune "$root" msrv
else
	stage "minimum supported Rust ($msrv) — skipped, not installed"
	echo "  install it to check locally: rustup toolchain install $msrv"
fi

# Licences and advisories across the whole dependency closure. "This project is
# Apache-2.0" is a claim about the few hundred crates wgpu, cosmic-text and
# winit bring with them, not just the fifteen in `crates/` — and a security
# advisory against one of them breaks no build, so with no gate the first anyone
# hears of one is a report from a user. See `deny.toml` for the policy.
if command -v cargo-deny >/dev/null 2>&1; then
	stage "licences and advisories"
	check cargo deny check --hide-inclusion-graph all
else
	stage "licences and advisories — skipped, cargo-deny not installed"
	echo "  install it to check locally: cargo install cargo-deny"
fi

# **The panic boundary, checked before the suite that depends on it.**
#
# `viewwstudio`'s `tests/pipeline.rs` loads a preview `cdylib` and asserts that a
# panic raised *inside guest code* is caught by the host. That only works when
# host and guest share one `libstd`. `apps/viewwstudio/src/compile.rs` passes
# `-C prefer-dynamic` for the guest explicitly; the host side comes from the
# checkout's `.cargo/config.toml`, and nothing until now noticed when it was
# absent.
#
# What absence looks like without this check is the worst kind of failure:
#
#     fatal runtime error: Rust cannot catch foreign exceptions, aborting
#     process didn't exit successfully (signal: 6, SIGABRT)
#
# — the test binary aborts, so all twenty-one tests in that file are lost, the
# exit code says nothing about which one, and the message points at the
# unwinder rather than at the missing three lines of configuration. It reads
# like a bug in the panic-catching code the test exists to prove.
#
# A `.cargo/` directory is a common `.gitignore` entry, which is exactly how
# this ends up present on the machine it was written on and absent everywhere
# else. Checked here rather than trusted.
if [ -f .cargo/config.toml ] && grep -q 'prefer-dynamic' .cargo/config.toml; then
	: # the host will link std dynamically; the boundary holds
else
	cat >&2 <<'EOF'
error: the studio's guest-panic boundary is not configured.

`.cargo/config.toml` is missing, or does not set `-C prefer-dynamic` for this
host. Without it the test binary statically links its own `libstd`, a panic
raised in a loaded preview is a *foreign* exception to it, and
`viewwstudio`'s `tests/pipeline.rs` does not fail — it aborts the process:

    fatal runtime error: Rust cannot catch foreign exceptions, aborting

If that file exists on your machine but not in a fresh clone, it is untracked.
`git ls-files .cargo/` answers that in one line. It also carries the Android
and iOS linker and runner wiring (see ci/mobile/ndk-clang.sh, ci/mobile/adb-runner.sh), so
it needs to be committed rather than reconstructed.

The host half is:

    [target.'cfg(all(target_os = "linux", target_env = "gnu"))']
    rustflags = ["-C", "prefer-dynamic", "-C", "rpath"]

with the same for the macOS triples. **Not for MSVC** — that toolchain ships
no dynamic std; see ci/check/platform-check.sh's Windows note.

Note also that setting the `RUSTFLAGS` environment variable at all discards
`target.*.rustflags` wholesale — cargo picks exactly one source — so a shell
or a CI step that exports it silently removes this boundary. ci/check/coverage.sh
carries the same warning for cargo-llvm-cov.
EOF
	exit 1
fi

# **`--features vieww-paint/native`, or the renderer is not tested.**
#
# `vieww-paint`'s `native` module is its rasterizer and, since the vello
# backends were removed, its *only* renderer — and it is behind an off-by-default
# feature. A plain `cargo test --workspace` therefore ran none of the 224 tests
# that cover it: not the parity suite, not the retained-repaint equivalence
# suite, not the glyph residency suite. They were not failing. They were not
# being compiled.
stage "tests (workspace)"
check cargo test --workspace --features vieww-paint/native

# **The one wasm check that needs no wasm toolchain.**
#
# `crates/vieww-platform-web` names `web-sys` types, and `web-sys` gates every
# binding behind a cargo feature of the same name — so a type named but not
# enabled is a compile error waiting for the first machine that has the target.
# Two of them (`Event`, `EventTarget`) sat in that crate from the day it was
# written for exactly that reason: nobody could compile it, so nobody found
# them, and the whole check was parked behind a target install.
#
# Comparing two lists of names in two files needs no target, so it runs here,
# on every machine, every time. `ci/check/wasm-check.sh` exits 2 when the target
# cannot be installed, which is not a failure of this stage — the audit ran.
stage "web-sys feature audit (the half of ci/check/wasm-check.sh that needs no target)"
check bash -c 'ci/check/wasm-check.sh || [ $? -eq 2 ]'

# **The fixture gallery, as a gate rather than as a gallery.**
#
# It renders every screen this framework presents as a reference for what it
# looks like, and it now exits non-zero if any of them has a layout that does
# not fit — see `examples/fixtures/src/runner.rs`. Five fixtures did: a settings
# card cut off mid-row, a staggered list painting three rows past the page, a
# shared-element transition laying a 192px detail view into a 60px box. Every
# one of them had been reporting itself on stderr, between the pictures, on
# every run, for as long as it existed.
#
# `--release` because the gallery renders 23 screens plus 7 animations plus two
# interaction suites, and an unoptimised rasterizer turns that from twenty
# seconds into minutes.
stage "fixture gallery (renders every screen; fails on a layout overflow)"
check bash -c 'cargo run --release -p fixtures -- "$(mktemp -d)" >/dev/null'

summary() {
	if [ "${#failures[@]}" -gt 0 ]; then
		printf '\n%s stage(s) failed:\n' "${#failures[@]}" >&2
		printf '  - %s\n' "${failures[@]}" >&2
		exit 1
	fi
}

if [ -n "$quick" ]; then
	summary
	printf '\nall quick checks passed. The end-to-end suites were skipped:\n'
	printf '  cargo test -p vieww-reload --test end_to_end -- --ignored --test-threads=1\n'
	exit 0
fi

# **The `#[ignore]`d suites, which is the whole reason this file is not just a
# README paragraph.** They are ignored because they run cargo inside a test and
# take minutes; they are run here because what they check — a real `cdylib`
# built, loaded and swapped with the element tree intact — is the claim the
# project is built around, and nothing else in the suite touches `dlopen`.
stage "hot reload, end to end (builds the guest crate; slow)"
check cargo test -p vieww-reload --test end_to_end -- --ignored --test-threads=1

# The packaged studio is the artefact users get, and everything above tests the
# checkout instead. `--debug --no-toolchain` is the cheap shape: it still builds
# the tree, still writes the SDK manifest, and still runs `verify_bundle`, which
# compiles a real preview against the bundle. What it skips is copying ~600MB of
# sysroot, which proves nothing that the full run does not.
stage "packaging (layout and a preview compiled against the bundle)"
check packaging/package.sh --debug --no-toolchain
vieww_prune "$root" package

summary
printf '\nall checks passed.\n'
