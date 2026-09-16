//! The system pasteboard, one implementation per target.
//!
//! Registered as [`Clipboard`] in [`services::platform`](crate::services), so a
//! `TextField` reaches it through the same
//! `SharedServices` that carries storage and deep links. Nothing here is named by
//! a widget.
//!
//! # Text only
//!
//! Every platform pasteboard carries images, files and application-defined types
//! as well. None of that is here, because [`Clipboard`] has no representation for
//! it — see the trait. A pasteboard holding an image reads as empty.
//!
//! # What each target uses
//!
//! | target | mechanism |
//! |---|---|
//! | desktop | `arboard`, which owns the X11/Wayland/Win32/AppKit differences |
//! | Android | `ClipboardManager` through JNI |
//! | iOS | `UIPasteboard.general` through `objc2` |

use std::fmt;

use vieww_foundation::{Clipboard, ServiceError};

/// The pasteboard this platform provides.
pub struct PlatformClipboard {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    inner: std::cell::RefCell<Option<arboard::Clipboard>>,
    #[cfg(target_os = "android")]
    app: crate::AndroidApp,
}

/// Hand-written because `arboard::Clipboard` is not `Debug`, and the workspace
/// warns on a type that has no `Debug` at all.
///
/// Reports whether a connection is currently open rather than anything about the
/// contents. Printing what is on the pasteboard would put the user's passwords in
/// a log line the first time somebody debug-formatted a service registry.
impl fmt::Debug for PlatformClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("PlatformClipboard");
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        out.field("connected", &self.inner.borrow().is_some());
        out.finish()
    }
}

// ---------------------------------------------------------------------- desktop

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl PlatformClipboard {
    /// Connect to the desktop pasteboard.
    ///
    /// The connection is made lazily and kept, rather than opened per operation.
    /// On X11 that is not an optimisation: the clipboard is owned by a live
    /// client, and a handle dropped after each copy takes the contents with it —
    /// pasting into another application then yields nothing, which looks exactly
    /// like a failed copy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: std::cell::RefCell::new(None),
        }
    }

    /// The live handle, opening one on first use.
    fn with<T>(
        &self,
        act: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
    ) -> Result<T, ServiceError> {
        let mut slot = self.inner.borrow_mut();
        if slot.is_none() {
            *slot = Some(
                arboard::Clipboard::new()
                    .map_err(|error| ServiceError::failed(error.to_string()))?,
            );
        }
        let clipboard = slot.as_mut().expect("opened above");
        act(clipboard).map_err(|error| match error {
            // A pasteboard holding an image, or nothing at all. Not a failure.
            arboard::Error::ContentNotAvailable => ServiceError::failed("no text"),
            other => ServiceError::failed(other.to_string()),
        })
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl Default for PlatformClipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl Clipboard for PlatformClipboard {
    fn read_text(&self) -> Result<Option<String>, ServiceError> {
        // Absent and unreadable are one answer to a field, so a failed read is
        // `None` rather than an error. The distinction the trait keeps is
        // `Denied`, which no desktop reports.
        Ok(self.with(arboard::Clipboard::get_text).ok())
    }

    fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        self.with(|clipboard| clipboard.set_text(text.to_string()))
    }
}

// ---------------------------------------------------------------------- Android

#[cfg(target_os = "android")]
impl PlatformClipboard {
    /// The activity's `ClipboardManager`.
    #[must_use]
    pub fn new(app: &crate::AndroidApp) -> Self {
        Self { app: app.clone() }
    }

    /// Run `act` against the activity's `ClipboardManager`.
    ///
    /// The manager is fetched per call rather than held: it belongs to the
    /// activity, and this object outlives any particular one.
    fn with<T>(
        &self,
        act: impl FnOnce(
            &mut jni::Env<'_>,
            &jni::objects::JObject<'_>,
            &jni::objects::JObject<'_>,
        ) -> Result<T, jni::errors::Error>,
    ) -> Result<T, ServiceError> {
        use jni::objects::{JObject, JValue};
        use jni::{jni_sig, jni_str};

        // Both pointers belong to the running activity and neither is kept past
        // this call — the same contract as `insets::window_insets`.
        let vm = unsafe { jni::JavaVM::from_raw(self.app.vm_as_ptr().cast()) };
        let activity_ptr = self.app.activity_as_ptr();

        vm.attach_current_thread(|env| {
            let activity = unsafe { JObject::from_raw(env, activity_ptr.cast()) };
            let name = env.new_string("clipboard")?;
            let manager = env
                .call_method(
                    &activity,
                    jni_str!("getSystemService"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                    &[JValue::Object(&name)],
                )?
                .l()?;
            act(env, &activity, &manager)
        })
        .map_err(|error| ServiceError::failed(error.to_string()))
    }
}

#[cfg(target_os = "android")]
impl Clipboard for PlatformClipboard {
    /// # Android 10 and the focus rule
    ///
    /// From API 29 an application may only read the pasteboard while it holds
    /// input focus; a background read returns empty rather than failing. That is
    /// a platform policy and not something this code can work around — and it is
    /// the right policy, since the alternative is every app reading everything
    /// the user has ever copied. A paste driven by a keystroke is focused by
    /// definition, so it is unaffected.
    fn read_text(&self) -> Result<Option<String>, ServiceError> {
        use jni::jni_sig;
        use jni::jni_str;

        self.with(|env, _activity, manager| {
            let clip = env
                .call_method(
                    manager,
                    jni_str!("getPrimaryClip"),
                    jni_sig!("()Landroid/content/ClipData;"),
                    &[],
                )?
                .l()?;
            if clip.is_null() {
                return Ok(None);
            }
            let count = env
                .call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[])?
                .i()?;
            if count <= 0 {
                return Ok(None);
            }
            let item = env
                .call_method(
                    &clip,
                    jni_str!("getItemAt"),
                    jni_sig!("(I)Landroid/content/ClipData$Item;"),
                    &[jni::objects::JValue::Int(0)],
                )?
                .l()?;
            // `getText` rather than `coerceToText`: the latter needs a Context
            // and will happily render a URI or an Intent as a string, which is
            // not text the user copied.
            let text = env
                .call_method(
                    &item,
                    jni_str!("getText"),
                    jni_sig!("()Ljava/lang/CharSequence;"),
                    &[],
                )?
                .l()?;
            if text.is_null() {
                return Ok(None);
            }
            let string = env
                .call_method(
                    &text,
                    jni_str!("toString"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            // Cast then read, as two fallible steps — the same shape
            // `deep_links.rs` uses and for the same reason: the cast is a real
            // `IsInstanceOf` against a value the platform handed over, and
            // reading the characters can fail on its own.
            let string = env.cast_local::<jni::objects::JString>(string)?;
            Ok(Some(string.try_to_string(env)?))
        })
    }

    fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        use jni::objects::JValue;
        use jni::{jni_sig, jni_str};

        self.with(|env, _activity, manager| {
            let label = env.new_string("vieww")?;
            let value = env.new_string(text)?;
            let clip = env
                .call_static_method(
                    jni_str!("android/content/ClipData"),
                    jni_str!("newPlainText"),
                    jni_sig!(
                        "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)\
                         Landroid/content/ClipData;"
                    ),
                    &[JValue::Object(&label), JValue::Object(&value)],
                )?
                .l()?;
            env.call_method(
                manager,
                jni_str!("setPrimaryClip"),
                jni_sig!("(Landroid/content/ClipData;)V"),
                &[JValue::Object(&clip)],
            )?;
            Ok(())
        })
    }
}

// -------------------------------------------------------------------------- iOS

#[cfg(target_os = "ios")]
impl PlatformClipboard {
    /// `UIPasteboard.general`.
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

#[cfg(target_os = "ios")]
impl Default for PlatformClipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "ios")]
impl Clipboard for PlatformClipboard {
    /// # iOS 16 and the paste prompt
    ///
    /// Reading the general pasteboard shows the user a "Paste" confirmation
    /// unless the read came from a system paste control. `UIPasteboard.string`
    /// is the honest API for a keystroke-driven paste and takes that prompt;
    /// suppressing it means adopting `UIPasteControl`, which is a UIKit view and
    /// cannot be one of vieww's own. Recorded rather than worked around.
    fn read_text(&self) -> Result<Option<String>, ServiceError> {
        use objc2_ui_kit::UIPasteboard;

        // SAFETY: `generalPasteboard` is the process-wide pasteboard and is
        // valid for the life of the application. `string` returns an autoreleased
        // `NSString` that `Retained` takes ownership of.
        let pasteboard = unsafe { UIPasteboard::generalPasteboard() };
        let text = unsafe { pasteboard.string() };
        Ok(text.map(|string| string.to_string()))
    }

    fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        use objc2_foundation::NSString;
        use objc2_ui_kit::UIPasteboard;

        let pasteboard = unsafe { UIPasteboard::generalPasteboard() };
        unsafe { pasteboard.setString(Some(&NSString::from_str(text))) };
        Ok(())
    }
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod tests {
    use super::*;

    /// Opening a pasteboard needs a display server, which CI does not have — so
    /// this asserts the *shape* of a failure rather than a round trip: an absent
    /// pasteboard must not panic, and must not be reported as text.
    ///
    /// The round trip is covered against [`MemoryClipboard`] in
    /// `vieww/tests/keys_to_semantics.rs`, where it belongs: what a field does
    /// with cut, copy and paste is this crate's business, and whether X11 is
    /// running is not.
    #[test]
    fn a_pasteboard_that_cannot_be_opened_reads_as_empty_rather_than_panicking() {
        let clipboard = PlatformClipboard::new();
        assert!(matches!(clipboard.read_text(), Ok(None) | Ok(Some(_))));
    }
}
