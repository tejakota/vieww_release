//! How a shape is filled, and what it casts.
//!
//! Gradients, shadows and blend modes are *descriptions*, so they live here
//! rather than in `vieww-paint`: the widget layer has to be able to name a
//! gradient in order to describe a button, and it sits below the paint layer.
//! Same reason [`Path`](crate::Path) is here.
//!
//! # Everything in this module is `Copy`, and that is a deliberate constraint
//!
//! [`Paint`](crate::Paint) is `Copy` today and the paint layer relies on it —
//! `Scene::fills` hands back paints by value, a recorded command copies one out
//! of the state stack, and damage compares two of them for equality. A `Vec` of
//! gradient stops anywhere in that chain turns every one of those into a clone,
//! per command, per frame.
//!
//! So a gradient carries a **fixed-capacity** stop array instead
//! ([`MAX_GRADIENT_STOPS`]). Eight is past the point where a designer's ramp
//! stops being legible, and the cost of the choice is bounded and known:
//! [`Gradient::with_stops`] silently keeps the first eight rather than
//! allocating. If a real design ever needs more, the fix is a `Box<[GradientStop]>`
//! here and a clone in `State::apply` — a trade this makes consciously rather
//! than by default.

use crate::{Color, Offset, Rect};

/// How many colour stops a [`Gradient`] can carry.
///
/// See the module docs: the cap is what keeps every paint type `Copy`.
pub const MAX_GRADIENT_STOPS: usize = 8;

/// One colour at one position along a gradient.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    /// Where along the gradient this colour lands, 0.0 to 1.0.
    pub offset: f32,
    pub color: Color,
}

impl GradientStop {
    #[must_use]
    pub const fn new(offset: f32, color: Color) -> Self {
        Self { offset, color }
    }
}

/// The geometry a gradient is laid out along.
///
/// Both variants are expressed in the **unit square of the box being painted**:
/// `(0, 0)` is its top-left corner and `(1, 1)` its bottom-right, whatever size
/// it turns out to be. That is what lets one `BoxDecoration` describe a button
/// without knowing how wide layout will make it — the same reason the framework's
/// own `Alignment` is fractional.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientGeometry {
    /// A ramp from `start` to `end`.
    Linear { start: Offset, end: Offset },
    /// A ramp outward from `center`, reaching its last stop at `radius`.
    ///
    /// The radius is a fraction of the box's **shorter** side, so a circle stays
    /// a circle in an oblong box.
    ///
    /// **`0.5` is the value that inscribes**, not `1.0` — the fraction is of the
    /// whole shorter side, and a radius reaches only half way across the box it
    /// is centred in. That is the convention everywhere (`RadialGradient::radius`
    /// defaults to 0.5) and CSS's, and the alternative reading cost a test
    /// failure before it was written down here.
    Radial { center: Offset, radius: f32 },
    /// A ramp swept *around* `center`, from `start_angle` to `end_angle`.
    ///
    /// A conic gradient: the colour varies with angle rather than with distance.
    /// What a pie chart, a hue wheel and a determinate ring's fill are made of —
    /// none of which the other two can express at all.
    ///
    /// Angles are in **radians from the positive X axis**, increasing clockwise
    /// in the Y-down space everything else here uses. So a full turn beginning
    /// at twelve o'clock is `-FRAC_PI_2` to `-FRAC_PI_2 + TAU`.
    Sweep {
        center: Offset,
        start_angle: f32,
        end_angle: f32,
    },
}

/// A ramp of colours filling a shape.
///
/// Stops are kept in the order they were given. Nothing sorts them, because a
/// caller listing them out of order has made a mistake worth seeing rather than
/// one worth silently repairing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gradient {
    pub geometry: GradientGeometry,
    stops: [GradientStop; MAX_GRADIENT_STOPS],
    /// How many entries of `stops` are real.
    count: u8,
    /// Whether the rasterizer should dither the ramp on the way to 8-bit
    /// output.
    ///
    /// Off by default, because a byte-for-byte golden of an undithered ramp is
    /// still a byte-for-byte golden — every committed image stays valid until
    /// somebody opts in. On, the renderer breaks up the bands a 256-step ramp
    /// falls into when it is stretched across a few hundred pixels: an ordered
    /// 4×4 Bayer offset is folded into the sample before quantisation, so a
    /// slow blue→black fade reads as continuous rather than as a staircase of
    /// visible steps. See `native/gradient.rs` for where the offset lands.
    dither: bool,
}

impl Gradient {
    /// A linear ramp between two points in the box's unit square.
    ///
    /// ```
    /// use vieww_foundation::{Color, Gradient, Offset};
    ///
    /// // Top to bottom.
    /// let g = Gradient::linear(Offset::new(0.5, 0.0), Offset::new(0.5, 1.0))
    ///     .with_stops(&[(0.0, Color::WHITE), (1.0, Color::BLACK)]);
    /// assert_eq!(g.stops().len(), 2);
    /// ```
    #[must_use]
    pub const fn linear(start: Offset, end: Offset) -> Self {
        Self::empty(GradientGeometry::Linear { start, end })
    }

    /// A linear ramp straight down the box.
    #[must_use]
    pub const fn vertical() -> Self {
        Self::linear(Offset::new(0.5, 0.0), Offset::new(0.5, 1.0))
    }

    /// A linear ramp across the box, left to right.
    ///
    /// Note this is *physical* left-to-right rather than leading-to-trailing:
    /// a gradient is decoration, and unlike text it does not flip in Arabic
    /// unless the caller asks it to.
    #[must_use]
    pub const fn horizontal() -> Self {
        Self::linear(Offset::new(0.0, 0.5), Offset::new(1.0, 0.5))
    }

    /// A radial ramp from `center`, reaching its last stop at `radius`.
    ///
    /// `radius` is a fraction of the box's shorter side, so **0.5 inscribes** —
    /// see [`GradientGeometry::Radial`].
    #[must_use]
    pub const fn radial(center: Offset, radius: f32) -> Self {
        Self::empty(GradientGeometry::Radial { center, radius })
    }

    /// A ramp swept around `center`, from `start_angle` to `end_angle` in
    /// radians — see [`GradientGeometry::Sweep`].
    #[must_use]
    pub const fn sweep(center: Offset, start_angle: f32, end_angle: f32) -> Self {
        Self::empty(GradientGeometry::Sweep {
            center,
            start_angle,
            end_angle,
        })
    }

    /// A full turn around the middle of the box, beginning at twelve o'clock.
    ///
    /// The one a hue wheel or a pie chart wants, and the reason the constant is
    /// here rather than at each call site: getting the quarter-turn offset wrong
    /// puts the seam at three o'clock, which looks deliberate and is not.
    #[must_use]
    pub const fn conic() -> Self {
        Self::sweep(
            Offset::new(0.5, 0.5),
            -std::f32::consts::FRAC_PI_2,
            -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU,
        )
    }

    /// A radial ramp filling the box from its centre.
    ///
    /// The one everybody actually wants, spelled so that nobody has to remember
    /// whether the inscribing radius is 0.5 or 1.0.
    #[must_use]
    pub const fn radial_fill() -> Self {
        Self::radial(Offset::new(0.5, 0.5), 0.5)
    }

    const fn empty(geometry: GradientGeometry) -> Self {
        Self {
            geometry,
            stops: [GradientStop::new(0.0, Color::TRANSPARENT); MAX_GRADIENT_STOPS],
            count: 0,
            dither: false,
        }
    }

    /// Ask the rasterizer for a dithered ramp.
    ///
    /// Idempotent with [`Self::undithered`]: whichever was called last wins, so
    /// an animation toggling the flag does not have to branch to undo it.
    #[must_use]
    pub const fn with_dither(mut self) -> Self {
        self.dither = true;
        self
    }

    /// Ask for the exact, undithered ramp — the default.
    #[must_use]
    pub const fn undithered(mut self) -> Self {
        self.dither = false;
        self
    }

    /// Whether this ramp wants dithering at rasterization time.
    #[must_use]
    pub const fn dither(&self) -> bool {
        self.dither
    }

    /// Repoint the ramp, keeping its stops.
    ///
    /// The direction of a gradient and the colours along it are separate
    /// decisions, and something animating one of them should not have to
    /// restate the other. `stops` is private so that a caller cannot leave
    /// `count` disagreeing with the array; this is the supported way to change
    /// the half of a gradient that is public.
    #[must_use]
    pub const fn with_geometry(mut self, geometry: GradientGeometry) -> Self {
        self.geometry = geometry;
        self
    }

    /// Replace the stops.
    ///
    /// Anything past [`MAX_GRADIENT_STOPS`] is dropped — see the module docs for
    /// why the cap exists and what it would take to lift it.
    #[must_use]
    pub fn with_stops(mut self, stops: &[(f32, Color)]) -> Self {
        self.count = 0;
        for &(offset, color) in stops.iter().take(MAX_GRADIENT_STOPS) {
            self.stops[self.count as usize] = GradientStop::new(offset, color);
            self.count += 1;
        }
        self
    }

    /// A two-stop ramp, which is the overwhelmingly common case.
    #[must_use]
    pub fn between(self, from: Color, to: Color) -> Self {
        self.with_stops(&[(0.0, from), (1.0, to)])
    }

    /// The stops, in the order they were given.
    #[must_use]
    pub fn stops(&self) -> &[GradientStop] {
        &self.stops[..self.count as usize]
    }

    /// `true` if this would paint nothing at all.
    ///
    /// A gradient with no stops has no colours to interpolate between, and one
    /// whose every stop is transparent is an expensive way to draw nothing.
    #[must_use]
    pub fn is_invisible(&self) -> bool {
        self.stops().is_empty() || self.stops().iter().all(|stop| stop.color.is_transparent())
    }

    /// One colour standing in for the whole ramp.
    ///
    /// Two jobs, both load-bearing. It is what a backend with no gradient
    /// support falls back to — a flat approximation is a worse picture, but a
    /// picture. And it is what `Paint::color` reports, so
    /// invisibility checks, damage and every existing test that reads a paint's
    /// colour keep working against a gradient without knowing it is one.
    ///
    /// The midpoint stop rather than an average: averaging two stops of a
    /// two-stop ramp gives the same answer, and averaging a ramp with a bright
    /// accent in the middle gives a colour that appears nowhere in it.
    #[must_use]
    pub fn representative(&self) -> Color {
        let stops = self.stops();
        if stops.is_empty() {
            return Color::TRANSPARENT;
        }
        stops[stops.len() / 2].color
    }

    /// Multiply every stop's alpha by `alpha`.
    ///
    /// What an enclosing [`push_alpha`](crate::Color) fade does to a gradient:
    /// the same thing it does to a solid colour, stop by stop.
    #[must_use]
    pub fn faded(mut self, alpha: f32) -> Self {
        for stop in &mut self.stops[..self.count as usize] {
            stop.color = fade(stop.color, alpha);
        }
        self
    }

    /// Where this gradient's geometry lands in a box of `bounds`, as absolute
    /// coordinates.
    ///
    /// Returns `(start, end)` for a linear ramp, `(centre, edge)` for a radial
    /// one — where the distance between the two is the radius — and
    /// `(centre, centre)` for a sweep, whose extent is its two **angles** and so
    /// has no second point to resolve. Backends resolve here rather than each
    /// doing its own unit-square arithmetic.
    #[must_use]
    pub fn resolve(&self, bounds: Rect) -> (Offset, Offset) {
        let at = |fraction: Offset| {
            Offset::new(
                bounds.left + fraction.dx * bounds.width(),
                bounds.top + fraction.dy * bounds.height(),
            )
        };
        match self.geometry {
            GradientGeometry::Linear { start, end } => (at(start), at(end)),
            GradientGeometry::Radial { center, radius } => {
                let centre = at(center);
                let shorter = bounds.width().min(bounds.height());
                (centre, Offset::new(centre.dx + radius * shorter, centre.dy))
            }
            // Twice, because the pair is the whole return type and a sweep has
            // only one point in it. A backend reads the angles off the geometry,
            // which need no resolving — they are absolute, not fractions of a
            // box that has not been laid out yet.
            GradientGeometry::Sweep { center, .. } => {
                let centre = at(center);
                (centre, centre)
            }
        }
    }
}

/// A blurred silhouette cast behind a box.
///
/// Modelled on CSS `box-shadow` rather than on an elevation token,
/// because elevation is a *token* — it belongs in a theme, which can express it
/// as one of these. Baking an elevation curve in here would make every other
/// design fight the framework.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub color: Color,
    /// How far the shadow is displaced from the box casting it.
    pub offset: Offset,
    /// The width of the blurred edge, in logical pixels.
    ///
    /// CSS's definition: the blur extends roughly this far in total, which is
    /// twice the standard deviation of the Gaussian a backend actually applies.
    pub blur: f32,
    /// How much larger than its box the shadow is before blurring.
    pub spread: f32,
    /// Cast *inside* the box rather than behind it — CSS's `inset` keyword.
    ///
    /// A flag rather than a second type because everything else about the two is
    /// the same: colour, offset, blur and spread all mean what they mean, only
    /// the side of the edge they land on changes. It is what a recessed surface
    /// — a slider's channel, the well a segmented control's selection sits in —
    /// actually is. Applications faked it with a gradient down the track, which
    /// is not the same primitive: a gradient does not follow the corner radius
    /// and does not darken relative to whatever is behind it, so it goes wrong
    /// the moment either changes.
    pub is_inset: bool,
}

impl Shadow {
    #[must_use]
    pub const fn new(color: Color, offset: Offset, blur: f32) -> Self {
        Self {
            color,
            offset,
            blur,
            spread: 0.0,
            is_inset: false,
        }
    }

    /// A shadow cast on the inside of the box, darkening its inner edge.
    ///
    /// The offset points the same way it does for an outer shadow — a positive
    /// `dy` is a light source above — so `inset(black, (0, 4), 8)` darkens the
    /// *top* inside edge, exactly as CSS's `inset 0 4px 8px` does.
    #[must_use]
    pub const fn inset(color: Color, offset: Offset, blur: f32) -> Self {
        Self {
            is_inset: true,
            ..Self::new(color, offset, blur)
        }
    }

    #[must_use]
    pub const fn spread(mut self, spread: f32) -> Self {
        self.spread = spread;
        self
    }

    /// `true` if drawing this would change nothing.
    #[must_use]
    pub const fn is_invisible(self) -> bool {
        self.color.is_transparent()
    }

    /// The area this shadow can tint, given the box casting it.
    ///
    /// Grown by the spread, moved by the offset, then grown again by the blur.
    /// A Gaussian never quite reaches zero, so the blur term is deliberately
    /// generous: under-reporting here leaves a smudge on the screen that only
    /// repaints when something else happens to damage it.
    ///
    /// An inset shadow is clipped to its caster and so can never tint a pixel
    /// outside it. Reporting the inflated rectangle for one would damage a ring
    /// of untouched pixels around every recessed control on every frame it
    /// repaints, which is pure cost.
    #[must_use]
    pub fn bounds(self, box_bounds: Rect) -> Rect {
        if self.is_inset {
            return box_bounds;
        }
        box_bounds
            .inflate(self.spread)
            .translate(self.offset)
            .inflate(self.blur * BLUR_REACH)
    }

    /// The standard deviation a Gaussian blur of this width corresponds to.
    #[must_use]
    pub fn std_dev(self) -> f32 {
        self.blur / 2.0
    }
}

/// How far past its nominal blur width a shadow is assumed to reach.
///
/// A Gaussian has infinite support; three standard deviations captures over 99%
/// of it, and [`Shadow::blur`] is two standard deviations. So 1.5 is "three
/// sigma", and it is the number damage tracking has to trust.
const BLUR_REACH: f32 = 1.5;

/// How a layer's pixels combine with what is already behind them.
///
/// A deliberately small set. These are the separable Porter-Duff mix modes that
/// every backend implements identically and that designers actually reach for;
/// the exotic ones differ between renderers in ways that would make a vieww app
/// look different on two phones, which is the one thing this framework exists to
/// prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    // ------------------------------------------------------- Porter-Duff
    //
    // How much of the source and of the destination survives, before any
    // colour arithmetic. These are the modes masking and knock-out are built
    // from, and the reason a duotone or a cut-out is expressible at all.
    /// Source over destination. What everything does unless told otherwise.
    #[default]
    Normal,
    /// Nothing survives. Erases the destination inside the drawn shape.
    Clear,
    /// The source replaces the destination outright, alpha included — the
    /// difference from [`Normal`](Self::Normal) is that a half-transparent
    /// source produces a half-transparent *result* rather than a blend.
    Src,
    /// The destination is kept and the source discarded. The identity, and
    /// useful as a runtime "draw nothing" without removing the node.
    Dst,
    /// Destination over source: the source is drawn *behind* what is already
    /// there.
    DstOver,
    /// The source, kept only where the destination already is. **This is the
    /// mask.** A gradient drawn `SrcIn` over a shape paints the gradient in the
    /// shape's silhouette.
    SrcIn,
    /// The destination, kept only where the source is.
    DstIn,
    /// The source, kept only where the destination is **not**.
    SrcOut,
    /// The destination, kept only where the source is not — a knock-out.
    DstOut,
    /// The source drawn over the destination, clipped to it: like
    /// [`SrcIn`](Self::SrcIn) but the destination shows through where the
    /// source is transparent.
    SrcAtop,
    /// The destination drawn over the source, clipped to the source.
    DstAtop,
    /// Whichever of the two is present alone. Where both are, neither.
    Xor,
    /// The two added, clamped. Lightens toward white; what a glow or an
    /// additive particle wants.
    Plus,

    // --------------------------------------------------------- separable
    //
    // Channel-by-channel arithmetic, all composited source-over.
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    /// Brightens the destination in proportion to the source.
    ColorDodge,
    /// Darkens the destination in proportion to the source.
    ColorBurn,
    /// [`Overlay`](Self::Overlay) with the operands swapped.
    HardLight,
    /// A gentler [`HardLight`](Self::HardLight); the classic soft shading mode.
    SoftLight,
    Difference,
    /// [`Difference`](Self::Difference) with a lower-contrast falloff.
    Exclusion,

    // ----------------------------------------------------- non-separable
    //
    // Defined on the colour as a whole rather than per channel, so they cannot
    // be expressed as three independent sums. This is the family a duotone,
    // a tint and a colourise are built from.
    /// The source's hue, the destination's saturation and luminosity.
    Hue,
    /// The source's saturation, the destination's hue and luminosity.
    Saturation,
    /// The source's hue **and** saturation over the destination's luminosity —
    /// the "colourise" of every image editor.
    Color,
    /// The source's luminosity over the destination's hue and saturation.
    Luminosity,
}

impl BlendMode {
    /// `true` for the mode that needs no compositing layer of its own.
    ///
    /// Only [`Normal`](Self::Normal) qualifies: every other mode is defined
    /// against *what is already there*, which means the group has to be
    /// rasterised separately before it can be combined with the backdrop.
    #[must_use]
    pub const fn is_normal(self) -> bool {
        matches!(self, Self::Normal)
    }

    /// `true` for the Porter-Duff coverage operators, which change **which
    /// pixels survive** rather than what colour they end up.
    ///
    /// Worth distinguishing because these are the ones that can *remove*
    /// destination pixels — a `Clear` or a `DstOut` inside a group erases part
    /// of it — so a caller reasoning about whether a layer can be skipped when
    /// its contents are empty has to know which family it is in.
    #[must_use]
    pub const fn is_coverage(self) -> bool {
        matches!(
            self,
            Self::Clear
                | Self::Src
                | Self::Dst
                | Self::DstOver
                | Self::SrcIn
                | Self::DstIn
                | Self::SrcOut
                | Self::DstOut
                | Self::SrcAtop
                | Self::DstAtop
                | Self::Xor
                | Self::Plus
        )
    }

    /// `true` for the four modes defined on the colour as a whole rather than
    /// channel by channel.
    ///
    /// These cannot be evaluated per channel, so a backend that implements
    /// blending itself needs a different code path for them — which is why they
    /// are worth being able to ask about rather than only match on.
    #[must_use]
    pub const fn is_non_separable(self) -> bool {
        matches!(
            self,
            Self::Hue | Self::Saturation | Self::Color | Self::Luminosity
        )
    }

    /// `true` for the modes whose result differs from the destination **where
    /// the source is not**.
    ///
    /// # Why this is worth asking about, and what it costs today
    ///
    /// A blend is applied when a composited group is combined with its
    /// backdrop, and a backend does that over the group's *region*. For every
    /// other mode that is harmless: outside the shapes the group drew, the
    /// result is the destination unchanged, so the region can be generous
    /// without anything showing. For these six it is not — outside the source,
    /// `Clear` gives nothing, `SrcIn` gives nothing, `DstIn` gives nothing —
    /// so the exact region matters, and getting it wrong does not produce a
    /// slightly-off blend, it erases whatever was already there.
    ///
    /// **The rasterisers this framework uses do not confine them.** Measured,
    /// not assumed, on 2026-08-29 against `vello_cpu` 0.0.9: a marker painted
    /// in one corner, a small blended layer in the middle, once per mode, and
    /// the corner read back. Twenty-two modes leave it alone. These six clear
    /// it — the compose is applied across the whole target rather than within
    /// the layer's clip path, which is passed and is correct.
    ///
    /// To re-derive the list after a rasteriser upgrade, delete
    /// [`confined`](Self::confined)'s substitution and run
    /// `vieww-paint`'s `no_blend_mode_reaches_outside_its_own_layer`: the
    /// modes it names are this list. That test is what holds the *guarantee*
    /// — no mode escapes — whether or not this table is still the right
    /// six.
    ///
    /// So the backends **refuse them and count the refusal** rather than
    /// wiping the surface: the layer is composited as
    /// [`Normal`](Self::Normal) and `SceneReport::unsupported_blends` records
    /// it. That is the same choice `SceneReport::skipped_text` makes — degrade
    /// to something honest and count it, never to nothing — and it is the only
    /// one available, because a mode that erases the screen is worse than a
    /// mode that is missing.
    ///
    /// # Why they are in the enum at all
    ///
    /// Because the model carrying a capability and a backend implementing it
    /// are different questions, and conflating them is how a colour model ends
    /// up unable to name a colour. These are the masking and duotone
    /// primitives; an application can express one today, a test can assert on
    /// the `Command` it produces, and the day a backend confines them the
    /// expression already exists. Removing them would mean the API had to
    /// change when that happens.
    #[must_use]
    pub const fn needs_isolated_compositing(self) -> bool {
        matches!(
            self,
            Self::Clear | Self::Src | Self::SrcIn | Self::DstIn | Self::SrcOut | Self::DstAtop
        )
    }

    /// The mode a backend should use in place of this one, and whether it had
    /// to substitute.
    ///
    /// `(Normal, true)` for the six above, `(self, false)` otherwise. A single
    /// place for the decision, so three backends cannot disagree about it.
    #[must_use]
    pub const fn confined(self) -> (Self, bool) {
        if self.needs_isolated_compositing() {
            (Self::Normal, true)
        } else {
            (self, false)
        }
    }

    /// Every mode, for a catalogue, a picker or an exhaustive test.
    ///
    /// An array rather than a derive so that adding a variant without adding it
    /// here is caught by `every_blend_mode_is_listed`, which counts.
    pub const ALL: [Self; 28] = [
        Self::Normal,
        Self::Clear,
        Self::Src,
        Self::Dst,
        Self::DstOver,
        Self::SrcIn,
        Self::DstIn,
        Self::SrcOut,
        Self::DstOut,
        Self::SrcAtop,
        Self::DstAtop,
        Self::Xor,
        Self::Plus,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
        Self::Darken,
        Self::Lighten,
        Self::ColorDodge,
        Self::ColorBurn,
        Self::HardLight,
        Self::SoftLight,
        Self::Difference,
        Self::Exclusion,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    /// The mode's name, for a debug overlay or a token editor.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Clear => "Clear",
            Self::Src => "Src",
            Self::Dst => "Dst",
            Self::DstOver => "DstOver",
            Self::SrcIn => "SrcIn",
            Self::DstIn => "DstIn",
            Self::SrcOut => "SrcOut",
            Self::DstOut => "DstOut",
            Self::SrcAtop => "SrcAtop",
            Self::DstAtop => "DstAtop",
            Self::Xor => "Xor",
            Self::Plus => "Plus",
            Self::Multiply => "Multiply",
            Self::Screen => "Screen",
            Self::Overlay => "Overlay",
            Self::Darken => "Darken",
            Self::Lighten => "Lighten",
            Self::ColorDodge => "ColorDodge",
            Self::ColorBurn => "ColorBurn",
            Self::HardLight => "HardLight",
            Self::SoftLight => "SoftLight",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Color => "Color",
            Self::Luminosity => "Luminosity",
        }
    }
}

/// A colour with its alpha multiplied by `alpha`.
///
/// Here rather than on `Color` because it is a *paint* operation — the rounding
/// and clamping are what a fade has to do, and putting it on the colour type
/// would invite it into places where alpha means transparency of a value rather
/// than strength of a draw.
#[must_use]
pub fn fade(color: Color, alpha: f32) -> Color {
    if alpha >= 1.0 {
        return color;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=255 immediately before the cast"
    )]
    let a = (f32::from(color.a) * alpha).round().clamp(0.0, 255.0) as u8;
    color.with_alpha(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_past_the_cap_are_dropped_rather_than_allocated_for() {
        let many: Vec<(f32, Color)> = (0..20).map(|i| (i as f32 / 19.0, Color::RED)).collect();
        let gradient = Gradient::vertical().with_stops(&many);
        assert_eq!(gradient.stops().len(), MAX_GRADIENT_STOPS);
    }

    #[test]
    fn a_gradient_reports_a_colour_that_appears_in_it() {
        let gradient = Gradient::vertical().between(Color::RED, Color::BLUE);
        let representative = gradient.representative();
        assert!(
            gradient
                .stops()
                .iter()
                .any(|stop| stop.color == representative),
            "the fallback colour must be one of the ramp's own, not an average \
             that appears nowhere in it"
        );
    }

    #[test]
    fn an_empty_gradient_is_invisible_rather_than_black() {
        let gradient = Gradient::vertical();
        assert!(gradient.is_invisible());
        assert_eq!(gradient.representative(), Color::TRANSPARENT);
    }

    #[test]
    fn a_gradient_of_transparent_stops_is_invisible() {
        let gradient = Gradient::vertical().between(Color::TRANSPARENT, Color::TRANSPARENT);
        assert!(gradient.is_invisible());
    }

    #[test]
    fn linear_geometry_resolves_into_the_box_it_paints() {
        let gradient = Gradient::vertical();
        let (start, end) = gradient.resolve(Rect::new(10.0, 20.0, 110.0, 70.0));
        assert_eq!(start, Offset::new(60.0, 20.0), "top centre");
        assert_eq!(end, Offset::new(60.0, 70.0), "bottom centre");
    }

    #[test]
    fn a_radial_gradient_measures_its_radius_against_the_shorter_side() {
        // 200 wide, 100 tall, so the shorter side is 100 and a radius of 1.0 is
        // 100 — which reaches *twice* as far as the box allows. The fraction is
        // of the whole side, not of the half.
        let gradient = Gradient::radial(Offset::new(0.5, 0.5), 1.0);
        let (centre, edge) = gradient.resolve(Rect::new(0.0, 0.0, 200.0, 100.0));
        assert_eq!(centre, Offset::new(100.0, 50.0));
        assert_eq!(edge.dx - centre.dx, 100.0);
    }

    #[test]
    fn a_sweep_resolves_its_centre_and_has_no_second_point() {
        // Its extent is the two angles, which are absolute radians and have
        // nothing about a box to be resolved against. The pair comes back with
        // the centre twice rather than with a meaningless edge.
        let gradient = Gradient::sweep(Offset::new(0.5, 0.5), 0.0, std::f32::consts::TAU);
        let (centre, edge) = gradient.resolve(Rect::new(0.0, 0.0, 200.0, 100.0));
        assert_eq!(centre, Offset::new(100.0, 50.0));
        assert_eq!(edge, centre);
    }

    #[test]
    fn a_conic_gradient_starts_at_twelve_oclock_and_goes_all_the_way_round() {
        // The quarter-turn offset is the whole reason `conic` exists: without it
        // the seam lands at three o'clock, which looks deliberate and is not.
        let GradientGeometry::Sweep {
            center,
            start_angle,
            end_angle,
        } = Gradient::conic().geometry
        else {
            panic!("conic is a sweep");
        };
        assert_eq!(center, Offset::new(0.5, 0.5));
        assert_eq!(start_angle, -std::f32::consts::FRAC_PI_2);
        assert!((end_angle - start_angle - std::f32::consts::TAU).abs() < 1e-6);
    }

    #[test]
    fn a_filling_radial_gradient_reaches_the_edge_and_stops() {
        // The one everybody means, and the reason `radial_fill` exists: getting
        // 0.5-versus-1.0 wrong here spills the ramp out of the top and bottom,
        // and the picture looks like a *stop* placement bug rather than a
        // radius one.
        let (centre, edge) = Gradient::radial_fill().resolve(Rect::new(0.0, 0.0, 200.0, 100.0));
        assert_eq!(centre, Offset::new(100.0, 50.0));
        assert_eq!(
            edge.dx - centre.dx,
            50.0,
            "half the shorter side, which is exactly the top and bottom edges"
        );
    }

    #[test]
    fn fading_a_gradient_fades_every_stop() {
        let gradient = Gradient::vertical()
            .between(Color::RED, Color::BLUE)
            .faded(0.5);
        assert!(gradient.stops().iter().all(|stop| stop.color.a == 128));
    }

    #[test]
    fn a_shadow_reaches_past_its_box_by_offset_spread_and_blur() {
        let shadow = Shadow::new(Color::BLACK, Offset::new(0.0, 4.0), 8.0);
        let bounds = shadow.bounds(Rect::new(0.0, 0.0, 100.0, 100.0));

        // The box is at 0..100, the shadow is displaced 4 down, and the blur
        // reaches 12 either side of that — so the band is -8..116, not
        // -12..112. Getting this wrong in the *test* is exactly how an
        // under-reported blur ships.
        assert_eq!(bounds.top, -8.0, "4 down, then 12 of blur back up");
        assert_eq!(bounds.bottom, 116.0, "4 down, then 12 of blur further down");
        assert!(
            bounds.left < 0.0 && bounds.right > 100.0,
            "a blur spreads sideways as well: {bounds}"
        );
    }

    #[test]
    fn spread_grows_the_shadow_before_it_is_blurred() {
        let plain = Shadow::new(Color::BLACK, Offset::ZERO, 0.0);
        let spread = plain.spread(10.0);
        let box_bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(plain.bounds(box_bounds), box_bounds);
        assert_eq!(
            spread.bounds(box_bounds),
            Rect::new(-10.0, -10.0, 110.0, 110.0)
        );
    }

    #[test]
    fn an_inset_shadow_never_paints_outside_the_box_it_is_cast_in() {
        let box_bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let inset = Shadow::inset(Color::BLACK, Offset::new(0.0, 6.0), 20.0).spread(8.0);
        assert_eq!(inset.bounds(box_bounds), box_bounds);
    }

    #[test]
    fn an_inset_shadow_keeps_the_colour_offset_and_blur_it_was_given() {
        let outer = Shadow::new(Color::BLACK, Offset::new(1.0, 4.0), 8.0);
        let inset = Shadow::inset(Color::BLACK, Offset::new(1.0, 4.0), 8.0);
        assert!(inset.is_inset && !outer.is_inset);
        assert_eq!(
            Shadow {
                is_inset: false,
                ..inset
            },
            outer
        );
    }

    #[test]
    fn blur_is_two_standard_deviations() {
        assert_eq!(Shadow::new(Color::BLACK, Offset::ZERO, 8.0).std_dev(), 4.0);
    }

    #[test]
    fn fading_rounds_rather_than_truncating() {
        // 255 * 0.5 is 127.5. Truncating gives 127, which drifts visibly darker
        // over a nested fade.
        assert_eq!(fade(Color::BLACK, 0.5).a, 128);
        assert_eq!(fade(Color::BLACK, 1.0).a, 255);
        assert_eq!(fade(Color::BLACK, 0.0).a, 0);
    }
}

/// How an open subpath's ends are finished.
///
/// # Why this is in the foundation crate
///
/// Because two layers need to name it and neither may depend on the other:
/// [`Sketch::Stroke`](crate::Sketch) describes a stroke to be recorded, and
/// `vieww_paint::Stroke` describes one to be rasterised. A second copy of this
/// enum in the paint crate would be two vocabularies for one idea, which is the
/// defect the framework's duplicated `Biometrics` traits were.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum StrokeCap {
    /// Cut off square, exactly at the end point. The default, and what a
    /// width-only stroke always was.
    #[default]
    Butt,
    /// A half-disc centred on the end point.
    Round,
    /// Square, extended by half the width past the end point.
    Square,
}

/// How a corner between two segments is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum StrokeJoin {
    /// The outer edges extended until they meet, subject to a miter limit.
    #[default]
    Miter,
    /// An arc of the stroke's radius.
    Round,
    /// The gap cut straight across.
    Bevel,
}

/// A dash pattern: alternating on and off lengths, and where to start in them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dash {
    /// On, off, on, off — in logical pixels. An odd-length pattern repeats with
    /// the roles swapped, which is what SVG and Canvas both do.
    pub pattern: Vec<f32>,
    /// How far into the pattern each subpath begins.
    pub offset: f32,
}

impl Dash {
    /// A pattern starting at its beginning.
    #[must_use]
    pub fn new(pattern: impl Into<Vec<f32>>) -> Self {
        Self {
            pattern: pattern.into(),
            offset: 0.0,
        }
    }

    /// Evenly spaced dashes of `length`.
    #[must_use]
    pub fn even(length: f32) -> Self {
        Self::new(vec![length, length])
    }

    /// A dotted line: zero-length dashes, which draw as caps.
    ///
    /// Only visible with a [`Round`](StrokeCap::Round) or
    /// [`Square`](StrokeCap::Square) cap — a zero-length butt-capped dash has no
    /// area and draws nothing at all. That is the correct answer and a
    /// surprising one, so it is said here rather than found.
    #[must_use]
    pub fn dotted(gap: f32) -> Self {
        Self::new(vec![0.0, gap])
    }

    /// Start `offset` into the pattern.
    #[must_use]
    pub fn offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    /// `true` if this pattern would change nothing.
    ///
    /// An empty pattern is **solid**, not invisible: a caller building one from
    /// data can legitimately produce an empty one, and a line that vanished
    /// would be the more surprising of the two answers.
    #[must_use]
    pub fn is_solid(&self) -> bool {
        self.pattern.is_empty() || self.pattern.iter().all(|&length| length <= 0.0)
    }
}

/// Everything about a stroke except how thick it is.
///
/// Separate from the width because the width is the thing every call site
/// already passes and the rest is what almost none of them care about:
/// `StrokeStyle::default()` is butt caps, miter joins and no dashes, which is
/// exactly what a width-only stroke always drew.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StrokeStyle {
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    /// How far a [`Miter`](StrokeJoin::Miter) may extend, in multiples of the
    /// width, before it is cut back to a bevel.
    ///
    /// `0.0` means the default, which is four — SVG's and Skia's. A corner
    /// sharper than about 29 degrees bevels; anything blunter miters. Without a
    /// limit a near-parallel join fires a spike across the screen.
    pub miter_limit: f32,
    pub dash: Option<Dash>,
}

impl StrokeStyle {
    /// The default: butt caps, miter joins, solid.
    #[must_use]
    pub fn plain() -> Self {
        Self::default()
    }

    /// Round on both the caps and the joins — what a drawn line looks like, and
    /// what most vector UI wants.
    #[must_use]
    pub fn rounded() -> Self {
        Self {
            cap: StrokeCap::Round,
            join: StrokeJoin::Round,
            ..Self::default()
        }
    }

    #[must_use]
    pub const fn cap(mut self, cap: StrokeCap) -> Self {
        self.cap = cap;
        self
    }

    #[must_use]
    pub const fn join(mut self, join: StrokeJoin) -> Self {
        self.join = join;
        self
    }

    #[must_use]
    pub const fn miter_limit(mut self, limit: f32) -> Self {
        self.miter_limit = limit;
        self
    }

    #[must_use]
    pub fn dash(mut self, dash: Dash) -> Self {
        self.dash = Some(dash);
        self
    }

    /// The miter limit to actually use: the declared one, or the default four.
    #[must_use]
    pub fn effective_miter_limit(&self) -> f32 {
        if self.miter_limit > 0.0 {
            self.miter_limit
        } else {
            4.0
        }
    }

    /// How far past the path's own geometry this style's ink can reach, in
    /// multiples of half the width.
    ///
    /// **Not one.** A square cap extends a further half-width past the end of a
    /// line, and a miter can reach `miter_limit` half-widths out from a sharp
    /// corner. A paint-bounds calculation that assumed one leaves the tip of
    /// every arrowhead and the point of every chevron outside the damaged
    /// region — ink drawn once and never repainted, which on a persistent
    /// surface is a mark that stays until something else happens to cover it.
    #[must_use]
    pub fn reach_factor(&self) -> f32 {
        let capped = match self.cap {
            StrokeCap::Butt | StrokeCap::Round => 1.0,
            StrokeCap::Square => std::f32::consts::SQRT_2,
        };
        let joined = match self.join {
            StrokeJoin::Round | StrokeJoin::Bevel => 1.0,
            StrokeJoin::Miter => self.effective_miter_limit().max(1.0),
        };
        capped.max(joined)
    }
}
