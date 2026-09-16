//! The top strip: window buttons, menus, the omnibar, and the view toggles.
//!
//! The menus and the omnibar are both built from [`Command`], not from lists
//! written here — see [`crate::command`]. A menu with its own idea of what
//! "Save" means is a menu that will eventually be wrong about it.

use vieww_foundation::Constraints;
use vieww_foundation::{Alignment, Border, Color, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{
    widget_node_from, Container, Flexible, LayoutBuilder, Positioned, Semantics, SizedBox, Stack,
};

use crate::ui::status_bar::HEIGHT as STATUS_BAR;

use crate::command::{Command, Menu};
use crate::state::Studio;
use crate::theme::StudioTheme;
use crate::ui::chrome::{gap, icon_button, kbd, label, label_bold, space, strip};
use crate::ui::icons;

/// Height of the title bar, in logical pixels. The tab strip matches it within
/// a pixel on purpose — see `editor.rs`.
pub const HEIGHT: f32 = 36.0;

/// How large the application's mark is drawn in the strip.
///
/// Half the strip's height, which is the size at which a two-shape mark still
/// resolves into two shapes rather than a blue smudge, and small enough that it
/// reads as a label on the window rather than as a button in it.
const MARK: f32 = 18.0;

/// How wide the omnibar is.
///
/// A constant now rather than a literal in one builder, because the strip
/// reserves exactly this much space for it in the flow and the centred layer
/// draws exactly this much over the top; two literals that drifted would leave
/// the right-hand icons floating.
const OMNIBAR: f32 = 380.0;

/// Where the menu titles stop, derived rather than measured.
///
/// Same approach and same caveat as [`menu_offset`]: a `Positioned` layer cannot
/// ask a sibling how wide it turned out, so this is the wordmark, the product
/// name, and the titles at their approximate character width. It is used only to
/// decide *whether* there is room to centre — being a few points out moves the
/// decision by a few points of window width, not the omnibar.
fn menus_end() -> f32 {
    const START: f32 = MARK + 10.0 + 86.0 + 8.0;
    const PADDING: f32 = 16.0 + 2.0;
    const PER_CHARACTER: f32 = 6.6;
    Menu::BAR.iter().fold(START, |at, menu| {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a menu title is a handful of characters"
        )]
        let width = menu.title().chars().count() as f32 * PER_CHARACTER + PADDING;
        at + width
    })
}

/// How wide a dropped menu is. One number rather than intrinsic sizing, because
/// menus each as wide as their own longest item make a strip whose panels jump
/// about as you move along it.
// Wide enough for a title and a *reason* on one row, which is what a disabled
// item now shows in place of its chord. 260 fit a title and `Ctrl+Shift+B`;
// "open a folder first" needs the rest.
const MENU_WIDTH: f32 = 380.0;
const MENU_ROW: f32 = 26.0;

#[derive(Debug)]
pub struct TitleBar {
    pub studio: Studio,
}

impl Widget for TitleBar {
    fn debug_name(&self) -> &'static str {
        "TitleBar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        let muted = theme.colors.on_surface_variant;

        let dark = self.studio.dark.clone();
        let panel_open = self.studio.panel_open.clone();
        let right_open = self.studio.right_open.get();
        let toggle_right = self.studio.clone();

        // **The omnibar is centred on the window, not on the slack.**
        //
        // It sat between two `gap()`s, which centres it between the left group
        // and the right group — and the left group is a wordmark and seven menu
        // titles while the right is three icons, so the box landed a long way
        // right of the middle of the window. Reported as "the command palette
        // options on the top can be moved to center", which is what a titlebar
        // search field does everywhere it appears.
        //
        // Centred by drawing it in a `Stack` layer over the strip rather than by
        // balancing the flex: balancing means padding one side by the difference
        // between two groups whose widths are the sum of a wordmark, seven
        // words in a variable-width font, and three icons — a number that is
        // wrong the moment a menu is renamed. The layer is positioned from the
        // strip's *measured* width instead, so it is right at every window size
        // and stays right when the groups change.
        //
        // The row keeps a hole exactly the width of the omnibar where the
        // omnibar used to be, so the right-hand icons do not slide left into the
        // space and end up underneath it.
        let row = Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(children![
                wordmark_glyph(),
                space(10.0),
                label_bold("vieww Studio", 12.5, theme.colors.on_surface),
                space(8.0),
                self.menu_titles(muted, theme.colors, &chrome),
                gap(),
                SizedBox::width(OMNIBAR),
                gap(),
                icon_button(icons::moon(), 15.0, muted, move || dark.set(!dark.peek())),
                icon_button(icons::panel(), 15.0, muted, move || {
                    panel_open.set(!panel_open.peek());
                }),
                // **The preview's own toggle, which did not exist.**
                //
                // The bottom panel had two ways back from being closed — this
                // strip's panel icon, and its tab row, which stays on screen
                // when it collapses. The preview pane had neither: its only
                // control was a `Hide` link *inside itself*, so dismissing it
                // removed the last thing on screen that referred to it. The way
                // back was View ▸ Toggle Preview Pane — an item that was, until
                // the menu was capped above, off the bottom of the window on a
                // laptop. Hiding the pane the product is named for was a
                // one-way door for a mouse.
                //
                // Tinted when the pane is showing, so the icon says which state
                // it is in rather than only what it does.
                icon_button(
                    icons::preview(),
                    15.0,
                    if right_open {
                        theme.colors.primary
                    } else {
                        muted
                    },
                    move || toggle_right.run(Command::ToggleRightPane),
                ),
            ]);

        // The dropped panel is a `Stack` layer over the strip rather than a
        // child of it, so opening a menu does not make the title bar taller and
        // shove the whole window down by two hundred pixels.
        let omnibar = self.omnibar(&chrome, muted);
        let centred = LayoutBuilder::new(move |constraints: Constraints| {
            let width = if constraints.max_width.is_finite() {
                constraints.max_width
            } else {
                // Nothing above bounded the strip, which does not happen in the
                // shell. Fall back to the flow position rather than to a guess.
                return SizedBox::shrink().into();
            };

            // Where the omnibar would sit if it were simply centred, and where
            // the menus end. On a narrow window those overlap — and a search box
            // drawn on top of "Build" is worse than one sitting off-centre — so
            // the layer is only used where it fits, and the flow position is the
            // fallback. `MENUS_END` is derived the same way `menu_offset` is,
            // for the same reason: a `Positioned` cannot ask a sibling how wide
            // it turned out.
            let left = (width - OMNIBAR) / 2.0;
            if left < menus_end() + 12.0 {
                return SizedBox::shrink().into();
            }

            Stack::new()
                .fit(vieww_widget::StackFit::Expand)
                .children(children![Positioned::new()
                    .left(left)
                    .top(0.0)
                    .bottom(0.0)
                    .width(OMNIBAR)
                    .child(
                        vieww_widget::Align::new(vieww_foundation::Alignment::CENTER)
                            .child(omnibar.clone())
                    )])
                .into()
        });

        let bar = Stack::new()
            .fit(vieww_widget::StackFit::Expand)
            .children(children![
                strip(HEIGHT, chrome.chrome_0, 8.0, row),
                // Padded by the same 8 the strip pads by, so the layer's
                // coordinates and the strip's agree.
                Container::new()
                    .padding(EdgeInsets::symmetric(8.0, 0.0))
                    .child(centred),
            ]);
        let bar = SizedBox::height(HEIGHT).child(bar);

        // **The dropped panel is not drawn here.** It used to be, inside a
        // `Stack` in this widget — and a `Stack` is only as tall as its
        // children, so a panel positioned at `top(HEIGHT)` hung outside the
        // title bar's own box and was clipped away by the column below. The
        // menu opened, the state changed, and nothing appeared.
        //
        // It is drawn by `MenuLayer` instead, which the shell puts in the same
        // overlay stack as the command palette, where there is a whole window
        // to hang from. See `ui::mod`.
        bar.into()
    }
}

/// The panel a menu drops, drawn over the whole window.
///
/// Separate from [`TitleBar`] for the reason given there: a dropdown clipped to
/// the strip it hangs from is a dropdown nobody ever sees.
#[derive(Debug)]
pub struct MenuLayer {
    pub studio: Studio,
}

impl Widget for MenuLayer {
    fn debug_name(&self) -> &'static str {
        "MenuLayer"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let Some(open) = self.studio.menu_open.get() else {
            return SizedBox::shrink().into();
        };
        let chrome = StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;

        // A full-window scrim under the panel, so clicking anywhere else shuts
        // the menu — which is what every menu bar does, and what makes one
        // safe to open by accident.
        let studio = self.studio.clone();
        let scrim = crate::ui::chrome::clickable(
            move || Container::new().color(Color::rgba(0, 0, 0, 1)).into(),
            move || studio.open_menu(None),
        );

        let bar = TitleBar {
            studio: self.studio.clone(),
        };

        Stack::new()
            .alignment(Alignment::TOP_LEFT)
            .children(children![
                Positioned::new()
                    .left(0.0)
                    .right(0.0)
                    .top(0.0)
                    .bottom(0.0)
                    .child(scrim),
                Positioned::new()
                    .left(menu_offset(open))
                    .top(HEIGHT)
                    .child(bar.dropdown(open, &chrome, colors)),
            ])
            .into()
    }
}

widget_node_from!(MenuLayer);

widget_node_from!(TitleBar);

impl TitleBar {
    /// File · Edit · View · Render · Help.
    ///
    /// Clicking one opens it; clicking the open one shuts it. No hover
    /// switching between open menus — that needs a pointer-tracking state
    /// machine, and this bar has five items.
    fn menu_titles(&self, muted: Color, colors: ColorScheme, chrome: &StudioTheme) -> WidgetNode {
        let current = self.studio.menu_open.get();
        let items = Menu::BAR
            .iter()
            .copied()
            .map(|menu| {
                let open = current == Some(menu);
                let chrome = *chrome;

                let studio = self.studio.clone();
                Semantics::button(menu.title())
                    .child(crate::ui::chrome::sensed(
                        move |sense| {
                            Container::new()
                                .color(if open {
                                    chrome.chrome_2
                                } else {
                                    crate::ui::chrome::hovered(chrome.chrome_0, &chrome, sense)
                                })
                                .radius(4.0)
                                .padding(EdgeInsets::symmetric(8.0, 3.0))
                                .child(label(
                                    menu.title(),
                                    12.0,
                                    if open || sense.emphasis() > 0.0 {
                                        colors.on_surface
                                    } else {
                                        muted
                                    },
                                ))
                                .into()
                        },
                        move || studio.open_menu(if open { None } else { Some(menu) }),
                    ))
                    .into()
            })
            .collect::<Vec<WidgetNode>>();

        Flex::row().spacing(2.0).children(items).into()
    }

    /// The panel one menu drops.
    ///
    /// # It is capped at the window and scrolls
    ///
    /// It used to be a plain column with no maximum height, and on 2026-08-25
    /// the View menu had reached 32 items: 876 logical pixels hanging from
    /// y=36, which clears a 900px window by 24 pixels and runs straight off the
    /// bottom of any laptop. The last ten commands — Zen Mode, Show Tasks and,
    /// worst of all, **Toggle Preview Pane** — were simply not reachable with a
    /// mouse, and nothing on screen suggested they were there.
    ///
    /// Splitting `Go` out of `View` fixed the immediate case, and would not have
    /// stayed fixed: a menu grows by one item at a time and nobody notices the
    /// one that crosses the edge. So the panel is now bounded by the window it
    /// is in and scrolls when its contents do not fit, which is a property
    /// rather than a headcount.
    pub(crate) fn dropdown(
        &self,
        menu: Menu,
        chrome: &StudioTheme,
        colors: ColorScheme,
    ) -> WidgetNode {
        let rows = Command::ALL
            .iter()
            .copied()
            .filter(|command| command.menu() == menu)
            .map(|command| self.item(command, chrome, colors))
            .collect::<Vec<WidgetNode>>();

        // What is left below the strip, less the status bar and a breath of
        // margin — so a long menu stops short of the window edge rather than
        // painting over the one row that is always supposed to be readable.
        let window = self.studio.window_size.get();
        let room = (window.height - HEIGHT - STATUS_BAR - 12.0).max(MENU_ROW * 4.0);
        #[expect(
            clippy::cast_precision_loss,
            reason = "a menu holds tens of items, not sixteen million"
        )]
        let wanted = rows.len() as f32 * MENU_ROW + 8.0;
        // **A menu that scrolls ends on half a row.**
        //
        // Cut at a whole row it looks finished, and a person who cannot see the
        // two items below the fold has no reason to try scrolling for them —
        // which is the same failure as the overflow, arrived at politely. Half
        // a row of a real item showing is the signal every long list uses.
        //
        // **Only the scrolling case gets a height at all.** A menu that fits is
        // exactly as tall as its rows, and `wanted` is that number computed a
        // second way — which is one number too many. Switching menus rebuilt
        // the rows and the box together, and for the frame in between, the
        // box was still the previous menu's: File's thirteen rows' worth of
        // height around Edit's twenty-two, which printed
        // `RenderColumn overflowed by 234px` on stderr on every menu switch
        // while the frame that actually reached the screen was correct. A box
        // that shrink-wraps its column cannot disagree with it.
        let height = (wanted > room).then(|| {
            let whole = ((room - 8.0) / MENU_ROW).floor().max(4.0);
            whole * MENU_ROW + 8.0 + MENU_ROW / 2.0 - MENU_ROW
        });

        let column = Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows);

        // Only pay for a viewport when there is something to scroll. A menu that
        // fits keeps its exact intrinsic height, so the common case is the same
        // widget it always was.
        let body: WidgetNode = if wanted > room {
            let scroll = self.studio.menu_scroll.clone();
            Clip::rect()
                .child(
                    Scrollable::vertical(scroll.offset())
                        .key("menu")
                        .on_drag(scroll.on_drag())
                        .on_drag_end(scroll.on_drag_end())
                        .on_extents(scroll.on_extents())
                        .child(column),
                )
                .into()
        } else {
            column.into()
        };

        let mut frame = Container::new()
            .color(chrome.chrome_2)
            .radius(6.0)
            .width(MENU_WIDTH);
        if let Some(height) = height {
            frame = frame.height(height);
        }

        frame
            .border(Border {
                color: chrome.line,
                width: 1.0,
            })
            // A dropped menu is a raised surface. It had a hairline and nothing
            // else, so over the editor — which is nearly the same colour — the
            // border was the only thing separating a menu from the code behind
            // it.
            .shadow(crate::ui::chrome::selection_shadow(self.studio.dark.get()))
            .padding(EdgeInsets::symmetric(0.0, 4.0))
            .child(body)
            .into()
    }

    /// One row: the title, and the chord that also runs it — or, when it is
    /// disabled, the reason instead of the chord.
    ///
    /// Greyed when [`Studio::can_run`] says no, and **inert** when greyed — an
    /// item that looks disabled and still fires is worse than one that never
    /// looked disabled at all.
    ///
    /// # A disabled row says why
    ///
    /// Seven of the nine Build rows are greyed in a studio with no folder open,
    /// and the menu used to explain none of them. The disabling is right; the
    /// silence is what makes it read as a broken application rather than an
    /// unconfigured one. [`Studio::why_disabled`] answers from the same check
    /// `can_run` does, so the grey and the reason cannot disagree — and it
    /// takes the chord's place rather than a slot of its own, because a
    /// shortcut for a command you cannot run is the least useful thing that
    /// could be in that space.
    fn item(&self, command: Command, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let studio = self.studio.clone();
        let enabled = studio.can_run(command);
        let reason = studio.why_disabled(command);
        // `studio.chord_for`, not `command.chord()`: a menu that shows the
        // built-in shortcut beside a command the user rebound is a menu that
        // teaches the wrong thing.
        let hint = studio
            .chord_for(command)
            .map(|chord| chord.describe(studio.host))
            .unwrap_or_default();
        let title = command.title();
        let chrome = *chrome;

        // The row's own name, and whether it can be used. A screen reader that
        // reads out a greyed item as though it were live sends somebody to
        // press a thing that does nothing — the same failure the *visible*
        // grey exists to prevent, for the reader who cannot see it.
        Semantics::button(title)
            .enabled(enabled)
            .child(crate::ui::chrome::sensed(
                move |sense| {
                    let mut row: Vec<WidgetNode> = vec![label(
                        title,
                        12.0,
                        if enabled {
                            colors.on_surface
                        } else {
                            colors.outline
                        },
                    )
                    .into()];
                    row.push(gap());
                    if let Some(reason) = reason.as_ref() {
                        // **Flexible and clipped**, because a reason is a sentence
                        // and a menu is a fixed width. `Build ▸ Export` with no
                        // folder open reads "Export packages a project — open a
                        // folder first", which is 86 logical pixels wider than the
                        // row it sits in: it overflowed the panel, painted outside
                        // the menu's own border, and printed a layout warning every
                        // frame the menu was open. Taking the leftover width rather
                        // than its intrinsic width is what bounds it; the clip is
                        // what stops the part that does not fit being drawn anyway.
                        row.push(
                            Flexible::expanded(1)
                                .child(
                                    Clip::rect().child(
                                        Container::new()
                                            .alignment(Alignment::CENTER_RIGHT)
                                            .child(label(reason, 10.5, colors.outline)),
                                    ),
                                )
                                .into(),
                        );
                    } else if hint.is_empty() {
                        row.push(SizedBox::shrink().into());
                    } else {
                        row.push(kbd(
                            &hint,
                            colors.on_surface_variant,
                            chrome.chrome_3,
                            chrome.line,
                        ));
                    }

                    Container::new()
                        // Only a live item lights up. A disabled row that responded
                        // to the pointer would be promising a click it is going to
                        // refuse, which is the exact thing the grey is there to say
                        // in advance.
                        .color(if enabled {
                            crate::ui::chrome::hover_overlay(colors, sense)
                        } else {
                            Color::rgba(0, 0, 0, 0)
                        })
                        .height(MENU_ROW)
                        .padding(EdgeInsets::symmetric(10.0, 0.0))
                        .alignment(Alignment::CENTER_LEFT)
                        .child(
                            Flex::row()
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .children(row),
                        )
                        .into()
                },
                move || {
                    if enabled {
                        studio.run(command);
                    } else {
                        // Still shut the menu. Leaving it open because the item was
                        // disabled reads as an unresponsive window.
                        studio.open_menu(None);
                    }
                },
            ))
            .into()
    }

    /// The centred field. Opens the palette, which is what its chip says.
    fn omnibar(&self, chrome: &StudioTheme, color: Color) -> WidgetNode {
        let studio = self.studio.clone();
        let text = crate::ui::palette::omnibar_hint(&studio);
        let chord = crate::ui::palette::omnibar_chord(&studio);
        let chrome = *chrome;

        crate::ui::chrome::sensed(
            move |sense| {
                Container::new()
                    .color(crate::ui::chrome::hovered(chrome.chrome_2, &chrome, sense))
                    .radius(6.0)
                    .height(24.0)
                    .width(OMNIBAR)
                    .padding(EdgeInsets::symmetric(10.0, 0.0))
                    .alignment(Alignment::CENTER)
                    .border(Border {
                        color: chrome.line,
                        width: 1.0,
                    })
                    .child(
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                crate::ui::chrome::glyph(icons::search(), 12.0, color),
                                space(8.0),
                                label(&text, 12.0, color),
                                gap(),
                                kbd(&chord, color, chrome.chrome_3, chrome.line),
                            ]),
                    )
                    .into()
            },
            move || studio.run(Command::CommandPalette),
        )
        .into()
    }
}

/// Where a menu's panel hangs from, measured across the strip.
///
/// Derived from the titles rather than measured, because the panel is
/// `Positioned` inside a `Stack` and a `Positioned` cannot ask a sibling how
/// wide it turned out. Approximated by a character width, which is close enough
/// that the panel lines up under its own title.
fn menu_offset(menu: Menu) -> f32 {
    /// Left edge of the first menu title: the mark, the gaps, the name.
    ///
    /// Was `45.0` where `MARK` is, for the three dots and the two gaps between
    /// them; a mark that is one square is narrower, and a menu panel that did
    /// not follow it would open an inch to the right of the word it belongs to.
    const START: f32 = 8.0 + MARK + 10.0 + 86.0 + 8.0;
    const PADDING: f32 = 16.0 + 2.0;
    const PER_CHARACTER: f32 = 6.6;

    let mut at = START;
    for other in Menu::BAR {
        if other == menu {
            break;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "five menu titles, none longer than six characters"
        )]
        {
            at += other.title().len() as f32 * PER_CHARACTER + PADDING;
        }
    }
    at
}

/// The application's own mark, at the left end of the strip.
///
/// # What was here, and why it went
///
/// Three coloured dots — a drawn imitation of macOS's window buttons, and
/// decoration only: `winit` owns the real close, minimise and zoom, and this
/// bar is drawn inside the client area rather than replacing the system one. So
/// they were a picture of controls that were not there, on every platform,
/// including the two where the real buttons are on the *other* end of the
/// window. Somebody was always going to click the red one.
///
/// The mark says the same thing a title bar's left end is for — which
/// application this window belongs to — and says something true. Same shape as
/// the icon in the dock and the splash on launch, from [`crate::ui::brand`], so
/// all three are one definition.
fn wordmark_glyph() -> WidgetNode {
    crate::ui::brand::mark(MARK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_element::Runtime;

    fn studio() -> (Runtime, Studio) {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime).with_host(vieww_foundation::TargetPlatform::Linux);
        (runtime, studio)
    }

    #[test]
    fn a_menu_is_shut_until_it_is_clicked() {
        let (_runtime, studio) = studio();
        let dump = vieww_widget::debug_tree(TitleBar {
            studio: studio.clone(),
        });
        assert!(dump.contains("File"), "the title is always there");
        assert!(!dump.contains("Save All"), "the panel is not");

        studio.menu_open.set(Some(Menu::File));
        let open = vieww_widget::debug_tree(MenuLayer { studio });
        assert!(
            open.contains("Save All"),
            "the panel is drawn by MenuLayer, over the window — inside the \
             TitleBar's own box it was clipped away and never appeared"
        );
    }

    #[test]
    fn every_menu_title_drops_something() {
        for menu in Menu::BAR {
            assert!(
                Command::ALL.iter().any(|c| c.menu() == menu),
                "{} is a menu with nothing in it — a title that opens an empty \
                 rectangle",
                menu.title()
            );
        }
    }

    #[test]
    fn a_menu_row_shows_the_chord_that_also_runs_it() {
        let (_runtime, studio) = studio();
        studio.menu_open.set(Some(Menu::File));
        let dump = vieww_widget::debug_tree(MenuLayer { studio });
        assert!(
            dump.contains("Ctrl+S"),
            "the row has to teach the shortcut, or nobody learns it"
        );
    }
}
