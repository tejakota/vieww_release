//! The motion and environment sensors.

use std::rc::Rc;

use crate::permission::{Grant, Guarded, Permission};
use crate::service::ServiceError;

/// Three components, in the device's own frame.
///
/// Right-handed, with `x` across the screen to the right, `y` up it, and `z`
/// out of it towards the user — the convention both platforms use once their
/// axes are reconciled, and worth writing down because it is the thing that is
/// silently different when a reading looks upside down.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vector3 {
    #[must_use]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Length. For "how hard was that shake".
    #[must_use]
    pub fn magnitude(self) -> f32 {
        self.z
            .mul_add(self.z, self.x.mul_add(self.x, self.y * self.y))
            .sqrt()
    }
}

/// Which sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sensor {
    /// Metres per second squared, **including gravity** — so a phone lying
    /// still reads about 9.8 on one axis rather than zero. Said here because
    /// assuming otherwise is the classic first bug.
    Accelerometer,
    /// Metres per second squared with gravity removed, where the platform
    /// fuses it out.
    LinearAcceleration,
    /// Radians per second about each axis.
    Gyroscope,
    /// Microtesla. A compass, and also every magnet near the device.
    Magnetometer,
    /// Device orientation as pitch, roll and yaw in radians, fused from the
    /// three above.
    Orientation,
    /// Whether something is close to the earpiece. `x` is 1.0 for near and 0.0
    /// for far; most hardware reports nothing in between whatever its units
    /// claim.
    Proximity,
    /// Ambient light in lux, in `x`.
    Light,
    /// Steps counted since the device booted, in `x`.
    ///
    /// The one sensor here that is permissioned on both platforms, because it
    /// is health data rather than physics.
    StepCounter,
}

impl Sensor {
    /// Whether the OS gates this one behind a permission prompt.
    ///
    /// Only the step counter is, on both platforms. Surfaced because an
    /// application that asks for activity permission in order to read the
    /// accelerometer is asking for something it does not need, and users
    /// notice.
    #[must_use]
    pub const fn needs_permission(self) -> bool {
        matches!(self, Self::StepCounter)
    }
}

/// How often to be told.
///
/// Named rather than numeric, because the number is a hint on every platform —
/// the OS coalesces, batches and throttles it, and an API taking a hertz value
/// would be promising a rate nothing delivers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum SensorRate {
    /// A few times a second. For a compass needle or a tilt readout.
    #[default]
    Ui,
    /// Tens of times a second. For a shake gesture or a level.
    Game,
    /// As fast as the hardware will go, and as expensive as this gets. For
    /// motion capture, and for nothing that runs for long.
    Fastest,
}

/// One sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionReading {
    pub sensor: Sensor,
    pub values: Vector3,
    /// Nanoseconds since an arbitrary origin, as the platform stamped it.
    ///
    /// Not wall-clock, and not comparable between devices or across a reboot —
    /// which is exactly what it is for. Integrating acceleration needs the
    /// interval between two samples, and a wall clock that can be adjusted
    /// under you is the wrong thing to subtract.
    pub timestamp_ns: u64,
}

/// A live sensor subscription. Dropping it stops the samples.
///
/// The same drop-to-cancel contract as
/// [`LocationWatch`](super::location::LocationWatch), for the same reason: a
/// subscription left running holds hardware awake, and a cancel that has to be
/// called is a cancel somebody forgets.
pub struct SensorSubscription {
    stop: Option<Box<dyn FnOnce()>>,
}

impl SensorSubscription {
    #[must_use]
    pub fn new(stop: impl FnOnce() + 'static) -> Self {
        Self {
            stop: Some(Box::new(stop)),
        }
    }

    #[must_use]
    pub const fn inert() -> Self {
        Self { stop: None }
    }
}

impl std::fmt::Debug for SensorSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SensorSubscription")
            .field("live", &self.stop.is_some())
            .finish()
    }
}

impl Drop for SensorSubscription {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

/// Reading the device's sensors.
///
/// # Why there is no `read_once`
///
/// Because sensors do not work that way. There is no current value to fetch —
/// the hardware is off until something subscribes, and the first sample arrives
/// when it arrives. A `read()` would have to start the sensor, wait an unknown
/// time and stop it again, which is both slower and less useful than the
/// subscription it would be hiding.
pub trait Sensors: 'static {
    /// Which sensors this device has.
    ///
    /// Unguarded: an application deciding whether to offer a compass must not
    /// prompt to find out.
    fn available(&self) -> Vec<Sensor>;

    /// Start receiving samples. `each` runs on the UI thread.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where the device has no such sensor.
    fn subscribe(
        &self,
        sensor: Sensor,
        rate: SensorRate,
        each: Rc<dyn Fn(MotionReading)>,
    ) -> Result<SensorSubscription, ServiceError>;

    /// Start receiving samples from a sensor the OS gates.
    ///
    /// Separate from [`subscribe`](Self::subscribe) rather than a
    /// `grant: Option<..>` parameter, so that the unpermissioned majority stay
    /// callable without a [`Gate`](crate::permission::Gate) and the permissioned
    /// one *cannot* be called without it. Which sensors are which is
    /// [`Sensor::needs_permission`].
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where the device has no such sensor.
    fn subscribe_guarded(
        &self,
        sensor: Sensor,
        rate: SensorRate,
        each: Rc<dyn Fn(MotionReading)>,
        grant: &Grant<'_, dyn Sensors>,
    ) -> Result<SensorSubscription, ServiceError>;
}

impl Guarded for dyn Sensors {
    /// Body sensors: the step counter and anything else the OS treats as
    /// health data. The physics sensors need no permission and
    /// [`Sensors::subscribe`] does not ask for one.
    const PERMISSION: Permission = Permission::new("body-sensors");
}

/// [`Sensors`] on a device with none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSensors;

impl Sensors for NoSensors {
    fn available(&self) -> Vec<Sensor> {
        Vec::new()
    }

    fn subscribe(
        &self,
        _sensor: Sensor,
        _rate: SensorRate,
        _each: Rc<dyn Fn(MotionReading)>,
    ) -> Result<SensorSubscription, ServiceError> {
        Err(ServiceError::unsupported("Sensors"))
    }

    fn subscribe_guarded(
        &self,
        _sensor: Sensor,
        _rate: SensorRate,
        _each: Rc<dyn Fn(MotionReading)>,
        _grant: &Grant<'_, dyn Sensors>,
    ) -> Result<SensorSubscription, ServiceError> {
        Err(ServiceError::unsupported("Sensors"))
    }
}
