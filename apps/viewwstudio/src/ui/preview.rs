//! The right pane: platform picker, device frame, and the honest caption.
//!
//! **What is inside the frame is the user's own screen, compiled and mounted.**
//! This paragraph used to describe M0's placeholder — "a placeholder drawn by
//! this application" — seven hundred lines above the code that mounts the real
//! guest widget tree, so a reviewer who read the top of the file and stopped
//! would have concluded the studio's headline feature did not work.
//!
//! What actually happens: `compile` builds the buffer to a `cdylib`, `loaded`
//! `dlopen`s it behind an ABI fingerprint check and a `catch_unwind`, and this
//! pane mounts the returned widget inside an `Inherited<ViewMetrics>` and a
//! device `ThemeData` so the guest sees the simulated platform's insets and
//! text scale. (Scroll physics is *not* among them — see `caption` below for
//! why, and why the caption used to say it was.)
//!
//! The caption under the frame states the two things that are still simulated
//! rather than real — widgets are not visually reskinned per platform, and the
//! previewed screen's state resets on every Render — because a preview that
//! quietly differs from the device is worse than one that says where it
//! differs.

use vieww_foundation::{Alignment, Color, EdgeInsets, FontFamily, TextStyle};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, Flexible, Inherited, Painting};

use crate::command::Command;
use crate::state::{Platform, PreviewState, Studio};
use crate::theme::StudioTheme;
use crate::ui::chrome::{
    clickable, gap, hairline, label, label_bold, mono, segmented, space, SegmentedColors,
};

const BAR: f32 = 35.0;

/// The pane width below which the preview toolbar gives up on one row.
///
/// # Why a toolbar that was fixed-width for a year suddenly is not
///
/// Every control in the bar is fixed-width — a segmented platform picker, three
/// labelled toggles, a Render button — and together they want roughly 420
/// points, which was fine while `MIN_PANE` sat above that and the pane could
/// not be dragged narrower than its own toolbar. Lowering the floor to let the
/// editor genuinely take the window (the point of a divider) left the row
/// laying its inflexible children out at natural size regardless: the `gap()`
/// collapsed, then the last control in the row — the Render button, the one
/// control that starts a compile — slid under the pane's rounded clip and
/// vanished a letter at a time. Reported as “the render pane is not
/// minimizing, the render button is going behind the window”.
///
/// Below this width the bar becomes two rows: the picker and the Render button
/// keep the first, the three toggles take the second. Nothing is hidden, nothing
/// is dropped — the toolbar keeps every control clickable at every width the
/// divider can produce, which is the property the one-row layout could not
/// offer at any threshold.
///
/// The number is the full row's measured natural width (≈420) plus a hand's
/// width of slack, so the switch happens while the single row still fits and
/// never because it already does not.
///
/// Public for `tests/seam.rs`, which pins the reflow it governs.
pub const COMPACT_BAR_BELOW: f32 = 440.0;

#[derive(Debug)]
pub struct PreviewPane {
    pub studio: Studio,
}

impl Widget for PreviewPane {
    fn debug_name(&self) -> &'static str {
        "PreviewPane"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        let width = self.studio.preview_width.get();
        let platform = self.studio.platform.get();
        let state = self.studio.preview.get();

        let tab = self.studio.right_tab.get();
        let mut column: Vec<WidgetNode> =
            vec![self.tabs(&chrome, theme.colors), hairline(chrome.line)];
        if tab == crate::state::RightTab::Devices {
            // Scrolled, not cut: the list is nine fixed rows under a paragraph
            // of explanation, which is taller than the pane on a laptop.
            let scroll = self.studio.devices_scroll.clone();
            column.push(
                Flexible::expanded(1)
                    .child(
                        Clip::rect().child(
                            Scrollable::vertical(scroll.offset())
                                .key("devices")
                                .on_drag(scroll.on_drag())
                                .on_drag_end(scroll.on_drag_end())
                                .on_extents(scroll.on_extents())
                                .child(self.devices(&chrome, theme.colors)),
                        ),
                    )
                    .into(),
            );
            return Container::new()
                .color(chrome.chrome_1)
                .width(width)
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(column),
                )
                .into();
        }
        if tab == crate::state::RightTab::Inspector {
            // The whole pane, because a tree is a list and a list wants the
            // height. The device frame's toolbar means nothing here.
            column.push(
                Flexible::expanded(1)
                    .child(crate::ui::inspector::InspectorPane {
                        studio: self.studio.clone(),
                    })
                    .into(),
            );
            return Container::new()
                .color(chrome.chrome_1)
                .width(width)
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(column),
                )
                .into();
        }
        if tab == crate::state::RightTab::GeneratedRust {
            // The Rust the active Say buffer compiles to. Read-only by
            // construction: it is a view of what the studio compiles, the
            // same file every Render produces, byte for byte.
            let generated = self.studio.generated_rust.get();
            let scroll = self.studio.generated_scroll.clone();
            let body: WidgetNode = match generated {
                Some(text) => Clip::rect().child(
                    Scrollable::vertical(scroll.offset())
                        .key("generated")
                        .on_drag(scroll.on_drag())
                        .on_drag_end(scroll.on_drag_end())
                        .on_extents(scroll.on_extents())
                        .child(
                            Container::new()
                                .padding(EdgeInsets::all(12.0))
                                .child(Flex::column().cross_axis_alignment(CrossAxisAlignment::Start).children(
                                    text.lines()
                                        .map(|line| {
                                            Text::new(line.to_owned())
                                                .style(
                                                    TextStyle::new(11.5)
                                                        .family(FontFamily::Monospace)
                                                        .color(theme.colors.on_surface),
                                                )
                                                .into()
                                        })
                                        .collect::<Vec<WidgetNode>>(),
                                )),
                        ),
                )
                .into(),
                None => Container::new()
                    .padding(EdgeInsets::all(20.0))
                    .child(label(
                        "Open a `.say` file and press Render — the Rust it compiles to appears here.",
                        12.0,
                        theme.colors.on_surface_variant,
                    ))
                    .into(),
            };
            column.push(Flexible::expanded(1).child(body).into());
            return Container::new()
                .color(chrome.chrome_1)
                .width(width)
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(column),
                )
                .into();
        }
        column.push(self.bar(ctx, &chrome, theme.colors, platform, state, width));
        column.push(hairline(chrome.line));
        // Plan 2 §8.2's second bullet: when the fingerprints differ, *say so
        // and offer the fallback* rather than failing. Between the toolbar and
        // the stage so it reads as a condition of the pane rather than as
        // something wrong with the frame under it — the frame is not wrong, it
        // is simply the last one that loaded.
        if matches!(state, PreviewState::AbiRefused) {
            column.push(self.abi_refusal(&chrome, theme.colors));
            column.push(hairline(chrome.line));
        }
        column.push(
            Flexible::expanded(1)
                .child(stage(
                    &chrome,
                    self.studio.stage_scroll.clone(),
                    Stage {
                        colors: theme.colors,
                        // The previewed app is themed by *its own* code, not by
                        // the studio around it. Until M3 compiles that code it
                        // gets the light scheme — the one the sample asks for.
                        // When `dark` is on, the dark scheme is used so the
                        // empty state's surface matches what the guest will be
                        // themed with once it loads.
                        device_colors: if self.studio.preview_dark.get() {
                            ColorScheme::dark()
                        } else {
                            ColorScheme::light()
                        },
                        show_insets: self.studio.show_insets.get(),
                        platform,
                        state,
                        screen: self.studio.preview_screen.get(),
                        zoom: self.studio.preview_zoom.get(),
                        viewport: self.studio.preview_screen_size(),
                        // The metrics the previewed screen is told — built from
                        // the chosen device (and its rotation), not the platform
                        // default, so a "Tablet" frame tells the guest it is on
                        // a tablet rather than on a 393-wide phone.
                        metrics: self.studio.preview_view_metrics(),
                        landscape: self.studio.landscape.get(),
                        dark: self.studio.preview_dark.get(),
                        live_preview: self.studio.live_preview.get(),
                        live_state: self.studio.live_state.clone(),
                        live_source: self.studio.live_source(),
                        live_mounts: self.studio.live_mounts_for(&self.studio.live_source()),
                        accessibility: vieww_foundation::Accessibility {
                            text_scale: self.studio.preview_text_scale.get(),
                            reduce_motion: self.studio.preview_reduce_motion.get(),
                            ..vieww_foundation::Accessibility::default()
                        },
                    },
                ))
                .into(),
        );
        // The note is dismissible: see `caption`.
        if self.studio.preview_note.get() {
            column.push(hairline(chrome.line));
            column.push(caption(&chrome, theme.colors, &self.studio));
        }

        Container::new()
            .color(chrome.chrome_1)
            .width(width)
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(column),
            )
            .into()
    }
}

widget_node_from!(PreviewPane);

impl PreviewPane {
    /// The §8.2 refusal, and the way out of it.
    ///
    /// Everything here is deliberately about the *studio and the project*
    /// rather than about the buffer: nothing the user types can fix this, so a
    /// message that reads like a compile error would send them to the wrong
    /// place. The exact fingerprints are in the Output panel, where the load
    /// attempt logged them; repeating two sixteen-digit hex numbers in a
    /// four-hundred-pixel pane would push the button they actually need off
    /// the bottom of it.
    fn abi_refusal(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let studio = self.studio.clone();
        let stamp = self
            .studio
            .toolchain
            .as_ref()
            .as_ref()
            .map_or_else(|_| "unknown".to_string(), crate::compile::Toolchain::stamp);

        Container::new()
            .color(blend(colors.error, chrome.chrome_1, 0.12))
            .padding(EdgeInsets::symmetric(12.0, 10.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(6.0)
                    .children(children![
                        label_bold(
                            "This project cannot be previewed in-process",
                            12.0,
                            colors.on_surface
                        ),
                        label(
                            "The preview loads a library into this process, which is only sound \
                             when the project's vieww is the same compilation as the studio's. \
                             It is not, so nothing was mounted.",
                            11.0,
                            colors.on_surface_variant
                        ),
                        mono(&format!("studio  {stamp}"), 10.0, colors.outline),
                        Button::new("Build and run instead").on_pressed(move || {
                            // Not a second implementation of Build: the same
                            // command the menu and ⌘R run, through
                            // `Studio::run`, which is the property that stops
                            // this becoming a fourth place that knows how to
                            // start a build.
                            studio.run(Command::BuildAndRun);
                        }),
                    ]),
            )
            .into()
    }

    /// Platform picker on the left, Render button on the right.
    /// Preview | Inspector.
    ///
    /// A strip rather than a segmented control, matching the bottom panel's
    /// tabs: they are the same kind of thing — one pane showing one of several
    /// views of the same window — and the studio has enough control vocabulary.
    /// The Devices tab.
    ///
    /// The prototype drew Preview / Inspector / **Devices** and the studio had
    /// two tabs. The preview had exactly three sizes, one per platform, which
    /// answers "does this look right on a phone" and nothing else — not
    /// "does the safe area eat my header", not "does this reflow on a tablet",
    /// and not "is the small-phone case the one that overflows", which is the
    /// one that actually breaks.
    fn devices(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        use crate::state::Device;

        // Copied out of the borrow: the closures below outlive this method.
        let chrome = *chrome;

        let chosen = self.studio.device.get();
        let landscape = self.studio.landscape.get();

        let mut rows: Vec<WidgetNode> = vec![Container::new()
            .padding(EdgeInsets::symmetric(14.0, 10.0))
            .child(label(
                "The frame the preview is drawn at. Choosing one also tells the \
                 previewed screen which platform it is on, because a frame that is \
                 the right shape and the wrong platform is a worse answer than either.",
                11.0,
                colors.on_surface_variant,
            ))
            .into()];

        // The platform default, first and always: it is what the preview did
        // before this tab existed, and it has to stay one click away.
        {
            let studio = self.studio.clone();
            let selected = chosen.is_none();
            rows.push(device_row(
                "Platform default".to_owned(),
                self.studio.platform.get().describe(),
                selected,
                chrome,
                colors,
                move || studio.clear_device(),
            ));
        }

        for device in Device::ALL {
            let studio = self.studio.clone();
            let selected = chosen.as_ref().is_some_and(|current| *current == device);
            rows.push(device_row(
                device.name.to_owned(),
                device.size_label(),
                selected,
                chrome,
                colors,
                move || studio.choose_device(device),
            ));
        }

        {
            let studio = self.studio.clone();
            let enabled = chosen.is_some();
            rows.push(
                Container::new()
                    .padding(EdgeInsets::symmetric(14.0, 12.0))
                    .child(crate::ui::chrome::button(
                        "Rotate",
                        move || {
                            Container::new()
                                .height(28.0)
                                .radius(6.0)
                                .color(if landscape {
                                    colors.primary
                                } else {
                                    chrome.chrome_3
                                })
                                .alignment(Alignment::CENTER)
                                .child(label(
                                    if landscape { "Landscape" } else { "Portrait" },
                                    11.5,
                                    if landscape {
                                        colors.on_primary
                                    } else if enabled {
                                        colors.on_surface
                                    } else {
                                        colors.outline
                                    },
                                ))
                                .into()
                        },
                        move || studio.toggle_landscape(),
                    ))
                    .into(),
            );
        }

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    fn tabs(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let active = self.studio.right_tab.get();
        let tabs = crate::state::RightTab::ALL
            .into_iter()
            .map(|tab| {
                let studio = self.studio.clone();
                let selected = tab == active;
                let chrome = *chrome;
                crate::ui::chrome::sensed(
                    move |sense| {
                        Container::new()
                            // The right pane's tabs had no selected *fill* at
                            // all — only the label colour changed — so the strip
                            // read as three labels rather than as a chooser.
                            .color(if selected {
                                chrome.chrome_2
                            } else {
                                crate::ui::chrome::hovered(chrome.chrome_1, &chrome, sense)
                            })
                            .radius(5.0)
                            .padding(EdgeInsets::symmetric(11.0, 0.0))
                            .height(30.0)
                            .alignment(Alignment::CENTER)
                            .child(label(
                                tab.label(),
                                11.5,
                                if selected {
                                    colors.on_surface
                                } else {
                                    colors.on_surface_variant
                                },
                            ))
                            .into()
                    },
                    move || studio.right_tab.set(tab),
                )
                .into()
            })
            .collect::<Vec<WidgetNode>>();

        let studio = self.studio.clone();
        Container::new()
            .color(chrome.chrome_1)
            .padding(EdgeInsets::symmetric(4.0, 0.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children({
                        let mut row = tabs;
                        row.push(gap());
                        // Shutting the pane from inside it. The command is in
                        // the View menu too; this is the affordance somebody
                        // reaches for while looking at the thing they want gone.
                        row.push(
                            clickable(
                                move || {
                                    Container::new()
                                        .padding(EdgeInsets::symmetric(9.0, 0.0))
                                        .height(30.0)
                                        .alignment(Alignment::CENTER)
                                        .child(label("Hide", 11.0, colors.outline))
                                        .into()
                                },
                                move || studio.run(crate::command::Command::ToggleRightPane),
                            )
                            .into(),
                        );
                        row
                    }),
            )
            .into()
    }

    fn bar(
        &self,
        ctx: &BuildContext,
        chrome: &StudioTheme,
        colors: ColorScheme,
        platform: Platform,
        state: PreviewState,
        width: f32,
    ) -> WidgetNode {
        let selected = Platform::ALL
            .iter()
            .position(|p| *p == platform)
            .unwrap_or(0);

        let studio = self.studio.clone();
        // Switching platform is an edit, exactly as the plan says (§4.7): it
        // marks the buffer dirty and changes nothing on screen until Render.
        let picker = segmented(
            ctx,
            &Platform::ALL.map(Platform::label),
            selected,
            SegmentedColors {
                track: chrome.chrome_0,
                border: chrome.line,
                accent: chrome.accent,
                accent_far: chrome.accent_soft,
                on_accent: Color::WHITE,
                muted: colors.on_surface_variant,
            },
            move |index| {
                if let Some(next) = Platform::ALL.get(index) {
                    studio.platform.set(*next);
                    studio.mark_dirty();
                }
            },
        );

        let insets = self.studio.show_insets.clone();
        let inset_toggle = clickable(
            {
                let chrome = *chrome;
                let insets = insets.clone();
                move || {
                    let on = insets.get();
                    Container::new()
                        .color(if on { chrome.chrome_3 } else { chrome.chrome_1 })
                        .radius(5.0)
                        .height(24.0)
                        .padding(EdgeInsets::symmetric(8.0, 0.0))
                        .alignment(Alignment::CENTER)
                        .child(label_bold(
                            "Safe area",
                            11.0,
                            if on {
                                colors.primary
                            } else {
                                colors.on_surface_variant
                            },
                        ))
                        .into()
                }
            },
            move || insets.set(!insets.peek()),
        );

        // The dark-mode toggle, beside the safe-area one. Two reasons it lives
        // here rather than in the Settings view: it is a per-preview decision
        // rather than a studio-wide preference (the studio's own chrome can be
        // dark while the previewed screen is light, and vice versa), and the
        // answer "does this look right in dark mode" is a question somebody
        // asks while looking at the preview, not while looking at a settings
        // form.
        let dark = self.studio.preview_dark.clone();
        let dark_toggle = clickable(
            {
                let chrome = *chrome;
                let dark = dark.clone();
                move || {
                    let on = dark.get();
                    Container::new()
                        .color(if on { chrome.chrome_3 } else { chrome.chrome_1 })
                        .radius(5.0)
                        .height(24.0)
                        .padding(EdgeInsets::symmetric(8.0, 0.0))
                        .alignment(Alignment::CENTER)
                        .child(label_bold(
                            "Dark",
                            11.0,
                            if on {
                                colors.primary
                            } else {
                                colors.on_surface_variant
                            },
                        ))
                        .into()
                }
            },
            move || dark.set(!dark.peek()),
        );

        // The "Live" button: opens the caution dialog, which on acceptance
        // mounts the built-in demo app inside the device frame without
        // compiling. Lives beside the Render button because the two answer
        // different questions — Render answers "does this screen compile and
        // look right" and Live answers "does a complete app flow feel right".
        let live_studio = self.studio.clone();
        let live_active = self.studio.live_preview.get();
        let live_toggle = clickable(
            {
                let chrome = *chrome;
                move || {
                    // **The same on/off treatment as Safe area and Dark.**
                    //
                    // It used to be its own: a 15% tint of `primary` over the
                    // bar, with `on_primary` text on top. `on_primary` is the
                    // colour chosen to read against a *full-strength* primary
                    // fill, and against a 15% one it is near-invisible — on
                    // the dark theme the active Live button was grey text on
                    // grey, and the control that was switched *on* was the
                    // hardest of the three to read. Three adjacent toggles
                    // that disagree about what "on" looks like also make the
                    // odd one out read as disabled.
                    Container::new()
                        .color(if live_active {
                            chrome.chrome_3
                        } else {
                            chrome.chrome_1
                        })
                        .radius(5.0)
                        .height(24.0)
                        .padding(EdgeInsets::symmetric(8.0, 0.0))
                        .alignment(Alignment::CENTER)
                        .child(label_bold(
                            "Live",
                            11.0,
                            if live_active {
                                colors.primary
                            } else {
                                colors.on_surface_variant
                            },
                        ))
                        .into()
                }
            },
            move || {
                if live_active {
                    live_studio.exit_live_preview();
                } else {
                    live_studio.run(crate::command::Command::LivePreview);
                }
            },
        );

        let render = self.render_button(chrome, colors, state);

        // **One row while the pane can hold it, two when it cannot.**
        //
        // The controls are all fixed-width (see `COMPACT_BAR_BELOW`), so the
        // one-row layout has a hard floor: below it the row does not squeeze,
        // it overflows, and the overflow is the Render button — the control
        // the whole pane exists around — going under the pane's clip. The
        // two-row layout keeps every control in the pane at every width the
        // divider can reach, at the cost of ~30 points of stage height that
        // nobody studying a 300-point preview was using anyway.
        //
        // Row one carries the picker and the Render button — the primary
        // controls — so the action that starts a compile stays in the
        // top-right corner it occupies in the one-row layout, and a person
        // dragging the divider never watches the button they were aiming at
        // move to a different row mid-drag. Row two takes the toggles.
        if width < COMPACT_BAR_BELOW {
            return Container::new()
                .color(chrome.chrome_1)
                // No fixed height: two rows of 26 and 24 plus this padding
                // land at ~66, and pinning `BAR` here would squeeze them
                // into each other. The stage gives the height back; see the
                // comment above.
                .padding(EdgeInsets::symmetric(8.0, 5.0))
                .child(Flex::column().spacing(5.0).children(children![
                            Flex::row()
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .children(children![picker, gap(), render]),
                            Flex::row()
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .spacing(6.0)
                                .children(children![
                                    inset_toggle,
                                    dark_toggle,
                                    live_toggle,
                                ]),
                        ]))
                .into();
        }

        Container::new()
            .color(chrome.chrome_1)
            .height(BAR)
            .padding(EdgeInsets::symmetric(8.0, 0.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![
                        picker,
                        gap(),
                        inset_toggle,
                        dark_toggle,
                        live_toggle,
                        space(6.0),
                        render,
                    ]),
            )
            .into()
    }

    /// The one control that starts a compile. Disabled while one is running.
    ///
    /// The studio's accent rather than the previewed application's `primary`:
    /// this button belongs to the studio, and it changing colour because
    /// somebody retuned the theme of the screen they are previewing is a
    /// small, constant lie about what owns what.
    fn render_button(
        &self,
        chrome: &StudioTheme,
        colors: ColorScheme,
        state: PreviewState,
    ) -> WidgetNode {
        let busy = matches!(state, PreviewState::Compiling);
        let dirty = self.studio.dirty.get();
        let studio = self.studio.clone();

        let text = if busy { "Rendering…" } else { "Render" };
        // A busy button keeps its shape and loses its ramp — the accent
        // desaturated into the surface, so it reads as the same control
        // waiting rather than as a different control.
        let (near, far) = if busy {
            (
                blend(chrome.accent, colors.surface, 0.55),
                blend(chrome.accent_soft, colors.surface, 0.55),
            )
        } else {
            (chrome.accent, chrome.accent_soft)
        };
        let ramp = vieww_foundation::Gradient::linear(
            vieww_foundation::Offset::new(0.0, 0.0),
            vieww_foundation::Offset::new(1.0, 1.0),
        )
        .between(near, far);
        let dark = chrome.dark;
        // How far through the compile's own pulse we are. `Animated` with a
        // target that flips is the studio's only source of a repeating clock
        // that stops when the work does; a busy button that pulsed for ever
        // would keep the frame loop awake after the compile finished.
        let glow = if busy { 0.85 } else { 0.0 };

        Semantics::button(text.to_owned())
            .child(crate::ui::chrome::sensed(
                move |sense| {
                    let hover = sense.hover.clamp(0.0, 1.0);
                    let press = sense.press.clamp(0.0, 1.0);
                    let lift = if busy {
                        0.0
                    } else {
                        press.mul_add(-1.4, hover)
                    };
                    let scale = press.mul_add(-0.03, 1.0);
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "lift is bounded to -0.4..=1.0 and the sum is clamped"
                    )]
                    let alpha = lift
                        .mul_add(70.0, if dark { 100.0 } else { 50.0 })
                        .clamp(0.0, 220.0) as u8;

                    let mut row = vec![label_bold(text, 12.0, Color::WHITE).into()];
                    if dirty && !busy {
                        row.push(space(7.0));
                        row.push(
                            Container::new()
                                .color(Color::WHITE)
                                .radius(3.5)
                                .size(7.0, 7.0)
                                .into(),
                        );
                    }

                    let body = Container::new()
                        .gradient(ramp)
                        .color(near)
                        .radius(6.0)
                        .height(24.0)
                        .padding(EdgeInsets::symmetric(12.0, 0.0))
                        .alignment(Alignment::CENTER)
                        .shadow(vieww_foundation::Shadow::new(
                            Color::rgba(0, 0, 0, alpha),
                            vieww_foundation::Offset::new(0.0, lift.mul_add(2.0, 1.5)),
                            lift.mul_add(5.0, 4.0),
                        ))
                        .child(
                            Flex::row()
                                .main_axis_size(MainAxisSize::Min)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .children(row),
                        );

                    Stack::new()
                        .children(children![
                            crate::ui::chrome::behind(crate::ui::chrome::GlowUnder {
                                color: far,
                                strength: hover.mul_add(0.55, glow),
                                radius: 6.0,
                            }),
                            vieww_widget::Transformed::new(vieww_foundation::Transform::scale(
                                scale, scale
                            ))
                            .child(body),
                        ])
                        .into()
                },
                move || {
                    if busy {
                        return;
                    }
                    studio.render();
                },
            ))
            .into()
    }
}

/// The stage the device stands on, and the fit that keeps it whole.
///
/// The frame is scaled to the space the pane actually has. Before this it was
/// drawn at a fixed 560 tall and simply *cut off* when the window was short —
/// which is the one thing a device frame must not do: a phone with its bottom
/// missing is not a smaller phone, it is a wrong one, and the screen inside it
/// is laid out against a size the device never has.
/// Everything the stage needs to draw one frame of the previewed screen.
///
/// A struct rather than eight parameters: the zoom was the eighth, and a
/// function whose call site is eight positional values is one where swapping
/// two `bool`s or two `ColorScheme`s compiles and is wrong.
#[derive(Debug)]
pub struct Stage {
    pub colors: ColorScheme,
    /// The scheme the *previewed app* is themed with, which has nothing to do
    /// with the studio's own chrome. Conflating them is how a preview stops
    /// being a preview.
    pub device_colors: ColorScheme,
    pub show_insets: bool,
    pub platform: Platform,
    pub state: PreviewState,
    pub screen: Option<crate::loaded::Preview>,
    /// A zoom the user asked for, or `None` to fit the pane.
    pub zoom: Option<f32>,
    /// What the *previewed screen* is told about text scale and motion. See
    /// `device` for why these had no reader at all until now.
    pub accessibility: vieww_foundation::Accessibility,
    /// The size the device frame is drawn at, from the Devices tab or from the
    /// platform's own default. Passed in rather than derived here, because the
    /// stage is a pure function of its spec and reading a signal inside it
    /// would make it one that rebuilds for reasons it does not declare.
    pub viewport: (f32, f32),
    /// What the previewed screen is *told* the surface is like — size, DPR,
    /// insets. Built by [`Studio::preview_view_metrics`], which honours the
    /// Devices tab and rotation rather than the platform default, and passed
    /// through the stage rather than re-derived in `device()` so the one
    /// source of truth is the one the studio reads from.
    ///
    /// [`Studio::preview_view_metrics`]: crate::state::Studio::preview_view_metrics
    pub metrics: vieww_foundation::ViewMetrics,
    /// Whether the frame is on its side.
    ///
    /// Carried rather than inferred from `viewport.0 > viewport.1`, because a
    /// Desktop frame is wider than it is tall the right way up. See
    /// `state::Device::landscape`.
    pub landscape: bool,
    /// Whether the previewed screen is themed dark.
    ///
    /// `ThemeData::adaptive(platform, dark)`'s `dark` argument. Used to be
    /// hardcoded to `false`; now read from [`Studio::preview_dark`] so a
    /// toolbar toggle answers "does this look right in dark mode".
    ///
    /// [`Studio::preview_dark`]: crate::state::Studio::preview_dark
    pub dark: bool,
    /// Whether the live demo is showing instead of the compiled screen.
    pub live_preview: bool,
    /// The live demo's state — route, counter, toggle, selected. Passed through
    /// so `device()` can build `LiveApp` when `live_preview` is true.
    pub live_state: crate::live::LiveState,
    /// The parsed `live.rs`, or why there is none. Read per frame by the
    /// studio so the preview follows the file as it is typed.
    pub live_source: crate::live::LiveSource,
    /// What each `mount` in that file resolves to.
    pub live_mounts: std::collections::HashMap<String, crate::loaded::Preview>,
}

fn stage(chrome: &StudioTheme, scroll: vieww_element::ScrollController, spec: Stage) -> WidgetNode {
    let chrome = *chrome;
    let Stage {
        colors,
        device_colors,
        show_insets,
        platform,
        state,
        screen,
        zoom,
        viewport,
        accessibility,
        metrics,
        landscape,
        dark,
        live_preview,
        live_state,
        live_source,
        live_mounts,
    } = spec;

    LayoutBuilder::new(move |constraints| {
        // **The fit is the default and the override is a multiplier of it, not
        // a replacement.** A zoom expressed as an absolute scale would mean
        // 100% is a different size in a wide pane than in a narrow one, and
        // "fit" would be a number that silently stopped being the fit the
        // moment the divider moved.
        // **The fit has a floor, and below it the stage scrolls.** Shrinking
        // the frame to whatever the pane has left is right until the number
        // gets small: at 20% — a 1366x679 window with the bottom panel open —
        // every dimension is correct and nothing inside the phone can be read,
        // which is a preview that has stopped previewing. `MIN_SCALE` is where
        // that stops, and what will not fit under it is scrolled to rather than
        // scaled away.
        let fitted = stage_scale(constraints, platform, viewport);
        let scale = zoom.map_or(fitted, |zoom| fitted * zoom);
        // Whether the frame at that scale is taller than the room there is.
        // Measured against the same padding `fit_scale_of` subtracts, so the
        // two agree about what "fits" means.
        let needed = viewport.1 * scale + STAGE_PADDING * 2.0;
        let overflowing =
            constraints.max_height.is_finite() && needed > constraints.max_height + 0.5;
        // **Stale means "older than the buffer", which needs there to be
        // something.** `PreviewState::Failed` alone is not enough: a studio
        // whose *first* Render failed is in `Failed` with no screen behind it,
        // and dimming an empty frame under a badge reading "showing last
        // successful render" claims a render that never happened. The badge is
        // the pane's one promise (plan §2.2) and it has to be true.
        //
        // **And it is not true of the Live Preview at all.** When `live` is on,
        // the frame below is drawn from `live.rs` — parsed, never compiled —
        // so `PreviewState` describes something the user is not looking at.
        // The pane shipped in exactly that state: a live flow, dimmed to 55%,
        // under a red badge reading "showing last successful render", because
        // a Render *before* the user pressed Live had failed and left the
        // compiled pipeline in `Failed`. Every word of that was wrong about
        // the picture it was over. The live path has its own caption, so this
        // one steps out of the way.
        let stale = state.is_stale() && screen.is_some() && !live_preview;

        let frame: WidgetNode = if stale {
            // Dimmed, and badged: the frame on screen is older than the buffer,
            // and the two together say so twice — one for someone reading the
            // badge, one for someone glancing at the pane.
            Opacity::new(0.55)
                .child(device(Frame {
                    colors: device_colors,
                    show_insets,
                    platform,
                    state,
                    screen: screen.as_ref(),
                    scale,
                    viewport,
                    accessibility,
                    metrics,
                    landscape,
                    dark,
                    live_preview,
                    live_state: live_state.clone(),
                    live_source: live_source.clone(),
                    live_mounts: live_mounts.clone(),
                    clipped: overflowing,
                }))
                .into()
        } else {
            device(Frame {
                colors: device_colors,
                show_insets,
                platform,
                state,
                screen: screen.as_ref(),
                scale,
                viewport,
                accessibility,
                metrics,
                landscape,
                dark,
                live_preview,
                live_state: live_state.clone(),
                live_source: live_source.clone(),
                live_mounts: live_mounts.clone(),
                clipped: overflowing,
            })
        };

        let stand: WidgetNode = Container::new()
            .color(chrome.chrome_1)
            .alignment(Alignment::CENTER)
            .padding(EdgeInsets::all(STAGE_PADDING))
            .child(frame)
            .into();

        // Clipped *and* scrollable, for the same reason the editor's code pane
        // is both: the clip is what stops a frame taller than the stage
        // painting over the caption under it, and the scroll is what makes the
        // bottom of the phone reachable. A clip on its own would be the cut-off
        // frame this whole function was written to stop drawing.
        let mut layers: Vec<WidgetNode> = vec![if overflowing {
            Clip::rect()
                .child(
                    Scrollable::vertical(scroll.offset())
                        .key("preview-stage")
                        .on_drag(scroll.on_drag())
                        .on_drag_end(scroll.on_drag_end())
                        .on_extents(scroll.on_extents())
                        .child(stand),
                )
                .into()
        } else {
            stand
        }];

        if stale {
            layers.push(
                Positioned::new()
                    .top(10.0)
                    .child(
                        Container::new()
                            .color(blend(colors.error, chrome.chrome_3, 0.25))
                            .radius(999.0)
                            .height(20.0)
                            .padding(EdgeInsets::symmetric(9.0, 0.0))
                            .alignment(Alignment::CENTER)
                            .child(label(
                                "showing last successful render",
                                10.5,
                                colors.on_surface,
                            )),
                    )
                    .into(),
            );
        }

        // **The Live Preview says what it is, in the frame.**
        //
        // The single most-reported confusion about this studio, and it is an
        // honest one: a new project's `live.rs` sketches three screens, so the
        // pane on the right shows an inbox, a detail view and a settings form
        // — and the *application* that same project builds is one screen
        // saying "Hello from vieww". Nothing on screen connected the two, so
        // the reasonable conclusion was that the build had dropped two thirds
        // of the app.
        //
        // Both halves were working exactly as designed. The design was simply
        // never stated at the one moment somebody needed it: while looking at
        // the three screens. A badge in the frame is that statement, and it
        // sits where the stale badge sits because the two can never both
        // apply — see `stale` above.
        //
        // Mutually exclusive with the stale badge by construction, so the
        // frame never carries two.
        if live_preview {
            layers.push(
                Positioned::new()
                    .top(10.0)
                    .child(
                        Container::new()
                            .color(blend(colors.primary, chrome.chrome_3, 0.3))
                            .radius(999.0)
                            .height(20.0)
                            .padding(EdgeInsets::symmetric(9.0, 0.0))
                            .alignment(Alignment::CENTER)
                            .child(label(
                                "live.rs sketch · not the built app",
                                10.5,
                                colors.on_surface,
                            )),
                    )
                    .into(),
            );
        }

        // The zoom readout belongs here rather than on the toolbar, because
        // here is the only place the number is known: it comes out of the
        // constraints this builder was handed.
        layers.push(
            Positioned::new()
                .right(10.0)
                .bottom(8.0)
                .child(
                    Container::new()
                        .color(chrome.chrome_3)
                        .radius(4.0)
                        .height(18.0)
                        .padding(EdgeInsets::symmetric(6.0, 0.0))
                        .alignment(Alignment::CENTER)
                        .child(label(
                            // Says so when there is more frame than stage.
                            // A percentage on its own would leave somebody
                            // looking at a phone with no bottom and no reason
                            // to try dragging it.
                            &if overflowing {
                                format!("{}% · scroll", scale_percent(scale))
                            } else {
                                format!("{}%", scale_percent(scale))
                            },
                            10.0,
                            colors.on_surface_variant,
                        )),
                )
                .into(),
        );

        Stack::new()
            .alignment(Alignment::TOP_CENTER)
            .children(layers)
            .into()
    })
    .into()
}

/// The margin between the frame and the edge of the stage.
const STAGE_PADDING: f32 = 20.0;

/// The smallest scale a device frame is still worth drawing at.
///
/// # Why a floor at all
///
/// [`fit_scale_of`] answers "how small does this have to be to fit?", and on a
/// short pane the honest answer runs away: a 1366x679 window with the bottom
/// panel open leaves the stage about 170 points, which is 20% of an iPhone —
/// a frame with every dimension right and nothing legible inside it. Half size
/// is the last scale at which 11-point body text in the previewed screen is
/// still text rather than texture, so it is where the shrinking stops and
/// [`Scrollable`] takes over.
pub const MIN_SCALE: f32 = 0.5;

/// The scale the stage actually draws at: the fit, floored, but never past what
/// the *width* or the frame's own natural size allow.
///
/// Both caps are what keep the floor from trading one bug for another. Floored
/// past the width, a tablet in a narrow pane would hang off both sides of a
/// stage that scrolls in only one direction; floored past `natural`, a desktop
/// frame — whose natural scale is well under a half — would be *blown up* by a
/// short window, which is the opposite of the complaint.
#[must_use]
pub fn stage_scale(constraints: Constraints, platform: Platform, screen: (f32, f32)) -> f32 {
    let (device_width, device_height) = screen;
    let natural = frame_height(platform) / device_height;

    let room_w = constraints.max_width - STAGE_PADDING * 2.0;
    let by_width = if room_w.is_finite() {
        (room_w / device_width).max(0.0)
    } else {
        f32::INFINITY
    };

    let floor = MIN_SCALE.min(by_width).min(natural);
    fit_scale_of(constraints, platform, screen).max(floor)
}

/// The largest scale at which the whole device still fits the stage.
///
/// Capped at 1.0 deliberately: a device is drawn at most at `frame_height`,
/// never blown up to fill a tall pane, because a phone rendered larger than a
/// phone stops being a size reference.
#[must_use]
pub fn fit_scale(constraints: Constraints, platform: Platform) -> f32 {
    fit_scale_of(constraints, platform, platform.screen())
}

/// The same, for an explicitly chosen device size.
///
/// Split out when the Devices tab arrived: the frame is no longer always the
/// platform's own default, and a `fit_scale` that still assumed it would draw
/// a tablet at a phone's scale.
#[must_use]
pub fn fit_scale_of(constraints: Constraints, platform: Platform, screen: (f32, f32)) -> f32 {
    let (device_width, device_height) = screen;
    let natural = frame_height(platform) / device_height;

    let room_h = constraints.max_height - STAGE_PADDING * 2.0;
    let room_w = constraints.max_width - STAGE_PADDING * 2.0;
    if !room_h.is_finite() || !room_w.is_finite() {
        return natural;
    }

    let by_height = (room_h / device_height).max(0.0);
    let by_width = (room_w / device_width).max(0.0);
    natural.min(by_height).min(by_width)
}

/// A scale as the percentage the readout shows.
#[must_use]
pub fn scale_percent(scale: f32) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a bounded positive ratio, times 100"
    )]
    {
        (scale * 100.0).round() as u32
    }
}

/// How tall the frame is drawn, in logical pixels.
///
/// The one number per platform that the frame's size is derived from; its width
/// comes from the device's aspect, and the readout on the bar from this over
/// the device's own height. Three consumers, one table.
#[must_use]
const fn frame_height(platform: Platform) -> f32 {
    match platform {
        Platform::Ios | Platform::Android => 560.0,
        Platform::Desktop => 300.0,
    }
}

/// The bezel, the cutouts, and inside them the simulated screen — at device
/// scale, then scaled as one picture.
///
/// # Why the whole frame is laid out at 393×852 and then shrunk
///
/// The first version drew a smaller frame and let the previewed screen lay
/// itself out inside it. That is not a preview: a screen given 258 points of
/// width wraps its headline where a 393-point screen would not, so what the
/// pane showed was a phone-shaped window, not a phone. Everything below is in
/// **device points** — the bezel, the notch, the safe-area insets, the text
/// inside — and [`Fitted`] scales the finished picture down to whatever room
/// the pane has. That is also exactly what M4's texture will do, which is the
/// point: the geometry does not change when the texture arrives.
/// Everything the device frame needs, in one place.
///
/// A struct rather than eight parameters — the same shape `Stage` above takes,
/// and for the same reason: this grew to eight when the Devices tab and the
/// preview's own accessibility arrived, and eight positional arguments of which
/// three are `(f32, f32)`-shaped is a call nobody can read.
#[derive(Debug)]
pub struct Frame<'a> {
    pub colors: ColorScheme,
    pub show_insets: bool,
    pub platform: Platform,
    pub state: PreviewState,
    pub screen: Option<&'a crate::loaded::Preview>,
    pub scale: f32,
    /// The size the frame is drawn at, from the Devices tab.
    pub viewport: (f32, f32),
    /// What the previewed screen is told about text scale and motion.
    pub accessibility: vieww_foundation::Accessibility,
    /// The metrics the previewed screen is *told* — size, DPR, safe area.
    /// Built by [`Studio::preview_view_metrics`], so it follows the chosen
    /// device (and its rotation) rather than the platform default.
    ///
    /// [`Studio::preview_view_metrics`]: crate::state::Studio::preview_view_metrics
    pub metrics: vieww_foundation::ViewMetrics,
    /// Whether the frame is on its side. See `Stage::landscape`.
    pub landscape: bool,
    /// Whether the previewed screen is themed dark.
    pub dark: bool,
    /// Whether the live demo is showing instead of the compiled screen.
    pub live_preview: bool,
    /// The live demo's state. Used to build `LiveApp` when `live_preview` is
    /// true.
    pub live_state: crate::live::LiveState,
    /// The parsed `live.rs`, or why there is none. Read per frame by the
    /// studio so the preview follows the file as it is typed.
    pub live_source: crate::live::LiveSource,
    /// What each `mount` in that file resolves to.
    pub live_mounts: std::collections::HashMap<String, crate::loaded::Preview>,
    /// Whether the stage is showing only part of this frame.
    ///
    /// Only the empty state reads it, and only to move its own sentence to the
    /// top of the screen: centred in a phone whose lower half is below the fold
    /// puts "Press Render" on the clip edge, which is where a message is least
    /// readable and most easily mistaken for a drawing bug.
    pub clipped: bool,
}

fn device(frame: Frame<'_>) -> WidgetNode {
    let Frame {
        colors,
        show_insets,
        platform,
        state,
        screen,
        scale,
        viewport,
        accessibility,
        metrics,
        landscape,
        dark,
        live_preview,
        live_state,
        live_source,
        live_mounts,
        clipped,
    } = frame;
    let (device_width, device_height) = viewport;

    // Device points, so a 55-point corner really is an iPhone's corner.
    let (corner, bezel): (f32, f32) = match platform {
        Platform::Ios => (56.0, 14.0),
        Platform::Android => (34.0, 12.0),
        Platform::Desktop => (12.0, 0.0),
    };

    // **One simulation, not two.** The frame used to reserve opaque bars at the
    // top and bottom *and* publish the same insets in `ViewMetrics`, so a guest
    // using `SafeArea` inset twice — once because the bar pushed its layout
    // down, once because `SafeArea::resolve` read the inset out of `ViewMetrics`
    // and applied it as padding. The bar was a *visualisation* of the unsafe
    // region; the `ViewMetrics` was the *simulation*. The simulation stays;
    // the visualisation becomes a tinted overlay over the same region, drawn
    // on top of the body rather than alongside it, so it costs nothing in
    // layout and `SafeArea` insets exactly once.
    //
    // The toolbar toggle now controls whether the simulation publishes insets
    // at all: when it is off the preview is full-bleed and a guest `SafeArea`
    // is a no-op, which is what a designer iterating on a layout wants from a
    // "show me the safe area" toggle — *change the space*, not just the tint.
    //
    // **And the metrics come from the chosen device, not the platform
    // default.** `metrics` was built by `Studio::preview_view_metrics`, so a
    // "Tablet" frame publishes a 834×1194 size with the right DPR rather than
    // the 393×852 of an iPhone. Rotation is in there too, because the device
    // is rotated before its metrics are taken.
    let view_metrics = simulated_metrics(metrics, show_insets);

    // Tinted when the overlay is on, so what the app may not draw into is
    // visible without opening a second window to compare against. Drawn as an
    // overlay rather than as Flex rows, so the body underneath is the same
    // rectangle whether the overlay is on or off — `SafeArea` insets the body
    // by `view_metrics.safe_area`, and that is the one source of the space.
    let inset_color = blend(colors.primary, colors.surface, 0.22);

    // The compiled screen, if one has ever loaded. It survives a failed
    // compile deliberately — plan §2.2, the pane never blanks — which is why
    // this reads the screen rather than the state.
    // **The previewed screen's own world.** It is told the device's metrics
    // and the device's platform, and themed by `ThemeData::adaptive`, so
    // `SafeArea` insets by an iPhone's 47 points and anything branching on
    // `TargetPlatform` takes the branch it would take on the device. This is
    // the whole of M7: the picker changes what the screen is *told*, not how
    // its widgets are painted.
    //
    // **A per-preview `Overlay` host sits between the frame and the guest.**
    // The studio's shell has its own `Overlay` at the root of the whole
    // window, and without this one a guest `Dialog`, `Dropdown` or `Menu`
    // would publish to *that* — painting over the whole studio at studio
    // scale, escaping the device frame. Wrapping the guest in an `Overlay`
    // here means `Overlay::of(ctx)` inside the guest finds this one first,
    // and entries are measured against the device's size and clipped to the
    // frame. The studio's shell overlay remains available to the chrome
    // above this point, so the studio's own menus, dialogs and tooltips
    // continue to work as they did.
    //
    // **The live preview bypasses compilation entirely.** When `live_preview`
    // is true, the body is the built-in `LiveApp` demo — a widget tree the
    // studio builds directly, against its own `Runtime`, with no `rustc`,
    // `cdylib` or ABI fingerprint check. The demo's signals (route, counter,
    // toggle) live on the studio's runtime, so a tap on a button changes the
    // route and the tree rebuilds in the same frame. This is the "hack" that
    // shows a complete app UX without building: the demo runs in-process, not
    // as a loaded library.
    let body: WidgetNode = if live_preview {
        // **`view_metrics`, the same as a guest screen.**
        //
        // This branch used to pass the *unmodified* `metrics` — the device's
        // real insets whatever the toggle said — because a live demo whose
        // titles ran under the dynamic island is a worked example of the
        // mistake, and that was reported from a screenshot. The reasoning
        // ended: "the frame paints the island at every setting, so this is
        // also the only reading under which what is drawn and what is
        // published agree."
        //
        // Which is exactly right, and identifies the wrong half as the one to
        // change. A guest screen got the toggle's zeroed insets and the island
        // painted over it anyway, so **every new project's first render showed
        // its own heading half-hidden behind a black pill** — the same defect,
        // on the screen that actually matters, left in place while the demo
        // was given a private exemption from it.
        //
        // The island is now drawn only when the insets are published (see
        // `cutouts` at the call site below), so the two agree at both settings
        // and neither path needs an exemption.
        Inherited::new(
            view_metrics,
            Inherited::new(
                accessibility,
                Theme::new(ThemeData::adaptive(platform.target(), dark)).child(
                    Overlay::new().child(crate::live::LiveApp {
                        state: live_state,
                        source: live_source,
                        mounts: live_mounts,
                    }),
                ),
            ),
        )
        .into()
    } else {
        match screen {
            Some(preview) => Inherited::new(
                view_metrics,
                Inherited::new(
                    accessibility,
                    Theme::new(ThemeData::adaptive(platform.target(), dark)).child(
                        Overlay::new().child(GuestRoot {
                            child: preview.node(),
                        }),
                    ),
                ),
            )
            .into(),
            None => empty_screen(colors, state, scale, clipped),
        }
    };

    // The body fills the whole screen; the safe-area tint, if it is on at all,
    // sits over the unsafe edges of it as overlays. This is the one place the
    // unsafe region is visible.
    //
    // **Drawn from `view_metrics.safe_area`, which is the only place its size
    // is decided.** The comment here claimed exactly that while the code read
    // `platform.insets()` — the *portrait* pair — and painted a top and a
    // bottom band and nothing else. Turn the frame on its side and the guest
    // was inset on its left and right (correctly, once `Device` carried its
    // orientation) while the tint went on drawing bands across the top and the
    // bottom: the picture of the unsafe region and the actual unsafe region on
    // opposite pairs of edges, in the one mode a person opens specifically to
    // check them.
    //
    // Reading the published insets makes the two agree by construction, and
    // costs a loop over four edges instead of a special case per orientation.
    let mut screen_layers: Vec<WidgetNode> = vec![Clip::rect().child(body).into()];
    if show_insets {
        let safe = view_metrics.safe_area;
        for (amount, place) in [
            (safe.top, Edge::Top),
            (safe.bottom, Edge::Bottom),
            (safe.left, Edge::Left),
            (safe.right, Edge::Right),
        ] {
            if amount <= 0.0 {
                continue;
            }
            let (positioned, width, height) = match place {
                Edge::Top => (
                    Positioned::new().top(0.0).height(amount),
                    device_width,
                    amount,
                ),
                Edge::Bottom => (
                    Positioned::new().bottom(0.0).height(amount),
                    device_width,
                    amount,
                ),
                Edge::Left => (
                    Positioned::new().left(0.0).width(amount),
                    amount,
                    device_height,
                ),
                Edge::Right => (
                    Positioned::new().right(0.0).width(amount),
                    amount,
                    device_height,
                ),
            };
            screen_layers.push(
                positioned
                    .child(
                        Container::new()
                            .color(inset_color)
                            .width(width)
                            .height(height),
                    )
                    .into(),
            );
        }
    }

    let screen = Stack::new()
        .alignment(Alignment::TOP_CENTER)
        .children(screen_layers);

    let inner_corner = (corner - bezel).max(0.0);
    let mut layers: Vec<WidgetNode> = vec![Container::new()
        .color(colors.surface)
        .radius(inner_corner)
        .child(Clip::rounded(inner_corner).child(screen))
        .into()];
    // The size the decorations centre against. The frame's padding is the
    // bezel, so the `Stack` the cutouts go into is the screen minus a bezel on
    // each edge — which is exactly the surface the published insets are
    // measured against, and the surface a real device's cutout is centred on.
    let screen_size = (device_width - bezel * 2.0, device_height - bezel * 2.0);

    // **The island and the home bar are part of the same simulation as the
    // insets, so they share its switch.**
    //
    // They used to be drawn unconditionally. "Safe area" off means the preview
    // publishes no insets and the screen is deliberately full-bleed — and the
    // frame then painted a 125×36 black pill over the top of that screen
    // anyway. On the studio's own project template, whose first line is a
    // heading in the top-left corner, the result was `Hello fr██████w`: the
    // studio obscuring the user's content with its own decoration, in the very
    // first frame of the very first project, on the default setting.
    //
    // Drawing an obstruction while telling the guest there is none is the
    // contradiction. Off means there is no notch — nothing claims the space
    // and nothing covers it. On means there is one, and `SafeArea` has already
    // reserved room so nothing lands under it.
    if show_insets {
        layers.extend(cutouts(colors, platform, landscape, screen_size));
    }

    // **The bezel is drawn, not filled.**
    //
    // It was one flat `#0B0B0D` rounded rectangle, which is a silhouette of a
    // phone rather than a phone: no thickness at the edge, no light on the
    // top, nothing under it holding it off the stage. `DeviceShell` is the
    // same silhouette with the three things a physical object has — a shadow
    // beneath it, a ramp across the metal, and a catch of light along the rim
    // — plus the side buttons, which are what make it read as a *device* at a
    // glance rather than as a rounded rectangle.
    let frame = Stack::new()
        .alignment(Alignment::TOP_CENTER)
        .children(children![
            Painting::sized(
                vieww_foundation::Size::new(device_width, device_height),
                DeviceShell {
                    corner,
                    bezel,
                    buttons: bezel > 0.0,
                },
            ),
            Container::new()
                .size(device_width, device_height)
                .padding(EdgeInsets::all(bezel))
                .child(
                    Stack::new()
                        .alignment(Alignment::TOP_CENTER)
                        .children(layers),
                ),
        ]);

    // The box the frame occupies in the pane, and the scale that gets it there.
    // `Fitted` measures its child unbounded — which is what hands the frame its
    // full device size — and scales the result into this box.
    Container::new()
        .size(device_width * scale, device_height * scale)
        .child(Fitted::new().child(frame))
        .into()
}

/// What the previewed screen is told about the device, given the toggle.
///
/// **The one rule the "Safe area" control encodes**, lifted out of `device` so
/// it can be asserted directly rather than inferred from where a heading landed
/// in a window. It is one line of arithmetic and the reason it is worth a name
/// is that a second place has to agree with it: `cutouts` may only draw the
/// island and the home bar when this publishes the space they occupy. An
/// obstruction drawn over a screen that was told there is none is the defect
/// both halves exist to prevent, and it is reachable by getting either half
/// wrong on its own.
#[must_use]
pub fn simulated_metrics(
    metrics: vieww_foundation::ViewMetrics,
    show_insets: bool,
) -> vieww_foundation::ViewMetrics {
    if show_insets {
        return metrics;
    }
    vieww_foundation::ViewMetrics {
        safe_area: vieww_foundation::EdgeInsets::ZERO,
        ..metrics
    }
}

/// What the display itself puts over the app: notch, punch-hole, home bar.
///
/// In device points, like everything else inside the frame — an iPhone's island
/// really is about 125×36 at 393 wide, so it stays right at every scale.
///
/// # Orientation
///
/// These were positioned against the top and the bottom unconditionally, so a
/// rotated frame kept its island across the top edge and its home indicator
/// along the bottom — which is a phone that has been turned while its hardware
/// stayed where it was. Turning a phone moves the cutout to a **side**; the
/// indicator stays on what is now the bottom, and shortens.
///
/// # Centre on the edge, explicitly
///
/// Each decoration used to pin **one** edge and let the stack's `TOP_CENTER`
/// alignment place the other axis. An axis a `Positioned` pins nothing on is
/// placed by the stack's alignment, and `TOP_CENTER`'s vertical half is *top* —
/// so a landscape island pinned `left` floated to the top of the side edge
/// instead of its middle, and the same for Android's punch-hole. Reported from
/// a landscape render: "the island icons of iPhone and Android are not
/// centered".
///
/// Every decoration now pins **both** of its axes against `screen_size` — the
/// (width, height) of the surface inside the bezel, the same surface the
/// published insets are cut from. Centring becomes arithmetic nobody else
/// owns, the picture cannot drift from the insets it must sit inside, and the
/// result is the same at every `Stack` alignment and at every device preset:
/// a cutout centred on the edge a real device carries it on, always inside
/// the safe area the simulation publishes for it.
fn cutouts(
    colors: ColorScheme,
    platform: Platform,
    landscape: bool,
    screen_size: (f32, f32),
) -> Vec<WidgetNode> {
    let ink = Color::hex(0x0B_0B0D);
    // Drawn against the screen, so on a light app screen these are dark. The
    // home indicator is a contrast of the screen's own ink rather than a fixed
    // white, which is what a real device does too.
    let indicator = blend(colors.on_surface, colors.surface, 0.55);

    let (screen_width, screen_height) = screen_size;
    // The two placement helpers. `centred` is the whole of this fix: an extent
    // along an edge, positioned so its midpoint is the edge's midpoint.
    let centred = |extent: f32, along: f32| (along - extent) / 2.0;

    match platform {
        Platform::Ios if landscape => {
            // The island on the left edge, upright device turned anticlockwise
            // — the orientation a phone is held in for video, and the one iOS
            // treats as the default landscape. Vertically centred, the way the
            // hardware sits in the frame; the published 59-point side inset
            // covers the 11 + 36 it spans with room to spare.
            vec![
                Positioned::new()
                    .left(11.0)
                    .top(centred(125.0, screen_height))
                    .child(Container::new().color(ink).radius(18.0).size(36.0, 125.0))
                    .into(),
                Positioned::new()
                    .bottom(7.0)
                    .left(centred(200.0, screen_width))
                    .child(
                        Container::new()
                            .color(indicator)
                            .radius(3.0)
                            .size(200.0, 5.0),
                    )
                    .into(),
            ]
        }
        Platform::Ios => {
            // The island floats: on the device it sits about eleven points
            // below the top of the display, and drawn flush it reads as a bite
            // out of the bezel rather than as a cutout in the screen.
            vec![
                Positioned::new()
                    .top(11.0)
                    .left(centred(125.0, screen_width))
                    .child(Container::new().color(ink).radius(18.0).size(125.0, 36.0))
                    .into(),
                Positioned::new()
                    .bottom(9.0)
                    .left(centred(140.0, screen_width))
                    .child(
                        Container::new()
                            .color(indicator)
                            .radius(3.0)
                            .size(140.0, 5.0),
                    )
                    .into(),
            ]
        }
        Platform::Android if landscape => vec![
            // The punch-hole is a dot in the middle of a long edge, not a
            // bookmark at its end.
            Positioned::new()
                .left(12.0)
                .top(centred(14.0, screen_height))
                .child(Container::new().color(ink).radius(7.0).size(14.0, 14.0))
                .into(),
            Positioned::new()
                .bottom(9.0)
                .left(centred(160.0, screen_width))
                .child(
                    Container::new()
                        .color(indicator)
                        .radius(2.0)
                        .size(160.0, 4.0),
                )
                .into(),
        ],
        Platform::Android => vec![
            Positioned::new()
                .top(12.0)
                .left(centred(14.0, screen_width))
                .child(Container::new().color(ink).radius(7.0).size(14.0, 14.0))
                .into(),
            Positioned::new()
                .bottom(9.0)
                .left(centred(108.0, screen_width))
                .child(
                    Container::new()
                        .color(indicator)
                        .radius(2.0)
                        .size(108.0, 4.0),
                )
                .into(),
        ],
        Platform::Desktop => Vec::new(),
    }
}

/// Which edge of the frame a safe-area band is painted along.
#[derive(Clone, Copy)]
enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// The frame before the first render, and while one is running.
///
/// `scale` is the stage's, and everything here is divided by it: the whole
/// frame is laid out in device points and then shrunk as one picture, so a
/// 14-point heading inside it lands on screen at 14 times the scale. That is
/// exactly right for the previewed app — a phone's text is a phone's text — and
/// exactly wrong for this, which is the studio's own instruction and should be
/// the same size as the caption under the pane whatever the frame is doing.
/// Dividing first and letting [`Fitted`] multiply back is what makes it so.
///
/// Bounded either way. The compensation cannot run past twice, because a
/// sentence enlarged without limit inside a 393-point screen wraps into a
/// column of single words; and it never shrinks the text below its own size
/// when the user has zoomed *in*, because a zoomed frame is still a frame and
/// the instruction is still the studio's.
fn empty_screen(colors: ColorScheme, state: PreviewState, scale: f32, clipped: bool) -> WidgetNode {
    let compensation = if scale > 0.0 {
        (1.0 / scale).clamp(0.25, 2.0)
    } else {
        1.0
    };
    let title_size = 14.0 * compensation;
    let note_size = 11.5 * compensation;
    let (title, note) = match state {
        PreviewState::Compiling => ("Compiling…", "rustc is building the buffer."),
        _ => (
            "No render yet",
            "Press Render to compile the buffer and mount it here.",
        ),
    };

    Container::new()
        .color(colors.surface)
        .padding(EdgeInsets::all(20.0 * compensation))
        .alignment(if clipped {
            Alignment::TOP_CENTER
        } else {
            Alignment::CENTER
        })
        .child(
            Flex::column()
                .main_axis_size(MainAxisSize::Min)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(8.0 * compensation)
                .children(children![
                    label_bold(title, title_size, colors.on_surface),
                    Text::new(note)
                        .align(TextAlign::Center)
                        .style(vieww_foundation::TextStyle {
                            color: colors.on_surface_variant,
                            size: note_size,
                            line_height: 1.4,
                            ..vieww_foundation::TextStyle::new(note_size)
                        }),
                ]),
        )
        .into()
}

/// The sentence that keeps the platform picker honest.
///
/// # It used to name two gaps, and both are closed
///
/// It read: *"Widgets are not visually reskinned per platform. State in the
/// previewed screen resets on every Render."* Both were true and both were the
/// reason a developer checked the simulator anyway.
///
/// `ThemeData::platform` is a field now and the controls that differ between
/// iOS and Android read it — the switch, the slider, the checkbox, the button's
/// shape, the activity indicator, the alert's buttons, the segmented control.
/// And `ElementTree::snapshot_states` carries the screen's state across the
/// remount a Render performs.
///
/// What is left is what is still true, and it is said with the same
/// plainness: the widget catalogue is one catalogue that knows which platform
/// it is drawing for, not two catalogues; and state comes back only where the
/// edit did not change the shape of the tree around it.
///
/// # What it does *not* simulate, and used to claim
///
/// **Scroll physics.** `ScrollPhysics::platform_default()` resolves through
/// `TargetPlatform::current()` at compile time — the studio's host, not the
/// picker — and a `ScrollController` the previewed screen constructs itself
/// takes the host's feel rather than the chosen device's. The caption used to
/// list "scroll physics" alongside the safe area and `TargetPlatform`, which
/// is the kind of quietly-wrong that sends a developer to the simulator to
/// check something the studio already told them was fine. Removed rather than
/// fixed: making it true is a real change (publishing physics into the guest
/// subtree) and the honest statement is that it is not done.
///
/// # Why it closes
///
/// Reported: "cross icon to close that dialog box in the render pane." It is
/// not a dialog, but the note behaves like one — five lines of prose that answer
/// a question once and then sit under every frame for the rest of the session,
/// in the pane where the *picture* is the point. So it takes a close button.
///
/// **And a way back.** `Command::TogglePreviewNote` is in the palette and the
/// View menu; a control that hides something with no route to bringing it back
/// is how a studio loses a feature to a stray click. The dismissal is per
/// session, not written to settings: the note is worth meeting once per run.
fn caption(chrome: &StudioTheme, colors: ColorScheme, studio: &crate::state::Studio) -> WidgetNode {
    let dismiss = studio.clone();
    // **Two modes, two notes.** The paragraph below describes what a *compiled*
    // preview does and does not simulate, which is the wrong subject entirely
    // when the frame is drawing `live.rs` — nothing in a live sketch is
    // compiled, so "screen state is carried across a Render" is describing a
    // mechanism that is not running. Leaving it there was half of why the
    // three-screens-versus-one-screen question kept being asked: the pane's
    // own explanation never mentioned that two different things can be in the
    // frame.
    let text = if studio.live_preview.get() {
        "Live Preview draws live.rs — a sketch of the flow, parsed rather than \
         compiled, which is why it updates as you type. It is not part of your \
         application and nothing in it runs: the screens, rows and buttons are \
         drawn from that file alone. Press Render to compile the file you have \
         open and see the real widgets, or Build and Run to start the whole \
         application. A `mount` line puts one of your own rendered screens \
         inside the sketch."
    } else {
        "Simulates layout, platform behaviour and platform appearance — safe area, \
         TargetPlatform, and the controls whose shapes differ. One catalogue drawn \
         per platform, not a second widget set: an iOS-style picker or navigation \
         bar has no counterpart here. Screen state is carried across a Render \
         where the edit left the tree's shape alone."
    };
    let note = Text::new(text).style(vieww_foundation::TextStyle {
        color: colors.on_surface_variant,
        size: 10.5,
        line_height: 1.45,
        ..vieww_foundation::TextStyle::new(10.5)
    });

    Container::new()
        .color(chrome.chrome_1)
        .padding(EdgeInsets::only(12.0, 7.0, 6.0, 7.0))
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .children(children![
                    Flexible::expanded(1).child(note),
                    crate::ui::chrome::named_icon_button(
                        "Hide this note",
                        crate::ui::icons::close(),
                        11.0,
                        colors.on_surface_variant,
                        move || dismiss.preview_note.set(false),
                    ),
                ]),
        )
        .into()
}

/// `amount` of `top` over `bottom`. Same helper as the editor's; see there.
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

/// One row in the Devices tab: a name, the size it means, and a tick.
fn device_row(
    name: String,
    size: String,
    selected: bool,
    chrome: StudioTheme,
    colors: ColorScheme,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    crate::ui::chrome::button(
        name.clone(),
        move || {
            Container::new()
                .height(30.0)
                .color(if selected {
                    chrome.selection
                } else {
                    vieww_foundation::Color::TRANSPARENT
                })
                .padding(EdgeInsets::symmetric(14.0, 0.0))
                .alignment(Alignment::CENTER_LEFT)
                .child(
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            label(&name, 12.0, colors.on_surface),
                            Flexible::expanded(1).child(vieww_widget::SizedBox::shrink()),
                            // The size, in the trailing slot, so a column of
                            // them lines up and can be read down — the same
                            // rule the explorer's git letters follow.
                            label(&size, 10.5, colors.outline),
                        ]),
                )
                .into()
        },
        on_tap,
    )
}

/// A name the studio can find the previewed screen's element by.
///
/// # Why a widget that does nothing
///
/// [`Studio::capture_preview_state`] has to snapshot *the guest's* subtree and
/// nothing else — the chrome around it belongs to the studio and is not being
/// replaced. There is no other handle on that boundary: the element tree is one
/// tree, the guest's own widget types come from a library the studio cannot
/// name, and the only thing that is stable across a recompile is a name the
/// studio itself put there.
///
/// It is layout-transparent — one child, passed straight through — so it costs
/// a `debug_name` and nothing else.
///
/// [`Studio::capture_preview_state`]: crate::state::Studio::capture_preview_state
#[derive(Debug)]
pub struct GuestRoot {
    pub child: WidgetNode,
}

/// The name [`Studio::capture_preview_state`] looks for.
///
/// [`Studio::capture_preview_state`]: crate::state::Studio::capture_preview_state
pub const GUEST_ROOT: &str = "GuestRoot";

impl Widget for GuestRoot {
    fn debug_name(&self) -> &'static str {
        GUEST_ROOT
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        self.child.clone()
    }
}

widget_node_from!(GuestRoot);

/// The physical shell a previewed screen sits in.
///
/// # Why this is a painting and not a `Container`
///
/// Three of the four things that make an object look like an object cannot be
/// expressed by one decorated box: a shadow *under* it, a ramp *across* it and
/// a rim of light along its top edge are three fills at three different
/// geometries, and the side buttons are a fourth outside the box entirely.
/// Nesting four containers would work and would put four boxes in the tree per
/// frame of a preview that is already the most expensive region in the window.
#[derive(Debug)]
struct DeviceShell {
    corner: f32,
    bezel: f32,
    /// Desktop frames have no bezel and so no side buttons.
    buttons: bool,
}

impl vieww_widget::Painter for DeviceShell {
    fn paint(&self, book: &mut vieww_foundation::Sketchbook, size: vieww_foundation::Size) {
        use vieww_foundation::{Color, Gradient, Offset, Rect, Shadow};

        let body = Rect::new(0.0, 0.0, size.width, size.height);

        // Under it first: a wide, soft, downward shadow is what puts the
        // device *on* the stage rather than *in* it.
        book.shadow(
            body,
            self.corner,
            Shadow::new(Color::rgba(0, 0, 0, 150), Offset::new(0.0, 18.0), 46.0),
        );

        // Side buttons, drawn before the body so the body's edge cuts them —
        // which is what a button sunk into a chassis looks like.
        if self.buttons {
            let metal = Gradient::horizontal().with_stops(&[
                (0.0, Color::hex(0x3A_3D46)),
                (0.5, Color::hex(0x24_262D)),
                (1.0, Color::hex(0x14_161A)),
            ]);
            // Volume pair and the silent switch on the left, power on the right.
            for (top, height) in [(0.130, 0.030), (0.180, 0.070), (0.265, 0.070)] {
                book.rrect(
                    Rect::new(-2.5, size.height * top, 1.0, size.height * (top + height)),
                    1.5,
                    metal,
                );
            }
            book.rrect(
                Rect::new(
                    size.width - 1.0,
                    size.height * 0.205,
                    size.width + 2.5,
                    size.height * 0.325,
                ),
                1.5,
                metal,
            );
        }

        // The chassis: light along the top edge, falling away down the face.
        book.rrect(
            body,
            self.corner,
            Gradient::vertical().with_stops(&[
                (0.0, Color::hex(0x33_3640)),
                (0.06, Color::hex(0x17_181D)),
                (0.85, Color::hex(0x0B_0B0D)),
                (1.0, Color::hex(0x1C_1E24)),
            ]),
        );

        if self.bezel <= 0.0 {
            return;
        }

        // The rim: a hairline of caught light just inside the outer edge.
        // A filled ring, not a stroke — see the matching note in `brand.rs`;
        // a one-point rounded-rect stroke is the shape class GPU stroke
        // rasterisation handles worst, and this rim is on screen whenever a
        // preview is.
        book.fill(
            vieww_foundation::Path::rounded_ring(body, self.corner, 1.0),
            Color::rgba(255, 255, 255, 34),
        );

        // And the well the glass sits in — a dark line just outside the
        // screen, which is what gives the bezel its thickness. The same
        // stroke-to-ring conversion as above: the band a stroke straddling
        // the path covers is the ring two points wide centred on it.
        book.fill(
            vieww_foundation::Path::rounded_ring(
                Rect::new(
                    self.bezel,
                    self.bezel,
                    size.width - self.bezel,
                    size.height - self.bezel,
                ),
                (self.corner - self.bezel).max(0.0) + 1.0,
                2.0,
            ),
            Color::rgba(0, 0, 0, 170),
        );
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
