//! How a box is painted: a fill, rounded corners and a border.
//!
//! Lives in `foundation` for the same reason [`TextStyle`](crate::TextStyle)
//! does — `docs/DESIGN.md` §7. The widget layer describes a decoration and the
//! render layer paints it, and neither may depend on the other, so the
//! vocabulary they meet on belongs here.
//!
//! A gradient fill and a cast shadow are here now, because the paint layer
//! grew the capability to draw them — the rule this module was written under
//! was that a decoration cannot describe what no canvas can paint, and that is
//! what changed. Images-as-fill are still absent: [`Image`](crate::Image) is a
//! *child* in this framework rather than a paint, and giving a box two ways to
//! show one would be two code paths for one picture.

use crate::{Color, Gradient, Rect, Shadow};

/// A solid border drawn just inside a box's bounds.
///
/// Inside, not centred on the edge and not outside, so that a bordered box
/// occupies exactly the space it was given. A border that straddled the boundary
/// would make every outlined control half a pixel larger than the filled one
/// beside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub color: Color,
    /// Thickness in logical pixels.
    pub width: f32,
}

impl Border {
    #[must_use]
    pub const fn new(color: Color, width: f32) -> Self {
        Self { color, width }
    }

    /// A hairline border — the thinnest that is still a line.
    #[must_use]
    pub const fn thin(color: Color) -> Self {
        Self::new(color, 1.0)
    }

    /// `true` if drawing this border would change no pixels.
    #[must_use]
    pub fn is_invisible(self) -> bool {
        self.width <= 0.0 || self.color.is_transparent()
    }
}

/// The fill, corner radius and border of a box.
///
/// The default is invisible: no fill, no border, square corners. That makes
/// `BoxDecoration::default()` free to paint and safe to hit test — a box that
/// draws nothing must not absorb the taps that belong to whatever is behind it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BoxDecoration {
    /// Painted across the whole box, *under* the border.
    ///
    /// Under, matching CSS, so a translucent border blends with the fill rather
    /// than with whatever happens to be behind the box.
    pub color: Color,
    /// Corner radius in logical pixels, clamped at paint time to half the
    /// shorter side — so a radius of [`f32::MAX`] is the reliable spelling of
    /// "a stadium", which is what a switch track and a chip both want.
    pub radius: f32,
    pub border: Option<Border>,
    /// Painted over [`color`](Self::color) and under the border.
    ///
    /// Over rather than instead of, so a translucent gradient over a solid fill
    /// is expressible — which is how a scrim over a surface colour is written
    /// without nesting two boxes.
    pub gradient: Option<Gradient>,
    /// Cast behind the box, before anything else is drawn.
    ///
    /// **One shadow, not a list.** Elevation-style shadows stack two or three, and
    /// this deliberately does not: a list would cost this type its `Copy`, which
    /// the paint layer relies on all the way down to per-command damage
    /// comparison. Two `DecoratedBox`es nest for the rare design that needs the
    /// ambient-plus-key pair, and the common case pays nothing.
    pub shadow: Option<Shadow>,
}

impl BoxDecoration {
    /// Nothing: no fill, no border, square corners.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            color: Color::TRANSPARENT,
            radius: 0.0,
            border: None,
            gradient: None,
            shadow: None,
        }
    }

    /// A solid fill.
    #[must_use]
    pub const fn filled(color: Color) -> Self {
        Self::new().color(color)
    }

    /// A border with nothing inside it.
    #[must_use]
    pub const fn outlined(border: Border) -> Self {
        Self::new().border(border)
    }

    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub const fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// Round the corners as far as the shape allows — a stadium for an oblong,
    /// a circle for a square.
    #[must_use]
    pub const fn stadium(self) -> Self {
        self.radius(f32::MAX)
    }

    #[must_use]
    pub const fn border(mut self, border: Border) -> Self {
        self.border = Some(border);
        self
    }

    /// Fill with a ramp of colours instead of — or over — the flat fill.
    #[must_use]
    pub const fn gradient(mut self, gradient: Gradient) -> Self {
        self.gradient = Some(gradient);
        self
    }

    /// Cast a shadow behind the box.
    #[must_use]
    pub const fn shadow(mut self, shadow: Shadow) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// The visible border, if there is one.
    #[must_use]
    pub fn visible_border(self) -> Option<Border> {
        self.border.filter(|border| !border.is_invisible())
    }

    /// The gradient, if one would actually paint something.
    #[must_use]
    pub fn visible_gradient(self) -> Option<Gradient> {
        self.gradient.filter(|gradient| !gradient.is_invisible())
    }

    /// The shadow, if one would actually paint something.
    #[must_use]
    pub fn visible_shadow(self) -> Option<Shadow> {
        self.shadow.filter(|shadow| !shadow.is_invisible())
    }

    /// `true` if painting this decoration would change no pixels.
    ///
    /// The render layer uses it to stay out of the way entirely: an invisible
    /// decoration records no draw calls and is transparent to hit testing.
    #[must_use]
    pub fn is_invisible(self) -> bool {
        self.color.is_transparent()
            && self.visible_border().is_none()
            && self.visible_gradient().is_none()
            && self.visible_shadow().is_none()
    }

    /// Everything this decoration can tint when painted into `bounds`.
    ///
    /// Larger than `bounds` exactly when a shadow escapes it, which is the whole
    /// reason this exists: a render object that reported its own size as its
    /// paint bounds would leave the shadow's outer half undamaged, and it would
    /// smear across the screen on the next scroll.
    #[must_use]
    pub fn paint_bounds(self, bounds: Rect) -> Rect {
        match self.visible_shadow() {
            Some(shadow) => bounds.union(shadow.bounds(bounds)),
            None => bounds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_decoration_paints_nothing() {
        assert!(BoxDecoration::default().is_invisible());
    }

    #[test]
    fn a_zero_width_border_is_not_a_border() {
        let decoration = BoxDecoration::outlined(Border::new(Color::BLACK, 0.0));
        assert!(decoration.visible_border().is_none());
        assert!(decoration.is_invisible());
    }

    #[test]
    fn a_transparent_border_is_not_a_border() {
        let decoration = BoxDecoration::outlined(Border::thin(Color::TRANSPARENT));
        assert!(decoration.is_invisible());
    }

    #[test]
    fn a_fill_alone_is_visible_and_so_is_a_border_alone() {
        assert!(!BoxDecoration::filled(Color::RED).is_invisible());
        assert!(!BoxDecoration::outlined(Border::thin(Color::RED)).is_invisible());
    }
}
