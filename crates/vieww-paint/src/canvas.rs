use vieww_foundation::{
    BlendMode, Color, Dash, GlyphRun, Gradient, Offset, Path, Rect, Shadow, StrokeCap, StrokeJoin,
    StrokeStyle, Transform,
};

/// Re-exported so a backend never has to know which crate the pixels came from.
///
/// [`Image`] itself lives in `vieww-foundation`, for the reason [`Path`] does:
/// the widget layer has to be able to name what it is describing, and it sits
/// below this crate.
pub use vieww_foundation::Image;

/// How a shape is filled.
///
/// A struct rather than a bare [`Color`] so that gaining a way to fill does not
/// change every call site — which is exactly what happened when it gained
/// [`gradient`](Self::gradient), and the reason the shape was chosen.
///
/// # `color` is always meaningful, even under a gradient
///
/// A gradient paint keeps `color` set to the ramp's
/// [representative](Gradient::representative) colour rather than leaving it
/// transparent. Three things depend on that and would each have needed a special
/// case otherwise: [`is_invisible`](Self::is_invisible), damage comparison, and
/// every backend that cannot draw a gradient — which falls back to a flat fill
/// and produces a worse picture rather than no picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paint {
    /// The solid fill, and the flat stand-in for a gradient.
    pub color: Color,
    /// A ramp of colours filling the shape's own bounds.
    pub gradient: Option<Gradient>,
}

impl Paint {
    /// A solid-colour paint.
    #[must_use]
    pub const fn solid(color: Color) -> Self {
        Self {
            color,
            gradient: None,
        }
    }

    /// A paint that fills with a ramp of colours.
    ///
    /// The ramp is resolved against the bounds of whatever shape it fills, so
    /// one `Paint` describes a button without knowing how wide layout made it.
    #[must_use]
    pub fn gradient(gradient: Gradient) -> Self {
        Self {
            color: gradient.representative(),
            gradient: Some(gradient),
        }
    }

    /// `true` if painting with this would change nothing.
    #[must_use]
    pub fn is_invisible(self) -> bool {
        match self.gradient {
            Some(gradient) => gradient.is_invisible(),
            None => self.color.is_transparent(),
        }
    }
}

impl From<Color> for Paint {
    fn from(color: Color) -> Self {
        Self::solid(color)
    }
}

impl From<Gradient> for Paint {
    fn from(gradient: Gradient) -> Self {
        Self::gradient(gradient)
    }
}

/// How a shape's outline is drawn.
///
/// # The vocabulary, and what width-only cost
///
/// This was width alone, on the argument that nothing in the widget library
/// needed more. That was true of the widget library and not of the thing the
/// widget library is for: a custom painter, a vector icon, a chart's own
/// geometry. Every one of those wants a round cap on a line end, a round join
/// on a polyline, or a dashed rule — and with none of them expressible, an
/// application drawing a dashed selection outline had to build it out of
/// individual segments and place each one itself.
///
/// The defaults are exactly what a width-only stroke always was — butt caps,
/// miter joins, no dashes — so nothing that existed before this changes shape.
///
/// # `reach` is no longer half the width
///
/// A square cap extends past the end of the line and a miter extends past a
/// corner, so the ink can land further out than `width / 2.0`. See
/// [`reach`](Self::reach): getting that wrong leaves the tip of every arrowhead
/// outside the damaged area, which is ink drawn once and never repainted.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    /// Thickness in logical pixels, centred on the path.
    pub width: f32,
    /// Caps, joins, miter limit and dashes.
    ///
    /// [`StrokeStyle::default()`](vieww_foundation::StrokeStyle) is butt caps,
    /// miter joins and no dashes — exactly what a width-only stroke always
    /// drew, so nothing built before this changes shape.
    pub style: StrokeStyle,
}

impl Stroke {
    #[must_use]
    pub fn new(width: f32) -> Self {
        Self {
            width,
            style: StrokeStyle::default(),
        }
    }

    /// A hairline — the thinnest stroke that is still a line.
    #[must_use]
    pub fn hairline() -> Self {
        Self::new(1.0)
    }

    /// Draw with `style`.
    #[must_use]
    pub fn styled(mut self, style: StrokeStyle) -> Self {
        self.style = style;
        self
    }

    #[must_use]
    pub fn cap(mut self, cap: StrokeCap) -> Self {
        self.style.cap = cap;
        self
    }

    #[must_use]
    pub fn join(mut self, join: StrokeJoin) -> Self {
        self.style.join = join;
        self
    }

    #[must_use]
    pub fn miter_limit(mut self, limit: f32) -> Self {
        self.style.miter_limit = limit;
        self
    }

    #[must_use]
    pub fn dash(mut self, dash: Dash) -> Self {
        self.style.dash = Some(dash);
        self
    }

    /// Round on both the caps and the joins.
    #[must_use]
    pub fn rounded(self) -> Self {
        self.styled(StrokeStyle::rounded())
    }

    /// `true` if stroking with this would change nothing.
    #[must_use]
    pub fn is_invisible(&self) -> bool {
        self.width <= 0.0
    }

    /// How far the ink reaches either side of the path itself.
    ///
    /// **Not simply half the width.** A square cap extends a further half-width
    /// past the end of a line and a miter can reach `miter_limit` half-widths
    /// out from a sharp corner, so this is what the paint bounds have to be
    /// grown by. Reporting the smaller number leaves the tip of every arrowhead
    /// outside the damaged area — ink drawn once and never repainted.
    #[must_use]
    pub fn reach(&self) -> f32 {
        self.width / 2.0 * self.style.reach_factor()
    }
}

/// A surface render objects draw onto.
///
/// Shaped after Skia's canvas and the classic canvas models on top of
/// it, because the model is proven and because it maps onto every backend worth
/// having: a GPU renderer, a CPU rasteriser, or a recorder.
///
/// # The state stack
///
/// [`save`](Canvas::save) and [`restore`](Canvas::restore) bracket changes to
/// the transform and the clip. Every implementation must treat the pair as
/// strictly nested — a `restore` with no matching `save` is a bug, not a no-op
/// — because unbalanced state is the classic way for one widget's clip to
/// silently truncate an unrelated part of the screen three levels away.
///
/// [`push_layer`](Canvas::push_layer) and [`pop_layer`](Canvas::pop_layer) are
/// the same discipline for compositing, and are a *separate* stack: a layer
/// composites its contents as a group, which save/restore never does.
pub trait Canvas {
    /// Push the current transform and clip onto the state stack.
    fn save(&mut self);

    /// Pop the most recently saved transform and clip.
    fn restore(&mut self);

    /// Multiply the current transform.
    fn transform(&mut self, transform: Transform);

    /// Shorthand for translating the current transform.
    fn translate(&mut self, offset: Offset) {
        self.transform(Transform::translate(offset));
    }

    /// Intersect the clip with `rect`, in the current coordinate space.
    fn clip_rect(&mut self, rect: Rect);

    /// Intersect the clip with a rounded rectangle.
    ///
    /// The common case by a wide margin — a card, an avatar, a thumbnail with
    /// the corners taken off — and the one that was missing for long enough
    /// that widgets worked around it by painting a rounded shape *over* their
    /// content instead of clipping it, which only works against a known
    /// background.
    ///
    /// A backend that cannot clip to a curve should fall back to the bounding
    /// rectangle: too little clipped is a worse picture, but a picture.
    fn clip_rrect(&mut self, rect: Rect, radius: f32) {
        self.clip_path(&Path::rounded_rect(rect, radius));
    }

    /// Intersect the clip with an arbitrary path, non-zero winding.
    ///
    /// The default implementation clips to the path's bounding box, which is
    /// correct only for a rectangle. Every real backend overrides it; the
    /// default exists so a test double is not obliged to rasterise.
    fn clip_path(&mut self, path: &Path) {
        self.clip_rect(path.bounds());
    }

    /// Fade everything drawn until the matching [`restore`](Self::restore).
    ///
    /// `alpha` is 0.0 to 1.0 and multiplies into whatever is already in force,
    /// so nesting two halves gives a quarter.
    ///
    /// # This is per-primitive alpha, not group opacity
    ///
    /// The distinction is real and worth stating rather than discovering. Group
    /// opacity composites the subtree and *then* fades the result; this fades
    /// each primitive as it is recorded. Where children overlap, the two
    /// differ — text over its own background at 50% shows the background
    /// through the text, because both were faded independently rather than
    /// together.
    ///
    /// It is kept because it is *free*: no compositing target, no second pass,
    /// and the resolved colour still lets a command be inspected on its own.
    /// For content that does not overlap itself — which is most content — it is
    /// the right tool. When children do overlap, use
    /// [`push_layer`](Self::push_layer), which costs a real layer and gets it
    /// right.
    ///
    /// A backend that cannot fade at all should ignore this rather than
    /// refusing to draw.
    fn push_alpha(&mut self, alpha: f32) {
        let _ = alpha;
    }

    /// Begin compositing everything up to the matching
    /// [`pop_layer`](Self::pop_layer) as one group.
    ///
    /// This is the honest version of opacity and the only way to blend: the
    /// contents are drawn into a target of their own, and *that* is then faded
    /// by `alpha` and combined with what is behind it using `blend`. Overlapping
    /// children fade together, which is what "50% opacity" means to everyone who
    /// is not implementing it.
    ///
    /// `bounds` is an **estimate** of the area the group will affect, in the
    /// current coordinate space, for a backend that has to size a compositing
    /// target before it knows what goes into it. It is not a clip: a recorder
    /// replaces it with the true extent of the contents when the group closes,
    /// so a child painting outside it — a shadow, most often — is not cut off.
    /// Pass the subtree's own bounds and do not agonise over it.
    ///
    /// A backend with no compositing support should push nothing and let the
    /// contents draw straight through, which loses the grouping but keeps the
    /// picture.
    ///
    /// # Panics
    ///
    /// Implementations panic on an unmatched [`pop_layer`](Self::pop_layer), for
    /// the reason [`restore`](Self::restore) does.
    fn push_layer(&mut self, bounds: Rect, alpha: f32, blend: BlendMode) {
        let (_, _, _) = (bounds, alpha, blend);
    }

    /// Begin a group whose pixels go through `filter` before compositing.
    ///
    /// Defaults to an unfiltered [`push_layer`](Self::push_layer), so a canvas
    /// that does not implement filtering still draws the group — visibly
    /// unfiltered, rather than dropped. That is the same choice
    /// `SceneReport::skipped_text` made: degrade to something honest and count
    /// it, never to nothing.
    fn push_filtered_layer(
        &mut self,
        bounds: Rect,
        alpha: f32,
        blend: BlendMode,
        filter: vieww_foundation::ImageFilter,
    ) {
        let _ = filter;
        self.push_layer(bounds, alpha, blend);
    }

    /// Composite the layer opened by the last [`push_layer`](Self::push_layer).
    fn pop_layer(&mut self) {}

    /// Fill an axis-aligned rectangle.
    fn fill_rect(&mut self, rect: Rect, paint: Paint);

    /// Fill a rounded rectangle.
    fn fill_rrect(&mut self, rect: Rect, radius: f32, paint: Paint) {
        self.fill_path(&Path::rounded_rect(rect, radius), paint);
    }

    /// Fill an arbitrary path, non-zero winding.
    fn fill_path(&mut self, path: &Path, paint: Paint);

    /// Draw a path's outline.
    ///
    /// Centred on the path, so the ink reaches [`Stroke::reach`] either side of
    /// it. That is the convention every 2D API uses, and the one that makes a
    /// stroked circle of radius *r* actually have radius *r*.
    ///
    /// The default implementation fills nothing — a recorder must override it.
    /// It is a default rather than a required method so that the several test
    /// canvases in this workspace did not all have to grow a body the day this
    /// arrived.
    fn stroke_path(&mut self, path: &Path, stroke: Stroke, paint: Paint) {
        let (_, _, _) = (path, stroke, paint);
    }

    /// Draw a rectangle's outline.
    fn stroke_rect(&mut self, rect: Rect, stroke: Stroke, paint: Paint) {
        self.stroke_path(&Path::rect(rect), stroke, paint);
    }

    /// Draw a rounded rectangle's outline.
    fn stroke_rrect(&mut self, rect: Rect, radius: f32, stroke: Stroke, paint: Paint) {
        self.stroke_path(&Path::rounded_rect(rect, radius), stroke, paint);
    }

    /// Cast a blurred shadow behind a rounded rectangle.
    ///
    /// Takes the shape casting the shadow rather than the shadow's own
    /// geometry, because [`Shadow`] is described relative to its caster — offset
    /// from it, spread around it — and resolving that in one place stops two
    /// widgets disagreeing about which direction "down" was.
    ///
    /// Rounded rectangles only, deliberately. A blurred *arbitrary path* means
    /// a real separable blur over a rasterised mask, which is a different and
    /// much heavier operation; a rounded rectangle has a closed-form
    /// approximation every GPU backend implements directly. Every shadow in a
    /// user interface is cast by a box.
    fn draw_shadow(&mut self, rect: Rect, radius: f32, shadow: Shadow) {
        let (_, _, _) = (rect, radius, shadow);
    }

    /// Draw a run of already-shaped glyphs.
    ///
    /// Takes positioned glyphs rather than a string because by this point every
    /// question of script, direction, kerning and line breaking is settled. A
    /// canvas that accepted text would have to shape it in order to draw it, and
    /// would reach different answers than layout already did — which is how text
    /// ends up overflowing the box that was measured for it.
    ///
    /// The run's origin sits on the **baseline**, not the top-left. See
    /// [`GlyphRun`].
    fn draw_glyphs(&mut self, run: &GlyphRun);

    /// Draw an image scaled into `rect`.
    fn draw_image(&mut self, rect: Rect, image: &Image);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Gradient;

    #[test]
    fn a_gradient_paint_still_reports_a_colour() {
        let paint = Paint::gradient(Gradient::vertical().between(Color::RED, Color::BLUE));
        assert!(
            !paint.color.is_transparent(),
            "a backend that cannot draw a ramp falls back to this, and damage \
             compares it"
        );
    }

    #[test]
    fn a_gradient_with_no_stops_is_invisible() {
        assert!(Paint::gradient(Gradient::vertical()).is_invisible());
    }

    #[test]
    fn a_solid_paints_visibility_is_still_its_alpha() {
        assert!(Paint::solid(Color::TRANSPARENT).is_invisible());
        assert!(!Paint::solid(Color::RED).is_invisible());
    }

    /// **A stroke does not always reach half its width, and assuming it did was
    /// the bug.**
    ///
    /// This asserted `reach() == width / 2.0` unconditionally, which is true of
    /// a butt-capped, round-joined stroke and of nothing else. A square cap
    /// extends past the end of a line; a miter extends past a sharp corner, by
    /// as much as the miter limit allows. Paint bounds grown by the smaller
    /// number leave the point of every chevron and the tip of every arrowhead
    /// outside the damaged region — ink drawn once and, on a persistent
    /// surface, never repainted.
    ///
    /// The cost of being right is a few pixels of extra damage around every
    /// mitered stroke, on rectangles that are already inflated by `AA_BLEED` and
    /// rounded outward. The cost of being wrong is a mark that stays on screen.
    #[test]
    fn a_strokes_reach_accounts_for_its_caps_and_joins() {
        // Round joins and butt caps: the ink genuinely stops at half the width.
        let plain = Stroke::new(4.0).join(StrokeJoin::Round);
        assert_eq!(plain.reach(), 2.0);

        // A round cap is a disc of the same radius — still half the width.
        assert_eq!(plain.clone().cap(StrokeCap::Round).reach(), 2.0);

        // A square cap reaches to the corner of a square of that half-width,
        // which is further than the edge of it.
        assert!(plain.clone().cap(StrokeCap::Square).reach() > 2.0);

        // A miter reaches as far as its limit allows, and that is the default.
        assert_eq!(Stroke::new(4.0).reach(), 2.0 * 4.0);
        assert_eq!(Stroke::new(4.0).miter_limit(2.0).reach(), 2.0 * 2.0);

        assert!(Stroke::new(0.0).is_invisible());
    }

    /// An empty dash pattern is a solid line rather than an invisible one — a
    /// caller building one from data can legitimately produce it.
    #[test]
    fn an_empty_dash_pattern_is_solid() {
        assert!(Dash::new(Vec::new()).is_solid());
        assert!(Dash::new(vec![0.0, 0.0]).is_solid());
        assert!(!Dash::even(4.0).is_solid());
    }
}
