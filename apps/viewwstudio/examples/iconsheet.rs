//! Every icon at three sizes, on one sheet, so a wrong path is obvious.
//!
//! ```console
//! cargo run -p viewwstudio --example iconsheet -- icons.png
//! ```
//!
//! Path data is a string, and a string that parses is not a string that draws
//! the right shape — a mirrored curve or a dropped command produces a perfectly
//! valid path of the wrong picture. There is no assertion that catches that.

use std::path::PathBuf;

use vieww_foundation::{Alignment, Color, EdgeInsets, IconData, Size};
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::Container;
use viewwstudio::ui::icons as studio;

const SHEET: Size = Size {
    width: 1080.0,
    height: 600.0,
};

fn main() {
    let path = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("icons.png"), PathBuf::from);

    let all: Vec<(&str, IconData)> = vec![
        ("swatch", studio::swatch()),
        ("preview", studio::preview()),
        ("book", studio::book()),
        ("save", studio::save()),
        ("branch", studio::branch()),
        ("export", studio::export()),
        ("lightbulb", studio::lightbulb()),
        ("chevron_down", studio::chevron_down()),
        ("chevron_right", studio::chevron_right()),
        ("check", studio::check()),
        ("close", studio::close()),
        ("plus", studio::plus()),
        ("folder", studio::folder()),
        ("search", studio::search()),
        ("code", studio::code()),
        ("warning", studio::warning()),
        ("dashboard", studio::dashboard()),
        ("tune", studio::tune()),
        ("gear", studio::gear()),
        ("play", studio::play()),
        ("phone", studio::phone()),
        ("clock", studio::clock()),
        ("shield", studio::shield()),
        ("error", studio::error()),
        ("trash", studio::trash()),
        ("moon", studio::moon()),
        ("panel", studio::panel()),
        ("split", studio::split()),
        ("collapse_all", studio::collapse_all()),
        ("file", studio::file()),
        ("safe_area", studio::safe_area()),
    ];

    let ink = Color::hex(0xCC_CCCC);
    let cells = all
        .into_iter()
        .map(|(name, data)| -> WidgetNode {
            Container::new()
                .width(88.0)
                .padding(EdgeInsets::symmetric(0.0, 10.0))
                .child(
                    Flex::column()
                        .main_axis_size(MainAxisSize::Min)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .spacing(6.0)
                        .children(children![
                            viewwstudio::ui::chrome::glyph(data.clone(), 28.0, ink),
                            viewwstudio::ui::chrome::glyph(data.clone(), 18.0, ink),
                            viewwstudio::ui::chrome::glyph(data.clone(), 12.0, ink),
                            Icon::new(data).size(18.0).color(Color::hex(0x55_5560)),
                            Text::new(name).size(9.0).color(Color::hex(0x9D_9D9D)),
                        ]),
                )
                .into()
        })
        .collect::<Vec<_>>();

    // Ten to a row, so the sheet is a few bands rather than one long strip.
    let rows = cells
        .chunks(10)
        .map(|chunk| -> WidgetNode {
            Flex::row()
                .main_axis_size(MainAxisSize::Min)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .children(chunk.to_vec())
                .into()
        })
        .collect::<Vec<_>>();

    let mut driver = FrameDriver::new(SHEET);
    driver.set_root(
        Container::new()
            .color(Color::hex(0x1F_1F1F))
            .alignment(Alignment::CENTER)
            .child(
                Flex::column()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(12.0)
                    .children(rows),
            ),
    );
    driver.draw_frame();

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a sheet size in logical pixels is a small positive number"
    )]
    let (width, height) = (SHEET.width as u32, SHEET.height as u32);

    // The CPU backend, unconditionally: this sheet's whole job is to be looked
    // at on whatever machine ran the build, and half of those — this container,
    // every CI runner without an adapter — have no GPU to render it with. It
    // used to print "no graphics adapter; nothing rendered" and exit 0, which
    // is indistinguishable from success to anything reading an exit code.
    let mut cpu = vieww_paint::native::NativeRenderer::new();
    let (png, _) = cpu
        .render_to_png(driver.scene(), width, height, Color::hex(0x1F_1F1F))
        .expect("rendering the sheet");
    std::fs::write(&path, png).expect("writing the PNG");
    println!("{}", path.display());
}
