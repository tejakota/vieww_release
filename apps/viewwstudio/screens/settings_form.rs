use vieww::prelude::*;

/// A form. Controls that branch on `TargetPlatform` show what the picker is
/// for: switch the preview to iOS or Android and this is told it is there.
#[derive(Debug)]
pub struct SettingsForm {
    pub notifications: bool,
    pub volume: f32,
}

impl Widget for SettingsForm {
    fn debug_name(&self) -> &'static str {
        "SettingsForm"
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
                            .spacing(theme.metrics.gap)
                            .children(children![
                                Text::new("Settings").style(theme.text.headline),
                                row(
                                    &theme,
                                    "Notifications",
                                    Switch::new(self.notifications).into(),
                                ),
                                row(&theme, "Volume", Slider::new(self.volume).into()),
                                Flexible::expanded(1).child(SizedBox::shrink()),
                                Button::new("Done").style(ButtonStyle::Filled),
                            ]),
                    ),
            )
            .into()
    }
}

fn row(theme: &ThemeData, label: &str, control: WidgetNode) -> WidgetNode {
    Flex::row()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(children![
            Flexible::expanded(1).child(Text::new(label.to_string()).style(theme.text.body)),
            control,
        ])
        .into()
}

pub fn screen() -> impl Widget {
    SettingsForm {
        notifications: true,
        volume: 0.4,
    }
}
