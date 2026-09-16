//! Platform service traits — the seam between the framework and the OS.
//!
//! # What belongs here
//!
//! Anything the framework needs from the operating system that is not
//! rendering, windowing or input: the clipboard, notifications, the share
//! sheet, deep links, opening URLs, and key/value storage.
//!
//! # What deliberately does not
//!
//! Any actual platform code. This crate is traits, a registry, and a
//! recording implementation for tests — no FFI, no `#[cfg(target_os)]`, and
//! no dependencies. The real implementations live in the per-platform crates
//! (`vieww-platform-winit` and the mobile backends) and are registered at
//! startup.
//!
//! That split is what lets the widget layer call `ctx.services()` in a
//! headless CI container: the test harness registers recorders, the same
//! widget code runs, and the assertions are about what was *requested* rather
//! than what the OS did with it.
//!
//! See [`services`] for the traits and the registry.

pub mod services;

pub use services::{
    recording, ClipboardService, DeepLinkService, Notification, NotificationService,
    SecureStorageError, SecureStorageService, Services, ShareResult, ShareService, SharedContent,
    StorageService, UrlOpenError, UrlService,
};
