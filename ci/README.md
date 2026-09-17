# ci/

Every check, certification and release script, grouped by what it proves.
One entry point runs them all:

```bash
ci/vieww            # list commands
ci/vieww gate       # the release script: runs everything, writes a checked CHECKLIST.md
```

The release process itself is in
[`docs/release/BETA-RELEASE-CHECKLIST.md`](../docs/release/BETA-RELEASE-CHECKLIST.md).

| Folder | Kind | Needs |
|---|---|---|
| `check/` | Fast and deterministic. Same answer on any machine. | Rust toolchain |
| `certify/` | Evidence from a real machine: GPU, display, timing | The OS under test, optionally a GPU and display |
| `release/` | Turns a certified tree into publishable files | Packaging tools per OS |
| `mobile/` | Android and iOS builds, device runs, hot reload | NDK / Xcode |
| `tools/` | One-off maintenance | See each script |

## Commands

| `ci/vieww …` | Script | What it proves |
|---|---|---|
| `gate [--mode cpu\|gpu] [--skip-package] [--hidpi] [--multi-monitor] [--against SHOTS]` | `release/gate.sh` | **The single release script.** Runs every script below for this OS and writes `target/release-gate/<os>-<mode>-<stamp>/CHECKLIST.md`, the checklist with this run's cells checked. |
| `checklist RUN… [-o FILE]` | `release/fill-checklist.py` | Combines the gate runs from every machine into one checked checklist. Also checks that every run used the same source and commit. |
| `artifacts` | `release/artifacts.sh` | Runs `packaging/package.sh --installers`, then writes `SHA256SUMS` and `MANIFEST.txt`. |
| `sums DIR…` | `release/artifacts.sh --merge` | Merges each OS's `SHA256SUMS` into the one the download page links. |
| `package-source` | `release/package-source.sh` | Clean source archive. Refuses a tree that fails `clean`. |
| `checks [--quick]` | `check/checks.sh` | fmt, clippy, check and docs with the real features, plus MSRV, cargo-deny, tests, fixtures, hot reload and packaging. Runs every stage and lists all failures at the end (`VIEWW_CHECKS_FAIL_FAST=1` to stop at the first). |
| `platform [os]` | `check/platform-check.sh` | Compiles for linux, macos, windows, ios, android or web on this host. |
| `clean` | `check/release-clean-check.sh` | Finds patch leftovers, nested archives, stale evidence, generated output and local paths. |
| `wasm` / `coverage` / `size` | `check/…` | Web build, line coverage, binary size budget. |
| `certify [OUT]` | `certify/certify.sh` (Linux, macOS), `certify/certify-windows.ps1` | Every suite with one verdict. Skipped stages are never passes. |
| `studio [--quick]` | `certify/release-check.sh` | Runs Studio's `release_check`, `tour` and `walkthrough` headless and builds a screenshot gallery. |
| `export [OUT] [--compile-only] [--install-targets]` | `certify/export-suite.sh` | Studio's export route. Scaffolds Rust and Say projects outside the repo, compiles them for desktop, Android, Windows and iOS, then runs the Export sheet's own plans: desktop binary, `.exe`, `.apk` (cargo-ndk + Gradle), iOS simulator `.app`. A missing toolchain is SKIPPED with its install command. Gate rows G2.7–G2.11. |
| `desktop [--expect-hidpi] [--expect-multi-monitor]` | `certify/desktop-suite.sh` | Real windows, clipboard, dialogs and DPI. |
| `shots [--against DIR]` | `certify/shot-suite.sh` | Every feature example, byte-compared across machines. |
| `mobile <script>` | `mobile/*.sh` | For example `apk`, `device-suite`, `a11y-android`, `ios-app`, `reload-android`. |
| `tool <script>` | `tools/*` | `patch-winit.sh`, `subset-fonts.py`. |

`.cargo/config.toml` points at `mobile/ndk-clang-*.sh`, `mobile/adb-runner.sh` and `mobile/simctl-runner.sh` as linkers and runners. Keep those paths in sync if you move them.

## Disk space

Every `ci/vieww` command builds **lean**, using `ci/lib/lean.sh`: no debuginfo and no incremental caches. Checks don't need either, and together they made up most of the ~100 GB a full gate used to need.

`gate` also deletes each build tree once the last stage that needs it has finished:

| When | Deleted |
|---|---|
| Start | Old `target/debug`, `release` and `static-std` trees. Built with other flags, so the lean build can't reuse them. |
| After docs / MSRV / packaging smoke (inside `checks`) | `target/doc`, `target/msrv`, `target/package` |
| After certification | `target/debug`, `target/static-std`, cross-target trees |
| After installers | Package staging trees. The installers, `SHA256SUMS` and `MANIFEST.txt` stay. |
| End | `target/release`. Image outputs of stages that **passed** are removed too; their metrics and logs stay. Failed stages keep everything. |

Certification doesn't repeat fmt, clippy, check or the workspace tests when `checks` already passed them in the same run; they show as `COVERED`.

`logs/disk.tsv` in each run records the size of `target/` and the free space after every row.

Options:

- `--keep-builds`: keep the build trees.
- `--keep-evidence`: keep every image.
- `VIEWW_FULL_DEBUG=1`: debuginfo and incremental builds. Needs far more disk.

## Where each pass runs

| Pass | Where | How |
|---|---|---|
| Linux CPU | Local machine | `ci/vieww gate --mode cpu` |
| Linux GPU | Same machine, afterwards | `ci/vieww gate --mode gpu`. Only the GPU-dependent rows: device class, Vulkan parity suite, GPU workload, census, desktop suite. It builds a small slice of the workspace. |
| Windows / macOS CPU, macOS GPU (paravirtual, via MoltenVK), Linux export route (Android SDK/NDK preinstalled) | GitHub-hosted runners | `.github/workflows/release-gate.yml`: run it manually, or push a `v*` tag. It never runs on pull requests. |

Hosted runners have much less free disk than a gate needs, so the workflow first runs `tools/ci-runner-prep.sh`. That script deletes unused SDKs and, on Windows, moves `target/` to the drive with the most room. It refuses to run outside GitHub Actions.

To choose which GPU the Vulkan code uses, set `VIEWW_VK_DEVICE=<part of the device name>`.

## CPU and GPU runs

The certification scripts read three environment variables. `gate --mode` sets them for you.

| Variable | Effect |
|---|---|
| `VIEWW_CERT_NO_GPU=1` | CPU-only run. GPU execution stages are `SKIPPED`; builds still run. |
| `VIEWW_CERT_REQUIRE_REAL_GPU=1` | `gpu/device-class` fails on llvmpipe, lavapipe or SwiftShader. |
| `VIEWW_CERT_NO_DESKTOP=1` | Skips the real-window suite on a headless box. |
| `VIEWW_CERT_STRICT=1` | Any skipped stage fails the run. |

## Known gaps in this folder

- `tools/patch-winit.sh` expects `ci/tools/winit-hover.patch`. That file is not in the tree, so the Android hover patch cannot be applied. This does not affect desktop.
- Hosted CI (`.github/workflows/release-gate.yml`) covers the Windows and macOS CPU passes only. Its runners have no GPU and no display, so GPU and real-window rows need real machines.
