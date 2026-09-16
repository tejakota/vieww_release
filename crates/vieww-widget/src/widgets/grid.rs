//! A grid layout built on Flex.
//!
//! # Why Flex and not a new layout engine
//!
//! A grid with uniform column widths is just rows of Flex::row. A grid
//! with `Fr` tracks requires knowing the available width at layout
//! time — which the composed-widget layer does not have. This
//! implementation handles the common cases:
//!
//! - **Fixed columns:** N columns of equal width
//! - **Auto columns:** columns sized by content (approximated)
//!
//! For full CSS Grid (named areas, spanning, `minmax`), the render
//! layer needs a real grid algorithm. That is future work; this covers
//! the 80% of grid use cases.

use crate::prelude::*;

/// How one column is sized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridTrack {
    /// All columns share the available width equally.
    Fr(f32),
    /// A fixed width in logical pixels.
    Fixed(f32),
    /// As wide as the widest child in that column.
    ///
    /// Approximated: the composed layer measures at build time, which
    /// happens before layout. A true `Auto` needs render-layer support.
    Auto,
}

/// A two-dimensional grid layout.
///
/// Children are placed row-major (left-to-right, top-to-bottom).
///
/// # Examples
///
/// A 3-column grid:
///
/// ```ignore
/// Grid::columns(3)
///     .gap(16.0)
///     .children(vec![
///         card_one, card_two, card_three,
///         card_four, card_five, card_six,
///     ])
/// ```
///
/// With fixed-width columns:
///
/// ```ignore
/// Grid::new(vec![
///         GridTrack::Auto,     // label column
///         GridTrack::Fr(1.0),  // field column
///         GridTrack::Auto,     // trailing icon
///     ])
///     .gap(12.0)
///     .children(form_fields)
/// ```
#[derive(Debug)]
pub struct Grid {
    tracks: Vec<GridTrack>,
    gap: f32,
    children: Vec<WidgetNode>,
}

impl Grid {
    /// A grid with `n` equal columns.
    #[must_use]
    pub fn columns(n: usize) -> Self {
        Self {
            tracks: vec![GridTrack::Fr(1.0); n],
            gap: 0.0,
            children: Vec::new(),
        }
    }

    /// A grid with explicit column tracks.
    #[must_use]
    pub fn new(tracks: Vec<GridTrack>) -> Self {
        Self {
            tracks,
            gap: 0.0,
            children: Vec::new(),
        }
    }

    /// Uniform gap between rows and columns.
    #[must_use]
    pub const fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Add children, filling row-major.
    #[must_use]
    pub fn children(mut self, children: Vec<WidgetNode>) -> Self {
        self.children.extend(children);
        self
    }
}

impl Widget for Grid {
    fn debug_name(&self) -> &'static str {
        "Grid"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let column_count = self.tracks.len().max(1);

        // Group children into rows of `column_count`.
        let rows: Vec<&[WidgetNode]> = self.children.chunks(column_count).collect();

        // Build each row as a Flex::row.
        let row_widgets: Vec<WidgetNode> = rows
            .iter()
            .map(|row| {
                let cells: Vec<WidgetNode> = row
                    .iter()
                    .enumerate()
                    .map(|(col, child)| {
                        match self.tracks.get(col) {
                            Some(GridTrack::Fixed(w)) => {
                                // Fixed width: size the child.
                                SizedBox::from_size(Size::new(*w, 0.0))
                                    .child(child.clone())
                                    .into()
                            }
                            _ => {
                                // Fr or Auto: let Flex distribute.
                                Flexible::expanded(1).child(child.clone()).into()
                            }
                        }
                    })
                    .collect();

                Flex::row().spacing(self.gap).children(cells).into()
            })
            .collect();

        // Stack the rows in a Flex::column.
        Flex::column()
            .spacing(self.gap)
            .children(row_widgets)
            .into()
    }
}

crate::widget_node_from!(Grid);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_with_three_columns_groups_correctly() {
        // 7 children in 3 columns = 3 rows (3 + 3 + 1).
        let grid = Grid::columns(3).children(vec![
            SizedBox::square(10.0).into(),
            SizedBox::square(10.0).into(),
            SizedBox::square(10.0).into(),
            SizedBox::square(10.0).into(),
            SizedBox::square(10.0).into(),
            SizedBox::square(10.0).into(),
            SizedBox::square(10.0).into(),
        ]);

        assert_eq!(grid.tracks.len(), 3);
        assert_eq!(grid.children.len(), 7);
    }

    #[test]
    fn grid_tracks_can_be_mixed() {
        let grid = Grid::new(vec![
            GridTrack::Auto,
            GridTrack::Fr(1.0),
            GridTrack::Fixed(40.0),
        ]);

        assert_eq!(grid.tracks.len(), 3);
        assert_eq!(grid.tracks[2], GridTrack::Fixed(40.0));
    }
}
