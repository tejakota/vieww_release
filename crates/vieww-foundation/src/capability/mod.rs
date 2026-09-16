//! The device capabilities a real application needs, declared.
//!
//! [`service`](crate::service) is the seam and [`permission`](crate::permission)
//! is the gate on it. Both were built before there was anything to plug into
//! them, which was the right order and left an obvious question unanswered:
//! *the seam is empty, so what does a camera actually look like here?*
//!
//! This module answers it for the six capabilities that come up first — the
//! camera, the user's location, notifications, biometric authentication, the
//! motion sensors, and embedding a native view inside the widget tree. Each one
//! is a trait, an `impl Guarded` where the OS gates it, and an implementation
//! that reports honestly that it is not there.
//!
//! # What this module is and is not
//!
//! **It is a set of declarations, not implementations.** Nothing here talks to
//! Android or iOS; a platform crate does that, and it does it by registering an
//! implementation of these traits into [`Services`](crate::service::Services),
//! with no change to this crate. That is [`service`](crate::service)'s rule,
//! and this module is the first proof that the rule holds for something more
//! complicated than a key-value store.
//!
//! So why declare them here at all, if a third party can define its own traits?
//! Because the *shape* is the interoperable part. If every camera package
//! invents its own `Photo`, an application cannot swap one for another and a
//! widget cannot be written against "a camera" — precisely the situation a
//! package free-for-all creates, where every camera package ships its own
//! incompatible answer to one question. One first-party trait per capability
//! is the thing a framework can offer that a package cannot.
//!
//! # The unavailable story, said out loud
//!
//! Every capability here ships with an implementation that does nothing and
//! says so: [`NoCamera`], [`NoLocation`], [`NoNotifications`],
//! [`NoBiometrics`], [`NoSensors`], [`NoPlatformViews`]. They return
//! [`ServiceError::Unsupported`](crate::service::ServiceError::Unsupported) —
//! **not** a panic, not a hang, and not a plausible fake.
//!
//! That choice is deliberate and it is the opposite of the one made for
//! [`MemoryStorage`](crate::service::MemoryStorage). A storage stub that keeps
//! values in a map is *correct for the life of the process*, so pretending is
//! the kinder failure. A camera cannot be faked into correctness — a fake photo
//! is worse than no photo, because the application ships believing it works.
//! So the storage-shaped capabilities get real fallbacks and the
//! device-shaped ones get honest refusals, and
//! [`is_permanent`](crate::service::ServiceError::is_permanent) is how a screen
//! tells the difference between "hide the button" and "try again".
//!
//! An application that registers nothing gets `None` from
//! [`Services::get`](crate::service::Services::get) — also honest, and the
//! reason these stubs are opt-in rather than defaults. Registering `NoCamera`
//! is a statement: *this build has a camera-shaped hole in it on purpose.*
//!
//! # Permission is not optional here
//!
//! Five of the six are [`Guarded`](crate::permission::Guarded), so their
//! methods take a [`Grant`](crate::permission::Grant) and cannot be called
//! without one. Platform views are the exception: embedding a map view needs no
//! permission of its own, and whatever it displays asks for its own.
//!
//! ```
//! use std::rc::Rc;
//! use vieww_foundation::capability::{Camera, NoCamera};
//! use vieww_foundation::permission::{AlwaysGranted, Permissions};
//! use vieww_foundation::service::Services;
//!
//! let mut services = Services::new();
//! services.provide::<dyn Camera>(Rc::new(NoCamera));
//! services.provide::<dyn Permissions>(Rc::new(AlwaysGranted));
//!
//! let gate = services.gate::<dyn Camera>().expect("both halves registered");
//! let (camera, grant) = gate.granted().expect("AlwaysGranted grants");
//!
//! // Permission said yes; the device still is not there, and says so.
//! let refusal = camera.capture(&Default::default(), &grant);
//! assert!(refusal.value().failed().is_some_and(|error| error.is_permanent()));
//! ```

mod biometrics;
mod camera;
mod location;
mod notifications;
mod platform_view;
mod sensors;

pub use biometrics::{BiometricKind, BiometricPrompt, BiometricStrength, Biometrics, NoBiometrics};
pub use camera::{Camera, CameraDevice, CameraFacing, FlashMode, NoCamera, Photo, PhotoRequest};
pub use location::{
    Coordinates, LocationAccuracy, LocationServices, LocationWatch, NoLocation, Position,
    PositionRequest,
};
pub use notifications::{
    NoNotifications, Notification, NotificationChannel, Notifications, PushRegistration, PushToken,
    ScheduledAt,
};
pub use platform_view::{
    NoPlatformViews, PlatformViewHandle, PlatformViewId, PlatformViewSpec, PlatformViews,
};
pub use sensors::{
    MotionReading, NoSensors, Sensor, SensorRate, SensorSubscription, Sensors, Vector3,
};

#[cfg(test)]
mod tests {
    //! What the module promises, checked: that a capability is reachable
    //! through the seam, that it cannot be called without a grant, and that an
    //! absent device refuses honestly rather than pretending.

    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::permission::{AlwaysDenied, AlwaysGranted, Permissions};
    use crate::service::{ServiceError, Services};
    use crate::Rect;

    fn granted() -> Services {
        let mut services = Services::new();
        services.provide::<dyn Permissions>(Rc::new(AlwaysGranted));
        services
    }

    #[test]
    fn every_declared_capability_goes_through_the_ordinary_seam() {
        // Nothing in `Services` knows these types exist; if this compiles and
        // passes, a third party's own capability is registered the same way.
        let mut services = granted();
        services.provide::<dyn Camera>(Rc::new(NoCamera));
        services.provide::<dyn LocationServices>(Rc::new(NoLocation));
        services.provide::<dyn Notifications>(Rc::new(NoNotifications));
        services.provide::<dyn Biometrics>(Rc::new(NoBiometrics));
        services.provide::<dyn Sensors>(Rc::new(NoSensors));
        services.provide::<dyn PlatformViews>(Rc::new(NoPlatformViews));

        assert!(services.get::<dyn Camera>().is_some());
        assert!(services.get::<dyn LocationServices>().is_some());
        assert!(services.get::<dyn Notifications>().is_some());
        assert!(services.get::<dyn Biometrics>().is_some());
        assert!(services.get::<dyn Sensors>().is_some());
        assert!(services.get::<dyn PlatformViews>().is_some());
    }

    #[test]
    fn an_absent_device_refuses_permanently_rather_than_failing_transiently() {
        // The distinction a screen acts on: hide the button, or offer a retry.
        let mut services = granted();
        services.provide::<dyn Camera>(Rc::new(NoCamera));
        let gate = services.gate::<dyn Camera>().expect("registered");
        let (camera, grant) = gate.granted().expect("AlwaysGranted grants");

        let task = camera.capture(&PhotoRequest::default(), &grant);
        let error = task.value().failed().expect("an absent camera fails");
        assert!(error.is_permanent(), "{error}");
        assert!(matches!(error, ServiceError::Unsupported { .. }), "{error}");
    }

    #[test]
    fn a_refused_permission_never_reaches_the_capability_at_all() {
        // Not "returns an error" — the call has no syntax. `granted()` is the
        // only way to a `Grant`, and it hands back `None`.
        let mut services = Services::new();
        services.provide::<dyn Permissions>(Rc::new(AlwaysDenied));
        services.provide::<dyn Camera>(Rc::new(NoCamera));

        let gate = services.gate::<dyn Camera>().expect("registered");
        assert!(gate.granted().is_none(), "denied means no grant exists");
    }

    #[test]
    fn asking_what_hardware_exists_needs_no_permission() {
        // The rule that stops every application prompting on launch: the
        // enumeration methods are on the trait unguarded, so a screen can
        // decide whether to offer a feature before it asks for anything.
        assert!(NoCamera
            .devices()
            .expect("an empty list is an answer")
            .is_empty());
        assert!(NoBiometrics.available().is_empty());
        assert!(NoSensors.available().is_empty());
        assert!(NoPlatformViews.registered().is_empty());
        assert!(!NoLocation.is_enabled());
    }

    #[test]
    fn an_empty_device_list_is_not_an_error() {
        // "This phone has no camera" is a true answer, and treating it as a
        // failure would make a tablet look broken.
        let devices = NoCamera.devices();
        assert!(devices.is_ok(), "{devices:?}");
    }

    #[test]
    fn tidying_up_against_a_missing_service_does_not_fail() {
        // Cancelling a notification that cannot exist has already achieved what
        // it was for; a logout path should not have to handle an error for it.
        assert!(NoNotifications.cancel("anything").is_ok());
        assert!(NoNotifications.configure(&[]).is_ok());
        assert!(NoNotifications.take_opened().is_none());
    }

    #[test]
    fn a_subscription_ends_when_its_handle_is_dropped() {
        // The whole contract of the watch handles: no cancel call to forget,
        // because dropping is the cancel.
        let stopped = Rc::new(Cell::new(false));
        let flag = Rc::clone(&stopped);
        let watch = LocationWatch::new(move || flag.set(true));

        assert!(!stopped.get(), "still running while the handle is held");
        drop(watch);
        assert!(stopped.get(), "and stopped the moment it is not");
    }

    #[test]
    fn a_sensor_subscription_ends_the_same_way() {
        let stopped = Rc::new(Cell::new(false));
        let flag = Rc::clone(&stopped);
        drop(SensorSubscription::new(move || flag.set(true)));
        assert!(stopped.get());
    }

    #[test]
    fn a_platform_view_is_destroyed_when_its_handle_goes() {
        // The most expensive leak in the module — a live browser or map engine
        // — so it is the one with the least room for a forgotten dispose call.
        let disposed = Rc::new(Cell::new(None));
        let seen = Rc::clone(&disposed);
        let handle = PlatformViewHandle::new(PlatformViewId(7), move |id| seen.set(Some(id)));

        assert_eq!(handle.id(), PlatformViewId(7));
        drop(handle);
        assert_eq!(disposed.get(), Some(PlatformViewId(7)));
    }

    #[test]
    fn only_the_health_sensor_is_permissioned() {
        // Asking for activity permission in order to read an accelerometer is
        // asking for something you do not need, and users notice.
        assert!(Sensor::StepCounter.needs_permission());
        for sensor in [
            Sensor::Accelerometer,
            Sensor::LinearAcceleration,
            Sensor::Gyroscope,
            Sensor::Magnetometer,
            Sensor::Orientation,
            Sensor::Proximity,
            Sensor::Light,
        ] {
            assert!(!sensor.needs_permission(), "{sensor:?}");
        }
    }

    #[test]
    fn a_coordinate_pair_outside_the_earth_is_caught() {
        assert!(Coordinates::new(51.5, -0.12).is_plausible());
        assert!(!Coordinates::new(151.5, -0.12).is_plausible());
        // The classic swap survives, which is why the doc says so: both halves
        // of a swapped London are still in range.
        assert!(Coordinates::new(-0.12, 51.5).is_plausible());
    }

    #[test]
    fn a_platform_view_spec_carries_parameters_across_the_language_boundary() {
        let spec = PlatformViewSpec::new("map")
            .with("lat", "51.5")
            .with("lon", "-0.12");
        assert_eq!(spec.kind, "map");
        assert_eq!(spec.parameters.get("lat").map(String::as_str), Some("51.5"));

        let refused = NoPlatformViews.create(&spec, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(refused.is_err());
    }

    #[test]
    fn a_notification_needs_only_a_title_and_a_body() {
        let notification = Notification::now("order-1", "On its way", "Arriving Tuesday");
        assert_eq!(notification.scheduled, ScheduledAt::Now);
        assert!(notification.channel.is_none());
    }

    #[test]
    fn a_vector_reports_its_magnitude() {
        assert!((Vector3::new(3.0, 4.0, 0.0).magnitude() - 5.0).abs() < 1e-6);
    }
}
