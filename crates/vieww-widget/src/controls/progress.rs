use std::any::Any;
use std::fmt;
use std::time::Duration;

use vieww_animation::{AnimationController, Curve};
use vieww_foundation::{BoxDecoration, Color, Key};

use crate::{
    widget_node_from, BuildContext, ColorScheme, DecoratedBox, ElementState, Flex, Flexible,
    SemanticRole, Semantics, SizedBox, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// How thick the bar is, unless asked otherwise.
///
/// Four logical pixels: thin enough to sit under an app bar without being
/// furniture, thick enough that the rounding at each end is visible.
const THICKNESS: f32 = 4.0;

/// How much of the track an indeterminate bar's travelling segment covers.
///
/// Three tenths. A shorter segment reads as a dot wandering about; a longer one
/// barely appears to move, because the ends spend most of the cycle clipped.
const SEGMENT: f32 = 0.3;

/// How long one sweep of an indeterminate bar takes.
///
/// Longer than a control's fade, because this is not feedback about something
/// the user just did — it is a sign of life, and a sign of life that hurries
/// reads as agitation.
const SWEEP: Duration = Duration::from_millis(1400);

/// Flex factors are integers, so a fraction is expressed in shares of this.
///
/// A thousand rather than a hundred: at a hundred, a bar animating across a
/// wide track visibly steps, because one share is a whole pixel on a phone.
const SHARES: f32 = 1000.0;

/// A bar that fills as something progresses, or sweeps while it cannot say.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::LinearProgress;
///
/// // Two thirds done.
/// let downloading = LinearProgress::new(0.66).label("Downloading");
///
/// // Working, with no idea how much is left — which is the common case, and
/// // what an `AsyncBuilder`'s `Pending` branch wants.
/// let loading = LinearProgress::indeterminate();
/// ```
///
/// # Determinate and indeterminate are one widget
///
/// They are the same thing with and without a number, and a screen that starts
/// indeterminate and becomes determinate — a download that learns its
/// content-length — should not swap one widget for another to say so. Swapping
/// would also replace the element and restart the animation, which is visible.
///
/// # Why it is composed rather than painted
///
/// A rectangle whose width is a fraction of its parent is a flex with two
/// children, and `Flex` already solves that including the rounding and the
/// direction. A render object would mean registering it in `vieww-render`'s
/// factory, and `controls/mod.rs` explains why nothing in this module does
/// that: an application must be able to write its own controls the same way.
///
/// The one cost is that fractions become integer flex shares — see `SHARES`.
#[derive(Clone)]
pub struct LinearProgress {
    /// `None` while there is nothing honest to report.
    value: Option<f32>,
    thickness: f32,
    label: Option<String>,
    key: Option<Key>,
}

impl LinearProgress {
    /// A bar showing `value`, clamped to `0.0 ..= 1.0`.
    ///
    /// Clamped rather than asserted: progress is usually a division, and
    /// `downloaded / total` produces `NaN` for an empty file and numbers above
    /// one for a server that under-reports. Neither deserves a panic in a
    /// loading indicator, so a non-finite value reads as zero.
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self {
            value: Some(if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }),
            thickness: THICKNESS,
            label: None,
            key: None,
        }
    }

    /// A bar that sweeps, for work whose size is unknown.
    #[must_use]
    pub const fn indeterminate() -> Self {
        Self {
            value: None,
            thickness: THICKNESS,
            label: None,
            key: None,
        }
    }

    /// How thick the bar is, in logical pixels.
    #[must_use]
    pub const fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self
    }

    /// What a screen reader calls this. The *percentage* is reported without
    /// being asked — this is the name of the thing being waited for.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The fraction filled, or `None` when it is not known.
    #[must_use]
    pub const fn value(&self) -> Option<f32> {
        self.value
    }

    #[must_use]
    pub const fn is_indeterminate(&self) -> bool {
        self.value.is_none()
    }

    /// The three shares an indeterminate bar is divided into at `phase`.
    ///
    /// `(before, segment, after)`, summing to `SHARES`. Pure, and separated
    /// from the widget for exactly that reason: where the segment is at a given
    /// moment is arithmetic that can be wrong, and needs no tree to check.
    ///
    /// The segment travels the *free* part of the track — `1 - SEGMENT` — so it
    /// starts flush with the leading edge and finishes flush with the trailing
    /// one, rather than sliding off either end.
    #[must_use]
    pub fn sweep(phase: f32) -> (u16, u16, u16) {
        let phase = if phase.is_finite() {
            phase.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let segment = (SEGMENT * SHARES) as u16;
        // Rounded rather than truncated: truncation biases every position
        // downwards, so the segment reaches the far end a frame late.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a share of a thousand is a small positive number"
        )]
        let before = (phase * (1.0 - SEGMENT) * SHARES).round() as u16;
        // Subtracted rather than computed, so the three always sum to SHARES
        // whatever the rounding did.
        let after = (SHARES as u16) - before - segment;
        (before, segment, after)
    }

    /// The two shares a determinate bar is divided into.
    ///
    /// `(filled, empty)`, summing to `SHARES`.
    ///
    /// **A `NaN` value survives this by accident, not by design.**
    /// `f32::clamp` *propagates* NaN rather than bounding it — it is the float
    /// cast below that rescues this, because a float-to-integer cast saturates
    /// and NaN saturates to zero. `RenderCircularProgress` had the same clamp,
    /// no cast to rescue it, and produced an arc with NaN coordinates; see its
    /// `geometry`. If this ever stops going through an integer, guard it.
    #[must_use]
    pub fn split(value: f32) -> (u16, u16) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a clamped fraction of a thousand is a small positive number"
        )]
        let filled = (value.clamp(0.0, 1.0) * SHARES).round() as u16;
        (filled, (SHARES as u16) - filled)
    }

    /// One share of the bar, filled or not.
    ///
    /// A zero-share child is omitted rather than added with a factor of zero:
    /// an empty flex child still costs an element, and a bar at 0% or 100% is
    /// the common case at both ends of every load.
    fn piece(shares: u16, fill: Option<BoxDecoration>, thickness: f32) -> Option<WidgetNode> {
        if shares == 0 {
            return None;
        }
        let content: WidgetNode = match fill {
            Some(decoration) => DecoratedBox::new(decoration)
                .child(SizedBox::height(thickness))
                .into(),
            None => SizedBox::height(thickness).into(),
        };
        Some(Flexible::expanded(shares).child(content).into())
    }

    /// The bar itself, at `phase` if it is indeterminate.
    fn bar(&self, theme: &ThemeData, phase: f32) -> WidgetNode {
        let colors = theme.colors;
        // The same rounding at both ends whatever the thickness, so a thick bar
        // is a lozenge and a thin one is a line with soft ends.
        let fill = BoxDecoration::filled(colors.primary).radius(self.thickness / 2.0);

        let pieces: Vec<Option<WidgetNode>> = match self.value {
            Some(value) => {
                let (filled, empty) = Self::split(value);
                vec![
                    Self::piece(filled, Some(fill), self.thickness),
                    Self::piece(empty, None, self.thickness),
                ]
            }
            None => {
                let (before, segment, after) = Self::sweep(phase);
                vec![
                    Self::piece(before, None, self.thickness),
                    Self::piece(segment, Some(fill), self.thickness),
                    Self::piece(after, None, self.thickness),
                ]
            }
        };
        let pieces: Vec<WidgetNode> = pieces.into_iter().flatten().collect();

        // The track under it, which is the primary colour at low opacity rather
        // than a palette entry of its own: a track is *the same bar, not yet
        // reached*, and giving it its own colour is how a theme ends up with a
        // progress track that no longer matches its progress.
        DecoratedBox::new(
            BoxDecoration::filled(ColorScheme::dimmed(colors.primary)).radius(self.thickness / 2.0),
        )
        .child(SizedBox::height(self.thickness).child(Flex::row().children(pieces)))
        .into()
    }
}

impl Widget for LinearProgress {
    fn debug_name(&self) -> &'static str {
        "LinearProgress"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// State only when there is something to animate.
    ///
    /// A determinate bar is a function of its value and nothing else, so it
    /// costs no state, no tick and no frames — which matters, because
    /// `is_animating` below is unconditionally true for the ones that do have
    /// state, and a screen full of finished progress bars must still be allowed
    /// to idle.
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        self.is_indeterminate()
            .then(|| Box::new(SweepState::default()) as Box<dyn ElementState>)
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let phase = ctx
            .state::<SweepState, _>(|state| state.phase())
            .unwrap_or(0.0);

        let mut semantics = Semantics::new().role(SemanticRole::ProgressBar);
        if let Some(label) = &self.label {
            semantics = semantics.label(label.clone());
        }
        // Read aloud as the percentage, because "progress indicator" on its own
        // tells a screen reader user that something is happening and nothing
        // about whether it is worth waiting for. An indeterminate bar says so
        // in words rather than reporting a number it does not have.
        semantics = semantics.value(match self.value {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a clamped fraction times a hundred is 0..=100"
            )]
            Some(value) => format!("{}%", (value * 100.0).round() as u32),
            None => String::from("in progress"),
        });

        semantics.child(self.bar(&theme, phase)).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "value",
                self.value
                    .map_or_else(|| String::from("indeterminate"), |value| value.to_string()),
            ),
            ("thickness", self.thickness.to_string()),
        ]
    }
}

/// Hand-written to match the rest of the module; the label is the only part a
/// tree dump wants that `debug_properties` does not already carry.
impl fmt::Debug for LinearProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinearProgress")
            .field("value", &self.value)
            .field("thickness", &self.thickness)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

widget_node_from!(LinearProgress);

/// Where an indeterminate bar's segment has got to.
///
/// Follows `PressState`'s shape, including the deferred start: beginning the
/// animation needs a `now`, and `create_state` has no honest source of one —
/// the frame does.
#[derive(Debug)]
pub struct SweepState {
    controller: AnimationController,
    started: bool,
}

impl Default for SweepState {
    fn default() -> Self {
        Self {
            // Ease-in-out, and repeating without reversing. The segment always
            // travels the same way, slowing at each end — a bar that ran back
            // and forth would read as a slider being dragged by nobody.
            controller: AnimationController::new(SWEEP).curve(Curve::EASE_IN_OUT),
            started: false,
        }
    }
}

impl SweepState {
    /// How far through one sweep, `0.0 ..= 1.0`.
    #[must_use]
    pub const fn phase(&self) -> f32 {
        self.controller.value()
    }
}

impl ElementState for SweepState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn tick(&mut self, now: Duration) -> bool {
        if !self.started {
            self.started = true;
            // `false`: forward every cycle, never in reverse.
            self.controller.repeat(false, now);
        }
        self.controller.tick(now)
    }

    /// Always. An indeterminate bar that stopped asking for frames would freeze
    /// mid-sweep, which looks precisely like the hang it exists to deny.
    ///
    /// This is why `create_state` returns `None` for a determinate bar: the
    /// cost of this answer is that the application never idles, and only a bar
    /// that is actually sweeping should impose it.
    fn is_animating(&self) -> bool {
        true
    }
}

// ------------------------------------------------------------------ circular

/// How wide a spinner is, unless asked otherwise.
///
/// Thirty-six logical pixels: smaller than a touch target, because nobody taps
/// a spinner, and large enough that a four-pixel band still reads as a ring
/// rather than a smudge.
const DIAMETER: f32 = 36.0;

/// How long one cycle of an indeterminate spinner takes.
///
/// One cycle is one full turn of the tail *and* one grow-and-shrink of the
/// sweep, because both come out of the same phase — see
/// `RenderCircularProgress::geometry`.
const SPIN: Duration = Duration::from_millis(1400);

/// A ring that fills as something progresses, or sweeps while it cannot say.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::CircularProgress;
///
/// // Two thirds done.
/// let saving = CircularProgress::new(0.66).label("Saving");
///
/// // Working, with no idea how much is left — what `AsyncValue::Pending` wants.
/// let loading = CircularProgress::indeterminate();
/// ```
///
/// # The same shape as [`LinearProgress`], deliberately
///
/// Determinate and indeterminate are one widget, only the indeterminate one
/// costs state and frames, and a screen reader is told the percentage rather
/// than just "progress indicator". The arguments are the same ones, and are
/// written out there.
///
/// # Where it differs, and why
///
/// **It is not composed.** A bar is two boxes in a row and this is an arc, and
/// nothing in the widget layer draws an arbitrary [`Path`](vieww_foundation::Path).
/// So this is the second control after `Slider` to own a render object.
///
/// **Its animation is linear where the bar's eases.** [`SweepState`] uses
/// `Curve::EASE_IN_OUT`, which is right for a segment travelling a track and
/// wrong here: easing the phase would ease the *rotation*, and a spinner that
/// speeds up and slows down once a second reads as dropped frames. The easing a
/// spinner wants is in the sweep growing and shrinking, which the render object
/// does with `sin²` and needs no curve for.
#[derive(Clone)]
pub struct CircularProgress {
    value: Option<f32>,
    diameter: f32,
    thickness: f32,
    label: Option<String>,
    key: Option<Key>,
}

impl CircularProgress {
    /// A ring filled to `value`, a fraction from zero to one.
    ///
    /// Clamped rather than asserted, for the reason
    /// [`LinearProgress::new`](LinearProgress::new) gives: progress is a
    /// division, and neither `NaN` nor 1.4 deserves a panic inside a loading
    /// indicator.
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self {
            value: Some(if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }),
            diameter: DIAMETER,
            thickness: THICKNESS,
            label: None,
            key: None,
        }
    }

    /// A ring that sweeps, for work whose size is unknown.
    #[must_use]
    pub const fn indeterminate() -> Self {
        Self {
            value: None,
            diameter: DIAMETER,
            thickness: THICKNESS,
            label: None,
            key: None,
        }
    }

    /// How wide the ring is, in logical pixels.
    #[must_use]
    pub const fn size(mut self, diameter: f32) -> Self {
        self.diameter = diameter;
        self
    }

    /// How thick the band is, in logical pixels.
    #[must_use]
    pub const fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self
    }

    /// What a screen reader calls this. The percentage is reported without being
    /// asked.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The fraction filled, or `None` when it is not known.
    #[must_use]
    pub const fn value(&self) -> Option<f32> {
        self.value
    }

    #[must_use]
    pub const fn is_indeterminate(&self) -> bool {
        self.value.is_none()
    }
}

impl Widget for CircularProgress {
    fn debug_name(&self) -> &'static str {
        "CircularProgress"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// State only when there is something to animate — see
    /// [`LinearProgress::create_state`], which explains why this is
    /// load-bearing rather than an optimisation.
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        self.is_indeterminate()
            .then(|| Box::new(SpinState::default()) as Box<dyn ElementState>)
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let phase = ctx
            .state::<SpinState, _>(|state| state.phase())
            .unwrap_or(0.0);

        let mut semantics = Semantics::new().role(SemanticRole::ProgressBar);
        if let Some(label) = &self.label {
            semantics = semantics.label(label.clone());
        }
        semantics = semantics.value(match self.value {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a clamped fraction times a hundred is 0..=100"
            )]
            Some(value) => format!("{}%", (value * 100.0).round() as u32),
            None => String::from("in progress"),
        });

        let arc = CircularProgressArc {
            value: self.value,
            phase,
            diameter: self.diameter,
            thickness: self.thickness,
            // The track is the primary colour dimmed, for the reason the
            // bar's is: a track is *the ring, not yet reached*, and a
            // palette entry of its own is how a theme ends up with a track
            // that no longer matches its indicator.
            track: if theme.platform.is_apple() {
                // Apple's indicator has no track at all — the faded spokes are
                // the track. A ring behind them would read as a second,
                // stationary circle.
                Color::TRANSPARENT
            } else {
                ColorScheme::dimmed(theme.colors.primary)
            },
            indicator: if theme.platform.is_apple() {
                // And it is grey rather than tinted: on iOS the spinner is
                // chrome, not an accent.
                theme.colors.on_surface_variant
            } else {
                theme.colors.primary
            },
            spokes: theme.platform.is_apple(),
            key: None,
        };

        // **An indeterminate spinner gets its own layer, and a determinate one
        // does not.** It repaints on every single frame by definition, and
        // without a boundary that repaint re-records the layer it sits in —
        // which is the whole screen, for a spinner in the middle of one.
        //
        // The same argument the demo's `Pulse` band is built on, and the reason
        // it is here rather than left to the caller: every indeterminate
        // spinner wants this, and a control whose cost depends on remembering
        // to wrap it is a control that is expensive by default.
        //
        // A determinate ring changes only when its value does, so a layer of
        // its own would cost a texture to save nothing.
        let child: WidgetNode = if self.is_indeterminate() {
            crate::RepaintBoundary::new().child(arc).into()
        } else {
            arc.into()
        };

        semantics.child(child).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "value",
                self.value
                    .map_or_else(|| String::from("indeterminate"), |value| value.to_string()),
            ),
            ("diameter", self.diameter.to_string()),
        ]
    }
}

/// Hand-written to match the rest of the module.
impl fmt::Debug for CircularProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircularProgress")
            .field("value", &self.value)
            .field("diameter", &self.diameter)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

widget_node_from!(CircularProgress);

/// The arc itself: a render leaf, the way [`SliderBar`](crate::SliderBar) is.
///
/// Public because the factory in `vieww-render` has to name it, and separate
/// from [`CircularProgress`] because that one reads the theme and owns the
/// animation, which a leaf cannot do.
#[derive(Debug, Clone)]
pub struct CircularProgressArc {
    pub value: Option<f32>,
    pub phase: f32,
    pub diameter: f32,
    pub thickness: f32,
    pub track: Color,
    pub indicator: Color,
    /// Draw the indeterminate form as Apple's ring of fading spokes.
    pub spokes: bool,
    pub key: Option<Key>,
}

impl Widget for CircularProgressArc {
    fn debug_name(&self) -> &'static str {
        "CircularProgressArc"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "value",
                self.value
                    .map_or_else(|| String::from("indeterminate"), |value| value.to_string()),
            ),
            ("diameter", self.diameter.to_string()),
            ("thickness", self.thickness.to_string()),
            // Which of the two indeterminate forms this is. Here rather than
            // left to a screenshot: the Inspector is where somebody asks "why
            // does the preview not look like the phone", and a shape the tree
            // does not mention is a shape nobody can check.
            (
                "form",
                (if self.spokes { "spokes" } else { "arc" }).to_owned(),
            ),
        ]
    }
}

widget_node_from!(CircularProgressArc);

/// Where an indeterminate spinner is in its cycle.
///
/// [`SweepState`] with one difference that matters: **no curve**. See
/// [`CircularProgress`] for why easing this would ease the rotation, which is
/// the one thing about a spinner that should be perfectly steady.
#[derive(Debug)]
pub struct SpinState {
    controller: AnimationController,
    started: bool,
}

impl Default for SpinState {
    fn default() -> Self {
        Self {
            controller: AnimationController::new(SPIN),
            started: false,
        }
    }
}

impl SpinState {
    /// How far through one cycle, `0.0 ..= 1.0`.
    #[must_use]
    pub const fn phase(&self) -> f32 {
        self.controller.value()
    }
}

impl ElementState for SpinState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn tick(&mut self, now: Duration) -> bool {
        if !self.started {
            self.started = true;
            // `false`: forward every cycle. A spinner that reversed would look
            // like it was undoing whatever it is waiting for.
            self.controller.repeat(false, now);
        }
        self.controller.tick(now)
    }

    /// Always, for the reason [`SweepState`] is.
    fn is_animating(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_determinate_bar_splits_the_track_in_proportion() {
        assert_eq!(LinearProgress::split(0.0), (0, 1000));
        assert_eq!(LinearProgress::split(0.25), (250, 750));
        assert_eq!(LinearProgress::split(1.0), (1000, 0));
    }

    #[test]
    fn the_two_shares_always_sum_to_the_whole_track() {
        for step in 0..=100 {
            let value = step as f32 / 100.0;
            let (filled, empty) = LinearProgress::split(value);
            assert_eq!(
                filled + empty,
                1000,
                "a rounding error at {value} would leave a gap at the end of \
                 the bar"
            );
        }
    }

    #[test]
    fn a_nonsense_value_reads_as_empty_rather_than_panicking() {
        assert_eq!(LinearProgress::new(f32::NAN).value(), Some(0.0));
        assert_eq!(
            LinearProgress::new(2.0).value(),
            Some(1.0),
            "a server that under-reports its content-length must not produce a \
             bar wider than its track"
        );
        assert_eq!(LinearProgress::new(-1.0).value(), Some(0.0));
    }

    #[test]
    fn the_sweeping_segment_starts_flush_left_and_ends_flush_right() {
        let (before, segment, after) = LinearProgress::sweep(0.0);
        assert_eq!(before, 0, "it must not start already part-way across");
        assert_eq!(segment, 300);
        assert_eq!(after, 700);

        let (before, segment, after) = LinearProgress::sweep(1.0);
        assert_eq!(after, 0, "nor stop short of the end");
        assert_eq!(segment, 300);
        assert_eq!(before, 700);
    }

    #[test]
    fn the_three_shares_always_sum_to_the_whole_track() {
        for step in 0..=100 {
            let phase = step as f32 / 100.0;
            let (before, segment, after) = LinearProgress::sweep(phase);
            assert_eq!(
                before + segment + after,
                1000,
                "the segment would change width mid-sweep at phase {phase}"
            );
        }
    }

    #[test]
    fn the_segment_only_ever_moves_forwards() {
        let mut last = 0;
        for step in 0..=100 {
            let (before, _, _) = LinearProgress::sweep(step as f32 / 100.0);
            assert!(
                before >= last,
                "a segment that stepped backwards at {step} would stutter"
            );
            last = before;
        }
    }

    #[test]
    fn a_nonsense_phase_does_not_produce_a_nonsense_bar() {
        let (before, segment, after) = LinearProgress::sweep(f32::NAN);
        assert_eq!((before, segment, after), (0, 300, 700));
    }

    #[test]
    fn only_an_indeterminate_bar_costs_state_and_frames() {
        assert!(
            LinearProgress::new(0.5).create_state().is_none(),
            "a finished download must let the application idle"
        );

        let mut state = LinearProgress::indeterminate()
            .create_state()
            .expect("an indeterminate bar animates");
        assert!(state.is_animating());
        assert!(
            state.tick(SWEEP / 4),
            "the first tick starts the repeat and moves the segment"
        );
    }

    #[test]
    fn the_sweep_repeats_rather_than_finishing() {
        let mut state = SweepState::default();
        state.tick(Duration::ZERO);
        state.tick(SWEEP / 2);
        let halfway = state.phase();
        assert!(halfway > 0.0);

        // Well past the end of one cycle.
        state.tick(SWEEP * 3);
        assert!(
            state.is_animating(),
            "a bar that stopped sweeping looks exactly like the hang it exists \
             to deny"
        );
    }

    #[test]
    fn both_kinds_report_something_a_screen_reader_can_say() {
        let determinate = crate::debug_tree(LinearProgress::new(0.42).label("Downloading"));
        assert!(determinate.contains("42%"), "{determinate}");
        assert!(determinate.contains("Downloading"), "{determinate}");

        let indeterminate = crate::debug_tree(LinearProgress::indeterminate());
        assert!(
            indeterminate.contains("in progress"),
            "an indeterminate bar must say so rather than reporting a number \
             it does not have: {indeterminate}"
        );
    }
}
