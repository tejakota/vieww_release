//! The 48-pixel column of view switches, and the selection rail beside them.
//!
//! A press here follows VS Code's rule rather than plain switching: the icon
//! of the view already showing folds the sidebar away, and any icon pressed
//! while the pane is folded brings it back with that view. Every icon in the
//! column is therefore a hide/reveal switch for the pane — the Explorer's
//! folder at the top included, and nothing privileged about it — while a
//! press on an icon whose view is not the one showing still just switches.

use vieww_foundation::{Alignment, Color, EdgeInsets, Offset, Rect, Size, Sketchbook};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Animated, Container, Flexible, Semantics, SizedBox, Stack};

use crate::state::{Studio, View};
use crate::theme::StudioTheme;
use crate::ui::chrome::label_bold;

pub const WIDTH: f32 = 48.0;

#[derive(Debug)]
pub struct ActivityBar {
    pub studio: Studio,
}

impl Widget for ActivityBar {
    fn debug_name(&self) -> &'static str {
        "ActivityBar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        let current = self.studio.view.get();
        let studio = self.studio.clone();
        let problems = self.studio.diagnostics.get().len();

        // The bar is split: the first five views sit at the top, the last two
        // at the bottom, with the slack between them. Splitting by index keeps
        // `View::ALL` the single list — a second array here is how the enum and
        // the bar drift apart.
        let (top, bottom) = View::ALL.split_at(5);

        let button = |view: View| -> WidgetNode {
            let selected = view == current;
            let signal = self.studio.view.clone();
            let studio_for_click = studio.clone();
            let badge = matches!(view, View::Problems) && problems > 0;
            let colors = theme.colors;
            let look = *chrome;
            // Read out here rather than inside: see `chrome::quick`.
            let (duration, curve) = crate::ui::chrome::quick(ctx);

            // **The only label these have.** An activity-bar entry is an icon
            // and nothing else — there is no text in the subtree for a screen
            // reader to read off, so without this the whole navigation column
            // announces as nine unnamed buttons.
            // **Where it is, and whether the pointer is on it** — the two facts a
            // tooltip needs and a widget cannot know about itself. `Measured`
            // answers the first during paint (one frame late, which for a
            // pointer that is already resting there is invisible), and the
            // detector answers the second. Neither takes the press: the
            // detector has no tap handler, so the `sensed` button underneath is
            // still what a click reaches.
            let anchors = studio.view_anchors.clone();
            let index = view.index();
            let hovered = studio.hovered_view.clone();
            let leaving = studio.hovered_view.clone();

            Semantics::button(view.title())
                .child(
                    vieww_widget::Measured::new()
                        .on_measured(std::rc::Rc::new(move |rect: vieww_foundation::Rect| {
                            anchors.borrow_mut()[index] = rect;
                        }))
                        .child(
                            vieww_widget::GestureDetector::new()
                                .on_hover(move |inside| {
                                    if inside {
                                        hovered.set(Some(view));
                                    } else if leaving.peek() == Some(view) {
                                        // Only the button being left clears it. Two buttons'
                                        // enter and leave can arrive in either order, and a
                                        // leave that cleared unconditionally would blank the
                                        // tooltip of the button the pointer had just moved on to.
                                        leaving.set(None);
                                    }
                                })
                                .child(crate::ui::chrome::sensed(
                                    move |sense| {
                                        // **Selection is animated, not switched.**
                                        //
                                        // The rail used to appear and vanish
                                        // whole. Nine icons in a column where
                                        // exactly one carries a two-point bar
                                        // is a state that is legible only once
                                        // the eye is already there; a rail that
                                        // grows, an icon that warms towards the
                                        // accent, and a wash that fades up
                                        // behind it are three cues arriving
                                        // together, which is what makes the
                                        // switch findable in peripheral vision.
                                        Animated::new(if selected { 1.0 } else { 0.0 })
                                            .duration(duration)
                                            .curve(curve)
                                            .key(format!("activity-{}", view.index()))
                                            .build(move |t| {
                                                activity_button(
                                                    ActivityLook {
                                                        chrome: look,
                                                        colors,
                                                        sense,
                                                        selected: t,
                                                        badge: if badge {
                                                            Some(problems)
                                                        } else {
                                                            None
                                                        },
                                                    },
                                                    view,
                                                )
                                            })
                                            .into()
                                    },
                                    move || {
                                        // VS Code's rule for this column, which is what makes
                                        // every icon in it a hide/reveal switch for the pane
                                        // instead of a switcher that is already lit: a press
                                        // brings its view in, revealing the sidebar if it was
                                        // folded, but a press on the view already showing
                                        // folds the pane away. The folding press moves the
                                        // width and not the view — the icon stays lit, so the
                                        // pane's absence reads as "collapsed", not "switched".
                                        let pane_open = studio_for_click.sidebar_width.peek() > 0.0;
                                        if selected && pane_open {
                                            studio_for_click.sidebar_width.set(0.0);
                                        } else {
                                            signal.set(view);
                                            if !pane_open {
                                                studio_for_click
                                                    .sidebar_width
                                                    .set(crate::state::OPEN_SIDEBAR);
                                            }
                                        }
                                        if matches!(view, View::Problems) {
                                            let tab = studio_for_click.panel_tab.clone();
                                            let open = studio_for_click.panel_open.clone();
                                            tab.set(crate::state::PanelTab::Problems);
                                            open.set(true);
                                        }
                                    },
                                )),
                        ),
                )
                .into()
        };

        let column = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(
                top.iter()
                    .copied()
                    .map(button)
                    .chain(std::iter::once(
                        Flexible::expanded(1).child(SizedBox::shrink()).into(),
                    ))
                    .chain(bottom.iter().copied().map(button))
                    .collect::<Vec<_>>(),
            );

        // No trailing hairline: the card shell gives every region its own
        // border, and a hairline inside one is a second line a pixel away from
        // the first.
        Container::new()
            .color(chrome.chrome_0)
            .width(WIDTH)
            .padding(EdgeInsets::symmetric(0.0, 6.0))
            .child(column)
            .into()
    }
}

widget_node_from!(ActivityBar);

/// The icon each view is switched by.
///
/// Named `view_icon` rather than `glyph`, which is what `chrome::glyph` is —
/// two functions called the same thing in one file, one taking a `View` and one
/// an `IconData`, is a compile error waiting for whoever adds an argument.
///
/// The mapping lives here rather than on `View` because `state.rs` describes
/// what the studio *is*, and an `IconData` is a picture of it — a state module
/// that returned paths would make the enum unusable from a test that never
/// draws.
fn view_icon(view: View) -> vieww_foundation::IconData {
    use crate::ui::icons_filled as icons;
    match view {
        View::Explorer => icons::folder(),
        View::Search => icons::search(),
        View::Snippets => icons::code(),
        View::Problems => icons::warning(),
        View::Inspector => icons::dashboard(),
        View::Learn => icons::lightbulb(),
        View::Docs => icons::book(),
        View::Toolchain => icons::tune(),
        View::Source => icons::branch(),
        View::Export => icons::export(),
        View::Tokens => icons::swatch(),
        View::Settings => icons::gear(),
    }
}

/// Everything one activity-bar entry needs to draw itself at a given moment.
///
/// A struct rather than six parameters because the builder closure already
/// captures four things and a reader counting commas at the call site is how a
/// tint ends up passed as a wash.
#[derive(Debug, Clone, Copy)]
struct ActivityLook {
    chrome: StudioTheme,
    colors: ColorScheme,
    sense: Sense,
    /// How selected this entry is, `0.0..=1.0`, mid-animation.
    selected: f32,
    /// The problem count, when this is the Problems entry and there are any.
    badge: Option<usize>,
}

/// How big an activity-bar icon is drawn.
///
/// **Twenty-four**, which is the size the set was drawn at: every path in
/// `ui::icons` is on the 24-unit grid, so this is the one size at which no
/// coordinate is scaled at all and a tenth of a design unit is a tenth of a
/// pixel. It is also what VS Code puts in a 48-pixel activity bar, which is
/// what this column is.
///
/// It used to be 21, chosen to force `RenderIcon` into a **one**-pixel pen
/// because a two-pixel one "picked the softness straight back up". Both halves
/// of that turned out to be the renderer's fault rather than the size's:
///
/// * A one-pixel pen has no antialiasing headroom. Every corner, diagonal and
///   curve on it is drawn at partial coverage of a single pixel, so it falls
///   below what the eye reads as a line — which is why the set looked *thin and
///   faint*, and why it appeared to sharpen under the pointer, where a brighter
///   colour lifts those same partial pixels into view.
/// * The softness at 24 was `hint_stems`' predecessor snapping every vertex in
///   both axes, which deformed exactly the curves and diagonals it could not
///   help. It now hints stems only, so the size that fits the grid is also the
///   size that draws cleanest.
///
/// At 24 the pen is 2 physical pixels at 1:1 — the weight the major outlined
/// icon sets are drawn at, and the weight this set was designed for.
const ICON: f32 = 24.0;

/// The colour of an activity-bar icon nobody is pointing at.
///
/// **Derived from the chrome rather than fixed**, which is the bug this
/// replaced: it was a literal `#AEAEAE` in both themes. Against the dark bar's
/// `#181818` that is 8.2:1 and perfectly good; against the light bar's
/// `#F8F8F8` it is **2.1:1** — under the 3:1 floor WCAG sets for a graphic that
/// carries meaning, and comfortably into the range where an icon reads as a
/// watermark. A whole navigation column was effectively invisible in the light
/// theme, and it "came back" on hover only because hover mixes towards
/// `on_surface`, which *is* theme-aware.
///
/// A step brighter than `on_surface_variant` in each theme: this is a
/// two-pixel line rather than a run of text, so it can afford to sit a little
/// under the label weight without going quiet. Both directions clear 4.5:1
/// against their own bar.
fn rest_ink(chrome: &StudioTheme) -> Color {
    if chrome.dark {
        Color::hex(0xB4_B4B4)
    } else {
        Color::hex(0x5A_5A5A)
    }
}

/// One entry: wash, rail, icon and badge, all driven by `look.selected`.
fn activity_button(look: ActivityLook, view: View) -> WidgetNode {
    let ActivityLook {
        chrome,
        colors,
        sense,
        selected,
        badge,
    } = look;
    let t = selected.clamp(0.0, 1.0);

    // The icon warms from the muted tint towards the accent as it is selected,
    // and towards the plain foreground under the pointer. Two separate
    // journeys, because hovering an unselected icon should not preview the
    // accent — that would make hover and selection the same signal.
    let resting = mix(rest_ink(&chrome), colors.on_surface, sense.emphasis());
    let tint = mix(resting, chrome.accent, t);

    let mut layers: Vec<WidgetNode> = vec![
        // The wash behind a selected entry: the accent at low strength, which
        // is what keeps the rail from being the only thing carrying the state.
        // `behind` rather than a bare painting — see its docs for the two ways
        // an unsized one silently draws nothing or everything.
        crate::ui::chrome::behind(ActivitySelection {
            wash: chrome.accent_wash,
            rail: chrome.accent,
            soft: chrome.accent_soft,
            t,
        }),
        Container::new()
            .color(crate::ui::chrome::hover_overlay(colors, sense))
            .size(WIDTH, 40.0)
            .alignment(Alignment::CENTER)
            .child(Icon::new(view_icon(view)).size(ICON).color(tint))
            .into(),
    ];

    if let Some(problems) = badge {
        layers.push(
            Positioned::new()
                .right(3.0)
                .bottom(2.0)
                .child(
                    Container::new()
                        .color(colors.error)
                        .radius(6.5)
                        .size(13.0, 13.0)
                        .alignment(Alignment::CENTER)
                        .shadow(vieww_foundation::Shadow::new(
                            Color::rgba(0, 0, 0, 90),
                            Offset::new(0.0, 1.0),
                            3.0,
                        ))
                        .child(label_bold(&problems.to_string(), 9.0, colors.on_error)),
                )
                .into(),
        );
    }

    Stack::new().children(layers).into()
}

/// `from` moved `t` of the way towards `to`.
fn mix(from: Color, to: Color, t: f32) -> Color {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "t is clamped to 0..=1, so the product is 0..=255"
    )]
    let alpha = (t.clamp(0.0, 1.0) * 255.0) as u8;
    to.with_alpha(alpha).over(from)
}

/// The wash and the rail that mark the selected entry.
#[derive(Debug)]
struct ActivitySelection {
    wash: Color,
    rail: Color,
    soft: Color,
    t: f32,
}

impl vieww_widget::Painter for ActivitySelection {
    fn paint(&self, book: &mut Sketchbook, size: Size) {
        if self.t <= 0.01 {
            return;
        }
        let t = self.t.clamp(0.0, 1.0);
        // The wash fades up as a rounded plate inset from the bar's edges.
        book.rrect(
            Rect::new(6.0, 4.0, size.width - 6.0, size.height - 4.0),
            8.0,
            fade(self.wash, t),
        );
        // The rail grows from its own centre, so switching views reads as one
        // marker sliding rather than two blinking.
        let half = (size.height / 2.0 - 8.0) * t;
        let centre = size.height / 2.0;
        let bar = Rect::new(0.0, centre - half, 2.5, centre + half);
        book.layer(t * 0.8, 4.0, None, |inner| {
            inner.rrect(bar, 1.25, self.soft);
        });
        book.rrect(
            bar,
            1.25,
            vieww_foundation::Gradient::vertical().between(self.rail, self.soft),
        );
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `color` at `t` of its own alpha.
fn fade(color: Color, t: f32) -> Color {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "t is clamped to 0..=1 and a is a u8, so the product is 0..=255"
    )]
    let alpha = (f32::from(color.a) * t.clamp(0.0, 1.0)) as u8;
    color.with_alpha(alpha)
}
