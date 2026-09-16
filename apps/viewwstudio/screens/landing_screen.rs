use vieww::prelude::*;

/// The screen the preview pane mounts.
///
/// Everything the studio needs from a buffer is here: a `Widget`, and a free
/// `screen()` the appended entry point calls. Nothing else is special.
#[derive(Debug)]
pub struct LandingScreen {
    pub name: String,
}

impl Widget for LandingScreen {
    fn debug_name(&self) -> &'static str {
        "LandingScreen"
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
                    .padding(EdgeInsets::symmetric(16.0, 14.0))
                    .child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(theme.metrics.gap)
                            .children(children![
                                Text::new("Good afternoon,").style(theme.text.headline),
                                Text::new(self.name.clone())
                                    .style(theme.text.headline)
                                    .bold(),
                                SizedBox::height(6.0),
                                Text::new(
                                    "Three screens compiled cleanly today. \
                                     Pick one up where you left it.",
                                )
                                .style(theme.text.body)
                                .color(theme.colors.on_surface_variant),
                                SizedBox::height(6.0),
                                chips(),
                                card(&theme, "LandingScreen", "edited 2 minutes ago"),
                                card(&theme, "CardGrid", "edited yesterday"),
                                Flexible::expanded(1).child(SizedBox::shrink()),
                                Button::new("Open editor").style(ButtonStyle::Filled),
                            ]),
                    ),
            )
            .into()
    }
}

fn chips() -> WidgetNode {
    Flex::row()
        .main_axis_size(MainAxisSize::Min)
        .spacing(6.0)
        .children(children![
            Chip::new("landing"),
            Chip::new("card grid"),
            Chip::new("settings"),
        ])
        .into()
}

fn card(theme: &ThemeData, title: &str, subtitle: &str) -> WidgetNode {
    Container::new()
        .color(theme.colors.surface_variant)
        .radius(theme.metrics.corner)
        .padding(EdgeInsets::all(12.0))
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .spacing(10.0)
                .children(children![
                    Avatar::initials(title),
                    Flexible::expanded(1).child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(2.0)
                            .children(children![
                                Text::new(title.to_string()).style(theme.text.title),
                                Text::new(subtitle.to_string())
                                    .style(theme.text.label)
                                    .color(theme.colors.on_surface_variant),
                            ])
                    ),
                ]),
        )
        .into()
}

pub fn screen() -> impl Widget {
    LandingScreen {
        name: "Teja".to_string(),
    }
}
