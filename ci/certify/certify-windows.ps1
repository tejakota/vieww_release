# Vieww Windows certification — every suite in the tree, one command, one verdict.
#
#   pwsh ci/certify/certify-windows.ps1 [-OutDir target\cert-windows] [-Strict]
#
# Same GPU/CPU switches as the Unix script (environment variables):
#   VIEWW_CERT_NO_GPU=1            CPU-only run; GPU execution stages SKIPPED
#   VIEWW_CERT_REQUIRE_REAL_GPU=1  fail gpu\device-class on a software rasterizer
#   VIEWW_CERT_NO_DESKTOP=1        skip the interactive desktop suite
#
# The Windows twin of `ci/certify/certify.sh`; see that script's header for
# the rules both follow:
#
#   * every suite under examples/ is a stage (the previous version ran six);
#   * OutDir is emptied first and meta\source.txt records a source digest, so
#     evidence cannot be mistaken for a certification of a different tree;
#   * a stage this machine cannot run is SKIPPED(reason), never PASS, and
#     -Strict turns any skip into a failure.
#
# Two fixes to the previous version worth knowing about: its capture helper
# declared a parameter named `$Args`, which is PowerShell's automatic variable
# and made every stage's argument list unreliable; and it treated a D3D12
# *build* as a certification of the D3D12 *backend*. `vieww-hal`'s `d3d12`
# module does not execute a `ScenePlan` yet, so the D3D12 line below is
# reported as what it is: BUILD-ONLY.

param(
  [string]$OutDir = "target\cert-windows",
  [switch]$Strict
)
$ErrorActionPreference = 'Continue'
$Root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $Root

if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
foreach ($d in 'meta','build','tests','gpu','visual','suites','standard','web','desktop','quality') {
  New-Item -ItemType Directory -Force -Path (Join-Path $OutDir $d) | Out-Null
}
$script:Failed = $false
$script:Skipped = 0
$stages = Join-Path $OutDir 'quality\stages.txt'
Set-Content $stages ''

function Record([string]$Name, [string]$Result) {
  Add-Content $stages ('{0,-28} {1}' -f $Name, $Result)
}

function Run-Stage([string]$Name, [string]$Exe, [string[]]$ArgList) {
  $file = Join-Path $OutDir ($Name + '.txt')
  New-Item -ItemType Directory -Force -Path (Split-Path $file) | Out-Null
  Set-Content $file (">>> {0} {1}" -f $Exe, ($ArgList -join ' '))
  $sw = [Diagnostics.Stopwatch]::StartNew()
  & $Exe @ArgList *>> $file
  $rc = $LASTEXITCODE
  Add-Content $file "exit=$rc"
  Add-Content $file ("seconds={0}" -f [int]$sw.Elapsed.TotalSeconds)
  if ($rc -eq 0) { Record $Name 'PASS' } else { Record $Name "FAIL(exit=$rc)"; $script:Failed = $true; Write-Host "    FAILED: $Name" }
  return $rc
}

function Skip-Stage([string]$Name, [string]$Reason) {
  $file = Join-Path $OutDir ($Name + '.txt')
  New-Item -ItemType Directory -Force -Path (Split-Path $file) | Out-Null
  Set-Content $file "SKIPPED($Reason)"
  Record $Name "SKIPPED($Reason)"
  $script:Skipped++
}

# ── meta ─────────────────────────────────────────────────────────────────────
@(
  "date=$(Get-Date -Format o)",
  "hostname=$env:COMPUTERNAME",
  "os=$([Environment]::OSVersion.VersionString)",
  "arch=$env:PROCESSOR_ARCHITECTURE",
  "rust=$(rustc --version 2>&1)",
  "cargo=$(cargo --version 2>&1)",
  "gpu=$((Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name) -join '; ')",
  "strict=$Strict"
) | Set-Content (Join-Path $OutDir 'meta\host.txt')

$sourceFiles = Get-ChildItem -Recurse -File -Path Cargo.toml, Cargo.lock, rust-toolchain.toml, crates, examples, apps, ci, scripts `
  -Include *.rs, *.toml, *.wgsl, *.sh, *.ps1, *.py, *.lock, *.ttf, *.otf, *.html -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -notmatch '[\\/](target|dist)[\\/]' } |
  Sort-Object { $_.FullName.Substring($Root.Length).Replace('\', '/') }
$hashes = ($sourceFiles | ForEach-Object { (Get-FileHash -Algorithm SHA256 $_.FullName).Hash }) -join "`n"
$digest = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($hashes))).Replace('-', '').ToLower()
# Same rule as certify.sh: in a git checkout, git's tree id (identical on every
# OS, unlike a hash of CRLF-checked-out files), marked if tracked files changed.
$tree = git rev-parse 'HEAD^{tree}' 2>$null
if ($LASTEXITCODE -eq 0 -and $tree) {
  $modified = git status --porcelain --untracked-files=no 2>$null
  $digest = if ($modified) { "git-tree:$tree+modified" } else { "git-tree:$tree" }
}
@("git_rev=$(git rev-parse HEAD 2>$null)", "source_digest=$digest") | Set-Content (Join-Path $OutDir 'meta\source.txt')

$hasVulkan = $false
$vulkanDevice = ''
if (Get-Command vulkaninfo -ErrorAction SilentlyContinue) {
  $summary = vulkaninfo --summary 2>$null
  $hasVulkan = ($LASTEXITCODE -eq 0)
  $vulkanDevice = (($summary | Select-String -Pattern '^\s*deviceName\s*=\s*(.*)$' | Select-Object -First 1).Matches.Groups[1].Value)
}

# ── build ────────────────────────────────────────────────────────────────────
if (Get-Command bash -ErrorAction SilentlyContinue) {
  Run-Stage 'build\release-clean' 'bash' @('ci/check/release-clean-check.sh') | Out-Null
} else { Skip-Stage 'build\release-clean' 'bash unavailable' }
# VIEWW_CERT_BUILD_COVERED=1: set by `ci/vieww gate` when ci/check/checks.sh already
# passed these on this tree in the same run (see certify.sh). COVERED is a pass.
if ($env:VIEWW_CERT_BUILD_COVERED -eq '1') {
  foreach ($s in 'build\fmt','build\check','build\clippy-core','tests\workspace') { Record $s 'COVERED(ci/check/checks.sh passed it in this gate run)' }
} else {
  Run-Stage 'build\fmt' 'cargo' @('fmt','--all','--','--check') | Out-Null
  Run-Stage 'build\check' 'cargo' @('check','--workspace','--all-targets') | Out-Null
  Run-Stage 'build\clippy-core' 'cargo' @('clippy','--keep-going','-p','vieww-gpu','-p','vieww-hal','-p','vieww-render-planner','-p','vieww-shaders','-p','vieww-paint','-p','vieww-render','-p','vieww-element','--all-targets','--features','vieww-hal/vulkan','--','-D','warnings') | Out-Null
  # Same features as checks.sh, so both share one debug build.
  Run-Stage 'tests\workspace' 'cargo' @('test','--workspace','--no-fail-fast','--features','vieww-paint/native') | Out-Null
}

# ── tests ────────────────────────────────────────────────────────────────────
Run-Stage 'tests\gpu-planner' 'cargo' @('test','-p','vieww-gpu') | Out-Null
Run-Stage 'tests\shaders' 'cargo' @('test','-p','vieww-shaders') | Out-Null
Run-Stage 'tests\quality' 'cargo' @('test','-p','vieww-render-planner','quality') | Out-Null
Run-Stage 'tests\paint-native' 'cargo' @('test','-p','vieww-paint','--features','native') | Out-Null

# ── GPU ──────────────────────────────────────────────────────────────────────
Run-Stage 'gpu\hal-build-vulkan' 'cargo' @('check','-p','vieww-hal','--features','vulkan') | Out-Null
if ($env:VIEWW_CERT_NO_GPU -eq '1') {
  foreach ($s in 'gpu\device-class','gpu\adapter','gpu\vulkan-suite','gpu\workload','gpu\census') { Skip-Stage $s 'CPU-only run requested' }
} elseif ($hasVulkan) {
  Set-Content (Join-Path $OutDir 'gpu\device.txt') "vulkan_device=$vulkanDevice"
  if ($vulkanDevice -match 'llvmpipe|lavapipe|swiftshader|software|Basic Render') {
    if ($env:VIEWW_CERT_REQUIRE_REAL_GPU -eq '1') { Record 'gpu\device-class' "FAIL(software rasterizer: $vulkanDevice)"; $script:Failed = $true }
    else { Record 'gpu\device-class' "SOFTWARE($vulkanDevice; correctness only, not a GPU certification)" }
  } else { Record 'gpu\device-class' "PASS($vulkanDevice)" }
  Run-Stage 'gpu\adapter' 'cargo' @('run','--release','-p','vieww-hal','--example','adapter','--features','vulkan') | Out-Null
  Run-Stage 'gpu\vulkan-suite' 'cargo' @('test','-p','vieww-hal','--features','vulkan','--','--ignored','--test-threads=1') | Out-Null
  Run-Stage 'gpu\workload' 'cargo' @('run','--release','-p','test-gpu-work','--',(Join-Path $OutDir 'gpu\workload')) | Out-Null
  Run-Stage 'gpu\census' 'cargo' @('run','--release','-p','fixtures','--',(Join-Path $OutDir 'gpu\census-out'),'--census') | Out-Null
} else {
  foreach ($s in 'gpu\device-class','gpu\adapter','gpu\vulkan-suite','gpu\workload','gpu\census') { Skip-Stage $s 'no Vulkan loader/ICD' }
}
Run-Stage 'gpu\d3d12-build' 'cargo' @('check','-p','vieww-hal','--features','d3d12') | Out-Null
Record 'gpu\d3d12-backend' 'BUILD-ONLY(d3d12 does not execute a ScenePlan yet; not certified)'

# ── visual, suites, standard ─────────────────────────────────────────────────
Run-Stage 'visual\premium' 'cargo' @('run','--release','-p','test-premium-ui','--',(Join-Path $OutDir 'visual\premium')) | Out-Null
Run-Stage 'visual\fixtures' 'cargo' @('run','--release','-p','fixtures','--',(Join-Path $OutDir 'visual\fixtures')) | Out-Null
foreach ($suite in 'test-layout-stress','test-animation-stress','test-text-fidelity','test-image-effects','test-scroll-stress','test-native-surface') {
  $short = $suite.Substring(5)
  Run-Stage "suites\$short" 'cargo' @('run','--release','-p',$suite,'--',(Join-Path $OutDir "suites\$short")) | Out-Null
}
Run-Stage 'standard\vieww-standard' 'cargo' @('run','--release','-p','vieww-standard','--',(Join-Path $OutDir 'standard\vieww-standard')) | Out-Null

# ── web ──────────────────────────────────────────────────────────────────────
Run-Stage 'web\baseline' 'cargo' @('run','--release','-p','test-web','--example','baseline','--',(Join-Path $OutDir 'web\baseline')) | Out-Null
$wasm = (rustup target list --installed 2>$null) -contains 'wasm32-unknown-unknown'
if ($wasm -and (Get-Command wasm-bindgen -ErrorAction SilentlyContinue) -and (Get-Command bash -ErrorAction SilentlyContinue)) {
  Run-Stage 'web\build' 'bash' @('examples/test-web/build-web.sh',(Join-Path $OutDir 'web\dist')) | Out-Null
  Run-Stage 'web\verify' 'python' @('examples/test-web/verify_web.py',(Join-Path $OutDir 'web\dist'),(Join-Path $OutDir 'web\baseline')) | Out-Null
} else {
  Skip-Stage 'web\build' 'wasm32 target, wasm-bindgen or bash unavailable'
  Skip-Stage 'web\verify' 'no freshly built wasm to verify'
}

# ── desktop ──────────────────────────────────────────────────────────────────
if ($env:VIEWW_CERT_NO_DESKTOP -eq '1') { Skip-Stage 'desktop\suite' 'VIEWW_CERT_NO_DESKTOP=1' }
elseif (Get-Command bash -ErrorAction SilentlyContinue) { Run-Stage 'desktop\suite' 'bash' @('ci/certify/desktop-suite.sh','--log',(Join-Path $OutDir 'desktop\app.log')) | Out-Null }
else { Skip-Stage 'desktop\suite' 'bash (Git Bash) unavailable' }

# ── gates on measured values ─────────────────────────────────────────────────
function Gate([string]$Name, [bool]$Ok) { if ($Ok) { Record "gate\$Name" 'PASS' } else { Record "gate\$Name" 'FAIL'; $script:Failed = $true } }
$gpuMetrics = Join-Path $OutDir 'gpu\workload\metrics.txt'
if (Test-Path $gpuMetrics) { Gate 'gpu-unsupported-zero' ((Get-Content $gpuMetrics) -contains 'unsupported_gpu_commands=0') }
$premium = Join-Path $OutDir 'visual\premium\metrics.txt'
if (Test-Path $premium) { Gate 'premium-overflows-zero' ((Get-Content $premium) -contains 'overflows=0') }
$std = Join-Path $OutDir 'standard\vieww-standard\vieww-standard.json'
if (Test-Path $std) {
  $json = Get-Content $std -Raw | ConvertFrom-Json
  Gate 'standard-passed' ($json.passed -eq $true)
  Gate 'standard-allocations-measured' ($json.allocation_counter_installed -eq $true)
} else { Gate 'standard-report-present' $false }
$census = Join-Path $OutDir 'gpu\census.txt'
if ((Test-Path $census) -and -not ((Get-Content $census -Raw) -match '^SKIPPED\(')) {
  $text = Get-Content $census -Raw
  Gate 'census-all-complete' ($text -match '(?m)^(\d+) of \1 fixtures plan complete')
  Gate 'census-no-flat-region-mismatch' ($text -match 'beyond the geometry-edge band: 0 of')
}

$complete = ($script:Skipped -eq 0)
if ($Strict -and -not $complete) { $script:Failed = $true }
@(
  'Vieww Windows Certification',
  '===========================',
  "COMPLETE=$complete (skipped stages: $($script:Skipped))",
  "FAILED=$($script:Failed)",
  (Get-Content (Join-Path $OutDir 'meta\source.txt')),
  '',
  (Get-Content $stages)
) | Set-Content (Join-Path $OutDir 'summary.txt')
Get-Content (Join-Path $OutDir 'summary.txt')
if ($script:Failed) { exit 1 } else { exit 0 }
