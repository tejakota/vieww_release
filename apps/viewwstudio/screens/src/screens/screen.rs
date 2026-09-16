//! The screen screen.

use vieww::prelude::*;

#[derive(Debug)]
pub struct Screen;

impl Widget for Screen {
    fn debug_name(&self) -> &'static str {
        "Screen"
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
                    .child(Text::new("Screen").style(theme.text.headline)),
            )
            .into()
    }
}

/// The entry point the preview looks for.
pub fn screen() -> impl Widget {
    Screen
}
