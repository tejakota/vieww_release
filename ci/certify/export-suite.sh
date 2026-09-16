#!/usr/bin/env bash
#
# Studio's export route, for real: scaffold a project, compile it for every
# target, then run the Export sheet's own plans (desktop, Windows .exe, Android
# APK, iOS simulator app) and check each produced its artefact.
#
#   ci/vieww export [OUT] [--compile-only] [--install-targets] [--only a,b]
#
#   OUT                default $TMPDIR/vieww-export-check — deliberately OUTSIDE
#                      this repository: cargo reads .cargo/config.toml from every
#                      directory above a project, and this repo's (prefer-dynamic,
#                      its own NDK linker shims) would silently change how the
#                      scaffolded project builds. A user's project is not here.
#   --compile-only     `cargo check --target` only: needs rustup targets, no SDKs
#   --install-targets  `rustup target add` the cross targets first (CI; changes
#                      the machine's toolchain, so not the default)
#   --only             desktop,windows,android,ios,ios-sim,ios-ipa
#
# What each export needs (the same list Studio's Toolchains view shows):
#   Android APK   Android SDK (ANDROID_HOME), NDK (ANDROID_NDK_HOME or
#                 <sdk>/ndk/*), JDK 17, cargo-ndk, Gradle 8.7+,
#                 rustup target aarch64-linux-android
#   iOS sim .app  macOS, Xcode, rustup target aarch64-apple-ios-sim
#   iOS .ipa      the above plus a signing identity (not automated)
#   Windows .exe  on Windows: nothing extra; elsewhere: MinGW-w64 and
#                 rustup target x86_64-pc-windows-gnu
# A missing toolchain is reported as REFUSED with its install command, exactly
# as Studio would tell a user — not as a failure.
#
# A scaffolded project pins `channel = "stable"` in its own rust-toolchain.toml,
# so targets must be installed for *that* toolchain as well as this repo's.
#
# Output: OUT/results.txt (one line per result) and OUT/logs/. The scaffolded
# projects and their build trees live under OUT too and are large (several GB);
# the release gate copies results and logs out and deletes the rest.

set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root" || exit 2
# shellcheck source=ci/lib/lean.sh
. "$root/ci/lib/lean.sh"
vieww_lean_env

out="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/vieww-export-check"
args=()
install_targets=""
while (($# > 0)); do
	case "$1" in
	--compile-only) args+=(--compile-only) ;;
	--install-targets) install_targets=yes ;;
	--only) args+=(--only "$2"); shift ;;
	-h | --help) sed -n '2,33p' "${BASH_SOURCE[0]}"; exit 0 ;;
	-*) echo "export-suite: unknown argument $1" >&2; exit 2 ;;
	*) out="$1" ;;
	esac
	shift
done

case "$(uname -s)" in
Darwin) host=macos ;;
MINGW* | MSYS* | CYGWIN*) host=windows ;;
*) host=linux ;;
esac

if [[ -n "$install_targets" ]]; then
	targets=(aarch64-linux-android)
	[[ "$host" == windows ]] || targets+=(x86_64-pc-windows-gnu)
	[[ "$host" == macos ]] && targets+=(aarch64-apple-ios aarch64-apple-ios-sim)
	for toolchain in "$(rustup show active-toolchain 2>/dev/null | cut -d' ' -f1)" stable; do
		[[ -n "$toolchain" ]] || continue
		echo "export-suite: rustup target add --toolchain $toolchain ${targets[*]}"
		rustup toolchain install "$toolchain" --profile minimal >/dev/null 2>&1 || true
		rustup target add --toolchain "$toolchain" "${targets[@]}" || true
	done
fi

# Newer NDKs are what cargo-ndk and AGP expect; hosted runners set both.
if [[ -z "${ANDROID_NDK_HOME:-}" && -n "${ANDROID_NDK_LATEST_HOME:-}" ]]; then
	export ANDROID_NDK_HOME="$ANDROID_NDK_LATEST_HOME"
fi

cargo build --release -p viewwstudio --example export_check || exit 2
case "$out" in
"$root"/*) echo "export-suite: OUT must be outside the repository (see --help)" >&2; exit 2 ;;
esac
mkdir -p "$out"
bin="target/release/examples/export_check"
[[ "$host" == windows ]] && bin="$bin.exe"
"$bin" "$out" ${args[@]+"${args[@]}"}
