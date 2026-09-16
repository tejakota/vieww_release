//! Platform services: the OS integration layer.
//!
//! # Why traits and not direct calls
//!
//! The framework must build and *test* on any platform, including in CI
//! with no OS services available. A direct call to `notify()` would fail
//! at link time on a headless box. A trait object resolves at runtime:
//! the desktop provides a real implementation, the test harness provides
//! a recorder, and the same widget code works in both.
//!
//! This is the *service locator* pattern rather than compile-time DI
//! because widgets need services deep in the tree and threading a
//! `Services` parameter through every constructor is the alternative
//! nobody enjoys.
//!
//! # The service registry
//!
//! One `Services` instance per application, provided by the platform
//! layer. Widgets fetch services by trait object:
//!
//! ```ignore
//! let clipboard = ctx.services::<dyn ClipboardService>();
//! clipboard.set_text("hello");
//! ```
//!
//! # What is here
//!
//! The traits for clipboard, notifications, sharing, deep links, URLs,
//! and storage. Each is small — the hard part is the platform FFI, not
//! the API — and each has a `RecordingService` implementation for tests.

pub mod desktop;

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A service that can be put on the clipboard or shared.
#[derive(Debug, Clone)]
pub enum SharedContent {
    Text(String),
    Url(String),
    /// Path to a file the platform can share.
    FilePath(std::path::PathBuf),
    /// Multiple files.
    FilePaths(Vec<std::path::PathBuf>),
}

/// Clipboard read/write.
pub trait ClipboardService {
    fn set_text(&self, text: &str);
    fn text(&self) -> Option<String>;
    fn set_content(&self, content: SharedContent);
    fn content(&self) -> Option<SharedContent>;
    /// `true` if the platform can report clipboard changes.
    fn supports_change_events(&self) -> bool {
        false
    }
    /// Register a clipboard-change callback. Returns `None` if unsupported.
    fn on_change(&self, _callback: Box<dyn Fn()>) -> Option<Box<dyn Fn()>> {
        None
    }
}

/// System notifications.
pub trait NotificationService {
    /// Request permission to show notifications. Async on some platforms;
    /// the callback fires when the user answers.
    fn request_permission(&self, callback: Box<dyn Fn(bool)>);
    fn show(&self, notification: Notification);
    fn cancel(&self, id: u64);
    fn clear_all(&self);
}

/// One notification.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u64,
    pub title: String,
    pub body: String,
    /// An icon name or path, platform-specific.
    pub icon: Option<String>,
    /// A deep link to open when tapped.
    pub action_url: Option<String>,
}

/// The OS share sheet.
pub trait ShareService {
    fn share(&self, content: SharedContent, callback: Box<dyn Fn(ShareResult)>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareResult {
    Shared,
    Cancelled,
    Failed,
}

/// Deep links: your app's URL scheme.
pub trait DeepLinkService {
    /// Register a URL scheme the platform should route to this app.
    fn register_scheme(&self, scheme: &str);
    /// The deep link that launched the app, if any.
    fn launch_link(&self) -> Option<String>;
    /// Subscribe to links while running.
    fn on_link(&self, callback: Box<dyn Fn(&str)>);
}

/// Opening URLs in the system browser / default app.
pub trait UrlService {
    /// Open `url` externally. Returns `Err` if nothing can handle it.
    fn open(&self, url: &str) -> Result<(), UrlOpenError>;
    fn can_open(&self, url: &str) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlOpenError {
    NoHandler,
    PermissionDenied,
    InvalidUrl,
}

/// Persistent storage, key-value style.
///
/// This already exists as `vieww_foundation::Storage`; the service trait
/// wraps it so widgets can fetch it through the same locator.
pub trait StorageService {
    fn set(&self, key: &str, value: &[u8]);
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn delete(&self, key: &str);
    fn keys(&self) -> Vec<String>;
}

/// Secure storage for secrets (tokens, passwords).
///
/// On desktop this is the OS keychain; on mobile, the Keystore/Keychain.
/// Never a plain file.
pub trait SecureStorageService {
    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecureStorageError>;
    fn get_secret(&self, key: &str) -> Option<String>;
    fn delete_secret(&self, key: &str);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecureStorageError {
    KeychainUnavailable,
    KeyNotFound,
    AccessDenied,
}

/// The service locator.
///
/// One instance per application. Services register themselves at startup;
/// widgets fetch them by type.
#[derive(Default)]
pub struct Services {
    services: RefCell<HashMap<TypeId, Rc<dyn Any>>>,
}

impl Services {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a service. The trait object is boxed behind `Rc<dyn Any>`
    /// and fetched by its `TypeId`.
    pub fn register<S: 'static>(&self, service: Rc<S>) {
        self.services
            .borrow_mut()
            .insert(TypeId::of::<S>(), service);
    }

    /// Fetch a service by type. `None` if not registered — a missing
    /// service is not an error, it is a platform that does not have it,
    /// and the caller should degrade.
    #[must_use]
    pub fn get<S: 'static>(&self) -> Option<Rc<S>> {
        self.services
            .borrow()
            .get(&TypeId::of::<S>())
            .and_then(|any| any.clone().downcast::<S>().ok())
    }
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Services({} registered)", self.services.borrow().len())
    }
}

/// Test implementations: record everything, do nothing.
///
/// For widget tests and headless CI. Every method records the call so a
/// test can assert "the share button was pressed" without a share sheet.
pub mod recording {
    use super::*;

    /// A clipboard that records writes.
    #[derive(Debug)]
    pub struct RecordingClipboard {
        pub writes: RefCell<Vec<SharedContent>>,
    }

    impl RecordingClipboard {
        #[must_use]
        pub fn new() -> Self {
            Self {
                writes: RefCell::new(Vec::new()),
            }
        }
    }

    impl Default for RecordingClipboard {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ClipboardService for RecordingClipboard {
        fn set_text(&self, text: &str) {
            self.writes
                .borrow_mut()
                .push(SharedContent::Text(text.to_owned()));
        }

        fn text(&self) -> Option<String> {
            self.writes.borrow().last().and_then(|c| match c {
                SharedContent::Text(t) => Some(t.clone()),
                _ => None,
            })
        }

        fn set_content(&self, content: SharedContent) {
            self.writes.borrow_mut().push(content);
        }

        fn content(&self) -> Option<SharedContent> {
            self.writes.borrow().last().cloned()
        }
    }

    /// A share service that records the content and calls back `Shared`.
    #[derive(Debug)]
    pub struct RecordingShare {
        pub shared: RefCell<Vec<SharedContent>>,
    }

    impl RecordingShare {
        #[must_use]
        pub fn new() -> Self {
            Self {
                shared: RefCell::new(Vec::new()),
            }
        }
    }

    impl Default for RecordingShare {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ShareService for RecordingShare {
        fn share(&self, content: SharedContent, callback: Box<dyn Fn(ShareResult)>) {
            self.shared.borrow_mut().push(content);
            callback(ShareResult::Shared);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recording::{RecordingClipboard, RecordingShare};

    #[test]
    fn services_fetch_by_type() {
        let services = Services::new();
        services.register(Rc::new(RecordingClipboard::new()));

        let clipboard: Option<Rc<RecordingClipboard>> = services.get();
        assert!(clipboard.is_some());

        let missing: Option<Rc<RecordingShare>> = services.get();
        assert!(missing.is_none(), "not registered");
    }

    #[test]
    fn recording_clipboard_records_writes() {
        let clip = RecordingClipboard::new();
        clip.set_text("hello");
        assert_eq!(clip.text().as_deref(), Some("hello"));

        clip.set_content(SharedContent::Url("https://example.com".into()));
        assert_eq!(clip.writes.borrow().len(), 2);
    }
}
