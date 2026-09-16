//! Proving it is the same person, using the device.

use crate::permission::{Grant, Guarded, Permission};
use crate::service::ServiceError;
use crate::task::Task;

/// What the device can check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BiometricKind {
    Fingerprint,
    Face,
    Iris,
    /// The device passcode, PIN or pattern — not a biometric, and offered
    /// because every platform offers it as the fallback and an application that
    /// refuses it locks out users who have no biometric enrolled.
    DeviceCredential,
}

/// How much a successful check is worth.
///
/// The reason this is surfaced rather than hidden: Android draws a hard line
/// between sensors it will let guard a keystore key and ones it will not, and
/// an application protecting a payment needs to know which side it is on. A
/// trait that returned a bare `true` would let a face unlock that the platform
/// itself classifies as weak stand in for one that it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BiometricStrength {
    /// Good enough to unlock a screen, not to release a key.
    Weak,
    /// The platform will bind cryptographic material to it.
    Strong,
}

/// What to put in front of the user.
///
/// The strings are required rather than defaulted because the platform shows
/// them verbatim and a default would be the framework writing copy in one
/// language for somebody else's application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BiometricPrompt {
    pub title: String,
    pub subtitle: Option<String>,
    /// What the cancel button says.
    pub cancel_label: String,
    /// Whether a passcode may be used when no biometric works.
    ///
    /// `true` is almost always right. `false` is for the case where the whole
    /// point is *this person*, not *this device's owner*.
    pub allow_device_credential: bool,
}

impl BiometricPrompt {
    /// A prompt with a title and a cancel label, allowing the passcode.
    #[must_use]
    pub fn new(title: impl Into<String>, cancel_label: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            cancel_label: cancel_label.into(),
            allow_device_credential: true,
        }
    }
}

/// Local authentication.
///
/// # What a success means, stated plainly
///
/// That the platform's own check passed on this device, a moment ago. It is not
/// an identity, it is not a token, and it is not something a server can verify
/// — a client that reports "biometrics succeeded" over the network has proved
/// nothing, because the client is what an attacker controls. The real use is
/// releasing something already on the device, which is why
/// [`SecureStorage`](crate::service::SecureStorage) sits beside this and why
/// [`BiometricStrength`] is visible.
pub trait Biometrics: 'static {
    /// What this device can check, and how strongly.
    ///
    /// Empty when the hardware exists but nothing is enrolled — which is a
    /// different situation from having no sensor, and both mean the same thing
    /// to a caller: do not offer this. Unguarded, because deciding whether to
    /// show the option must not itself prompt.
    fn available(&self) -> Vec<(BiometricKind, BiometricStrength)>;

    /// Ask the user to authenticate.
    ///
    /// A [`Task`] because the platform's own sheet is on screen for as long as
    /// the user takes.
    ///
    /// A refusal comes back as [`ServiceError::Denied`], not as `Ok(false)`.
    /// The distinction is the point: "the user cancelled" and "the check ran
    /// and rejected them" want different screens, and a boolean collapses them.
    fn authenticate(
        &self,
        prompt: &BiometricPrompt,
        grant: &Grant<'_, dyn Biometrics>,
    ) -> Task<BiometricStrength, ServiceError>;
}

impl Guarded for dyn Biometrics {
    const PERMISSION: Permission = Permission::new("biometrics");
}

/// [`Biometrics`] on a device with no sensor.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBiometrics;

impl Biometrics for NoBiometrics {
    fn available(&self) -> Vec<(BiometricKind, BiometricStrength)> {
        Vec::new()
    }

    fn authenticate(
        &self,
        _prompt: &BiometricPrompt,
        _grant: &Grant<'_, dyn Biometrics>,
    ) -> Task<BiometricStrength, ServiceError> {
        Task::failed(ServiceError::unsupported("Biometrics"))
    }
}
