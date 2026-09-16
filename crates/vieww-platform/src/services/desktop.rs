//! Desktop implementations of the platform services.
//!
//! # What is actually implementable
//!
//! On desktop (Windows, macOS, Linux), the platform services map to:
//!
//! | Service | Windows | macOS | Linux |
//! |---|---|---|---|
//! | Clipboard | Win32 clipboard | NSPasteboard | X11/Wayland clipboard |
//! | Open URL | ShellExecute | NSWorkspace.open | xdg-open |
//! | Notifications | Win32 toast | NSUserNotification | notify-send |
//! | Share | N/A (no share sheet) | NSSharingService | N/A |
//! | Storage | AppData folder | ~/Library/Application Support | XDG_DATA_HOME |
//!
//! The clipboard and URL opening are the most useful and the most
//! portable. Notifications vary. Sharing has no desktop equivalent
//! (there is no system share sheet).
//!
//! # This implementation
//!
//! Clipboard via `arboard` (a cross-platform Rust clipboard crate) and
//! URL opening via `open` (a crate that shells out to the right command).
//! Both are optional dependencies — the service degrades to a no-op
//! without them.

use std::rc::Rc;

use crate::services::{ClipboardService, Services, SharedContent, UrlOpenError, UrlService};

/// A clipboard backed by `arboard`.
///
/// `arboard` handles the platform differences: Win32 on Windows,
/// NSPasteboard on macOS, X11/Wayland on Linux.
#[derive(Debug)]
pub struct DesktopClipboard {
    #[cfg(feature = "arboard")]
    inner: std::cell::RefCell<Option<arboard::Clipboard>>,
}

impl DesktopClipboard {
    #[must_use]
    pub fn new() -> Option<Self> {
        #[cfg(feature = "arboard")]
        {
            match arboard::Clipboard::new() {
                Ok(clip) => Some(Self {
                    inner: std::cell::RefCell::new(Some(clip)),
                }),
                Err(_) => None,
            }
        }
        #[cfg(not(feature = "arboard"))]
        None
    }
}

impl Default for DesktopClipboard {
    fn default() -> Self {
        Self::new().unwrap_or(Self::fallback())
    }
}

impl DesktopClipboard {
    #[cfg(not(feature = "arboard"))]
    fn fallback() -> Self {
        Self {
            #[cfg(feature = "arboard")]
            inner: std::cell::RefCell::new(None),
        }
    }
}

impl ClipboardService for DesktopClipboard {
    fn set_text(&self, text: &str) {
        // Referenced so the parameter is not dead in the build without
        // `arboard`, where the body below compiles to nothing.
        let _ = text;
        #[cfg(feature = "arboard")]
        if let Ok(mut clip) = self.inner.try_borrow_mut() {
            if let Some(clipboard) = clip.as_mut() {
                let _ = clipboard.set_text(text.to_owned());
            }
        }
    }

    fn text(&self) -> Option<String> {
        #[cfg(feature = "arboard")]
        if let Ok(mut clip) = self.inner.try_borrow_mut() {
            if let Some(clipboard) = clip.as_mut() {
                return clipboard.get_text().ok();
            }
        }
        None
    }

    fn set_content(&self, content: SharedContent) {
        // Only text is portable to the clipboard.
        if let SharedContent::Text(t) = content {
            self.set_text(&t);
        }
    }

    fn content(&self) -> Option<SharedContent> {
        self.text().map(SharedContent::Text)
    }
}

/// A URL opener backed by the `open` crate (shells out to the right
/// platform command).
#[derive(Debug)]
pub struct DesktopUrlOpener;

impl UrlService for DesktopUrlOpener {
    fn open(&self, url: &str) -> Result<(), UrlOpenError> {
        #[cfg(feature = "open")]
        {
            if url::Url::parse(url).is_err() {
                return Err(UrlOpenError::InvalidUrl);
            }
            open::that(url).map_err(|_| UrlOpenError::NoHandler)
        }
        #[cfg(not(feature = "open"))]
        {
            let _ = url;
            Err(UrlOpenError::NoHandler)
        }
    }

    fn can_open(&self, url: &str) -> bool {
        #[cfg(feature = "open")]
        {
            url::Url::parse(url).is_ok()
        }
        #[cfg(not(feature = "open"))]
        {
            let _ = url;
            false
        }
    }
}

/// Register the desktop services.
///
/// Call this at application startup:
///
/// ```ignore
/// let services = Services::new();
/// vieww_platform::services::desktop::register(&services);
/// ```
pub fn register(services: &Services) {
    if let Some(clipboard) = DesktopClipboard::new() {
        services.register(Rc::new(clipboard));
    }
    services.register(Rc::new(DesktopUrlOpener));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_services_register() {
        let services = Services::new();
        register(&services);

        // The URL opener should always be present.
        let opener: Option<Rc<DesktopUrlOpener>> = services.get();
        assert!(opener.is_some(), "URL opener is registered");
    }
}
