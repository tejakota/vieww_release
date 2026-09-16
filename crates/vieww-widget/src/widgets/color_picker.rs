//! A colour picker using ElementState for the selected colour.
//!
//! # Why ElementState
//!
//! The selected colour is widget state: it changes when the user
//! interacts, it triggers a rebuild of the picker, and it needs to
//! survive rebuilds. `ElementState` is the vieww pattern for exactly
//! this. The application reads the current colour through a callback
//! rather than holding a Signal.

use std::any::Any;

use std::rc::Rc;

use vieww_foundation::{Border, Color, Size};

use crate::prelude::*;
use crate::{widget_node_from, ElementState};

/// State for the colour picker.
#[derive(Debug)]
pub struct ColorPickerState {
    /// The currently selected colour.
    pub selected: Color,
    /// `true` when the state has changed and the element should rebuild.
    pending: bool,
}

impl ColorPickerState {
    /// Create state with an initial colour.
    #[must_use]
    pub fn new(color: Color) -> Self {
        Self {
            selected: color,
            pending: false,
        }
    }
}

impl ElementState for ColorPickerState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// The chosen colour, as eight hex digits.
    ///
    /// Hex rather than four numbers because it is the format a colour is
    /// written in everywhere else in this framework, so a snapshot that ends up
    /// in a log is readable by the person reading the log.
    fn snapshot(&self) -> Option<String> {
        let c = self.selected;
        Some(format!("{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a))
    }

    fn restore(&mut self, saved: &str) -> bool {
        if saved.len() != 8 {
            return false;
        }
        let byte = |at: usize| u8::from_str_radix(&saved[at..at + 2], 16).ok();
        let (Some(r), Some(g), Some(b), Some(a)) = (byte(0), byte(2), byte(4), byte(6)) else {
            return false;
        };
        self.selected = Color::rgba(r, g, b, a);
        self.pending = true;
        true
    }
}

/// A colour picker with preset swatches and HSV controls.
///
/// # Examples
///
/// ```ignore
/// ColorPicker::new(Color::rgb(58, 122, 246))
///     .presets(vec![
///         Color::rgb(239, 68, 68),
///         Color::rgb(34, 197, 94),
///         Color::rgb(59, 130, 246),
///     ])
///     .on_color_changed(|color| {
///         println!("selected: {:?}", color);
///     })
/// ```
pub struct ColorPicker {
    /// The initial colour.
    initial: Color,
    /// Preset colours shown as swatches.
    presets: Vec<Color>,
    /// Callback when the colour changes.
    on_color_changed: Option<Rc<dyn Fn(Color)>>,
    /// Whether to show the HSV sliders.
    show_sliders: bool,
}

// By hand: the callback is a boxed closure and no closure is `Debug`.
impl std::fmt::Debug for ColorPicker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColorPicker")
            .field("initial", &self.initial)
            .field("presets", &self.presets)
            .field("show_sliders", &self.show_sliders)
            .field(
                "on_color_changed",
                &self.on_color_changed.as_ref().map(|_| "<callback>"),
            )
            .finish()
    }
}

impl ColorPicker {
    /// A picker starting at `initial`.
    #[must_use]
    pub fn new(initial: Color) -> Self {
        Self {
            initial,
            presets: default_presets(),
            on_color_changed: None,
            show_sliders: true,
        }
    }

    /// Set the preset swatches.
    #[must_use]
    pub fn presets(mut self, presets: Vec<Color>) -> Self {
        self.presets = presets;
        self
    }

    /// Set the colour-changed callback.
    #[must_use]
    pub fn on_color_changed<F: Fn(Color) + 'static>(mut self, f: F) -> Self {
        self.on_color_changed = Some(Rc::new(f));
        self
    }

    /// Show or hide the HSV sliders.
    #[must_use]
    pub const fn show_sliders(mut self, show: bool) -> Self {
        self.show_sliders = show;
        self
    }
}

impl Widget for ColorPicker {
    fn debug_name(&self) -> &'static str {
        "ColorPicker"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(ColorPickerState::new(self.initial)))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let selected = ctx
            .state::<ColorPickerState, _>(|s| s.selected)
            .unwrap_or(self.initial);

        let handle = ctx.state_handle();
        // **The picker's own chrome follows the application's scheme.**
        //
        // Its labels, its track and its hex readout were five hex literals —
        // a near-black, two greys and a light grey — so the one control on the
        // screen whose entire subject is colour was the one control that
        // ignored the theme. On a dark scheme it drew near-black text on a dark
        // ground.
        //
        // The *swatches* are deliberately still the caller's own colours: those
        // are the picker's content, not its chrome, and an application choosing
        // its presets means exactly what it says.
        let colors = ThemeData::of(ctx).colors;

        // The preset swatches: tappable coloured boxes.
        let swatches: Vec<WidgetNode> = self
            .presets
            .iter()
            .map(|&color| {
                let is_selected = color == selected;
                let handle = handle.clone();
                // Clone the `Rc`, not a borrow: `on_tap` needs a `'static`
                // closure, and a `&dyn Fn` borrowed from `self` cannot live
                // that long.
                let on_change = self.on_color_changed.clone();

                Pressable::themed(ctx, move |_press: f32| {
                    Container::new()
                        .color(color)
                        .radius(6.0)
                        .border(if is_selected {
                            Border::new(Color::BLACK, 2.0)
                        } else {
                            Border::new(Color::TRANSPARENT, 2.0)
                        })
                        .child(SizedBox::square(28.0))
                        .into()
                })
                .on_tap(move || {
                    if let Some(state) = &handle {
                        if let Some(picker) = state
                            .borrow_mut()
                            .as_any_mut()
                            .downcast_mut::<ColorPickerState>()
                        {
                            picker.selected = color;
                            picker.pending = true;
                        }
                    }
                    if let Some(cb) = &on_change {
                        cb(color);
                    }
                })
                .into()
            })
            .collect();

        // The current colour display.
        let current_display = Container::new()
            .color(selected)
            .radius(8.0)
            .child(SizedBox::from_size(Size::new(120.0, 48.0)));

        // The hex code display.
        let hex = format!("#{:02X}{:02X}{:02X}", selected.r, selected.g, selected.b);
        let hex_display = Text::new(hex).size(14.0).color(colors.on_surface);

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(16.0)
            .children(children![
                // Current colour and hex.
                Flex::row().spacing(16.0).children(children![
                    current_display,
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(4.0)
                        .children(children![
                            Text::new("Selected colour")
                                .size(12.0)
                                .color(colors.on_surface_variant),
                            hex_display,
                        ]),
                ]),
                // Preset swatches.
                // Annotated: inside `children![]` each arm is passed to
                // `WidgetNode::from`, so a bare `.into()` here has no target
                // type to infer.
                if !swatches.is_empty() {
                    WidgetNode::from(Flex::row().spacing(8.0).children(swatches))
                } else {
                    WidgetNode::from(SizedBox::shrink())
                },
                // The HSV readout. `show_sliders` used to be a stored,
                // settable field that `build` never read — the struct doc
                // promised "HSV controls" and the widget drew swatches and
                // nothing else, while `rgb_to_hsv`/`hsv_to_rgb` sat unused
                // beside it. A knob that silently does nothing is worse than
                // no knob, so it now decides whether this section exists.
                if self.show_sliders {
                    WidgetNode::from(hsv_section(selected, colors))
                } else {
                    WidgetNode::from(SizedBox::shrink())
                },
            ])
            .into()
    }
}

/// The hue / saturation / value readout for `color`.
///
/// Three labelled bars, each filled in proportion to its channel and tinted
/// with the colour that channel would produce on its own. Drawn from
/// [`rgb_to_hsv`] so the bars and the swatch can never disagree.
fn hsv_section(color: Color, colors: ColorScheme) -> Flex {
    let (h, s, v) = rgb_to_hsv(color);

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(8.0)
        .children(children![
            Text::new("HSV").size(12.0).color(colors.on_surface_variant),
            // Hue runs 0-360; saturation and value run 0-1.
            hsv_bar("H", h / 360.0, hsv_to_rgb(h, 1.0, 1.0), colors),
            hsv_bar("S", s, hsv_to_rgb(h, s.max(0.001), 1.0), colors),
            // The value bar shows the colour itself; a desaturated fill at
            // v=0.96 is near-white and reads as an empty track.
            hsv_bar("V", v, hsv_to_rgb(h, s, v), colors),
        ])
}

/// One labelled channel bar, filled to `fraction` of its width.
fn hsv_bar(label: &str, fraction: f32, fill: Color, colors: ColorScheme) -> WidgetNode {
    const TRACK: f32 = 180.0;
    let fraction = fraction.clamp(0.0, 1.0);

    Flex::row()
        .spacing(8.0)
        .children(children![
            Text::new(label).size(12.0).color(colors.on_surface_variant),
            Stack::new()
                .push(
                    Container::new()
                        .color(colors.surface_variant)
                        .radius(3.0)
                        .child(SizedBox::from_size(Size::new(TRACK, 6.0))),
                )
                .push(
                    Container::new()
                        .color(fill)
                        .radius(3.0)
                        .child(SizedBox::from_size(Size::new(TRACK * fraction, 6.0))),
                ),
            Text::new(format!("{:.0}%", fraction * 100.0))
                .size(11.0)
                .color(colors.on_surface_variant),
        ])
        .into()
}

widget_node_from!(ColorPicker);

/// RGB to HSV conversion.
#[must_use]
pub(crate) fn rgb_to_hsv(color: Color) -> (f32, f32, f32) {
    let r = color.r as f32 / 255.0;
    let g = color.g as f32 / 255.0;
    let b = color.b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    let value = max;

    (hue, saturation, value)
}

/// HSV to RGB conversion.
#[must_use]
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    Color::rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

fn default_presets() -> Vec<Color> {
    vec![
        Color::rgb(239, 68, 68),  // red
        Color::rgb(249, 115, 22), // orange
        Color::rgb(250, 204, 21), // yellow
        Color::rgb(34, 197, 94),  // green
        Color::rgb(59, 130, 246), // blue
        Color::rgb(168, 85, 247), // purple
        Color::rgb(236, 72, 153), // pink
        Color::rgb(15, 23, 42),   // near-black
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_roundtrip() {
        let original = Color::rgb(58, 122, 246);
        let (h, s, v) = rgb_to_hsv(original);
        let back = hsv_to_rgb(h, s, v);

        // Allow for rounding (±1 per channel).
        assert!((back.r as i16 - original.r as i16).abs() <= 1);
        assert!((back.g as i16 - original.g as i16).abs() <= 1);
        assert!((back.b as i16 - original.b as i16).abs() <= 1);
    }

    #[test]
    fn red_has_hue_zero() {
        let (h, _, _) = rgb_to_hsv(Color::rgb(255, 0, 0));
        assert!((h - 0.0).abs() < 0.01 || (h - 360.0).abs() < 0.01);
    }

    #[test]
    fn black_has_zero_value() {
        let (_, _, v) = rgb_to_hsv(Color::rgb(0, 0, 0));
        assert_eq!(v, 0.0);
    }

    #[test]
    fn white_has_zero_saturation() {
        let (_, s, v) = rgb_to_hsv(Color::rgb(255, 255, 255));
        assert_eq!(s, 0.0);
        assert_eq!(v, 1.0);
    }

    #[test]
    fn color_picker_state_defaults() {
        let mut state = ColorPickerState::new(Color::RED);
        assert_eq!(state.selected, Color::RED);
        assert!(!state.take_pending());
    }

    #[test]
    fn take_pending_resets() {
        let mut state = ColorPickerState::new(Color::RED);
        state.pending = true;
        assert!(state.take_pending());
        assert!(!state.take_pending());
    }
}
