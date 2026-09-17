#!/usr/bin/env bash
#
# The single release script: runs every other check, certification and
# packaging script for this machine, then fills in
# docs/release/BETA-RELEASE-CHECKLIST.md and writes the checked copy.
#
#   ci/vieww gate --mode cpu         # the full pass with GPU execution skipped (CI, any machine)
#   ci/vieww gate --mode gpu         # the narrow GPU pass, after a cpu pass: device class,
#                                    # Vulkan parity suite, GPU workload, census, desktop
#                                    # suite. Software Vulkan is a FAIL. ~20-30 min, small build.
#                                    # Pick the GPU with VIEWW_VK_DEVICE=<name substring>.
#   ci/vieww gate --mode all         # everything in one run (GPU stages if Vulkan works)
#   ci/vieww gate --skip-package     # no installers (e.g. a second run on the same OS)
#   ci/vieww gate --out DIR          # default target/release-gate/<os>-<mode>-<stamp>
#   ci/vieww gate --hidpi            # also run the desktop suite expecting HiDPI (G2.3)
#   ci/vieww gate --multi-monitor    # ... expecting two displays (G2.4)
#   ci/vieww gate --against DIR      # diff feature screenshots against another OS's shots/ (G1.13)
#   ci/vieww gate --skip-shots       # don't capture feature screenshots
#   ci/vieww gate --keep-builds      # don't delete target/debug, target/release, ... as phases finish
#   ci/vieww gate --keep-evidence    # keep every image of stages that passed
#   ci/vieww gate --only G1.1,G3.1   # rerun just these rows (e.g. the ones that failed);
#                                    # G1.3 brings its stage rows (G1.4-G1.12), G3.1 brings G8.*
#   VIEWW_FULL_DEBUG=1 ci/vieww gate # debuginfo + incremental builds (needs far more disk)
#
# Disk: builds are lean by default (ci/lib/lean.sh: no debuginfo, no
# incremental caches), and each build tree is deleted as soon as the last
# stage that needs it is done: target/doc and the MSRV tree after the docs and
# MSRV checks, target/debug and target/static-std after certification,
# target/release and the package staging trees at the end. logs/disk.tsv
# records target/ size and free space after every row.
#
# After every machine has run, combine them into one checklist:
#   ci/vieww checklist linux-gpu-*/ linux-cpu-*/ windows-gpu-*/ macos-gpu-*/ -o CHECKLIST.md
#
# Linux and macOS: bash. Windows: Git Bash (certification runs through
# pwsh/powershell). Every row runs even when an earlier one fails, so one run
# shows every problem. Exit status is non-zero if any automated row FAILED.
#
# Output:
#   CHECKLIST.md    the release checklist with this run's cells checked
#   rows.tsv        row id, check, verdict, evidence (input to ci/vieww checklist)
#   host.txt        the "Platform evidence record" fields, filled in
#   logs/<row>.txt  the full output of each row
#   cert/           the certification bundle (ci/vieww certify)
#   studio/         the Studio screenshot gallery (open index.html)
#   package/        SHA256SUMS + MANIFEST.txt of the built artifacts
#   shots/          feature screenshots, for --against on the other OSes
#
# The rows a person must judge (visual review, clean-machine install, signing,
# upgrade) stay ☐ in CHECKLIST.md; nothing here marks them.

set -uo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root" || exit 2

mode=auto
package=yes
out=""
hidpi=""
multi=""
against=""
shots=yes
keep_builds=""
keep_evidence=""
only=""
while (($# > 0)); do
	case "$1" in
	--mode) mode="$2"; shift ;;
	--mode=*) mode="${1#--mode=}" ;;
	--skip-package) package="" ;;
	--out) out="$2"; shift ;;
	--hidpi) hidpi=yes ;;
	--multi-monitor) multi=yes ;;
	--against) against="$2"; shift ;;
	--skip-shots) shots="" ;;
	--keep-builds) keep_builds=yes ;;
	--keep-evidence) keep_evidence=yes ;;
	--only) only="$2"; shift ;;
	--only=*) only="${1#--only=}" ;;
	-h | --help) sed -n '2,48p' "${BASH_SOURCE[0]}"; exit 0 ;;
	*) echo "gate: unknown argument $1" >&2; exit 2 ;;
	esac
	shift
done
case "$mode" in auto | all) mode=auto ;; cpu | gpu) ;; *) echo "gate: --mode must be cpu, gpu or all" >&2; exit 2 ;; esac

case "$(uname -s)" in
Darwin) os=macos ;;
Linux) os=linux ;;
MINGW* | MSYS* | CYGWIN*) os=windows ;;
*) echo "gate: unsupported host $(uname -s)" >&2; exit 2 ;;
esac

# shellcheck source=ci/lib/lean.sh
. "$root/ci/lib/lean.sh"
vieww_lean_env

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
out="${out:-$root/target/release-gate/$os-$mode-$stamp}"
mkdir -p "$out/logs"
results="$out/rows.tsv"
: >"$results"
failed=0

# row ID DESCRIPTION CMD... — run, log, record PASS/FAIL.
# wanted ID — is this row part of the run? Everything is, unless --only.
wanted() {
	[[ -z "$only" ]] && return 0
	local id="$1" list=",${only// /},"
	[[ "$id" == PRE.* ]] && return 0
	[[ "$list" == *",$id,"* ]] && return 0
	# Rows read out of another row's output come with it.
	[[ "$id" =~ ^G1\.([4-9]|1[0-2])$ && "$list" == *",G1.3,"* ]] && return 0
	[[ "$id" == G8.* && "$list" == *",G3.1,"* ]] && return 0
	[[ "$id" == G1.13x && "$list" == *",G1.13,"* ]] && return 0
	return 1
}

row() {
	wanted "$1" || return 0
	local id="$1" what="$2"
	shift 2
	local log="$out/logs/$id.txt"
	echo "==> $id $what"
	{
		echo ">>> $*"
		date
	} >"$log"
	"$@" >>"$log" 2>&1
	local rc=$?
	echo "exit=$rc" >>"$log"
	printf '%s\t%s\n' "$id" "$(vieww_disk "$root")" >>"$out/logs/disk.tsv"
	if ((rc == 0)); then
		printf '%s\t%s\tPASS\tlogs/%s.txt\n' "$id" "$what" "$id" >>"$results"
	else
		printf '%s\t%s\tFAIL(exit=%s)\tlogs/%s.txt\n' "$id" "$what" "$rc" "$id" >>"$results"
		failed=1
		echo "    FAIL: $id (see $log)"
		# The reason, here, so nobody has to open the log to find out.
		grep -v '^exit=' "$log" | tail -n 4 | sed 's/^/      | /'

	fi
}
note() {
	wanted "$1" || return 0
	[[ "$3" == FAIL* ]] && failed=1
	printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "${4:-}" >>"$results"
}

# ── host record ──────────────────────────────────────────────────────────────
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
pinned="$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)"
{
	echo "os=$os"
	echo "mode=$mode"
	echo "date=$stamp"
	echo "machine=$(hostname)"
	echo "arch=$(uname -m)"
	case "$os" in
	macos)
		echo "os_version=macOS $(sw_vers -productVersion 2>/dev/null)"
		echo "cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null)"
		echo "gpu=$(system_profiler SPDisplaysDataType 2>/dev/null | sed -n 's/^ *Chipset Model: //p' | paste -sd ';' -)"
		;;
	windows)
		echo "os_version=$(cmd.exe /c ver 2>/dev/null | tr -d '\r' | sed '/^$/d')"
		echo "cpu=$(powershell.exe -NoProfile -Command '(Get-CimInstance Win32_Processor).Name' 2>/dev/null | tr -d '\r')"
		echo "gpu=$(powershell.exe -NoProfile -Command '(Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name + " (driver " + $_.DriverVersion + ")" }) -join "; "' 2>/dev/null | tr -d '\r')"
		;;
	linux)
		echo "os_version=$(. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME")"
		echo "cpu=$(sed -n 's/^model name[[:space:]]*:[[:space:]]*//p' /proc/cpuinfo | head -1)"
		echo "gpu=$(command -v lspci >/dev/null && lspci | grep -iE 'vga|3d|display' | cut -d: -f3- | paste -sd ';' -)"
		echo "display_server=${XDG_SESSION_TYPE:-}${WAYLAND_DISPLAY:+ wayland=$WAYLAND_DISPLAY}${DISPLAY:+ x11=$DISPLAY}"
		;;
	esac
	if command -v vulkaninfo >/dev/null; then
		vulkaninfo --summary 2>/dev/null | sed -nE 's/^[[:space:]]*(deviceName|driverInfo|apiVersion)[[:space:]]*=[[:space:]]*/vulkan_\1=/p' | head -3
	else
		echo "vulkan=not found (vulkaninfo missing)"
	fi
	echo "rustc=$(rustc --version 2>/dev/null | head -1)"
	echo "cargo=$(cargo --version 2>&1 | head -1)"
	echo "toolchain_pinned=$pinned"
	echo "version=$version"
	echo "git_rev=$(git rev-parse HEAD 2>/dev/null || echo 'none (not a git checkout)')"
} >"$out/host.txt"

# Start from an empty build cache. Trees from earlier runs or from normal
# development were built with different flags (debuginfo, incremental), so the
# lean build cannot reuse them — they would only sit beside it, and they are
# usually most of target/. --keep-builds leaves them alone.
echo "gate: disk before: $(vieww_disk "$root")"
if [[ -z "$keep_builds" && "$VIEWW_LEAN" == 1 ]]; then
	vieww_prune "$root" debug release static-std doc msrv package tmp desktop-suite
	echo "gate: cleared old build trees: $(vieww_disk "$root")"
fi

# ── preflight: disk ──────────────────────────────────────────────────────────
# A full gate builds debug and release trees, a static-std tree and every
# example: roughly 40 GB of target/. When the disk fills mid-link, lld dies
# with SIGBUS ("ld terminated with signal 7 [Bus error]") rather than saying
# "no space left", which reads as a compiler bug. Say it up front instead.
free_kb=$(df -Pk "$root" 2>/dev/null | awk 'NR==2 {print $4}')
if [[ -n "$free_kb" ]]; then
	free_gb=$((free_kb / 1024 / 1024))
	echo "disk_free_gb=$free_gb" >>"$out/host.txt"
	need_gb=30
	[[ "$mode" == gpu ]] && need_gb=10
	# A partial rerun builds a fraction of the tree (the export route alone is
	# a few GB); the full-gate figure would fail small hosted runners for nothing.
	[[ -n "$only" ]] && ((need_gb > 15)) && need_gb=15
	[[ "$VIEWW_LEAN" == 1 ]] || need_gb=100
	[[ -n "$keep_builds" ]] && need_gb=$((need_gb + 20))
	if ((free_gb < need_gb)); then
		note PRE.1 "at least $need_gb GB free for target/" "FAIL(${free_gb} GB free; lld crashes with SIGBUS when the disk fills)" host.txt
		echo "gate: only ${free_gb} GB free under $root (want $need_gb) — expect link failures (SIGBUS). Free space, run 'cargo clean', or set CARGO_TARGET_DIR to a bigger disk." >&2
	else
		note PRE.1 "at least $need_gb GB free for target/" "PASS(${free_gb} GB free)" host.txt
	fi
fi

# ── Gate 0: identity and repository integrity ────────────────────────────────
# `--mode gpu` is the narrow pass for a machine that already ran `--mode cpu`
# (or whose CPU results come from CI): only what a GPU can change. See the
# "GPU pass" note in ci/README.md.
if [[ "$mode" != gpu ]]; then
note G0.1 "workspace version" "RECORDED($version) — confirm it is the beta version" "host.txt"
row G0.2 "Cargo.lock is current (--locked)" cargo metadata --locked --format-version 1 --no-deps
fi
if rustc --version 2>/dev/null | grep -q " $pinned "; then
	note G0.3 "rustc matches rust-toolchain.toml ($pinned)" PASS host.txt
else
	note G0.3 "rustc matches rust-toolchain.toml ($pinned)" "FAIL($(rustc --version 2>/dev/null | head -1))" host.txt
fi
if [[ "$mode" != gpu ]]; then
row G0.4 "release-clean (no patch leftovers, stale evidence, local paths)" bash ci/check/release-clean-check.sh "$root"
if git rev-parse --git-dir >/dev/null 2>&1; then
	# target/ is excluded: this gate is writing its own run folder there. Whether
	# target/ is *ignored* (a committed .gitignore) is checked by G0.4.
	row G0.5 "git working tree clean" bash -c 'st=$(git status --porcelain -- . ":(exclude)target"); [[ -z "$st" ]] || { echo "changed or untracked files:"; echo "$st" | head -40; exit 1; }'
else
	note G0.5 "git working tree clean" "FAIL(not a git checkout — certify from the tagged commit)" ""
fi
row G0.6 "LICENSE present" test -s LICENSE
row G0.7 "no private keys or credentials in the tree" bash -c '
	! grep -rIlE --exclude-dir=target --exclude-dir=.git \
		-e "-----BEGIN [A-Z ]*PRIVATE KEY-----" \
		-e "AKIA[0-9A-Z]{16}" -e "gh[pousr]_[A-Za-z0-9]{36}" -e "xox[baprs]-[A-Za-z0-9-]{10,}" .'
if command -v cargo-deny >/dev/null; then
	row G0.8 "cargo deny check all" cargo deny check --hide-inclusion-graph all
else
	note G0.8 "cargo deny check all" "FAIL(cargo-deny not installed: cargo install cargo-deny --locked)" ""
fi

# ── Gate 1: framework ────────────────────────────────────────────────────────
row G1.1 "ci/vieww checks (fmt, clippy, check, docs, MSRV, tests, fixtures, packaging)" bash ci/check/checks.sh
row G1.2 "ci/vieww platform $os" bash ci/check/platform-check.sh "$os"
# Cross-target check trees (target/<triple>/), if this OS's check made any.
if [[ -z "$keep_builds" ]]; then
	for triple_dir in "$root"/target/*-*-*/; do
		[[ -d "$triple_dir" ]] && vieww_prune "$root" "$(basename "$triple_dir")"
	done
fi

# checks.sh passed fmt, clippy, check and the workspace tests: certification
# records those as COVERED instead of running them a second time.
if grep -q $'^G1.1\t.*\tPASS\t' "$results"; then
	export VIEWW_CERT_BUILD_COVERED=1
fi
fi # mode != gpu (Gate 0 rest, G1.1, G1.2)

cert_env=()
case "$mode" in
cpu) cert_env=(VIEWW_CERT_NO_GPU=1) ;;
gpu) cert_env=(VIEWW_CERT_REQUIRE_REAL_GPU=1 VIEWW_CERT_ONLY=gpu) ;;
esac
if [[ "$os" == windows ]]; then
	ps=$(command -v pwsh || command -v powershell.exe)
	row G1.3 "certification ($mode)" env ${cert_env[@]+"${cert_env[@]}"} "$ps" -NoProfile -ExecutionPolicy Bypass \
		-File ci/certify/certify-windows.ps1 -OutDir "$(cygpath -w "$out/cert" 2>/dev/null || echo "$out/cert")"
else
	row G1.3 "certification ($mode)" env ${cert_env[@]+"${cert_env[@]}"} bash ci/certify/certify.sh "$out/cert"
fi
unset VIEWW_CERT_BUILD_COVERED

# Every debug-profile stage is done (clippy, check, docs, tests, platform
# checks, GPU suite). What follows builds in release only.
[[ -n "$keep_builds" ]] || vieww_prune "$root" debug static-std doc msrv tmp
# Individual certification stages the checklist names, lifted out of the bundle.
stage() { sed -n "s#^$1[[:space:]]\\{1,\\}##p" "$out/cert/quality/stages.txt" 2>/dev/null | head -1; }
for pair in \
	"G1.4|GPU device class|gpu/device-class" \
	"G1.5|Vulkan compositor pixel parity (17 tests)|gpu/vulkan-suite" \
	"G1.6|GPU census: every fixture plans complete|gate/census-all-complete" \
	"G1.7|CPU renderer: premium UI, fixtures|visual/premium" \
	"G1.8|animation stress p95 within budget|suites/animation-stress" \
	"G1.9|Vieww Standard (12 clauses, measured)|standard/vieww-standard" \
	"G1.10|desktop suite (real windows)|desktop/suite"; do
	IFS='|' read -r id what key <<<"$pair"
	# The CPU-renderer rows are not the GPU pass's to answer.
	[[ "$mode" == gpu && "$id" =~ ^G1\.(7|8|9)$ ]] && continue
	[[ "$os" == windows ]] && key="${key//\//\\\\}"
	v="$(stage "$key")"
	# A bare FAIL(exit=1) sends a person digging through logs; lift the one
	# number or line that decided it into the verdict.
	if [[ "$v" == FAIL* ]]; then
		why=""
		case "$id" in
		G1.8) why="p95 $(sed -n 's/^render_p95_ms=//p' "$out/cert/suites/animation-stress/metrics.txt" 2>/dev/null) ms, $(sed -n 's/^over_budget=//p' "$out/cert/suites/animation-stress/metrics.txt" 2>/dev/null)/$(sed -n 's/^frames=//p' "$out/cert/suites/animation-stress/metrics.txt" 2>/dev/null) frames over budget" ;;
		G1.9) why="$(sed -n 's/^ *- //p' "$out/cert/standard/vieww-standard.txt" 2>/dev/null | paste -sd ';' -)" ;;
		G1.10) why="$(grep -m1 -oE 'Segmentation fault|died on signal [0-9]+[^.]*|no report after [0-9]+s|[0-9]+ failed' "$out/cert/desktop/suite.txt" 2>/dev/null)" ;;
		esac
		[[ -n "${why// /}" ]] && v="FAIL($why)"
	fi
	note "$id" "$what" "${v:-NOT RUN}" "cert/quality/stages.txt"
done
note G1.11 "D3D12 backend" "N/A(BUILD-ONLY: no ScenePlan executor; not claimed)" ""
note G1.12 "Metal backend" "N/A(BUILD-ONLY: no ScenePlan executor; not claimed)" ""
if [[ "$mode" != gpu && -n "$shots" ]]; then
	if [[ -n "$against" ]]; then
		row G1.13 "feature screenshots byte-identical to $against" bash ci/certify/shot-suite.sh --out "$out/shots" --against "$against"
	else
		row G1.13x "feature screenshots captured" bash ci/certify/shot-suite.sh --out "$out/shots"
	fi
fi

# ── Gate 2: Studio ───────────────────────────────────────────────────────────
[[ "$mode" == gpu ]] || row G2.1 "Studio release_check + tour + walkthrough (full, not --quick)" bash ci/certify/release-check.sh "$out/studio"
[[ -n "$hidpi" ]] && row G2.3 "desktop suite on a HiDPI display" bash ci/certify/desktop-suite.sh --expect-hidpi
[[ -n "$multi" ]] && row G2.4 "desktop suite across two displays" bash ci/certify/desktop-suite.sh --expect-multi-monitor

# ── Studio's export route: scaffold → compile every target → real exports ──
# One run of ci/certify/export-suite.sh answers G2.7-G2.11. Its projects live
# outside the repository (see that script), under a scratch directory removed
# afterwards; only results.txt and the logs are kept in the run folder.
if [[ "$mode" != gpu ]] && { wanted G2.7 || wanted G2.8 || wanted G2.9 || wanted G2.10 || wanted G2.11; }; then
	export_out="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/vieww-export-check-$stamp"
	export_args=("$export_out")
	[[ -n "${VIEWW_EXPORT_INSTALL_TARGETS:-}" ]] && export_args+=(--install-targets)
	echo "==> G2.7-G2.11 Studio export route (scaffold, compile, export)"
	bash ci/certify/export-suite.sh "${export_args[@]}" >"$out/logs/export-suite.txt" 2>&1
	mkdir -p "$out/export"
	cp "$export_out/results.txt" "$out/export/" 2>/dev/null
	cp -R "$export_out/logs" "$out/export/" 2>/dev/null
	[[ "$VIEWW_LEAN" == 1 && -z "$keep_builds" ]] && rm -rf "$export_out"
	results_file="$out/export/results.txt"

	# export_verdict ROW WHAT PATTERN REQUIRED — fold export-check lines into a row.
	# REQUIRED=1: a SKIPPED/REFUSED line is not a pass (the host could run it).
	export_verdict() {
		local id="$1" what="$2" pattern="$3" required="$4" lines fails skips passes
		if [[ ! -f "$results_file" ]]; then
			note "$id" "$what" "FAIL(export suite produced no results; see logs/export-suite.txt)" logs/export-suite.txt
			return
		fi
		lines=$(grep -E "^export-check: ($pattern) " "$results_file")
		fails=$(echo "$lines" | grep -c ' FAIL' || true)
		skips=$(echo "$lines" | grep -E ' (SKIPPED|REFUSED) ' | sed -E 's/^export-check: ([^ ]+) (SKIPPED|REFUSED) (.*)$/\1: \3/' | paste -sd ';' -)
		passes=$(echo "$lines" | grep -c ' PASS' || true)
		if [[ -z "$lines" ]]; then
			note "$id" "$what" "NOT RUN" export/results.txt
		elif ((fails > 0)); then
			note "$id" "$what" "FAIL($(echo "$lines" | grep ' FAIL' | head -2 | cut -c15-220 | paste -sd ';' -))" export/results.txt
		elif [[ -n "$skips" && "$required" == 1 ]]; then
			note "$id" "$what" "SKIPPED(${skips:0:300})" export/results.txt
		elif ((passes > 0)); then
			note "$id" "$what" "PASS" export/results.txt
		else
			note "$id" "$what" "SKIPPED(${skips:0:300})" export/results.txt
		fi
	}
	# Which cross targets each host is expected to be able to compile.
	case "$os" in
	macos) compile_pattern='compile/(desktop|android|windows|ios|ios-sim)-[a-z]+' ;;
	*) compile_pattern='compile/(desktop|android|windows)-[a-z]+' ;;
	esac
	export_verdict G2.7 "scaffolded Rust + Say projects compile for every target" "$compile_pattern" 1
	export_verdict G2.8 "Studio export: desktop binary" 'export/desktop' 1
	export_verdict G2.9 "Studio export: Windows .exe" 'export/windows' 1
	export_verdict G2.10 "Studio export: Android .apk (cargo-ndk + Gradle)" 'export/android' 1
	if [[ "$os" == macos ]]; then
		export_verdict G2.11 "Studio export: iOS simulator .app" 'export/ios-sim' 1
	else
		note G2.11 "Studio export: iOS simulator .app" "N/A(macOS only — Apple's toolchain)" ""
	fi
fi

# ── Gate 3 / 8: artifacts and integrity ──────────────────────────────────────
if [[ "$mode" == gpu ]]; then
	: # installers are CPU work; the CPU pass or CI builds them
elif [[ -n "$package" ]] && wanted G3.1; then
	row G3.1 "installers build (package.sh --installers) + bundle compiles a guest" bash ci/release/artifacts.sh
	mkdir -p "$out/package"
	cp target/package/SHA256SUMS target/package/MANIFEST.txt "$out/package/" 2>/dev/null
	# Keep what gets published; the unpacked staging trees (the .app, the
	# AppDir, the .deb root, the portable tree with its bundled toolchain) are
	# each a full copy of what is already inside the installers.
	if [[ -z "$keep_builds" && "$VIEWW_LEAN" == 1 && -d target/package ]]; then
		find target/package -mindepth 1 -maxdepth 1 \
			! -name 'viewwstudio-*.deb' ! -name 'viewwstudio-*.AppImage' ! -name 'viewwstudio-*.tar.gz' \
			! -name 'viewwstudio-*.zip' ! -name 'viewwstudio-*.msi' ! -name 'viewwstudio-*.dmg' \
			! -name SHA256SUMS ! -name MANIFEST.txt ! -name index.html ! -name build.json \
			-exec rm -rf {} +
	fi
	if [[ -s "$out/package/SHA256SUMS" ]]; then
		note G8.1 "SHA256SUMS generated for every artifact" PASS package/SHA256SUMS
		note G8.2 "toolchain + source recorded with the artifacts" PASS package/MANIFEST.txt
	else
		note G8.1 "SHA256SUMS generated for every artifact" "FAIL(no artifacts)" logs/G3.1.txt
	fi
else
	note G3.1 "installers build" "SKIPPED(--skip-package)" ""
fi

# ── cleanup ──────────────────────────────────────────────────────────────────
# The last release-profile stage is done. The evidence is in $out; the
# binaries are reproducible from the same commit.
[[ -n "$keep_builds" ]] || vieww_prune "$root" release

# Stages that passed keep their numbers and logs, not their pictures: a PASS
# row's evidence is the metrics file, and a folder of PNGs per suite adds up
# across runs and machines. Failed stages keep everything. shots/ (needed for
# --against on the next OS) and studio/ (the manual G2.2 review) are kept.
if [[ -z "$keep_evidence" && -f "$out/cert/quality/stages.txt" ]]; then
	while read -r stage verdict; do
		[[ "$verdict" == PASS* ]] || continue
		dir="$out/cert/${stage//\\//}"
		[[ -d "$dir" ]] || continue
		find "$dir" -type f ! -name '*.txt' ! -name '*.json' ! -name '*.md' ! -name '*.csv' -delete
		find "$dir" -type d -empty -delete
	done <"$out/cert/quality/stages.txt"
	# The census diffs are the only way to see a failed census gate; keep them then.
	if grep -qE '^gpu[/\\]census[[:space:]]+PASS' "$out/cert/quality/stages.txt" &&
		! grep -qiE 'census.*FAIL' "$out/cert/quality/stages.txt"; then
		rm -rf "$out/cert/gpu/census-out"
	fi
fi
echo "gate: disk after: $(vieww_disk "$root"), evidence $(du -sh "$out" 2>/dev/null | cut -f1)"

# ── the checked checklist ────────────────────────────────────────────────────
py=$(command -v python3 || command -v python || true)
if [[ -n "$py" ]]; then
	"$py" ci/release/fill-checklist.py "$out" -o "$out/CHECKLIST.md" || true
else
	echo "gate: no python on PATH, so CHECKLIST.md was not written (rows.tsv has every verdict)" >&2
fi

echo
cat "$results" | column -t -s $'\t' 2>/dev/null || cat "$results"
echo
if ((failed)); then echo "AUTOMATED VERDICT: FAIL"; else echo "AUTOMATED VERDICT: PASS (manual rows still open)"; fi
echo "checklist: $out/CHECKLIST.md"
echo "evidence:  $out"
exit "$failed"
