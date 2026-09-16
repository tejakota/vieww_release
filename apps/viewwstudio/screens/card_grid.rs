use vieww::prelude::*;

/// A column of cards. The second screen, so the tab strip has something real
/// in it and switching buffers changes what compiles.
#[derive(Debug)]
pub struct CardGrid {
    pub titles: Vec<String>,
}

impl Widget for CardGrid {
    fn debug_name(&self) -> &'static str {
        "CardGrid"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        SafeArea::new()
            .child(
                Container::new()
                    .color(theme.colors.surface)
                    .padding(EdgeInsets::all(16.0))
                    .child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .spacing(12.0)
                            .children(
                                self.titles
                                    .iter()
                                    .map(|title| card(&theme, title))
                                    .collect::<Vec<WidgetNode>>(),
                            ),
                    ),
            )
            .into()
    }
}

fn card(theme: &ThemeData, title: &str) -> WidgetNode {
    Container::new()
        .color(theme.colors.surface_variant)
        .radius(theme.metrics.corner)
        .padding(EdgeInsets::all(16.0))
        .child(Text::new(title.to_string()).style(theme.text.title))
        .into()
}

pub fn screen() -> impl Widget {
    CardGrid {
        titles: vec![
            "Inbox".to_string(),
            "Drafts".to_string(),
            "Archive".to_string(),
            "Sent".to_string(),
        ],
    }
}
