//! The desktop entry point — the same demo, on Linux, macOS and Windows.
//!
//! ```console
//! ci/certify/desktop-suite.sh                 # build, run, verify
//! cargo run -p vieww-platform-winit --example desktop --release
//! ```
//!
//! # Why this exists when `examples/ios.rs` already runs on a desktop
//!
//! It runs there as a *preview*: an entry point for a platform nobody is
//! sitting at, kept compiling by being runnable on the machine they are. What
//! it does not do is ask the desktop anything. Every check it installs is one
//! `device_tests.rs` wrote for a phone, and the questions a workstation can
//! answer and a phone cannot — a second window, the shortcut modifier, the
//! pasteboard, the conventional data directory — were asked by nothing at all.
//!
//! That was the shape of the gap: `ci/mobile/device-suite.sh` could say "this Android
//! phone is behaving", and `ci/certify/release-check.sh` could say "the studio's
//! controls are all reachable and here are the pictures", and between them
//! nobody could say whether the *platform layer* works on a Mac. This is the
//! third leg.
//!
//! # The same tree, deliberately
//!
//! `shared/screen.rs` and `shared/device_tests.rs`, unchanged, exactly as the
//! Android and iOS entry points take them — so a report from this run and a
//! report from a phone carry the same check names against the same widgets, and
//! the two are comparable line for line. `shared/desktop_tests.rs` is the only
//! thing added, and it holds only what a phone would have to answer
//! `Unsupported` to.
//!
//! # What a passing run means
//!
//! That the platform layer behaved on **this** machine, in **this** session. It
//! is not a claim about the GPU (every pixel here is the CPU rasterizer's; see
//! `PENDING.md` §2.6), and it is not a claim about a second monitor, a HiDPI
//! display or a keyboard layout this machine does not have — those are what
//! `ci/certify/desktop-suite.sh`'s flags are for, because only the operator knows them.

use vieww_foundation::{Size, TargetPlatform};
use vieww_platform_winit::App;

/// A window shaped like the demo column, for the reason `examples/ios.rs` gives
/// at length: this is a **request**, the window manager decides, and nothing in
/// the tree may depend on getting it.
const WINDOW: Size = Size {
    width: 420.0,
    height: 880.0,
};

#[path = "shared/screen.rs"]
mod screen;

#[path = "shared/device_tests.rs"]
mod device_tests;

#[path = "shared/desktop_tests.rs"]
mod desktop_tests;

use desktop_tests::DesktopSuite;
use device_tests::DeviceSuite;

fn main() {
    let perf = screen::PerfLink::new();
    let device = DeviceSuite::new();
    let desktop = DesktopSuite::new(&device);

    // Before the loop: none of it needs a frame, and `data_dir` and the
    // shortcut modifier are answers a broken build gets wrong before it draws
    // anything.
    desktop.environment();

    let app = App::new().title("vieww — desktop suite").size(WINDOW);
    // Taken before `run`, per `PlatformWindows`' own docs: the handle has to
    // exist before there is a loop to back it.
    let windows = app.windows();

    let result = app
        .on_frame(perf.hook())
        .on_frame(device.hook())
        .after_frame(device.tap_targets())
        .after_frame(desktop.surface(WINDOW))
        .before_frame(device.modal_semantics())
        .before_frame(desktop.clipboard())
        .before_frame(desktop.windows(windows))
        .run(|driver| {
            let demo = screen::Demo::new(driver);
            perf.attach(&demo);
            let probe = device.probe(demo.taps());
            device.watch_drawer(demo.drawer());
            driver.set_root(demo.screen(
                "A frame, on a desktop.",
                TargetPlatform::current(),
                probe,
            ));
        });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("vieww failed: {error}"),
    }
}
