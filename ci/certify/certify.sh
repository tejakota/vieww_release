#!/usr/bin/env bash
#
# Vieww certification for Linux and macOS — every suite in the tree, one
# command, one verdict. (Windows: `ci/certify/certify-windows.ps1`.)
#
#   ci/vieww certify [OUT]                # default OUT: target/cert-<platform>
#   VIEWW_CERT_STRICT=1 ci/vieww certify   # any SKIPPED stage fails the run
#
# The GPU/CPU axis of a release matrix is selected with environment variables:
#
#   VIEWW_CERT_NO_GPU=1          CPU-only run: every GPU execution stage is
#                                SKIPPED("CPU-only run requested"), builds still run
#   VIEWW_CERT_REQUIRE_REAL_GPU=1  fail `gpu/device-class` unless the Vulkan device
#                                is real hardware (llvmpipe, lavapipe and
#                                SwiftShader are software rasterizers — they prove
#                                correctness, never GPU certification)
#   VIEWW_CERT_NO_DESKTOP=1      skip the interactive desktop suite (headless box)
#
# On macOS the Vulkan stages need MoltenVK through the Vulkan SDK
# (`vulkaninfo` on PATH); without it they are SKIPPED, not passed. Metal is
# recorded as BUILD-ONLY: it compiles, it does not execute a ScenePlan.
#
# # What changed, and why
#
# The previous version of this script ran fmt/check/clippy, the workspace
# tests, the Vulkan suites, `test-gpu-work`, `test-premium-ui` and the
# fixtures — and **not** `test-layout-stress`, `test-animation-stress`,
# `test-text-fidelity`, `test-image-effects`, `test-scroll-stress`,
# `test-native-surface`, `vieww-standard` or the web backend. The suites
# existed; the certification command did not prove them. Every suite is a
# stage here now, and `STAGES` below is the list a reviewer checks against
# `examples/`.
#
# # Evidence is tied to the source it came from
#
# `OUT` is **emptied first**, and `meta/source.txt` records a digest of every
# source file the build reads. A bundle whose digest does not match the tree
# next to it is historical evidence, not a certification of that tree — which
# is exactly the confusion a stale `cert-linux/build/fmt.txt` shipped inside a
# source archive caused. The default `OUT` is under `target/`, so a source
# archive never picks evidence up by accident.
#
# # Skips are not passes
#
# A stage that cannot run on this machine — no Vulkan ICD, no wasm target, no
# Playwright, no display — is recorded as `SKIPPED(reason)`, never as `PASS`.
# The summary's `COMPLETE=` line is `true` only when nothing was skipped, and
# with `VIEWW_CERT_STRICT=1` any skip fails the run. A certification of a
# machine that could not run half of it should say so on its first line.
#
# Exit code: 0 when nothing failed (and, in strict mode, nothing was skipped).

set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
case "$(uname -s)" in
Darwin) PLATFORM=macos ;;
Linux) PLATFORM=linux ;;
*) echo "certify.sh: unsupported host $(uname -s); on Windows use ci/certify/certify-windows.ps1" >&2; exit 2 ;;
esac
OUT="${1:-$ROOT/target/cert-$PLATFORM}"
STRICT="${VIEWW_CERT_STRICT:-0}"
# VIEWW_CERT_ONLY=gpu: only what a GPU can change — device class, the Vulkan
# compositor parity suite, the GPU workload, the census and the real-window
# desktop suite. Everything else here renders on the CPU and gives the same
# answer with or without a GPU, so a machine that already ran the CPU pass
# (or CI) does not need to repeat it. Builds a small slice of the workspace.
ONLY="${VIEWW_CERT_ONLY:-}"
cd "$ROOT" || exit 2
# shellcheck source=ci/lib/lean.sh
. "$ROOT/ci/lib/lean.sh"
vieww_lean_env

rm -rf "$OUT"
mkdir -p "$OUT"/{meta,build,tests,gpu,visual,suites,standard,web,desktop,quality}

FAILED=0
SKIPPED=0
: >"$OUT/quality/stages.txt"

# GNU coreutils on Linux, `shasum` (Perl) on macOS.
if command -v sha256sum >/dev/null; then SHA256=(sha256sum); else SHA256=(shasum -a 256); fi

record() { printf '%-28s %s\n' "$1" "$2" >>"$OUT/quality/stages.txt"; }

# run NAME CMD... — capture output in OUT/NAME.txt, record PASS/FAIL.
run() {
	local name="$1"
	shift
	local file="$OUT/$name.txt"
	mkdir -p "$(dirname "$file")"
	echo ">>> $*" | tee "$file"
	local start=$SECONDS
	"$@" >>"$file" 2>&1
	local rc=$?
	echo "exit=$rc" >>"$file"
	echo "seconds=$((SECONDS - start))" >>"$file"
	if ((rc == 0)); then
		record "$name" PASS
	else
		record "$name" "FAIL(exit=$rc)"
		FAILED=1
		echo "    FAILED: $name (see $file)"
	fi
	return $rc
}

skip() {
	local name="$1" reason="$2"
	mkdir -p "$(dirname "$OUT/$name.txt")"
	echo "SKIPPED($reason)" >"$OUT/$name.txt"
	record "$name" "SKIPPED($reason)"
	SKIPPED=$((SKIPPED + 1))
	echo "    skipped: $name — $reason"
}

# ── meta ─────────────────────────────────────────────────────────────────────
{
	echo "date=$(date --iso-8601=seconds 2>/dev/null || date)"
	echo "hostname=$(hostname)"
	echo "platform=$PLATFORM"
	echo "os=$(uname -a)"
	if [[ "$PLATFORM" == macos ]]; then
		echo "macos_version=$(sw_vers -productVersion 2>/dev/null)"
		echo "cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null)"
	else
		echo "cpu=$(sed -n 's/^model name[[:space:]]*:[[:space:]]*//p' /proc/cpuinfo 2>/dev/null | head -1)"
	fi
	echo "no_gpu=${VIEWW_CERT_NO_GPU:-0}"
	echo "require_real_gpu=${VIEWW_CERT_REQUIRE_REAL_GPU:-0}"
	echo "arch=$(uname -m)"
	echo "rust=$(rustc --version 2>/dev/null | head -1)"
	echo "cargo=$(cargo --version 2>&1 | head -1)"
	echo "toolchain_file=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)"
	echo "display=${DISPLAY:-}"
	echo "wayland=${WAYLAND_DISPLAY:-}"
	echo "vulkaninfo=$(command -v vulkaninfo || true)"
	if command -v vulkaninfo >/dev/null; then
		vulkaninfo --summary 2>/dev/null | sed -n 's/^[[:space:]]*deviceName[[:space:]]*=[[:space:]]*/vulkan_device=/p' | head -3
	fi
	echo "strict=$STRICT"
} >"$OUT/meta/host.txt"
{
	echo "git_rev=$(git rev-parse HEAD 2>/dev/null || echo none)"
	echo "git_dirty=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')"
	# Every file the build reads, content-hashed. Sorted paths so the digest
	# is a function of the tree, not of `find`'s order.
	digest=$(find Cargo.toml Cargo.lock rust-toolchain.toml crates examples apps ci scripts \
		-type f \( -name '*.rs' -o -name '*.toml' -o -name '*.wgsl' -o -name '*.sh' \
		-o -name '*.ps1' -o -name '*.py' -o -name '*.lock' -o -name '*.ttf' -o -name '*.otf' \
		-o -name '*.html' \) -not -path '*/target/*' -not -path '*/dist/*' -print0 2>/dev/null |
		sort -z | xargs -0 "${SHA256[@]}" | "${SHA256[@]}" | cut -d' ' -f1)
	# In a git checkout the digest is git's own content address for the
	# committed tree, plus a marker if the working tree differs. It is the same
	# on every OS; a hash of files on disk is not: Windows checks text files out
	# with CRLF line endings, and the Windows and Unix scripts hashed differently,
	# so two runs of one commit reported different digests (G0.12).
	if tree=$(git rev-parse 'HEAD^{tree}' 2>/dev/null); then
		if [[ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]]; then
			digest="git-tree:$tree+modified"
		else
			digest="git-tree:$tree"
		fi
	fi
	echo "source_digest=$digest"
} >"$OUT/meta/source.txt"

have_vulkan() {
	command -v vulkaninfo >/dev/null && vulkaninfo --summary >/dev/null 2>&1
}
vulkan_device() {
	vulkaninfo --summary 2>/dev/null | sed -n 's/^[[:space:]]*deviceName[[:space:]]*=[[:space:]]*//p' | head -1
}

if [[ "$ONLY" != gpu ]]; then
# ── build ────────────────────────────────────────────────────────────────────
run build/release-clean bash ci/check/release-clean-check.sh "$ROOT"
# `VIEWW_CERT_BUILD_COVERED=1` is set by `ci/vieww gate` when ci/check/checks.sh
# already passed fmt, clippy, check and the workspace tests on this tree in the
# same run: rerunning them proves nothing new and costs an hour. Recorded as
# COVERED, which is a pass with its evidence elsewhere, not a skip.
covered() { record "$1" "COVERED(ci/check/checks.sh passed it in this gate run)"; }
if [[ "${VIEWW_CERT_BUILD_COVERED:-0}" == 1 ]]; then
	for stage in build/fmt build/check build/clippy-core tests/workspace; do covered "$stage"; done
else
	run build/fmt cargo fmt --all -- --check
	run build/check cargo check --workspace --all-targets
	run build/clippy-core cargo clippy --keep-going \
		-p vieww-gpu -p vieww-hal -p vieww-render-planner -p vieww-shaders -p vieww-paint \
		-p vieww-render -p vieww-element --all-targets --features vieww-hal/vulkan -- -D warnings
fi

# ── tests ────────────────────────────────────────────────────────────────────
# --no-fail-fast: one aborting test binary must not hide every binary after it.
# The same features as ci/check/checks.sh, so the two share one debug build
# rather than compiling the workspace twice under different feature sets.
[[ "${VIEWW_CERT_BUILD_COVERED:-0}" == 1 ]] ||
	run tests/workspace cargo test --workspace --no-fail-fast --features vieww-paint/native
run tests/gpu-planner cargo test -p vieww-gpu
run tests/shaders cargo test -p vieww-shaders
run tests/quality cargo test -p vieww-render-planner quality
run tests/paint-native cargo test -p vieww-paint --features native

fi  # ONLY != gpu

# ── GPU ──────────────────────────────────────────────────────────────────────
run gpu/hal-build cargo check -p vieww-hal --features vulkan
if [[ "$PLATFORM" == macos ]]; then
	run gpu/metal-build cargo check -p vieww-hal --features metal --all-targets
	record gpu/metal-backend "BUILD-ONLY(metal does not execute a ScenePlan yet; not certified)"
fi
if [[ "${VIEWW_CERT_NO_GPU:-0}" == 1 ]]; then
	for stage in gpu/device-class gpu/adapter gpu/vulkan-suite gpu/workload gpu/census; do
		skip "$stage" "CPU-only run requested"
	done
elif have_vulkan; then
	# The device the framework itself picks (discrete, then integrated; override
	# with VIEWW_VK_DEVICE=<name substring>), not merely the first one
	# vulkaninfo lists — on a laptop with two GPUs those differ.
	run gpu/adapter cargo run --release -p vieww-hal --example adapter --features vulkan
	device="$(sed -n 's/^name=//p' "$OUT/gpu/adapter.txt" | head -1)"
	[[ -n "$device" ]] || device="$(vulkan_device)"
	{
		echo "vulkan_device=$device"
		echo "vk_device_override=${VIEWW_VK_DEVICE:-}"
		vulkaninfo 2>/dev/null | grep -iE 'heapSize|size *=.*(GiB|MiB)' | head -4 | sed 's/^[[:space:]]*/heap: /'
	} >"$OUT/gpu/device.txt"
	if echo "$device" | grep -qiE 'llvmpipe|lavapipe|swiftshader|software'; then
		echo "device_class=software" >>"$OUT/gpu/device.txt"
		if [[ "${VIEWW_CERT_REQUIRE_REAL_GPU:-0}" == 1 ]]; then
			record gpu/device-class "FAIL(software rasterizer: $device)"
			FAILED=1
		else
			record gpu/device-class "SOFTWARE($device; correctness only, not a GPU certification)"
		fi
	elif echo "$device" | grep -qiE 'paravirtual|virtio|virgl|venus'; then
		# A hypervisor passing a real host GPU through (GitHub's macOS runners:
		# "Apple Paravirtual device"). Real GPU execution, so the parity suite
		# and census are evidence; the timings belong to a shared host.
		echo "device_class=virtual" >>"$OUT/gpu/device.txt"
		record gpu/device-class "PASS(virtual GPU: $device; pixels are evidence, timings are not)"
	else
		echo "device_class=hardware" >>"$OUT/gpu/device.txt"
		record gpu/device-class "PASS($device)"
	fi
	if ! run gpu/vulkan-suite cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1; then
		# A parity failure on one driver is usually undefined behaviour another
		# driver forgave. Re-run the compositor tests under the Khronos
		# validation layer (core + synchronization) so the evidence names it.
		if vulkaninfo 2>/dev/null | grep -q VK_LAYER_KHRONOS_validation; then
			vlog="$OUT/gpu/vulkan-validation.txt"
			VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation \
				VK_LAYER_ENABLES=VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT \
				cargo test -p vieww-hal --features vulkan --test vulkan_compositor -- --ignored --test-threads=1 --nocapture >"$vlog" 2>&1 || true
			n=$(grep -c 'Validation Error' "$vlog" || true)
			record gpu/vulkan-validation "INFO(${n:-0} validation errors; see gpu/vulkan-validation.txt)"
		else
			record gpu/vulkan-validation "INFO(validation layer not installed: apt install vulkan-validationlayers)"
		fi
	fi
	run gpu/workload cargo run --release -p test-gpu-work -- "$OUT/gpu/workload"
	run gpu/census cargo run --release -p fixtures -- "$OUT/gpu/census-out" --census
else
	for stage in gpu/device-class gpu/adapter gpu/vulkan-suite gpu/workload gpu/census; do
		skip "$stage" "no Vulkan loader/ICD$([[ "$PLATFORM" == macos ]] && echo ' (install the Vulkan SDK / MoltenVK)')"
	done
fi

if [[ "$ONLY" != gpu ]]; then
# ── visual (CPU renderer) ────────────────────────────────────────────────────
run visual/premium cargo run --release -p test-premium-ui -- "$OUT/visual/premium"
run visual/fixtures cargo run --release -p fixtures -- "$OUT/visual/fixtures"

# ── the stress and fidelity suites ───────────────────────────────────────────
STAGES=(
	test-layout-stress
	test-animation-stress
	test-text-fidelity
	test-image-effects
	test-scroll-stress
	test-native-surface
)
for suite in "${STAGES[@]}"; do
	run "suites/${suite#test-}" cargo run --release -p "$suite" -- "$OUT/suites/${suite#test-}"
done

# ── the Vieww standard (all twelve clauses, measured) ────────────────────────
# `RUSTFLAGS=` (set, empty) replaces `.cargo/config.toml`'s `-C prefer-dynamic`
# for this one binary: a dynamically linked `std` ignores `#[global_allocator]`,
# and the runner refuses to report `steady_allocations` it could not count. A
# separate target directory keeps the two flag sets from rebuilding each other.
run standard/vieww-standard env RUSTFLAGS= CARGO_TARGET_DIR="$ROOT/target/static-std" \
	cargo run --release -p vieww-standard -- "$OUT/standard/vieww-standard"
# A whole release tree built only for this one binary.
vieww_prune "$ROOT" static-std

# ── web ──────────────────────────────────────────────────────────────────────
if rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
	run web/wasm-check bash ci/check/wasm-check.sh
	if command -v wasm-bindgen >/dev/null; then
		run web/build bash examples/test-web/build-web.sh "$OUT/web/dist"
	else
		skip web/build "wasm-bindgen not on PATH"
	fi
else
	skip web/wasm-check "wasm32-unknown-unknown target not installed"
	skip web/build "wasm32-unknown-unknown target not installed"
fi
run web/baseline cargo run --release -p test-web --example baseline -- "$OUT/web/baseline"
if [[ -f "$OUT/web/dist/test_web_bg.wasm" ]]; then
	python3 examples/test-web/verify_web.py "$OUT/web/dist" "$OUT/web/baseline" >"$OUT/web/verify.txt" 2>&1
	rc=$?
	echo "exit=$rc" >>"$OUT/web/verify.txt"
	case $rc in
	0) record web/verify PASS ;;
	3) record web/verify "SKIPPED($(sed -n 's/^web.verify=MACHINE-SKIPPED(\(.*\))/\1/p' "$OUT/web/verify.txt"))"; SKIPPED=$((SKIPPED + 1)) ;;
	*) record web/verify "FAIL(exit=$rc)"; FAILED=1 ;;
	esac
else
	skip web/verify "no freshly built wasm to verify"
fi

fi  # ONLY != gpu

# ── desktop ──────────────────────────────────────────────────────────────────
if [[ "${VIEWW_CERT_NO_DESKTOP:-0}" == 1 ]]; then
	skip desktop/suite "VIEWW_CERT_NO_DESKTOP=1"
elif [[ "$PLATFORM" == macos || -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" ]]; then
	run desktop/suite bash ci/certify/desktop-suite.sh --log "$OUT/desktop/app.log"
else
	skip desktop/suite "no graphical session"
fi

# ── quality evidence ─────────────────────────────────────────────────────────
metric() { sed -n "s/^$2=//p" "$1" 2>/dev/null | head -1; }
{
	echo "platform=$PLATFORM"
	for file in "$OUT"/gpu/workload/metrics.txt "$OUT"/visual/premium/metrics.txt "$OUT"/suites/*/metrics.txt; do
		[[ -f "$file" ]] || continue
		echo "# ${file#"$OUT"/}"
		cat "$file"
	done
	if [[ -f "$OUT/standard/vieww-standard/vieww-standard.json" ]]; then
		echo "# standard/vieww-standard.json"
		cat "$OUT/standard/vieww-standard/vieww-standard.json"
	fi
	if [[ -f "$OUT/gpu/census.txt" ]] && ! grep -q '^SKIPPED(' "$OUT/gpu/census.txt"; then
		echo "# gpu/census verdict"
		grep -E "fixtures plan complete|beyond the geometry-edge band|^VERDICT" "$OUT/gpu/census.txt"
	fi
} >"$OUT/quality/metrics.txt"

# Gates on measured values — not on a suite's say-so alone.
gate() {
	local name="$1" ok="$2"
	if [[ "$ok" == 1 ]]; then record "gate/$name" PASS; else record "gate/$name" FAIL; FAILED=1; fi
}
if [[ -f "$OUT/gpu/workload/metrics.txt" ]]; then
	gate gpu-unsupported-zero "$([[ "$(metric "$OUT/gpu/workload/metrics.txt" unsupported_gpu_commands)" == 0 ]] && echo 1 || echo 0)"
fi
if [[ -f "$OUT/visual/premium/metrics.txt" ]]; then
	gate premium-overflows-zero "$([[ "$(metric "$OUT/visual/premium/metrics.txt" overflows)" == 0 ]] && echo 1 || echo 0)"
fi
std_json="$OUT/standard/vieww-standard/vieww-standard.json"
if [[ -f "$std_json" ]]; then
	gate standard-passed "$(grep -q '"passed": true' "$std_json" && echo 1 || echo 0)"
	gate standard-allocations-measured "$(grep -q '"allocation_counter_installed": true' "$std_json" && echo 1 || echo 0)"
elif [[ "$ONLY" != gpu ]]; then
	gate standard-report-present 0
fi
# Only when the census actually ran: a skipped stage leaves a census.txt that
# says SKIPPED, and gating on it turned every CPU-only run into two FAILs.
if [[ -f "$OUT/gpu/census.txt" ]] && ! grep -q '^SKIPPED(' "$OUT/gpu/census.txt"; then
	gate census-all-complete "$(grep -qE '^([0-9]+) of \1 fixtures plan complete' "$OUT/gpu/census.txt" && echo 1 || echo 0)"
	gate census-no-flat-region-mismatch "$(grep -q 'beyond the geometry-edge band: 0 of' "$OUT/gpu/census.txt" && echo 1 || echo 0)"
fi

COMPLETE=true
((SKIPPED > 0)) && COMPLETE=false
if [[ "$STRICT" == 1 && "$SKIPPED" -gt 0 ]]; then
	FAILED=1
fi

{
	echo "Vieww $PLATFORM certification"
	echo "==========================="
	echo "COMPLETE=$COMPLETE (skipped stages: $SKIPPED)"
	echo "FAILED=$FAILED"
	cat "$OUT/meta/source.txt"
	echo
	cat "$OUT/quality/stages.txt"
} >"$OUT/summary.txt"
cat "$OUT/summary.txt"
exit "$FAILED"
