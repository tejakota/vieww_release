//! The widget_6 widget.

use vieww::prelude::*;

#[derive(Debug)]
pub struct Widget6;

impl Widget for Widget6 {
    fn debug_name(&self) -> &'static str {
        "Widget6"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        Container::new()
            .color(theme.colors.surface_variant)
            .radius(theme.metrics.corner)
            .padding(EdgeInsets::all(12.0))
            .child(Text::new("Widget6").style(theme.text.body))
            .into()
    }
}
