use vieww_foundation::{Capture, Color, Key};

use crate::{widget_node_from, BuildContext, Widget, WidgetKind, WidgetNode};

/// Hides its child from screenshots, screen recordings, the task-switcher
/// thumbnail and screen readers — in one declaration.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Sensitive;
///
/// let balance = Sensitive::new()
///     .label("Balance, hidden while the screen is being recorded")
///     .child(Text::new("£12,402.11"));
/// ```
///
/// `docs/AIMS.md` §D. The declaration is *what is sensitive*; **when** to hide
/// it is not the application's problem, and that is the whole difference from
/// every other framework's answer to this. [`Capture`] is published above the
/// tree by the frame driver, this widget reads it, and no application code runs
/// at the moment the screenshot is taken.
///
/// # Why the application must not be the one deciding
///
/// Because it cannot win. The screenshot, the recording and the recents
/// thumbnail are three platform mechanisms with different notifications and
/// different orderings — on some Android builds the thumbnail is captured
/// *before* `onPause` is delivered, so an application masking in a lifecycle
/// callback has already lost. Reading a published state means the mask is
/// already in the frame.
///
/// # What it does not do
///
/// It does not stop the platform capturing at all. `FLAG_SECURE` and its
/// equivalents are the platform layer's business and are a separate, blunter
/// tool — they black out the whole window, including the parts you were happy to
/// have in a support screenshot. This is the per-subtree half, and the two
/// compose.
///
/// # It is not invisible to a screen reader for no reason
///
/// While masked the subtree is dropped from the semantics tree, because a screen
/// reader is a second way to read the pixels and a screen recording usually
/// carries audio. What replaces it is a node saying *something is here and it is
/// hidden* — see [`label`](Self::label). Vanishing silently would read as a bug.
#[derive(Debug, Clone)]
pub struct Sensitive {
    cover: Color,
    label: Option<String>,
    /// Overrides the published state. `None` — the default — is the whole point;
    /// see [`always`](Self::always).
    forced: Option<bool>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Default for Sensitive {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensitive {
    /// Masked whenever something other than the screen is reading the surface.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cover: Color::BLACK,
            label: None,
            forced: None,
            child: None,
            key: None,
        }
    }

    /// What the mask is filled with.
    ///
    /// Opaque, or it is not a mask — a translucent cover over a card number is a
    /// card number. [`Color::BLACK`] by default, matching what the platforms do
    /// to a secure window.
    #[must_use]
    pub const fn cover(mut self, cover: Color) -> Self {
        self.cover = cover;
        self
    }

    /// What a screen reader is told while the subtree is hidden.
    ///
    /// Worth setting. The default says only that a masked region is here, which
    /// is enough not to read as a bug and not enough to be useful.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Mask regardless of what is reading the surface.
    ///
    /// For a subtree that is hidden *now* for a reason of the application's own
    /// — a balance the user tapped to hide, a field behind a "reveal" toggle.
    /// Distinct from the automatic behaviour and deliberately a different
    /// method, so that reading a call site tells you which of the two it is.
    #[must_use]
    pub const fn always(mut self, masked: bool) -> Self {
        self.forced = Some(masked);
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Whether this would mask under `capture`.
    ///
    /// Public so the decision can be tested without mounting a tree, following
    /// [`SafeArea::resolve`](crate::SafeArea::resolve).
    #[must_use]
    pub fn resolve(&self, capture: Capture) -> bool {
        self.forced.unwrap_or_else(|| capture.is_recorded())
    }
}

impl Widget for Sensitive {
    fn debug_name(&self) -> &'static str {
        "Sensitive"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // Nothing published means no platform under us — a test, or a tree being
        // dumped. `Capture::Screen` is the honest answer there: a surface nobody
        // is reading.
        let capture = ctx.inherit_or(Capture::default());
        let mut mask = SensitiveMask::new(self.resolve(*capture), self.cover, self.label.clone());
        if let Some(child) = &self.child {
            mask = mask.child(child.clone());
        }
        mask.into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![(
            "masked",
            match self.forced {
                Some(masked) => masked.to_string(),
                None => "when recorded".to_owned(),
            },
        )]
    }
}

widget_node_from!(Sensitive);

/// The resolved half of [`Sensitive`]: masked or not, decided.
///
/// Separate because the render factory builds an object from a *widget* and
/// cannot see inherited values — so the reading of [`Capture`] has to happen in
/// a `build`, and what comes out of that build is this. Public because a
/// third-party widget with its own idea of when to hide something can produce
/// one directly, which is §A's rule applied to a widget rather than a service.
#[derive(Debug, Clone)]
pub struct SensitiveMask {
    masked: bool,
    cover: Color,
    label: Option<String>,
    child: Option<WidgetNode>,
}

impl SensitiveMask {
    #[must_use]
    pub const fn new(masked: bool, cover: Color, label: Option<String>) -> Self {
        Self {
            masked,
            cover,
            label,
            child: None,
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Whether the child is currently hidden — read by the render layer.
    #[must_use]
    pub const fn is_masked(&self) -> bool {
        self.masked
    }

    #[must_use]
    pub const fn cover_color(&self) -> Color {
        self.cover
    }

    #[must_use]
    pub fn mask_label(&self) -> Option<String> {
        self.label.clone()
    }
}

impl Widget for SensitiveMask {
    fn debug_name(&self) -> &'static str {
        "SensitiveMask"
    }

    fn kind(&self) -> WidgetKind<'_> {
        match &self.child {
            Some(child) => WidgetKind::RenderSingleChild(child),
            None => WidgetKind::RenderLeaf,
        }
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("masked", self.masked.to_string())]
    }
}

widget_node_from!(SensitiveMask);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_masked_while_only_the_user_is_looking() {
        assert!(!Sensitive::new().resolve(Capture::Screen));
    }

    #[test]
    fn a_recording_masks_without_the_application_being_asked() {
        assert!(
            Sensitive::new().resolve(Capture::Recorded),
            "the declaration is what is sensitive; when to hide it is not the \
             application's problem"
        );
    }

    #[test]
    fn always_overrides_in_both_directions() {
        assert!(Sensitive::new().always(true).resolve(Capture::Screen));
        assert!(
            !Sensitive::new().always(false).resolve(Capture::Recorded),
            "an explicit `false` is a deliberate opt-out — a support screenshot \
             of a screen with nothing secret on it"
        );
    }
}
