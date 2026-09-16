//! Every ordinary control, on one screen, to be *looked at* and pressed.
//!
//! ```console
//! cargo run -p vieww-platform-winit --example controls --release
//!
//! # the same screen read right-to-left
//! cargo run -p vieww-platform-winit --example controls --release -- --rtl
//! ```
//!
//! # What this is for
//!
//! `TRACKER.md`'s example audit found eleven ordinary controls that were fully
//! tested and had **never been drawn** — `Checkbox`, `Switch`, `Slider`, `Chip`,
//! `Menu`, `DatePicker`, `TimePicker`, `Image`, `Svg`, `Badge`, `Avatar`. A unit
//! test can tell you a checkbox reports `true` after a tap. It cannot tell you
//! the tick is centred, that the box is the same height as the label beside it,
//! or that the whole row reads as one control rather than two things that happen
//! to be adjacent.
//!
//! Those are whole-screen judgements made in about a second, and this is the
//! screen to make them on.
//!
//! # Things to check
//!
//! 1. **Every control lines up on its left edge**, and each row is one touch
//!    target tall. A control that is visibly shorter than its neighbours is one
//!    whose `Metrics::touch_target` is not being honoured — the failure
//!    `crates/vieww/tests/touch_targets.rs` catches numerically, seen directly.
//!
//! 2. **Press each one.** Checkbox, switch and chip change on press; the slider
//!    tracks the drag; the menu opens over the screen and closes on selection.
//!    The value beside each control is read from the same signal the control
//!    writes, so a control that looks like it changed and does not update the
//!    text is not actually reporting.
//!
//! 3. **`Badge` sits on the corner of what it badges**, and `Avatar` falls back
//!    to initials. Under `--rtl` the badge moves to the other corner — it is
//!    positioned directionally, which is the thing to verify here.
//!
//! 4. **The date and time pickers are the two densest grids in the framework.**
//!    Look for a weekday column that does not line up with its header, and for
//!    the selected day's circle being off-centre in its cell.
//!
//! # Run it in release
//!
//! Debug builds of the layout pass are ten to thirty times slower.

use std::rc::Rc;

use vieww_element::{Runtime, ScrollController, Signal};
use vieww_foundation::{Date, Image as Pixels, Size, TextDirection, Time};
use vieww_gestures::ScrollPhysics;
use vieww_platform_winit::App;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Directionality};

/// The dropdown's options.
///
/// Three, deliberately: enough that the tick beside the current one is a
/// distinguishable thing to look at, few enough that the list does not need to
/// scroll — which is a separate behaviour and not what this row is showing.
const COUNTRIES: [&str; 3] = ["Ireland", "Japan", "Peru"];

const SURFACE: Size = Size {
    width: 880.0,
    height: 720.0,
};

/// The theme's own surface, for `drawer.rs`'s reason: a screen that picks its
/// own background cannot drift from what an application using `Theme` gets.
const BACKGROUND: Color = ThemeData::dark().colors.surface;

fn main() {
    let rtl = std::env::args().skip(1).any(|arg| arg == "--rtl");

    let app = App::new()
        .title("vieww — controls")
        .size(SURFACE)
        .background(BACKGROUND);

    let result = app.run(move |driver: &mut FrameDriver| {
        let runtime = driver.elements().runtime().clone();
        driver.set_root(Shell {
            state: State::new(&runtime),
            rtl,
        });
    });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("controls failed: {error}"),
    }
}

/// One signal per control, held above the tree.
///
/// Every control here is **controlled** — it renders the value it is given and
/// reports the value it wants, and never holds state of its own. That is the
/// framework's design rather than this example's convenience, and it is the
/// reason a screen like this can exist at all: the value beside each control is
/// a second reader of the same signal, so a control that draws one thing and
/// reports another is visible immediately instead of at integration time.
#[derive(Debug, Clone)]
struct State {
    checked: Signal<bool>,
    switched: Signal<bool>,
    volume: Signal<f32>,
    chips: Signal<u32>,
    menu_open: Signal<bool>,
    picked: Signal<usize>,
    /// The dropdown's chosen country, and whether its list is showing.
    country: Signal<Option<usize>>,
    country_open: Signal<bool>,
    /// Where the closed dropdown ended up, so its list opens against it.
    ///
    /// `Measured` is the only way a widget learns where it landed, and a widget
    /// may not hold state — so the anchor round-trips through the application,
    /// which is the same contract `Menu` has.
    country_anchor: Signal<Option<Rect>>,
    date: Signal<Date>,
    time: Signal<Time>,
    /// The page's scroll position.
    ///
    /// The catalogue is taller than any window it is worth opening — 1064pt of
    /// controls in 664 — so without this the bottom of the time picker is
    /// simply unreachable, which the overflow report said and nobody read.
    page: ScrollController,
}

impl State {
    fn new(runtime: &Runtime) -> Self {
        Self {
            checked: runtime.signal(true),
            switched: runtime.signal(false),
            volume: runtime.signal(0.4_f32),
            chips: runtime.signal(0_u32),
            menu_open: runtime.signal(false),
            picked: runtime.signal(usize::MAX),
            country: runtime.signal(None),
            country_open: runtime.signal(false),
            country_anchor: runtime.signal(None),
            date: runtime.signal(today()),
            time: runtime.signal(Time::new(9, 30).expect("09:30 is a real time")),
            page: ScrollController::new(runtime, ScrollPhysics::android()),
        }
    }
}

#[derive(Debug)]
struct Shell {
    state: State,
    rtl: bool,
}

impl Widget for Shell {
    fn debug_name(&self) -> &'static str {
        "Shell"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let direction = if self.rtl {
            TextDirection::Rtl
        } else {
            TextDirection::Ltr
        };
        // **`Overlay` inside the theme, around everything else.** Every
        // overlay in the framework — a menu, a dropdown's list, a tooltip, a
        // scrim — is decided deep in the tree and has to be drawn at the top of
        // it, and this is the place that lets it. Inside `Theme` so an entry is
        // built with the application's colours rather than the defaults;
        // outside `Body` so it covers the whole window.
        Theme::new(ThemeData::dark())
            .child(
                Overlay::new().child(Directionality::new(direction).child(Body {
                    state: self.state.clone(),
                })),
            )
            .into()
    }
}

widget_node_from!(Shell);

/// The screen: two columns of controls, with the menu overlaid when it is open.
#[derive(Debug)]
struct Body {
    state: State,
}

impl Widget for Body {
    fn debug_name(&self) -> &'static str {
        "Body"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let columns = Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(48.0)
            .children(children![
                Flexible::new(1).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(20.0)
                        .children(children![
                            heading("Selection", &theme),
                            Toggles {
                                state: self.state.clone()
                            },
                            heading("Identity", &theme),
                            Identity,
                            heading("Pictures", &theme),
                            Pictures,
                        ])
                ),
                Flexible::new(1).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(20.0)
                        .children(children![
                            heading("Dates and times", &theme),
                            Pickers {
                                state: self.state.clone()
                            },
                        ])
                ),
            ]);

        // The menu goes over everything, so it is a sibling in a `Stack` rather
        // than a child of the button that opens it — a menu inside its own
        // trigger would be clipped by whatever the trigger sits in.
        // **Scrollable, because the catalogue is taller than the window.** 1064pt
        // of controls in 664 of space: without this the bottom of the time
        // picker cannot be reached at all, and a `Flex` that runs out of room
        // places its children anyway rather than clipping — so the overflow was
        // both invisible to a test and unreachable to a user.
        let scroll = self.state.page.clone();
        let page = Scrollable::vertical(scroll.offset())
            .on_drag(scroll.on_drag())
            .on_drag_end(scroll.on_drag_end())
            .on_extents(scroll.on_extents())
            .child(Padding::new(EdgeInsets::all(28.0)).child(columns));

        let mut stack = Stack::new().children(children![page]);
        if self.state.menu_open.get() {
            // **`push`, not `children`.** `Stack::children` *replaces* the
            // list, so this dropped the page and left the menu floating on an
            // empty screen the moment it opened.
            //
            // **The `RepaintBoundary` that used to wrap this is gone, and this
            // is the note that says why so nobody puts it back.**
            //
            // The page above is a `Scrollable` and therefore a layer, and a
            // layer's children once composited over their parent's *whole*
            // recording — so an overlay left in the parent's recording drew
            // under the page it covered, however late it came in the tree.
            // Wrapping the menu in a boundary of its own made it a second layer
            // and bought back tree order. `LayerTree::composite` fixed the cause
            // in `efa6610` by splicing a child in where the paint pass left the
            // hole for it, and ten tests in
            // `crates/vieww/tests/layer_paint_order.rs` cover the flattener.
            //
            // The boundary stayed anyway, because a measurement on 2026-08-16
            // under Xvfb with lavapipe reported that removing it drew **nothing
            // at all** at 78% CPU. **Re-measured on 2026-08-17 on the same rig
            // and it does not reproduce**: the menu draws, the scrim is right,
            // scrolling under it behaves, choosing an item closes it and the
            // page redraws underneath, and the process sits at 2.6% CPU. Taken
            // twice more with two unrelated layout fixes reverted, in case they
            // were what closed it; identical both times.
            //
            // Read that as a defect that can no longer be reproduced rather than
            // one anybody fixed. What is different now is that the claim is
            // tested instead of remembered:
            // `crates/vieww/tests/overlay_to_pixels.rs` mounts exactly this
            // configuration, rasterises the **second** frame through
            // `GpuRenderer::render_damaged` and reads the pixels back — a first
            // frame is a full repaint by construction, which is why every test
            // that came before it could not have seen this whatever it asserted.
            stack = stack.push(MenuOverlay {
                state: self.state.clone(),
            });
        }
        SafeArea::new().child(stack).into()
    }
}

widget_node_from!(Body);

fn heading(text: &str, theme: &ThemeData) -> WidgetNode {
    Text::new(text).style(theme.text.title).into()
}

/// A control and the value it is currently reporting, side by side.
///
/// The pairing is the whole point of this screen — see [`State`].
fn row(control: impl Into<WidgetNode>, reading: &str, theme: &ThemeData) -> WidgetNode {
    Flex::row()
        .main_axis_size(MainAxisSize::Min)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .spacing(16.0)
        .children(children![
            control.into(),
            Text::new(reading)
                .style(theme.text.label)
                .color(theme.colors.on_surface_variant),
        ])
        .into()
}

#[derive(Debug)]
struct Toggles {
    state: State,
}

impl Widget for Toggles {
    fn debug_name(&self) -> &'static str {
        "Toggles"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let (checked, switched) = (self.state.checked.get(), self.state.switched.get());
        let volume = self.state.volume.get();
        let chips = self.state.chips.get();

        let set_checked = self.state.checked.clone();
        let set_switched = self.state.switched.clone();
        let set_volume = self.state.volume.clone();
        let bump_chips = self.state.chips.clone();
        let open_menu = self.state.menu_open.clone();
        let picked = self.state.picked.get();

        // The dropdown reads three signals and writes three, which is more
        // wiring than any other control here — and that is the thing to look
        // at. `country_anchor` is the round trip a widget that may not hold
        // state has to make to know where it is.
        let country = self.state.country.get();
        let country_open = self.state.country_open.get();
        let country_anchor = self.state.country_anchor.get();
        let set_country = self.state.country.clone();
        let toggle_country = self.state.country_open.clone();
        let set_anchor = self.state.country_anchor.clone();

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .main_axis_size(MainAxisSize::Min)
            .spacing(12.0)
            .children(children![
                row(
                    Checkbox::new(checked)
                        .label("Remember me")
                        .on_changed(Rc::new(move |next| set_checked.set(next))),
                    if checked { "true" } else { "false" },
                    &theme
                ),
                row(
                    Switch::new(switched)
                        .label("Aeroplane mode")
                        .on_changed(Rc::new(move |next| set_switched.set(next))),
                    if switched { "on" } else { "off" },
                    &theme
                ),
                row(
                    Constrained::new(Constraints::tight_for_width(220.0)).child(
                        Slider::new(volume)
                            .label("Volume")
                            .divisions(10)
                            .on_changed(Rc::new(move |next| set_volume.set(next)))
                    ),
                    &format!("{:.1}", volume),
                    &theme
                ),
                row(
                    Chip::new("Espresso").on_pressed(move || bump_chips.set(bump_chips.peek() + 1)),
                    &format!("pressed {chips}×"),
                    &theme
                ),
                row(
                    Constrained::new(Constraints::tight_for_width(220.0)).child({
                        let mut dropdown = Dropdown::new(COUNTRIES, country)
                            .label("Country")
                            .placeholder("Choose a country")
                            .open(country_open)
                            .on_measured(Rc::new(move |rect| {
                                // Only when it moves. Writing every frame
                                // would mark the reader pending every
                                // frame, which is the spin `wake_if_pending`
                                // exists to avoid.
                                if set_anchor.peek() != Some(rect) {
                                    set_anchor.set(Some(rect));
                                }
                            }))
                            .on_selected(Rc::new(move |index| {
                                set_country.set(Some(index));
                            }))
                            .on_toggled(Rc::new(move |open| toggle_country.set(open)));
                        if let Some(anchor) = country_anchor {
                            dropdown = dropdown.anchor(anchor);
                        }
                        dropdown
                    }),
                    &match country {
                        Some(index) => COUNTRIES[index].to_owned(),
                        None => "nothing chosen".to_owned(),
                    },
                    &theme
                ),
                row(
                    Button::new("Open a menu").on_pressed(move || open_menu.set(true)),
                    &match picked {
                        usize::MAX => "nothing chosen".to_owned(),
                        index => format!("chose item {index}"),
                    },
                    &theme
                ),
                // A pressable is the raw material every control above is built
                // from: a builder handed the press amount, 0.0 to 1.0. Worth
                // seeing on its own, because a third party's control gets
                // exactly this and nothing else.
                Pressable::new(move |press| {
                    DecoratedBox::rounded(
                        theme.colors.primary.with_alpha(alpha(press)),
                        theme.metrics.corner,
                    )
                    .child(
                        Padding::new(EdgeInsets::symmetric(12.0, 20.0))
                            .child(Text::new("Hold me").style(theme.text.body)),
                    )
                    .into()
                }),
            ])
            .into()
    }
}

widget_node_from!(Toggles);

/// `Avatar` and `Badge` — the two widgets that decorate something else.
#[derive(Debug)]
struct Identity;

impl Widget for Identity {
    fn debug_name(&self) -> &'static str {
        "Identity"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        // **The caption is under the row, not in it.** At their intrinsic widths
        // the six controls and the sentence are 724pt wide in a 388pt column,
        // and a `Flex` with nothing left to give places its children anyway —
        // so the caption used to paint straight across the time picker in the
        // next column. Nothing clips it and no test sees it.
        //
        // Making the caption `Flexible` instead is the obvious repair and is
        // worse: the controls leave no free space at all, so the sentence is
        // handed nothing, wraps to one letter a line, and the row grows tall
        // enough to push the avatars off the bottom of the window. Tried, and
        // recorded here because it looks right in the source either way.
        Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(8.0)
            .children(children![
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(24.0)
                    .children(children![
                        Avatar::initials("Ada Lovelace"),
                        Avatar::initials("Grace Hopper"),
                        // A badge on an icon: the count sits on the corner, and
                        // moves to the other corner under `--rtl`.
                        // **The colour is not optional.** `Icon` deliberately
                        // does not read the theme — it is a leaf, with no
                        // `build` to look one up in — so its default is black,
                        // and a black tick on this surface is an invisible one.
                        // Both of these were unreadable until somebody looked.
                        Badge::new(
                            Icon::new(icons::check())
                                .size(28.0)
                                .color(theme.colors.on_surface)
                        )
                        .count(3),
                        Badge::new(
                            Icon::new(icons::close())
                                .size(28.0)
                                .color(theme.colors.on_surface)
                        )
                        .dot(),
                        // The one control on this screen that is *supposed* to
                        // be bigger than everything else: a floating action
                        // button is the page's single primary action, and it
                        // reads as wrong if it matches the height of the row it
                        // sits in.
                        FloatingActionButton::new(icons::add()).on_pressed(|| {}),
                        FloatingActionButton::new(icons::check())
                            .label("Extended")
                            .on_pressed(|| {}),
                    ]),
                Text::new("initials, a badge, a dot, and two action buttons")
                    .style(theme.text.label),
            ])
            .into()
    }
}

widget_node_from!(Identity);

/// `Image` and `Svg` — raster and vector, side by side at the same size.
///
/// Both are drawn from data built here rather than loaded from a file, because
/// an example that needs an asset on disk is one that fails for a reason that
/// has nothing to do with the widget.
#[derive(Debug)]
struct Pictures;

impl Widget for Pictures {
    fn debug_name(&self) -> &'static str {
        "Pictures"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        // A 2×2 checker, blown up to 72pt. `BoxFit::Fill` with four pixels is
        // the clearest possible read on whether the sampler is doing what the
        // fit says.
        let checker = Pixels::from_rgba8(
            vec![
                220, 80, 80, 255, // red
                40, 40, 48, 255, // near-black
                40, 40, 48, 255, //
                80, 160, 220, 255, // blue
            ],
            2,
            2,
        );

        // Two overlapping shapes, so the viewbox fit is visible: if the parts
        // move relative to each other, the shared fit transform is wrong.
        let mark = VectorImage::new(
            vec![
                VectorShape {
                    path: Path::rect(Rect::new(2.0, 2.0, 16.0, 16.0)),
                    color: theme.colors.primary,
                },
                VectorShape {
                    path: Path::rect(Rect::new(8.0, 8.0, 22.0, 22.0)),
                    color: theme.colors.error,
                },
            ],
            Rect::new(0.0, 0.0, 24.0, 24.0),
        );

        // The caption sits under the row for `Identity`'s reason: three 72pt
        // pictures and a sentence are 499pt wide in a 388pt column, and the
        // sentence was landing on the next column's time picker.
        Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(8.0)
            .children(children![
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(24.0)
                    .children(children![
                        Image::new(checker)
                            .fit(BoxFit::Fill)
                            .label("A four-pixel checker")
                            .width(72.0)
                            .height(72.0),
                        Svg::new(mark).label("Two squares").width(72.0).height(72.0),
                        // `Fitted` scales whatever is inside it down to fit. The
                        // text is deliberately too wide for the box it is given.
                        Constrained::new(Constraints::tight(Size::new(140.0, 72.0))).child(
                            Fitted::new().child(
                                Text::new("scaled to fit")
                                    .style(theme.text.title)
                                    .color(theme.colors.on_surface)
                            )
                        ),
                    ]),
                Text::new("raster, vector, fitted").style(theme.text.label),
            ])
            .into()
    }
}

widget_node_from!(Pictures);

/// The two calendar grids, which are the densest layouts in the framework.
#[derive(Debug)]
struct Pickers {
    state: State,
}

impl Widget for Pickers {
    fn debug_name(&self) -> &'static str {
        "Pickers"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let date = self.state.date.get();
        let time = self.state.time.get();

        let select = self.state.date.clone();
        let page = self.state.date.clone();
        let set_time = self.state.time.clone();

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .main_axis_size(MainAxisSize::Min)
            .spacing(16.0)
            .children(children![
                DatePicker::new(date)
                    .selected(Some(date))
                    .today(today())
                    .on_selected(Rc::new(move |next| select.set(next)))
                    // Paging is a separate signal write from selecting, so an
                    // arrow that moved the selection would be visible here.
                    .on_month_changed(Rc::new(move |next| page.set(next))),
                Text::new(format!(
                    "{:04}-{:02}-{:02}",
                    date.year(),
                    date.month(),
                    date.day()
                ))
                .style(theme.text.label)
                .color(theme.colors.on_surface_variant),
                TimePicker::new(time)
                    .twelve_hour(true)
                    .step(15)
                    .on_changed(Rc::new(move |next| set_time.set(next))),
                Text::new(format!("{:02}:{:02}", time.hour(), time.minute()))
                    .style(theme.text.label)
                    .color(theme.colors.on_surface_variant),
            ])
            .into()
    }
}

widget_node_from!(Pickers);

/// The menu, over the screen, with a barrier that dismisses it.
#[derive(Debug)]
struct MenuOverlay {
    state: State,
}

impl Widget for MenuOverlay {
    fn debug_name(&self) -> &'static str {
        "MenuOverlay"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let close = self.state.menu_open.clone();
        let choose_open = self.state.menu_open.clone();
        let choose = self.state.picked.clone();

        Center::new()
            .child(
                Menu::new(vec![
                    MenuItem::new("Cut").icon(icons::close()),
                    MenuItem::new("Copy").icon(icons::add()),
                    MenuItem::new("Paste").icon(icons::check()),
                ])
                .on_selected(Rc::new(move |index| {
                    choose.set(index);
                    choose_open.set(false);
                }))
                .on_dismiss(move || close.set(false)),
            )
            .into()
    }
}

widget_node_from!(MenuOverlay);

/// The day the calendar opens on and marks as today.
///
/// A constant rather than the system clock, deliberately: an example whose
/// screenshot changes every midnight is one nobody can compare against
/// yesterday's.
fn today() -> Date {
    Date::new(2026, 8, 16).expect("2026-08-16 is a real date")
}

/// A press amount, 0.0 to 1.0, as an alpha byte that never fully disappears.
fn alpha(press: f32) -> u8 {
    let opacity = 0.3 + 0.7 * press.clamp(0.0, 1.0);
    (opacity * 255.0).round() as u8
}
