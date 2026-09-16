//! Taking a picture.

use crate::permission::{Grant, Guarded, Permission};
use crate::service::ServiceError;
use crate::task::Task;

/// Which way a camera points.
///
/// Not a device index. Indices differ between manufacturers and change when a
/// phone is folded; "the selfie camera" is the thing an application actually
/// means, and it is the thing that stays true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraFacing {
    /// Towards the user.
    Front,
    /// Away from the user.
    Back,
    /// A camera that is neither — a plugged-in webcam, a clip-on lens.
    External,
}

/// One camera the platform has found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDevice {
    /// The platform's own identifier, opaque and stable within one run.
    pub id: String,
    pub facing: CameraFacing,
    /// A name fit to show a user, when there is more than one to choose from.
    pub name: String,
}

/// Whether the flash fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlashMode {
    Off,
    On,
    /// Let the platform decide from the light it can see.
    #[default]
    Auto,
}

/// What to take a picture of, and how.
///
/// Every field is optional-shaped, so `PhotoRequest::default()` is the
/// reasonable request: the back camera if there is one, at whatever size the
/// device likes, flash on the platform's judgement. An API whose simple case
/// needs five decisions is an API nobody reaches for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhotoRequest {
    /// A specific [`CameraDevice::id`], or `None` for the platform's default.
    pub device: Option<String>,
    /// Which way to point, when no specific device was named.
    pub facing: Option<CameraFacing>,
    /// The longest edge the result should have, in pixels.
    ///
    /// A cap rather than a size: a modern sensor produces twelve megapixels,
    /// an avatar needs five hundred pixels, and moving the difference through
    /// memory and up to a server is the most common performance mistake in
    /// applications that take photographs. Asking the platform to downscale is
    /// far cheaper than doing it afterwards, because it can do it before the
    /// full-size image is ever materialised.
    pub max_dimension: Option<u32>,
    pub flash: FlashMode,
}

/// A picture, as the platform encoded it.
///
/// # Why bytes rather than a decoded image
///
/// Because the caller almost always wants the bytes. A photograph is uploaded,
/// stored or attached far more often than it is drawn, and every one of those
/// wants the JPEG the camera already produced. A trait that decoded eagerly
/// would spend the memory and the milliseconds on every capture to serve the
/// minority case, and the majority case would then re-encode to undo it.
///
/// Drawing one is [`Image`](crate::Image)'s job, from these bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    pub bytes: Vec<u8>,
    /// `image/jpeg`, `image/heic`, `image/png`.
    ///
    /// Stated rather than assumed: iOS produces HEIC by default and an
    /// application that hard-codes JPEG uploads a file its server cannot read.
    pub mime: String,
    pub width: u32,
    pub height: u32,
}

/// The device's cameras.
///
/// # Stills only, and why the video is missing
///
/// A video recording is a session with a start, a stop, a file growing on disk,
/// an interruption when a call arrives, and a preview surface composited into
/// the tree — five mechanisms this trait does not have, and one of them
/// ([`PlatformViews`](super::PlatformViews)) is only just declared. Adding
/// `record()` here would be a method that cannot be implemented honestly, which
/// is worse than an absence a reader can see.
///
/// The preview is the same story: showing what the lens sees means a native
/// surface inside the widget tree, which is a platform view. The two compose
/// once both exist, and neither has to pretend in the meantime.
pub trait Camera: 'static {
    /// The cameras this device has.
    ///
    /// Unguarded on purpose. On both platforms this is metadata, not imagery —
    /// an application legitimately needs to know whether a front camera exists
    /// before it decides whether to offer a button that would ask for
    /// permission at all, and requiring permission to find that out means every
    /// application asks for the camera on launch.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where there is no camera subsystem at all.
    fn devices(&self) -> Result<Vec<CameraDevice>, ServiceError>;

    /// Take one picture.
    ///
    /// Returns a [`Task`] because it is slow in a way no platform hides: the
    /// shutter, the capture, the encode, and — on both platforms — a whole
    /// system camera interface the user has to interact with first.
    fn capture(
        &self,
        request: &PhotoRequest,
        grant: &Grant<'_, dyn Camera>,
    ) -> Task<Photo, ServiceError>;
}

impl Guarded for dyn Camera {
    const PERMISSION: Permission = Permission::new("camera");
}

/// A [`Camera`] on a device that has none, or in a build that did not include
/// one.
///
/// Refuses rather than pretends — see the module documentation for why this is
/// the opposite choice from [`MemoryStorage`](crate::service::MemoryStorage).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCamera;

impl Camera for NoCamera {
    fn devices(&self) -> Result<Vec<CameraDevice>, ServiceError> {
        // An empty list, not an error: "this device has no cameras" is a true
        // and useful answer, and it is the one that lets a screen hide its
        // camera button without treating the absence as a failure.
        Ok(Vec::new())
    }

    fn capture(
        &self,
        _request: &PhotoRequest,
        _grant: &Grant<'_, dyn Camera>,
    ) -> Task<Photo, ServiceError> {
        Task::failed(ServiceError::unsupported("Camera"))
    }
}
