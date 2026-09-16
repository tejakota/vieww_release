//! Putting a native view inside the widget tree.

use std::collections::HashMap;

use crate::service::ServiceError;
use crate::Rect;

/// A platform view's identity, for as long as it is mounted.
///
/// Allocated by whoever creates the view and handed back on every call about
/// it. A `u64` rather than a string because it is minted, not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlatformViewId(pub u64);

/// What kind of native view to create, and how to set it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformViewSpec {
    /// The name the host application registered a factory under — `"map"`,
    /// `"webview"`, `"ad-slot"`.
    ///
    /// A registry of names rather than a fixed enum, for
    /// [`service`](crate::service)'s reason: a framework that enumerated the
    /// embeddable view types would be a framework you have to fork to embed a
    /// new one.
    pub kind: String,
    /// Creation parameters, as strings.
    ///
    /// Strings because this crosses a language boundary — into Kotlin or Swift
    /// — and a typed payload would need a serialisation format baked into the
    /// framework. The application already has one it likes, and can put JSON in
    /// here if it wants it.
    pub parameters: HashMap<String, String>,
}

impl PlatformViewSpec {
    #[must_use]
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            parameters: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.parameters.insert(key.into(), value.into());
        self
    }
}

/// A live native view. Dropping it destroys the view.
///
/// The same contract as the other subscriptions in this module: the resource
/// ends when the handle does, so unmounting the widget that owns one is enough
/// and there is no dispose call to forget. A leaked native view is far more
/// expensive than a leaked callback — it is a whole embedded browser or map
/// engine still running.
pub struct PlatformViewHandle {
    id: PlatformViewId,
    dispose: Option<Box<dyn FnOnce(PlatformViewId)>>,
}

impl PlatformViewHandle {
    #[must_use]
    pub fn new(id: PlatformViewId, dispose: impl FnOnce(PlatformViewId) + 'static) -> Self {
        Self {
            id,
            dispose: Some(Box::new(dispose)),
        }
    }

    #[must_use]
    pub const fn id(&self) -> PlatformViewId {
        self.id
    }
}

impl std::fmt::Debug for PlatformViewHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformViewHandle")
            .field("id", &self.id)
            .field("live", &self.dispose.is_some())
            .finish()
    }
}

impl Drop for PlatformViewHandle {
    fn drop(&mut self) {
        if let Some(dispose) = self.dispose.take() {
            dispose(self.id);
        }
    }
}

/// Native views embedded in the tree.
///
/// # The honest account of what this costs
///
/// A platform view is not a widget. It is a separate rendering surface owned by
/// the operating system, composited beside vieww's rather than inside it, and
/// everything that follows from that is a real limitation rather than a missing
/// feature:
///
/// - **It does not participate in painting.** A platform view cannot be
///   clipped by an ancestor's rounded corner, cannot be faded by an
///   [`Opacity`](https://docs.rs/vieww-widget) above it, and cannot have
///   another widget drawn over it without the platform's own compositing
///   support. It sits at a rectangle and it draws itself there.
/// - **It has its own pointer handling.** Touches inside its rectangle go to
///   it, not through the gesture arena, so a scroll gesture started inside an
///   embedded map is the map's and a parent list will not get it.
/// - **It is expensive.** Each one is a live native component with its own
///   memory and, for a browser or a map, its own processes.
///
/// Every framework's version of this is well known for exactly these problems, and they
/// are not bugs in the implementation — they follow from what embedding is.
/// Saying so in the declaration is better than a caller finding out from a
/// screenshot.
///
/// # No permission
///
/// Embedding a view needs none; whatever the view then does asks for its own.
/// This is the one capability in the module that is not
/// [`Guarded`](crate::permission::Guarded), and it is deliberate rather than an
/// omission.
pub trait PlatformViews: 'static {
    /// The `kind` values this platform has factories registered for.
    #[must_use]
    fn registered(&self) -> Vec<String>;

    /// Create one, positioned at `bounds` in logical pixels from the top-left
    /// of the window.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where embedding is not possible at all,
    /// and [`ServiceError::Failed`] where no factory is registered for
    /// [`kind`](PlatformViewSpec::kind).
    fn create(
        &self,
        spec: &PlatformViewSpec,
        bounds: Rect,
    ) -> Result<PlatformViewHandle, ServiceError>;

    /// Move or resize a view, after layout has placed it somewhere new.
    ///
    /// Called every frame the rectangle changes, so implementations should make
    /// an unchanged rectangle cheap — this is on the path of any scroll that
    /// contains one.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Failed`] if the view is already gone.
    fn set_bounds(&self, id: PlatformViewId, bounds: Rect) -> Result<(), ServiceError>;

    /// Send the view a message, and get one back.
    ///
    /// The escape hatch, and the reason it is a string pair rather than
    /// anything richer is [`PlatformViewSpec::parameters`]'s: this is a
    /// language boundary, and the framework does not get to choose the
    /// application's serialisation format.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unsupported`] where the view accepts no messages.
    fn send(&self, id: PlatformViewId, message: &str) -> Result<String, ServiceError>;
}

/// [`PlatformViews`] where embedding is not available.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoPlatformViews;

impl PlatformViews for NoPlatformViews {
    fn registered(&self) -> Vec<String> {
        Vec::new()
    }

    fn create(
        &self,
        _spec: &PlatformViewSpec,
        _bounds: Rect,
    ) -> Result<PlatformViewHandle, ServiceError> {
        Err(ServiceError::unsupported("PlatformViews"))
    }

    fn set_bounds(&self, _id: PlatformViewId, _bounds: Rect) -> Result<(), ServiceError> {
        Err(ServiceError::unsupported("PlatformViews"))
    }

    fn send(&self, _id: PlatformViewId, _message: &str) -> Result<String, ServiceError> {
        Err(ServiceError::unsupported("PlatformViews"))
    }
}
