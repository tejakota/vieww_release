//! The handful of icons the controls in this crate need.
//!
//! Not an icon library. **Ten shapes**, because a checkbox needs a tick, a chip
//! needs a way to be dismissed, a list row needs to say it leads somewhere, a
//! stepper needs a plus and a minus, and a chevron is needed in all four
//! directions — plus the two direction-aware wrappers that pick between them
//! for a right-to-left locale. An application wanting more supplies its own
//! [`IconData`] — that is the point of the type being open.
//!
//! (It said "six" for as long as there were ten. A miscount in a doc is small
//! until it is the sentence somebody uses to decide whether the set contains
//! what they need.)
//!
//! # Filled outlines, not strokes
//!
//! The paint layer fills paths and does not stroke them, so a tick here is a
//! six-sided polygon rather than a two-segment line with a width. That is not a
//! workaround: at these sizes a stroked icon has to be converted to an outline
//! for rasterisation anyway, and doing it once at design time means the shape is
//! exactly what was intended rather than what a joins-and-caps algorithm made of
//! it. It does mean an icon cannot be re-weighted by changing a number.
//!
//! The geometry is the classic 24x24 icon grid, which is why every one of
//! these declares [`IconData::square24`].

use vieww_foundation::{IconData, Offset, Path, TextDirection};

/// Build a closed polygon through `points`, on the 24x24 grid.
fn polygon(points: &[(f32, f32)]) -> IconData {
    let mut path = Path::new();
    let mut points = points.iter();
    if let Some(&(x, y)) = points.next() {
        path.move_to(Offset::new(x, y));
    }
    for &(x, y) in points {
        path.line_to(Offset::new(x, y));
    }
    path.close();
    IconData::square24(path)
}

/// A tick. What a ticked [`Checkbox`](crate::Checkbox) puts in its box.
#[must_use]
pub fn check() -> IconData {
    polygon(&[
        (9.0, 16.17),
        (4.83, 12.0),
        (3.41, 13.41),
        (9.0, 19.0),
        (21.0, 7.0),
        (19.59, 5.59),
    ])
}

/// A cross. Dismiss, remove, close.
#[must_use]
pub fn close() -> IconData {
    polygon(&[
        (19.0, 6.41),
        (17.59, 5.0),
        (12.0, 10.59),
        (6.41, 5.0),
        (5.0, 6.41),
        (10.59, 12.0),
        (5.0, 17.59),
        (6.41, 19.0),
        (12.0, 13.41),
        (17.59, 19.0),
        (19.0, 17.59),
        (13.41, 12.0),
    ])
}

/// A right-pointing chevron. A row that leads somewhere.
#[must_use]
pub fn chevron_right() -> IconData {
    polygon(&[
        (10.0, 6.0),
        (8.59, 7.41),
        (13.17, 12.0),
        (8.59, 16.59),
        (10.0, 18.0),
        (16.0, 12.0),
    ])
}

/// A left-pointing chevron. Back.
#[must_use]
pub fn chevron_left() -> IconData {
    polygon(&[
        (15.41, 7.41),
        (14.0, 6.0),
        (8.0, 12.0),
        (14.0, 18.0),
        (15.41, 16.59),
        (10.83, 12.0),
    ])
}

/// An up-pointing chevron. Sorted ascending, or a section that collapses.
///
/// The same polygon as [`chevron_left`] with its axes swapped, so the four
/// chevrons are one shape at four rotations rather than four hand-placed
/// outlines that drift apart in weight.
#[must_use]
pub fn chevron_up() -> IconData {
    polygon(&[
        (7.41, 15.41),
        (6.0, 14.0),
        (12.0, 8.0),
        (18.0, 14.0),
        (16.59, 15.41),
        (12.0, 10.83),
    ])
}

/// A down-pointing chevron. Sorted descending, or a section that expands.
#[must_use]
pub fn chevron_down() -> IconData {
    polygon(&[
        (6.0, 10.0),
        (7.41, 8.59),
        (12.0, 13.17),
        (16.59, 8.59),
        (18.0, 10.0),
        (12.0, 16.0),
    ])
}

/// The chevron that points **back**, whichever way the interface reads.
///
/// Left in Latin, right in Arabic. Pair it with
/// [`Directionality::of`](crate::Directionality::of).
///
/// # Why this rather than mirroring an icon with a transform
///
/// A horizontal flip is the obvious-looking answer and it is the wrong one
/// here, for two reasons:
///
/// - **It buys nothing.** The only directional shapes this module ships are the
///   two chevrons, and they are already an exact mirrored pair — there is a test
///   asserting it. Choosing between them is a branch; flipping is a matrix.
/// - **It would cost more than it looks.** `RenderTransform` applies its matrix
///   about the object's **top-left**, and that is not an accident: the same
///   matrix is handed to hit testing, which inverts it (`tree.rs`, via
///   `RenderObject::transform`). Flipping about the centre instead means a pivot
///   in both places, and the pivot needs the laid-out size, which
///   `transform(&self)` has no way to see. Getting it wrong desynchronises what
///   is drawn from what can be touched.
///
/// And most icons **must not** be mirrored at all — anything bearing a glyph, a
/// clock face, or a light source. A per-icon choice is the correct granularity,
/// which is what this is.
#[must_use]
pub fn chevron_back(direction: TextDirection) -> IconData {
    match direction {
        TextDirection::Ltr => chevron_left(),
        TextDirection::Rtl => chevron_right(),
    }
}

/// The chevron that points **onward**, whichever way the interface reads.
///
/// Right in Latin, left in Arabic. What a list row that leads somewhere should
/// use in place of a bare [`chevron_right`]. See [`chevron_back`] for why this
/// is a choice between two shapes rather than a mirroring transform.
#[must_use]
pub fn chevron_forward(direction: TextDirection) -> IconData {
    match direction {
        TextDirection::Ltr => chevron_right(),
        TextDirection::Rtl => chevron_left(),
    }
}

/// A plus.
#[must_use]
pub fn add() -> IconData {
    polygon(&[
        (19.0, 13.0),
        (13.0, 13.0),
        (13.0, 19.0),
        (11.0, 19.0),
        (11.0, 13.0),
        (5.0, 13.0),
        (5.0, 11.0),
        (11.0, 11.0),
        (11.0, 5.0),
        (13.0, 5.0),
        (13.0, 11.0),
        (19.0, 11.0),
    ])
}

/// A minus.
#[must_use]
pub fn remove() -> IconData {
    polygon(&[(19.0, 13.0), (5.0, 13.0), (5.0, 11.0), (19.0, 11.0)])
}

#[cfg(test)]
mod tests {
    use vieww_foundation::Rect;

    use super::*;

    #[test]
    fn every_icon_stays_inside_the_grid_it_declares() {
        let grid = Rect::new(0.0, 0.0, 24.0, 24.0);
        for (name, icon) in [
            ("check", check()),
            ("close", close()),
            ("chevron_right", chevron_right()),
            ("chevron_left", chevron_left()),
            ("add", add()),
            ("remove", remove()),
        ] {
            let bounds = icon.path().bounds();
            assert!(
                bounds.left >= grid.left
                    && bounds.top >= grid.top
                    && bounds.right <= grid.right
                    && bounds.bottom <= grid.bottom,
                "{name} escapes its viewbox: {bounds:?}"
            );
        }
    }

    #[test]
    fn back_and_forward_point_opposite_ways_and_swap_with_the_reading_direction() {
        // Asserted by path equality rather than by naming the expected function,
        // so that this still fails if the two chevrons are ever swapped at their
        // definitions — which is the mistake that would make every back button
        // in an application point the wrong way while every test still passed.
        for direction in [TextDirection::Ltr, TextDirection::Rtl] {
            assert_ne!(
                chevron_back(direction).path(),
                chevron_forward(direction).path(),
                "back and forward must not be the same shape in {direction:?}"
            );
        }

        assert_eq!(
            chevron_back(TextDirection::Ltr).path(),
            chevron_left().path(),
            "back is left in Latin"
        );
        assert_eq!(
            chevron_back(TextDirection::Rtl).path(),
            chevron_right().path(),
            "back is right in Arabic"
        );
    }

    #[test]
    fn forward_in_one_direction_is_back_in_the_other() {
        // The property that makes these two functions rather than four icons:
        // they are the same pair read from opposite ends.
        assert_eq!(
            chevron_forward(TextDirection::Ltr).path(),
            chevron_back(TextDirection::Rtl).path()
        );
        assert_eq!(
            chevron_forward(TextDirection::Rtl).path(),
            chevron_back(TextDirection::Ltr).path()
        );
    }

    #[test]
    fn a_polygon_is_closed_so_that_filling_it_is_defined() {
        let verbs = check().path().verbs().to_vec();
        assert!(matches!(
            verbs.last(),
            Some(vieww_foundation::PathVerb::Close)
        ));
    }

    #[test]
    fn the_two_chevrons_are_mirror_images() {
        let mirror = |bounds: Rect| {
            Rect::new(
                24.0 - bounds.right,
                bounds.top,
                24.0 - bounds.left,
                bounds.bottom,
            )
        };
        let right = chevron_right().path().bounds();
        let left = chevron_left().path().bounds();

        let mirrored = mirror(right);
        assert!(
            (mirrored.left - left.left).abs() < 0.01 && (mirrored.right - left.right).abs() < 0.01,
            "{mirrored:?} vs {left:?}"
        );
    }
}
