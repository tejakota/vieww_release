#!/usr/bin/env bash
#
# One command per platform: "does vieww compile for this, on this machine".
#
#     ci/check/platform-check.sh linux
#     ci/check/platform-check.sh macos
#     ci/check/platform-check.sh windows
#     ci/check/platform-check.sh ios
#     ci/check/platform-check.sh android
#     ci/check/platform-check.sh web
#     ci/check/platform-check.sh                 # every one this machine can attempt
#
# # Why this exists
#
# `TRACKER.md` has, for as long as it has existed, listed macOS, Windows, iOS,
# Android and the web as "real code, unit-tested for its logic, never built on
# that platform's own toolchain". That was true and it was also unactionable:
# there was no answer to "so what do I run on a Mac?" A person with the machine
# that could close the gap had to read three crates' module docs to work out
# which cargo invocation was the one that mattered, and the most likely outcome
# of that is that they do not.
#
# So this is the missing half of that status line. Every row in it now has one
# command, and each of them does one of three things:
#
#   * passes, and says exactly what it did and did not prove;
#   * fails at the compiler, which is the useful outcome and the whole point;
#   * refuses to start, naming the SDK, target or toolchain that is missing.
#
# The third is the one this script is really for. Every one of these builds
# fails in a confusing way when a prerequisite is absent — a missing Rust target
# looks like a broken checkout, a missing NDK looks like a Rust linker bug (see
# `ci/mobile/apk.sh`'s comment on `-lunwind`), and a missing Xcode looks like nothing
# at all until forty seconds in. Naming the prerequisite up front turns each of
# those into one line.
#
# # What passing here does and does not mean
#
# It means the code **compiles** for that platform. It does not mean a frame
# has ever reached a screen there, and this script says so on every success,
# because the distinction is the entire subject of `TRACKER.md`'s status column
# and it is the one that gets quietly dropped when a green check appears.
#
# Running an application is a different claim with different scripts:
# `ci/mobile/ios-app.sh`, `ci/mobile/apk.sh`, `ci/mobile/device-suite.sh`.

set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

host="$(uname -s)"
requested="${1:-all}"
failures=0

# Validated up front rather than inferred afterwards from "did any block run".
# That inference was the first shape of this and it was wrong: a platform that
# matched and then *skipped* — the web check on a machine with no wasm target,
# which is most of them — looked identical to a typo, and reported itself as
# "unknown platform: web" straight after printing three paragraphs about web.
case "$requested" in
all | linux | macos | windows | ios | android | web) ;;
*)
	echo "unknown platform: $requested" >&2
	echo "one of: linux macos windows ios android web" >&2
	exit 2
	;;
esac

# ── preflight: is cargo usable at all on this machine? ─────────────────────
#
# `rust-toolchain.toml` pins the `stable` channel, and rustup re-syncs a
# pinned channel's manifest before running *any* proxied command — so on a
# machine that cannot reach `static.rust-lang.org`, plain `cargo --version`
# exits 1 under a twenty-six frame backtrace, with the already-installed
# toolchain sitting right there on disk.
#
# Without this check every platform below then reports "see the compiler output
# above" for a compiler that never ran, which is the most misleading answer this
# script could give: it blames the code for a network policy. `TRACKER.md`
# records the same confusion costing real time once already, in viewwstudio's
# toolchain test.
if ! cargo --version >/dev/null 2>&1; then
	echo "error: \`cargo --version\` does not work on this machine." >&2
	echo >&2
	if rustup toolchain list 2>/dev/null | grep -q .; then
		echo "       A toolchain *is* installed. rust-toolchain.toml pins the" >&2
		echo "       'stable' channel and rustup re-syncs a pinned channel before" >&2
		echo "       running anything, which fails when static.rust-lang.org is" >&2
		echo "       unreachable — the installed toolchain is never consulted." >&2
		echo >&2
		echo "       To use what is already installed and skip the sync:" >&2
		echo >&2
		echo "           RUSTUP_TOOLCHAIN=stable ci/check/platform-check.sh $requested" >&2
	else
		echo "       No Rust toolchain is installed. See https://rustup.rs." >&2
	fi
	exit 2
fi

note() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
skip() { printf '   skipped: %s\n' "$*"; }
pass() { printf '   \033[32mOK\033[0m — %s\n' "$*"; }
fail() {
	printf '   \033[31mFAILED\033[0m — %s\n' "$*"
	failures=$((failures + 1))
}

# Is this platform one the caller asked for?
wanted() { [ "$requested" = "all" ] || [ "$requested" = "$1" ]; }

# Is a Rust target installed? Answered with stderr discarded — see
# `ci/check/wasm-check.sh`'s stage 1 for why a rustup query is not trustworthy on a
# machine that cannot reach `static.rust-lang.org`.
has_target() { rustup target list --installed 2>/dev/null | grep -qx "$1"; }

# ── linux ──────────────────────────────────────────────────────────────────
#
# The verified platform, and included anyway: a per-platform report whose
# verified row is missing invites the reader to assume the others are like it.
if wanted linux; then
	note "linux — the host build, and the Vulkan backend"
	if [ "$host" != "Linux" ]; then
		skip "this is $host; run on Linux"
	elif cargo check --workspace --all-targets --quiet &&
		cargo check -p vieww-hal --features vulkan --all-targets --quiet; then
		pass "the workspace and the Vulkan backend compile"
		echo "        (\`ci/check/checks.sh\` is the full gate; this is only the compile)"
	else
		fail "see the compiler output above"
	fi
fi

# ── macos ──────────────────────────────────────────────────────────────────
if wanted macos; then
	note "macos — the Metal backend"
	if [ "$host" != "Darwin" ]; then
		# Not attempted rather than attempted-and-failed. The `metal` crate is
		# declared under `[target.'cfg(target_os = "macos")'.dependencies]`, so
		# enabling the feature elsewhere resolves to nothing at all and would
		# report a hollow success — see `crates/vieww-hal/Cargo.toml`'s own
		# comment on that shape.
		skip "this is $host; the metal feature resolves to nothing off macOS, so a pass here would mean nothing"
	elif ! xcode-select -p >/dev/null 2>&1; then
		fail "no Xcode command line tools — run: xcode-select --install"
	elif cargo check -p vieww-hal --features metal --all-targets; then
		pass "vieww-hal's Metal backend compiles"
		echo "        Next, and this is the actual gap: \`src/metal.rs\`'s"
		echo "        \`render_clear_to_pixels\` is not ported. That module's doc"
		echo "        lists the exact calls the port needs, in order."
	else
		fail "see the compiler output above — this is the first real feedback this code has had"
	fi
fi

# ── windows ────────────────────────────────────────────────────────────────
if wanted windows; then
	note "windows — the D3D12 backend"
	case "$host" in
	MINGW* | MSYS* | CYGWIN* | Windows_NT)
		if cargo check -p vieww-hal --features d3d12 --all-targets; then
			pass "vieww-hal's D3D12 backend compiles"
			echo "        Note: \`.cargo/config.toml\` deliberately does not set"
			echo "        \`-C prefer-dynamic\` for MSVC — that toolchain ships no"
			echo "        dynamic std, so viewwstudio's guest-panic boundary needs"
			echo "        an out-of-process preview here. See that file."
		else
			fail "see the compiler output above"
		fi
		;;
	*)
		skip "this is $host; the d3d12 feature resolves to nothing off Windows"
		;;
	esac
fi

# ── ios ────────────────────────────────────────────────────────────────────
if wanted ios; then
	note "ios — the device triple"
	target="aarch64-apple-ios"
	if [ "$host" != "Darwin" ]; then
		skip "this is $host; the iOS SDK exists only on macOS"
	elif ! has_target "$target"; then
		fail "$target is not installed — run: rustup target add $target"
	elif ! xcrun --sdk iphoneos --show-sdk-path >/dev/null 2>&1; then
		fail "no iPhoneOS SDK — install Xcode (not just the command line tools)"
	elif cargo check -p vieww-platform-winit --target "$target"; then
		pass "the platform layer compiles for a real iPhone"
		echo "        \`ci/mobile/ios-app.sh --sim\` is the next claim: a frame on a screen."
	else
		fail "see the compiler output above"
	fi
fi

# ── android ────────────────────────────────────────────────────────────────
if wanted android; then
	note "android — the arm64 device triple"
	target="aarch64-linux-android"
	if ! has_target "$target"; then
		fail "$target is not installed — run: rustup target add $target"
	# `android-env.sh` already knows how to find a usable NDK, and knows the
	# traps (a version directory with no `source.properties` is a half-finished
	# download). Asking it is the point of it existing.
	elif ! (. ci/mobile/android-env.sh && android_env "platform-check") >/dev/null 2>&1; then
		fail "no usable Android SDK/NDK — run \`. ci/mobile/android-env.sh && android_env x\` to see why"
	elif (
		. ci/mobile/android-env.sh && android_env "platform-check" >/dev/null 2>&1
		cargo check -p vieww-platform-winit --target "$target"
	); then
		pass "the platform layer compiles for arm64 Android"
		echo "        \`ci/mobile/apk.sh --example android\` is the next claim, and"
		echo "        \`ci/mobile/device-suite.sh\` the one after it."
	else
		fail "see the compiler output above"
	fi
fi

# ── web ────────────────────────────────────────────────────────────────────
if wanted web; then
	note "web — wasm32, the crate that has never been compiled"
	# Delegated rather than duplicated: that script has a stage that needs no
	# toolchain at all, and re-implementing half of it here is how two scripts
	# come to disagree about what "checked" means.
	if ci/check/wasm-check.sh; then
		pass "vieww-platform-web compiles for wasm32 — update TRACKER.md and delete that crate's banner"
	elif [ $? -eq 2 ]; then
		skip "the wasm target cannot be installed here; see the message above"
	else
		fail "see the output above — expected on the first run, and it is ordinary work from here"
	fi
fi

# ── summary ────────────────────────────────────────────────────────────────
printf '\n'
if [ "$failures" -eq 0 ]; then
	echo "no platform check failed on this machine."
	echo
	echo "Compiling is not running. A green line above means the code type-checks"
	echo "for that platform; no frame has reached a screen on macOS, Windows, iOS,"
	echo "Android or the web, and TRACKER.md should keep saying so until one has."
else
	echo "$failures platform check(s) failed."
	echo
	echo "That is the intended use of this script rather than a problem with it:"
	echo "this code has never been compiled for most of these targets, and the"
	echo "first compiler to see it is expected to have things to say."
fi
exit "$failures"
