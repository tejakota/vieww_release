//! Mobile hardware capabilities, behind the permission seam.
//!
//! Haptics, biometrics, push, camera, Bluetooth, geofencing, the microphone and
//! on-device inference. The checklist calls these "mobile-specific"; most of
//! them exist on desktop too, and the ones that do not report
//! [`ServiceError::Unsupported`] rather than being absent from the vocabulary.
//!
//! # Every capability that needs a permission takes a `Grant`
//!
//! This is the whole point of the module and the reason it lands as one piece
//! rather than one capability at a time. [`permission`](crate::permission)'s
//! rule is that a guarded capability's methods take a
//! [`Grant`], which has no public constructor — so
//! *using the camera without having resolved the camera permission* is not
//! something an application can express. It is not guarded at runtime; it has no
//! syntax.
//!
//! `docs/AIMS.md` §C asked for that decision to be made "before the second
//! service, not the fifth". It was made before the first. This module is the
//! first eight, all taking it.
//!
//! [`Haptics`] is the exception and is deliberately unguarded: no platform asks
//! permission to vibrate, and inventing one would make every call site carry a
//! gate for a question nobody is ever asked.
//!
//! # Which methods take the grant, and the rule a backend must follow
//!
//! Not all of them, and the split is deliberate rather than an oversight. The
//! grant sits on **the method that starts hardware, prompts, or reaches the
//! network** — `start`, `register`, `add`, `authenticate` — and not on the
//! `take_*` methods that drain what those produced.
//!
//! The reason is that a `take_*` call happens on every frame. Requiring a live
//! grant to read a queue would mean re-resolving the permission sixty times a
//! second for an answer that has not changed, and the predictable consequence is
//! an application that hoists one grant to the top of the frame and passes it
//! everywhere — which is the design being worked around rather than used.
//! Draining a queue the application already, with permission, caused to fill is
//! not a second use of the camera.
//!
//! What that leaves is the one freedom a backend author still has, so it is
//! written down here rather than inferred from these eight: **a new method that
//! makes the hardware do something takes a grant.** The headless implementations
//! below already model the other half of it — a stopped scanner delivers nothing
//! it queued, so a permission revoked between `start` and the next frame cannot
//! be laundered through a stale queue.
//!
//! # What is here and what is not
//!
//! These are **seams with headless implementations**, not platform backends.
//! Every trait below can be driven end to end in a test, and every one of them
//! is `Unsupported` on a desktop with no hardware behind it. A real backend is
//! separate work, per platform, and it does not change a line of this file —
//! which is the property `docs/AIMS.md` §A calls the framework's actual moat.
//!
//! The honest reading of the checklist, therefore: the *shape* of each mobile
//! row is closed and the *hardware* is not. Nothing here should be read as
//! "vieww talks to a camera".

use std::cell::RefCell;
use std::rc::Rc;

use crate::permission::{Grant, Guarded, Permission};
use crate::service::{ServiceError, Services};
use crate::task::Task;

/// How long a haptic lasts and how it feels.
///
/// Named patterns rather than a duration and an amplitude, because the platforms
/// do not agree on either and a framework that exposed milliseconds would let an
/// application ask for something iOS silently rounds to one of these anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HapticPattern {
    /// The lightest tick — a picker passing a detent, a toggle landing.
    Selection,
    Light,
    Medium,
    Heavy,
    /// Something completed.
    Success,
    /// Something needs attention but is not an error.
    Warning,
    /// Something failed.
    Error,
}

/// Vibration and taptic feedback.
///
/// **Unguarded, deliberately.** No platform asks permission to vibrate, so a
/// gate here would be a ceremony around a question that is never put to the
/// user — and every ceremony that protects nothing is one an application learns
/// to route around.
pub trait Haptics: 'static {
    /// Play a pattern.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where there is no vibrator. Not an error
    /// worth handling at most call sites: a haptic that does not happen is a
    /// missing garnish, so this returns a `Result` for the caller that cares and
    /// is ordinarily ignored.
    fn play(&self, pattern: HapticPattern) -> Result<(), ServiceError>;
}

/// **Biometrics is declared once, in [`capability`](crate::capability), and
/// re-exported here.**
///
/// This module used to declare a second trait of the same name with a different
/// shape — a callback taking a `BiometricOutcome` where the other returns a
/// [`Task<BiometricStrength, ServiceError>`](crate::task::Task) — and both were
/// registered under the `dyn Biometrics` key with a permission both spelled
/// `biometrics`. Two modules, so both compiled; one service registry, so which
/// one an application got depended on which module it had imported. That
/// resolves to a *different type* for a caller who imported the other, and it
/// compiles cleanly on both sides right up until the two meet.
///
/// The cost while there is no native bridge is confusion. The cost afterwards is
/// a breaking public-API migration, because a JNI or Objective-C binding written
/// against one shape hardens it. So the duplicate is gone rather than deprecated,
/// and the survivor is the richer one: it distinguishes
/// [`BiometricStrength::Weak`] from [`Strong`](BiometricStrength::Strong), which
/// is the distinction Android enforces around keystore keys, and it reports a
/// user's refusal as [`ServiceError::Denied`] rather than folding it into the
/// same value as a failed check.
pub use crate::capability::{
    BiometricKind, BiometricPrompt, BiometricStrength, Biometrics, NoBiometrics,
};

/// A message delivered by the platform's push service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushMessage {
    /// The payload, as the sender wrote it.
    pub data: Vec<(String, String)>,
    /// What the OS displayed, if it displayed anything.
    pub title: Option<String>,
    pub body: Option<String>,
}

/// Remote notifications.
///
/// # The token is polled, not pushed, and that is not laziness
///
/// A registration token arrives on a platform thread whenever the OS feels like
/// it — at launch, after a reinstall, or when it is rotated months later. A
/// callback would run off the UI thread; a signal written from there is the
/// exact race `docs/DESIGN.md` forbids. So the application reads
/// [`take_token`](Self::take_token) during a frame, like
/// [`DeepLinks`](crate::service::DeepLinks).
pub trait PushNotifications: 'static {
    /// Ask the OS to register this install with its push service.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where there is no push service — which
    /// includes a de-Googled Android and every desktop.
    fn register(&self, _: &Grant<'_, dyn PushNotifications>) -> Result<(), ServiceError>;

    /// A new registration token since this was last called.
    ///
    /// Delivered once. A token that arrives while nobody is polling is kept
    /// until somebody does, because a token missed is an install that silently
    /// never receives anything.
    fn take_token(&self) -> Option<String>;

    /// A message delivered since this was last called.
    fn take_message(&self) -> Option<PushMessage>;
}

impl Guarded for dyn PushNotifications {
    const PERMISSION: Permission = Permission::new("notifications");
}

/// One machine-readable code read out of a camera frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Barcode {
    /// `QR_CODE`, `EAN_13`, `CODE_128` — the platform's own name, not an enum.
    ///
    /// A `&'static str` for [`Permission`]'s reason: a third party's detector
    /// knows formats this crate has never heard of, and an enum means editing
    /// this file to add one.
    pub format: String,
    pub value: String,
}

/// **`CameraFacing` is declared once, in [`capability`](crate::capability), and
/// re-exported here.**
///
/// Same defect as [`Biometrics`] above and the same fix: a barcode scan and a
/// photo request take the same conceptual value, and they used to take two
/// unrelated types that differed by one variant. The survivor carries
/// [`External`](CameraFacing::External) — a plugged-in webcam or a clip-on lens
/// is a real third answer, and a scanner enumerating cameras on a desktop has to
/// be able to say it.
pub use crate::capability::CameraFacing;

/// The camera, as a source of scanned codes.
///
/// # Scanning, not a preview widget
///
/// Deliberately the narrower capability. A live preview is a *texture* that has
/// to reach the compositor, which is a different and much larger piece of work
/// touching [`Image`](crate::Image), the paint layer and every backend. Barcode
/// scanning is the thing applications overwhelmingly want a camera for, it needs
/// no new rendering surface, and shipping it first does not foreclose the
/// preview — a `CameraPreview` capability lands beside this one when there is a
/// backend to draw it.
///
/// Saying so here rather than calling this trait `Camera` and quietly meaning a
/// tenth of one.
pub trait BarcodeScanner: 'static {
    /// Start reading frames.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] with no camera, [`ServiceError::Failed`] if
    /// another application holds it.
    fn start(
        &self,
        facing: CameraFacing,
        _: &Grant<'_, dyn BarcodeScanner>,
    ) -> Result<(), ServiceError>;

    /// Stop reading frames.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if it was not running.
    fn stop(&self) -> Result<(), ServiceError>;

    /// A code seen since this was last called.
    fn take_scanned(&self) -> Option<Barcode>;
}

impl Guarded for dyn BarcodeScanner {
    const PERMISSION: Permission = Permission::new("camera");
}

/// A Bluetooth Low Energy peripheral that has been seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Peripheral {
    /// The platform's own identifier. **Not** a MAC address: iOS never gives one
    /// out, and an API shaped around having one does not port.
    pub id: String,
    pub name: Option<String>,
    /// Signal strength in dBm, if the platform reported it.
    pub rssi: Option<i16>,
}

/// Bluetooth Low Energy.
///
/// Scanning and connecting only. GATT characteristic traffic is the next piece
/// and is deliberately not guessed at here — it needs a byte-level protocol
/// surface that ought to be designed against a real device rather than against
/// an idea of one.
pub trait BluetoothLe: 'static {
    /// Start looking for peripherals advertising `service`, or everything when
    /// `None`.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] with no radio, [`ServiceError::Failed`]
    /// when the radio is off — which is recoverable by the user and must not
    /// read as permanent.
    fn start_scan(
        &self,
        service: Option<&str>,
        _: &Grant<'_, dyn BluetoothLe>,
    ) -> Result<(), ServiceError>;

    /// Stop looking.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if no scan was running.
    fn stop_scan(&self) -> Result<(), ServiceError>;

    /// A peripheral seen since this was last called.
    fn take_discovered(&self) -> Option<Peripheral>;
}

impl Guarded for dyn BluetoothLe {
    const PERMISSION: Permission = Permission::new("bluetooth");
}

/// A circular region the OS watches on the application's behalf.
#[derive(Debug, Clone, PartialEq)]
pub struct Geofence {
    /// The application's own name for it, returned with every crossing.
    pub id: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Metres.
    pub radius: f32,
}

/// Which way a boundary was crossed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeofenceTransition {
    Entered,
    Exited,
}

/// Region monitoring that continues while the application is not running.
///
/// # `f64` here and `f32` everywhere else
///
/// `docs/DESIGN.md` §4 says every scalar in this crate is `f32`, and that rule
/// is about *geometry*: pixels, where `f32` has far more precision than a screen
/// can show. A latitude in `f32` has about a metre of resolution at the equator,
/// which is coarse compared to the radius of the smallest fence anyone sets. The
/// rule's reason does not reach here, so neither does the rule.
pub trait Geofencing: 'static {
    /// Ask the OS to watch a region.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where there is no such facility, and
    /// [`ServiceError::Failed`] past the platform's limit — iOS allows twenty
    /// per application, which is a real ceiling applications hit.
    fn add(&self, fence: Geofence, _: &Grant<'_, dyn Geofencing>) -> Result<(), ServiceError>;

    /// Stop watching a region.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if `id` is not being watched.
    fn remove(&self, id: &str) -> Result<(), ServiceError>;

    /// A crossing since this was last called.
    ///
    /// Crossings that happened while the process was dead are delivered on the
    /// next launch — which is the entire reason to use the OS's monitoring
    /// rather than watching coordinates in a background task.
    fn take_transition(&self) -> Option<(String, GeofenceTransition)>;
}

impl Guarded for dyn Geofencing {
    const PERMISSION: Permission = Permission::new("location-always");
}

/// Raw audio captured from the microphone.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    /// Interleaved samples, normalised to −1.0..=1.0.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Microphone capture.
pub trait Microphone: 'static {
    /// Begin capturing.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] with no input device, and
    /// [`ServiceError::Failed`] when one is held by something else.
    fn start(&self, _: &Grant<'_, dyn Microphone>) -> Result<(), ServiceError>;

    /// Stop capturing.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if it was not running.
    fn stop(&self) -> Result<(), ServiceError>;

    /// Audio captured since this was last called.
    ///
    /// A chunk rather than a callback per sample, for
    /// [`DeepLinks`](crate::service::DeepLinks)' reason: capture runs on an
    /// audio thread with a hard deadline, and anything that reaches the UI from
    /// there has to be handed over rather than called into.
    fn take_chunk(&self) -> Option<AudioChunk>;
}

impl Guarded for dyn Microphone {
    const PERMISSION: Permission = Permission::new("microphone");
}

/// One output of a model, with how sure it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    pub label: String,
    /// 0.0 to 1.0.
    pub confidence: f32,
}

/// On-device inference.
///
/// # Deliberately opaque about what a model *is*
///
/// [`load`](Self::load) takes a name and nothing else, and inference takes bytes
/// and returns labels. Core ML, TFLite, ONNX and NNAPI disagree about every
/// other detail — tensor layout, quantisation, the shape of an input — and a
/// framework that picked one of their vocabularies would be shipping a binding
/// to that runtime under a neutral name.
///
/// **Unguarded**, because no platform asks permission to run a model. The
/// permission an application actually needs is for whatever produced the input:
/// the camera, or the microphone, both of which are guarded above.
pub trait Inference: 'static {
    /// Make a model available under a name the application chooses.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where there is no runtime, and
    /// [`ServiceError::Failed`] when the bytes are not a model this runtime
    /// understands.
    fn load(&self, name: &str, model: &[u8]) -> Result<(), ServiceError>;

    /// Run a loaded model.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] when `name` was never loaded, or the input does
    /// not fit the model.
    fn run(&self, name: &str, input: &[u8]) -> Result<Vec<Prediction>, ServiceError>;
}

// ---------------------------------------------------------------------------
// Headless implementations
// ---------------------------------------------------------------------------

/// Haptics that are recorded rather than felt.
#[derive(Debug, Default)]
pub struct HeadlessHaptics {
    played: RefCell<Vec<HapticPattern>>,
}

impl HeadlessHaptics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every pattern played so far, in order.
    #[must_use]
    pub fn played(&self) -> Vec<HapticPattern> {
        self.played.borrow().clone()
    }
}

impl Haptics for HeadlessHaptics {
    fn play(&self, pattern: HapticPattern) -> Result<(), ServiceError> {
        self.played.borrow_mut().push(pattern);
        Ok(())
    }
}

/// Biometrics that answer whatever a test told them to.
///
/// Rewritten onto [`capability::Biometrics`](crate::capability::Biometrics) when
/// the duplicate trait above was removed. The scripted outcomes map onto that
/// trait's vocabulary rather than a parallel enum: a success carries the
/// [`BiometricStrength`] the device would have proved, and every failure is the
/// [`ServiceError`] the real backend would report — which is the distinction the
/// callback shape could not make, because "the user cancelled" and "the sensor
/// rejected them" want different screens.
#[derive(Debug)]
pub struct ScriptedBiometrics {
    available: Vec<(BiometricKind, BiometricStrength)>,
    outcome: RefCell<Result<BiometricStrength, ServiceError>>,
    prompts: RefCell<Vec<BiometricPrompt>>,
}

impl ScriptedBiometrics {
    /// A device with a strong fingerprint reader that says yes.
    #[must_use]
    pub fn succeeding() -> Self {
        Self {
            available: vec![(BiometricKind::Fingerprint, BiometricStrength::Strong)],
            outcome: RefCell::new(Ok(BiometricStrength::Strong)),
            prompts: RefCell::new(Vec::new()),
        }
    }

    /// A device with the hardware and nothing enrolled — the case applications
    /// forget, because it looks like "available" until the prompt appears.
    ///
    /// Empty availability, exactly as the trait specifies: enrolled-nothing and
    /// no-sensor are different situations that mean the same thing to a caller
    /// deciding whether to offer the option at all.
    #[must_use]
    pub fn not_enrolled() -> Self {
        Self {
            available: Vec::new(),
            outcome: RefCell::new(Err(ServiceError::unsupported("Biometrics"))),
            prompts: RefCell::new(Vec::new()),
        }
    }

    /// A device whose sensor works and whose user declines.
    #[must_use]
    pub fn refusing() -> Self {
        let mut scripted = Self::succeeding();
        scripted.outcome = RefCell::new(Err(ServiceError::Denied(
            "the user dismissed the prompt".to_owned(),
        )));
        scripted
    }

    /// Change what the next prompt answers.
    pub fn set_outcome(&self, outcome: Result<BiometricStrength, ServiceError>) {
        *self.outcome.borrow_mut() = outcome;
    }

    /// Declare what this device can check.
    #[must_use]
    pub fn with_available(mut self, available: Vec<(BiometricKind, BiometricStrength)>) -> Self {
        self.available = available;
        self
    }

    /// Every prompt put in front of the user.
    #[must_use]
    pub fn prompts(&self) -> Vec<BiometricPrompt> {
        self.prompts.borrow().clone()
    }
}

impl Biometrics for ScriptedBiometrics {
    fn available(&self) -> Vec<(BiometricKind, BiometricStrength)> {
        self.available.clone()
    }

    fn authenticate(
        &self,
        prompt: &BiometricPrompt,
        _grant: &Grant<'_, dyn Biometrics>,
    ) -> Task<BiometricStrength, ServiceError> {
        self.prompts.borrow_mut().push(prompt.clone());
        match self.outcome.borrow().clone() {
            Ok(strength) => Task::ready(strength),
            Err(error) => Task::failed(error),
        }
    }
}

/// A push service with no network behind it.
#[derive(Debug, Default)]
pub struct HeadlessPush {
    registered: RefCell<bool>,
    tokens: RefCell<Vec<String>>,
    messages: RefCell<Vec<PushMessage>>,
}

impl HeadlessPush {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend the service issued a token.
    pub fn push_token(&self, token: impl Into<String>) {
        self.tokens.borrow_mut().push(token.into());
    }

    /// Pretend a message arrived.
    pub fn push_message(&self, message: PushMessage) {
        self.messages.borrow_mut().push(message);
    }

    /// `true` once [`PushNotifications::register`] has been called.
    #[must_use]
    pub fn is_registered(&self) -> bool {
        *self.registered.borrow()
    }
}

impl PushNotifications for HeadlessPush {
    fn register(&self, _: &Grant<'_, dyn PushNotifications>) -> Result<(), ServiceError> {
        *self.registered.borrow_mut() = true;
        Ok(())
    }

    fn take_token(&self) -> Option<String> {
        take_first(&self.tokens)
    }

    fn take_message(&self) -> Option<PushMessage> {
        take_first(&self.messages)
    }
}

/// A scanner that reads whatever a test put in front of it.
#[derive(Debug, Default)]
pub struct HeadlessBarcodeScanner {
    running: RefCell<bool>,
    facing: RefCell<Option<CameraFacing>>,
    scanned: RefCell<Vec<Barcode>>,
}

impl HeadlessBarcodeScanner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend a code came into view.
    pub fn push_scanned(&self, barcode: Barcode) {
        self.scanned.borrow_mut().push(barcode);
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        *self.running.borrow()
    }

    #[must_use]
    pub fn facing(&self) -> Option<CameraFacing> {
        *self.facing.borrow()
    }
}

impl BarcodeScanner for HeadlessBarcodeScanner {
    fn start(
        &self,
        facing: CameraFacing,
        _: &Grant<'_, dyn BarcodeScanner>,
    ) -> Result<(), ServiceError> {
        *self.running.borrow_mut() = true;
        *self.facing.borrow_mut() = Some(facing);
        Ok(())
    }

    fn stop(&self) -> Result<(), ServiceError> {
        if !*self.running.borrow() {
            return Err(ServiceError::failed("the scanner is not running"));
        }
        *self.running.borrow_mut() = false;
        Ok(())
    }

    fn take_scanned(&self) -> Option<Barcode> {
        // Nothing is delivered while stopped. A queued frame arriving after
        // `stop` is how a scanner ends up acting on a code the user has already
        // walked away from.
        if !*self.running.borrow() {
            return None;
        }
        take_first(&self.scanned)
    }
}

/// A radio that discovers whatever a test tells it to.
#[derive(Debug, Default)]
pub struct HeadlessBluetooth {
    scanning: RefCell<bool>,
    discovered: RefCell<Vec<Peripheral>>,
}

impl HeadlessBluetooth {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend a peripheral advertised.
    pub fn push_discovered(&self, peripheral: Peripheral) {
        self.discovered.borrow_mut().push(peripheral);
    }

    #[must_use]
    pub fn is_scanning(&self) -> bool {
        *self.scanning.borrow()
    }
}

impl BluetoothLe for HeadlessBluetooth {
    fn start_scan(
        &self,
        _service: Option<&str>,
        _: &Grant<'_, dyn BluetoothLe>,
    ) -> Result<(), ServiceError> {
        *self.scanning.borrow_mut() = true;
        Ok(())
    }

    fn stop_scan(&self) -> Result<(), ServiceError> {
        if !*self.scanning.borrow() {
            return Err(ServiceError::failed("no scan is running"));
        }
        *self.scanning.borrow_mut() = false;
        Ok(())
    }

    fn take_discovered(&self) -> Option<Peripheral> {
        if !*self.scanning.borrow() {
            return None;
        }
        take_first(&self.discovered)
    }
}

/// Region monitoring in a `Vec`.
#[derive(Debug, Default)]
pub struct HeadlessGeofencing {
    watched: RefCell<Vec<Geofence>>,
    transitions: RefCell<Vec<(String, GeofenceTransition)>>,
}

impl HeadlessGeofencing {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend the user crossed a boundary.
    pub fn push_transition(&self, id: impl Into<String>, transition: GeofenceTransition) {
        self.transitions.borrow_mut().push((id.into(), transition));
    }

    /// The regions currently being watched.
    #[must_use]
    pub fn watched(&self) -> Vec<Geofence> {
        self.watched.borrow().clone()
    }
}

impl Geofencing for HeadlessGeofencing {
    fn add(&self, fence: Geofence, _: &Grant<'_, dyn Geofencing>) -> Result<(), ServiceError> {
        let mut watched = self.watched.borrow_mut();
        if watched.iter().any(|held| held.id == fence.id) {
            return Err(ServiceError::failed("that region is already watched"));
        }
        watched.push(fence);
        Ok(())
    }

    fn remove(&self, id: &str) -> Result<(), ServiceError> {
        let mut watched = self.watched.borrow_mut();
        let before = watched.len();
        watched.retain(|fence| fence.id != id);
        if watched.len() == before {
            return Err(ServiceError::failed("that region is not watched"));
        }
        Ok(())
    }

    fn take_transition(&self) -> Option<(String, GeofenceTransition)> {
        // Deliberately *not* gated on the region still being watched: a crossing
        // that happened while the process was dead is the reason this capability
        // exists, and it arrives before the application has re-added anything.
        take_first(&self.transitions)
    }
}

/// A microphone that captures what a test hands it.
#[derive(Debug, Default)]
pub struct HeadlessMicrophone {
    running: RefCell<bool>,
    chunks: RefCell<Vec<AudioChunk>>,
}

impl HeadlessMicrophone {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pretend audio was captured.
    pub fn push_chunk(&self, chunk: AudioChunk) {
        self.chunks.borrow_mut().push(chunk);
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        *self.running.borrow()
    }
}

impl Microphone for HeadlessMicrophone {
    fn start(&self, _: &Grant<'_, dyn Microphone>) -> Result<(), ServiceError> {
        *self.running.borrow_mut() = true;
        Ok(())
    }

    fn stop(&self) -> Result<(), ServiceError> {
        if !*self.running.borrow() {
            return Err(ServiceError::failed("the microphone is not running"));
        }
        *self.running.borrow_mut() = false;
        Ok(())
    }

    fn take_chunk(&self) -> Option<AudioChunk> {
        if !*self.running.borrow() {
            return None;
        }
        take_first(&self.chunks)
    }
}

/// A model runtime that answers from a lookup table.
#[derive(Debug, Default)]
pub struct ScriptedInference {
    loaded: RefCell<Vec<String>>,
    answers: RefCell<Vec<(String, Vec<Prediction>)>>,
}

impl ScriptedInference {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What a named model should answer, whatever it is given.
    pub fn set_answer(&self, name: impl Into<String>, predictions: Vec<Prediction>) {
        self.answers.borrow_mut().push((name.into(), predictions));
    }

    /// Which models have been loaded.
    #[must_use]
    pub fn loaded(&self) -> Vec<String> {
        self.loaded.borrow().clone()
    }
}

impl Inference for ScriptedInference {
    fn load(&self, name: &str, model: &[u8]) -> Result<(), ServiceError> {
        if model.is_empty() {
            return Err(ServiceError::failed("an empty model is not a model"));
        }
        self.loaded.borrow_mut().push(name.to_owned());
        Ok(())
    }

    fn run(&self, name: &str, _input: &[u8]) -> Result<Vec<Prediction>, ServiceError> {
        if !self.loaded.borrow().iter().any(|held| held == name) {
            return Err(ServiceError::failed("that model was never loaded"));
        }
        Ok(self
            .answers
            .borrow()
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, predictions)| predictions.clone())
            .unwrap_or_default())
    }
}

/// Pop the front of a queue, or `None`.
///
/// One helper rather than the same four lines in seven `take_*` methods — and
/// one place for the property every one of them shares: an event is delivered
/// **once**, so this is a take rather than a read.
fn take_first<T>(queue: &RefCell<Vec<T>>) -> Option<T> {
    let mut queue = queue.borrow_mut();
    if queue.is_empty() {
        None
    } else {
        Some(queue.remove(0))
    }
}

/// Register a headless implementation of every capability in this module.
///
/// The unguarded two are usable immediately; the six guarded ones still need a
/// [`Permissions`](crate::permission::Permissions) implementation registered
/// before a [`Gate`](crate::permission::Gate) can be obtained for them, which is
/// the point of the guard and not an oversight here.
pub fn provide_headless(services: &mut Services) {
    services.provide::<dyn Haptics>(Rc::new(HeadlessHaptics::new()));
    services.provide::<dyn Biometrics>(Rc::new(ScriptedBiometrics::succeeding()));
    services.provide::<dyn PushNotifications>(Rc::new(HeadlessPush::new()));
    services.provide::<dyn BarcodeScanner>(Rc::new(HeadlessBarcodeScanner::new()));
    services.provide::<dyn BluetoothLe>(Rc::new(HeadlessBluetooth::new()));
    services.provide::<dyn Geofencing>(Rc::new(HeadlessGeofencing::new()));
    services.provide::<dyn Microphone>(Rc::new(HeadlessMicrophone::new()));
    services.provide::<dyn Inference>(Rc::new(ScriptedInference::new()));
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::permission::{
        AlwaysDenied, AlwaysGranted, AlwaysRestricted, PermissionState, ScriptedPermissions,
    };

    // Taken from the `Guarded` impls rather than re-spelled, so a test can never
    // stage a permission the capability does not actually sit behind — which
    // would pass while proving nothing.
    const CAMERA: Permission = <dyn BarcodeScanner as Guarded>::PERMISSION;
    const MICROPHONE: Permission = <dyn Microphone as Guarded>::PERMISSION;

    fn granted() -> Services {
        let mut services = Services::new();
        provide_headless(&mut services);
        services.provide::<dyn crate::permission::Permissions>(Rc::new(AlwaysGranted));
        services
    }

    #[test]
    fn haptics_need_no_permission_and_no_ceremony() {
        // The exception, asserted rather than only argued: nobody is ever asked
        // whether an application may vibrate, so a gate here would guard
        // nothing.
        let mut services = Services::new();
        provide_headless(&mut services);
        let haptics = services.get::<dyn Haptics>().expect("registered");
        haptics.play(HapticPattern::Selection).expect("headless");
        haptics.play(HapticPattern::Success).expect("headless");
    }

    #[test]
    fn a_guarded_capability_is_unreachable_without_a_gate() {
        // The §C claim on a real capability. `BarcodeScanner::start` takes a
        // `Grant`, and a `Grant` has no public constructor — so the only way to
        // reach the camera is through a gate that read the permission first.
        //
        // The compile-time half of this lives in `permission.rs` as two
        // `compile_fail` doctests — one that a grant cannot be forged, one that
        // it cannot be kept past its check. This is the runtime half, which is
        // that the gate actually refuses.
        let mut services = Services::new();
        provide_headless(&mut services);
        services.provide::<dyn crate::permission::Permissions>(Rc::new(AlwaysDenied));

        let gate = services
            .gate::<dyn BarcodeScanner>()
            .expect("the capability and a permissions impl are both registered");
        assert!(
            gate.granted().is_none(),
            "a refused permission hands out no capability at all"
        );
        assert_eq!(
            gate.state(),
            PermissionState::Blocked,
            "`AlwaysDenied` models the refusal the OS will not ask about again, \
             which is the one an application has to route to Settings"
        );
    }

    #[test]
    fn a_granted_gate_hands_over_the_scanner() {
        let services = granted();
        let gate = services.gate::<dyn BarcodeScanner>().expect("registered");
        let Some((scanner, grant)) = gate.granted() else {
            panic!("AlwaysGranted grants");
        };
        scanner
            .start(CameraFacing::Back, &grant)
            .expect("headless starts");
        // Stopping succeeds only if it was actually started, which is the one
        // observable the trait object exposes — and enough to show the grant
        // reached a real implementation rather than a stub.
        scanner.stop().expect("it was running");
        assert!(scanner.stop().is_err(), "and now it is not");
    }

    #[test]
    fn a_stopped_scanner_delivers_nothing_it_queued() {
        // The case that produces a real bug: a frame decoded just as the user
        // dismissed the scanner, acted on a second later.
        let scanner = HeadlessBarcodeScanner::new();
        let services = granted();
        let gate = services.gate::<dyn BarcodeScanner>().expect("registered");
        let (_, grant) = gate.granted().expect("granted");

        scanner.start(CameraFacing::Back, &grant).expect("starts");
        scanner.push_scanned(Barcode {
            format: "QR_CODE".into(),
            value: "https://example.com".into(),
        });
        scanner.stop().expect("running");
        assert!(
            scanner.take_scanned().is_none(),
            "a code decoded before the scanner stopped must not arrive after it"
        );
    }

    #[test]
    fn biometrics_report_not_enrolled_rather_than_unavailable() {
        // The state applications forget. `available` is about hardware and
        // answers empty here for a *different* reason than "no sensor" — which
        // is why the failure is `Unsupported` rather than `Denied`: the user
        // never got as far as refusing.
        let services = granted();
        let biometrics = ScriptedBiometrics::not_enrolled();
        let gate = services.gate::<dyn Biometrics>().expect("registered");
        let (_, grant) = gate.granted().expect("granted");

        assert!(biometrics.available().is_empty());

        let prompt = BiometricPrompt::new("Unlock your notes", "Not now");
        let task = biometrics.authenticate(&prompt, &grant);
        assert_eq!(
            task.value().failed(),
            Some(&ServiceError::unsupported("Biometrics")),
            "no sensor enrolled is unsupported, not a refusal"
        );
        assert_eq!(biometrics.prompts(), vec![prompt]);
    }

    /// **The consolidation's own regression test.**
    ///
    /// There is exactly one `Biometrics` in the crate now, so a service
    /// registered through `capability` and one registered through `mobile`
    /// resolve to the same key and the same type. Before, these were two traits
    /// and this test could not have been written: the second `provide` would
    /// have registered under a different `TypeId` and `get` would have handed
    /// back whichever the caller's import happened to name.
    #[test]
    fn mobile_and_capability_name_one_biometrics() {
        let mut services = Services::new();
        services.provide::<dyn Biometrics>(Rc::new(ScriptedBiometrics::succeeding()));
        assert!(
            services
                .get::<dyn crate::capability::Biometrics>()
                .is_some(),
            "the same registration has to be visible under the capability path"
        );
    }

    /// A refusal and a failure are different answers, which is the distinction
    /// the callback-shaped trait could not express.
    #[test]
    fn a_refused_prompt_is_denied_rather_than_failed() {
        let services = granted();
        let biometrics = ScriptedBiometrics::refusing();
        let gate = services.gate::<dyn Biometrics>().expect("registered");
        let (_, grant) = gate.granted().expect("granted");

        let task = biometrics.authenticate(&BiometricPrompt::new("Unlock", "Cancel"), &grant);
        assert!(matches!(
            task.value(),
            crate::task::AsyncValue::Failed(ServiceError::Denied(_))
        ));
    }

    #[test]
    fn a_push_token_survives_nobody_polling_for_it() {
        // A token missed is an install that silently never receives anything,
        // so it has to wait rather than being dropped on the floor.
        let push = HeadlessPush::new();
        push.push_token("abc123");
        // Several frames go by in which nothing asks.
        assert_eq!(push.take_token().as_deref(), Some("abc123"));
        assert!(push.take_token().is_none(), "delivered once");
    }

    #[test]
    fn a_geofence_crossing_arrives_even_though_nothing_is_watching_yet() {
        // The whole reason to use the OS's monitoring: the crossing happened
        // while the process was dead, so it is delivered *before* the
        // application has re-registered its regions.
        let fences = HeadlessGeofencing::new();
        fences.push_transition("home", GeofenceTransition::Entered);
        assert!(fences.watched().is_empty());
        assert_eq!(
            fences.take_transition(),
            Some(("home".to_owned(), GeofenceTransition::Entered))
        );
    }

    #[test]
    fn the_same_region_is_not_watched_twice() {
        let services = granted();
        let gate = services.gate::<dyn Geofencing>().expect("registered");
        let (_, grant) = gate.granted().expect("granted");
        let fences = HeadlessGeofencing::new();

        let fence = Geofence {
            id: "home".into(),
            latitude: 51.5,
            longitude: -0.12,
            radius: 100.0,
        };
        fences.add(fence.clone(), &grant).expect("free");
        assert!(
            fences.add(fence, &grant).is_err(),
            "adding twice would double every crossing"
        );
        fences.remove("home").expect("watched");
        assert!(
            fences.remove("home").is_err(),
            "and removing twice is a bug"
        );
    }

    #[test]
    fn a_model_that_was_never_loaded_is_an_error_rather_than_an_empty_answer() {
        // Returning `[]` would look exactly like a model that found nothing,
        // which is the failure that gets shipped.
        let inference = ScriptedInference::new();
        assert!(inference.run("classifier", &[1, 2, 3]).is_err());

        inference.load("classifier", &[0xAA]).expect("bytes");
        inference.set_answer(
            "classifier",
            vec![Prediction {
                label: "cat".into(),
                confidence: 0.94,
            }],
        );
        let out = inference.run("classifier", &[1, 2, 3]).expect("loaded");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "cat");
    }

    #[test]
    fn every_capability_arrives_through_the_ordinary_seam() {
        let mut services = Services::new();
        provide_headless(&mut services);

        assert!(services.get::<dyn Haptics>().is_some());
        assert!(services.get::<dyn Biometrics>().is_some());
        assert!(services.get::<dyn PushNotifications>().is_some());
        assert!(services.get::<dyn BarcodeScanner>().is_some());
        assert!(services.get::<dyn BluetoothLe>().is_some());
        assert!(services.get::<dyn Geofencing>().is_some());
        assert!(services.get::<dyn Microphone>().is_some());
        assert!(services.get::<dyn Inference>().is_some());
    }

    #[test]
    fn a_managed_device_withholds_every_capability_without_offering_settings() {
        // The state a whole class of users is in on every launch — a school
        // iPad, a work profile — and the one an application is least likely to
        // have been run in. `AlwaysRestricted` exists so that running the whole
        // application in it is one line rather than a per-permission script.
        //
        // The assertion that matters is the last one: this looks exactly like a
        // refusal until you ask what the remedy is, and offering Settings here
        // sends the user to a switch they are not allowed to touch.
        let mut services = Services::new();
        provide_headless(&mut services);
        services.provide::<dyn crate::permission::Permissions>(Rc::new(AlwaysRestricted));

        for state in [
            services
                .gate::<dyn BarcodeScanner>()
                .expect("registered")
                .state(),
            services
                .gate::<dyn Microphone>()
                .expect("registered")
                .state(),
            services
                .gate::<dyn Geofencing>()
                .expect("registered")
                .state(),
            services
                .gate::<dyn PushNotifications>()
                .expect("registered")
                .state(),
        ] {
            assert_eq!(state, PermissionState::Restricted);
            assert!(state.is_final(), "policy is not put to the user");
            assert!(
                !state.needs_settings(),
                "and there is nothing in Settings that would help"
            );
        }
    }

    #[test]
    fn a_camera_denied_last_week_is_never_prompted_for_again() {
        // "The user denied this last week and the OS will not show the prompt
        // again" — the state a design that cannot express it has not finished
        // modelling, staged on a real capability rather than on a stand-in.
        //
        // Second launch, so the refusal is already remembered rather than being
        // reached by asking.
        let permissions = Rc::new(
            ScriptedPermissions::new()
                .already(CAMERA, PermissionState::Blocked)
                .will_answer(CAMERA, PermissionState::Granted),
        );
        let mut services = Services::new();
        provide_headless(&mut services);
        services.provide::<dyn crate::permission::Permissions>(permissions.clone());

        let gate = services.gate::<dyn BarcodeScanner>().expect("registered");
        let refused = Rc::new(Cell::new(None));
        let seen = Rc::clone(&refused);
        gate.access(
            |_, _| unreachable!("the camera was refused for good"),
            move |state| seen.set(Some(state)),
        );

        assert_eq!(refused.get(), Some(PermissionState::Blocked));
        assert_eq!(
            permissions.times_asked(CAMERA),
            0,
            "the scripted user would have said yes — and is never asked, \
             because the OS would show no dialog to say it in. A gate that \
             asked anyway would pass this test only by being told a lie about \
             what the platform does."
        );
        assert!(
            refused.get().expect("refused").needs_settings(),
            "this is the one refusal that routes to Settings"
        );
    }

    #[test]
    fn the_microphone_walks_the_whole_refusal_sequence() {
        // Undetermined, denied, blocked — the Android sequence, on a capability
        // rather than in the abstract, because the thing being checked is that
        // an application holding one gate across all three sees three different
        // answers without doing anything differently.
        let permissions = Rc::new(
            ScriptedPermissions::new()
                .will_answer(MICROPHONE, PermissionState::Denied)
                .will_answer(MICROPHONE, PermissionState::Blocked),
        );
        let mut services = Services::new();
        provide_headless(&mut services);
        services.provide::<dyn crate::permission::Permissions>(permissions.clone());
        let gate = services.gate::<dyn Microphone>().expect("registered");

        assert_eq!(gate.state(), PermissionState::Undetermined);
        assert!(gate.state().can_ask(), "nobody has been asked yet");

        let seen = Rc::new(RefCell::new(Vec::new()));
        for _ in 0..3 {
            let sink = Rc::clone(&seen);
            gate.access(
                |_, _| unreachable!("the scripted user refused twice"),
                move |state| sink.borrow_mut().push(state),
            );
        }

        assert_eq!(
            *seen.borrow(),
            vec![
                PermissionState::Denied,
                PermissionState::Blocked,
                PermissionState::Blocked
            ]
        );
        assert_eq!(
            permissions.times_asked(MICROPHONE),
            2,
            "asked while it was still a question, and not once after"
        );
    }

    #[test]
    fn revoking_the_microphone_mid_session_stops_the_next_start() {
        // Android lets the user revoke from Settings while the process runs, so
        // a grant is scoped to the check that produced it rather than to the
        // gate. `Grant` not being storable is what makes this the *only*
        // reachable shape — the compile-fail doctest in `permission.rs` is the
        // proof that the tempting alternative does not compile.
        let permissions =
            Rc::new(ScriptedPermissions::new().already(MICROPHONE, PermissionState::Granted));
        let mut services = Services::new();
        provide_headless(&mut services);
        services.provide::<dyn crate::permission::Permissions>(permissions.clone());

        let gate = services.gate::<dyn Microphone>().expect("registered");
        let mic = HeadlessMicrophone::new();
        {
            let (_, grant) = gate.granted().expect("granted at launch");
            mic.start(&grant).expect("headless starts");
        }
        assert!(mic.is_running());

        permissions.set_state(MICROPHONE, PermissionState::Blocked);
        assert!(
            gate.granted().is_none(),
            "the same gate refuses immediately — nothing was cached, and the \
             grant from a moment ago could not have been kept to get past this"
        );
    }

    #[test]
    fn the_permissions_are_all_different() {
        // A copy-paste in the `Guarded` impls would make two capabilities share
        // one permission, and the symptom is an application that gets the camera
        // by asking for the microphone.
        let names = [
            <dyn Biometrics as Guarded>::PERMISSION,
            <dyn PushNotifications as Guarded>::PERMISSION,
            <dyn BarcodeScanner as Guarded>::PERMISSION,
            <dyn BluetoothLe as Guarded>::PERMISSION,
            <dyn Geofencing as Guarded>::PERMISSION,
            <dyn Microphone as Guarded>::PERMISSION,
        ];
        let mut sorted = names.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "two capabilities share a name");
    }
}
