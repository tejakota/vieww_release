//! What the surface is like, as opposed to what is drawn on it.
//!
//! Size, pixel density, and the parts of the screen that belong to the system
//! rather than to the application. All of it comes from the platform and none of
//! it can be computed, which is why it is a value handed down rather than
//! something a widget asks for.
//!
//! # Two kinds of obstruction, and why the difference matters
//!
//! A notch and a soft keyboard both cover part of the window, and the right
//! response to each is the opposite of the other. Content must be *inset* away
//! from a notch permanently. Content must be allowed to *scroll out from under*
//! a keyboard, and re-occupy the space when it goes.
//!
//! Collapsing them into one number is the bug where the keyboard opening pushes
//! a list into a shorter box and the item being typed into scrolls away.

use crate::{EdgeInsets, Size};

/// The surface's dimensions and the system furniture on it.
///
/// Published to the whole tree by the platform bridge, and read through
/// `BuildContext::inherit`. Nothing below the platform layer produces one, and
/// everything that reacts to a phone's shape consumes one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewMetrics {
    /// The whole surface, in logical pixels — including the parts under system
    /// furniture.
    pub size: Size,
    /// Physical pixels per logical pixel.
    ///
    /// Carried for the benefit of anything that needs to align to the pixel
    /// grid — a hairline divider that should be one *device* pixel rather than
    /// one logical one. Layout does not use it: everything above the platform
    /// bridge is in logical pixels by construction.
    pub device_pixel_ratio: f32,
    /// Permanently obscured by hardware or persistent system UI: a status bar,
    /// a notch, a display cutout, a home indicator.
    ///
    /// Content that must stay *readable* is
    /// inset by [`padding`](Self::padding), which accounts for the keyboard too.
    pub safe_area: EdgeInsets,
    /// Temporarily covered by a system overlay — in practice, the soft keyboard.
    ///
    /// Content is not inset by this. A scrollable is expected to scroll out from
    /// under it, which is why it is reported separately from the safe area.
    pub view_insets: EdgeInsets,
}

impl ViewMetrics {
    /// Metrics for a surface with no system furniture on it at all.
    ///
    /// Every desktop window, and the right answer for a test.
    #[must_use]
    pub const fn plain(size: Size) -> Self {
        Self {
            size,
            device_pixel_ratio: 1.0,
            safe_area: EdgeInsets::ZERO,
            view_insets: EdgeInsets::ZERO,
        }
    }

    /// How far content must be inset to stay out of the system's way.
    ///
    /// The safe area, reduced on any side the keyboard already displaces —
    /// because on that side the keyboard has pushed content clear of the home
    /// indicator anyway, and insetting for both would leave a visible gap above
    /// the keyboard. This is `padding`, and the subtraction is the
    /// part that is easy to leave out.
    #[must_use]
    pub fn padding(&self) -> EdgeInsets {
        EdgeInsets {
            left: (self.safe_area.left - self.view_insets.left).max(0.0),
            top: (self.safe_area.top - self.view_insets.top).max(0.0),
            right: (self.safe_area.right - self.view_insets.right).max(0.0),
            bottom: (self.safe_area.bottom - self.view_insets.bottom).max(0.0),
        }
    }

    /// `true` when a soft keyboard is up.
    #[must_use]
    pub fn keyboard_is_visible(&self) -> bool {
        self.view_insets.bottom > 0.0
    }

    /// The part of the surface that is neither system furniture nor keyboard.
    #[must_use]
    pub fn safe_size(&self) -> Size {
        let padding = self.padding();
        Size::new(
            (self.size.width - padding.left - padding.right).max(0.0),
            (self.size.height - padding.top - padding.bottom - self.view_insets.bottom).max(0.0),
        )
    }
}

impl Default for ViewMetrics {
    fn default() -> Self {
        Self::plain(Size::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> ViewMetrics {
        ViewMetrics {
            size: Size::new(390.0, 844.0),
            device_pixel_ratio: 3.0,
            // A notch at the top and a home indicator at the bottom.
            safe_area: EdgeInsets::only(0.0, 47.0, 0.0, 34.0),
            view_insets: EdgeInsets::ZERO,
        }
    }

    #[test]
    fn a_desktop_window_has_no_furniture_on_it() {
        let metrics = ViewMetrics::plain(Size::new(800.0, 600.0));
        assert_eq!(metrics.padding(), EdgeInsets::ZERO);
        assert!(!metrics.keyboard_is_visible());
        assert_eq!(metrics.safe_size(), Size::new(800.0, 600.0));
    }

    #[test]
    fn content_is_inset_past_the_notch_and_the_home_indicator() {
        let metrics = phone();
        assert_eq!(metrics.padding(), EdgeInsets::only(0.0, 47.0, 0.0, 34.0));
        assert_eq!(metrics.safe_size(), Size::new(390.0, 844.0 - 47.0 - 34.0));
    }

    #[test]
    fn the_keyboard_takes_over_the_bottom_inset_rather_than_adding_to_it() {
        let mut metrics = phone();
        metrics.view_insets = EdgeInsets::only(0.0, 0.0, 0.0, 300.0);

        assert_eq!(
            metrics.padding().bottom,
            0.0,
            "insetting for the home indicator *and* the keyboard leaves a gap \
             above the keyboard that looks like a layout bug"
        );
        assert_eq!(metrics.padding().top, 47.0, "the notch has not moved");
        assert!(metrics.keyboard_is_visible());
    }

    #[test]
    fn a_keyboard_shorter_than_the_indicator_still_leaves_some_inset() {
        let mut metrics = phone();
        metrics.view_insets = EdgeInsets::only(0.0, 0.0, 0.0, 10.0);
        assert!(
            (metrics.padding().bottom - 24.0).abs() < f32::EPSILON,
            "34 - 10, not clamped to zero: got {}",
            metrics.padding().bottom
        );
    }

    #[test]
    fn the_safe_size_never_goes_negative_on_a_small_surface() {
        let metrics = ViewMetrics {
            size: Size::new(100.0, 20.0),
            device_pixel_ratio: 1.0,
            safe_area: EdgeInsets::only(0.0, 47.0, 0.0, 34.0),
            view_insets: EdgeInsets::ZERO,
        };
        let size = metrics.safe_size();
        assert!(size.height >= 0.0, "a negative size would panic a layout");
    }
}
