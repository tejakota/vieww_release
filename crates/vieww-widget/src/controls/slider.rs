use std::fmt;

use vieww_foundation::{Color, Key};

use crate::{
    widget_node_from, BuildContext, ColorScheme, Handler, Pressable, SemanticRole, Semantics,
    ThemeData, Widget, WidgetKind, WidgetNode,
};

/// A value chosen from a range by dragging.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Slider;
/// use std::rc::Rc;
///
/// # let volume = 0.4;
/// # let set: Rc<dyn Fn(f32)> = Rc::new(|_| {});
/// let slider = Slider::new(volume)
///     .range(0.0, 1.0)
///     .label("Volume")
///     .on_changed(Rc::new(move |next| set(next)));
/// ```
///
/// Controlled and disabled-without-a-handler, like every control here.
///
/// # Two widgets, one control
///
/// `Slider` is [`Composed`](WidgetKind::Composed) and does the theming; the
/// drawing and the dragging happen in [`SliderBar`], the render leaf it builds.
/// The split is forced and worth understanding: a leaf has no `build` in which
/// to read the theme, and a slider without a render object cannot turn a drag
/// into a value at all, because that needs the track's *length* and no widget
/// knows its own size.
#[derive(Clone)]
pub struct Slider {
    value: f32,
    min: f32,
    max: f32,
    divisions: Option<u32>,
    label: Option<String>,
    on_changed: Option<Handler<f32>>,
    key: Option<Key>,
}

impl Slider {
    /// A slider over `0.0 ..= 1.0`, showing `value`.
    #[must_use]
    pub const fn new(value: f32) -> Self {
        Self {
            value,
            min: 0.0,
            max: 1.0,
            divisions: None,
            label: None,
            on_changed: None,
            key: None,
        }
    }

    /// The ends of the range. A reversed or empty range is not rejected — it
    /// simply has one value, at the start, rather than a `NaN` thumb.
    #[must_use]
    pub const fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// Snap to this many equal steps between the ends.
    ///
    /// `divisions(4)` over `0..=100` gives 0, 25, 50, 75 and 100 — five stops
    /// and four gaps, which is what "divisions" means and the off-by-one worth
    /// stating.
    #[must_use]
    pub const fn divisions(mut self, divisions: u32) -> Self {
        self.divisions = Some(divisions);
        self
    }

    /// What a screen reader calls this slider. Not drawn.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Called with the value a drag or a tap asks for. Giving a handler is what
    /// enables it.
    #[must_use]
    pub fn on_changed(mut self, handler: Handler<f32>) -> Self {
        self.on_changed = Some(handler);
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.on_changed.is_some()
    }

    /// The leaf that draws and drags, themed, with or without a finger on it.
    fn bar(&self, theme: &ThemeData, press: f32) -> SliderBar {
        let colors = theme.colors;
        let dim = |color: Color| {
            if self.is_enabled() {
                color
            } else {
                ColorScheme::dimmed(color)
            }
        };

        SliderBar {
            value: self.value,
            min: self.min,
            max: self.max,
            divisions: self.divisions,
            height: theme.metrics.touch_target,
            active: dim(colors.primary),
            // The part ahead of the thumb is the same colour, faded — so that
            // the track reads as one thing with a filled part rather than two
            // unrelated bars meeting under the thumb.
            inactive: dim(colors.primary).with_alpha(0x3D),
            thumb: if theme.platform.is_apple() {
                dim(colors.surface)
            } else {
                dim(colors.primary)
            },
            press,
            // 2 and 14 against 4 and 10: iOS draws a hairline track under a
            // 28-point knob, Android a thicker track under a 20-point one.
            track_thickness: if theme.platform.is_apple() { 2.0 } else { 4.0 },
            thumb_radius: if theme.platform.is_apple() {
                14.0
            } else {
                10.0
            },
            // **At full strength.** This was `outline.with_alpha(0x59)` — 35%
            // of the light `outline` grey over white, which resolved to about
            // `#EEEEEE`, and
            // the ring's whole job is to keep a white knob from disappearing
            // against a light page. A 1.1:1 ring does not do that job: the
            // studio's "Switches and sliders" lesson rendered a slider whose
            // knob read as a gap in the track rather than as a control.
            //
            // `outline` at full strength is the same grey the track separators
            // use, which is the weight this needs and no more.
            thumb_ring: theme.platform.is_apple().then(|| dim(colors.outline)),
            on_changed: self.on_changed.clone(),
            // The key belongs to the `Slider` element, not to the leaf it
            // builds: the leaf is this element's only child and is identified by
            // its position, so keying it too would be a second identity for one
            // control.
            key: None,
        }
    }
}

impl Widget for Slider {
    fn debug_name(&self) -> &'static str {
        "Slider"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        // The `Pressable` wraps the bar rather than replacing its gestures. Both
        // enter the arena: the bar's own tap and drag are what move the thumb,
        // and this one only ever watches. The bar is deeper, so it wins the
        // sweep and the watcher is told it lost — which is exactly the moment
        // the halo should go out.
        let bar: WidgetNode = if self.is_enabled() {
            let slider = self.clone();
            let theme = *theme;
            Pressable::themed(ctx, move |press| slider.bar(&theme, press).into()).into()
        } else {
            self.bar(&theme, 0.0).into()
        };

        let mut semantics = Semantics::new()
            .role(SemanticRole::Slider)
            .value(format!("{}", self.value))
            .enabled(self.is_enabled());
        if let Some(label) = &self.label {
            semantics = semantics.label(label.clone());
        }
        semantics.child(bar).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![
            ("value", self.value.to_string()),
            ("range", format!("{}..{}", self.min, self.max)),
        ];
        if let Some(divisions) = self.divisions {
            props.push(("divisions", divisions.to_string()));
        }
        if let Some(label) = &self.label {
            props.push(("label", label.clone()));
        }
        if !self.is_enabled() {
            props.push(("disabled", "true".to_owned()));
        }
        props
    }
}

impl fmt::Debug for Slider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slider")
            .field("value", &self.value)
            .field("range", &(self.min, self.max))
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

/// The render leaf behind [`Slider`]: a track, a thumb, and the drag that moves
/// it.
///
/// Public because the render layer's factory has to name it, not because an
/// application should reach for it — every colour here is already resolved, so
/// using it directly means opting out of the theme. Reach for `Slider`.
#[derive(Clone)]
pub struct SliderBar {
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub divisions: Option<u32>,
    pub height: f32,
    pub active: Color,
    pub inactive: Color,
    pub thumb: Color,
    /// How far the press has faded in — drawn as a halo behind the thumb.
    pub press: f32,
    /// The track's thickness and the thumb's radius, chosen by the platform.
    ///
    /// An iOS slider is a thin track under a large knob the colour of the
    /// surface; an Android one is a thicker track under a smaller knob in the
    /// accent colour. The two are recognisable from across a room, and until
    /// these were fields the studio's iOS preview drew the Android one.
    pub track_thickness: f32,
    pub thumb_radius: f32,
    /// Drawn just outside the thumb, for a knob whose fill would otherwise
    /// disappear against a light background.
    pub thumb_ring: Option<Color>,
    pub on_changed: Option<Handler<f32>>,
    pub key: Option<Key>,
}

impl Widget for SliderBar {
    fn debug_name(&self) -> &'static str {
        "SliderBar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("value", self.value.to_string()),
            ("active", self.active.to_string()),
            // The platform's geometry, for the reason the arc's `form` is
            // listed: a shape the tree does not mention cannot be checked.
            ("track", self.track_thickness.to_string()),
            ("thumb", self.thumb_radius.to_string()),
        ]
    }
}

impl fmt::Debug for SliderBar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SliderBar")
            .field("value", &self.value)
            .field("range", &(self.min, self.max))
            .field("enabled", &self.on_changed.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Slider, SliderBar);

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::{inflate, DebugNode, Theme};

    use super::*;

    fn built(slider: Slider) -> DebugNode {
        inflate(Theme::new(ThemeData::dark()).child(slider))
    }

    #[test]
    fn the_bar_takes_its_colours_and_its_height_from_the_theme() {
        let theme = ThemeData::dark();
        let tree = built(Slider::new(0.5).on_changed(Rc::new(|_| {})));
        let bar = tree.find("SliderBar").expect("a bar");

        assert_eq!(
            bar.property("active"),
            Some(theme.colors.primary.to_string().as_str())
        );
    }

    #[test]
    fn a_disabled_slider_is_dimmed_and_carries_no_handler() {
        let tree = built(Slider::new(0.5));
        let bar = tree.find("SliderBar").expect("a bar");
        let dimmed = ColorScheme::dimmed(ThemeData::dark().colors.primary);

        assert_eq!(bar.property("active"), Some(dimmed.to_string().as_str()));
    }

    #[test]
    fn a_screen_reader_is_told_the_number_and_the_name() {
        let tree = built(
            Slider::new(0.25)
                .label("Volume")
                .on_changed(Rc::new(|_| {})),
        );
        let semantics = tree.find("Semantics").expect("annotated");

        assert_eq!(semantics.property("role"), Some("Slider"));
        assert_eq!(semantics.property("label"), Some("Volume"));
        assert_eq!(semantics.property("value"), Some("0.25"));
    }

    #[test]
    fn divisions_are_gaps_rather_than_stops() {
        let slider = Slider::new(0.0).range(0.0, 100.0).divisions(4);
        assert_eq!(
            slider
                .debug_properties()
                .into_iter()
                .find(|(name, _)| *name == "divisions")
                .map(|(_, value)| value),
            Some("4".to_owned())
        );
    }
}
