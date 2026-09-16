//! iOS's answer to "where is the system furniture", through UIKit.
//!
//! The counterpart to [`crate::insets`]'s JNI, and deliberately the same shape:
//! one function that asks the platform for a [`SystemInsets`], and nothing else.
//! What is done with the numbers is shared, tested on every machine, and lives
//! next door.
//!
//! # Status: written, not run
//!
//! **No frame has reached an iPhone**, so nothing in this file has executed
//! against a real UIKit. It cross-compiles for `aarch64-apple-ios` and links for
//! `aarch64-apple-ios-sim` in CI, which establishes that the selectors resolve
//! and the types line up — and establishes nothing at all about the numbers
//! coming back. Every design decision below is written down with its reasoning
//! precisely because the reasoning is currently all there is to check.
//!
//! # Why `objc2` rather than hand-rolled `objc_msgSend`
//!
//! `-[UIView safeAreaInsets]` returns a 32-byte struct, and how a struct comes
//! back from a message send is ABI-dependent: four doubles are returned in
//! registers on arm64 and through a hidden pointer on x86_64, where the symbol
//! is `objc_msgSend_stret` instead. Declaring it wrong does not fail to compile.
//! It silently reads garbage on the simulator architecture nobody tested on.
//!
//! `objc2` already encodes that rule, `winit` already pulls it in for this
//! target so it costs nothing new in the tree, and DESIGN §9's "do not attempt
//! from scratch" list is exactly about this kind of thing.

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::sel;
use objc2_ui_kit::{UIKeyboardLayoutGuide, UIView};
use vieww_foundation::EdgeInsets;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::insets::SystemInsets;

/// Where the notch, the home indicator and the keyboard are, in points.
///
/// `None` when the window is not a UIKit window or has no view yet — the same
/// contract as the Android side, so a caller can tell "there is no furniture"
/// from "we could not ask".
///
/// # No scale conversion
///
/// UIKit measures in points and vieww measures in logical pixels, and on iOS
/// those are the same unit. The `/ scale` the Android path needs is *not* a
/// missing step here; adding it would divide the insets by three on a modern
/// phone.
pub(crate) fn window_insets(window: &winit::window::Window) -> Option<SystemInsets> {
    let view = ui_view(window)?;

    let safe = view.safeAreaInsets();
    Some(SystemInsets {
        safe_area: EdgeInsets {
            left: safe.left as f32,
            top: safe.top as f32,
            right: safe.right as f32,
            bottom: safe.bottom as f32,
        },
        keyboard: keyboard_inset(view),
    })
}

/// The `UIView` behind `window`, if there is one.
///
/// Taken through `raw-window-handle` rather than `winit::platform::ios`, which
/// has no accessor for it — and the handle is the portable seam every GPU
/// backend already goes through, so it is a route that is exercised whether or
/// not this function is.
fn ui_view(window: &winit::window::Window) -> Option<&UIView> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::UiKit(uikit) = handle.as_raw() else {
        return None;
    };

    // SAFETY: the pointer comes from `raw-window-handle`, which documents it as
    // a live `UIView` owned by the window — and the window outlives the borrow,
    // because `handle` borrows it. `UIView` is `MainThreadOnly` and this is
    // called from the event loop, which `winit` runs on the main thread on iOS;
    // that is the same guarantee every other UIKit call in this crate rests on.
    Some(unsafe { uikit.ui_view.cast::<UIView>().as_ref() })
}

/// How much of the bottom of `view` the soft keyboard is covering.
///
/// # Why `keyboardLayoutGuide` rather than the keyboard notifications
///
/// The canonical way to track the keyboard is to observe
/// `UIKeyboardWillChangeFrameNotification` and read
/// `UIKeyboardFrameEndUserInfoKey`. It is also a *push* model: it needs an
/// observer object, a block, a retain cycle to think about, and somewhere to
/// put the value until the next frame reads it.
///
/// Everything else in this bridge is *pull* — [`crate::app`] asks for the
/// metrics it needs on the frames it needs them, which is what
/// [`crate::insets::InsetWatch`] is for. `UIKeyboardLayoutGuide` is the pull
/// version of the same fact: a guide, automatically attached to every view,
/// whose frame is where the keyboard is. Reading it costs two message sends and
/// fits the existing shape exactly.
///
/// If it turns out not to resolve without an Auto Layout constraint referencing
/// it, the notification observer is the fallback, and it is a bigger change than
/// it looks — see the module header on what is and is not established here.
///
/// # The guard, and the failure it exists to stop
///
/// A guide that has never been laid out reports `CGRectZero`. Subtracting that
/// from the bottom of the view yields *the entire view height* as a keyboard
/// inset — a screen that is suddenly zero pixels tall, from a keyboard that is
/// not up. So the frame is only believed when it actually reaches the bottom
/// edge of the view, which a real keyboard always does and `CGRectZero` never
/// does on a view taller than nothing.
fn keyboard_inset(view: &UIView) -> EdgeInsets {
    // iOS 15. Below it the selector does not exist, and sending it anyway is an
    // unrecognised-selector exception, which on iOS is a crash rather than an
    // error — this is not a place to find out at runtime.
    if !view.respondsToSelector(sel!(keyboardLayoutGuide)) {
        return EdgeInsets::ZERO;
    }

    // SAFETY: `respondsToSelector:` has just confirmed the selector, and the
    // guide is a `UILayoutGuide` whose `layoutFrame` is a plain property read.
    let guide: Retained<UIKeyboardLayoutGuide> = unsafe { view.keyboardLayoutGuide() };
    let keyboard = unsafe { guide.layoutFrame() };
    let bounds = view.bounds();

    let view_bottom = bounds.origin.y + bounds.size.height;
    let keyboard_bottom = keyboard.origin.y + keyboard.size.height;

    // A point of slack, because these are two independently computed floats and
    // an exact comparison between them is a coin toss.
    let reaches_the_bottom = keyboard.size.height > 0.0 && keyboard_bottom >= view_bottom - 1.0;
    if !reaches_the_bottom {
        return EdgeInsets::ZERO;
    }

    let covered = (view_bottom - keyboard.origin.y).clamp(0.0, bounds.size.height);
    EdgeInsets::only(0.0, 0.0, 0.0, covered as f32)
}
