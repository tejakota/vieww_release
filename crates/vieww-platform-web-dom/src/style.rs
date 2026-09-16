//! The CSS half of the translation: an accumulator, and how vieww's paint and
//! layout types spell themselves in CSS.
//!
//! # Why an accumulator rather than one element per widget
//!
//! A vieww tree says `Padding > DecoratedBox > Align > Flex` — four widgets for
//! one visual box. Emitting a `<div>` per widget would be correct only by
//! accident: four nested divs break every sizing relationship CSS has, because
//! `flex: 1` on the outer one does not reach the inner one, and a percentage
//! height resolves against the wrong parent.
//!
//! So the walk *accumulates*. Layout-transparent widgets fold their property
//! into a [`Style`] and descend; a widget that needs a box of its own — a
//! `Flex`, a `Stack`, a `Text` — flushes the accumulator into one element and
//! starts a fresh one below. A property that would overwrite one already set is
//! also a flush, because two paddings are two boxes in any language.

use std::fmt::Write as _;

use vieww_foundation::{
    Border, BoxDecoration, Color, EdgeInsets, FontFamily, FontWeight, Gradient, GradientGeometry,
    Shadow, TextAlign, TextStyle, Transform,
};

/// `rgba()`, because a colour with an alpha is the common case on this target
/// and `#rrggbbaa` is not understood by every browser vieww supports.
#[must_use]
pub fn css_color(color: Color) -> String {
    if color.a == 255 {
        format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
    } else {
        format!(
            "rgba({},{},{},{:.4})",
            color.r,
            color.g,
            color.b,
            f32::from(color.a) / 255.0
        )
    }
}

/// A length in logical pixels.
///
/// Logical pixels *are* CSS pixels — that equivalence is the whole reason this
/// backend needs no scale factor anywhere, while the canvas one has to apply
/// the device-pixel ratio to its finished scene by hand.
#[must_use]
pub fn px(value: f32) -> String {
    if (value - value.round()).abs() < 0.01 {
        format!("{}px", value.round() as i32)
    } else {
        format!("{value:.2}px")
    }
}

/// A vieww gradient as a CSS `linear-gradient` / `radial-gradient` / `conic-gradient`.
///
/// The stops carry their own offsets, so the only translation is the geometry
/// and the fact that CSS wants percentages where vieww wants fractions.
#[must_use]
pub fn css_gradient(gradient: &Gradient) -> String {
    let stops = gradient
        .stops()
        .iter()
        .map(|stop| format!("{} {:.2}%", css_color(stop.color), stop.offset * 100.0))
        .collect::<Vec<_>>()
        .join(", ");
    match gradient.geometry {
        GradientGeometry::Linear { start, end } => {
            // CSS measures the angle from "to top", clockwise; vieww gives two
            // points in the box's own 0..1 space.
            let dx = end.dx - start.dx;
            let dy = end.dy - start.dy;
            let angle = dx.atan2(-dy).to_degrees();
            format!("linear-gradient({angle:.2}deg, {stops})")
        }
        GradientGeometry::Radial { center, radius } => format!(
            "radial-gradient({:.2}% {:.2}% at {:.2}% {:.2}%, {stops})",
            radius * 100.0,
            radius * 100.0,
            center.dx * 100.0,
            center.dy * 100.0,
        ),
        GradientGeometry::Sweep {
            center,
            start_angle,
            ..
        } => format!(
            "conic-gradient(from {:.2}deg at {:.2}% {:.2}%, {stops})",
            start_angle.to_degrees() + 90.0,
            center.dx * 100.0,
            center.dy * 100.0,
        ),
    }
}

/// A vieww shadow as one `box-shadow` entry.
///
/// vieww's `blur` is CSS's: the total width of the blurred edge, twice the
/// Gaussian's standard deviation. So this is a rename, not a conversion — which
/// is the point of having defined it that way.
#[must_use]
pub fn css_shadow(shadow: Shadow) -> String {
    format!(
        "{}{} {} {} {} {}",
        if shadow.is_inset { "inset " } else { "" },
        px(shadow.offset.dx),
        px(shadow.offset.dy),
        px(shadow.blur),
        px(shadow.spread),
        css_color(shadow.color),
    )
}

/// The font stack a family resolves to.
///
/// `Named` first, then the same generic behind it, so a page that ships Geist
/// still renders if the face fails to load — which on this target is a real
/// possibility and on the canvas one is not.
#[must_use]
pub fn css_family(family: FontFamily) -> String {
    match family {
        FontFamily::SansSerif => {
            "ui-sans-serif, system-ui, -apple-system, 'Segoe UI', sans-serif".into()
        }
        FontFamily::Serif => "ui-serif, Georgia, serif".into(),
        FontFamily::Monospace => "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace".into(),
        FontFamily::Named(name) => {
            format!("'{name}', ui-sans-serif, system-ui, sans-serif")
        }
    }
}

/// CSS's numeric weights are what [`FontWeight`] already stores.
#[must_use]
pub const fn css_weight(weight: FontWeight) -> u16 {
    weight.value()
}

/// `Positioned`'s four edges — left, top, right, bottom — each `Some` only if
/// the author pinned it. Two opposite edges pinned is how a `Stack` child
/// stretches; one edge and a size is how it sits.
pub type Inset = (Option<f32>, Option<f32>, Option<f32>, Option<f32>);

/// One element's worth of CSS, built up as the walk descends.
///
/// Every field is `Option` so that "not set" and "set to the default" stay
/// different things: the first can be filled in by a widget further down, and
/// the second cannot be overwritten without flushing.
#[derive(Debug, Default, Clone)]
pub struct Style {
    pub padding: Option<EdgeInsets>,
    pub margin: Option<EdgeInsets>,
    pub background: Option<Color>,
    pub gradient: Option<Gradient>,
    pub radius: Option<f32>,
    pub border: Option<Border>,
    pub shadow: Option<Shadow>,
    pub opacity: Option<f32>,
    pub transform: Option<Transform>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub min_width: Option<f32>,
    pub max_width: Option<f32>,
    pub min_height: Option<f32>,
    pub max_height: Option<f32>,
    pub aspect_ratio: Option<f32>,
    pub clip: bool,
    pub cursor: Option<&'static str>,
    pub flex: Option<f32>,
    /// Set by `Positioned`; makes this child absolute in its `Stack`.
    pub position: Option<Inset>,
    /// Raw declarations from a `Styled` widget — the escape hatch for the
    /// things CSS can do and a rasteriser cannot: `backdrop-filter`,
    /// `background-clip: text`, a repeating grid, a transition, a media query's
    /// worth of custom property.
    pub raw: Vec<String>,
}

impl Style {
    /// `true` if `slot` is already spoken for, so setting it needs a new box.
    #[must_use]
    pub const fn has(&self, slot: Slot) -> bool {
        match slot {
            Slot::Padding => self.padding.is_some(),
            Slot::Background => self.background.is_some() || self.gradient.is_some(),
            Slot::Size => self.width.is_some() || self.height.is_some(),
            Slot::Bounds => {
                self.min_width.is_some()
                    || self.max_width.is_some()
                    || self.min_height.is_some()
                    || self.max_height.is_some()
            }
            Slot::Opacity => self.opacity.is_some(),
            Slot::Transform => self.transform.is_some(),
            Slot::Position => self.position.is_some(),
            Slot::Flex => self.flex.is_some(),
        }
    }

    /// Fold a decoration in. Returns `false` if it would overwrite something.
    pub fn decorate(&mut self, decoration: &BoxDecoration) -> bool {
        if self.has(Slot::Background) || self.radius.is_some() || self.border.is_some() {
            return false;
        }
        if decoration.color.a > 0 {
            self.background = Some(decoration.color);
        }
        self.gradient = decoration.gradient;
        self.radius = Some(decoration.radius);
        self.border = decoration.border;
        self.shadow = decoration.shadow;
        true
    }

    /// The `style` attribute this describes.
    #[must_use]
    pub fn to_css(&self) -> String {
        let mut css = String::new();
        if let Some(p) = self.padding {
            let _ = write!(
                css,
                "padding:{} {} {} {};",
                px(p.top),
                px(p.right),
                px(p.bottom),
                px(p.left)
            );
        }
        if let Some(m) = self.margin {
            let _ = write!(
                css,
                "margin:{} {} {} {};",
                px(m.top),
                px(m.right),
                px(m.bottom),
                px(m.left)
            );
        }
        if let Some(color) = self.background {
            let _ = write!(css, "background-color:{};", css_color(color));
        }
        if let Some(gradient) = &self.gradient {
            let _ = write!(css, "background-image:{};", css_gradient(gradient));
        }
        if let Some(radius) = self.radius {
            // `f32::MAX` is vieww's spelling of "a stadium"; CSS's is 9999px.
            let r = if radius > 9999.0 { 9999.0 } else { radius };
            let _ = write!(css, "border-radius:{};", px(r));
        }
        if let Some(border) = self.border {
            let _ = write!(
                css,
                "border:{} solid {};box-sizing:border-box;",
                px(border.width),
                css_color(border.color)
            );
        }
        if let Some(shadow) = self.shadow {
            let _ = write!(css, "box-shadow:{};", css_shadow(shadow));
        }
        if let Some(alpha) = self.opacity {
            let _ = write!(css, "opacity:{alpha:.4};");
        }
        if let Some(t) = self.transform {
            let _ = write!(
                css,
                "transform:matrix({:.5},{:.5},{:.5},{:.5},{:.4},{:.4});",
                t.a, t.b, t.c, t.d, t.tx, t.ty
            );
        }
        if let Some(w) = self.width {
            let _ = write!(css, "width:{};flex-shrink:0;", px(w));
        }
        if let Some(h) = self.height {
            let _ = write!(css, "height:{};flex-shrink:0;", px(h));
        }
        if let Some(v) = self.min_width {
            let _ = write!(css, "min-width:{};", px(v));
        }
        if let Some(v) = self.max_width {
            if v.is_finite() {
                let _ = write!(css, "max-width:{};width:100%;", px(v));
            }
        }
        if let Some(v) = self.min_height {
            let _ = write!(css, "min-height:{};", px(v));
        }
        if let Some(v) = self.max_height {
            if v.is_finite() {
                let _ = write!(css, "max-height:{};", px(v));
            }
        }
        if let Some(ratio) = self.aspect_ratio {
            let _ = write!(css, "aspect-ratio:{ratio:.4};");
        }
        if self.clip {
            css.push_str("overflow:hidden;");
        }
        if let Some(cursor) = self.cursor {
            let _ = write!(css, "cursor:{cursor};");
        }
        if let Some(factor) = self.flex {
            let _ = write!(css, "flex:{factor:.3} 1 0;min-width:0;min-height:0;");
        }
        if let Some((left, top, right, bottom)) = self.position {
            css.push_str("position:absolute;");
            if let Some(v) = left {
                let _ = write!(css, "left:{};", px(v));
            }
            if let Some(v) = top {
                let _ = write!(css, "top:{};", px(v));
            }
            if let Some(v) = right {
                let _ = write!(css, "right:{};", px(v));
            }
            if let Some(v) = bottom {
                let _ = write!(css, "bottom:{};", px(v));
            }
        }
        for raw in &self.raw {
            css.push_str(raw);
            if !raw.ends_with(';') {
                css.push(';');
            }
        }
        css
    }

    /// `true` if this would produce no CSS at all, so the box can be skipped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.to_css().is_empty()
    }
}

/// The property groups that cannot be set twice on one element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Padding,
    Background,
    Size,
    Bounds,
    Opacity,
    Transform,
    Position,
    Flex,
}

/// The CSS for a run of text.
#[must_use]
pub fn text_css(style: &TextStyle, align: TextAlign) -> String {
    let mut css = format!(
        "font-family:{};font-size:{};font-weight:{};line-height:{:.4};color:{};",
        css_family(style.family),
        px(style.size),
        css_weight(style.weight),
        style.line_height,
        css_color(style.color),
    );
    if style.italic {
        css.push_str("font-style:italic;");
    }
    if style.letter_spacing.abs() > 0.001 {
        let _ = write!(css, "letter-spacing:{};", px(style.letter_spacing));
    }
    css.push_str(match align {
        TextAlign::Left | TextAlign::Start => "text-align:left;",
        TextAlign::Right | TextAlign::End => "text-align:right;",
        TextAlign::Center => "text-align:center;",
    });
    css
}
