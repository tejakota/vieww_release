//! The five pieces every region of the shell is made of.
//!
//! Written once because an IDE is mostly the same four shapes repeated: a
//! hairline, a row of fixed height, a piece of small text, an icon button,
//! and a slack-absorbing gap. Each region composing its own would let the tab
//! strip and the panel strip end up a pixel apart, which is exactly the class
//! of drift the mockup was built to settle.

use vieww_foundation::{
    Alignment, Border, Color, EdgeInsets, FontFamily, FontWeight, IconData, Offset, Rect, Shadow,
    Size, Sketchbook, TextStyle,
};
use vieww_widget::prelude::*;
use vieww_widget::{
    Align, Animated, Clip, Container, Flexible, Painting, Pressable, Semantics, SizedBox, Stack,
};

use crate::theme::StudioTheme;

/// The radius every region of the shell is cut to.
///
/// A field of `ThemeData::metrics` would be the tidier home, and it is the
/// wrong one: `metrics.corner` is the radius of a *control* — a button, a
/// field, a card in someone's application — and an IDE's window chrome has no
/// business moving when an app author retunes it. Same argument as
/// [`StudioTheme`] itself.
pub const CARD_CORNER: f32 = 8.0;

/// The gutter between two regions, and the window's own inset.
///
/// It is also the width of a [`VerticalDivider`](crate::ui::divider), on
/// purpose: the drag handle *is* the gutter, so the split costs no pixels of
/// its own and two adjacent cards are exactly this far apart whether or not
/// they can be resized.
pub const CARD_GAP: f32 = 6.0;

/// One region of the shell: a rounded surface with a hairline around it.
///
/// # Why the child is clipped and the border is padded away from it
///
/// `Container::radius` rounds what the *container* paints and deliberately
/// does not clip its child — a clip is a compositing operation and a rounded
/// fill is not, so the framework charges for it only when asked. A region is
/// exactly the case that has to ask: a tab strip with its own background would
/// otherwise paint square corners over the rounded ones and the card would
/// have no radius at all.
///
/// The one-pixel padding is the other half of the same problem. Without it the
/// clip runs to the outer edge and the child's background covers the border it
/// was drawn to sit inside, so every card loses its outline to its own
/// contents.
///
/// # Why it is a gradient and not a fill
///
/// Every region in the shell used to be one flat colour, and a window of eight
/// flat rectangles separated by hairlines reads as a diagram of an editor
/// rather than as a set of surfaces. The sheen is deliberately faint — twelve
/// of 255 in the dark theme, fading out by a third of the way down — because
/// the effect wanted is *"light is falling on this from above"* and anything
/// legible as a ramp on a flat panel reads as a mistake instead.
///
/// The same light direction is what the [`elevation`](StudioTheme::elevation)
/// shadow and the [`rim`](StudioTheme::rim) highlight assume: lit from the top,
/// shadow cast downward. Three cues, one story.
#[must_use]
pub fn card(chrome: &StudioTheme, background: Color, child: impl Into<WidgetNode>) -> Container {
    Container::new()
        .gradient(chrome.sheen(background))
        .color(background)
        .radius(CARD_CORNER)
        .shadow(chrome.elevation(2))
        .border(Border {
            color: chrome.line,
            width: 1.0,
        })
        .padding(EdgeInsets::all(1.0))
        .child(
            Clip::rounded(CARD_CORNER - 1.0).child(Stack::new().children(children![
                        child.into(),
                        // The rim: one pixel of caught light along the top
                        // edge, above the child so a region that paints its
                        // own strip does not cover it.
                        Positioned::new()
                            .left(0.0)
                            .right(0.0)
                            .top(0.0)
                            .height(1.0)
                            .child(Container::new().color(chrome.rim(background))),
                    ])),
        )
}

/// A one-pixel separator. Horizontal by default; `vertical` for a column edge.
#[must_use]
pub fn hairline(color: Color) -> WidgetNode {
    Container::new().color(color).height(1.0).into()
}

/// A one-pixel separator running down the screen.
#[must_use]
pub fn hairline_vertical(color: Color) -> WidgetNode {
    Container::new().color(color).width(1.0).into()
}

/// Text at a size and colour, on the UI face.
#[must_use]
pub fn label(text: &str, size: f32, color: Color) -> Text {
    Text::new(text).style(TextStyle {
        color,
        size,
        line_height: 1.35,
        ..TextStyle::new(size)
    })
}

/// The same, bolder — a tab that is selected, a section header.
#[must_use]
pub fn label_bold(text: &str, size: f32, color: Color) -> Text {
    Text::new(text).style(TextStyle {
        color,
        size,
        weight: FontWeight::Medium,
        line_height: 1.35,
        ..TextStyle::new(size)
    })
}

/// Fixed-pitch text: code, line numbers, a diagnostic's `17:31`.
#[must_use]
pub fn mono(text: &str, size: f32, color: Color) -> Text {
    Text::new(text).style(TextStyle {
        color,
        size,
        family: FontFamily::Monospace,
        line_height: 1.55,
        ..TextStyle::new(size)
    })
}

/// A row of fixed height with a background and horizontal padding.
///
/// Every strip in the shell — title bar, tab strip, panel tabs, status bar —
/// is one of these, which is why they align.
#[must_use]
pub fn strip(
    height: f32,
    background: Color,
    padding: f32,
    child: impl Into<WidgetNode>,
) -> Container {
    Container::new()
        .color(background)
        .height(height)
        .padding(EdgeInsets::symmetric(padding, 0.0))
        .child(child)
}

/// Slack: the thing between the left group and the right group of a strip.
#[must_use]
pub fn gap() -> WidgetNode {
    Flexible::expanded(1).child(SizedBox::shrink()).into()
}

/// A fixed-width empty box, for spacing inside a row.
#[must_use]
pub fn space(width: f32) -> WidgetNode {
    SizedBox::width(width).into()
}

/// A clickable region that dims very slightly while held.
///
/// `Pressable` hands its builder a press amount in `0..=1`; the shell uses it
/// for a wash rather than a scale, because a title-bar button that moves is
/// wrong in a way a phone button is not.
#[must_use]
pub fn clickable(
    build: impl Fn() -> WidgetNode + 'static,
    on_tap: impl Fn() + 'static,
) -> Pressable {
    Pressable::new(move |_press| build()).on_tap(on_tap)
}

/// A `clickable` whose subtree is built from how the pointer is treating it.
///
/// # Why this exists rather than a wash applied for you
///
/// The obvious version of hover — wrap every `clickable` in a translucent
/// overlay — cannot work in this shell. Half these controls paint their own
/// opaque background, so a wash *behind* them is invisible; the other half
/// contain a second `clickable` (a tab's close button, a row's inline action),
/// and a wash *in front* of them would swallow its taps. There is no
/// `IgnorePointer` in the library to opt the overlay out.
///
/// So the control decides. It is handed a [`Sense`] and blends its own
/// background with [`hovered`], which costs one line at each call site and
/// leaves nested controls hit-testable.
#[must_use]
pub fn sensed(
    build: impl Fn(Sense) -> WidgetNode + 'static,
    on_tap: impl Fn() + 'static,
) -> Pressable {
    Pressable::sensed(build).on_tap(on_tap)
}

/// `base`, lifted toward the theme's hover tint by how much the pointer is on
/// it.
///
/// # Why an alpha composite rather than a second colour per control
///
/// A hover that is "this control's colour, plus a little light" holds together
/// across ten surfaces at four different depths — the activity bar, the sidebar,
/// two tab strips, the menus — where ten hand-picked hover colours drift the
/// moment one of the underlying depths is retuned. `chrome_4` has been the
/// theme's declared hover tint since it was written; until now nothing read it.
///
/// The ceiling is deliberate. A full-strength hover reads as *selected*, and a
/// row that looked selected under the pointer would make the actual selection
/// unreadable — which is the thing hover is supposed to be helping with.
#[must_use]
pub fn hovered(base: Color, chrome: &crate::theme::StudioTheme, sense: Sense) -> Color {
    let strength = sense.emphasis().clamp(0.0, 1.0);
    if strength <= f32::EPSILON {
        return base;
    }
    // 0..=64 of 255: enough to see against a six-point luminance step, far
    // enough below the selected fill that the two never read as the same state.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "strength is clamped to 0..=1, so the product is 0..=64"
    )]
    let alpha = (strength * 64.0) as u8;
    chrome.chrome_4.with_alpha(alpha).over(base)
}

/// A hover wash for a control that paints **no background of its own** — a row
/// in a list, a tree item, a settings line.
///
/// # Why this one reads the colour scheme and [`hovered`] reads the chrome
///
/// A control with its own fill knows what it is sitting on and can blend
/// towards the chrome's hover tint. A transparent one does not: the same row
/// appears over `chrome_1` in the sidebar and `chrome_2` in a popover, and a
/// fixed tint would be a lightening on one and a darkening on the other.
/// `on_surface` is the scheme's "whatever contrasts with the ground here"
/// colour — near-white in the dark theme and near-black in the light one — so a
/// few percent of it lifts a row in both without either being special-cased.
#[must_use]
pub fn hover_overlay(colors: ColorScheme, sense: Sense) -> Color {
    let strength = sense.emphasis().clamp(0.0, 1.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "strength is clamped to 0..=1, so the product is 0..=20"
    )]
    let alpha = (strength * 20.0) as u8;
    colors.on_surface.with_alpha(alpha)
}

/// The shadow a selected tab casts, so selection is a *depth* rather than six
/// points of luminance.
///
/// # Why the selected tab needed more than a colour
///
/// Selected and unselected differed by `chrome_2` against `chrome_1` — a step
/// small enough that on a dim laptop panel, or with the high-contrast chrome
/// off, the only reliable signal left was the 3px marker along the top edge.
/// The marker is right and it is not enough on its own: it is at the far edge of
/// the shape it describes, so the eye has to already be looking at the strip.
///
/// A shadow makes the selected tab a raised surface, which is what every editor
/// people arrive from already teaches. Downward and soft, because the light in
/// this shell comes from above — the same direction the palette and the menus
/// are lit from.
#[must_use]
pub fn selection_shadow(dark: bool) -> vieww_foundation::Shadow {
    // Heavier in the dark theme: a shadow is a darkening, and it has to travel
    // further to read against a dark ground than a light one.
    let alpha = if dark { 110 } else { 55 };
    vieww_foundation::Shadow::new(
        Color::rgba(0, 0, 0, alpha),
        vieww_foundation::Offset::new(0.0, 2.0),
        7.0,
    )
}

/// A `clickable` that announces itself as a button called `label`.
///
/// # Why this helper exists rather than a rule people remember
///
/// `Semantics`, `SemanticRole` and `Liveness` have been in `vieww-widget`
/// since before the studio was written, and there was not one reference to any
/// of them in the whole of `apps/viewwstudio` — sixty commands, nine sidebar
/// views, five panel tabs, and nothing that announced a role or a name. The
/// studio is both the framework's flagship application and the evidence that
/// its accessibility story works; it was evidence of the opposite.
///
/// A helper rather than a convention, because a convention is a thing every
/// future control has to be reminded of and a helper is one it gets by using
/// the same function everything else uses. Everything shaped like a button in
/// this shell goes through `clickable`; this is that with a name attached, and
/// the plain one is left for the cases where the label belongs on an ancestor.
#[must_use]
pub fn button(
    label: impl Into<String>,
    build: impl Fn() -> WidgetNode + 'static,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    Semantics::button(label)
        .child(clickable(build, on_tap))
        .into()
}

/// A named region whose contents stay reachable — a pane, a strip, a panel.
///
/// `merge(false)`, which is what makes it a region rather than one control:
/// a screen reader says what this is and can still walk into it.
#[must_use]
pub fn region(label: impl Into<String>, child: impl Into<WidgetNode>) -> WidgetNode {
    Semantics::container(label).child(child).into()
}

/// A square icon button: the shape every strip's trailing actions use.
#[must_use]
pub fn icon_button(
    data: IconData,
    size: f32,
    color: Color,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    // Unnamed, because an icon has no text to derive a name from and this
    // helper does not know what the icon means. `named_icon_button` is the one
    // to reach for; this stays for the handful of call sites whose label sits
    // on an ancestor region.
    // **Every icon button in the shell hovers, because this is all of them.**
    //
    // A 24x24 glyph with no label and no border is the least self-evident
    // control in the window — the studio has fourteen of them and, until this,
    // not one gave any sign of being pressable before it was pressed. The wash
    // is a rounded square rather than a circle: these sit in strips, a hair
    // apart, and circles in a row read as a set of dots.
    sensed(
        move |sense| {
            Container::new()
                .color(hover_overlay_neutral(color, sense))
                .radius(5.0)
                .size(24.0, 24.0)
                .alignment(Alignment::CENTER)
                .child(glyph(data.clone(), size, color))
                .into()
        },
        on_tap,
    )
    .into()
}

/// A hover wash derived from the icon's own colour.
///
/// [`hover_overlay`] wants a [`ColorScheme`], and [`icon_button`] does not have
/// one — it is handed a single colour and nothing else, at fourteen call sites
/// that would all have to start threading a theme through to get one. The
/// icon's tint is already a contrasting colour against whatever it sits on,
/// which is the only property the wash needs.
#[must_use]
fn hover_overlay_neutral(icon: Color, sense: Sense) -> Color {
    let strength = sense.emphasis().clamp(0.0, 1.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "strength is clamped to 0..=1, so the product is 0..=28"
    )]
    let alpha = (strength * 28.0) as u8;
    icon.with_alpha(alpha)
}

/// An icon button that says what it does.
///
/// **The one control that cannot name itself.** Text-bearing buttons have a
/// label a screen reader can read off the tree; an icon is a path and a colour.
/// Every icon button in a shell is therefore a thing somebody using a screen
/// reader cannot identify, which is why this exists and why the unnamed one
/// above says so.
#[must_use]
pub fn named_icon_button(
    label: impl Into<String>,
    data: IconData,
    size: f32,
    color: Color,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    Semantics::button(label)
        .child(icon_button(data, size, color, on_tap))
        .into()
}

/// A keyboard hint — `⌘P` in the omnibar, `⌘⏎` on the Render button.
///
/// Drawn as a chip rather than as plain text so it reads as a key and not as
/// part of the sentence beside it.
#[must_use]
pub fn kbd(text: &str, foreground: Color, background: Color, border: Color) -> WidgetNode {
    Container::new()
        .color(background)
        .radius(4.0)
        .height(16.0)
        .padding(EdgeInsets::symmetric(5.0, 0.0))
        .alignment(Alignment::CENTER)
        .border(Border {
            color: border,
            width: 1.0,
        })
        .child(mono(text, 9.5, foreground))
        .into()
}

/// A compact segmented control, 24 points tall.
///
/// `vieww-widget`'s `SegmentedControl` is the right control and the wrong
/// height here: it floors itself at `Metrics::touch_target`, which is a finger
/// on a phone and half a toolbar on a desktop. Rather than fight it with a
/// constraint — the floor is deliberate, and overriding it in the theme would
/// shrink every control in the studio — the toolbar builds its own from the
/// same parts, and leaves the framework's alone for the screens that need a
/// finger-sized target.
/// The four colours a segmented track is drawn from.
///
/// A struct rather than four parameters because the call site was already at
/// eight arguments and the next reader would have had to count commas to know
/// which colour was the fill and which the border.
#[derive(Debug, Clone, Copy)]
pub struct SegmentedColors {
    /// The track behind the segments.
    pub track: Color,
    pub border: Color,
    /// The selected segment's fill, and the near end of its ramp.
    pub accent: Color,
    /// The far end of the selected segment's ramp.
    pub accent_far: Color,
    /// Text on the selected segment.
    pub on_accent: Color,
    /// Text on every other segment.
    pub muted: Color,
}

#[must_use]
pub fn segmented(
    ctx: &BuildContext,
    labels: &[&str],
    selected: usize,
    colors: SegmentedColors,
    on_selected: impl Fn(usize) + 'static,
) -> WidgetNode {
    let SegmentedColors {
        track: background,
        border,
        accent,
        accent_far,
        on_accent,
        muted,
    } = colors;

    let on_selected = std::rc::Rc::new(on_selected);
    let count = labels.len().max(1);

    // **Uniform segments, because the thumb slides between them.**
    //
    // Content-width segments and a sliding thumb cannot both be had without
    // measuring text, and a thumb that guesses its own width is a thumb that
    // is a pixel short of its label on one entry in six. Uniform is also what
    // every segmented control on every platform does.
    let width = labels
        .iter()
        .map(|text| segment_width(text))
        .fold(SEGMENT_MIN, f32::max);

    let cells = labels
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let text = (*text).to_string();
            let chosen = index == selected;
            let handler = std::rc::Rc::clone(&on_selected);
            clickable(
                move || {
                    Container::new()
                        // No fill: the thumb behind provides it, and a segment
                        // that painted its own would cover the thumb sliding
                        // under it.
                        .width(width)
                        .height(SEGMENT_HEIGHT)
                        .alignment(Alignment::CENTER)
                        .child(if chosen {
                            label_bold(&text, 11.5, on_accent)
                        } else {
                            label(&text, 11.5, muted)
                        })
                        .into()
                },
                move || handler(index),
            )
            .into()
        })
        .collect::<Vec<WidgetNode>>();

    #[expect(
        clippy::cast_precision_loss,
        reason = "a segmented control has single-digit segments"
    )]
    let target = selected.min(count - 1) as f32;

    let (duration, curve) = quick(ctx);
    let thumb = Animated::new(target)
        .duration(duration)
        .curve(curve)
        .build(move |t| {
            // `Align` places a fixed-size child fractionally, so the slide is one
            // number: -1.0 is hard left, 1.0 is hard right, and the segments are
            // evenly spaced between them.
            #[expect(
                clippy::cast_precision_loss,
                reason = "a segmented control has single-digit segments"
            )]
            let span = (count - 1) as f32;
            let x = if span <= 0.0 {
                0.0
            } else {
                (t / span).mul_add(2.0, -1.0)
            };
            Align::new(Alignment::new(x, 0.0))
                .child(
                    Container::new()
                        .gradient(
                            vieww_foundation::Gradient::linear(
                                Offset::new(0.0, 0.0),
                                Offset::new(1.0, 1.0),
                            )
                            .between(accent, accent_far),
                        )
                        .color(accent)
                        .radius(4.0)
                        .size(width, SEGMENT_HEIGHT)
                        .shadow(Shadow::new(
                            Color::rgba(0, 0, 0, 70),
                            Offset::new(0.0, 1.0),
                            3.0,
                        )),
                )
                .into()
        });

    // 3 points of padding inside a 1-point border, so the selected segment's
    // fill sits *within* the track rather than against it. At 2 points the
    // blue met the border and the control read as one solid block.
    Container::new()
        .color(background)
        .radius(6.0)
        .height(26.0)
        .padding(EdgeInsets::all(3.0))
        .border(Border {
            color: border,
            width: 1.0,
        })
        .child(
            SizedBox::width(width * count as f32).child(Stack::new().children(children![
                    thumb,
                    Flex::row()
                        .main_axis_size(MainAxisSize::Min)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(cells),
                ])),
        )
        .into()
}

/// The height of one segment, and of the thumb that slides behind them.
const SEGMENT_HEIGHT: f32 = 18.0;

/// The narrowest a segment gets, however short its label.
const SEGMENT_MIN: f32 = 46.0;

/// Roughly how wide a segment's label needs.
///
/// An estimate, and it says so: the text stack can measure this properly and
/// doing it here would mean threading a font store through a pure-layout
/// helper for a control whose labels are one short word. 6.4 points per
/// character at 11.5pt medium is measured off the shipped UI face and rounded
/// up; the padding either side is what keeps the estimate's error invisible.
fn segment_width(text: &str) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a segment label is a word, not a document"
    )]
    let characters = text.chars().count() as f32;
    characters.mul_add(6.4, 20.0)
}

// ---------------------------------------------------------------- new parts

/// A soft coloured wash under whatever is stacked on top of it.
#[derive(Debug)]
pub struct GlowUnder {
    pub color: Color,
    /// 0 draws nothing at all, so a resting control costs no layer.
    pub strength: f32,
    pub radius: f32,
}

impl GlowUnder {
    /// This glow as a widget, for a call site that is building a `Stack`.
    #[must_use]
    pub fn widget(self) -> WidgetNode {
        Painting::new(self).into()
    }
}

impl vieww_widget::Painter for GlowUnder {
    fn paint(&self, book: &mut Sketchbook, size: Size) {
        if self.strength <= 0.01 {
            return;
        }
        book.layer(self.strength, 7.0, None, |inner| {
            inner.rrect(
                Rect::new(3.0, 3.0, size.width - 3.0, size.height + 1.0),
                self.radius,
                self.color,
            );
        });
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The accent bar that marks a selected tab, drawn as a ramp with a bloom.
///
/// Its `t` is how selected it is, which is what lets it grow out of the strip
/// rather than appear on it.
#[derive(Debug)]
pub struct TabIndicator {
    pub accent: Color,
    pub accent_soft: Color,
    pub t: f32,
}

impl TabIndicator {
    /// This indicator as a widget, for a call site building a row.
    #[must_use]
    pub fn widget(self) -> WidgetNode {
        Painting::new(self).into()
    }
}

impl vieww_widget::Painter for TabIndicator {
    fn paint(&self, book: &mut Sketchbook, size: Size) {
        let t = self.t.clamp(0.0, 1.0);
        if t <= 0.01 {
            return;
        }
        // Grows from the middle out, so switching tabs reads as one indicator
        // travelling rather than two fading past each other.
        let half = size.width * 0.5 * t;
        // Anchored to the box it was given rather than to the top of it: the
        // editor pins this along a tab's top edge and the panel along its
        // bottom, and a bar that always drew at y=0 would float three points
        // above the panel's strip.
        let bar = Rect::new(
            size.width / 2.0 - half,
            size.height - 2.5,
            size.width / 2.0 + half,
            size.height,
        );
        book.layer(t * 0.7, 5.0, None, |inner| {
            inner.rrect(bar, 1.5, self.accent);
        });
        book.rrect(
            bar,
            1.5,
            vieww_foundation::Gradient::horizontal().between(self.accent, self.accent_soft),
        );
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The two motion values an implicit animation inside a `'static` closure
/// needs, read out where a `BuildContext` still exists.
///
/// `Animated::themed(ctx, …)` is the right call and it cannot be made from
/// inside a `Pressable` builder: those closures outlive the build that made
/// them, so capturing `ctx` is a borrow that escapes. Reading the two `Copy`
/// values here and passing them in is the same theme lookup with a lifetime
/// that fits.
#[must_use]
pub fn quick(ctx: &BuildContext) -> (std::time::Duration, Curve) {
    let motion = ThemeData::of(ctx).motion;
    (motion.duration_short, motion.curve_standard)
}

/// The slower pair, for something changing shape rather than tint.
#[must_use]
pub fn considered(ctx: &BuildContext) -> (std::time::Duration, Curve) {
    let motion = ThemeData::of(ctx).motion;
    (motion.duration_medium, motion.curve_emphasized)
}

/// A [`Painting`] stretched to fill the [`Stack`] it is placed in.
///
/// # Why every painting in this shell goes through here
///
/// A `Painting` with no requested size takes the largest box its constraints
/// allow, and *collapses to nothing* on an axis that is unbounded — which is
/// the honest answer and a trap, because the two places a decoration naturally
/// goes are both unbounded on one axis. A non-positioned child of a `Stack` is
/// loosely constrained by the incoming constraints, not by the stack's eventual
/// size, so a wash behind a 40-point button grew to the height of the whole
/// column; a child of a row inside a horizontal `Scrollable` has no bounded
/// width at all, so a tab's indicator laid out three points tall and zero wide
/// and drew nothing. Both were silent.
///
/// `Positioned` on all four sides is the fix for both: it is laid out *after*
/// the stack has sized itself to its non-positioned children, against that
/// size, tightly. The painting fills exactly the thing it is decorating.
#[must_use]
pub fn behind(painter: impl vieww_widget::Painter) -> WidgetNode {
    Positioned::new()
        .left(0.0)
        .right(0.0)
        .top(0.0)
        .bottom(0.0)
        .child(Painting::new(painter))
        .into()
}

/// A chrome icon: the studio's centreline set, drawn as a stroke at the weight
/// that matches its size.
///
/// # Why every call site goes through here
///
/// Because `Icon::new(data).size(n).color(c)` *fills*, and this set's paths are
/// centrelines — the same call that draws a filled glyph correctly draws one
/// of these as a blot. Fifteen call sites each remembering to add `.stroke()`
/// with the right number is fifteen chances to get the weight wrong; one
/// function is none.
#[must_use]
pub fn glyph(data: IconData, size: f32, color: Color) -> Icon {
    glyph_icon(data).size(size).color(color)
}

/// The same, for a call site that sets its own size and colour afterwards.
#[must_use]
pub fn glyph_icon(data: IconData) -> Icon {
    Icon::new(data).stroke(crate::ui::icons::WEIGHT)
}

/// A small rounded label — a count, a state, a language name.
#[must_use]
pub fn pill(text: &str, foreground: Color, background: Color) -> WidgetNode {
    Container::new()
        .color(background)
        .radius(7.0)
        .height(14.0)
        .padding(EdgeInsets::symmetric(6.0, 0.0))
        .alignment(Alignment::CENTER)
        .child(label_bold(text, 9.5, foreground))
        .into()
}

/// A dot that reads as a state: rendered as a disc with a matching halo.
#[must_use]
pub fn status_dot(color: Color, lit: bool) -> WidgetNode {
    Painting::sized(
        Size::square(10.0),
        StatusDot {
            color,
            halo: if lit { 0.55 } else { 0.0 },
        },
    )
    .into()
}

#[derive(Debug)]
struct StatusDot {
    color: Color,
    halo: f32,
}

impl vieww_widget::Painter for StatusDot {
    fn paint(&self, book: &mut Sketchbook, size: Size) {
        let centre = Offset::new(size.width / 2.0, size.height / 2.0);
        if self.halo > 0.0 {
            book.layer(self.halo, 3.0, None, |inner| {
                inner.circle(centre, 4.0, self.color);
            });
        }
        book.circle(centre, 3.0, self.color);
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
