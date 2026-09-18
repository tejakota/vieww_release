//! What this machine actually has, asked at runtime and answered with a reason.
//!
//! ```no_run
//! # fn a_real_window_reaches_wait() {
//! vieww_hardware::skip_without!(display);
//! // ... a test that opens a window
//! # }
//! ```
//!
//! # Why this crate exists
//!
//! `NEXT.md` has had the same shape for several sessions: a list of things that
//! are true only if hardware says so, and no hardware. Items 1 through 4 are all
//! *measurements that exist and have never been taken on a device*, and item 3
//! is sharper than that — four files in this tree disagree about whether a frame
//! has ever reached an iPhone, and the disagreement survives because nothing in
//! the repository can be asked.
//!
//! So this crate turns "we cannot test that here" from a line in a document into
//! a value in the program. A test that needs a screen calls
//! [`skip_without!`]`(display)`; on a machine with a screen it runs, and on one
//! without it prints *why* it did not and passes. The gap stops being an
//! argument and becomes a check.
//!
//! # A skip that nobody notices is worse than no test
//!
//! This is the failure mode the crate is built around, and it is worth stating
//! plainly because the obvious design walks straight into it.
//!
//! A test that skips prints a line and exits green. A test that skips *for ever*
//! — because CI's image lost its `DISPLAY`, because the runner's GPU stopped
//! reporting `INDIRECT_EXECUTION`, because `adb` fell out of the image — also
//! prints a line and exits green, and the two are indistinguishable from the
//! exit code, which is the only thing anybody looks at. The repository would
//! then contain a test for the one bug class 1,066 other tests cannot reach
//! (`docs/PRODUCTION-GAPS.md` §6) and get nothing from it, while the coverage
//! *claim* would read exactly as it does now.
//!
//! [`VIEWW_REQUIRE_HARDWARE`] is the answer. A machine that is *supposed* to
//! have a screen sets `VIEWW_REQUIRE_HARDWARE=display` and a missing one becomes
//! a panic instead of a skip. The check is then honest in both directions: it
//! skips where hardware genuinely is not, and it fails where hardware was
//! promised and is not there. Without that variable set somewhere in CI, treat
//! every gated test in this workspace as unproven — the point of this paragraph
//! is that nothing else in the output will tell you.
//!
//! # What a probe can and cannot establish
//!
//! Every probe here answers a *necessary* condition, never a sufficient one. A
//! `DISPLAY` can be set and point at a server that refuses the connection; `adb`
//! can list a device that is about to disconnect; a simulator that exists is not
//! a phone. Each [`Capability`]'s documentation says exactly what its answer
//! covers, and a `Present` is always "there is something here to try", never
//! "this will work".
//!
//! The [`Availability`] reason is therefore part of the value rather than a
//! nicety. A test that prints `skipped` teaches nobody anything; one that prints
//! `no DISPLAY and no WAYLAND_DISPLAY in the environment` tells a reader in one
//! line whether they are looking at a headless container or a broken session.
//!
//! [`VIEWW_REQUIRE_HARDWARE`]: REQUIRE_VAR

use std::sync::OnceLock;

/// The environment variable that turns a skip into a failure.
///
/// A comma-separated list of capability names — `display`, `gpu`,
/// `android-device`, `ios-device`, `ios-simulator` — or `all`. Case and
/// surrounding whitespace do not matter, and `_` reads the same as `-`, because
/// a CI file that says `android_device` meant `android-device` and refusing it
/// would only cost somebody an afternoon.
///
/// ```console
/// VIEWW_REQUIRE_HARDWARE=display,gpu cargo test -p vieww-platform-winit
/// ```
///
/// # An unknown name is an error, deliberately
///
/// `VIEWW_REQUIRE_HARDWARE=dispaly` under a lenient parser requires nothing,
/// runs nothing, and reports success — the exact outcome the variable exists to
/// prevent, produced by the exact kind of typo nobody proofreads a CI file for.
/// So a name this crate does not know panics at the first gate that consults it,
/// naming the ones it does know.
pub const REQUIRE_VAR: &str = "VIEWW_REQUIRE_HARDWARE";

/// A piece of hardware — or of the operating system's furniture — that a test
/// can need.
///
/// Copy, and named rather than a bare string, so `skip_without!(dispaly)` is a
/// compile error rather than a silent pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// Somewhere to put a window.
    ///
    /// **What it establishes.** On Linux and the other free unixes, that
    /// `WAYLAND_DISPLAY` or `DISPLAY` is set to something non-empty — which is
    /// what winit reads to find a compositor or an X server, and the absence of
    /// both is the definitive headless case. On macOS and Windows the window
    /// server is part of the operating system, and on Android and iOS an
    /// application *is* a surface, so all four are present by construction.
    ///
    /// **What it does not.** That the server will accept a connection. A stale
    /// `DISPLAY` left over from a closed SSH session, an X server that refuses
    /// on `MIT-MAGIC-COOKIE`, or a macOS process with no window server session
    /// (a plain `ssh` login, or a daemon) all pass this probe and then fail to
    /// open a window. Connecting for real would mean creating an event loop,
    /// which winit permits **once per process** on several platforms — so a
    /// probe that did it would spend the one event loop the test then needs.
    /// That trade is the whole reason this is a necessary condition rather than
    /// a sufficient one.
    Display,

    /// A graphics adapter that can run vieww's renderer — `vieww-hal`'s
    /// Vulkan backend, which `vieww-paint`'s `native` renderer presents
    /// through (see `docs/RENDERER-MIGRATION.md`).
    ///
    /// **What it establishes.** That
    /// [`vieww_hal::vulkan::VulkanDevice::new`] succeeds — a real Vulkan 1.2
    /// instance and logical device, the same call `vieww-hal`'s own
    /// `tests/vulkan_smoke.rs` makes, rather than a second, drifting Vulkan
    /// bring-up of our own. This capability used to probe a `wgpu`/vello
    /// device instead (`vieww_paint::gpu::GpuRenderer::headless`), and used
    /// to have a Vulkan-probing sibling named `CustomRenderer` beside it —
    /// both gone now that vello is gone and `vieww-hal` is the only GPU
    /// stack in the workspace, so there is only the one capability to name.
    ///
    /// **What it does not.** That any particular frame will render, that
    /// there is memory for a large surface, that the device can present to a
    /// window (`for_window` needs a specific surface, and a machine with two
    /// GPUs can have one that cannot present to a window the other created),
    /// that it is hardware rather than a software rasterizer such as
    /// `lavapipe`, or that any shader beyond the HAL's current clear-colour
    /// smoke test will run.
    ///
    /// # Why it is behind a feature
    ///
    /// Enabling it puts `vieww-hal` (and, through its `vulkan` feature,
    /// `ash` and `naga`) in the build graph. A test that only needs to know
    /// whether there is a `DISPLAY` should not pay for a graphics stack to
    /// find out, and most consumers of this crate are that test.
    ///
    /// Without the feature the variant does not exist, so `skip_without!(gpu)`
    /// fails to compile. That is deliberate and is the same argument as the
    /// unknown-name panic above: the alternative is a probe that answers
    /// "absent, because this crate was built without the feature", which is a
    /// permanent silent skip wearing a reason.
    #[cfg(feature = "gpu")]
    Gpu,

    /// An Android device or emulator, attached and authorised.
    ///
    /// **What it establishes.** That `adb devices` runs and lists at least one
    /// entry in state `device`. Entries in `unauthorized` (the phone has not
    /// been told to trust this host), `offline`, `no permissions` and
    /// `recovery` are all counted as absent and named in the reason, because
    /// those are the four states somebody stares at a blank `adb shell` over.
    ///
    /// **What it does not.** Which device, what API level, whether it has a GPU
    /// vieww can run on, or whether an APK can be installed on it. Nor does it
    /// distinguish a physical phone from an emulator — `adb` does not, in this
    /// listing, and the distinction that matters for `NEXT.md` items 1 and 2 is
    /// a real handset, which only the serial number hints at.
    AndroidDevice,

    /// A physical iOS device, connected and visible to Xcode's tooling.
    ///
    /// **What it establishes.** That `xcrun xctrace list devices` runs and its
    /// `== Devices ==` section contains an entry that looks like an iOS device —
    /// a name, a parenthesised OS version, and a parenthesised identifier. The
    /// version is what separates a phone from the host Mac, which is listed in
    /// the same section with an identifier and no version.
    ///
    /// **What it does not, and this is the point of `NEXT.md` item 3.** That the
    /// device is unlocked, that it trusts this host, that a provisioning profile
    /// exists for it, or that anything has ever been installed on it — and
    /// therefore *not* that a frame has ever reached an iPhone. It answers the
    /// strictly weaker question "is there a phone plugged in that
    /// `ci/mobile/ios-app.sh` could be pointed at". That is still the question nobody
    /// in this repository can currently answer, which is why four files disagree
    /// about the stronger one.
    IosDevice,

    /// An iOS simulator that could be booted.
    ///
    /// **What it establishes.** That `xcrun simctl list devices available` runs
    /// and lists at least one device under an iOS runtime. `available` is doing
    /// real work in that command: it excludes simulators whose runtime has been
    /// deleted, which otherwise list happily and fail on boot.
    ///
    /// **What it does not.** That the simulator will boot, and — much more
    /// importantly — **it is not a substitute for [`IosDevice`]**. A simulator
    /// runs the x86/ARM host build against a host GPU; it exercises the UIKit
    /// glue in `vieww-platform-winit/src/ios.rs` but says nothing about a real
    /// GPU, a real refresh rate, or a real thermal envelope. Kept as a separate
    /// capability from `IosDevice` for exactly that reason: a test that gates on
    /// the simulator must not be read as evidence about a phone.
    ///
    /// [`IosDevice`]: Capability::IosDevice
    IosSimulator,
}

/// Every capability name this crate knows, including ones the current build
/// cannot probe.
///
/// The second half is what makes `VIEWW_REQUIRE_HARDWARE=gpu` on a build without
/// the `gpu` feature say *"`gpu` needs vieww-hardware's `gpu` feature"* rather
/// than *"unknown capability"*, which is a two-hour difference to whoever set
/// the variable.
const KNOWN_NAMES: &[&str] = &[
    "display",
    "gpu",
    "android-device",
    "ios-device",
    "ios-simulator",
];

impl Capability {
    /// The name this capability answers to in [`REQUIRE_VAR`] and in messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Display => "display",
            #[cfg(feature = "gpu")]
            Self::Gpu => "gpu",
            Self::AndroidDevice => "android-device",
            Self::IosDevice => "ios-device",
            Self::IosSimulator => "ios-simulator",
        }
    }

    /// Everything this build can probe.
    ///
    /// Shorter than `KNOWN_NAMES` when the `gpu` feature is off, and that
    /// difference is the whole reason both exist.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Display,
            #[cfg(feature = "gpu")]
            Self::Gpu,
            Self::AndroidDevice,
            Self::IosDevice,
            Self::IosSimulator,
        ]
    }

    /// Ask the machine, once.
    ///
    /// # Cached, and why that is safe
    ///
    /// A display does not appear halfway through a test binary, and the two
    /// probes that shell out cost tens of milliseconds each — `xcrun` on a cold
    /// Xcode is worse. So each answer is computed once per process and held in a
    /// `static`.
    ///
    /// **The cache holds the probe, not the decision.** [`gate`](Self::gate)
    /// re-reads [`REQUIRE_VAR`] on every call, so a test that sets the variable
    /// with `std::env::set_var` and then gates is not silently answered from a
    /// cache filled before it. Caching the *gate* would have been the obvious
    /// shape and would have made the override untestable — and an override this
    /// crate cannot test is an override nobody should trust, given what the
    /// module docs claim it is for.
    #[must_use]
    pub fn probe(self) -> &'static Availability {
        match self {
            Self::Display => {
                static CELL: OnceLock<Availability> = OnceLock::new();
                CELL.get_or_init(probe_display)
            }
            #[cfg(feature = "gpu")]
            Self::Gpu => {
                static CELL: OnceLock<Availability> = OnceLock::new();
                CELL.get_or_init(probe_gpu)
            }
            Self::AndroidDevice => {
                static CELL: OnceLock<Availability> = OnceLock::new();
                CELL.get_or_init(probe_android)
            }
            Self::IosDevice => {
                static CELL: OnceLock<Availability> = OnceLock::new();
                CELL.get_or_init(probe_ios_device)
            }
            Self::IosSimulator => {
                static CELL: OnceLock<Availability> = OnceLock::new();
                CELL.get_or_init(probe_ios_simulator)
            }
        }
    }

    /// Whether the capability is here, with the reason discarded.
    ///
    /// For a caller that is branching rather than skipping. A test should use
    /// [`skip_without!`] instead — a skip whose reason was thrown away is the
    /// thing this crate is against.
    #[must_use]
    pub fn is_available(self) -> bool {
        self.probe().is_present()
    }

    /// What a test should do about this capability: run, or skip and say why.
    ///
    /// # Panics
    ///
    /// If [`REQUIRE_VAR`] demands this capability and it is absent — that is the
    /// variable's entire job — or if it names a capability this crate does not
    /// know, or names one this build was not compiled to probe.
    #[must_use]
    pub fn gate(self) -> Gate {
        let raw = std::env::var(REQUIRE_VAR).unwrap_or_default();
        let required = match requires(&raw, self) {
            Ok(required) => required,
            // The variable itself is wrong, so *nothing* it says can be
            // trusted — including its silence about this capability. Panicking
            // is the only answer that cannot be mistaken for a pass.
            Err(error) => panic!("{REQUIRE_VAR}: {error}"),
        };

        match self.probe() {
            Availability::Present(_) => Gate::Run,
            Availability::Absent(reason) => {
                assert!(
                    !required,
                    "{REQUIRE_VAR} requires `{}`, and it is not here: {reason}",
                    self.name()
                );
                Gate::Skip(format!("no {}: {reason}", self.name()))
            }
        }
    }
}

/// Every capability and what the machine said about it, one per line.
///
/// For printing once at the top of a device run, so that a report from somebody
/// else's laptop carries what their laptop had. A frame number without the
/// machine that produced it is the kind of measurement `docs/AIMS.md` §F
/// complains about.
#[must_use]
pub fn summary() -> String {
    let mut out = String::new();
    for capability in Capability::all() {
        let availability = capability.probe();
        let mark = if availability.is_present() {
            "yes"
        } else {
            "no "
        };
        out.push_str(&format!(
            "{mark} {:<14} {}\n",
            capability.name(),
            availability.reason()
        ));
    }
    out
}

/// Whether a capability is here, and how that was established either way.
///
/// The reason is not decoration. Both variants carry one because both are worth
/// printing: an absent reason tells a reader what to fix, and a present one
/// tells them what the probe actually saw — `DISPLAY=:0` and
/// `WAYLAND_DISPLAY=wayland-1` are different machines, and a screenshot test
/// that fails on one and not the other starts with knowing which it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// It is here. The string says how we know.
    Present(String),
    /// It is not. The string says exactly what was looked for and not found.
    Absent(String),
}

impl Availability {
    /// A `Present` with its evidence.
    fn present(reason: impl Into<String>) -> Self {
        Self::Present(reason.into())
    }

    /// An `Absent` with what was looked for.
    fn absent(reason: impl Into<String>) -> Self {
        Self::Absent(reason.into())
    }

    /// Whether the capability is here.
    #[must_use]
    pub const fn is_present(&self) -> bool {
        matches!(self, Self::Present(_))
    }

    /// The reason, whichever way the answer went.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::Present(reason) | Self::Absent(reason) => reason,
        }
    }
}

/// What a gated test should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// The hardware is here. Run the test.
    Run,
    /// It is not. The string is the whole explanation, ready to print.
    Skip(String),
}

/// Whether `raw` — the contents of [`REQUIRE_VAR`] — demands `capability`.
///
/// Split out from [`Capability::gate`] and given a plain string parameter so the
/// decision can be tested on a machine with none of this hardware, which is the
/// machine this crate was written on. The unit tests below are the only ones
/// that can run everywhere, so the parsing is where the testable behaviour was
/// deliberately concentrated.
///
/// # Errors
///
/// A name this crate does not know, or one it knows and this build cannot probe.
/// Both are the caller's typo or misconfiguration rather than a fact about the
/// machine, so neither is expressible as `false`.
fn requires(raw: &str, capability: Capability) -> Result<bool, String> {
    let mut required = false;
    for token in raw.split(',') {
        // `-` and `_` read the same, and an empty token — a trailing comma, or
        // the empty string when the variable is unset — asks for nothing.
        let token = token.trim().to_ascii_lowercase().replace('_', "-");
        if token.is_empty() {
            continue;
        }
        if token == "all" {
            required = true;
            continue;
        }
        if !KNOWN_NAMES.contains(&token.as_str()) {
            return Err(format!(
                "unknown capability `{token}`. Known: {}, or `all`",
                KNOWN_NAMES.join(", ")
            ));
        }
        // Known, but not compiled in — which today means exactly one thing.
        #[cfg(not(feature = "gpu"))]
        if token == "gpu" {
            return Err(
                "`gpu` needs vieww-hardware's `gpu` feature, and this build does not have it"
                    .to_owned(),
            );
        }
        if token == capability.name() {
            required = true;
        }
    }
    Ok(required)
}

// ---------------------------------------------------------------------------
// The probes
// ---------------------------------------------------------------------------

/// The display probe on the platforms where a window server is part of the
/// operating system.
///
/// Not merely a `true`: the reason is what a reader sees when a *different*
/// capability skips on the same machine, and "present by construction" beside
/// "no adapter" says immediately which half is missing.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn probe_display() -> Availability {
    Availability::present("the window server is part of the OS on this target")
}

/// The display probe on the two targets where the application *is* a surface.
///
/// A process running at all on Android or iOS was started by an activity or a
/// scene, so there is nothing to look for. The caveat is real and belongs in
/// this comment rather than in a reason string: the surface may be gone —
/// `Lifecycle` in `vieww-platform-winit` exists because Android takes it away —
/// and this probe cannot see that.
#[cfg(any(target_os = "android", target_os = "ios"))]
fn probe_display() -> Availability {
    Availability::present("an app on this target has a surface by construction")
}

/// The display probe everywhere else, which in practice means Linux and the
/// BSDs.
#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "android",
    target_os = "ios"
)))]
fn probe_display() -> Availability {
    display_from_env(
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("DISPLAY").ok().as_deref(),
    )
}

/// The Linux display decision, as a function of the two variables.
///
/// Compiled on every target and unit-tested on every target, unlike the probe
/// that calls it. The rule it encodes is winit's: it looks for a Wayland
/// compositor first and falls back to X11, so a session with both set is a
/// Wayland session with `XWayland` available, and naming both in the reason is
/// how a reader knows which backend a failing test was on.
///
/// An **empty** value counts as unset. `DISPLAY=` is what a shell leaves behind
/// when something unsets it badly, and it is not a display — but
/// `std::env::var` returns `Ok("")` for it, so the obvious `is_ok()` check
/// reports a screen that is not there. That is a silent *anti*-skip: the test
/// runs and fails with a window-creation error instead of skipping with a
/// reason.
///
/// Only the Linux probe calls it outside tests, so on the other desktop
/// targets it is dead in the library build — and `-D warnings` made that the
/// clippy failure on the macOS and Windows runners.
#[cfg_attr(
    any(
        target_os = "macos",
        target_os = "windows",
        target_os = "android",
        target_os = "ios"
    ),
    allow(dead_code)
)]
fn display_from_env(wayland: Option<&str>, x11: Option<&str>) -> Availability {
    let wayland = wayland.filter(|value| !value.is_empty());
    let x11 = x11.filter(|value| !value.is_empty());
    match (wayland, x11) {
        (Some(wayland), Some(x11)) => Availability::present(format!(
            "WAYLAND_DISPLAY={wayland} (and DISPLAY={x11} for XWayland)"
        )),
        (Some(wayland), None) => Availability::present(format!("WAYLAND_DISPLAY={wayland}")),
        (None, Some(x11)) => Availability::present(format!("DISPLAY={x11}")),
        (None, None) => {
            Availability::absent("no DISPLAY and no WAYLAND_DISPLAY in the environment")
        }
    }
}

/// The adapter probe: the renderer's own answer, not a second opinion.
///
/// # Why the device is created and never dropped
///
/// The same reason `vieww-paint`'s own GPU tests hold one: creating a `wgpu`
/// device per test and dropping them all at process exit segfaulted
/// intermittently in driver teardown on Intel/Mesa, *after* every test had
/// passed. This crate would otherwise add exactly one more create-and-drop to
/// every binary that gates on the GPU, which is the shape that crashes. So the
/// probed renderer is parked in a `static` and outlives the process's interest
/// in it.
///
/// It costs one device. A caller that then builds its own gets a second, which
/// is what an application does anyway.
/// **The verdict is what gets cached, and the device is not cached at all.**
/// [`VulkanDevice`] owns a raw `ash::Instance`/`ash::Device` with a `Drop`
/// impl that tears them down, and nothing outside this function needs the
/// device again, so `mem::forget` parks it forever rather than dropping it
/// and making a caller who then wants one pay Vulkan instance creation
/// twice.
///
/// This function is called once per process because `probe` caches it; the
/// forget therefore leaks exactly one device, which is what the paragraph
/// above is asking for.
///
/// [`VulkanDevice`]: vieww_hal::vulkan::VulkanDevice
#[cfg(feature = "gpu")]
fn probe_gpu() -> Availability {
    use vieww_hal::vulkan::VulkanDevice;
    use vieww_hal::Device;

    match VulkanDevice::new() {
        Ok(device) => {
            let info = device.info();
            let reason = format!("{} ({})", info.name, info.device_type);
            std::mem::forget(device);
            Availability::present(reason)
        }
        Err(error) => Availability::absent(format!(
            "no Vulkan device: {error} (needs a Vulkan loader and an ICD — a \
             real GPU's driver, or software Vulkan such as lavapipe)"
        )),
    }
}

/// Run a tool and hand back its stdout, or say why that did not happen.
///
/// The two failures are told apart on purpose. A missing binary is a
/// *configuration* fact — no Android SDK on this machine — and a non-zero exit
/// is the tool refusing, which on `adb` usually means the server could not
/// start. A single "adb failed" would send a reader looking in the wrong place.
fn run(tool: &str, args: &[&str]) -> Result<String, String> {
    match std::process::Command::new(tool).args(args).output() {
        Ok(output) if output.status.success() => {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Ok(output) => Err(format!(
            "`{tool} {}` exited {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Err(format!("`{tool}` could not be run: {error}")),
    }
}

/// The Android probe.
fn probe_android() -> Availability {
    match run("adb", &["devices"]) {
        Ok(listing) => android_from_listing(&listing),
        Err(error) => Availability::absent(error),
    }
}

/// The `adb devices` decision, as a function of the listing.
///
/// Pure so it can be tested without an SDK. The format is a header line
/// followed by `serial<TAB>state`, and the states that are not `device` are
/// carried into the reason rather than collapsed to "none": `unauthorized` means
/// go and tap "Allow" on the phone, and that is a different afternoon from "the
/// cable is out".
fn android_from_listing(listing: &str) -> Availability {
    let mut ready = Vec::new();
    let mut waiting = Vec::new();
    for line in listing.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let (Some(serial), Some(state)) = (fields.next(), fields.next()) else {
            continue;
        };
        if state == "device" {
            ready.push(serial.to_owned());
        } else {
            waiting.push(format!("{serial} ({state})"));
        }
    }

    if ready.is_empty() {
        if waiting.is_empty() {
            Availability::absent("`adb devices` listed nothing attached")
        } else {
            Availability::absent(format!(
                "`adb devices` listed nothing in state `device`: {}",
                waiting.join(", ")
            ))
        }
    } else {
        Availability::present(format!("adb: {}", ready.join(", ")))
    }
}

/// The physical-iOS-device probe.
///
/// `xctrace` rather than `simctl`, because `simctl` only ever knows about
/// simulators — asking it about a phone is a category error that reads as a
/// working check.
fn probe_ios_device() -> Availability {
    match run("xcrun", &["xctrace", "list", "devices"]) {
        Ok(listing) => ios_device_from_listing(&listing),
        Err(error) => Availability::absent(error),
    }
}

/// The `xctrace list devices` decision, as a function of the listing.
///
/// The output is sections — `== Devices ==`, sometimes `== Devices Offline ==`,
/// then `== Simulators ==` — and only the first is about hardware that is here
/// now. Inside it the host Mac appears as `Name (UDID)` and a phone as
/// `Name (18.0) (UDID)`, so **two** parenthesised groups is the discriminator,
/// and it is a heuristic on a tool with no machine-readable mode rather than a
/// contract. If Apple changes the format this reports "no device" on a machine
/// that has one, which is the safe direction to be wrong in: a skip that should
/// have run, not a claim that should not have been made.
fn ios_device_from_listing(listing: &str) -> Availability {
    let mut devices = Vec::new();
    let mut in_devices = false;
    for line in listing.lines() {
        let line = line.trim();
        if line.starts_with("==") {
            in_devices = line == "== Devices ==";
            continue;
        }
        if !in_devices || line.is_empty() {
            continue;
        }
        // `Name (18.0) (00008110-...)`: a version group and an identifier group.
        // The host Mac has only the identifier.
        if line.matches('(').count() >= 2 {
            devices.push(line.to_owned());
        }
    }

    if devices.is_empty() {
        Availability::absent(
            "`xcrun xctrace list devices` showed no iOS hardware under `== Devices ==` \
             (the host Mac is listed there and does not count)",
        )
    } else {
        Availability::present(format!("xctrace: {}", devices.join("; ")))
    }
}

/// The simulator probe.
fn probe_ios_simulator() -> Availability {
    match run("xcrun", &["simctl", "list", "devices", "available"]) {
        Ok(listing) => ios_simulator_from_listing(&listing),
        Err(error) => Availability::absent(error),
    }
}

/// The `simctl list devices available` decision, as a function of the listing.
///
/// Runtimes are `-- iOS 17.0 --` headers and devices are indented lines ending
/// in a state. Only the iOS runtimes count: a Mac with Xcode installed has
/// watchOS and tvOS simulators too, and neither can run this framework.
///
/// A booted one is named first in the reason, because "there is a simulator" and
/// "there is a simulator already running" are minutes apart in practice.
fn ios_simulator_from_listing(listing: &str) -> Availability {
    let mut available = Vec::new();
    let mut booted = Vec::new();
    let mut in_ios = false;
    for line in listing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            in_ios = trimmed.starts_with("-- iOS");
            continue;
        }
        if trimmed.starts_with("==") {
            in_ios = false;
            continue;
        }
        if !in_ios || trimmed.is_empty() {
            continue;
        }
        if trimmed.ends_with("(Booted)") {
            booted.push(trimmed.to_owned());
        }
        available.push(trimmed.to_owned());
    }

    if available.is_empty() {
        Availability::absent(
            "`xcrun simctl list devices available` listed no simulator on an iOS runtime",
        )
    } else if booted.is_empty() {
        Availability::present(format!(
            "simctl: {} available, none booted (first: {})",
            available.len(),
            available[0]
        ))
    } else {
        Availability::present(format!("simctl: booted {}", booted.join("; ")))
    }
}

/// Turn a name a test wrote into a [`Capability`].
///
/// Public because [`skip_without!`] expands to it in the caller's crate, and
/// hidden because nobody should write it by hand. The identifiers are the
/// [`REQUIRE_VAR`] names with `-` written as `_`, so that one vocabulary covers
/// the source and the environment.
#[doc(hidden)]
#[macro_export]
macro_rules! capability {
    (display) => {
        $crate::Capability::Display
    };
    (gpu) => {
        $crate::Capability::Gpu
    };
    (android_device) => {
        $crate::Capability::AndroidDevice
    };
    (ios_device) => {
        $crate::Capability::IosDevice
    };
    (ios_simulator) => {
        $crate::Capability::IosSimulator
    };
}

/// Return from a test, saying why, unless the hardware is here.
///
/// ```no_run
/// # fn a_window_opens() {
/// vieww_hardware::skip_without!(display);
/// # }
/// ```
///
/// Names are `display`, `gpu`, `android_device`, `ios_device` and
/// `ios_simulator`, and several may be given at once:
/// `skip_without!(display, gpu)`. A name this crate does not have is a compile
/// error, which is the difference between this and a string.
///
/// # What it expands to, and why it prints
///
/// A `return` and an `eprintln!`, in the shape the workspace's GPU tests
/// already use — `eprintln!("skipping: no Vulkan device")` wherever a test
/// treats the absence of one as "skip" rather than "fail". This is that
/// convention with the reason filled in by a probe instead of by hand, so a
/// reader of `cargo test -- --nocapture` sees the same line for the same
/// meaning wherever it comes from.
///
/// It **panics instead** when [`REQUIRE_VAR`] names the capability. See this
/// crate's docs on why a permanently-skipping test is the failure mode worth
/// building around.
///
/// # It only works in a function returning `()`
///
/// Which is every `#[test]`, and the reason this is a macro rather than a
/// function: a function cannot return from its caller.
#[macro_export]
macro_rules! skip_without {
    ($($capability:ident),+ $(,)?) => {
        $(
            if let $crate::Gate::Skip(reason) = $crate::capability!($capability).gate() {
                eprintln!("skipping: {reason}");
                return;
            }
        )+
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- the display decision -------------------------------------------

    #[test]
    fn no_display_variables_is_absent_and_says_so() {
        let answer = display_from_env(None, None);
        assert_eq!(
            answer,
            Availability::absent("no DISPLAY and no WAYLAND_DISPLAY in the environment")
        );
        // The reason is the product here, not the boolean: this exact string is
        // what a skipped window test prints, and a test that only checked
        // `is_present` would let it silently become "".
        assert!(answer.reason().contains("WAYLAND_DISPLAY"));
    }

    #[test]
    fn either_variable_alone_is_a_display() {
        assert!(display_from_env(Some("wayland-1"), None).is_present());
        assert!(display_from_env(None, Some(":0")).is_present());
    }

    #[test]
    fn both_variables_names_both_because_the_backend_is_the_question() {
        let answer = display_from_env(Some("wayland-1"), Some(":0"));
        assert!(answer.reason().contains("wayland-1"), "{answer:?}");
        assert!(answer.reason().contains(":0"), "{answer:?}");
    }

    /// `DISPLAY=` is not a display, and treating it as one produces a test that
    /// *runs* and fails inside winit rather than skipping with a reason — the
    /// one direction of wrongness this crate cannot tolerate.
    #[test]
    fn an_empty_variable_is_not_a_display() {
        assert!(!display_from_env(Some(""), Some("")).is_present());
        assert!(!display_from_env(Some(""), None).is_present());
        assert_eq!(
            display_from_env(Some(""), Some(":0")),
            Availability::present("DISPLAY=:0")
        );
    }

    // ----- adb ------------------------------------------------------------

    #[test]
    fn a_device_in_state_device_is_a_device() {
        let answer = android_from_listing("List of devices attached\nR5CT30ABCDE\tdevice\n\n");
        assert!(answer.is_present());
        assert!(answer.reason().contains("R5CT30ABCDE"), "{answer:?}");
    }

    #[test]
    fn an_empty_listing_is_no_device() {
        assert!(!android_from_listing("List of devices attached\n\n").is_present());
    }

    /// The state nobody guesses from a boolean. An unauthorised phone is
    /// plugged in, powered, and listed — and every command that matters fails
    /// until somebody taps "Allow" on its screen.
    #[test]
    fn an_unauthorised_device_is_absent_and_the_reason_says_which() {
        let answer = android_from_listing(
            "List of devices attached\nR5CT30ABCDE\tunauthorized\nemulator-5554\toffline\n",
        );
        assert!(!answer.is_present());
        assert!(answer.reason().contains("unauthorized"), "{answer:?}");
        assert!(answer.reason().contains("offline"), "{answer:?}");
    }

    #[test]
    fn one_ready_device_among_broken_ones_is_enough() {
        let answer = android_from_listing(
            "List of devices attached\nbad\tunauthorized\nR5CT30ABCDE\tdevice\n",
        );
        assert!(answer.is_present());
    }

    // ----- xctrace --------------------------------------------------------

    /// The listing that made this probe worth writing: the host Mac is in the
    /// `== Devices ==` section, so "the section is non-empty" would call every
    /// Mac an attached iPhone.
    #[test]
    fn the_host_mac_alone_is_not_an_ios_device() {
        let listing = "== Devices ==\n\
             Teja's MacBook Pro (00006000-001A2B3C4D5E6F00)\n\
             == Simulators ==\n\
             iPhone 15 Pro Simulator (17.4) (5B1B0A00-0000-4000-8000-000000000000)\n";
        let answer = ios_device_from_listing(listing);
        assert!(!answer.is_present(), "{answer:?}");
        assert!(answer.reason().contains("host Mac"), "{answer:?}");
    }

    #[test]
    fn a_phone_in_the_devices_section_is_an_ios_device() {
        let listing = "== Devices ==\n\
             Teja's MacBook Pro (00006000-001A2B3C4D5E6F00)\n\
             Teja's iPhone (18.0) (00008110-000A1B2C3D4E5F26)\n\
             == Simulators ==\n";
        let answer = ios_device_from_listing(listing);
        assert!(answer.is_present(), "{answer:?}");
        assert!(answer.reason().contains("iPhone"), "{answer:?}");
    }

    /// Simulators are listed in their own section, and a simulator is not the
    /// thing `NEXT.md` item 3 is asking about.
    #[test]
    fn a_simulator_is_not_counted_as_a_device() {
        let listing = "== Devices ==\n\
             Teja's MacBook Pro (00006000-001A2B3C4D5E6F00)\n\
             == Simulators ==\n\
             iPhone 15 (17.4) (5B1B0A00-0000-4000-8000-000000000000)\n\
             iPhone 15 Pro (17.4) (6C2C1B11-1111-4111-9111-111111111111)\n";
        assert!(!ios_device_from_listing(listing).is_present());
    }

    // ----- simctl ---------------------------------------------------------

    #[test]
    fn an_ios_runtime_with_a_simulator_is_a_simulator() {
        let listing = "== Devices ==\n\
             -- iOS 17.4 --\n    \
             iPhone 15 (5B1B0A00-0000-4000-8000-000000000000) (Shutdown)\n";
        let answer = ios_simulator_from_listing(listing);
        assert!(answer.is_present(), "{answer:?}");
        assert!(answer.reason().contains("none booted"), "{answer:?}");
    }

    #[test]
    fn a_booted_simulator_is_named_because_it_saves_a_boot() {
        let listing = "== Devices ==\n\
             -- iOS 17.4 --\n    \
             iPhone 15 (5B1B0A00-0000-4000-8000-000000000000) (Booted)\n";
        let answer = ios_simulator_from_listing(listing);
        assert!(answer.reason().contains("Booted"), "{answer:?}");
    }

    /// A Mac with Xcode has watchOS and tvOS simulators whether or not it has an
    /// iOS one, and neither runs this framework.
    #[test]
    fn other_platforms_runtimes_do_not_count() {
        let listing = "== Devices ==\n\
             -- tvOS 17.4 --\n    \
             Apple TV (1A1A1A1A-0000-4000-8000-000000000000) (Shutdown)\n\
             -- watchOS 10.4 --\n    \
             Apple Watch Series 9 (2B2B2B2B-0000-4000-8000-000000000000) (Shutdown)\n";
        assert!(!ios_simulator_from_listing(listing).is_present());
    }

    // ----- the override ---------------------------------------------------

    #[test]
    fn an_unset_variable_requires_nothing() {
        assert_eq!(requires("", Capability::Display), Ok(false));
    }

    #[test]
    fn a_named_capability_is_required_and_the_others_are_not() {
        assert_eq!(requires("display", Capability::Display), Ok(true));
        assert_eq!(requires("display", Capability::AndroidDevice), Ok(false));
    }

    #[test]
    fn a_list_is_read_as_a_list() {
        assert_eq!(
            requires("display,android-device", Capability::AndroidDevice),
            Ok(true)
        );
    }

    /// A CI file is written by hand, in a hurry, by somebody who has just
    /// learned that the variable exists. Whitespace, case and `_` are all
    /// things that file will contain.
    #[test]
    fn spelling_that_obviously_meant_it_is_accepted() {
        assert_eq!(
            requires(" DISPLAY , Android_Device ", Capability::AndroidDevice),
            Ok(true)
        );
        assert_eq!(requires("display,", Capability::Display), Ok(true));
    }

    #[test]
    fn all_requires_everything() {
        for capability in Capability::all() {
            assert_eq!(requires("all", *capability), Ok(true), "{capability:?}");
        }
    }

    /// The whole argument for erroring rather than ignoring: under a lenient
    /// parser this variable requires nothing, every gated test skips, and the
    /// run is green — which is precisely the state the variable was set to
    /// escape.
    #[test]
    fn a_misspelled_capability_is_an_error_rather_than_a_silent_nothing() {
        let error = requires("dispaly", Capability::Display).unwrap_err();
        assert!(error.contains("dispaly"), "{error}");
        assert!(error.contains("display"), "{error}");
    }

    #[test]
    fn a_known_name_beside_a_bad_one_does_not_rescue_it() {
        assert!(requires("display,nonsense", Capability::Display).is_err());
    }

    /// `gpu` is a name this crate knows in every build and can only *probe* in
    /// one. Told apart from a typo because the fix is different: add a feature,
    /// not correct a spelling.
    #[cfg(not(feature = "gpu"))]
    #[test]
    fn requiring_the_gpu_without_the_feature_says_which_feature() {
        let error = requires("gpu", Capability::Display).unwrap_err();
        assert!(error.contains("feature"), "{error}");
    }

    // ----- the probes, on whatever machine this is ------------------------

    /// Not an assertion about hardware — there is none to assert — but about the
    /// probes themselves: each answers, does not panic, and says something.
    ///
    /// It also pins the caching contract. Two calls return the same answer
    /// because the second is the first, which is what makes a gate in a hundred
    /// tests cost one `adb` invocation rather than a hundred.
    #[test]
    fn every_probe_answers_with_a_reason_and_answers_the_same_way_twice() {
        for capability in Capability::all() {
            let first = capability.probe();
            let second = capability.probe();
            assert_eq!(first, second, "{capability:?} changed its mind");
            assert!(
                !first.reason().is_empty(),
                "{capability:?} gave no reason for {first:?}"
            );
        }
    }

    #[test]
    fn the_summary_names_every_capability() {
        let summary = summary();
        for capability in Capability::all() {
            assert!(summary.contains(capability.name()), "{summary}");
        }
    }
}
