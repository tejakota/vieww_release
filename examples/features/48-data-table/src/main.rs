//! A data table: fixed columns, a sort indicator, and cells built on demand.
//!
//! The cell builder is `(row, column) -> WidgetNode`, so the table holds no
//! copy of the data and a cell can be anything — the status column here is a
//! chip rather than text, which is the point of not taking strings.

use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;
use vieww_widget::SortDirection;

const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);

const ROWS: [(&str, &str, &str); 5] = [
    ("Ada Lovelace", "Analyst", "active"),
    ("Grace Hopper", "Compiler", "active"),
    ("Karen Spärck Jones", "Retrieval", "away"),
    ("Barbara Liskov", "Systems", "active"),
    ("Radia Perlman", "Networks", "away"),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("48 — data table", Size::new(680.0, 340.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(12.0)
                        .children(children![
                            Text::new("DataTable").color(INK).size(18.0).bold(),
                            DataTable::new(
                                vec![
                                    DataColumn::new("Name", 240.0).sortable(),
                                    DataColumn::new("Team", 200.0).sortable(),
                                    DataColumn::new("Status", 140.0),
                                ],
                                ROWS.len(),
                                44.0,
                                |row: usize, column: usize| match column {
                                    0 => Text::new(ROWS[row].0).color(INK).size(14.0).into(),
                                    1 => Text::new(ROWS[row].1).color(MUTED).size(14.0).into(),
                                    _ => Chip::new(ROWS[row].2).into(),
                                },
                            )
                            .sorted_by(0, SortDirection::Ascending),
                        ]),
                ),
        );
    })
}
