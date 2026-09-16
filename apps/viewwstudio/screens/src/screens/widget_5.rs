//! The widget_5 widget.

use vieww::prelude::*;

#[derive(Debug)]
pub struct Widget5;

impl Widget for Widget5 {
    fn debug_name(&self) -> &'static str {
        "Widget5"
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
            .child(Text::new("Widget5").style(theme.text.body))
            .into()
    }
}
