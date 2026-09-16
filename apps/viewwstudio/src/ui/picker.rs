//! The Open Folder overlay.
//!
//! Drawn the way the palette is — a scrim and a card in the overlay layer —
//! because it is the same kind of thing: a modal that takes the whole window
//! for one decision and leaves the layout under it alone.
//!
//! # Why this exists rather than a call to the platform
//!
//! There is no platform file dialog in vieww to call. `crate::picker` has the
//! long version; the short one is that `HTMLPROTOTYPEMAPPING.md` §7 lists the
//! native picker as absent with the note *build the picker in-app*, and this is
//! that. Until it existed the workspace root was `argv[1]` and only `argv[1]`.
//!
//! # Directories only, and the current one is the answer
//!
//! Every row navigates. The button confirms *where you are*, not what is
//! selected — which is how every folder picker on every desktop works, and
//! avoids a second selection state that can disagree with the listing.
//!
//! # Volumes, the row above the entries
//!
//! [`Picker::volumes`](crate::picker::Picker::volumes) is the part of the
//! picker that reaches across filesystem boundaries — `/`, `$HOME`,
//! `/Volumes/USB`, `D:\`, `/mnt/external`. Without it the picker can walk into
//! subdirectories and back out to a root, and stop there: `Path::parent("/")`
//! is `None` on Unix and `Path::parent("C:\\")` is `None` on Windows, so a
//! user whose project is on `D:` and whose picker opened on `C:\Users\me` has
//! no way to get there. The volume strip is that way: one chip per mount, a
//! click navigates to its root and the entries list re-reads.
//!
//! Drawn above the entries rather than among them because a volume is a *peer*
//! of `cwd`, not a child — see [`crate::picker::Volume`] for the longer
//! argument. Same scroll physics as the entries, on a separate controller so
//! the two axes cannot disagree.

use vieww_foundation::{Alignment, Border, Color, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Constrained, Container, Flexible, Scrollable, SizedBox};

use crate::picker::Volume;
use crate::state::{PickerMode, Studio};
use crate::theme::StudioTheme;
use crate::ui::chrome::{clickable, gap, hairline, label, label_bold, space};

const WIDTH: f32 = 520.0;
const ROW: f32 = 30.0;

/// The tallest the list gets before it scrolls.
///
/// # What this replaces
///
/// A hard cap of eleven rows, with a note under them reading *"11 of 34 folders
/// shown — go into one to narrow it"*. The defence was that a directory holds
/// "tens of entries at the very worst"; `/usr/lib`, a home directory, a
/// monorepo's `packages/` and anything with a `node_modules` in it are all
/// hundreds, and the twelfth folder was simply unreachable — the modal listed
/// it, counted it, and gave no way to get to it.
///
/// The original reason for the cap was right and is kept: *"a modal that grows
/// past the window is a modal with its confirm button off screen"*. So the
/// list is bounded in **height** rather than in rows, and scrolls inside that
/// bound. Eleven rows' worth, which is the same modal it always was when there
/// are eleven folders or fewer.
const MAX_LIST: f32 = ROW * 11.0;

/// The height of the volume chip strip.
///
/// Smaller than a directory row because the strip is a *header* — it stays
/// visible while the entries scroll under it, and a header that takes a
/// directory row's height each is a header that costs the modal two folders
/// of its scroll budget per volume.
const VOLUME_STRIP: f32 = 28.0;

#[derive(Debug)]
pub struct FolderPicker {
    pub studio: Studio,
}

impl Widget for FolderPicker {
    fn debug_name(&self) -> &'static str {
        "FolderPicker"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let Some(picker) = self.studio.picker.get() else {
            // Absent rather than transparent, for the reason the palette gives:
            // an invisible overlay still eats the pointer events under it.
            return SizedBox::shrink().into();
        };

        let chrome = *StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;
        let mode = self.studio.picker_mode.get();

        let mut column: Vec<WidgetNode> = vec![
            // The header is the path, and it is the one thing here that has to
            // stay readable at 520 points — see `Picker::crumb`.
            //
            // The title and the confirm button change with `mode`: the picker
            // is reused for both "open an existing folder" and "pick where
            // the new project goes", and a modal whose title does not match
            // what its confirm button will do is the kind of mismatch that
            // makes a user hesitate on the click they should not have to.
            Container::new()
                .padding(EdgeInsets::symmetric(14.0, 11.0))
                .child(
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            label_bold(mode.title(), 13.0, colors.on_surface),
                            space(10.0),
                            Flexible::expanded(1).child(label(
                                &picker.crumb(),
                                11.5,
                                colors.on_surface_variant
                            )),
                        ]),
                )
                .into(),
            hairline(chrome.line),
        ];

        // The volume strip — see the module docs for why it sits here rather
        // than among the entries. Always drawn, even when the picker is at a
        // volume the strip itself lists: hiding the row that matches `cwd`
        // would make the strip jump in and out as the user navigates, which is
        // the kind of motion a header should never make.
        column.push(self.volumes(&picker, colors, chrome));
        column.push(hairline(chrome.line));

        // "Up" first and always, including from a directory that could not be
        // read — it is the only way out of one.
        if picker.parent().is_some() {
            let studio = self.studio.clone();
            column.push(navigation_row(
                "..".to_owned(),
                None,
                colors,
                chrome,
                move || studio.picker_up(),
            ));
        }

        if let Some(error) = picker.error.as_ref() {
            column.push(
                Container::new()
                    .padding(EdgeInsets::symmetric(14.0, 10.0))
                    .child(label(error, 12.0, colors.error))
                    .into(),
            );
        } else if picker.entries.is_empty() {
            column.push(
                Container::new()
                    .padding(EdgeInsets::symmetric(14.0, 10.0))
                    .child(label(
                        "No folders in here. Open this one, or go up.",
                        12.0,
                        colors.on_surface_variant,
                    ))
                    .into(),
            );
        } else {
            let mut rows: Vec<WidgetNode> = Vec::new();
            for entry in &picker.entries {
                let studio = self.studio.clone();
                let path = entry.path.clone();
                rows.push(navigation_row(
                    entry.name.clone(),
                    // The mark is a hint, not a filter — a folder without it is
                    // still openable, and saying which ones are vieww projects
                    // is most of what somebody is scanning this list for.
                    entry.is_vieww.then(|| "vieww".to_owned()),
                    colors,
                    chrome,
                    move || studio.picker_to(&path),
                ));
            }
            let scroll = self.studio.picker_scroll.clone();
            let list = Scrollable::vertical(scroll.offset())
                .on_drag(scroll.on_drag())
                .on_drag_end(scroll.on_drag_end())
                .on_extents(scroll.on_extents())
                .child(
                    Flex::column()
                        .main_axis_size(MainAxisSize::Min)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(rows),
                );
            column.push(
                Constrained::new(Constraints::loose(vieww_foundation::Size::new(
                    f32::INFINITY,
                    // Bounded so the modal cannot grow its buttons off the
                    // bottom of the window — the cap's original argument, kept.
                    MAX_LIST,
                )))
                .child(list)
                .into(),
            );
        }

        column.push(hairline(chrome.line));
        column.push(self.actions(&picker, colors, chrome, mode));

        let studio = self.studio.clone();
        let scrim = clickable(
            move || Container::new().color(Color::rgba(0, 0, 0, 90)).into(),
            move || studio.close_picker(),
        );

        Stack::new()
            .alignment(Alignment::TOP_CENTER)
            .children(children![
                Positioned::new()
                    .left(0.0)
                    .right(0.0)
                    .top(0.0)
                    .bottom(0.0)
                    .child(scrim),
                Positioned::new().top(64.0).child(
                    Container::new()
                        .color(chrome.chrome_2)
                        .radius(10.0)
                        .width(WIDTH)
                        .border(Border {
                            color: chrome.line,
                            width: 1.0,
                        })
                        .child(
                            Flex::column()
                                .main_axis_size(MainAxisSize::Min)
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .children(column)
                        )
                ),
            ])
            .into()
    }
}

widget_node_from!(FolderPicker);

impl FolderPicker {
    /// Cancel, and confirm.
    ///
    /// What confirming does is mode-dependent — see [`PickerMode`] and
    /// [`Studio::picker_choose`]. The label and the hint on the left are
    /// derived from `mode`, so the button says what it will do and the hint
    /// says what to expect.
    ///
    /// In [`PickerMode::Open`], the hint says whether the picked folder is a
    /// vieww project, because that is the difference between a studio that can
    /// build and one that can only edit — and it is knowable *before* the
    /// click, which is the only time saying it helps.
    ///
    /// In [`PickerMode::NewProject`], the hint says the project will be
    /// created inside the picked folder — the vieww-or-not question does not
    /// apply, because the new project's manifest will mention vieww whether
    /// the parent has one or not.
    fn actions(
        &self,
        picker: &crate::picker::Picker,
        colors: ColorScheme,
        chrome: StudioTheme,
        mode: PickerMode,
    ) -> WidgetNode {
        let is_vieww = picker.cwd_is_vieww();
        let cancel = {
            let studio = self.studio.clone();
            clickable(
                move || {
                    Container::new()
                        .height(30.0)
                        .radius(6.0)
                        .padding(EdgeInsets::symmetric(14.0, 0.0))
                        .alignment(Alignment::CENTER)
                        .color(chrome.chrome_1)
                        .child(label("Cancel", 12.0, colors.on_surface_variant))
                        .into()
                },
                move || studio.close_picker(),
            )
        };
        let confirm_label = mode.confirm_label();
        let confirm = {
            let studio = self.studio.clone();
            clickable(
                move || {
                    Container::new()
                        .height(30.0)
                        .radius(6.0)
                        .padding(EdgeInsets::symmetric(14.0, 0.0))
                        .alignment(Alignment::CENTER)
                        .color(colors.primary)
                        .child(label(confirm_label, 12.0, colors.on_primary))
                        .into()
                },
                move || studio.picker_choose(),
            )
        };

        // The hint to the left of the buttons is mode-dependent too:
        //
        // - In `Open` mode, the question the user is answering is "will this
        //   be a buildable vieww project?", and the answer is whether the
        //   picked folder's `Cargo.toml` mentions vieww. That is the
        //   historical hint and it stays.
        // - In `NewProject` mode, the question is different: "the new
        //   project will live inside this folder". Whether the folder itself
        //   has a `Cargo.toml` mentioning vieww is irrelevant — the new
        //   project's manifest will mention vieww because the scaffold writes
        //   it — so the vieww-or-not hint is wrong here. The hint that *is*
        //   useful is "the project will be created inside this folder", which
        //   is what the picked folder means in this mode.
        let (hint, hint_color) = match mode {
            PickerMode::Open => (
                if is_vieww {
                    "A vieww project — Render and Build will both work."
                } else {
                    "No vieww in its Cargo.toml — Render will refuse."
                },
                if is_vieww {
                    colors.success
                } else {
                    colors.outline
                },
            ),
            PickerMode::NewProject => (
                "A new vieww project will be created inside this folder.",
                colors.on_surface_variant,
            ),
        };

        Container::new()
            .padding(EdgeInsets::symmetric(14.0, 10.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![
                        // Flexible, for the reason the menu's disabled-reason
                        // row is: this is a sentence beside two fixed buttons
                        // in a fixed-width dialog, and in `NewProject` mode
                        // it is three logical pixels too long. The buttons keep
                        // their intrinsic width and the sentence takes what is
                        // left.
                        Flexible::expanded(1).child(
                            Clip::rect().child(
                                Container::new()
                                    .alignment(Alignment::CENTER_LEFT)
                                    .child(label(hint, 10.5, hint_color))
                            )
                        ),
                        cancel,
                        space(8.0),
                        confirm,
                    ]),
            )
            .into()
    }

    /// The volume strip: one chip per mounted volume, horizontally scrollable
    /// when there are more than the modal width can show.
    ///
    /// Each chip is a clickable that calls [`Studio::picker_to`] with the
    /// volume's path — the same wire a directory row uses, just a different
    /// address. The chip whose `path` equals `picker.cwd` is filled with the
    /// primary colour so the user can see at a glance which volume the picker
    /// is currently in, which is the only piece of selection state the strip
    /// needs to carry: confirming happens at the bottom of the modal, not
    /// here.
    fn volumes(
        &self,
        picker: &crate::picker::Picker,
        colors: ColorScheme,
        chrome: StudioTheme,
    ) -> WidgetNode {
        let mut chips: Vec<WidgetNode> = Vec::with_capacity(picker.volumes.len());
        for volume in &picker.volumes {
            let studio = self.studio.clone();
            let path = volume.path.clone();
            let is_current = path == picker.cwd;
            chips.push(volume_chip(
                volume.clone(),
                is_current,
                colors,
                chrome,
                move || studio.picker_to(&path),
            ));
        }

        let scroll = self.studio.picker_volume_scroll.clone();
        let row = Scrollable::horizontal(scroll.offset())
            .on_drag(scroll.on_drag())
            .on_drag_end(scroll.on_drag_end())
            .on_extents(scroll.on_extents())
            .child(
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(6.0)
                    .children(chips),
            );

        Container::new()
            .height(VOLUME_STRIP)
            .padding(EdgeInsets::symmetric(10.0, 6.0))
            .child(row)
            .into()
    }
}

/// One directory row.
fn navigation_row(
    name: String,
    mark: Option<String>,
    colors: ColorScheme,
    chrome: StudioTheme,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    clickable(
        move || {
            let mut row: Vec<WidgetNode> =
                vec![label(&name, 12.5, colors.on_surface).into(), gap()];
            if let Some(mark) = mark.as_ref() {
                row.push(
                    Container::new()
                        .radius(4.0)
                        .color(chrome.chrome_1)
                        .padding(EdgeInsets::symmetric(6.0, 2.0))
                        .child(label(mark, 10.0, colors.primary))
                        .into(),
                );
            }
            Container::new()
                .height(ROW)
                .padding(EdgeInsets::symmetric(14.0, 0.0))
                .child(
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(row),
                )
                .into()
        },
        on_tap,
    )
    .into()
}

/// One volume chip.
///
/// Smaller than a directory row and visually distinct: a flat chip with a
/// hairline border rather than a full-height strip with a background, because
/// a row that *looked* like a directory row would be clicked the way a
/// directory row is clicked — to *enter* — and a volume chip is *jump to its
/// root*, which is a different action and needs to read as one.
fn volume_chip(
    volume: Volume,
    is_current: bool,
    colors: ColorScheme,
    chrome: StudioTheme,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    clickable(
        move || {
            // The chip's text is the volume's name — `/`, `Home`, `C:`,
            // `USB Drive`. The path it would navigate to is shown by the
            // breadcrumb above, so repeating it here would be the breadcrumb
            // said twice.
            let text = &volume.name;
            Container::new()
                .radius(6.0)
                .height(20.0)
                .padding(EdgeInsets::symmetric(8.0, 0.0))
                .alignment(Alignment::CENTER)
                .color(if is_current {
                    colors.primary
                } else {
                    chrome.chrome_1
                })
                .border(Border {
                    color: if is_current {
                        colors.primary
                    } else {
                        chrome.line
                    },
                    width: 1.0,
                })
                .child(label(
                    text,
                    11.0,
                    if is_current {
                        colors.on_primary
                    } else {
                        colors.on_surface_variant
                    },
                ))
                .into()
        },
        on_tap,
    )
    .into()
}
