//! The shell, and the regions it is assembled from.
//!
//! One module per region of the mockup, and one `Shell` that puts them in a
//! column. Every region takes the whole [`Studio`] and
//! reads only the signals it needs, which is what makes a rebuild cost the
//! region that changed rather than the window.

pub mod activity_bar;
pub mod brand;
pub mod chrome;
pub mod completion;
pub mod context_menu;
pub mod dialog;
pub mod divider;
pub mod editor;
pub mod find_bar;
pub mod icons;
pub mod icons_filled;
pub mod inspector;
pub mod overlays;
pub mod palette;
pub mod panel;
pub mod picker;
pub mod preview;
pub mod scrollbar;
pub mod shortcuts;
pub mod sidebar;
pub mod splash;
pub mod status_bar;
pub mod title_bar;
pub mod tooltip;
pub mod welcome;

use vieww_foundation::{Alignment, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, Flexible, Inherited};

pub use shortcuts::Shortcuts;

use crate::state::{Studio, MAX_SIDEBAR, MIN_PANE, MIN_SIDEBAR};
use crate::theme::StudioTheme;
use crate::ui::chrome::{card, space, CARD_GAP};

/// The whole application: keys, theme, chrome, overlay host, and the bands.
#[derive(Debug)]
pub struct Shell {
    pub studio: Studio,
}

impl Widget for Shell {
    fn debug_name(&self) -> &'static str {
        "Shell"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // **The theme, with the token editor's overrides applied.**
        //
        // `theme_data(dark)` and `studio_theme(dark)` are still the base — see
        // `Studio::themed` for why the edits are a sparse map re-applied over
        // whichever base is current rather than a theme stored whole.
        let (theme_data, studio_theme) = self.studio.themed();

        // `Overlay` inside `Theme` and outside the body, for the reason the
        // framework's own catalogue gives: an entry has to be built with the
        // application's colours and drawn above everything else.
        //
        // **The platform's services, published above everything.**
        //
        // This is what gives the text fields a pasteboard. `RenderEditableText`
        // implements cut, copy and paste in full and reads its `Clipboard`
        // from `SharedServices` in the inherited scope — with nothing there it
        // reports the keys unhandled, which is why ⌘C did nothing. Not a
        // missing feature: a missing three lines.
        //
        // Above the theme, because a service is not a visual concern and
        // anything below here may want one.
        let services = self.studio.services.as_ref().clone();

        // Reduce motion is deliberately *not* set from the studio's own
        // setting: `preview_reduce_motion` is about the previewed screen, and
        // the preview publishes its own below. The studio's chrome has one
        // animation — the press wash — and turning it off is not a setting
        // anybody has asked for.
        let accessibility = vieww_foundation::Accessibility {
            text_scale: self.studio.ui_scale(),
            high_contrast: self.studio.high_contrast.get(),
            ..vieww_foundation::Accessibility::default()
        };

        // The shortcut layer is **outside** the theme, and that is not an
        // accident of ordering: keys bubble outward from the focused object, so
        // whatever catches the ones nothing else wanted has to be the last
        // ancestor on the way out. Anything wrapped around it would be catching
        // them first.
        Shortcuts {
            studio: self.studio.clone(),
            // **The studio's own accessibility preferences, published where
            // the framework already looks for them.** `vieww-foundation`'s
            // `Accessibility::text_scale` is applied to every `Text` and
            // `TextField` in `vieww-render`'s factory — the one funnel they all
            // pass through — so publishing it here is the whole of "larger
            // interface text". Doing it any other way would mean every one of
            // several hundred call sites multiplying a size, and one of them
            // forgetting.
            //
            // The preview subtree publishes its *own* over the top of this, for
            // the reason `ui::preview` gives: the simulated device's text scale
            // is a different setting about a different screen.
            child: Inherited::new(
                services,
                Inherited::new(
                    accessibility,
                    Theme::new(theme_data).child(Inherited::new(
                        studio_theme,
                        Overlay::new().child(
                            Stack::new()
                                .alignment(Alignment::TOP_CENTER)
                                .children(children![
                                    Body {
                                        studio: self.studio.clone()
                                    },
                                    // Both paint over everything and are absent
                                    // from the tree when shut. They live here
                                    // rather than inside the regions they belong
                                    // to because a panel clipped to the strip it
                                    // hangs from is a panel nobody sees.
                                    title_bar::MenuLayer {
                                        studio: self.studio.clone()
                                    },
                                    palette::Palette {
                                        studio: self.studio.clone()
                                    },
                                    // Above the palette: both are modal, and a
                                    // folder picker opened over one has to be what
                                    // Escape closes first. `Studio::escape` orders
                                    // them the same way.
                                    picker::FolderPicker {
                                        studio: self.studio.clone()
                                    },
                                    // Above the picker, because it is the one
                                    // modal that is not a choice the user opened —
                                    // it is a question the window asked on its way
                                    // out, and nothing may be drawn over it.
                                    welcome::Welcome {
                                        studio: self.studio.clone()
                                    },
                                    welcome::About {
                                        studio: self.studio.clone()
                                    },
                                    context_menu::ContextMenuLayer {
                                        studio: self.studio.clone()
                                    },
                                    dialog::ReplacePlanDialog {
                                        studio: self.studio.clone()
                                    },
                                    dialog::NamePromptDialog {
                                        studio: self.studio.clone()
                                    },
                                    dialog::DeletePrompt {
                                        studio: self.studio.clone()
                                    },
                                    dialog::ConflictPrompt {
                                        studio: self.studio.clone()
                                    },
                                    dialog::QuitPrompt {
                                        studio: self.studio.clone()
                                    },
                                    dialog::LiveCaution {
                                        studio: self.studio.clone()
                                    },
                                    dialog::LiveMissing {
                                        studio: self.studio.clone()
                                    },
                                    // Above everything, including the modals: the
                                    // overlays are a picture of the window as it
                                    // was drawn, and a picture with a hole where a
                                    // dialog is would be a lie about the frame.
                                    // Passthrough, so it never takes a click.
                                    overlays::Overlays {
                                        studio: self.studio.clone()
                                    },
                                    // Above everything, including the overlays: the splash covers the
                                    // chrome during launch. Leaves the tree when `studio.splash` goes `None`.
                                    // Under the splash and over everything else: an
                                    // explanation of a control is worth nothing on
                                    // top of a modal that has taken the control
                                    // away, and worth everything over the chrome.
                                    tooltip::TooltipLayer {
                                        studio: self.studio.clone()
                                    },
                                    splash::Splash {
                                        studio: self.studio.clone()
                                    },
                                ]),
                        ),
                    )),
                ),
            )
            .into(),
        }
        .into()
    }
}

widget_node_from!(Shell);

/// Title bar, the working area, status bar.
#[derive(Debug)]
struct Body {
    studio: Studio,
}

impl Widget for Body {
    fn debug_name(&self) -> &'static str {
        "Body"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);

        // The card shell. Every region below is a rounded surface with a
        // hairline of its own, and this paints the ground they sit on — which
        // is the only thing here that is genuinely new. Before it the regions
        // met edge to edge and a `hairline` was the separator; now the gutter
        // is, and the hairlines that used to draw a region's outer edge have
        // been deleted from the five regions that drew one.
        Container::new()
            .color(chrome.window)
            .padding(EdgeInsets::all(CARD_GAP))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .spacing(CARD_GAP)
                    .children(children![
                        card(
                            &chrome,
                            chrome.chrome_0,
                            title_bar::TitleBar {
                                studio: self.studio.clone()
                            }
                        ),
                        Flexible::expanded(1).child(Workspace {
                            studio: self.studio.clone()
                        }),
                        card(
                            &chrome,
                            chrome.chrome_0,
                            status_bar::StatusBar {
                                studio: self.studio.clone()
                            }
                        ),
                    ]),
            )
            .into()
    }
}

widget_node_from!(Body);

/// Activity bar, sidebar, and the main column beside them.
#[derive(Debug)]
struct Workspace {
    studio: Studio,
}

impl Widget for Workspace {
    fn debug_name(&self) -> &'static str {
        "Workspace"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);

        // No `spacing` on this row, and that is not an oversight: the divider
        // between the sidebar and the editor *is* the gutter, so a spacing
        // here would put a second one beside it. Where there is no divider —
        // the activity bar's right edge — the gutter is an explicit `space`.
        //
        // **The activity bar leaves the tree when it is hidden**, the way the
        // right pane does rather than the way the sidebar collapses: it has
        // no divider to keep the place of, so a zero-width card and the gap
        // beside it would be forty-nine points of layout answering to
        // nothing. `Command::ToggleActivityBar` (Ctrl+Alt+U, the View menu)
        // is the switch; the choice persists in the settings file beside
        // `right_open`.
        let mut row: Vec<WidgetNode> = Vec::new();
        if self.studio.activity_bar_open.get() {
            row.push(
                card(
                    &chrome,
                    chrome.chrome_0,
                    activity_bar::ActivityBar {
                        studio: self.studio.clone(),
                    },
                )
                .into(),
            );
            row.push(space(CARD_GAP));
        }
        row.push(
            card(
                &chrome,
                chrome.chrome_1,
                sidebar::Sidebar {
                    studio: self.studio.clone(),
                },
            )
            .into(),
        );
        row.push(
            divider::VerticalDivider {
                width: self.studio.sidebar_width.clone(),
                grows: divider::Grows::Leading,
                min: MIN_SIDEBAR,
                max: MAX_SIDEBAR,
                active: self.studio.sidebar_dragging.clone(),
                hovered: self.studio.sidebar_seam_hovered.clone(),
            }
            .into(),
        );
        row.push(
            Flexible::expanded(1)
                .child(MainColumn {
                    studio: self.studio.clone(),
                })
                .into(),
        );

        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(row)
            .into()
    }
}

widget_node_from!(Workspace);

/// The editor and preview, with the bottom panel under both.
#[derive(Debug)]
struct MainColumn {
    studio: Studio,
}

impl Widget for MainColumn {
    fn debug_name(&self) -> &'static str {
        "MainColumn"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);

        // **Absent, not zero-width, when it is shut.** The sidebar collapses to
        // a width so its divider stays put and one press brings the same width
        // back; the right pane has no divider to preserve on its far side, and
        // a zero-width card still costs a build and a layout every frame.
        let mut row: Vec<WidgetNode> = vec![Flexible::expanded(1)
            .child(card(
                &chrome,
                chrome.chrome_2,
                editor::EditorGroup {
                    studio: self.studio.clone(),
                },
            ))
            .into()];
        if self.studio.right_open.get() {
            row.push(
                divider::VerticalDivider {
                    width: self.studio.preview_width.clone(),
                    grows: divider::Grows::Trailing,
                    min: MIN_PANE,
                    // **The clamp follows the window now.** It was a flat
                    // 900.0 with a note saying the window width "is not
                    // knowable here" — which stopped being true once the
                    // shell started reporting its size into
                    // `Studio::window_size`. On a 3440-wide monitor the old
                    // number capped the preview at a quarter of the screen and
                    // handed the rest to the editor, whether or not that was
                    // what the user was looking at.
                    //
                    // Seventy per cent rather than everything: the editor keeps
                    // a usable column at every window size, so dragging the
                    // divider all the way right cannot leave the code pane a
                    // sliver. The floor keeps the bound above `MIN_PANE` on a
                    // window too small for the ratio to clear it, because a max
                    // below the min is a divider that cannot move at all.
                    max: (self.studio.window_size.get().width * 0.7).max(MIN_PANE * 2.0),
                    active: self.studio.preview_dragging.clone(),
                    hovered: self.studio.preview_seam_hovered.clone(),
                }
                .into(),
            );
            row.push(
                card(
                    &chrome,
                    chrome.chrome_1,
                    preview::PreviewPane {
                        studio: self.studio.clone(),
                    },
                )
                .into(),
            );
        }

        let panes = Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(row);

        // **The seam above the panel is the gutter, exactly as beside the
        // panes.** So there is no `spacing` on this column when the divider is
        // in it: a spacing *and* a divider would put two gutters at one seam
        // and the shell's rhythm would break at the bottom of the window.
        //
        // Only while the panel is open. Collapsed, the panel is its tab strip
        // and nothing else — there is no height to drag, and a seam offering to
        // resize a strip that is already at its minimum is a control that does
        // nothing. The strip's own click brings the body back at the height it
        // had, which is the "hider" half of the request and already worked.
        let mut column: Vec<WidgetNode> = vec![Flexible::expanded(1).child(panes).into()];
        if self.studio.panel_open.get() {
            column.push(
                divider::HorizontalDivider {
                    height: self.studio.panel_height.clone(),
                    min: crate::state::MIN_PANEL,
                    max: crate::state::max_panel(self.studio.window_size.get().height),
                    active: self.studio.panel_dragging.clone(),
                    hovered: self.studio.panel_seam_hovered.clone(),
                }
                .into(),
            );
        } else {
            column.push(space(CARD_GAP));
        }
        column.push(
            card(
                &chrome,
                chrome.chrome_1,
                panel::Panel {
                    studio: self.studio.clone(),
                },
            )
            .into(),
        );

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(column)
            .into()
    }
}

widget_node_from!(MainColumn);
