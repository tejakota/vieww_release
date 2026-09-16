//! The Live Preview: the user's `live.rs`, drawn without compiling anything.
//!
//! # What it is
//!
//! `Render` compiles one `pub fn screen()` and mounts it in the device frame.
//! That answers "does this screen look right" and cannot answer "does the whole
//! flow hang together" — for that you need several screens and the way between
//! them, and compiling a whole app on every keystroke is not a preview.
//!
//! So this draws [`crate::livedoc`]: a `live.rs` at the workspace root that
//! describes screens, their rows, and the buttons between them. The studio
//! parses that file and renders it with its own widgets, in its own process, at
//! typing speed. No `rustc`, no `cdylib`, no ABI check.
//!
//! # It is the user's file, not a demo
//!
//! It used to be three screens of invented content compiled into the studio, the
//! same in every project — asked about directly: *"is the live preview static or
//! does it live preview any workspace that I'm working on?"* It was static, and
//! a preview that shows somebody else's app while calling itself live is worse
//! than no preview.
//!
//! Now: no `live.rs` in the workspace, no live preview. The frame says so and
//! the Render menu offers to write the file. A file that does not parse shows
//! the line and the reason inside the frame, where the person editing it is
//! already looking.
//!
//! # What it deliberately cannot do
//!
//! No logic, no data, no real controls — the switch flips and the counter counts
//! because the *preview* holds those, not because the app does. Everything past
//! the shape of a flow is what `Render` and `Build and Run` are for, and the
//! caution before the first mount says exactly that.

use std::collections::HashMap;

use vieww_element::Signal;
use vieww_foundation::{Alignment, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, Flex};

use crate::livedoc::{LiveDoc, LiveError, LiveItem, LiveScreen};

/// The state the preview holds while it is up, as signals on the studio's
/// `Runtime`.
///
/// Writing any of these rebuilds the studio's tree, which is what makes a tap
/// feel immediate: the route changes, the tree rebuilds, the new screen is on
/// screen in the same frame.
///
/// The route is an **index**, not a name: `LiveDoc::screen` clamps one that has
/// run off the end of a file being edited, so deleting a screen while the
/// preview is up moves it rather than blanking it.
#[derive(Debug, Clone)]
pub struct LiveState {
    pub route: Signal<usize>,
    pub counter: Signal<u32>,
    pub toggle: Signal<bool>,
}

impl LiveState {
    /// A fresh set of signals on `runtime`, all at their defaults.
    #[must_use]
    pub fn new(runtime: &vieww_element::Runtime) -> Self {
        Self {
            route: runtime.signal(0),
            counter: runtime.signal(0),
            toggle: runtime.signal(false),
        }
    }
}

/// What the studio hands the frame: a parsed file, or the reason there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveSource {
    /// A `live.rs` that parsed.
    Doc(LiveDoc),
    /// A `live.rs` that did not.
    Broken(LiveError),
    /// No `live.rs` at the workspace root.
    Missing,
}

/// The live preview, mounted inside the device frame.
#[derive(Debug, Clone)]
pub struct LiveApp {
    pub state: LiveState,
    pub source: LiveSource,
    /// The screens named by `mount` in the file, already resolved to whatever
    /// the studio has compiled this session. Resolved outside the build because
    /// a widget cannot reach the studio, and looked up by the *string the file
    /// wrote* so an unrendered file can be named in the message.
    pub mounts: std::collections::HashMap<String, crate::loaded::Preview>,
}

impl Widget for LiveApp {
    fn debug_name(&self) -> &'static str {
        "LiveApp"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let colors = theme.colors;

        let body: WidgetNode = match &self.source {
            LiveSource::Doc(doc) => self.app(doc, colors),
            LiveSource::Broken(error) => Self::message(
                colors,
                "live.rs has a mistake",
                &format!("Line {}: {}", error.line, error.message),
            ),
            LiveSource::Missing => Self::message(
                colors,
                "No live.rs in this workspace",
                "The Live Preview draws live.rs at the workspace root. Create one \
                 from the Render menu, or press Render to compile a screen instead.",
            ),
        };

        // **Inside the safe area, on a background that is not.**
        //
        // Reported from a screenshot in an iPhone frame: every screen's heading
        // ran underneath the dynamic island. The preview publishes the device's
        // real `ViewMetrics` precisely so a screen can inset itself, and this
        // was the one screen in the studio ignoring them.
        //
        // The colour goes outside and the insets go inside, which is what a real
        // application does: the background reaches the physical edges, the
        // *content* stops at the notch and the home indicator.
        Container::new()
            .color(colors.surface)
            .child(SafeArea::new().child(body))
            .into()
    }
}

widget_node_from!(LiveApp);

impl LiveApp {
    /// The whole app: the active screen, and the bar under it.
    fn app(&self, doc: &LiveDoc, colors: ColorScheme) -> WidgetNode {
        let index = self.state.route.get();
        let Some(screen) = doc.screen(index) else {
            return Self::message(
                colors,
                "Nothing to show",
                "live.rs parsed but has no screens in it.",
            );
        };

        // Names resolved once per build rather than once per button: a screen
        // with twenty buttons should not walk the screen list twenty times, and
        // the map is what a button's handler captures.
        let targets: HashMap<String, usize> = doc
            .screens
            .iter()
            .enumerate()
            .map(|(index, screen)| (screen.name.clone(), index))
            .collect();

        let mut column: Vec<WidgetNode> = vec![Flexible::expanded(1)
            .child(Clip::rect().child(self.screen(screen, &targets, colors)))
            .into()];
        if !doc.nav.is_empty() {
            column.push(self.bottom_nav(doc, &targets, colors));
        }

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(column)
            .into()
    }

    /// One screen: its heading, then its items in the order the file lists them.
    fn screen(
        &self,
        screen: &LiveScreen,
        targets: &HashMap<String, usize>,
        colors: ColorScheme,
    ) -> WidgetNode {
        let mut rows: Vec<WidgetNode> = vec![Container::new()
            .color(colors.surface_variant)
            .padding(EdgeInsets::symmetric(16.0, 14.0))
            .child(
                Text::new(screen.title.clone())
                    .size(24.0)
                    .bold()
                    .color(colors.on_surface),
            )
            .into()];

        for item in &screen.items {
            rows.push(self.item(item, targets, colors));
        }

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    fn item(
        &self,
        item: &LiveItem,
        targets: &HashMap<String, usize>,
        colors: ColorScheme,
    ) -> WidgetNode {
        match item {
            LiveItem::Mount(file) => match self.mounts.get(file) {
                // The user's own screen, exactly as it last compiled.
                Some(preview) => Container::new()
                    .color(colors.surface)
                    .child(preview.node())
                    .into(),
                // Named but never rendered. Said plainly, with the one action
                // that fills it in — a blank space here reads as a bug in the
                // preview rather than as a step not taken yet.
                None => Container::new()
                    .color(colors.surface_variant)
                    .padding(EdgeInsets::all(16.0))
                    .child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(4.0)
                            .children(children![
                                Text::new(file.clone())
                                    .size(13.0)
                                    .bold()
                                    .color(colors.on_surface),
                                Text::new("Open this file and press Render once to place it here.")
                                    .size(11.5)
                                    .color(colors.on_surface_variant),
                            ]),
                    )
                    .into(),
            },
            LiveItem::Row { title, detail } => Self::row(title, detail, colors),
            LiveItem::Text(body) => Container::new()
                .padding(EdgeInsets::symmetric(16.0, 12.0))
                .child(Text::new(body.clone()).style(vieww_foundation::TextStyle {
                    color: colors.on_surface_variant,
                    size: 13.0,
                    line_height: 1.5,
                    ..vieww_foundation::TextStyle::new(13.0)
                }))
                .into(),
            LiveItem::Button { label, target } => {
                let route = self.state.route.clone();
                let index = targets.get(target).copied();
                Container::new()
                    .padding(EdgeInsets::symmetric(16.0, 8.0))
                    .child(Button::new(label.clone()).on_pressed(move || {
                        if let Some(index) = index {
                            route.set(index);
                        }
                    }))
                    .into()
            }
            LiveItem::Toggle(label) => {
                let toggle = self.state.toggle.clone();
                let on = self.state.toggle.get();
                Container::new()
                    .padding(EdgeInsets::symmetric(16.0, 10.0))
                    .child(
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                Flexible::expanded(1).child(
                                    Text::new(label.clone()).size(15.0).color(colors.on_surface)
                                ),
                                crate::ui::chrome::clickable(
                                    move || {
                                        Container::new()
                                            .color(if on {
                                                colors.primary
                                            } else {
                                                colors.surface_variant
                                            })
                                            .radius(14.0)
                                            .size(48.0, 28.0)
                                            .alignment(if on {
                                                Alignment::CENTER_RIGHT
                                            } else {
                                                Alignment::CENTER_LEFT
                                            })
                                            .padding(EdgeInsets::symmetric(3.0, 0.0))
                                            .child(
                                                Container::new()
                                                    .color(colors.surface)
                                                    .radius(11.0)
                                                    .size(22.0, 22.0),
                                            )
                                            .into()
                                    },
                                    move || toggle.set(!toggle.peek()),
                                ),
                            ]),
                    )
                    .into()
            }
            LiveItem::Counter(label) => {
                let value = self.state.counter.get();
                let down = self.state.counter.clone();
                let up = self.state.counter.clone();
                let step = |glyph: &'static str, on_tap: Box<dyn Fn()>| -> WidgetNode {
                    crate::ui::chrome::clickable(
                        move || {
                            Container::new()
                                .color(colors.surface_variant)
                                .radius(16.0)
                                .size(32.0, 32.0)
                                .alignment(Alignment::CENTER)
                                .child(Text::new(glyph).size(18.0).color(colors.on_surface))
                                .into()
                        },
                        on_tap,
                    )
                    .into()
                };
                Container::new()
                    .padding(EdgeInsets::symmetric(16.0, 10.0))
                    .child(
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .spacing(10.0)
                            .children(children![
                                Flexible::expanded(1).child(
                                    Text::new(label.clone()).size(15.0).color(colors.on_surface)
                                ),
                                step(
                                    "\u{2212}",
                                    Box::new(move || {
                                        down.set(down.peek().saturating_sub(1));
                                    })
                                ),
                                Text::new(format!("{value}"))
                                    .size(15.0)
                                    .color(colors.on_surface),
                                step(
                                    "+",
                                    Box::new(move || {
                                        up.set(up.peek().saturating_add(1));
                                    })
                                ),
                            ]),
                    )
                    .into()
            }
        }
    }

    /// A list row: an initial in a circle, a title, and a line under it.
    fn row(title: &str, detail: &str, colors: ColorScheme) -> WidgetNode {
        let initial = title.chars().next().unwrap_or('?').to_string();
        let mut lines: Vec<WidgetNode> = vec![Text::new(title.to_owned())
            .size(15.0)
            .color(colors.on_surface)
            .into()];
        if !detail.is_empty() {
            lines.push(
                Text::new(detail.to_owned())
                    .size(12.0)
                    .color(colors.on_surface_variant)
                    .into(),
            );
        }

        Container::new()
            .color(colors.surface)
            .padding(EdgeInsets::symmetric(16.0, 12.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(12.0)
                    .children(children![
                        Container::new()
                            .color(colors.primary)
                            .radius(20.0)
                            .size(40.0, 40.0)
                            .alignment(Alignment::CENTER)
                            .child(
                                Text::new(initial)
                                    .size(16.0)
                                    .bold()
                                    .color(colors.on_primary)
                            ),
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(2.0)
                            .children(lines),
                    ]),
            )
            .into()
    }

    /// The bar along the bottom, one entry per name on the file's `nav` line.
    fn bottom_nav(
        &self,
        doc: &LiveDoc,
        targets: &HashMap<String, usize>,
        colors: ColorScheme,
    ) -> WidgetNode {
        let current = self.state.route.get();
        let mut items: Vec<WidgetNode> = Vec::new();

        for name in &doc.nav {
            let Some(index) = targets.get(name).copied() else {
                // A `nav` entry naming a screen that is not in the file — which
                // is every keystroke of typing one. Drawn greyed rather than
                // dropped: a bar that loses and regains entries while somebody
                // types looks broken.
                items.push(
                    Flexible::expanded(1)
                        .child(
                            Container::new()
                                .color(colors.surface_variant)
                                .padding(EdgeInsets::symmetric(8.0, 8.0))
                                .alignment(Alignment::CENTER)
                                .child(Text::new(name.clone()).size(11.0).color(colors.outline)),
                        )
                        .into(),
                );
                continue;
            };
            let selected = index == current;
            let route = self.state.route.clone();
            let label = name.clone();
            items.push(
                Flexible::expanded(1)
                    .child(crate::ui::chrome::clickable(
                        move || {
                            Container::new()
                                .color(colors.surface)
                                .padding(EdgeInsets::symmetric(8.0, 8.0))
                                .child(
                                    Flex::column()
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .spacing(4.0)
                                        .children(children![
                                            Container::new()
                                                .color(if selected {
                                                    colors.primary
                                                } else {
                                                    colors.surface_variant
                                                })
                                                .radius(3.0)
                                                .size(24.0, 3.0),
                                            Text::new(label.clone()).size(11.0).color(
                                                if selected {
                                                    colors.primary
                                                } else {
                                                    colors.on_surface_variant
                                                }
                                            ),
                                        ]),
                                )
                                .into()
                        },
                        move || route.set(index),
                    ))
                    .into(),
            );
        }

        Container::new()
            .color(colors.surface_variant)
            .padding(EdgeInsets::symmetric(0.0, 8.0))
            .height(56.0)
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(items),
            )
            .into()
    }

    /// What the frame shows instead of an app: no file, a broken file, an empty
    /// one. Centred and quiet — it is an explanation, not an error page.
    fn message(colors: ColorScheme, heading: &str, body: &str) -> WidgetNode {
        Container::new()
            .color(colors.surface)
            .alignment(Alignment::CENTER)
            .padding(EdgeInsets::all(28.0))
            .child(
                Flex::column()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(8.0)
                    .children(children![
                        Text::new(heading.to_owned())
                            .size(15.0)
                            .bold()
                            .color(colors.on_surface),
                        Text::new(body.to_owned()).style(vieww_foundation::TextStyle {
                            color: colors.on_surface_variant,
                            size: 12.5,
                            line_height: 1.5,
                            ..vieww_foundation::TextStyle::new(12.5)
                        }),
                    ]),
            )
            .into()
    }
}
