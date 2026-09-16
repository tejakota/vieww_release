//! Where the device is.

use std::rc::Rc;

use crate::permission::{Grant, Guarded, Permission};
use crate::service::ServiceError;
use crate::task::Task;

/// A point on the Earth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinates {
    /// Degrees north of the equator, −90 to 90.
    pub latitude: f64,
    /// Degrees east of Greenwich, −180 to 180.
    pub longitude: f64,
}

impl Coordinates {
    #[must_use]
    pub const fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    /// Whether both components are inside the ranges the type documents.
    ///
    /// Worth having because the commonest bug in this area is a latitude and a
    /// longitude the wrong way round, and a swapped pair is usually *still in
    /// range* — so this catches the garbage, not the classic. It is offered
    /// rather than enforced in a constructor for exactly that reason: a
    /// validating constructor here would suggest a guarantee it cannot give.
    #[must_use]
    pub fn is_plausible(self) -> bool {
        (-90.0..=90.0).contains(&self.latitude) && (-180.0..=180.0).contains(&self.longitude)
    }
}

/// A fix, with everything the platform knew about it.
///
/// # Why accuracy is not optional
///
/// Because a position without it cannot be used responsibly. Two hundred metres
/// of uncertainty is a city district and five metres is a doorway, and an
/// application that draws both as the same dot is lying to the user about where
/// they are. Every platform reports it; making it an `Option` here would invite
/// callers to ignore it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub coordinates: Coordinates,
    /// Radius of the 68% confidence circle, in metres.
    pub accuracy_metres: f32,
    /// Metres above the WGS-84 ellipsoid, where the platform knows.
    pub altitude_metres: Option<f32>,
    /// Metres per second over the ground, where the platform knows.
    pub speed_metres_per_second: Option<f32>,
    /// Degrees clockwise from true north, where the platform knows.
    pub heading_degrees: Option<f32>,
    /// Milliseconds since the Unix epoch, as the platform stamped it.
    ///
    /// The platform's stamp rather than the moment of delivery: a fix can be
    /// cached and handed over minutes later, and an application showing "you
    /// are here" needs to know which.
    pub timestamp_ms: i64,
}

/// How hard to work for a fix.
///
/// Battery is the whole reason this is a choice. Continuous high accuracy keeps
/// the GNSS radio on and is the single most expensive thing an application can
/// ask a phone for; a weather screen wants the city and should never cause
/// that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocationAccuracy {
    /// Whatever is nearly free — the network's idea of where you are.
    /// Kilometres. For a weather forecast or a currency guess.
    Coarse,
    /// A block or so, without keeping the satellite radio running.
    #[default]
    Balanced,
    /// Metres, with the radio on. For navigation, and for nothing else.
    Fine,
}

/// What kind of fix is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PositionRequest {
    pub accuracy: LocationAccuracy,
    /// Accept a fix already in hand if it is no older than this, in
    /// milliseconds.
    ///
    /// The cheapest request there is: a cached fix costs no radio at all. `0`
    /// insists on a fresh one.
    pub max_age_ms: u32,
    /// Give up after this many milliseconds.
    ///
    /// `None` means "however long it takes", which indoors can be never — a
    /// GNSS fix under a roof genuinely does not arrive, and a screen with no
    /// timeout spins until the user leaves the building.
    pub timeout_ms: Option<u32>,
}

/// Where the device is, once or repeatedly.
pub trait LocationServices: 'static {
    /// Whether the user has location switched on at all, device-wide.
    ///
    /// Separate from permission, and the distinction is the difference between
    /// two error messages that need different buttons: permission is settled
    /// inside the application, and this is settled in the system settings.
    /// Unguarded, because knowing that the switch is off is what lets an
    /// application explain instead of asking for a permission that cannot help.
    fn is_enabled(&self) -> bool;

    /// One fix.
    fn current(
        &self,
        request: &PositionRequest,
        grant: &Grant<'_, dyn LocationServices>,
    ) -> Task<Position, ServiceError>;

    /// Follow the device until the returned handle is dropped.
    ///
    /// `each` runs on the UI thread. Dropping the handle stops the updates, for
    /// the reason dropping a [`Task`] cancels its delivery: a subscription that
    /// had to be cancelled by a call is a subscription somebody forgets to
    /// cancel, and a forgotten one here holds the GNSS radio on.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where continuous updates do not exist.
    fn watch(
        &self,
        request: &PositionRequest,
        each: Rc<dyn Fn(Position)>,
        grant: &Grant<'_, dyn LocationServices>,
    ) -> Result<LocationWatch, ServiceError>;
}

/// A live location subscription. Dropping it stops the updates.
///
/// Opaque, and holds whatever the implementation needs to unsubscribe — a
/// registration id, a platform observer, a channel. The type is here rather
/// than in the platform crate so the *drop-to-cancel* contract is part of the
/// declaration and not a convention each implementation might miss.
pub struct LocationWatch {
    stop: Option<Box<dyn FnOnce()>>,
}

impl LocationWatch {
    /// Wrap the callback that ends this subscription.
    #[must_use]
    pub fn new(stop: impl FnOnce() + 'static) -> Self {
        Self {
            stop: Some(Box::new(stop)),
        }
    }

    /// A subscription that was never really started.
    #[must_use]
    pub const fn inert() -> Self {
        Self { stop: None }
    }
}

impl std::fmt::Debug for LocationWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocationWatch")
            .field("live", &self.stop.is_some())
            .finish()
    }
}

impl Drop for LocationWatch {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

impl Guarded for dyn LocationServices {
    const PERMISSION: Permission = Permission::new("location");
}

/// [`LocationServices`] on a device without them.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLocation;

impl LocationServices for NoLocation {
    fn is_enabled(&self) -> bool {
        false
    }

    fn current(
        &self,
        _request: &PositionRequest,
        _grant: &Grant<'_, dyn LocationServices>,
    ) -> Task<Position, ServiceError> {
        Task::failed(ServiceError::unsupported("LocationServices"))
    }

    fn watch(
        &self,
        _request: &PositionRequest,
        _each: Rc<dyn Fn(Position)>,
        _grant: &Grant<'_, dyn LocationServices>,
    ) -> Result<LocationWatch, ServiceError> {
        Err(ServiceError::unsupported("LocationServices"))
    }
}
