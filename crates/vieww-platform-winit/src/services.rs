//! The platform's own capabilities, assembled.
//!
//! # Why this is a free function and not a method on [`App`](crate::App)
//!
//! An application needs its services *before* it runs, because they go into the
//! tree — `Inherited<SharedServices>` above the root, the same way `Theme` and
//! `ViewMetrics` get down there. `App::run` consumes the `App`, so a builder
//! method would hand the registry back only after it was too late to use.
//!
//! So the registry is built on its own, added to, and published above the root:
//!
//! ```no_run
//! # use std::rc::Rc;
//! # use vieww_foundation::{SharedServices, Storage};
//! # use vieww_platform_winit::{services, App};
//! # use vieww_widget::Inherited;
//! # fn root() -> vieww_widget::WidgetNode { unimplemented!() }
//! let mut registry = services::platform();
//! // Anything else the application brings, including services vieww has never
//! // heard of — see `vieww_foundation::service`.
//! let services = SharedServices::new(registry);
//!
//! App::new().run(move |driver| {
//!     // `set_root`, not `driver.elements().set_root` — the second compiles,
//!     // looks identical, and silently costs the app its view metrics.
//!     driver.set_root(Inherited::new(services, root()));
//! })?;
//! # Ok::<(), vieww_platform_winit::PlatformError>(())
//! ```
//!
//! # What is registered
//!
//! | capability | desktop / iOS | Android |
//! |---|---|---|
//! | [`Storage`] | a file in [`data_dir`](crate::storage::data_dir) | a file in the activity's private directory |
//! | [`DeepLinks`] | launch link from the command line | launch link from the activity's intent |
//! | [`Clipboard`] | the system pasteboard | the activity's clipboard manager |
//! | [`AssetBundle`] | a directory beside the executable | the APK's asset manager |
//! | tray, global hotkeys | with `desktop-services` | — |
//!
//! # What is declared and not implemented, said plainly
//!
//! The device capabilities — camera, location, notifications and push,
//! biometrics, sensors, platform views — are **declared** in
//! [`vieww_foundation::capability`] and **not implemented here**. Each is a
//! trait with a documented shape, a permission, and an honest `No…` fallback;
//! none of them has a JNI or Objective-C bridge behind it in this repository.
//!
//! That is a real gap and it is stated rather than implied. What the
//! declarations buy is that the bridge, when it is written, plugs into a shape
//! that already exists — application code, widgets and tests can be written
//! against `dyn Camera` today, and the platform crate that implements it needs
//! no change to `vieww-foundation` and none to this module either.
//!
//! [`unavailable_capabilities`] registers the honest refusals for a build that
//! wants the calls to compile and fail cleanly. It is opt-in: leaving them
//! unregistered means [`Services::get`] answers `None`, which is a different
//! and equally true statement — *nothing claims to provide this*.

use std::rc::Rc;

use vieww_asset::{AssetBundle, DirectoryBundle};
use vieww_foundation::capability::{
    Biometrics, Camera, LocationServices, NoBiometrics, NoCamera, NoLocation, NoNotifications,
    NoPlatformViews, NoSensors, Notifications, PlatformViews, Sensors,
};
use vieww_foundation::{Clipboard, DeepLinks, Services, Storage};

use crate::clipboard::PlatformClipboard;

use crate::deep_links::PlatformDeepLinks;
use crate::storage::FileStorage;

/// Where an application's files sit next to its executable.
///
/// `assets/` beside the binary, and — because `cargo run` puts the binary in
/// `target/debug` and the assets are in the project root — the working
/// directory as a fallback. Development convenience, stated as such: on iOS the
/// executable's directory *is* the bundle, so the first branch is also the
/// shipping answer there.
#[must_use]
fn executable_assets() -> DirectoryBundle {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("assets")))
        .filter(|dir| dir.is_dir());

    DirectoryBundle::at(beside.unwrap_or_else(|| std::path::PathBuf::from("assets")))
}

/// The capabilities available on this platform, off a desktop or iOS.
///
/// **Not for Android** — storage there needs the activity's private directory
/// and the launch link needs its intent. Use `for_android`.
#[must_use]
pub fn platform() -> Services {
    let mut services = Services::new();
    services.provide::<dyn Storage>(Rc::new(FileStorage::platform()));
    services.provide::<dyn DeepLinks>(Rc::new(PlatformDeepLinks::from_command_line()));
    services.provide::<dyn AssetBundle>(Rc::new(executable_assets()));
    // Not on Android: the pasteboard there needs the activity, so it is
    // registered by `for_android` instead. This function is still compiled for
    // that target — it is the desktop and iOS entry point — so the exclusion has
    // to be here rather than assumed.
    #[cfg(not(target_os = "android"))]
    services.provide::<dyn Clipboard>(Rc::new(PlatformClipboard::new()));
    // Tray and global hotkeys, when this build asked for them. Whichever of the
    // two this session cannot provide is left unregistered rather than
    // registered as something that always fails — see [`crate::desktop`].
    #[cfg(all(
        feature = "desktop-services",
        not(any(target_os = "android", target_os = "ios"))
    ))]
    crate::desktop::provide(&mut services);
    services
}

/// An Android application's own files, out of the APK.
///
/// Assets are compressed inside the package, so there is no path to them and
/// `std::fs` cannot help: the asset manager is the only way in. Holding the
/// `AndroidApp` rather than the manager keeps `ndk`'s types out of this crate's
/// public API, which matters because it is a transitive dependency whose version
/// is chosen by `winit`.
#[cfg(target_os = "android")]
#[derive(Debug, Clone)]
pub struct AndroidAssets {
    android: crate::AndroidApp,
}

#[cfg(target_os = "android")]
impl AssetBundle for AndroidAssets {
    fn open(&self, path: &str) -> Result<Vec<u8>, vieww_asset::AssetError> {
        use std::io::Read;
        use vieww_asset::AssetError;

        let name = std::ffi::CString::new(path)
            .map_err(|_| AssetError::Unreadable(format!("{path} contains a NUL byte")))?;

        let mut asset = self
            .android
            .asset_manager()
            .open(&name)
            .ok_or_else(|| AssetError::NotFound(path.to_owned()))?;

        let mut bytes = Vec::new();
        asset
            .read_to_end(&mut bytes)
            .map_err(|error| AssetError::Unreadable(format!("{path}: {error}")))?;
        Ok(bytes)
    }
}

/// The capabilities available inside an Android activity.
///
/// Storage falls back to [`MemoryStorage`](vieww_foundation::MemoryStorage) when
/// the activity has no private directory yet — which happens if this is called
/// before the activity is attached. A working store that forgets at exit beats a
/// missing service that makes every caller handle an error it cannot fix.
#[cfg(target_os = "android")]
#[must_use]
pub fn for_android(android: &crate::AndroidApp) -> Services {
    use vieww_foundation::MemoryStorage;

    let mut services = Services::new();
    match FileStorage::for_android(android) {
        Some(storage) => services.provide::<dyn Storage>(Rc::new(storage)),
        None => services.provide::<dyn Storage>(Rc::new(MemoryStorage::new())),
    }
    services.provide::<dyn DeepLinks>(Rc::new(PlatformDeepLinks::from_android(android)));
    services.provide::<dyn AssetBundle>(Rc::new(AndroidAssets {
        android: android.clone(),
    }));
    services.provide::<dyn Clipboard>(Rc::new(PlatformClipboard::new(android)));
    services
}

/// Register an honest refusal for every device capability this repository
/// declares but does not implement.
///
/// [`NoCamera`], [`NoLocation`], [`NoNotifications`], [`NoBiometrics`],
/// [`NoSensors`] and [`NoPlatformViews`]. Every call through them returns
/// [`ServiceError::Unsupported`](vieww_foundation::ServiceError::Unsupported),
/// which [`is_permanent`](vieww_foundation::ServiceError::is_permanent) reports
/// as permanent — so a screen can hide the button rather than offer a retry
/// that cannot work.
///
/// # Why this is not part of [`platform`]
///
/// Because "registered, and it refuses" and "not registered" are different
/// claims and an application should get to choose which one it is making. A
/// build with no camera bridge that registers nothing has
/// [`Services::get`](vieww_foundation::Services::get) answer `None`, and code
/// that checks for the service simply does not offer the feature. A build that
/// calls this is saying something stronger: *the call sites exist, they compile,
/// and they fail cleanly on this target* — which is what an application wants
/// while a bridge is being written for one platform and not yet the other.
///
/// # The guarded five need a `Permissions` too
///
/// Five of the six are [`Guarded`](vieww_foundation::permission::Guarded), so
/// reaching them at all goes through
/// [`Services::gate`](vieww_foundation::Services::gate), which needs a
/// `dyn Permissions` registered as well. This function does not register one:
/// choosing what an unanswerable permission prompt should say is the
/// application's decision, and
/// [`AlwaysDenied`](vieww_foundation::permission::AlwaysDenied) is the honest
/// answer for a build with no bridge behind any of them.
pub fn unavailable_capabilities(services: &mut Services) {
    services.provide::<dyn Camera>(Rc::new(NoCamera));
    services.provide::<dyn LocationServices>(Rc::new(NoLocation));
    services.provide::<dyn Notifications>(Rc::new(NoNotifications));
    services.provide::<dyn Biometrics>(Rc::new(NoBiometrics));
    services.provide::<dyn Sensors>(Rc::new(NoSensors));
    services.provide::<dyn PlatformViews>(Rc::new(NoPlatformViews));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_platform_registers_storage_deep_links_and_assets() {
        let services = platform();
        assert!(services.has::<dyn Storage>());
        assert!(services.has::<dyn DeepLinks>());
        assert!(services.has::<dyn AssetBundle>());
        assert!(services.has::<dyn Clipboard>());
    }

    #[test]
    fn an_application_can_add_a_service_this_crate_has_never_heard_of() {
        // The extensibility claim, tested at the layer that would break it: the
        // platform crate builds a registry and hands it over *open*.
        trait Torch: 'static {}
        struct Off;
        impl Torch for Off {}

        let mut services = platform();
        services.provide::<dyn Torch>(Rc::new(Off));
        assert!(services.get::<dyn Torch>().is_some());
        assert!(
            services.has::<dyn Storage>(),
            "and adding one does not disturb the built-ins"
        );
    }
}
