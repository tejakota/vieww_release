//! Hot reload on a phone: an activity that loads its widgets from a library
//! pushed over `adb`.
//!
//! ```console
//! # once, so there is something to load
//! ci/mobile/reload-guest-android.sh --package dev.vieww.reload --no-run
//!
//! # then the activity
//! cargo apk run -p reload-android --release
//!
//! # and on every edit to examples/reload-guest, in another terminal
//! ci/mobile/reload-guest-android.sh --package dev.vieww.reload --no-run
//! ```
//!
//! The window follows the push. Nothing has to be tapped, and the app is not
//! reinstalled — the activity keeps running and swaps the tree between frames,
//! exactly as `examples/reload-host` does on a desktop.
//!
//! # Why this is a separate application from `examples/android.rs`
//!
//! Because it must be. This crate depends on `vieww-reload`, which enables
//! `vieww-widget/hot-reload`, which adds a field to `WidgetNode` and switches
//! reconciliation to type paths. The demo is deliberately built *without* that,
//! because it is what the device suite reports on and a suite that measures a
//! different `WidgetNode` from the one that ships is measuring nothing. Two
//! crates, two package ids, no shared build.
//!
//! # The one thing Android needs that a desktop does not
//!
//! **Two directories instead of one.** On a desktop the guest is watched in
//! `target/`, which is writable and executable, so the rebuilt library is
//! copied beside itself and loaded. Here the library arrives over `adb`, which
//! can only write the app's external directory, and that is a FUSE mount which
//! is generally `noexec`; the load has to happen from `internal_data_path()`,
//! which `adb` cannot write. So this host watches one directory and stages into
//! another — `Reloader::staged_in` — and that is the whole platform
//! difference. See `docs/HANDOFF.md` for the measurement that established the
//! internal directory is loadable at all.

#![cfg(target_os = "android")]

use std::path::Path;
use std::time::Duration;

use vieww_foundation::task::FrameWaker;
use vieww_foundation::Color;
use vieww_platform_winit::{AndroidApp, App};
use vieww_reload::{Change, Reloaded, Reloader, Watch, WidgetNode};
use vieww_widget::prelude::*;

/// What `ci/mobile/reload-guest-android.sh` pushes. Must match the script.
const GUEST: &str = "libreload_guest.so";

/// Where staged copies go, under the app's private directory.
///
/// A subdirectory rather than the files directory itself, so the accumulating
/// numbered copies — one per reload, leaked on purpose because their vtables
/// are still referenced — stay separable from anything else the app keeps.
const STAGING: &str = "reload";

/// Called by `android-activity`'s glue once the activity exists.
///
/// Plain Rust ABI rather than `extern "C"`, for the reason `examples/android.rs`
/// gives: the glue looks the symbol up by name and calls it as Rust, and
/// `AndroidApp` is not `repr(C)`.
#[no_mangle]
fn android_main(android: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("vieww-reload"),
    );

    if let Err(error) = run(android) {
        // A phone discards stderr, and this is the only way the reason for an
        // activity that opened and vanished reaches anybody.
        log::error!("vieww-reload: {error}");
    }
}

fn run(android: AndroidApp) -> Result<(), String> {
    // Where `adb push` can reach. Watched, never loaded from.
    let external = android
        .external_data_path()
        .ok_or("this device reports no external data path, so nothing can be pushed to it")?
        .join(GUEST);

    // Where `dlopen` is permitted. Loaded from, never pushed to.
    let staging = android
        .internal_data_path()
        .ok_or("this device reports no internal data path, so nothing can be loaded")?
        .join(STAGING);

    log::info!(
        "vieww-reload: watching {}, staging into {}",
        external.display(),
        staging.display()
    );

    // **`None` is a normal starting state, not a failure.** Installing the
    // activity and pushing a guest cannot both be first: `adb` stages into the
    // app's external directory, which exists once the app is installed. So the
    // host opens a window either way and waits, which is what a reload host
    // should do regardless — a developer starts it once and pushes all day.
    let mut reloader = match Reloader::staged_in(&external, &staging) {
        Ok(loaded) => {
            log::info!("vieww-reload: loaded a guest at start-up");
            Some(loaded)
        }
        Err(error) => {
            log::info!("vieww-reload: waiting for a guest — {error}");
            None
        }
    };

    let first = reloader
        .as_ref()
        .map_or_else(|| waiting_screen(&external), Reloader::root);

    let app = App::new()
        .title("vieww — hot reload")
        .background(Color::rgb(18, 18, 22));

    // Without this the swap would wait for a frame drawn for some other reason
    // — a touch, a rotation — because it rides `before_frame` and an idle
    // activity draws nothing. On a phone there is no mouse to jiggle, so this
    // matters more here than on a desktop rather than less.
    //
    // Its own `Watch` rather than `Reloader::wake_on_change`, because there may
    // be no `Reloader` yet and the frame this asks for is exactly the one that
    // would create it.
    let waker = app.waker();
    let mut watch = Watch::new(&external);
    std::thread::Builder::new()
        .name("vieww-reload-watch".to_owned())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            if watch.poll() == Change::Rebuilt {
                waker.wake();
            }
        })
        .map_err(|error| format!("could not start the watcher thread: {error}"))?;

    let staging_for_poll = staging.clone();
    let external_for_poll = external.clone();

    let report = app
        .before_frame(move |driver| {
            // Still waiting for the first guest. Tested with `is_none` rather
            // than matched on `as_mut`, because the arm that succeeds has to
            // *assign* to this same `Option` and a match would still be holding
            // it borrowed.
            if reloader.is_none() {
                // Retried on every frame the watcher asked for, and silent until
                // it works: a log line per attempt would bury the one that
                // matters under a poll loop.
                if let Ok(loaded) = Reloader::staged_in(&external_for_poll, &staging_for_poll) {
                    match loaded.root() {
                        Ok(root) => {
                            log::info!("vieww-reload: the first guest arrived");
                            // `remount`, not `set_root`: what is on screen is
                            // the placeholder, which shares no element with the
                            // guest's tree.
                            driver.remount(root);
                            reloader = Some(loaded);
                        }
                        // A guest that panics while building is kept *out*: the
                        // placeholder stays, the next push is tried, and the
                        // device does not lose the process over an `unwrap` in a
                        // build the developer is already fixing.
                        Err(error) => log::warn!("vieww-reload: {error}"),
                    }
                }
                return;
            }

            let Some(active) = reloader.as_mut() else {
                return;
            };

            match active.poll() {
                Reloaded::Nothing => {}
                Reloaded::Root(root) => {
                    log::info!(
                        "vieww-reload: reloaded #{} — state kept",
                        active.generation()
                    );
                    driver.set_root(root);
                }
                Reloaded::Restart(root) => {
                    // The guest's state changed shape. Reusing the old
                    // allocations would be undefined behaviour, so the tree goes.
                    log::info!(
                        "vieww-reload: reloaded #{} — state CHANGED SHAPE, tree rebuilt",
                        active.generation()
                    );
                    driver.remount(root);
                }
                Reloaded::Failed(error) => {
                    // A compile error in the guest, or a push caught halfway.
                    // The activity carries on running whatever was loaded last,
                    // which is what a developer wants mid-edit.
                    log::warn!("vieww-reload: keeping the running build — {error}");
                }
            }
        })
        .run_android(android, move |driver| driver.set_root(first))
        .map_err(|error| format!("{error}"))?;

    log::info!("vieww-reload: {report}");
    Ok(())
}

/// What is on screen before any guest has been pushed.
///
/// On a desktop this case is a line on stderr and the developer reads it. A
/// phone has no terminal attached by default, so an activity that opened and
/// showed nothing would be indistinguishable from one that crashed — and the
/// first thing anyone would do is reinstall it, which is not the fix.
///
/// It names the exact command, including the package, because the two-app split
/// is the part of this setup most likely to be misremembered.
fn waiting_screen(watching: &Path) -> WidgetNode {
    const INK: Color = Color::rgb(226, 232, 240);
    const DIM: Color = Color::rgb(148, 163, 184);

    Container::new()
        .padding(EdgeInsets::all(32.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(16.0)
                .children(children![
                    Text::new("waiting for a guest").color(INK).size(22.0),
                    Text::new("ci/mobile/reload-guest-android.sh --package dev.vieww.reload")
                        .color(INK)
                        .size(13.0),
                    Text::new(format!("watching {}", watching.display()))
                        .color(DIM)
                        .size(11.0),
                ]),
        )
        .into()
}
