//! The find bar, and the replace row under it.
//!
//! Sits between the breadcrumbs and the code rather than floating over the
//! text, which is the one thing a find bar must not do: the match it just
//! found is often on the line it would be covering.
//!
//! Every button here goes through a method on [`Studio`], never through its own
//! copy of the search logic — see [`crate::find`] for why replace-all in
//! particular has exactly one implementation.

use std::rc::Rc;

use vieww_foundation::{Alignment, Border, Color, EdgeInsets, FontFamily, TextStyle};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, Flexible, SizedBox};

use crate::state::Studio;
use crate::theme::StudioTheme;
use crate::ui::chrome::{clickable, gap, hairline, label, label_bold, space};

const ROW: f32 = 34.0;

#[derive(Debug)]
pub struct FindBar {
    pub studio: Studio,
}

impl Widget for FindBar {
    fn debug_name(&self) -> &'static str {
        "FindBar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        if !self.studio.find_open.get() {
            return SizedBox::shrink().into();
        }

        let chrome = StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;
        let matches = self.studio.find_matches();
        let query = self.studio.find_query.get();
        let replacing = self.studio.find_replacing.get();

        // "3 of 12", or "No results", or nothing at all when the box is empty.
        // The third case matters: a bar that says "No results" before anything
        // has been typed reads as a failed search rather than an empty one.
        let count = if query.is_empty() {
            String::new()
        } else if matches.is_empty() {
            "No results".to_string()
        } else {
            format!(
                "{} of {}",
                self.studio.find_index.get().min(matches.len() - 1) + 1,
                matches.len()
            )
        };

        let mut rows: Vec<WidgetNode> = vec![
            hairline(chrome.line),
            self.find_row(&chrome, colors, &count, matches.is_empty()),
        ];
        if replacing {
            rows.push(self.replace_row(&chrome, colors, matches.len()));
        }
        rows.push(hairline(chrome.line));

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }
}

widget_node_from!(FindBar);

impl FindBar {
    fn find_row(
        &self,
        chrome: &StudioTheme,
        colors: ColorScheme,
        count: &str,
        empty: bool,
    ) -> WidgetNode {
        let studio = self.studio.clone();
        let field = box_field(
            self.studio.find_query.get(),
            "Find",
            chrome,
            colors,
            self.studio.caret_visible(crate::caret::Active::Find),
            Rc::new(move |text: String| {
                studio.find_query.set(text);
                // A new query invalidates which match was selected.
                studio.find_index.set(0);
            }),
        );

        let previous = self.studio.clone();
        let next = self.studio.clone();
        let close = self.studio.clone();
        let replacing = self.studio.clone();

        Container::new()
            .color(chrome.chrome_1)
            .height(ROW)
            .padding(EdgeInsets::symmetric(10.0, 0.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![
                        // The disclosure that shows the replace row, in the
                        // place every editor puts it.
                        toggle(
                            "»",
                            self.studio.find_replacing.get(),
                            colors,
                            *chrome,
                            move || {
                                let on = !replacing.find_replacing.peek();
                                replacing.find_replacing.set(on);
                            }
                        ),
                        space(6.0),
                        Flexible::expanded(1).child(field),
                        space(8.0),
                        self.option("Aa", &self.studio.find_case_sensitive, colors, *chrome),
                        space(4.0),
                        self.option("ab", &self.studio.find_whole_word, colors, *chrome),
                        space(10.0),
                        label(
                            count,
                            11.0,
                            if empty && !count.is_empty() {
                                colors.error
                            } else {
                                colors.on_surface_variant
                            }
                        ),
                        gap(),
                        step_button("‹", colors, *chrome, move || {
                            previous.run(crate::command::Command::FindPrevious);
                        }),
                        space(3.0),
                        step_button("›", colors, *chrome, move || {
                            next.run(crate::command::Command::FindNext);
                        }),
                        space(6.0),
                        step_button("✕", colors, *chrome, move || {
                            close.find_open.set(false);
                            close.find_replacing.set(false);
                        }),
                    ]),
            )
            .into()
    }

    fn replace_row(&self, chrome: &StudioTheme, colors: ColorScheme, matches: usize) -> WidgetNode {
        let studio = self.studio.clone();
        let field = box_field(
            self.studio.find_replacement.get(),
            "Replace with",
            chrome,
            colors,
            self.studio.caret_visible(crate::caret::Active::Replace),
            Rc::new(move |text: String| studio.find_replacement.set(text)),
        );

        let one = self.studio.clone();
        let all = self.studio.clone();
        let enabled = matches > 0;

        Container::new()
            .color(chrome.chrome_1)
            .height(ROW)
            .padding(EdgeInsets::only(32.0, 0.0, 10.0, 0.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![
                        Flexible::expanded(1).child(field),
                        space(8.0),
                        action("Replace", enabled, colors, *chrome, move || {
                            one.replace_current();
                        }),
                        space(4.0),
                        action(
                            &format!("All ({matches})"),
                            enabled,
                            colors,
                            *chrome,
                            move || all.replace_all()
                        ),
                    ]),
            )
            .into()
    }

    /// One of the two letter switches — case, whole word.
    fn option(
        &self,
        glyph: &str,
        signal: &vieww_element::Signal<bool>,
        colors: ColorScheme,
        chrome: StudioTheme,
    ) -> WidgetNode {
        let on = signal.get();
        let signal = signal.clone();
        toggle(glyph, on, colors, chrome, move || {
            signal.set(!signal.peek())
        })
    }
}

/// A small single-line field with a box around it.
fn box_field(
    text: String,
    placeholder: &str,
    chrome: &StudioTheme,
    colors: ColorScheme,
    caret: bool,
    on_changed: Rc<dyn Fn(String)>,
) -> WidgetNode {
    let field = TextField::text(text)
        .size(12.0)
        .show_cursor(caret)
        .style(
            TextStyle::new(12.0)
                .family(FontFamily::SansSerif)
                .color(colors.on_surface),
        )
        .placeholder(placeholder)
        .single_line()
        .selection_color(chrome.selection)
        .on_changed(Rc::new(move |value: vieww_foundation::TextEditingValue| {
            on_changed(value.text);
        }));

    Container::new()
        .color(chrome.chrome_0)
        .radius(5.0)
        .height(24.0)
        .padding(EdgeInsets::symmetric(8.0, 0.0))
        .alignment(Alignment::CENTER_LEFT)
        .border(Border {
            color: chrome.line,
            width: 1.0,
        })
        .child(field)
        .into()
}

/// A square switch that shows whether it is on.
fn toggle(
    glyph: &str,
    on: bool,
    colors: ColorScheme,
    chrome: StudioTheme,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    let glyph = glyph.to_string();
    clickable(
        move || {
            Container::new()
                .color(if on { chrome.chrome_3 } else { chrome.chrome_1 })
                .radius(4.0)
                .size(22.0, 22.0)
                .alignment(Alignment::CENTER)
                .child(label_bold(
                    &glyph,
                    11.0,
                    if on {
                        colors.primary
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

/// A next/previous/close chevron.
fn step_button(
    glyph: &str,
    colors: ColorScheme,
    chrome: StudioTheme,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    let glyph = glyph.to_string();
    clickable(
        move || {
            Container::new()
                .color(chrome.chrome_1)
                .radius(4.0)
                .size(22.0, 22.0)
                .alignment(Alignment::CENTER)
                .child(label(&glyph, 13.0, colors.on_surface_variant))
                .into()
        },
        on_tap,
    )
    .into()
}

/// A worded button — Replace, All.
fn action(
    text: &str,
    enabled: bool,
    colors: ColorScheme,
    chrome: StudioTheme,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    let text = text.to_string();
    clickable(
        move || {
            Container::new()
                .color(chrome.chrome_3)
                .radius(5.0)
                .height(22.0)
                .padding(EdgeInsets::symmetric(9.0, 0.0))
                .alignment(Alignment::CENTER)
                .child(label(
                    &text,
                    11.5,
                    if enabled {
                        colors.on_surface
                    } else {
                        // Greyed rather than absent: a button that disappears
                        // when there is nothing to replace moves the two beside
                        // it, and the row jitters as you type.
                        Color::rgba(
                            colors.on_surface.r,
                            colors.on_surface.g,
                            colors.on_surface.b,
                            90,
                        )
                    },
                ))
                .into()
        },
        move || {
            if enabled {
                on_tap();
            }
        },
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_element::Runtime;

    fn studio() -> (Runtime, Studio) {
        let runtime = Runtime::new();
        (runtime.clone(), Studio::new(&runtime))
    }

    #[test]
    fn a_shut_find_bar_is_not_in_the_tree() {
        let (_runtime, studio) = studio();
        let dump = vieww_widget::debug_tree(FindBar {
            studio: studio.clone(),
        });
        assert!(
            !dump.contains("TextField"),
            "a hidden search box still takes the keystrokes meant for the editor"
        );

        studio.find_open.set(true);
        let open = vieww_widget::debug_tree(FindBar { studio });
        assert!(open.contains("TextField"));
    }

    #[test]
    fn an_empty_query_says_nothing_rather_than_no_results() {
        let (_runtime, studio) = studio();
        studio.find_open.set(true);
        let dump = vieww_widget::debug_tree(FindBar {
            studio: studio.clone(),
        });
        assert!(
            !dump.contains("No results"),
            "an untyped search has not failed"
        );

        studio.find_query.set("qqqqq".into());
        let dump = vieww_widget::debug_tree(FindBar { studio });
        assert!(dump.contains("No results"));
    }

    #[test]
    fn the_replace_row_appears_only_when_asked_for() {
        let (_runtime, studio) = studio();
        studio.find_open.set(true);
        let dump = vieww_widget::debug_tree(FindBar {
            studio: studio.clone(),
        });
        assert!(!dump.contains("\"Replace\""));

        studio.find_replacing.set(true);
        let dump = vieww_widget::debug_tree(FindBar { studio });
        assert!(
            dump.contains("\"Replace\""),
            "the Replace button is the row"
        );
    }
}
