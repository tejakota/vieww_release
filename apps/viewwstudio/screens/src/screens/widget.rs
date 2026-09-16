//! The widget widget.

use vieww::prelude::*;

#[derive(Debug)]
pub struct Widget;

impl Widget for Widget {
    fn debug_name(&self) -> &'static str {
        "Widget"
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
            .child(Text::new("Widget").style(theme.text.body))
            .into()
    }
}
