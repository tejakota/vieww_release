# Vieww Beta Release Checklist

| | |
|---|---|
| **Release** | |
| **Version** | (currently `0.1.0` in `Cargo.toml`; decide the beta version, see G0.1) |
| **Certified commit / tag** | |
| **Release date** | |
| **Release owner** | |

## How to use this

This beta is certified with the machines at hand:

| Leg | Where | Command |
|---|---|---|
| Linux, CPU pass | Your laptop | `ci/vieww gate --mode cpu` |
| Linux, GPU pass | The same laptop, afterwards | `ci/vieww gate --mode gpu`. This is the narrow GPU slice; see below. |
| Windows, CPU pass | GitHub-hosted runner | Actions → **release-gate** → Run workflow |
| macOS, CPU pass | GitHub-hosted runner | Same workflow run as Windows |
| macOS, GPU pass (Apple paravirtual GPU through MoltenVK) | GitHub-hosted runner | Same workflow run. Pixels are valid evidence; timings are not. |
| Linux export route (Android `.apk`, Windows cross `.exe`) | GitHub-hosted Ubuntu runner | Same workflow run, job `linux-export`. Your laptop's gate also runs it if the Android toolchain is installed. |
| Windows / macOS GPU and real windows | A real PC and Mac, if one can be borrowed | `ci/vieww gate --mode gpu`; otherwise WAIVED (see B15) |

1. Freeze a commit and tag it as a candidate. Every run must come from that exact commit, and G0.12 checks it.
2. **Linux:** run `ci/vieww gate --mode cpu`, then `ci/vieww gate --mode gpu`. Keep the laptop plugged in and don't use it heavily while the timing suites run.
3. **Windows and macOS:** push the tag, or run the workflow by hand. It uploads `gate-windows-cpu`, `gate-macos-cpu`, `gate-macos-gpu`, `gate-linux-export`, the installers, and a merged `checklist-ci` that also contains every log.
4. **Merge** the downloaded run folders with the Linux ones:

   ```bash
   ci/vieww checklist target/release-gate/linux-cpu-*/ target/release-gate/linux-gpu-*/ gate-windows-cpu/ gate-macos-cpu/ gate-macos-gpu/ gate-linux-export/ -o CHECKLIST.md
   ```

5. Do the **[manual]** rows by hand on the installed packages.

**Disk:** builds are lean, and each build tree is deleted when its stages finish. Plan for roughly 30 GB free for a CPU pass and under 10 GB for the GPU pass. Both are estimates; `logs/disk.tsv` records the real numbers.

### The GPU pass (`--mode gpu`)

Only the checks a GPU can change. Everything else renders on the CPU and gives the same answer with or without a GPU, so the CPU pass or CI already covered it.

- Device class: real hardware, not a software renderer.
- The Vulkan compositor parity suite (17 pixel-exact tests at 128×96).
- The GPU workload (640×400, 24 frames).
- The fixture census.
- The real-window desktop suite.

Every GPU test is small: its largest render target is a few MB, so a 2 GB GPU is ample. On a laptop with two GPUs, the framework picks the discrete one. Choose explicitly with `VIEWW_VK_DEVICE=<part of the name>`, e.g. `VIEWW_VK_DEVICE=intel`. `vulkaninfo --summary` lists the names. Use whichever GPU has a working Vulkan driver; the Intel HD 620 on Mesa does.

**Verdicts**

- **PASS**: verified, with evidence.
- **FAIL**: a known defect. P0 and P1 items block the release.
- **N/A**: not applicable, with a reason.
- **WAIVED**: an accepted risk, with owner, rationale and issue.

**Release policy:** the beta ships when every P0 below is closed, and every row in the Linux, Windows and macOS columns has a verdict with evidence.

### What "CPU" and "GPU" mean for this product

These are facts about the current code, and they decide what each column can prove.

| Layer | Linux | Windows | macOS |
|---|---|---|---|
| **Pixels users see** | CPU renderer (`vieww_paint::native`) | CPU renderer | CPU renderer |
| **How pixels reach the window** | Vulkan swapchain | Vulkan swapchain | Vulkan through **MoltenVK** |
| **Runtime needed to open any window** | Vulkan loader + driver (Mesa/NVIDIA/AMD) | Vulkan runtime (ships with GPU drivers) | MoltenVK. **Not bundled today (B1).** |
| **GPU compositor** (`vieww-gpu` + Vulkan executor) | Headless only, parity-tested | Headless only | Headless via MoltenVK |
| **D3D12 / Metal executors** | — | **Not implemented**: BUILD-ONLY, not claimed | **Not implemented**: BUILD-ONLY, not claimed |

So "**GPU testing**" means two things:

- Presentation works on the real driver.
- The headless GPU compositor matches the CPU renderer on real hardware.

"**CPU testing**" means the product still works and stays within budget on a weak machine. A machine with no Vulkan driver at all cannot open Studio. That is a documented limitation (B6), not a pass.

Software Vulkan (lavapipe, llvmpipe, SwiftShader) proves correctness only. `--mode gpu` fails on it.

---

## Gate 0: Release identity and repository integrity

| ID | Check | How | Result | Evidence |
|---|---|---|---|---|
| G0.1 | Workspace version is the intended beta version, e.g. `0.1.0-beta.1` | [auto] records the version; a person confirms | ☐ | |
| G0.2 | `Cargo.lock` is current | [auto] `cargo metadata --locked` | ☐ | |
| G0.3 | `rustc` matches `rust-toolchain.toml` (1.98.1) on every machine | [auto] | ☐ | |
| G0.4 | No `.orig`, `.rej` or `.bak` files, nested archives, stale evidence, generated output or local paths | [auto] `ci/vieww clean` | ☐ | |
| G0.5 | Git working tree is clean and the tag points at the certified commit | [auto] plus a person checks the tag | ☐ | |
| G0.6 | `LICENSE` present and correct | [auto] present; [manual] correct | ☐ | |
| G0.7 | No private keys, tokens or credentials | [auto] pattern scan | ☐ | |
| G0.8 | `cargo deny check all` (licences, advisories, bans, sources) | [auto]. Fails if cargo-deny is missing. | ☐ | |
| G0.9 | `README.md` describes the beta and its supported platforms accurately | [manual] | ☐ | |
| G0.10 | Known limitations are documented (release notes template, Known limitations section) | [manual] | ☐ | |
| G0.11 | `CHANGELOG.md` or release notes are finalized. **Neither exists yet (B4).** | [manual] | ☐ | |
| G0.12 | Every run in this checklist used the same source digest (`cert/meta/source.txt`) | [auto] `ci/vieww checklist` across all runs | ☐ | |

## Gate 1: Framework certification

`ci/vieww gate` covers every [auto] row below.

For a single-row rerun:

- `ci/vieww checks`
- `ci/vieww platform <os>`
- `ci/vieww certify` (Windows: `pwsh ci/certify/certify-windows.ps1`)

`checks` runs every stage and lists every failure at the end.

### 1.1 Build and test matrix

Columns: Linux · Windows · macOS

| ID | Check | Linux | Windows | macOS |
|---|---|:-:|:-:|:-:|
| G1.1 | `ci/vieww checks`: fmt, clippy `-D warnings` and docs `-D warnings`, all with `--features vieww-paint/native,vieww-hal/vulkan`; check `--all-targets`; MSRV 1.89; deny; workspace tests; fixtures; hot reload; packaging smoke | ☐ | ☐ | ☐ |
| G1.2 | `ci/vieww platform <os>`: native backends build for this OS | ☐ | ☐ | ☐ |
| G1.3 | `ci/vieww certify`: overall verdict `FAILED=0`. The GPU run should also show `COMPLETE=true`. | ☐ | ☐ | ☐ |

### 1.2 Rendering: CPU and GPU

Columns: Linux CPU · Linux GPU · Windows CPU · Windows GPU · macOS CPU · macOS GPU

| ID | Check | L-CPU | L-GPU | W-CPU | W-GPU | M-CPU | M-GPU |
|---|---|:-:|:-:|:-:|:-:|:-:|:-:|
| G1.4 | GPU device class: real hardware | N/A | ☐ | N/A | ☐ | N/A | ☐ |
| G1.5 | Vulkan compositor pixel parity (17 tests, max channel difference 1) | N/A | ☐ | N/A | ☐ | N/A | ☐ (MoltenVK) |
| G1.6 | GPU census: 23/23 fixtures plan complete, 0 flat-region mismatches | N/A | ☐ | N/A | ☐ | N/A | ☐ |
| G1.7 | CPU renderer: premium UI (0 overflows) and fixtures | ☐ | N/A (CPU work) | ☐ | N/A (CPU work) | ☐ | N/A (CPU work) |
| G1.8 | Animation stress p95 ≤ 16.6 ms **on release-class hardware** (B7). Hosted CI runners are shared, so treat their timings as indicative. | ☐ | N/A (CPU work) | ☐ | N/A (CPU work) | ☐ | N/A (CPU work) |
| G1.9 | Vieww Standard: all 12 clauses measured, 0 steady allocations, 0 unsupported GPU commands | ☐ | N/A (CPU work) | ☐ | N/A (CPU work) | ☐ | N/A (CPU work) |
| G1.10 | Desktop suite: real windows open, first frame is not blank, no panic | ☐ | ☐ | ☐ | ☐ | ☐ | ☐ |
| G1.11 | D3D12 backend | N/A | N/A | N/A | N/A (BUILD-ONLY) | N/A | N/A |
| G1.12 | Metal backend | N/A | N/A | N/A | N/A | N/A | N/A (BUILD-ONLY) |
| G1.13 | Cross-machine shots: pixels byte-identical to another OS's `shots/`. Run `ci/vieww shots --against gate-windows/shots` (and the macOS one) on Linux. | ☐ | N/A (CPU pixels) | ☐ | N/A (CPU pixels) | ☐ | N/A (CPU pixels) |
| G1.14 | A machine **without** a Vulkan driver shows a clear error, not a crash or a blank window | ☐ | N/A | ☐ | N/A | ☐ | N/A |

## Gate 2: Vieww Studio

| ID | Check | Linux | Windows | macOS |
|---|---|:-:|:-:|:-:|
| G2.1 | [auto] `ci/vieww studio` passes in full (not `--quick`): `release_check`, `tour`, `walkthrough` | ☐ | ☐ | ☐ |
| G2.2 | [manual] Gallery `studio/index.html` reviewed: light and dark, normal and small window. No clipped widgets, missing glyphs, overlaps, stray lines or stale layers. Icons aligned. Hover, focus and pressed states intentional. Empty, error and loading states intentional. | ☐ | ☐ | ☐ |
| G2.3 | [auto with `gate --hidpi`] desktop suite on a HiDPI display (N/A if none) | ☐ | ☐ | ☐ |
| G2.4 | [auto with `gate --multi-monitor`] desktop suite across two displays (N/A if none) | ☐ | ☐ | ☐ |
| G2.5 | [manual] Window: move, resize, minimize, restore, close. Dialogs and child windows behave. Exits with no hang. | ☐ | ☐ | ☐ |
| G2.6 | [manual] OS conventions: ⌘ on macOS and Ctrl elsewhere. Native clipboard round-trip. File picker. Data dir: Linux `$XDG_DATA_HOME` or `~/.local/share`; Windows `%APPDATA%`; macOS `~/Library/Application Support`. | ☐ | ☐ | ☐ |

### 2.1 Studio's export route: desktop, Windows, Android, iOS

This is what a user's **Export** button runs, not the framework's own examples. `ci/vieww gate` runs `ci/certify/export-suite.sh`, which:

1. Scaffolds a Rust project and a Say project exactly as New Project does.
2. Compiles both for every target.
3. Runs Studio's own export plans and checks each produced its file.

The projects are built **outside the repository**, like a user's project, so this repo's `.cargo/config.toml` can't affect them. A missing toolchain is recorded as SKIPPED with the install command Studio would show. The hosted CI jobs install everything needed.

**Toolchains each export needs**

| Export | Needs | Hosted CI job |
|---|---|---|
| Desktop binary | cargo | all |
| Windows `.exe` | Windows: nothing extra. Linux/macOS: MinGW-w64 + `rustup target add x86_64-pc-windows-gnu` | windows-cpu (native), linux-export (cross) |
| Android `.apk` | Android SDK (`ANDROID_HOME`), NDK (`ANDROID_NDK_HOME`), JDK 17, Gradle 8.7+, `cargo install cargo-ndk`, `rustup target add aarch64-linux-android` | linux-export, windows-cpu, macos-cpu |
| iOS simulator `.app` | macOS, Xcode, `rustup target add aarch64-apple-ios-sim` | macos-cpu |
| iOS device `.ipa` | The above, plus an Apple signing identity | manual (G2.13) |

A scaffolded project pins `channel = "stable"`, so on your own machine add those targets for `stable` as well, e.g. `rustup target add --toolchain stable aarch64-linux-android`.

| ID | Check | Linux | Windows | macOS |
|---|---|:-:|:-:|:-:|
| G2.7 | [auto] Scaffolded Rust and Say projects compile for desktop, Android and Windows, plus iOS on macOS (`cargo check --target`) | ☐ | ☐ | ☐ |
| G2.8 | [auto] Export → desktop binary is produced | ☐ | ☐ | ☐ |
| G2.9 | [auto] Export → Windows `.exe` is produced (native on Windows, MinGW cross elsewhere) | ☐ | ☐ | ☐ |
| G2.10 | [auto] Export → Android `.apk` is produced (cargo-ndk + Gradle) | ☐ | ☐ | ☐ |
| G2.11 | [auto] Export → iOS simulator `.app` is produced | N/A | N/A | ☐ |
| G2.12 | [manual] The exported `.apk` installs and launches on an arm64 phone (developer mode), and the `.app` launches in the iOS simulator (`xcrun simctl install booted …`) | ☐ | ☐ | ☐ |
| G2.13 | [manual] Signed iOS `.ipa` from a machine with an Apple developer identity (see B3) | N/A | N/A | ☐ |
| G2.14 | [manual] The same exports from an **installed** Studio, not a checkout (see B16) | ☐ | ☐ | ☐ |

## Gate 3: Packaged build and install

| ID | Check | Linux | Windows | macOS |
|---|---|:-:|:-:|:-:|
| G3.1 | [auto] `ci/vieww artifacts`: installers build, and the bundle compiles a guest against its own SDK | ☐ | ☐ | ☐ |
| G3.2 | [manual] Clean machine (VM or fresh user with no checkout and no Rust): install, launch, launcher or Start-menu entry, icon | ☐ | ☐ | ☐ |
| G3.3 | [manual] Uninstall removes the app and **keeps user projects**. Reinstall or upgrade over the previous beta works. | ☐ | ☐ | ☐ |
| G3.4 | [manual] No missing runtime: Linux Vulkan loader; Windows DLLs and Vulkan runtime; macOS **MoltenVK (B1)** | ☐ | ☐ | ☐ |

**Artifacts `packaging/package.sh --installers` produces today**

| OS | Artifacts | Build tools |
|---|---|---|
| Linux | `viewwstudio-linux-x86_64.deb`, `.AppImage`, `.tar.gz` | `dpkg-deb`; `APPIMAGETOOL` must be set |
| Windows | `viewwstudio-windows-x86_64.msi`, `.zip` | WiX (`dotnet tool install --global wix`) |
| macOS | `viewwstudio-macos-<arch>.dmg`, one per architecture | `hdiutil`. **No macOS archive exists**, unlike the draft's assumption. |

Linux and Windows file names say `x86_64` whatever the host. Do not publish ARM64 builds under those names (B8).

## Gate 4: Packaged runtime smoke test

Run the **installed** app. Columns: Linux · Windows · macOS

| ID | Check | L | W | M |
|---|---|:-:|:-:|:-:|
| G4.1 | Launch from installed location; startup; main window renders | ☐ | ☐ | ☐ |
| G4.2 | Create or open a workspace, edit, save, reopen | ☐ | ☐ | ☐ |
| G4.3 | **Preview/guest build using the installed SDK bundle** (not the checkout) | ☐ | ☐ | ☐ |
| G4.4 | Export sheet from the installed app: desktop, and Android where its toolchain is installed (G2.14) | ☐ | ☐ | ☐ |
| G4.5 | Clipboard, file picker, theme switch, resize and DPI | ☐ | ☐ | ☐ |
| G4.6 | Quit and relaunch: session and settings restored | ☐ | ☐ | ☐ |

## Gate 5: Upgrade, persistence and failure recovery

| ID | Check | L | W | M |
|---|---|:-:|:-:|:-:|
| G5.1 | Previous beta's workspace and settings open after upgrade (N/A for the first beta) | ☐ | ☐ | ☐ |
| G5.2 | Unsaved-changes warning is correct | ☐ | ☐ | ☐ |
| G5.3 | A failed guest build is reported clearly; Studio stays usable; the next build succeeds | ☐ | ☐ | ☐ |
| G5.4 | Restart after a failure needs no data deletion | ☐ | ☐ | ☐ |

## Gate 6: Performance and stability

Record **release-hardware** numbers separately from headless results. The measured suites land in `cert/quality/metrics.txt`.

| ID | Check | L | W | M |
|---|---|:-:|:-:|:-:|
| G6.1 | Startup time (cold and warm) recorded | ☐ | ☐ | ☐ |
| G6.2 | Idle CPU and steady-state memory recorded | ☐ | ☐ | ☐ |
| G6.3 | Scroll and animation feel smooth on the weakest supported machine (the CPU-mode machine) | ☐ | ☐ | ☐ |
| G6.4 | 30 minutes of open, edit and rebuild cycles: no growth or degradation | ☐ | ☐ | ☐ |
| G6.5 | Large file (10k+ lines) editing is responsive | ☐ | ☐ | ☐ |

## Gate 7: Accessibility, input and text

| ID | Check | L | W | M |
|---|---|:-:|:-:|:-:|
| G7.1 | Keyboard-only navigation of primary workflows; focus visible | ☐ | ☐ | ☐ |
| G7.2 | Selection, cut, copy, paste, undo, redo, find and replace | ☐ | ☐ | ☐ |
| G7.3 | IME entry (e.g. Japanese, Chinese) | ☐ | ☐ | ☐ |
| G7.4 | Screen reader exposes the main controls (Orca, Narrator, VoiceOver) | ☐ | ☐ | ☐ |
| G7.5 | Unicode, long lines, wrapping and mixed sizes render without layout corruption | ☐ | ☐ | ☐ |

## Gate 8: Supply chain and artifact integrity

| ID | Check | Result |
|---|---|:-:|
| G8.1 | [auto] `SHA256SUMS` generated for every artifact on each OS | ☐ |
| G8.2 | [auto] `MANIFEST.txt` records version, commit, toolchain, host and signing state | ☐ |
| G8.3 | [manual] Merge the three OS files with `ci/vieww sums linux/ windows/ macos/`, then publish as `SHA256SUMS`. The download page links that exact name. | ☐ |
| G8.4 | [auto] `ci/vieww checklist`: all artifacts built from the certified commit (compare `git_rev` in the three manifests) | ☐ |
| G8.5 | [manual] Release archive contents reviewed (`tar tzf`, `unzip -l`, DMG mounted) | ☐ |
| G8.6 | [manual] Reproducibility documented: **not reproducible today** (timestamps, bundled toolchain) | ☐ |
| G8.7 | Optional: SBOM (`cargo cyclonedx`) and provenance attestation | ☐ |

## Gate 9: Signing and trust

`packaging/package.sh` signs nothing. There are two options:

- Ship an **unsigned developer beta** and say so on the download page and in the release notes. Mark G9.* WAIVED with an owner.
- Close B2 and B3.

| ID | Check | Result |
|---|---|:-:|
| G9.1 | Linux: package signing policy decided; integrity via published `SHA256SUMS` documented | ☐ |
| G9.2 | Windows: Authenticode certificate; `.exe` and `.msi` signed; signature validates on a clean machine; SmartScreen behaviour documented | ☐ |
| G9.3 | macOS: Developer ID signed with Hardened Runtime; notarized; ticket stapled; Gatekeeper launch verified on a clean Mac | ☐ |

## Gate 10: Publishing

| ID | Check | Result |
|---|---|:-:|
| G10.1 | Release notes written from `docs/release/RELEASE-NOTES-TEMPLATE.md`: supported OS and arch, install steps, known limitations, signing state | ☐ |
| G10.2 | Tag created from the certified commit; release draft created | ☐ |
| G10.3 | `packaging/site/index.html` `__REPO__` resolved; every artifact name matches the page | ☐ |
| G10.4 | Assets and checksums uploaded and reviewed before publishing | ☐ |
| G10.5 | After publishing: download, verify checksum, install and launch on one clean machine per OS | ☐ |

---

## Known blockers (pre-filled from the repository review)

| ID | Sev | Description | Fix | Status |
|---|---|---|---|---|
| B1 | **P0** | **MoltenVK is not bundled in the macOS `.app`.** Without the Vulkan SDK installed, Studio cannot open a window on a user's Mac. The instance and device portability flags needed for MoltenVK were added in this pass, but have **not been run on a Mac yet**. | Copy `libMoltenVK.dylib` into `Contents/Frameworks` and `MoltenVK_icd.json` into `Contents/Resources/vulkan/icd.d`. `vieww_hal::vulkan::load_entry` now looks there first (`Contents/Frameworks/libvulkan.1.dylib`, then `libMoltenVK.dylib`), then in `$VULKAN_SDK` and Homebrew. The remaining work is copying those files in `package.sh`. Verify with G3.2 and G3.4 on a clean Mac. | Open |
| B2 | P0 for public, P2 for dev preview | Windows artifacts are unsigned (SmartScreen warning) | Authenticode certificate plus a `signtool` step, or WAIVE with disclosure | Open |
| B3 | P0 for public, P2 for dev preview | macOS artifacts are neither signed nor notarized. Gatekeeper blocks the first launch. | Developer ID, Hardened Runtime, `notarytool`, staple, or WAIVE with disclosure | Open |
| B4 | P1 | No `CHANGELOG.md` and no release notes | Fill in `RELEASE-NOTES-TEMPLATE.md` | Open |
| B5 | P1 | Version is `0.1.0`, not a beta pre-release version | Set `[workspace.package] version` and re-lock | Open |
| B6 | P1 | A Vulkan runtime is required on every OS; machines or VMs without one cannot open Studio | Document it in the release notes (G1.14). A CPU-only presentation fallback would be later work. | Open |
| B7 | P1 | `test-animation-stress` misses its 16.6 ms budget. p95 was 24 ms on a 2-core container; on the first real run (Linux, i5-7200U, 2017 laptop) it was **35 ms**, with 72 of 72 frames over budget. | Decide the minimum supported CPU and publish it, or optimise the 200-tile animation path. Re-measure G1.8 on that minimum machine. | Open |
| B8 | P2 | Linux and Windows artifact names hard-code `x86_64` | Use the host arch in `package.sh`, or build x86_64 only | Open |
| B9 | P2 | No hosted CI | `.github/workflows/release-gate.yml` runs the Windows and macOS legs | Fixed |
| B10 | P2 | GPU compositor: geometry edges have no anti-aliasing; not used by the live window | Not user-facing in this beta. Keep it out of the marketing claims. | Accepted |
| B11 | **P0** | **Desktop app segfaulted on real Linux (Wayland, Intel Mesa)** during the desktop suite. Cause: window teardown destroyed the winit window and the Vulkan device while the swapchain and `VkSurfaceKHR` were still alive (Vulkan validation: `VUID-vkDestroyDevice-device-05137`, `VUID-vkDestroyInstance-instance-00629`). Any closing dialog or app exit could crash. | **Fixed** in this tree (`Drop for Gpu` releases the swapchain first). Validation errors are now 0 on X11 and Wayland. Confirm with G1.10 on the same laptop. | Fixed, awaiting re-run |
| B12 | P1 | Vieww Standard startup took 52.7 ms against a 33.3 ms budget on the i5-7200U. The same run measured the **system font scan at 46 s**. Every window runs that scan synchronously before its first frame (`use_system_fonts`), so Studio may take tens of seconds to open on slower disks. | Measure Studio launch time (G6.1) on that laptop. If it's slow, move the scan off the first frame or cache it. | Open |
| B13 | P2 | Layout overflow warnings while Studio runs: `RenderRow` overflowed by 9 px (release_check) and by up to 73 px (walkthrough); `RenderColumn` overflowed by 41 px in the desktop demo. | Review in G2.2; fix or accept | Open |
| B14 | P2 | On a pure Wayland session with no XWayland, the clipboard does not work: `arboard` is built without Wayland data-control. | Document it, or enable `arboard`'s `wayland-data-control` feature and test on GNOME and KDE | Open |
| B15 | P1 | **No real Windows or Mac in the test plan.** Hosted CI runners have no GPU and no Vulkan driver, so Studio cannot open a window there. Automated builds, tests and installers are covered, but "the installed app opens and works" (G1.10, G3.2–G3.4, G4) is not, on either OS. | Borrow one Windows PC (any GPU with a current driver) and one Mac for an hour each: install the CI-built installer and do G3.2 + G4. Run `ci/vieww gate --mode gpu` there if Rust is installed. Otherwise WAIVE for a Linux-first developer preview and say so in the release notes. | Open |
| B16 | **P0 for exports** | **A project created by an installed Studio cannot be built.** New Project writes `vieww = { path = <checkout> }` only when the Studio binary finds the source checkout it was compiled from (`scaffold::checkout_root`, a compile-time path). On a user's machine that path doesn't exist, so the project gets `vieww = "0.0.1"`. No such crate is on crates.io, and the workspace version is 0.1.0. Preview still works (it uses the bundled SDK), but `cargo build`, Export and every mobile build fail with a dependency resolution error. Invisible on a developer machine, where the checkout exists. | Publish the vieww crates at the workspace version and scaffold that version, or ship the crate sources in the SDK bundle and scaffold a path to them. Verify with G2.14. | Open |
| B17 | P1 | **Android templates did not compile.** The Say template called `run_android(mount, android)` with its arguments swapped, and both templates called `log::error!` without depending on `log`. Hidden because those lines are `#[cfg(target_os = "android")]`, which no desktop build compiles. | **Fixed** in this tree, with a regression test (`the_mobile_entry_points_match_the_platform_api`) and a real Android compile in G2.7. Confirm on CI. | Fixed, awaiting CI |
| B18 | P1 | **iOS simulator export could not work.** It demanded a signing identity (simulator builds don't need one), and the `.app` stayed in Xcode's DerivedData, so the export folder never got the file. | **Fixed**: identity only required for `.ipa`; `xcodebuild` writes into the export folder. Confirm with G2.11 on macOS CI. | Fixed, awaiting CI |
| B19 | P2 | Android export builds `arm64-v8a` only, so it won't install on an x86_64 emulator on an Intel/AMD PC. | Document, or add `x86_64` to the `cargo ndk -t` list and `abiFilters` | Open |
| B20 | P2 | The scaffold pins `channel = "stable"`, but Studio's Toolchains view checks the targets of Studio's own toolchain (1.98.1 here). A user can see "ready" and still have the export fail with a missing target for `stable`. | Pin the scaffold to the Studio's exact version, or check targets inside the project directory | Open |
| B21 | P1 | Vulkan compositor diverges on NVIDIA (Tesla T4, Linux): 10/17 parity tests fail — every test that runs a post pass (layers, blur, blend, shadow, mask); plain draws, gradients, images and strokes pass; a steady frame renders differently the second time; workload parity 31%, census flat-region mismatch. lavapipe passes 17/17 with 0 core validation errors. Not user-facing today (Studio presents CPU pixels, B10), but blocks G1.5/G1.6. | Shader image loads are now bounds-checked (naga `ReadZeroSkipWrite`) to remove one class of driver-dependent UB; `certify` re-runs a failing suite under the Khronos validation layer (`gpu/vulkan-validation.txt`). Re-run `ci/vieww gate --mode gpu` on NVIDIA with `vulkan-validationlayers` installed and fix what it names | Open |

---

## Platform evidence records

Paste `host.txt` from each gate run.

| Field | Linux | Windows | macOS |
|---|---|---|---|
| Machine / OS version | | | |
| Arch / CPU | | | |
| GPU / driver / Vulkan device | | | |
| Display server | | N/A | N/A |
| Rust / Cargo | | | |
| Commit / source digest | | | |
| Gate folders (gpu, cpu) | | | |
| Artifacts + SHA256SUMS | | | |
| Signing status | | | |
| Known deviations | | | |
| **Verdict** | PASS / FAIL / WAIVED | PASS / FAIL / WAIVED | PASS / FAIL / WAIVED |

## Final release decision

| Gate | Status | Owner | Evidence |
|---|---|---|---|
| 0 Repository / identity | ☐ | | |
| 1 Framework (CPU + GPU) | ☐ | | |
| 2 Studio | ☐ | | |
| 3–4 Packaged install / runtime | ☐ | | |
| 5 Upgrade / recovery | ☐ | | |
| 6 Performance | ☐ | | |
| 7 Accessibility / input | ☐ | | |
| 8 Supply chain | ☐ | | |
| 9 Signing / trust | ☐ | | |
| 10 Publishing | ☐ | | |
| Blockers B1–B21 closed or waived | ☐ | | |

**BETA RELEASE: APPROVED / NOT APPROVED**

Approved by: ______ Date: ______ Certified commit: ______
