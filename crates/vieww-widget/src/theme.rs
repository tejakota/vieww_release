//! Colours, type and metrics, published to a subtree.
//!
//! Every control in this crate reads its appearance from here rather than
//! hard-coding one, which is what makes a light and a dark build of the same
//! screen the same widget tree. The mechanism is the one Phase 1 already had —
//! [`Inherited<T>`](crate::Inherited) — so a theme is not a special case in the
//! element tree, just the first thing worth publishing through it.
//!
//! ```
//! use vieww_widget::prelude::*;
//! use vieww_widget::{Theme, ThemeData};
//!
//! let app = Theme::new(ThemeData::dark()).child(Text::new("evening"));
//! ```
//!
//! # Why the values are a plain struct
//!
//! `ColorScheme` and `Typography` are public fields, not a builder with private
//! state, because an application overriding one colour should not have to know
//! which builder method sets it: `ThemeData { colors: ColorScheme { primary,
//! ..scheme }, ..ThemeData::light() }` is the whole API for that, and it works
//! for any field that will ever be added.

use std::rc::Rc;
use std::time::Duration;

use vieww_animation::{Curve, SpringPreset, SpringSpec};
use vieww_foundation::{Color, FontWeight, Key, TargetPlatform, TextStyle};

use crate::{widget_node_from, BuildContext, Inherited, Widget, WidgetKind, WidgetNode};

/// The alpha applied to a colour to say "not available".
///
/// One number, in one place, because a disabled control that dims its label by
/// a different amount than its background is the sort of thing nobody notices
/// until every screen looks slightly wrong.
pub const DISABLED_ALPHA: u8 = 0x61;

/// The alpha of the wash a press draws over a control. The standard 12%.
///
/// One number for the same reason [`DISABLED_ALPHA`] is: a switch that reacts to
/// a finger more strongly than the button beside it reads as a bug in the switch
/// rather than as a choice.
pub const PRESSED_ALPHA: u8 = 0x1F;

/// The colour roles a widget can ask for.
///
/// Roles, not names: a control asks for `on_surface` rather than "dark grey", so
/// that swapping the scheme swaps every control with it. The set is the standard
/// trimmed to what the controls here actually use — a role nothing reads is a
/// role nobody can be sure is right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorScheme {
    /// The accent: a filled button, a checked switch, a slider's track.
    pub primary: Color,
    /// Text and icons drawn on top of [`primary`](Self::primary).
    pub on_primary: Color,
    /// The background a screen is built on.
    pub surface: Color,
    /// Text and icons drawn on top of [`surface`](Self::surface).
    pub on_surface: Color,
    /// A raised or recessed area of the same background — a switch's off track,
    /// a chip, a card.
    pub surface_variant: Color,
    /// Secondary text: a subtitle, a hint, a caption.
    pub on_surface_variant: Color,
    /// Borders and dividers.
    pub outline: Color,
    /// Something has gone wrong.
    pub error: Color,
    /// Text and icons drawn on top of [`error`](Self::error).
    pub on_error: Color,
    /// Something finished, and finished correctly.
    ///
    /// The counterpart to [`error`](Self::error), and it is a role rather than
    /// "green" for the reason the rest of this type is: a confirmation drawn in
    /// a colour chosen at the call site cannot follow the scheme, and green in
    /// particular is the one colour a scheme is most likely to want to change —
    /// it is the hardest to see for the most common form of colour blindness,
    /// which is why nothing here uses colour as the *only* signal.
    pub success: Color,
    /// Text and icons drawn on top of [`success`](Self::success).
    pub on_success: Color,
}

impl ColorScheme {
    /// This scheme with every foreground/background pair driven apart.
    ///
    /// # Why the scheme is where high contrast is applied
    ///
    /// [`Accessibility::high_contrast`](vieww_foundation::Accessibility::high_contrast)
    /// was published by the foundation crate and honoured by nothing, and the
    /// foundation module's own docs said why: *"what 'higher contrast' means is
    /// a property of a palette, and this crate has no palette."* That is
    /// correct, and this is the palette. So the preference is read at
    /// [`ThemeData::of`] — the one funnel every control in the catalogue
    /// already passes through, exactly once per build — and applied here.
    ///
    /// # What it does, and what it deliberately does not
    ///
    /// Every *foreground* role is pushed to the end of the lightness axis that
    /// is furthest from the surface it sits on: on a light scheme text goes to
    /// black, on a dark one to white. Grounds move the other way, but only a
    /// little — a surface driven all the way to pure white is a glare source,
    /// and the platforms' own high-contrast modes do not do it.
    ///
    /// [`outline`](Self::outline) gets the largest change, because it is the
    /// role that fails first: a hairline divider at 1.4:1 is invisible to the
    /// people this setting exists for, and a form whose field boundaries have
    /// disappeared is unusable in a way that slightly grey body text is not.
    ///
    /// **Hue is preserved.** `primary`, `error` and `success` keep their
    /// identity and gain lightness separation from their own `on_` partner;
    /// they are not collapsed to black and white. A high-contrast mode that
    /// throws away the difference between "destructive" and "confirm" has
    /// traded one accessibility failure for another, and colour is not the only
    /// signal here precisely so that it can survive this.
    #[must_use]
    pub fn high_contrast(self) -> Self {
        // Which way "away from the ground" points, decided once from the
        // surface rather than from a `dark` flag the scheme does not carry.
        let dark = self.surface.lightness() < 0.5;
        let (ink, ground) = if dark {
            (Color::WHITE, Color::BLACK)
        } else {
            (Color::BLACK, Color::WHITE)
        };

        Self {
            // Foregrounds go the whole way. Secondary text goes most of the
            // way and not all of it: a subtitle that is exactly as loud as the
            // heading above it is a hierarchy destroyed, and the roles still
            // have to read as two things.
            on_surface: ink,
            on_surface_variant: self.on_surface_variant.lerp_oklab(ink, 0.75),
            on_primary: self.on_primary.lerp_oklab(
                if self.primary.lightness() < 0.5 {
                    Color::WHITE
                } else {
                    Color::BLACK
                },
                1.0,
            ),
            on_error: self.on_error.lerp_oklab(
                if self.error.lightness() < 0.5 {
                    Color::WHITE
                } else {
                    Color::BLACK
                },
                1.0,
            ),
            on_success: self.on_success.lerp_oklab(
                if self.success.lightness() < 0.5 {
                    Color::WHITE
                } else {
                    Color::BLACK
                },
                1.0,
            ),
            // Grounds move a quarter of the way, not all of it. See above.
            surface: self.surface.lerp_oklab(ground, 0.25),
            surface_variant: self.surface_variant.lerp_oklab(ground, 0.25),
            // The role that fails first, and therefore the one that moves most.
            outline: self.outline.lerp_oklab(ink, 0.85),
            // Hues kept; separation gained by moving each towards the end of
            // the axis its own foreground is not at.
            primary: self.primary.lerp_oklab(ink, 0.3),
            error: self.error.lerp_oklab(ink, 0.3),
            success: self.success.lerp_oklab(ink, 0.3),
        }
    }

    /// A neutral light scheme.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            primary: Color::hex(0x2C_62EA),
            on_primary: Color::WHITE,
            surface: Color::hex(0xFF_FBFE),
            on_surface: Color::hex(0x1B_1B1F),
            surface_variant: Color::hex(0xE3_E2E6),
            on_surface_variant: Color::hex(0x46_464F),
            outline: Color::hex(0x77_7680),
            error: Color::hex(0xBA_1A1A),
            on_error: Color::WHITE,
            success: Color::hex(0x1B_6C33),
            on_success: Color::WHITE,
        }
    }

    /// A neutral dark scheme.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            primary: Color::hex(0xAD_C6FF),
            on_primary: Color::hex(0x00_2E69),
            surface: Color::hex(0x1B_1B1F),
            on_surface: Color::hex(0xE4_E1E6),
            surface_variant: Color::hex(0x46_464F),
            on_surface_variant: Color::hex(0xC7_C5D0),
            outline: Color::hex(0x91_8F9A),
            error: Color::hex(0xFF_B4AB),
            on_error: Color::hex(0x69_0005),
            success: Color::hex(0x86_D89B),
            on_success: Color::hex(0x00_3917),
        }
    }

    /// Apple's light scheme, built on the system blue.
    ///
    /// Only the accent and the greys differ from [`light`](Self::light) — the
    /// *roles* are identical, which is the point: a control asks for `primary`
    /// and gets whichever blue the platform's users recognise, without knowing
    /// there are two.
    ///
    /// # Why these are Apple's *increased-contrast* colours, not its default ones
    ///
    /// `vieww-accessibility`'s theme audit is run over this scheme by
    /// `vieww-widget`'s own tests, and against the default palette it failed
    /// four pairs: white on `systemBlue` (4.02:1), white on `systemRed`
    /// (3.55:1), white on `systemGreen` (2.22:1) and the hairline `outline`
    /// against `surface` (1.71:1), each under WCAG AA's 4.5:1 for text or
    /// 1.4.11's 3:1 for a non-text boundary.
    ///
    /// Those numbers are a real property of Apple's default palette, not a
    /// mistake in it: Apple draws those fills under 17pt semibold labels,
    /// which AA judges as *large* text at 3:1, and its separators are
    /// deliberately faint. A framework cannot make that assumption on an
    /// application's behalf — a `Button` here takes whatever label size the
    /// theme's type scale gives it, and `outline` is a text field's border as
    /// well as a divider — so the scheme has to clear the stricter bar.
    ///
    /// The fix is not a hand-tuned palette. Apple **publishes** an
    /// increased-contrast variant of every system colour, for exactly this
    /// reason, and those are what this scheme now uses: `systemBlue` →
    /// `#0040DD` (7.56:1), `systemRed` → `#D70015` (5.38:1), and the greys
    /// step to `systemGray` (3.26:1). Still Apple's colours, still what its
    /// users recognise, chosen from the set Apple ships for legibility rather
    /// than invented here.
    ///
    /// `success` is the one adjustment beyond that set: Apple's accessible
    /// green `#248A3D` measures 4.40:1 under white, which misses 4.5:1 by
    /// six hundredths. It is darkened one step to `#217C37` (5.25:1) rather
    /// than left as the single failing pair in an otherwise passing scheme.
    #[must_use]
    pub const fn apple_light() -> Self {
        Self {
            // systemBlue, increased-contrast variant.
            primary: Color::hex(0x00_40DD),
            on_primary: Color::WHITE,
            surface: Color::WHITE,
            on_surface: Color::hex(0x00_0000),
            surface_variant: Color::hex(0xF2_F2F7),
            on_surface_variant: Color::hex(0x3C_3C43),
            // systemGray, not the fainter separator grey: this role is a text
            // field's border as well as a divider, and 1.4.11 applies to it.
            outline: Color::hex(0x8E_8E93),
            // systemRed, increased-contrast variant.
            error: Color::hex(0xD7_0015),
            on_error: Color::WHITE,
            // systemGreen's increased-contrast variant, one step darker — see
            // this function's doc for the six-hundredths that required it.
            success: Color::hex(0x21_7C37),
            on_success: Color::WHITE,
        }
    }

    /// Apple's dark scheme.
    ///
    /// # Why the labels on the accent fills are black here and white in light
    ///
    /// The same audit failed white on `systemBlue` (3.65:1) and white on
    /// `systemRed` (3.41:1) in dark mode, and the light scheme's fix does not
    /// transfer: Apple's increased-contrast variants get *lighter* on a dark
    /// background, not darker, so keeping a white label makes both pairs
    /// worse rather than better.
    ///
    /// There are two ways out and only one of them survives being looked at.
    /// Darkening the fill until white clears 4.5:1 lands around `#0056D6`,
    /// which measures 2.78:1 against the black `surface` behind it — a button
    /// that is legible only because you already know where it is. Taking
    /// Apple's lighter accessible fill and putting a **black** label on it
    /// gives 7.42:1 for the label *and* 7.42:1 for the button against the
    /// surface.
    ///
    /// So that is what this does, and it is not a new idea in this palette:
    /// `on_success` was already black on `systemGreen` here, for the same
    /// reason, before any of this was measured.
    #[must_use]
    pub const fn apple_dark() -> Self {
        Self {
            // systemBlue dark, increased-contrast variant.
            primary: Color::hex(0x40_9CFF),
            on_primary: Color::hex(0x00_0000),
            surface: Color::hex(0x00_0000),
            on_surface: Color::WHITE,
            surface_variant: Color::hex(0x1C_1C1E),
            on_surface_variant: Color::hex(0xEB_EBF5),
            // systemGray2 dark — the same step up the grey ramp `apple_light`
            // takes, and for the same reason.
            outline: Color::hex(0x63_6366),
            // systemRed dark, increased-contrast variant.
            error: Color::hex(0xFF_6961),
            on_error: Color::hex(0x00_0000),
            success: Color::hex(0x30_D158),
            on_success: Color::hex(0x00_0000),
        }
    }

    /// The light scheme this platform's users recognise.
    #[must_use]
    pub const fn adaptive_light(platform: TargetPlatform) -> Self {
        if platform.is_apple() {
            Self::apple_light()
        } else {
            Self::light()
        }
    }

    /// The dark scheme this platform's users recognise.
    #[must_use]
    pub const fn adaptive_dark(platform: TargetPlatform) -> Self {
        if platform.is_apple() {
            Self::apple_dark()
        } else {
            Self::dark()
        }
    }

    /// `color` at the opacity that means "disabled".
    #[must_use]
    pub const fn dimmed(color: Color) -> Color {
        color.with_alpha(DISABLED_ALPHA)
    }

    /// `base` with a finger on it, where `on` is what is drawn on top of it and
    /// `amount` is how far the press has faded in.
    ///
    /// Two colours rather than one, because there is no single direction that is
    /// right: a press has to move the surface *towards its own contrast*, and
    /// only the control knows which colour that is. Guessing from luminance gets
    /// a mid-grey wrong in both directions.
    ///
    /// Composited into one colour rather than painted as a second layer — see
    /// [`Color::over`]. For a control with nothing behind it, a text button say,
    /// `base` is transparent and the result is the wash on its own, which is
    /// exactly what should appear.
    ///
    /// `amount` is clamped, and `0.0` gives `base` back untouched — so a control
    /// that is not being pressed pays nothing for being able to be.
    #[must_use]
    pub fn pressed(base: Color, on: Color, amount: f32) -> Color {
        let amount = if amount.is_nan() {
            0.0
        } else {
            amount.clamp(0.0, 1.0)
        };
        if amount <= 0.0 {
            return base;
        }
        let alpha = (f32::from(PRESSED_ALPHA) * amount)
            .round()
            .clamp(0.0, 255.0) as u8;
        on.with_alpha(alpha).over(base)
    }
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::light()
    }
}

/// The type scale.
///
/// Five sizes, each with a job. More would be a design system; fewer would send
/// every application back to hard-coding numbers, which is the thing a scale
/// exists to stop.
///
/// # Line height and letter spacing are part of a size, not extras
///
/// They come with each entry rather than being left at the default, because
/// typography does not scale linearly: display text set at the body's 1.2 line
/// height looks airy and unedited, and body text set at the display's 1.1 is
/// cramped to the point of being harder to read. Letter spacing runs the other
/// way — large text wants a touch of negative tracking, small uppercase-ish
/// label text wants a touch of positive.
///
/// The numbers below are the ones a caller would otherwise have to rediscover.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Typography {
    /// The largest thing on a screen, used once if at all.
    pub display: TextStyle,
    /// A screen or section heading.
    pub headline: TextStyle,
    /// A list row's primary line, a dialog's heading.
    pub title: TextStyle,
    /// Running text, and the default for anything unspecified.
    pub body: TextStyle,
    /// A button, a chip, a caption — short text that labels something else.
    pub label: TextStyle,
}

impl Typography {
    /// The default scale, in [`on_surface`](ColorScheme::on_surface) colour.
    #[must_use]
    pub const fn scale(on_surface: Color) -> Self {
        Self {
            display: TextStyle::new(32.0)
                .color(on_surface)
                .line_height(1.12)
                .letter_spacing(-0.5),
            headline: TextStyle::new(24.0)
                .color(on_surface)
                .line_height(1.2)
                .letter_spacing(-0.25),
            title: TextStyle::new(18.0)
                .color(on_surface)
                .weight(FontWeight::Medium)
                .line_height(1.28),
            // The one built for reading rather than for glancing at, and the
            // only one whose line height is chosen for a paragraph.
            body: TextStyle::new(15.0).color(on_surface).line_height(1.45),
            label: TextStyle::new(14.0)
                .color(on_surface)
                .weight(FontWeight::Medium)
                .line_height(1.15)
                .letter_spacing(0.1),
        }
    }

    /// The same scale, in a different colour.
    ///
    /// What a control does when it paints text on something other than the
    /// surface — a filled button's label on the accent, an error message.
    #[must_use]
    pub const fn colored(self, color: Color) -> Self {
        Self {
            display: self.display.color(color),
            headline: self.headline.color(color),
            title: self.title.color(color),
            body: self.body.color(color),
            label: self.label.color(color),
        }
    }
}

impl Default for Typography {
    fn default() -> Self {
        Self::scale(ColorScheme::light().on_surface)
    }
}

/// The numbers that decide how big and how far apart things are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// The corner radius of a button, a card, a field.
    pub corner: f32,
    /// The spacing unit. Padding and gaps are multiples of it.
    pub gap: f32,
    /// The smallest a thing a finger has to hit may be, on either axis.
    ///
    /// Every control here enforces it with a minimum constraint rather than a
    /// fixed size, so a control can be *bigger* than the target — a full-width
    /// button — but never smaller. 48 is the number both platform guidelines
    /// arrive at from opposite directions (Android says 48dp, iOS says 44pt with
    /// more generous spacing).
    pub touch_target: f32,
}

impl Metrics {
    /// Android's numbers, and the default. The standard 8dp grid and 48dp target.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            corner: 8.0,
            gap: 8.0,
            touch_target: 48.0,
        }
    }

    /// Apple's numbers: a 44pt touch target and a rounder corner.
    ///
    /// The two guidelines genuinely differ here and both are right for their
    /// platform — 48dp with tight spacing against 44pt with generous spacing.
    ///
    /// **The corner is an approximation and will stay one.** iOS corners are
    /// *continuous* (a squircle), and this framework has one radius and a
    /// circular arc. 10 is what reads closest at control sizes; it is not the
    /// same curve and no number here would make it one. Fixing that properly is
    /// new geometry in `vieww-foundation::path`, not a token.
    #[must_use]
    pub const fn apple() -> Self {
        Self {
            corner: 10.0,
            gap: 8.0,
            touch_target: 44.0,
        }
    }

    /// The numbers this platform's users expect.
    #[must_use]
    pub const fn adaptive(platform: TargetPlatform) -> Self {
        if platform.is_apple() {
            Self::apple()
        } else {
            Self::new()
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// How things move.
///
/// # Why motion belongs in the theme
///
/// Colour and size have long been design-system tokens and motion has not, which
/// is how every application ends up with four slightly different fade durations
/// and a spring per developer. The expressive-motion argument — and this
/// struct agrees with it — is that motion is a component of the design system,
/// not a per-widget decision: an application should be able to say "this brand
/// is springy" or "this brand is calm" in one line and have every control
/// follow, exactly as it can already say "this brand is teal".
///
/// # The spatial / effects split, and why it is not optional
///
/// The two families are separated because **they have different correctness
/// constraints, not just different taste**.
///
/// - **Spatial** tokens drive things with a position or a size — a sheet
///   sliding up, a card growing, a handle snapping to a stop. Overshoot is
///   welcome here; it is what makes motion read as physical.
/// - **Effects** tokens drive things with a bounded range — opacity, colour,
///   elevation. Overshoot is *wrong* here: a 0 → 1 opacity spring that
///   overshoots asks for alpha 1.06, which clamps, so the animation visibly
///   stalls at full opacity and then does nothing for 80ms. It looks like a
///   dropped frame, and it is the single most common way a spring is misused.
///
/// [`Motion::effects_never_overshoot`] enforces that as an invariant, and the
/// tests here check it for every built-in.
///
/// # Three speeds, not a duration per widget
///
/// `fast` / `default` / `slow` rather than milliseconds, because the question a
/// control has is "is this a small thing or a big thing", not "how many
/// milliseconds". A switch thumb is fast; a bottom sheet is slow; almost
/// everything else is default.
///
/// # Examples
///
/// ```
/// use vieww_widget::{Motion, ThemeData};
///
/// // A calm brand: nothing overshoots anywhere.
/// let calm = ThemeData {
///     motion: Motion::standard(),
///     ..ThemeData::light()
/// };
///
/// // A playful one: spatial motion bounces, fades still do not.
/// let playful = ThemeData {
///     motion: Motion::expressive(),
///     ..ThemeData::light()
/// };
///
/// assert!(playful.motion.spatial_default.spec().damping_ratio < 1.0);
/// assert!(playful.motion.effects_default.spec().damping_ratio >= 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// Small things that move: a switch thumb, a chip's selection indicator.
    pub spatial_fast: SpringPreset,
    /// The default for anything that moves or resizes.
    pub spatial_default: SpringPreset,
    /// Large things that move: a bottom sheet, a full-screen route.
    pub spatial_slow: SpringPreset,
    /// Small fades: a press wash, a focus ring.
    pub effects_fast: SpringPreset,
    /// The default for anything that fades or recolours.
    pub effects_default: SpringPreset,
    /// Large fades: a scrim behind a modal.
    pub effects_slow: SpringPreset,
    /// Duration for the implicit-animation widgets, small changes.
    ///
    /// Durations as well as springs because [`Animated`](crate::Animated) and
    /// [`AnimatedContainer`](crate::AnimatedContainer) interpolate a tween over
    /// a fixed time rather than simulating — a spring cannot be asked how long
    /// it will take, so those two need a number.
    pub duration_short: Duration,
    /// Duration for the implicit-animation widgets, ordinary changes.
    pub duration_medium: Duration,
    /// Duration for the implicit-animation widgets, large changes.
    pub duration_long: Duration,
    /// The easing the duration-based widgets use by default.
    pub curve_standard: Curve,
    /// The easing for motion that should read as deliberate.
    pub curve_emphasized: Curve,
}

impl Motion {
    /// These tokens with every duration collapsed, for a reduced-motion system.
    ///
    /// The springs are left alone deliberately. A `SpringPreset` is consumed by
    /// something that is already being settled at tick time — see
    /// `Ticker::settle` — so zeroing them here would be a second, redundant
    /// suppression of the same motion, and it would make a preset read back as
    /// nonsense (`stiffness: 0`) to anything that inspected it.
    ///
    /// What genuinely needs this is the **duration** half, which the implicit
    /// animation widgets interpolate a tween over: `Animated`,
    /// `AnimatedContainer` and everything built on them read a number from here
    /// and drive it themselves. Zero is a cut, which is what reduced motion
    /// asks for.
    #[must_use]
    pub const fn reduced(self) -> Self {
        Self {
            duration_short: Duration::ZERO,
            duration_medium: Duration::ZERO,
            duration_long: Duration::ZERO,
            ..self
        }
    }

    /// Critically damped everywhere. Nothing overshoots.
    ///
    /// The right default for an application that is a tool: a spreadsheet, a
    /// settings screen, an admin console. Motion here is wayfinding, not
    /// personality.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            spatial_fast: SpringPreset::Custom(SpringSpec::new(500.0)),
            spatial_default: SpringPreset::Standard,
            spatial_slow: SpringPreset::Custom(SpringSpec::new(120.0)),
            effects_fast: SpringPreset::Custom(SpringSpec::new(500.0)),
            effects_default: SpringPreset::Standard,
            effects_slow: SpringPreset::Custom(SpringSpec::new(120.0)),
            duration_short: Duration::from_millis(120),
            duration_medium: Duration::from_millis(200),
            duration_long: Duration::from_millis(320),
            curve_standard: Curve::FAST_OUT_SLOW_IN,
            curve_emphasized: Curve::EASE_IN_OUT,
        }
    }

    /// Spatial motion overshoots; effects motion does not.
    ///
    /// The expressive scheme. Use it when the product *is* the feel —
    /// a consumer app, a launcher, anything where a control being fun to press
    /// is part of the pitch.
    ///
    /// The three spatial springs share a damping ratio and differ in stiffness,
    /// so a sheet and a switch bounce the same *amount* at different speeds. A
    /// slow spring with a small damping ratio wobbles for most of a second,
    /// which is why `spatial_slow` is only slightly under-damped.
    #[must_use]
    pub const fn expressive() -> Self {
        Self {
            spatial_fast: SpringPreset::Custom(SpringSpec {
                stiffness: 550.0,
                damping_ratio: 0.75,
                initial_velocity: 0.0,
            }),
            spatial_default: SpringPreset::Expressive,
            spatial_slow: SpringPreset::Custom(SpringSpec {
                stiffness: 140.0,
                damping_ratio: 0.9,
                initial_velocity: 0.0,
            }),
            // Deliberately identical to `standard()`. See the type docs: a
            // bouncing opacity clamps and reads as a stutter, so "expressive"
            // does not reach this half of the table.
            effects_fast: SpringPreset::Custom(SpringSpec::new(500.0)),
            effects_default: SpringPreset::Standard,
            effects_slow: SpringPreset::Custom(SpringSpec::new(120.0)),
            duration_short: Duration::from_millis(120),
            duration_medium: Duration::from_millis(220),
            duration_long: Duration::from_millis(380),
            curve_standard: Curve::FAST_OUT_SLOW_IN,
            curve_emphasized: Curve::cubic(0.2, 0.0, 0.0, 1.0),
        }
    }

    /// The motion this platform's users expect.
    ///
    /// Apple's platforms have used gently under-damped spatial motion since iOS
    /// 7, with springs the default interaction curve; Android's guidance is the
    /// Expressive scheme. They agree closely enough that this returns the same
    /// thing on both — the parameter is here so that the call site reads the
    /// same as [`ColorScheme::adaptive_light`] and [`Metrics::adaptive`], and so
    /// that the day they diverge is a one-line change rather than a search.
    #[must_use]
    pub const fn adaptive(_platform: TargetPlatform) -> Self {
        Self::expressive()
    }

    /// `true` when no effects token can overshoot its range.
    ///
    /// An invariant rather than a convention. A custom [`Motion`] that fails
    /// this will produce opacity animations that clamp at 1.0 and appear to
    /// stall — check it in a test rather than discovering it on a device.
    ///
    /// ```
    /// use vieww_widget::Motion;
    ///
    /// assert!(Motion::standard().effects_never_overshoot());
    /// assert!(Motion::expressive().effects_never_overshoot());
    /// ```
    #[must_use]
    pub fn effects_never_overshoot(&self) -> bool {
        [self.effects_fast, self.effects_default, self.effects_slow]
            .iter()
            .all(|preset| preset.spec().damping_ratio >= 1.0)
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self::standard()
    }
}

/// Everything a control needs to know about how it should look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeData {
    pub colors: ColorScheme,
    pub text: Typography,
    pub metrics: Metrics,
    /// How things move. See [`Motion`].
    pub motion: Motion,
    /// Which platform's *shapes* the controls draw.
    ///
    /// # Why this is a field and was not one
    ///
    /// [`adaptive`](Self::adaptive) has always chosen the platform's colours,
    /// metrics and motion — and then thrown the platform itself away, so no
    /// control could ask. Its own doc was candid about the consequence: "it does
    /// not buy an iOS switch's shape, an iOS picker", and a studio preview
    /// that changes what a screen is *told* without changing what it *looks
    /// like* is a preview a developer cannot trust for the one question they
    /// opened it to answer.
    ///
    /// Carrying the platform closes that. The controls that differ visibly
    /// between iOS and Android — the switch, the slider, the checkbox and
    /// radio, the button's corner, the activity indicator, the alert's button
    /// row, the segmented control — read this and draw the shape the platform's
    /// users recognise. Everything else ignores it, which is why this is still
    /// *one* catalogue rather than two.
    pub platform: TargetPlatform,
}

impl ThemeData {
    /// A theme built from a scheme, with the type scale coloured to match.
    #[must_use]
    pub const fn from_colors(colors: ColorScheme) -> Self {
        Self {
            text: Typography::scale(colors.on_surface),
            colors,
            metrics: Metrics::new(),
            motion: Motion::standard(),
            // **The machine this is running on**, resolved at compile time.
            //
            // An application that writes `ThemeData::light()` and nothing else
            // has not said it wants to look the same everywhere; it has said it
            // has not thought about it. Looking native is the answer that is
            // right more often, and it is the one a person is least surprised
            // by when they run the same binary on their phone and their laptop.
            //
            // The *colours* deliberately do not follow — see `light()` — so
            // this is a narrower promise than [`adaptive`](Self::adaptive)
            // makes, and the two are not interchangeable.
            //
            // # What this costs, and what pays for it
            //
            // A default that follows the host is a default a test cannot assert
            // a shape against: the same assertion would pass on Linux and fail
            // on a Mac. That is a real hazard and it is why this briefly was a
            // fixed platform instead. The answer is not a wrong default, it is
            // a *deterministic constructor* —
            // [`with_platform`](Self::with_platform) — and a test that asserts
            // a shape says which platform's shape it means. `platform_shapes.rs`
            // is that rule written down, and
            // `no_test_asserts_a_shape_against_an_unpinned_theme` is the guard
            // that keeps it true.
            platform: TargetPlatform::current(),
        }
    }

    /// The light scheme, on this machine's shapes.
    ///
    /// The *scheme* is the standard one on every platform — `adaptive_light` is what
    /// picks Apple's greys and blue. Only the shapes follow the host, because
    /// those are what a control has no way to be neutral about: a switch is
    /// either a pill with a fixed knob or a track with a growing one, and there
    /// is no third thing it can be while it waits to be told.
    #[must_use]
    pub const fn light() -> Self {
        Self::from_colors(ColorScheme::light())
    }

    /// The dark scheme, on this machine's shapes. See [`light`](Self::light).
    #[must_use]
    pub const fn dark() -> Self {
        Self::from_colors(ColorScheme::dark())
    }

    /// The same theme, drawn for `platform` rather than for this machine.
    ///
    /// # The two things this is for
    ///
    /// **A test that asserts a shape.** Anything checking that a switch is a
    /// pill, or that a button is a stadium, is checking a *platform's* answer;
    /// written against a theme that follows the host it passes on the machine it
    /// was written on and fails on the next one. Naming the platform is the
    /// whole fix, and it makes the test say what it is actually testing.
    ///
    /// **An application that means to look the same everywhere.** A branded app
    /// is a legitimate and common choice, and this is how it says so in one
    /// line — rather than by accepting whatever the default happens to be and
    /// hoping nobody changes it.
    ///
    /// It does not touch the colours, the metrics or the motion; it is the
    /// shapes and nothing else. [`adaptive`](Self::adaptive) is the one that
    /// moves all four together.
    #[must_use]
    pub const fn with_platform(mut self, platform: TargetPlatform) -> Self {
        self.platform = platform;
        self
    }

    /// A theme whose colours and metrics are the ones this platform's users
    /// expect, light or dark.
    ///
    /// ```
    /// use vieww_widget::{Theme, ThemeData};
    /// use vieww_widget::prelude::*;
    /// use vieww_foundation::TargetPlatform;
    ///
    /// // What an application writes: one line, and every control follows.
    /// let theme = ThemeData::adaptive(TargetPlatform::current(), false);
    /// # let _ = Theme::new(theme);
    /// ```
    ///
    /// # This is tokens, not a second widget set
    ///
    /// Some toolkits ship two platform catalogues: different
    /// structure, different gestures, hundreds of widgets. This is one
    /// catalogue wearing the platform's accent, greys, corner radius and touch
    /// target. **Be clear about what that does and does not buy.**
    ///
    /// It buys the things users actually notice first — a button that is the
    /// system blue, a target the size the guidelines ask for, dividers the
    /// right grey — **and, since [`platform`](Self::platform) became a field,
    /// the shapes**: an iOS switch is a pill with a large white knob, an iOS
    /// checkbox is a tick rather than a filled square, an iOS slider has a
    /// shadowed circular thumb on a thin track, an iOS alert stacks its buttons
    /// under a divider, and the activity indicator is a ring of fading spokes
    /// rather than a sweeping arc.
    ///
    /// It still does not buy a platform-native picker, a platform navigation bar or
    /// the sliding back-gesture. This is one catalogue that knows which
    /// platform it is drawing for, not two catalogues.
    ///
    /// # It takes the platform rather than reading it
    ///
    /// So a test can ask for one it is not running on, and so an application
    /// can deliberately look the same everywhere — which is a legitimate
    /// choice, and a common one for a branded app. `TargetPlatform::current()`
    /// is resolved at compile time, so passing it costs nothing.
    #[must_use]
    pub const fn adaptive(platform: TargetPlatform, dark: bool) -> Self {
        let colors = if dark {
            ColorScheme::adaptive_dark(platform)
        } else {
            ColorScheme::adaptive_light(platform)
        };
        Self {
            text: Typography::scale(colors.on_surface),
            colors,
            metrics: Metrics::adaptive(platform),
            motion: Motion::adaptive(platform),
            platform,
        }
    }

    /// The theme in force at this position, or the default one.
    ///
    /// This is the theme lookup, and it never fails: a
    /// control built with no [`Theme`] above it — in a test, in a tree dump, in
    /// the first five minutes of a new application — gets the light theme rather
    /// than a panic. Every control in this crate calls it exactly once per build.
    #[must_use]
    pub fn of(ctx: &BuildContext) -> Rc<Self> {
        thread_local! {
            /// One allocation per thread rather than one per themeless build.
            static FALLBACK: Rc<ThemeData> = Rc::new(ThemeData::light());
        }

        let theme = ctx
            .inherit::<Self>()
            .unwrap_or_else(|| FALLBACK.with(Rc::clone));

        // **Where the accessibility preferences stop being decoration.**
        //
        // `Accessibility` is already inherited — the render factory reads it
        // for `text_scale` — and this is the other half. Applied here rather
        // than at the `Theme` widget because a subtree can publish its own
        // theme (a dark section of a light screen), and a preference honoured
        // only at the root would be silently dropped by every one of them.
        //
        // The common case allocates nothing: `unchanged` is a bare comparison
        // of two bools, and a build with no preferences set returns the same
        // `Rc` the inherit handed back.
        let accessibility = crate::AccessibilityOf::of(ctx);
        if !accessibility.high_contrast && !accessibility.reduce_motion {
            return theme;
        }
        Rc::new(theme.as_ref().with_accessibility(accessibility))
    }

    /// This theme, adjusted for what the person has asked their system for.
    ///
    /// Applied automatically by [`of`](Self::of); public because an application
    /// building a theme outside the tree — a preview, a screenshot harness, a
    /// test — wants the same answer without a `BuildContext`.
    ///
    /// [`text_scale`](vieww_foundation::Accessibility::text_scale) is **not**
    /// applied here: it is applied in `vieww-render`'s factory, at the single
    /// funnel every `Text` and `TextField` passes through, and doing it twice
    /// would square it.
    #[must_use]
    pub fn with_accessibility(&self, accessibility: vieww_foundation::Accessibility) -> Self {
        let colors = if accessibility.high_contrast {
            self.colors.high_contrast()
        } else {
            self.colors
        };
        Self {
            colors,
            // Retinted, because the type scale is coloured from `on_surface`
            // and a high-contrast scheme has just moved it.
            text: if accessibility.high_contrast {
                Typography::scale(colors.on_surface)
            } else {
                self.text
            },
            metrics: self.metrics,
            // **Belt as well as braces.** `Tickers` settles the framework's own
            // animations at tick time, which covers every `AnimationController`,
            // spring, timeline and route transition. This covers the other
            // path: application code that reads `theme.motion` and drives
            // something itself. Both are needed — the first because a widget
            // cannot read a context when its state is created, the second
            // because an application's own animation never reaches the registry.
            motion: if accessibility.reduce_motion {
                self.motion.reduced()
            } else {
                self.motion
            },
            platform: self.platform,
        }
    }
}

impl Default for ThemeData {
    fn default() -> Self {
        Self::light()
    }
}

/// Publishes a [`ThemeData`] to its subtree.
///
/// A thin wrapper over [`Inherited<ThemeData>`](crate::Inherited) that exists so
/// application code says `Theme` instead of naming the generic — and so that
/// nesting one theme inside another is obviously the supported thing to do. An
/// inner theme shadows an outer one, which is how a dark section of a light
/// screen works.
#[derive(Debug, Clone)]
pub struct Theme {
    data: Rc<ThemeData>,
    child: WidgetNode,
    key: Option<Key>,
}

impl Theme {
    #[must_use]
    pub fn new(data: ThemeData) -> Self {
        Self::shared(Rc::new(data), crate::SizedBox::shrink())
    }

    /// Publish an already-shared theme.
    #[must_use]
    pub fn shared(data: Rc<ThemeData>, child: impl Into<WidgetNode>) -> Self {
        Self {
            data,
            child: child.into(),
            key: None,
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = child.into();
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The theme this widget publishes.
    #[must_use]
    pub fn data(&self) -> &Rc<ThemeData> {
        &self.data
    }
}

impl Widget for Theme {
    fn debug_name(&self) -> &'static str {
        "Theme"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Inherited::shared(Rc::clone(&self.data), self.child.clone()).into()
    }
}

widget_node_from!(Theme);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_reaches_a_descendant_and_the_inner_one_wins() {
        let root = BuildContext::root().child_with(Rc::new(ThemeData::light()));
        assert_eq!(ThemeData::of(&root).colors, ColorScheme::light());

        let nested = root.child_with(Rc::new(ThemeData::dark()));
        assert_eq!(ThemeData::of(&nested).colors, ColorScheme::dark());
    }

    #[test]
    fn a_build_with_no_theme_above_it_gets_the_default_rather_than_panicking() {
        assert_eq!(
            ThemeData::of(&BuildContext::root()).colors,
            ColorScheme::light()
        );
    }

    #[test]
    fn the_fallback_theme_is_allocated_once_per_thread() {
        let first = ThemeData::of(&BuildContext::root());
        let second = ThemeData::of(&BuildContext::root());
        assert!(
            Rc::ptr_eq(&first, &second),
            "a themeless build must not allocate a theme per widget"
        );
    }

    #[test]
    fn the_type_scale_is_coloured_to_match_the_scheme() {
        let dark = ThemeData::dark();
        assert_eq!(dark.text.body.color, dark.colors.on_surface);
        assert_eq!(
            dark.text.colored(dark.colors.on_primary).body.color,
            dark.colors.on_primary,
            "recolouring the scale recolours every size in it"
        );
    }

    #[test]
    fn dimming_only_changes_the_alpha() {
        let dimmed = ColorScheme::dimmed(Color::RED);
        assert_eq!(dimmed.with_alpha(255), Color::RED);
        assert_eq!(dimmed.a, DISABLED_ALPHA);
    }

    #[test]
    fn the_theme_widget_publishes_what_it_was_given() {
        let dump = crate::debug_tree(Theme::new(ThemeData::dark()).child(crate::Text::new("hi")));
        assert!(dump.contains("Theme"), "{dump}");
        assert!(dump.contains("Inherited<ThemeData>"), "{dump}");
    }

    #[test]
    fn an_adaptive_theme_takes_the_platforms_accent_and_target() {
        let ios = ThemeData::adaptive(TargetPlatform::IOS, false);
        let android = ThemeData::adaptive(TargetPlatform::Android, false);

        assert_eq!(
            ios.colors.primary,
            Color::hex(0x00_40DD),
            "iOS controls are the system blue — Apple's increased-contrast \
             variant of it, which is what `apple_light` ships so that a white \
             label on a filled button clears AA; see that function's doc"
        );
        assert_ne!(
            android.colors.primary, ios.colors.primary,
            "and Android's is not"
        );

        assert_eq!(ios.metrics.touch_target, 44.0, "44pt is Apple's number");
        assert_eq!(android.metrics.touch_target, 48.0, "48dp is Google's");
    }

    #[test]
    fn the_mac_harness_looks_like_the_iphone_it_stands_in_for() {
        // `is_apple`, not `== IOS` — the whole reason that helper exists is so
        // running the desktop harness exercises the path an iPhone will.
        assert_eq!(
            ThemeData::adaptive(TargetPlatform::MacOS, false)
                .colors
                .primary,
            ThemeData::adaptive(TargetPlatform::IOS, false)
                .colors
                .primary
        );
        assert_eq!(
            ThemeData::adaptive(TargetPlatform::Linux, false)
                .colors
                .primary,
            ThemeData::adaptive(TargetPlatform::Android, false)
                .colors
                .primary,
            "and the Linux harness stands in for Android"
        );
    }

    #[test]
    fn every_scheme_colours_its_type_scale_to_match() {
        // The trap `from_colors` exists to prevent: a scheme swapped without
        // its typography leaves text in the old scheme's ink, which on a dark
        // surface is invisible rather than merely wrong.
        for (platform, dark) in [
            (TargetPlatform::IOS, false),
            (TargetPlatform::IOS, true),
            (TargetPlatform::Android, false),
            (TargetPlatform::Android, true),
        ] {
            let theme = ThemeData::adaptive(platform, dark);
            assert_eq!(
                theme.text.body.color, theme.colors.on_surface,
                "{platform:?} dark={dark} draws body text in the wrong ink"
            );
        }
    }

    #[test]
    fn a_dark_scheme_is_actually_darker_than_its_light_one() {
        for platform in [TargetPlatform::IOS, TargetPlatform::Android] {
            let light = ThemeData::adaptive(platform, false).colors.surface;
            let dark = ThemeData::adaptive(platform, true).colors.surface;
            let luminance = |c: Color| u32::from(c.r) + u32::from(c.g) + u32::from(c.b);
            assert!(
                luminance(dark) < luminance(light),
                "{platform:?} has them the wrong way round"
            );
        }
    }

    // ── Motion tokens ─────────────────────────────────────────────────────

    /// The invariant the spatial/effects split exists for. An opacity spring
    /// that overshoots asks for alpha > 1, clamps, and reads as a dropped
    /// frame — so no effects token in any built-in scheme may be under-damped.
    #[test]
    fn no_built_in_effects_token_overshoots() {
        for (name, motion) in [
            ("standard", Motion::standard()),
            ("expressive", Motion::expressive()),
            ("adaptive(iOS)", Motion::adaptive(TargetPlatform::IOS)),
            (
                "adaptive(Android)",
                Motion::adaptive(TargetPlatform::Android),
            ),
            ("default", Motion::default()),
        ] {
            assert!(
                motion.effects_never_overshoot(),
                "{name} has an under-damped effects token, which will clamp"
            );
        }
    }

    /// The other half of the split: expressive spatial motion has to actually
    /// overshoot, or the scheme is standard wearing a different name.
    #[test]
    fn expressive_spatial_motion_overshoots_and_standard_does_not() {
        let expressive = Motion::expressive();
        for preset in [
            expressive.spatial_fast,
            expressive.spatial_default,
            expressive.spatial_slow,
        ] {
            assert!(
                preset.spec().damping_ratio < 1.0,
                "expressive spatial motion must overshoot: {preset:?}"
            );
        }

        let standard = Motion::standard();
        for preset in [
            standard.spatial_fast,
            standard.spatial_default,
            standard.spatial_slow,
        ] {
            assert!(
                preset.spec().damping_ratio >= 1.0,
                "standard motion must not overshoot anywhere: {preset:?}"
            );
        }
    }

    /// fast / default / slow have to be ordered, or the names lie. Stiffness is
    /// inversely related to settle time, so "fast" is the stiffest.
    #[test]
    fn the_three_speeds_are_actually_ordered() {
        for motion in [Motion::standard(), Motion::expressive()] {
            assert!(
                motion.spatial_fast.spec().stiffness > motion.spatial_default.spec().stiffness,
                "fast must be stiffer than default"
            );
            assert!(
                motion.spatial_default.spec().stiffness > motion.spatial_slow.spec().stiffness,
                "default must be stiffer than slow"
            );
            assert!(motion.duration_short < motion.duration_medium);
            assert!(motion.duration_medium < motion.duration_long);
        }
    }

    /// A theme is one value, so setting motion must not disturb colour — this
    /// is the `..ThemeData::light()` pattern the type docs promise.
    #[test]
    fn motion_can_be_swapped_without_touching_the_rest_of_the_theme() {
        let base = ThemeData::light();
        let playful = ThemeData {
            motion: Motion::expressive(),
            ..base
        };

        assert_eq!(playful.colors, base.colors);
        assert_eq!(playful.metrics, base.metrics);
        assert_ne!(playful.motion, base.motion);
    }

    /// Every theme constructor has to populate motion, or an application that
    /// builds its theme one way silently gets no tokens.
    #[test]
    fn every_theme_constructor_carries_motion() {
        assert_eq!(ThemeData::light().motion, Motion::standard());
        assert_eq!(ThemeData::dark().motion, Motion::standard());
        assert_eq!(
            ThemeData::adaptive(TargetPlatform::IOS, false).motion,
            Motion::adaptive(TargetPlatform::IOS)
        );
    }
}
