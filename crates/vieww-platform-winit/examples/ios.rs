//! The iOS entry point — a frame on an iPhone.
//!
//! ```console
//! # simulator: no signing, no device, boots and installs in seconds
//! ci/mobile/ios-app.sh --sim
//!
//! # a real iPhone, plugged in and trusted. Needs a signing identity.
//! VIEWW_IOS_IDENTITY="Apple Development: you@example.com (XXXXXXXXXX)" \
//! VIEWW_IOS_PROFILE=~/Library/MobileDevice/Provisioning\ Profiles/xxx.mobileprovision \
//!   ci/mobile/ios-app.sh --device
//! ```
//!
//! # Why this is a plain binary and Android's is a cdylib
//!
//! iOS *does* call `main`. `UIApplicationMain` is started from inside it by
//! winit, so the entry point is an ordinary Rust binary and there is no
//! equivalent of `App::run_android` — [`App::run`] is already correct here.
//! Android is the odd one out: its activity loads a shared library and calls
//! `android_main` inside it, so that target has to be a `cdylib`.
//!
//! What iOS needs instead is a **bundle**: a directory with an `Info.plist`
//! beside the executable, signed for the device it is going to. `ci/mobile/ios-app.sh`
//! builds one; the interesting parts of it are commented there rather than
//! here.
//!
//! # It runs on the desktop too, deliberately
//!
//! Nothing in this file is `cfg`-gated, so `cargo clippy --all-targets` on any
//! machine type-checks it and `cargo run --example ios` opens an ordinary
//! window. An entry point that only compiles on the platform nobody is sitting
//! at is one that rots between releases.
//!
//! ```console
//! # the demo screen, on whatever you are sitting at
//! cargo run -p vieww-platform-winit --example ios --release
//! ```
//!
//! That is currently the **only** way to look at the control gallery in
//! `shared/screen.rs` without a phone, which makes it worth more than its iOS
//! name suggests.

use vieww_foundation::{Size, TargetPlatform};
use vieww_platform_winit::App;

/// A phone-shaped window, for the desktop run.
///
/// Tall rather than square: `App`'s default 800x600 is shorter than the demo
/// column, on the one screen anybody can actually check it against.
///
/// **This is a request and not a size.** Asking for 880 on a 1080p display with
/// a panel and a title bar produced a 701-tall surface — the suite's
/// `view_metrics_reached_the_tree` line is where that showed up, and it is worth
/// reading rather than skimming for exactly this reason. So nothing in the tree
/// may depend on getting the height it asked for; see the frame graph's
/// `Flexible` in `shared/screen.rs`, which is what makes that true. On a real
/// device the OS decides the window and this is ignored entirely.
const PREVIEW: Size = Size {
    width: 420.0,
    height: 880.0,
};

#[path = "shared/screen.rs"]
mod screen;

#[path = "shared/device_tests.rs"]
mod device_tests;

use device_tests::DeviceSuite;

fn main() {
    // On iOS this never returns: winit hands control to `UIApplicationMain`,
    // which owns the process until the system tears it down. On a desktop it
    // returns when the window closes, and the report is worth printing.
    //
    // Identical to the Android entry point, deliberately: the same demo, the
    // same per-frame hooks, the same frame graph, the same device suite. What
    // differs between the two platforms is below this line and nothing above
    // it — Android needs the activity handle, iOS needs a bundle, and neither
    // needs a different tree. The suite reporting the same check names on both
    // is what makes the two runs comparable at all.
    let perf = screen::PerfLink::new();
    let suite = DeviceSuite::new();

    let result = App::new()
        .title("vieww")
        .size(PREVIEW)
        .on_frame(perf.hook())
        .on_frame(suite.hook())
        // Reports where the button and the field ended up, so the
        // device script taps what is on screen rather than a constant
        // that only suited one phone. `after_frame`, not `on_frame`:
        // geometry does not exist until the frame has been laid out.
        .after_frame(suite.tap_targets())
        // Raises the drawer and asks whether the screen behind it left the
        // semantics tree. **Installed here as well as on Android, and it is the
        // one hook in the suite that costs nothing to run anywhere**: it reads
        // vieww's own `SemanticsTree` and writes a signal, with no bridge, no
        // pasteboard and no platform call in it. The others are Android-only
        // because they need something the platform provides; this one is not.
        //
        // Which also means the `BlockSemantics` claim is checkable on a desktop
        // — this example is the desktop preview — rather than only on a phone.
        .before_frame(suite.modal_semantics())
        .run(|driver| {
            // `FrameDriver::set_root`, *not* `driver.elements().set_root` — see
            // the Android example for why the difference is load-bearing.
            let demo = screen::Demo::new(driver);
            perf.attach(&demo);
            let probe = suite.probe(demo.taps());
            suite.watch_drawer(demo.drawer());
            driver.set_root(demo.screen("A frame, on an iPhone.", TargetPlatform::IOS, probe));
        });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("vieww failed: {error}"),
    }
}
