//! The editor group: tab strip, breadcrumbs, gutter and code.
//!
//! A **fully editable** pane, and this paragraph used to say otherwise: it
//! described M0's "buffer it cannot edit" long after the caret, the scrollable
//! viewport, folding and the diagnostics underlay had all landed, which meant
//! anyone opening this file to judge what the studio could do read a
//! description of a milestone two years behind the code and stopped there.
//!
//! What is here now: a `TextField` over the active buffer with an `on_changed`
//! that goes through `Studio::edit` (so history, folding and the dirty flag
//! stay in step), a caret with its own blink ticker, per-line gutter numbers
//! sized from the user's font-size setting, fold markers, inline diagnostics,
//! a find/replace bar, a breadcrumb trail down to the symbol under the caret,
//! and a minimap of the file's shape.

use vieww_foundation::{Alignment, Color, Constraints, EdgeInsets, FontFamily, Size, TextStyle};
use vieww_widget::prelude::*;
use vieww_widget::{
    widget_node_from, Animated, AnimatedContainer, Constrained, Container, Flexible,
    GestureDetector, Positioned, SizedBox, Stack, StackFit,
};

use std::rc::Rc;

use crate::state::{Severity, Studio};
use crate::theme::StudioTheme;
use crate::ui::chrome::{clickable, hairline, icon_button, label, mono, space};
use crate::ui::icons;

const TAB_STRIP: f32 = 35.0;

/// The widest a tab's *label* is drawn.
///
/// The bound the ellipsis is measured against, and the reason a tab strip stays
/// a strip: without one, a file called
/// `generated_bindings_for_the_platform_layer.rs` is a tab three inches wide.
/// 150 fits about twenty-two characters of the UI font, which is the length
/// the old character-count rule was aiming at — the difference is that this one
/// is a promise about pixels rather than about glyphs, so a name in wide
/// capitals and a name in narrow lowercase both land inside it.
pub(crate) const TAB_LABEL_MAX: f32 = 150.0;

/// Roughly how wide one tab is drawn.
///
/// # An estimate, and why one is enough
///
/// The only caller is [`Studio::reveal_tab`], which turns it into a scroll
/// offset that brings the active tab into view. Being a few pixels out moves
/// that tab a few pixels from where it would ideally sit; being *absent* — which
/// is what the strip did before it scrolled at all — means the active tab can
/// be off screen entirely. A measurement would need the text shaper, which the
/// widget layer does not have and a scroll decision cannot wait for.
///
/// **Bounded by the same number the label is.** The estimate used to be
/// unbounded above and the label unbounded with it; now the label cannot exceed
/// [`TAB_LABEL_MAX`], so neither can this, and the error on a long name went
/// from "however wrong the per-character mean is over forty characters" to
/// "nothing at all, because both sides are the cap".
///
/// The constant is the chrome either side of the label: 12 of padding twice, a
/// 13px file icon, two 7px gaps and the 17px close button.
pub(crate) fn tab_extent(name: &str) -> f32 {
    // The disambiguating folder name a tab may carry is not counted here: it is
    // only present when two open files share a name, and `reveal` is allowed to
    // be a few pixels out — see the note above.
    const CHROME: f32 = 12.0 * 2.0 + 13.0 + 7.0 * 2.0 + 17.0;
    /// The mean advance of a 12.5px UI glyph. Digits and capitals run wider,
    /// `i` and `l` narrower; a file name is mostly lowercase.
    const PER_CHARACTER: f32 = 6.9;
    #[expect(
        clippy::cast_precision_loss,
        reason = "a file name long enough to lose precision here is past the cap anyway"
    )]
    let characters = name.chars().count() as f32;
    CHROME + (characters * PER_CHARACTER).min(TAB_LABEL_MAX)
}
const BREADCRUMB: f32 = 26.0;
/// A source line's row height as a multiple of the code font size.
///
/// The settings panel changes the font *size*; the ratio between a glyph and
/// the row that holds it does not change with it, so this stays a constant
/// and the row height is derived. 20/13 is the pair M0 shipped with, which is
/// why every size still looks like the studio rather than like a new editor.
pub(crate) const CODE_LINE_RATIO: f32 = 20.0 / 13.0;

/// One source line's height in the code pane, for a given code font size.
///
/// `pub(crate)` because `Studio::reveal_line` needs it to turn a line number
/// into a scroll offset, and there is exactly one right answer: the pane does
/// not wrap (see `wrap(false)` below), so a line's top is its index times
/// this. A second copy of the arithmetic in `state.rs` would be a second
/// thing to keep in step with the gutter.
#[must_use]
pub(crate) fn code_line(size: f32) -> f32 {
    size * CODE_LINE_RATIO
}

#[derive(Debug)]
pub struct EditorGroup {
    pub studio: Studio,
}

impl Widget for EditorGroup {
    fn debug_name(&self) -> &'static str {
        "EditorGroup"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);

        Container::new()
            .color(chrome.chrome_2)
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children![
                        self.tabs(ctx, &chrome, theme.colors),
                        hairline(chrome.line),
                        WidgetNode::from(Breadcrumbs {
                            studio: self.studio.clone(),
                        }),
                        // Between the breadcrumbs and the code rather than
                        // floating over it: the match a find just landed on is
                        // usually on the line a floating bar would cover.
                        crate::ui::find_bar::FindBar {
                            studio: self.studio.clone()
                        },
                        hairline(chrome.line),
                        // Clipped *and* scrollable: the clip is what keeps a
                        // fling from painting over the panel, and the scroll is
                        // what makes the rest of the file reachable. M0 had
                        // only the first and printed an overflow every frame.
                        Flexible::expanded(1).child(Clip::rect().child(WidgetNode::from(
                            CodePane {
                                studio: self.studio.clone(),
                            },
                        ))),
                    ]),
            )
            .into()
    }
}

widget_node_from!(EditorGroup);

impl EditorGroup {
    fn tabs(&self, ctx: &BuildContext, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        // `tabs`, not `buffers`: a tab is a name, a folder and a dot, and
        // none of them changes when a character is typed. See `BufferTab`.
        let buffers = self.studio.tabs.get();
        let active = self.studio.active_buffer.get();

        // **Two tabs called `mod.rs` are two tabs nobody can tell apart.**
        //
        // The strip labelled every tab with its file name alone, which is fine
        // for the three bundled screens and useless in a real Rust project:
        // opening `src/library.rs` and `src/screens/library.rs` — both of which
        // a person browsing one codebase will do — produced two identical tabs,
        // and `mod.rs` produces one per module. The disambiguator is only added
        // where it is needed, so the common case stays a bare file name.
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for buffer in buffers.iter() {
            *counts.entry(buffer.name.as_str()).or_default() += 1;
        }

        // Read out where a `BuildContext` still exists: the builders below
        // outlive this build, so capturing `ctx` is a borrow that escapes.
        let (quick_duration, quick_curve) = crate::ui::chrome::quick(ctx);
        let (slow_duration, slow_curve) = crate::ui::chrome::considered(ctx);

        let tabs = buffers
            .iter()
            .enumerate()
            .map(|(index, buffer)| {
                let selected = index == active;
                let signal = self.studio.clone();
                let studio = self.studio.clone();
                let full_name = buffer.name.clone();
                // The containing folder, and only when the bare name is
                // ambiguous among what is open.
                let qualifier = (counts.get(buffer.name.as_str()).copied().unwrap_or(0) > 1)
                    .then(|| {
                        buffer
                            .path
                            .as_ref()
                            .and_then(|path| path.parent())
                            .and_then(std::path::Path::file_name)
                            .map(|folder| folder.to_string_lossy().into_owned())
                    })
                    .flatten();
                let name = buffer.name.clone();
                let unsaved = buffer.dirty;
                let chrome = *chrome;

                // The *unelided* name, so a screen reader reads the file rather
                // than the abbreviation the strip had room for.
                vieww_widget::Semantics::button(full_name)
                    .child(crate::ui::chrome::sensed(
                        move |sense| {
                            // The selected tab keeps its own colour under the
                            // pointer. Hovering the thing that is already chosen and
                            // watching it change is a control answering a question
                            // nobody asked; the unselected ones are the ones a
                            // pointer is asking about.
                            let background = if selected {
                                chrome.chrome_2
                            } else {
                                crate::ui::chrome::hovered(chrome.chrome_1, &chrome, sense)
                            };
                            // **The label brightens, and that is the main signal.**
                            //
                            // The fill cannot carry hover on its own here: the
                            // selected tab is `chrome_2` and the strip is
                            // `chrome_1`, five points of luminance apart, so any
                            // hover wash strong enough to see would land on top of
                            // the selected colour and make the two states
                            // indistinguishable. Text has the whole range from
                            // `on_surface_variant` to `on_surface` free, and it is
                            // the part of a tab people are already looking at.
                            let foreground = if selected || sense.emphasis() > 0.0 {
                                colors.on_surface
                            } else {
                                colors.on_surface_variant
                            };

                            let mut row: Vec<WidgetNode> = vec![
                                crate::ui::chrome::glyph_icon(icons::file())
                                    .size(13.0)
                                    .color(colors.on_surface_variant)
                                    .into(),
                                space(7.0),
                                // **Cut to a width, by the shaper, in the render
                                // layer.** The strip used to keep twenty-four
                                // characters and hope: `WWWWWWWW` at that count is
                                // half again as wide as `illillillil`, so the same
                                // rule overflowed the slot for one file and left a
                                // third of it empty for the next. `Constrained` is
                                // what makes the cut possible at all — with no
                                // bounded width nothing is ever too wide — and
                                // `EllipsisMiddle` is what keeps `.rs` on the end
                                // and the distinguishing head at the front.
                                Constrained::new(Constraints::loose(Size::new(
                                    TAB_LABEL_MAX,
                                    f32::INFINITY,
                                )))
                                .child(
                                    label(&name, 12.5, foreground)
                                        .max_lines(1)
                                        .overflow(vieww_text::TextOverflow::EllipsisMiddle),
                                )
                                .into(),
                            ];
                            if let Some(qualifier) = qualifier.as_ref() {
                                // Dimmer and smaller than the name: it is there to
                                // separate two tabs, not to be read on its own.
                                row.push(space(5.0));
                                row.push(label(qualifier, 10.5, colors.outline).into());
                            }
                            row.push(space(7.0));
                            // An unsaved dot *or* a close button, never both: they
                            // occupy the same slot in every editor anybody has used,
                            // and a tab that shows both is a tab that shifts when it
                            // is saved.
                            if unsaved {
                                row.push(
                                    Container::new()
                                        .color(foreground)
                                        .radius(3.5)
                                        .size(7.0, 7.0)
                                        .into(),
                                );
                            } else {
                                // A close button that closes. It is its own
                                // clickable rather than a picture inside the tab's,
                                // because a click on it must not also select the
                                // tab it is about to remove.
                                let closing = studio.clone();
                                let index = index;
                                row.push(
                                    crate::ui::chrome::sensed(
                                        move |sense| {
                                            // The one control in the strip that
                                            // *must* answer the pointer separately
                                            // from its tab: it destroys something,
                                            // and "am I about to close this file or
                                            // switch to it" is a question the user
                                            // is entitled to have answered before
                                            // the click, not after.
                                            let tint = if sense.emphasis() > 0.0 {
                                                colors.on_surface
                                            } else {
                                                colors.on_surface_variant
                                            };
                                            Container::new()
                                                .color(crate::ui::chrome::hover_overlay(
                                                    colors, sense,
                                                ))
                                                .radius(3.0)
                                                .size(17.0, 17.0)
                                                .alignment(Alignment::CENTER)
                                                .child(crate::ui::chrome::glyph(
                                                    icons::close(),
                                                    11.0,
                                                    tint,
                                                ))
                                                .into()
                                        },
                                        move || {
                                            closing.focus_buffer(index);
                                            closing.run(crate::command::Command::CloseTab);
                                        },
                                    )
                                    .into(),
                                );
                            }

                            // The selected tab is marked along its top edge, not by
                            // colour alone: the two chrome depths differ by six
                            // points of luminance, which is enough to see and not
                            // enough to rely on.
                            //
                            // # Why 3.0 and not 1.5
                            //
                            // 1.5 was the original height and it was right that a
                            // hairline read as "subtle". It was wrong about whether
                            // "subtle" was the right register for the one signal a
                            // tab strip exists to carry — at 1.5 the marker
                            // disappeared against the tab background at small UI
                            // scales and the only remaining signal was the chrome
                            // depth, which is the part nobody trusts. 3.0 is the
                            // smallest height at which the marker reads as a bar
                            // rather than a line, and is what VS Code and IntelliJ
                            // both ship.
                            // **The marker grows out of the strip.**
                            //
                            // It used to be a `Container` whose colour flipped
                            // between the accent and the tab's own background,
                            // which is a switch: at any instant one tab has a
                            // bar and the others do not, and switching tabs is
                            // two instantaneous events with nothing joining
                            // them. `TabIndicator` is the same three points of
                            // height driven by an `Animated` fraction, growing
                            // from the middle out and carrying a bloom, so a
                            // switch reads as one marker travelling.
                            let indicator_accent = chrome.accent;
                            let indicator_soft = chrome.accent_soft;
                            //
                            // Pinned along the tab's top edge rather than
                            // stacked as the first row of a column: a tab in a
                            // horizontal `Scrollable` is laid out with no
                            // bounded width, so a column's `Stretch` has no
                            // width to stretch to and the indicator came out
                            // three points tall and zero wide.
                            let marker = Animated::new(if selected { 1.0 } else { 0.0 })
                                .duration(quick_duration)
                                .curve(quick_curve)
                                .key(format!("tab-marker-{index}"))
                                .build(move |t| {
                                    crate::ui::chrome::TabIndicator {
                                        accent: indicator_accent,
                                        accent_soft: indicator_soft,
                                        t,
                                    }
                                    .widget()
                                });

                            // **Selection is a depth, not only a tint.**
                            //
                            // Selected and unselected were `chrome_2` against
                            // `chrome_1` — six points of luminance — plus the
                            // marker above, which sits at the top edge of the
                            // shape it describes and so only helps an eye
                            // already looking at the strip. Raising the
                            // selected tab off the strip is the signal every
                            // editor people arrive from already teaches, and it
                            // reads from anywhere in the window.
                            //
                            // `AnimatedContainer` rather than `Container`,
                            // because a shadow that appears is a different tab
                            // and a shadow that grows is the same tab rising.
                            // The colour and the shadow interpolate together.
                            // An unselected tab's shadow is a real shadow with
                            // nothing in it — same struct, zero alpha, zero
                            // offset, zero blur — because a property that is
                            // absent on one side snaps rather than animating,
                            // and a shadow that snaps on is exactly the thing
                            // this replaced.
                            let resting = vieww_foundation::Shadow::new(
                                Color::rgba(0, 0, 0, 0),
                                vieww_foundation::Offset::ZERO,
                                0.0,
                            );
                            let tab = AnimatedContainer::new()
                                .duration(slow_duration)
                                .curve(slow_curve)
                                .key(format!("tab-body-{index}"))
                                .color(background)
                                .shadow(if selected {
                                    chrome.elevation(2)
                                } else {
                                    resting
                                })
                                .height(TAB_STRIP);
                            tab.child(Stack::new().children(children![
                                    Container::new()
                                        .padding(EdgeInsets::symmetric(12.0, 0.0))
                                        .alignment(Alignment::CENTER_LEFT)
                                        .child(
                                            Flex::row()
                                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                                .children(row)
                                        ),
                                    Positioned::new()
                                        .left(0.0)
                                        .right(0.0)
                                        .top(0.0)
                                        .height(3.0)
                                        .child(marker),
                                ]))
                            .into()
                        },
                        move || signal.focus_buffer(index),
                    ))
                    .into()
            })
            .collect::<Vec<WidgetNode>>();

        // **The tabs scroll; the toolbar does not.**
        //
        // Two regions rather than one row, because they fail differently when
        // there are too many files: the tabs are a list that should run off the
        // edge and be reachable by scrolling, and Save and New File are fixed
        // controls that must never leave. In one row the toolbar was simply the
        // far end of the list, so it was the first thing to go.
        //
        // `Clip` around the scrollable is what stops a tab painting over the
        // toolbar mid-scroll; `Flexible` is what gives the strip whatever width
        // the toolbar does not need.
        let scroll = self.studio.tabs_scroll.clone();
        let strip = Scrollable::horizontal(scroll.offset())
            .key("editor-tabs")
            .on_drag(scroll.on_drag())
            .on_drag_end(scroll.on_drag_end())
            // **The reveal runs from the layout callback, not from the click.**
            //
            // `ScrollController::reveal` needs the viewport length to decide
            // where to scroll to, and the only thing that knows it is layout —
            // which has not run when a tab is switched by `Ctrl+]`, by opening
            // a file, or on the very first frame after a session is restored.
            // Called there, the first reveal of a session silently did nothing
            // against a viewport of zero. Called here it is answered by the
            // frame that measured the strip, and it is idempotent: once the
            // active tab is inside the window, this changes no offset and books
            // no further frame.
            .on_extents({
                let controller = scroll.clone();
                let studio = self.studio.clone();
                std::rc::Rc::new(move |extents| {
                    controller.resize(extents);
                    studio.reveal_active_tab();
                })
            })
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(tabs),
            );

        Container::new()
            .color(chrome.chrome_1)
            .height(TAB_STRIP)
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children![
                        Flexible::expanded(1).child(Clip::rect().child(strip)),
                        save_button(&self.studio, colors),
                        // "New File", not a split-editor button that did
                        // nothing. A control that looks live and is not is worse
                        // than no control: it teaches people the app is broken.
                        icon_button(icons::plus(), 14.0, colors.on_surface_variant, {
                            let studio = self.studio.clone();
                            move || studio.run(crate::command::Command::NewFile)
                        }),
                    ]),
            )
            .into()
    }
}

/// What the editor shows when nothing is open.
///
/// The two ways out, as buttons rather than as a sentence naming menu items:
/// this is the one screen in the studio where the user has *nothing* in front
/// of them, and a paragraph telling them where to look is worse than a control
/// they can press.
fn empty_editor(studio: &Studio, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
    let action =
        |title: &'static str, command: crate::command::Command, studio: Studio| -> WidgetNode {
            let chrome = *chrome;
            crate::ui::chrome::sensed(
                move |sense| {
                    Container::new()
                        .color(crate::ui::chrome::hovered(chrome.chrome_1, &chrome, sense))
                        .radius(6.0)
                        .height(30.0)
                        .padding(EdgeInsets::symmetric(14.0, 0.0))
                        .alignment(Alignment::CENTER)
                        .child(label(title, 12.0, colors.primary))
                        .into()
                },
                move || studio.run(command),
            )
            .into()
        };

    Container::new()
        .color(chrome.chrome_2)
        .alignment(Alignment::CENTER)
        .child(
            Flex::column()
                .main_axis_size(MainAxisSize::Min)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(10.0)
                .children(children![
                    label("No file open", 14.0, colors.on_surface),
                    label(
                        "Open one from the Explorer, or start a new one.",
                        11.5,
                        colors.on_surface_variant
                    ),
                    space(4.0),
                    Flex::row()
                        .main_axis_size(MainAxisSize::Min)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            action("New file", crate::command::Command::NewFile, studio.clone()),
                            space(8.0),
                            action(
                                "Open folder\u{2026}",
                                crate::command::Command::OpenFolder,
                                studio.clone()
                            ),
                        ]),
                ]),
        )
        .into()
}

/// Writes the active buffer to disk. Only shown when there is something to
/// write, and tinted when there is — a save button that is always available is
/// a save button that never tells you anything.
fn save_button(studio: &Studio, colors: ColorScheme) -> WidgetNode {
    let dirty = studio.active_tab().is_some_and(|b| b.dirty);
    if !dirty {
        return SizedBox::shrink().into();
    }

    let studio = studio.clone();
    // Through the command, so the button, ⌘S and File ▸ Save are one thing.
    // The outcome — including a failure — lands in the Output panel rather
    // than on stderr, which is where it used to go: a save that failed into a
    // terminal nobody is watching is a save that failed silently, and for the
    // one operation whose entire job is not losing work that is the worst
    // possible place to be quiet.
    icon_button(icons::save(), 14.0, colors.primary, move || {
        studio.run(crate::command::Command::Save);
    })
}

/// The trail above the editor, as a widget rather than a function.
///
/// # Why this is its own element
///
/// It reads [`Studio::symbol_trail`], and that reads the **caret**. As a plain
/// function called from `EditorGroup::build`, the subscription belonged to the
/// `EditorGroup` element — so every caret move, which is every keystroke and
/// every arrow key, rebuilt the entire editor: the tab strip, the gutter, the
/// minimap, the scrollbars, the find bar. Measured on the studio's own tree,
/// one caret write rebuilt **413 elements**, and the strip that needed them was
/// a row of four labels.
///
/// A `Composed` widget is an element of its own, so the subscription lands here
/// and a caret move rebuilds this trail and nothing above it. That is the whole
/// change: the same code, moved down one element, so the signal it reads
/// invalidates the region that reads it rather than the region that contains
/// that region.
#[derive(Debug)]
pub struct Breadcrumbs {
    pub studio: Studio,
}

widget_node_from!(Breadcrumbs);

impl Widget for Breadcrumbs {
    fn debug_name(&self) -> &'static str {
        "Breadcrumbs"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        breadcrumbs(&self.studio, &chrome, theme.colors)
    }
}

fn breadcrumbs(studio: &Studio, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
    // **The whole trail now, not just the file.** This stopped at
    // `[workspace] / [file]` behind a comment saying the rest "needs a parse of
    // the buffer, which is M6's tree-sitter and not this milestone's". M6
    // landed: `Studio::symbol_trail` walks the same parse the highlighter
    // already holds, so the type and member under the caret cost nothing extra
    // to show — and a breadcrumb bar that cannot say which function you are in
    // is a strip of chrome rather than a navigation aid.
    let Some(buffer) = studio.active() else {
        // Nothing is open, so there is no trail. The strip stays — its height
        // is part of the pane's geometry and a bar that vanishes makes the
        // editor jump — but it says nothing rather than "screens / no buffer",
        // which reads as a file called "no buffer".
        return Container::new()
            .color(chrome.chrome_2)
            .height(BREADCRUMB)
            .into();
    };
    let buffer = Some(buffer);
    let root = studio.root.get();
    let name = buffer.as_ref().map_or("no buffer", |b| b.name.as_str());
    let place = root.as_ref().map_or_else(
        || "scratch".to_string(),
        |path| {
            path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into(),
            )
        },
    );
    let mut crumbs: Vec<String> = vec![place, name.to_owned()];
    // Only for a file the parser understands. A `Cargo.toml` has definitions
    // too and this parser cannot see them, and a trail that is silently absent
    // on some files reads better than one that is confidently wrong.
    if buffer.as_ref().is_some_and(|b| b.language.highlighted()) {
        crumbs.extend(
            studio
                .symbol_trail()
                .into_iter()
                // `impl CardGrid` and `fn build` — the kind is what tells a
                // method apart from a field with the same name.
                .map(|symbol| format!("{} {}", symbol.kind, symbol.name)),
        );
    }
    let mut row: Vec<WidgetNode> = Vec::new();
    for (index, crumb) in crumbs.iter().enumerate() {
        if index > 0 {
            row.push(space(5.0));
            row.push(label("/", 11.5, colors.outline).into());
            row.push(space(5.0));
        }
        // The last crumb is where the caret is, so it carries the emphasis.
        let colour = if index + 1 == crumbs.len() && crumbs.len() > 2 {
            colors.on_surface
        } else {
            colors.on_surface_variant
        };
        row.push(label(crumb, 11.5, colour).into());
    }

    Container::new()
        .color(chrome.chrome_2)
        .height(BREADCRUMB)
        .padding(EdgeInsets::symmetric(14.0, 0.0))
        .alignment(Alignment::CENTER_LEFT)
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(row),
        )
        .into()
}

/// Gutter and text, scrolling as one.
///
/// # Why they are inside the same scrollable, not two synchronised ones
///
/// A gutter that scrolls itself against a signal the text also writes is two
/// sources for one number, and they drift by exactly one frame at exactly the
/// moment somebody flings the view. Putting both inside one `Scrollable`'s
/// child makes the alignment structural: there is one offset, applied to one
/// subtree, and line 200's number is beside line 200 because they are in the
/// same row.
/// The gutter, the text and the minimap, as an element of their own.
///
/// # Why this is not just a function on `EditorGroup`
///
/// The same argument as [`Breadcrumbs`], one level up in cost. This reads the
/// buffer's **text** — it has to; it is the thing that draws it — and as a
/// plain call inside `EditorGroup::build` that subscription belonged to the
/// group. So a keystroke rebuilt the tab strip, the breadcrumb trail and the
/// find bar as well, none of which is a picture of the text.
///
/// Scoped here, a keystroke rebuilds the pane that shows the file and leaves
/// its siblings alone. The tab strip already reads
/// [`Studio::tabs`](crate::Studio), so it now has nothing left that a keystroke
/// touches.
#[derive(Debug)]
pub struct CodePane {
    pub studio: Studio,
}

widget_node_from!(CodePane);

impl Widget for CodePane {
    fn debug_name(&self) -> &'static str {
        "CodePane"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        code_pane(&self.studio, &chrome, theme.colors)
    }
}

fn code_pane(studio: &Studio, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
    let Some(buffer) = studio.active() else {
        // **An empty editor is a state, not an accident.** It was a bare
        // rectangle the colour of the pane, because until the last tab could be
        // closed the case was unreachable — and a blank pane with nothing in it
        // and nothing to press is indistinguishable from a studio that has
        // failed to draw.
        return empty_editor(studio, chrome, colors);
    };

    // What the pane shows, which is the whole buffer unless something is
    // folded. The gutter, the field and the decorations are all built from
    // this one projection, so they cannot disagree about which line is which.
    let view = studio.folded_view();
    let folded = !view.is_complete(&buffer.value.text);
    // `foldable` and `folds` used to be read here for the gutter. They are the
    // `Gutter` widget's business now, and reading them here would put its
    // subscriptions back on this element.
    // Read, not a constant: the Settings panel's "Font size" picker writes
    // this signal, so reading it here is both what makes the setting visible
    // and what makes the pane rebuild when it changes.
    let code_size = studio.font_size.get();
    let line_box = code_line(code_size);
    let style = TextStyle {
        color: colors.on_surface,
        size: code_size,
        family: FontFamily::Monospace,
        line_height: CODE_LINE_RATIO,
        ..TextStyle::new(code_size)
    };

    // The gutter is a widget of its own reading one published signal — see
    // `state::GutterRow`. It used to be built here from the buffer's text, the
    // fold set, the foldable regions and the caret, which made it 175 elements
    // rebuilt on every character typed inside a line to redraw the numbers it
    // already had.
    let gutter = WidgetNode::from(Gutter {
        studio: studio.clone(),
        line_box,
        code_size,
    });

    // Highlighting and range decorations are computed over the source's byte
    // offsets, so they are only correct while the field is holding the source.
    // With a fold on, the field's text is a different string and every offset
    // in them points at the wrong place — so they are dropped rather than
    // drawn somewhere plausible and wrong. Projecting them is the next piece
    // of this feature; drawing a squiggle under the wrong token is not a step
    // towards it.
    // **Projected, not dropped.** This used to be `Vec::new()` while anything
    // was folded, with a comment saying why: spans are computed over the
    // source's byte offsets and the field is holding a different string. That
    // was the right refusal and the wrong resting place — a folded file with no
    // colour was the most visible rough edge in this pane.
    // `Studio::projected_spans` re-cuts the runs through `folding::line_map`,
    // which already knew which source line each visible line came from.
    let highlighted = studio.projected_spans(style, *chrome);
    // Diagnostics are keyed to *rows*, so these do project: a source line's
    // row is its position in `view.lines`.
    let diagnostic_lines = Rc::new(
        studio
            .active_diagnostics()
            .iter()
            .filter_map(|d| {
                let source_line = d.line.saturating_sub(1) as usize;
                view.lines.iter().position(|&line| line == source_line)
            })
            .collect::<Vec<_>>(),
    );
    let studio_for_edit = studio.clone();
    // The field holds the *projection*. Its value is the source's caret and
    // selection put back into view coordinates by the same map the gutter
    // uses; what it reports comes back through `edit_projected`, which turns
    // it into an edit of the real buffer. See `crate::folding`.
    let field_value = if folded {
        let mut value = vieww_foundation::TextEditingValue::new(view.text.clone());
        value.selection = vieww_foundation::TextSelection::collapsed(crate::folding::view_offset(
            &buffer.value.text,
            &view,
            buffer.value.selection.cursor().offset,
        ));
        value
    } else {
        buffer.value.clone()
    };
    let wrapping = studio.word_wrap.get();
    let field = TextField::new(field_value)
        .style(style)
        // Both halves of `show_cursor`: is this the field taking the keyboard,
        // and is the blink in its on phase. Without it four fields each paint a
        // motionless caret and none of them looks live — see `crate::caret`.
        .show_cursor(studio.caret_visible(crate::caret::Active::Editor))
        .spans(highlighted)
        .cursor(colors.primary, 2.0)
        .selection_color(chrome.selection)
        .diagnostics(diagnostic_lines, colors.error.with_alpha(24))
        // Squiggles under the offending *text*, the matched bracket, and every
        // other occurrence of the word at the caret. All three are positioned
        // from the shaped paragraph, which is why they need the framework's
        // `TextDecoration` seam and could not be drawn from here before it.
        // Same projection, with the one asymmetry `folding::project_range`
        // argues for: a mark with either end inside a fold is dropped rather
        // than moved to the header line, because a squiggle under the wrong
        // token is not a step towards one under the right token.
        .decorations(Rc::new(studio.projected_decorations(*chrome)))
        // The gutter beside this field is one fixed-height row per *source*
        // line (built straight from `buffer.line_count()`, just below), and
        // `diagnostic_lines` above is keyed the same way — by rustc's line
        // number, not by a visual row. Left to wrap, a long line quietly
        // shifts every row under it out of step with the gutter and points
        // every diagnostic highlight at the wrong line — which is what the
        // grey block in the gutter used to be. M0 already calls for the code
        // pane to clip and overflow rather than reflow (see M0.md); this is
        // what actually gets that.
        .wrap(wrapping)
        // What the field measured, so the hints below can be put at the end of
        // the lines they belong to. The read is one frame behind by
        // construction — see `TextLayoutProbe` — and the studio bumps
        // `layout_generation` when it changes, which is what gets the frame
        // that draws them in the right place.
        .probe(studio.layout_probe.clone())
        .on_changed(Rc::new(move |value| studio_for_edit.edit_projected(value)))
        // **A pointer reports a selection; it does not report text.**
        //
        // Without this the field assembles a whole `TextEditingValue` around
        // the reported selection out of the *widget's* text — this frame's
        // build, which during a drag is behind the buffer, and which several
        // drag updates in one frame all read. The studio then commits that
        // string, and a selection gesture has edited the document. That is the
        // 247 characters the screencast loses to one click-and-drag.
        //
        // `Studio::select` takes a selection and nothing else, so the pointer
        // path has no text to be wrong about.
        .on_selection({
            let studio = studio.clone();
            Rc::new(move |selection| studio.select_projected(selection))
        });

    // **The field scrolls sideways; the gutter and the minimap do not.**
    //
    // Which is why this is nested here rather than wrapped around the row: a
    // gutter that slid out from under its line numbers would be worse than no
    // horizontal scrolling at all, and the minimap is a picture of the whole
    // file whose whole point is that it does not move with the text.
    //
    // Inside a horizontal `Scrollable` the field's width is unbounded, which is
    // exactly what `wrap(false)` wants: one visual line per source line, as
    // long as it needs to be, and the window slides over it.
    //
    // **With wrapping on, the horizontal scrollable is not there at all.**
    // Not present-but-disabled: inside it the field's width is unbounded,
    // which is what makes `wrap(true)` a no-op — there is no edge to wrap at.
    // Long string literals and doc comments are the reason anybody turns this
    // on, and until now the only answer to them was constant sideways travel.
    let scrolling_field: WidgetNode = if wrapping {
        with_inline_diagnostics(studio, field, chrome, colors)
    } else {
        let scroll_x = studio.editor_scroll_x.clone();
        Scrollable::horizontal(scroll_x.offset())
            .key("editor-columns")
            .on_drag(scroll_x.on_drag())
            .on_drag_end(scroll_x.on_drag_end())
            .on_extents(scroll_x.on_extents())
            .child(with_inline_diagnostics(studio, field, chrome, colors))
            .into()
    };

    let content = Container::new()
        .color(chrome.chrome_2)
        .padding(EdgeInsets::only(0.0, 8.0, 0.0, 8.0))
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .children(children![
                    Container::new()
                        .width(52.0)
                        .padding(EdgeInsets::only(0.0, 0.0, 10.0, 0.0))
                        .child(gutter),
                    Flexible::expanded(1).child(
                        Container::new()
                            .padding(EdgeInsets::only(4.0, 0.0, 16.0, 0.0))
                            .child(scrolling_field)
                    ),
                ]),
        );

    let scroll = studio.editor_scroll.clone();
    let scrolling: WidgetNode = Scrollable::vertical(scroll.offset())
        .on_drag(scroll.on_drag())
        .on_drag_end(scroll.on_drag_end())
        .on_extents(scroll.on_extents())
        .child(content)
        .into();

    // The two bars, over the pane rather than beside it.
    //
    // Beside would mean a column of chrome taken out of the code and a row out
    // of the last line, on the surface where every point is a character. Over
    // costs nothing when there is nothing to scroll — `Scrollbar` draws an empty
    // box then — and is what every editor does.
    let bars: Vec<WidgetNode> = vec![
        Positioned::new()
            .right(2.0)
            .top(2.0)
            .bottom(crate::ui::scrollbar::WIDTH + 2.0)
            .child(crate::ui::scrollbar::Scrollbar {
                scroll: studio.editor_scroll.clone(),
                axis: vieww_foundation::Axis::Vertical,
                chrome: *chrome,
                color: chrome.chrome_3,
            })
            .into(),
        // Only where there is sideways travel to make: with wrapping on the
        // horizontal controller is not driving anything, and a bar under a
        // wrapped file would be a control for a scroll that cannot happen.
        if wrapping {
            SizedBox::shrink().into()
        } else {
            Positioned::new()
                .left(52.0)
                .right(crate::ui::scrollbar::WIDTH + 2.0)
                .bottom(2.0)
                .child(crate::ui::scrollbar::Scrollbar {
                    scroll: studio.editor_scroll_x.clone(),
                    axis: vieww_foundation::Axis::Horizontal,
                    chrome: *chrome,
                    color: chrome.chrome_3,
                })
                .into()
        },
    ];

    // **The minimap is beside the scrollable, not inside it.**
    //
    // It used to be the last child of the row *within* the vertical
    // `Scrollable`, which meant it scrolled with the text — one long strip of
    // bars that slid past like a second copy of the file. A minimap's whole
    // purpose is to be the part that does not move: the file at a glance, with
    // the window you are looking through drawn on it. Inside the scrollable it
    // could not show that window at all, because it *was* the window.
    Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(children![
            Flexible::expanded(1).child(
                Stack::new()
                    .fit(StackFit::Expand)
                    .alignment(Alignment::TOP_LEFT)
                    .children({
                        let mut layers: Vec<WidgetNode> = vec![scrolling];
                        layers.extend(bars);
                        layers
                    })
            ),
            Minimap {
                studio: studio.clone(),
                rows: Rc::new(minimap_rows(
                    &studio.highlighted_spans(
                        TextStyle {
                            color: Color::TRANSPARENT,
                            ..TextStyle::new(studio.font_size.get())
                        },
                        *chrome,
                    ),
                    &buffer.value.text
                )),
                // The same list the gutter marks, from the same source.
                flagged: Rc::new(
                    studio
                        .active_diagnostics()
                        .iter()
                        .map(|d| d.line)
                        .collect::<Vec<u32>>(),
                ),
                chrome: *chrome,
                colors,
            },
        ])
        .into()
}

/// The field, with each diagnostic's message drawn after the end of its line.
///
/// # Why this is a `Stack` and not another decoration
///
/// [`TextDecoration`](vieww_foundation::TextDecoration) can mark a range of
/// the field's own text; it cannot add text that is not in the buffer, and a
/// hint after the end of a line is exactly that. So it goes in the
/// application's tree, above the field — which means it needs a position, and
/// the position is `line_rect(n).right`, which only exists after shaping.
/// [`TextLayoutProbe`](vieww_foundation::TextLayoutProbe) is how the field
/// hands it back.
///
/// # It reads the previous frame's geometry, on purpose
///
/// The report was published during the *last* layout. Typing at the end of a
/// flagged line therefore moves its hint one frame after the character
/// appears. The alternative — laying the hint out from the field's own layout
/// — would mean the field's size depending on a child whose position depends
/// on the field's size, and that does not converge. One frame is the price,
/// and `Studio::layout_generation` is what makes sure it is only one.
///
/// # Nothing is drawn until there is something to draw it against
///
/// An empty report means no layout has happened yet. Placing hints at the
/// origin in that frame would stack every message in the top-left corner and
/// then have them jump — worse than the frame of absence this returns.
fn with_inline_diagnostics(
    studio: &Studio,
    field: TextField,
    chrome: &StudioTheme,
    colors: ColorScheme,
) -> WidgetNode {
    if !studio.inline_diagnostics.get() {
        return field.into();
    }
    // Read so the pane rebuilds when the measurement changes. The report
    // itself is deliberately *not* a signal; this is.
    let _generation = studio.layout_generation.get();

    let report = studio.layout_probe.snapshot();
    if report.is_empty() {
        return field.into();
    }

    // **Keep the caret's column on screen — when the caret is what moved.**
    //
    // Horizontal scrolling without this is scrolling you have to do by hand:
    // typing past the right edge would put the caret somewhere off it and leave
    // the window where it was. `reveal` does nothing when the range is already
    // visible — which is what makes it safe on every rebuild — and moves the
    // least distance that brings it back when it is not. The margin is about
    // three characters, so the caret never sits flush against the edge it just
    // came through.
    //
    // **But "safe on every rebuild" was read too broadly, and it ate the whole
    // feature.** A scroll of this pane is itself a rebuild — the pane
    // subscribes to the scroll offset to draw against it. So the frame after
    // the user dragged the window sideways, the reveal saw the caret the drag
    // had just pushed off screen and dragged the window straight back to it.
    // Wheel, trackpad and the scrollbar all publish through the same signal,
    // and every path of sideways travel sprang back to where it began.
    // Reported as "the horizontal scroll in the editor is not functional".
    //
    // The reveal therefore runs when the *caret* moved — the measured cursor
    // rectangle changed, which is typing, and navigation, and nothing else —
    // and keeps still when the user is the one moving the window. A caret that
    // has not moved produces an equal `(generation, left, width)` and the gate
    // stays shut; `revealed_caret`'s documentation carries the full story.
    //
    // Reading the report from the *previous* layout is the contract
    // `TextLayoutProbe` states, and it is why this settles after one extra
    // frame rather than oscillating: the frame that scrolls publishes a report
    // whose cursor is already in view, and the next one asks for nothing.
    {
        let report_key = (
            studio.layout_generation.get(),
            report.cursor.left,
            report.cursor.width(),
        );
        if studio.revealed_caret.get() != report_key {
            studio.revealed_caret.set(report_key);
            studio
                .editor_scroll_x
                .reveal(report.cursor.left, report.cursor.width().max(2.0), 24.0);
        }
    }

    let diagnostics = studio.active_diagnostics();
    if diagnostics.is_empty() {
        return field.into();
    }

    /// How far after the end of the line the hint starts.
    const LEAD: f32 = 18.0;
    /// A hint is quieter than the code it annotates.
    const HINT_SIZE: f32 = 11.5;

    let mut children: Vec<WidgetNode> = vec![field.into()];
    // One hint per line, not per diagnostic: two errors on one line would
    // otherwise be drawn on top of each other. The first is shown and the rest
    // are counted, which is what the Problems panel is for.
    let mut done: Vec<u32> = Vec::new();
    for diagnostic in &diagnostics {
        if done.contains(&diagnostic.line) {
            continue;
        }
        done.push(diagnostic.line);
        let Some(rect) = report.line(diagnostic.line.saturating_sub(1) as usize) else {
            continue;
        };
        let more = diagnostics
            .iter()
            .filter(|other| other.line == diagnostic.line)
            .count()
            - 1;
        let color = match diagnostic.severity {
            Severity::Error => colors.error,
            Severity::Warning => chrome.warning,
        };
        let mut text = diagnostic.message.clone();
        // One line, always. A wrapped hint would push nothing (it is in a
        // stack) but would run over the code on the line below it, which reads
        // as corruption rather than as a long message.
        if let Some(cut) = text.find('\n') {
            text.truncate(cut);
        }
        if more > 0 {
            text.push_str(&format!("   +{more} more"));
        }

        children.push(
            Positioned::new()
                .left(rect.right + LEAD)
                .top(rect.top)
                .child(
                    Container::new()
                        .height(code_line(studio.font_size.get()))
                        .alignment(Alignment::CENTER_LEFT)
                        .child(label(&text, HINT_SIZE, color)),
                )
                .into(),
        );
    }

    // The completion list, in the same stack and for the same reason: it is
    // positioned from the caret rect this report carries, and putting it at
    // window level would mean tracking the field's global offset through a
    // sidebar width and two scroll offsets that all move.
    if studio.completion.get().is_some() {
        children.push(
            crate::ui::completion::CompletionList {
                studio: studio.clone(),
                caret: report.cursor,
                bounds: report.paragraph,
            }
            .into(),
        );
    }

    Stack::new()
        // The field sizes the stack and the hints are placed against it. Any
        // other fit either forces the hints to the stack's full size or lets
        // them decide how wide the code pane is.
        .fit(StackFit::Loose)
        .alignment(Alignment::TOP_LEFT)
        .children(children)
        .into()
}

/// The minimap: the file, scaled down, with the window you are looking through
/// drawn on it.
///
/// # What this replaced, and why the replacement is not glyphs
///
/// It was a bar chart — one grey bar per line, its width the line's length —
/// with a comment saying a real minimap "needs the external-texture object
/// (plan §6.1) and arrives with M2". That is one way to build one, and it is
/// not the way to build a good one: a minimap is drawn at two or three points
/// per line, and at that size glyph rasterisation is noise. Every editor that
/// offers both — VS Code names them "proportional" and "blocks" — falls back to
/// blocks below about four points, because a block *is* what a letter looks
/// like when it is two pixels tall, only without the sampling artefacts.
///
/// So this draws blocks, and the thing that makes it a scaled render of the
/// buffer rather than a chart of its line lengths is that the blocks are
/// **per token, at the token's column, in the token's colour**, taken from the
/// same highlighter that colours the code. Indentation, the shape of a match
/// arm, a long string literal, a block of comments, a run of `use` statements —
/// all of them are recognisable, which is the entire job.
///
/// # It fits the pane, and it says where you are
///
/// The rows are scaled so the whole file fits the height available, down to a
/// floor of one point per line; past that the file is longer than the minimap
/// has rows for and it is sampled, which is the honest thing and is what the
/// "of N lines" in the studio's status bar is for. The viewport is drawn as a
/// lightened band, and a press anywhere on the strip jumps there.
#[derive(Debug)]
struct Minimap {
    studio: Studio,
    /// One row per source line: the coloured runs on it, as
    /// `(start column, length, colour)`.
    ///
    /// Behind an [`Rc`] because the picture is drawn inside a [`LayoutBuilder`]
    /// closure, which outlives this widget and has to own what it draws. A
    /// refcount bump per build; a clone of one entry per source line would be a
    /// copy of the whole file's shape on every frame the editor rebuilds.
    rows: Rc<Vec<Vec<(usize, usize, Color)>>>,
    /// Lines carrying a diagnostic, drawn across the full width.
    flagged: Rc<Vec<u32>>,
    chrome: StudioTheme,
    colors: ColorScheme,
}

/// How wide the strip is, and how much of a line fits across it.
const MINIMAP_WIDTH: f32 = 64.0;
/// Points per character across. Two is what makes 80 columns fit in 64 points
/// with room for the padding either side.
const MINIMAP_COLUMN: f32 = 0.7;
/// The tallest a row gets when the file is short enough to have the room, and
/// the shortest it gets before the file starts being sampled instead.
const MINIMAP_ROW_MAX: f32 = 4.0;
const MINIMAP_ROW_MIN: f32 = 1.0;

impl Widget for Minimap {
    fn debug_name(&self) -> &'static str {
        "Minimap"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let _ = ctx;
        let studio = self.studio.clone();
        let rows = Rc::clone(&self.rows);
        let flagged = Rc::clone(&self.flagged);
        let chrome = self.chrome;
        let colors = self.colors;

        // **Measured, not guessed.** This used to take the window's height less
        // a flat 260 for "the chrome above and below the code pane", with a
        // comment saying the two agreed to within a few points. They agree until
        // the chrome changes height: opening *replace* adds a second 34-point
        // row to the find bar, the estimate does not know, and a 62-line file
        // asks for 248 points of rows in the 239 the pane now has — the studio's
        // one standing layout overflow, and it moved with the window, the font
        // size and the panel. A `LayoutBuilder` is one extra layout pass and the
        // number is simply right.
        LayoutBuilder::new(move |constraints: Constraints| {
            let lines = rows.len().max(1);
            // Infinity means nothing above bounded this — the minimap is not in
            // a scrollable today, and if it ever is, the window estimate is the
            // best answer available rather than an infinite row height.
            let available = if constraints.max_height.is_finite() {
                constraints.max_height
            } else {
                (studio.window_size.get().height - 260.0).max(120.0)
            };
            #[expect(
                clippy::cast_precision_loss,
                reason = "a line count that overflows f32's exact range is a file no editor opens"
            )]
            let ideal = available / lines as f32;
            let row = ideal.clamp(MINIMAP_ROW_MIN, MINIMAP_ROW_MAX);
            // How many source lines each drawn row stands for, once one row per
            // line no longer fits.
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let stride = ((MINIMAP_ROW_MIN / ideal).ceil() as usize).max(1);

            let dim = blend(colors.on_surface_variant, chrome.chrome_2, 0.55);

            // **The window, drawn on the file.** This is the half the old bar chart
            // could not have: it scrolled with the text, so there was no "here" to
            // mark. `editor_scroll`'s offset and extent are in code-pane points;
            // scaled by the same ratio the rows are, they are where the band goes.
            let line_box = code_line(studio.font_size.get());
            let extent = studio.editor_scroll.viewport();
            #[expect(clippy::cast_precision_loss, reason = "a line count fits f32")]
            let ratio = row / (line_box * stride as f32);
            let band_top = studio.editor_scroll.offset() * ratio;
            let band_height = (extent * ratio).max(row * 3.0);

            let studio = studio.clone();
            let lines_total = lines;
            let jump = move |dy: f32| {
                // Where on the strip, as a fraction, becomes which line — and the
                // caret goes there, which scrolls the pane to it. Column 1, because
                // a minimap addresses a line and nothing narrower.
                #[expect(clippy::cast_precision_loss, reason = "a line count fits f32")]
                let rows = (lines_total as f32 / stride as f32).max(1.0);
                let fraction = (dy / (rows * row)).clamp(0.0, 1.0);
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a fraction of a line count is a line number"
                )]
                let line = (fraction * lines_total as f32) as u32 + 1;
                studio.jump_to(line, 1);
            };
            let drag = jump.clone();

            Container::new()
                .color(chrome.chrome_2)
                .width(MINIMAP_WIDTH)
                .padding(EdgeInsets::only(6.0, 0.0, 6.0, 0.0))
                .child(
                    GestureDetector::new()
                        // Zero, so a strip two points wide per line does not claim
                        // a 48-point band of the code pane beside it. See
                        // `ui::divider::GRAB` for the same rule and why it matters.
                        .touch_target(0.0)
                        .on_tap_down(move |details| jump(details.local.dy))
                        .drag_axis(vieww_foundation::Axis::Vertical)
                        .on_drag_update(move |details| drag(details.local.dy))
                        // **One painter, not one node per line.** The band and
                        // the blocks are drawn in one recording, in that order —
                        // the wash is opaque, and drawn *last* it hid exactly the
                        // part of the file you are looking at, which is the one
                        // part a minimap must never hide.
                        .child(vieww_widget::Painting::new(MinimapStrip {
                            rows: Rc::clone(&rows),
                            flagged: Rc::clone(&flagged),
                            row,
                            stride,
                            dim,
                            error: colors.error,
                            band: (
                                band_top,
                                band_height,
                                blend(colors.on_surface, chrome.chrome_2, 0.13),
                            ),
                        })),
                )
                .into()
        })
        .into()
    }
}

widget_node_from!(Minimap);

/// The column of line numbers, fold markers and error marks beside the code.
///
/// # Why this is a widget with one signal rather than a helper
///
/// It is one fixed-height row per source line — 175 elements on a 62-line file
/// — and it was built inline in `code_pane` from the buffer's text, the fold
/// set, the foldable regions and the caret. So every character typed rebuilt
/// all of it, to produce the same numbers in the same places.
///
/// It now reads exactly one thing: [`Studio::gutter`](crate::Studio), a
/// published `Vec<GutterRow>` republished with `set_if_changed`. Typing inside
/// a line changes no row, so the signal does not fire and none of these
/// elements is touched. See [`GutterRow`](crate::state::GutterRow) for the
/// derivation and the oracle test that keeps it honest.
///
/// `line_box` and `code_size` are passed in rather than read here because they
/// are the *pane's* geometry — the same numbers the text field is laid out
/// with, and a gutter that read the font size itself could disagree with the
/// field it sits beside by one frame.
#[derive(Debug)]
struct Gutter {
    studio: Studio,
    line_box: f32,
    code_size: f32,
}

widget_node_from!(Gutter);

impl Widget for Gutter {
    fn debug_name(&self) -> &'static str {
        "Gutter"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    /// **The half that actually stops the rebuild.** Narrowing this widget's
    /// own subscriptions to one signal was not enough: it lives inside the code
    /// pane, the code pane rebuilds on every keystroke because it shows the
    /// text, and a rebuilt parent reconstructs its children.
    ///
    /// Every field is compared, and none of them is a closure — `studio` is a
    /// handle to the same signals however many times it is cloned, and the two
    /// numbers are the pane's geometry. The row data itself is not a field: it
    /// is read from a signal during `build`, and if it changed this element is
    /// already `pending`, which the caller checks.
    fn same_configuration(&self, other: &WidgetNode) -> bool {
        other.downcast_ref::<Self>().is_some_and(|new| {
            self.studio.same_studio(&new.studio)
                && (self.line_box - new.line_box).abs() < f32::EPSILON
                && (self.code_size - new.code_size).abs() < f32::EPSILON
        })
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = *StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;
        let rows = self.studio.gutter.get();
        let (line_box, code_size) = (self.line_box, self.code_size);

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::End)
            .children(
                rows.iter()
                    .map(|row| {
                        let color = if row.flagged {
                            colors.error
                        } else if row.active {
                            chrome.gutter_active
                        } else {
                            chrome.gutter
                        };
                        // A marker only where there is a region to open or
                        // close, and it says which: a collapsed region shows a
                        // filled chevron, an open one an outline.
                        let marker: WidgetNode = match row.fold {
                            Some(is_folded) => {
                                let studio = self.studio.clone();
                                let header = row.fold_line;
                                let gutter_tint = chrome.gutter;
                                clickable(
                                    move || {
                                        crate::ui::chrome::glyph_icon(if is_folded {
                                            icons::chevron_right()
                                        } else {
                                            icons::chevron_down()
                                        })
                                        .size(11.0)
                                        .color(if is_folded {
                                            colors.primary
                                        } else {
                                            gutter_tint
                                        })
                                        .into()
                                    },
                                    move || studio.toggle_fold_at(header),
                                )
                                .into()
                            }
                            None => SizedBox::shrink().into(),
                        };
                        WidgetNode::from(
                            Container::new()
                                .height(line_box)
                                .alignment(Alignment::CENTER_RIGHT)
                                .child(
                                    Flex::row()
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .children(children![
                                            if row.flagged {
                                                WidgetNode::from(crate::ui::chrome::glyph(
                                                    icons::error(),
                                                    9.0,
                                                    colors.error,
                                                ))
                                            } else {
                                                SizedBox::shrink().into()
                                            },
                                            space(4.0),
                                            mono(&row.number.to_string(), code_size, color),
                                            space(3.0),
                                            marker,
                                        ]),
                                ),
                        )
                    })
                    .collect::<Vec<WidgetNode>>(),
            )
            .into()
    }
}

/// The minimap's picture: the viewport band, then one block per coloured run.
///
/// # Why this is a painter and not a widget tree
///
/// It used to be one `Positioned > SizedBox > Stack` per source line and one
/// `Positioned > Container` per coloured run inside that — **187 elements** on
/// a 62-line file, every one of them a solid rectangle, all rebuilt on every
/// character typed. That was the single largest region in the studio's
/// keystroke rebuild, larger than the editor field it is a picture of.
///
/// A minimap is a *drawing*. It has no layout to negotiate, no children to
/// reconcile, no state, and nothing in it can be hit-tested independently — the
/// `GestureDetector` above maps a `dy` to a line and neither knows nor cares
/// which block was under the finger. Every one of those elements existed only
/// so that a rectangle could be filled at a computed offset, which is what
/// [`Painter`](vieww_widget::Painter) is for and what `docs/VISUALS.md` says a
/// picture should be built with.
///
/// `should_repaint` compares the values rather than defaulting to `true`, so a
/// frame that rebuilds this widget with the same file, the same scale and the
/// same scroll position re-records nothing at all.
#[derive(Debug)]
struct MinimapStrip {
    /// One entry per source line: `(start column, length, colour)` runs.
    rows: Rc<Vec<Vec<(usize, usize, Color)>>>,
    /// Lines carrying a diagnostic, drawn across the full width.
    flagged: Rc<Vec<u32>>,
    /// Height of one drawn row, in points.
    row: f32,
    /// How many source lines each drawn row stands for.
    stride: usize,
    /// The colour an unstyled run is drawn in.
    dim: Color,
    /// The colour a flagged line is drawn in.
    error: Color,
    /// The viewport marker: `(top, height, colour)`.
    band: (f32, f32, Color),
}

impl vieww_widget::Painter for MinimapStrip {
    fn paint(&self, book: &mut Sketchbook, size: Size) {
        let width = size.width;
        // The band first: it is opaque, and the file has to be legible on it.
        let (top, height, colour) = self.band;
        book.rrect(Rect::new(0.0, top, width, top + height), 2.0, colour);

        // A hairline of separation between rows once there is room for one, so a
        // block of code reads as lines rather than as a slab.
        let block = (self.row - 0.5).max(MINIMAP_ROW_MIN);
        for (drawn, (index, runs)) in self
            .rows
            .iter()
            .enumerate()
            .step_by(self.stride)
            .enumerate()
        {
            #[expect(clippy::cast_precision_loss, reason = "a drawn-row index is small")]
            let top = drawn as f32 * self.row;
            if top > size.height {
                // Past the bottom of the strip. The clip would drop these
                // anyway; not recording them is cheaper than clipping them.
                break;
            }
            #[expect(clippy::cast_possible_truncation, reason = "a line number fits u32")]
            let number = index as u32 + 1;
            if self.flagged.contains(&number) {
                // A flagged line is the one thing worth seeing from the other
                // side of the room, so it is the full width rather than the
                // shape of its code.
                book.rect(Rect::new(0.0, top, width, top + self.row), self.error);
                continue;
            }
            for (column, length, color) in runs {
                #[expect(clippy::cast_precision_loss, reason = "a column is small")]
                let left = *column as f32 * MINIMAP_COLUMN;
                if left > width {
                    // Past the right edge of the strip. Dropped rather than
                    // clamped: a pile of blocks against the edge would read as a
                    // solid margin that is not in the file.
                    break;
                }
                #[expect(clippy::cast_precision_loss, reason = "a run length is small")]
                let run = (*length as f32 * MINIMAP_COLUMN)
                    .min(width - left)
                    .max(MINIMAP_COLUMN);
                let ink = if *color == Color::TRANSPARENT {
                    self.dim
                } else {
                    *color
                };
                book.rect(Rect::new(left, top, left + run, top + block), ink);
            }
        }
    }

    fn should_repaint(&self, previous: &dyn vieww_widget::Painter) -> bool {
        // A painter of another type is by definition a different drawing.
        previous.as_any().downcast_ref::<Self>().is_none_or(|old| {
            !Rc::ptr_eq(&self.rows, &old.rows)
                || !Rc::ptr_eq(&self.flagged, &old.flagged)
                || (self.row - old.row).abs() > f32::EPSILON
                || self.stride != old.stride
                || self.dim != old.dim
                || self.error != old.error
                || self.band != old.band
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The coloured runs on each line, from the same spans that colour the code.
///
/// A run is a stretch of non-space characters inside one span: that is what
/// makes the picture look like code rather than like a bar chart, because the
/// gaps between words survive the scaling and the indentation does.
fn minimap_rows(spans: &[vieww_text::TextSpan], text: &str) -> Vec<Vec<(usize, usize, Color)>> {
    let mut rows: Vec<Vec<(usize, usize, Color)>> = vec![Vec::new()];
    let mut column = 0usize;

    // The spans cover the buffer in order, so walking them is walking the text.
    // Falling back to the raw text when there are none is what keeps a file with
    // no grammar — and one past the highlight size limit — looking like itself.
    let flat;
    let spans: &[vieww_text::TextSpan] = if spans.is_empty() {
        flat = vec![vieww_text::TextSpan::new(
            text,
            vieww_foundation::TextStyle::new(0.0),
        )];
        &flat
    } else {
        spans
    };

    for span in spans {
        let color = span.style.color;
        let mut run: Option<usize> = None;
        for character in span.text.chars() {
            match character {
                '\n' => {
                    if let Some(start) = run.take() {
                        rows.last_mut()
                            .expect("a row")
                            .push((start, column - start, color));
                    }
                    rows.push(Vec::new());
                    column = 0;
                }
                c if c.is_whitespace() => {
                    if let Some(start) = run.take() {
                        rows.last_mut()
                            .expect("a row")
                            .push((start, column - start, color));
                    }
                    // A tab is why this counts columns rather than bytes: four
                    // of them at the head of a line is an indent the picture has
                    // to show, and one byte is not.
                    column += if c == '\t' { 4 } else { 1 };
                }
                _ => {
                    run.get_or_insert(column);
                    column += 1;
                }
            }
        }
        if let Some(start) = run {
            rows.last_mut()
                .expect("a row")
                .push((start, column - start, color));
        }
    }
    rows
}

/// `amount` of `top` over `bottom`, both opaque.
///
/// Written here rather than reached for from `vieww-foundation` because the
/// only thing the shell needs is an opaque wash, and an alpha-composited
/// version of the same call would put a translucent layer in the tree for
/// every flagged line.
fn blend(top: Color, bottom: Color, amount: f32) -> Color {
    let mix = |a: u8, b: u8| -> u8 {
        let a = f32::from(a);
        let b = f32::from(b);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a convex combination of two bytes is a byte"
        )]
        {
            (b + (a - b) * amount).round().clamp(0.0, 255.0) as u8
        }
    };

    Color::rgb(
        mix(top.r, bottom.r),
        mix(top.g, bottom.g),
        mix(top.b, bottom.b),
    )
}

#[cfg(test)]
mod minimap_tests {
    use super::*;

    fn spans(text: &str, runs: &[(&str, Color)]) -> Vec<vieww_text::TextSpan> {
        let _ = text;
        runs.iter()
            .map(|(text, color)| {
                vieww_text::TextSpan::new(
                    *text,
                    TextStyle {
                        color: *color,
                        ..TextStyle::new(13.0)
                    },
                )
            })
            .collect()
    }

    /// The property that makes this a scaled render of the buffer rather than a
    /// chart of line lengths: indentation is preserved, so the *shape* of the
    /// code is what you see.
    #[test]
    fn indentation_survives_the_scaling() {
        let text = "fn f() {\n    let x = 1;\n}\n";
        let rows = minimap_rows(&[], text);
        assert_eq!(rows.len(), 4, "three lines and the empty tail: {rows:?}");
        assert_eq!(rows[0][0].0, 0, "`fn` starts at column 0");
        assert_eq!(rows[1][0].0, 4, "and `let` four columns in");
    }

    /// And the gaps between words, which is the other half of why it reads as
    /// code. A bar per line cannot show either.
    #[test]
    fn each_run_of_non_space_is_its_own_block() {
        let rows = minimap_rows(&[], "let x = 1;\n");
        let widths: Vec<(usize, usize)> = rows[0].iter().map(|(at, len, _)| (*at, *len)).collect();
        assert_eq!(widths, vec![(0, 3), (4, 1), (6, 1), (8, 2)]);
    }

    /// A tab is an indent the picture has to show, and it is one byte.
    #[test]
    fn a_tab_counts_as_an_indent_rather_than_as_one_column() {
        let rows = minimap_rows(&[], "\tx\n");
        assert_eq!(rows[0][0].0, 4);
    }

    /// The blocks take the highlighter's colours, which is what makes a comment
    /// block, a string and a run of keywords tell themselves apart at 4 points
    /// per line.
    #[test]
    fn the_blocks_are_the_colours_the_code_is() {
        let red = Color::rgb(0xFF, 0, 0);
        let blue = Color::rgb(0, 0, 0xFF);
        let rows = minimap_rows(&spans("", &[("let ", red), ("x\n", blue)]), "let x\n");
        assert_eq!(rows[0][0].2, red);
        assert_eq!(rows[0][1].2, blue);
    }

    /// A file with no grammar — or one past the highlighter's size limit —
    /// still gets a picture, because the alternative is a blank strip beside a
    /// file that plainly has content in it.
    #[test]
    fn a_file_with_no_spans_is_still_drawn() {
        let rows = minimap_rows(&[], "SELECT 1;\nSELECT 2;\n");
        assert_eq!(rows.len(), 3);
        assert!(!rows[0].is_empty(), "{rows:?}");
    }

    #[test]
    fn an_empty_buffer_is_one_empty_row_rather_than_a_panic() {
        assert_eq!(minimap_rows(&[], ""), vec![Vec::new()]);
    }
}
