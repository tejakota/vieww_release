//! The icon set: centreline paths, drawn as strokes.
//!
//! # Why these are lines and not shapes
//!
//! A *filled* set is a
//! coherent choice — it is what a phone's system icons are, and it is what
//! `vieww-widget::icons` still ships for the framework's own controls. It is
//! the wrong choice for an editor's chrome. A window that is nine tenths grey
//! surfaces and 11-point labels, with a column of solid glyphs down the left,
//! reads as heavier than everything around it; every desktop editor people
//! arrive from — VS Code's Codicons, JetBrains' set, Xcode's SF Symbols at
//! `.light` — draws its chrome icons as lines for exactly that reason.
//!
//! So every path here is a **centreline**: the line the pen travels, not the
//! outline of a shape. [`Icon::stroke`](vieww_widget::Icon::stroke) is what
//! draws it, and [`WEIGHT`] is the one number that sets the weight of the whole
//! interface.
//!
//! # What that buys, beyond the look
//!
//! A filled set bakes its weight into its geometry, so the same icon at 13
//! points and at 21 has proportionally different-looking strokes and the only
//! fix is a second set. These stay even at every size, because the pen is a
//! number and the number does not scale with the box.
//!
//! # Drawn on the 24-unit grid, sized in points
//!
//! Every path is [`IconData::square24`], so the coordinates below are the
//! familiar 24-unit icon grid even though the shapes are original. Paths are
//! inset roughly half a stroke from the grid's edges, because a stroke straddles
//! its path and a centreline at `x = 3` with a 1.5-wide pen reaches `x = 2.25`.
//!
//! # Parsed once, at first use
//!
//! [`parse_path_data`] runs on a `&str`, and an icon is rebuilt on every
//! rebuild of the region that shows it — a keystroke rebuilds a quarter of the
//! studio's element tree, and every chrome icon in it re-parses its path data
//! from scratch. That is a few hundred verbs and a few dozen `Vec` growths per
//! rebuild, spent producing a value that has been the same since the process
//! started.
//!
//! So `icon` memoises on the path string, and the handful of icons that
//! *build* their string — the ones made of `circle`s — memoise the finished
//! [`IconData`] with `built` so they do not `format!` per call either.
//! `IconData` is an `Rc<Path>` behind the scenes, so a cache hit is a refcount
//! bump rather than a copy.
//!
//! Thread-local rather than a `OnceLock` static because `Rc` is not `Sync`, and
//! the studio's tree is built on one thread. A second thread that asked for an
//! icon would simply fill its own table.

use std::cell::{OnceCell, RefCell};

use vieww_foundation::{parse_path_data, FastMap, IconData, Path};

/// The pen width every chrome icon is drawn with, in the 24-unit design grid.
///
/// One number for the whole interface. `Icon::stroke` scales it into whatever
/// box the icon lands in, so a 13-point icon in the status bar and a 21-point
/// one in the activity bar are the same pen rather than two sets, and retuning
/// the weight of the studio is editing this line.
///
/// The pen is rasterised as a **filled outline**, not a stroke:
/// `RenderIcon::paint` expands it with `Path::stroke_outline` before drawing.
/// That is the fix for the smeared chrome — a thin stroke is the one
/// primitive GPU rasterisers disagree about most, and a fill of the same ink
/// is the one they agree about — so this number now governs a pre-expanded
/// outline rather than a live stroke. The weight semantics are unchanged: the
/// outline is built from the *rounded whole-pixel* pen `RenderIcon` computes
/// at each size, which is what keeps a 13-point icon and a 21-point one at
/// the same weight on the same grid.
pub const WEIGHT: f32 = 1.6;

/// Build an icon from `d`, or a visible placeholder if the data is malformed.
///
/// A panic here would take the window down for a typo in a path string, and a
/// silently empty icon would leave a hole nobody could explain. A crossed box is
/// wrong in a way that is obvious in the first screenshot.
fn icon(data: &str) -> IconData {
    thread_local! {
        /// Keyed by the path string itself, so the cache cannot disagree with
        /// the source: editing a `d` string is a different key and therefore a
        /// different entry, with no name to keep in step. The table is bounded
        /// by the number of distinct strings in this file.
        static PARSED: RefCell<FastMap<String, IconData>> =
            RefCell::new(FastMap::default());
    }
    PARSED.with(|parsed| {
        // Looked up through `&str`, so a hit — which is every call after the
        // first — allocates nothing at all. Only a miss owns the key.
        if let Some(hit) = parsed.borrow().get(data) {
            return hit.clone();
        }
        let built = parse_path_data(data).map_or_else(|_| fallback(), IconData::square24);
        parsed.borrow_mut().insert(data.to_owned(), built.clone());
        built
    })
}

/// Memoise a whole icon expression, for the ones that build their path data
/// rather than quoting it.
///
/// [`icon`]'s own cache spares those the *parse* but not the `format!`s that
/// produce the string to look up — `gear` alone builds nine — so they cache the
/// finished value instead. Each expansion gets a `OnceCell` of its own, which
/// is why this is a macro: there is no key to collide and nothing to name.
macro_rules! built {
    ($build:expr) => {{
        thread_local! {
            static SLOT: OnceCell<IconData> = const { OnceCell::new() };
        }
        SLOT.with(|slot| slot.get_or_init(|| $build).clone())
    }};
}

/// What an icon whose path data does not parse comes out as: a crossed box.
///
/// Public so a test can ask "is this the fallback" instead of guessing at a
/// property the fallback has and real icons do not. The guess this replaced was
/// *"more than six verbs"*, which was true of the old filled set and false of
/// `code()` — two chevrons, six verbs, and a perfectly good icon failing a test
/// that was really asking whether a string had a typo in it.
#[must_use]
pub fn fallback() -> IconData {
    parse_path_data("M4 4h16v16H4zM4 4l16 16M20 4L4 20")
        .map_or_else(|_| IconData::square24(Path::new()), IconData::square24)
}

/// A circle of `radius` about (`cx`, `cy`), as two arcs.
///
/// Written out rather than composed from `Path::arc`, because these are string
/// constants and one helper that produces a string keeps the whole file one
/// kind of thing.
fn circle(cx: f32, cy: f32, radius: f32) -> String {
    format!(
        "M{cx} {top}a{radius} {radius} 0 1 1 0 {diameter}a{radius} {radius} 0 1 1 0 -{diameter}",
        top = cy - radius,
        diameter = radius * 2.0,
    )
}

/// Explorer: a folder with a tab.
#[must_use]
pub fn folder() -> IconData {
    icon("M3.4 19.6V5.4h5.4l2.2 2.7h9.6v11.5z")
}

/// Search: a lens and its handle.
#[must_use]
pub fn search() -> IconData {
    icon("M16.6 10.4a6.2 6.2 0 1 1-12.4 0 6.2 6.2 0 0 1 12.4 0M14.9 14.9l5 5")
}

/// Snippets: angle brackets.
#[must_use]
pub fn code() -> IconData {
    icon("M9.2 7.4 3.9 12l5.3 4.6M14.8 7.4 20.1 12l-5.3 4.6")
}

/// Problems: a triangle with a bang in it.
#[must_use]
pub fn warning() -> IconData {
    icon("M12 4.2 21.3 20.1H2.7zM12 10.1v4.4M12 17.1v1.1")
}

/// Inspector: a pane layout.
#[must_use]
pub fn dashboard() -> IconData {
    icon("M3.6 4.6h16.8v14.8H3.6zM3.6 9.4h16.8M10 9.4v10")
}

/// Toolchain: three sliders with their knobs.
#[must_use]
pub fn tune() -> IconData {
    built!({
        let mut d = String::from(
            "M3.8 7.4h4.1M12.3 7.4h7.9M3.8 12h8.3M16.7 12h3.5M3.8 16.6h6.4M14.9 16.6h5.3",
        );
        d.push_str(&circle(10.1, 7.4, 2.1));
        d.push_str(&circle(14.4, 12.0, 2.1));
        d.push_str(&circle(12.6, 16.6, 2.1));
        icon(&d)
    })
}

/// Theme tokens: two overlapping swatches.
#[must_use]
pub fn swatch() -> IconData {
    icon("M4.4 4.4h8.6v8.6H4.4zM11 11h8.6v8.6H11z")
}

/// Settings: a gear, as a ring and eight teeth.
#[must_use]
pub fn gear() -> IconData {
    built!({
        let mut d = circle(12.0, 12.0, 2.7);
        d.push_str(&circle(12.0, 12.0, 6.6));
        // Eight teeth on the compass points and the diagonals. Written out
        // rather than generated in a loop for the same reason as `circle`: the
        // file is path data, and a reader comparing two icons should be
        // comparing two strings.
        for (dx, dy) in [
            (0.0_f32, -1.0_f32),
            (0.707, -0.707),
            (1.0, 0.0),
            (0.707, 0.707),
            (0.0, 1.0),
            (-0.707, 0.707),
            (-1.0, 0.0),
            (-0.707, -0.707),
        ] {
            let (x0, y0) = (12.0 + dx * 6.4, 12.0 + dy * 6.4);
            let (x1, y1) = (12.0 + dx * 8.9, 12.0 + dy * 8.9);
            d.push_str(&format!("M{x0:.2} {y0:.2}L{x1:.2} {y1:.2}"));
        }
        icon(&d)
    })
}

/// Render: a play triangle.
#[must_use]
pub fn play() -> IconData {
    icon("M8.2 5.4 19 12 8.2 18.6z")
}

/// A phone, for the status bar's simulated device.
#[must_use]
pub fn phone() -> IconData {
    icon("M6.6 3.6h10.8v16.8H6.6zM10.2 5.9h3.6")
}

/// Timings: a clock.
#[must_use]
pub fn clock() -> IconData {
    built!({
        let mut d = circle(12.0, 12.0, 8.3);
        d.push_str("M12 6.7v5.6l3.6 2.1");
        icon(&d)
    })
}

/// The ABI check: a shield.
#[must_use]
pub fn shield() -> IconData {
    icon("M12 3.4 19.4 6.4v5.1c0 4.5-3.1 7.7-7.4 9.1-4.3-1.4-7.4-4.6-7.4-9.1V6.4z")
}

/// An error: a crossed circle.
#[must_use]
pub fn error() -> IconData {
    built!({
        let mut d = circle(12.0, 12.0, 8.3);
        d.push_str("M9.1 9.1l5.8 5.8M14.9 9.1l-5.8 5.8");
        icon(&d)
    })
}

/// Clear: a bin.
///
/// The two ridge lines are `x = 8.0 / 16.0` rather than the
/// original's `10.2 / 13.8`. At this icon's real render size — 13pt,
/// `panel.rs`'s `icon_button(icons::trash(), 13.0, ...)` — the tighter
/// spacing left under a logical pixel between the two ridges' stroked
/// bands, which the rasteriser's antialiasing bridged completely: the two
/// lines merged into one soft column instead of reading as two. Confirmed
/// by rendering this path at its real size and nearest-neighbour-upscaling
/// the actual pixels (no smoothing), so the merge — and this fix — were
/// visible rather than inferred.
#[must_use]
pub fn trash() -> IconData {
    icon("M4.4 6.6h15.2M9.4 6.6V4.3h5.2v2.3M6.5 6.6l1 13.1h9l1-13.1M8.0 10.2v6M16.0 10.2v6")
}

/// The dark-mode switch: a crescent.
#[must_use]
pub fn moon() -> IconData {
    icon("M20 14.6A8.6 8.6 0 0 1 9.4 4a8.6 8.6 0 1 0 10.6 10.6z")
}

/// The bottom panel: a frame with a band along the bottom.
#[must_use]
pub fn panel() -> IconData {
    icon("M3.6 4.6h16.8v14.8H3.6zM3.6 14.6h16.8")
}

/// The right pane: a frame with a band down the side.
#[must_use]
pub fn preview() -> IconData {
    icon("M3.6 4.6h16.8v14.8H3.6zM14.4 4.6v14.8")
}

/// The editor split: a frame divided down the middle.
#[must_use]
pub fn split() -> IconData {
    icon("M3.6 4.6h16.8v14.8H3.6zM12 4.6v14.8")
}

/// Collapse all: a double chevron.
#[must_use]
pub fn collapse_all() -> IconData {
    icon("M6.2 10.4 12 5.6l5.8 4.8M6.2 18.4 12 13.6l5.8 4.8")
}

/// Docs: an open book.
#[must_use]
pub fn book() -> IconData {
    icon(
        "M12 7.4a3 3 0 0 0-3-3H4.3v12.8H9a3 3 0 0 1 3 3M12 7.4a3 3 0 0 1 3-3h4.7v12.8H15a3 3 0 0 \
         0-3 3M12 7.4v12.8",
    )
}

/// A file, in a list or on a tab.
#[must_use]
pub fn file() -> IconData {
    icon("M6.2 3.6h7.4l4.2 4.3v12.5H6.2zM13.6 3.6v4.3h4.2")
}

/// Save: a diskette.
#[must_use]
pub fn save() -> IconData {
    icon("M5.4 4.4h11.2l3 3v12.2H5.4zM8.6 4.4v5h6.8v-5M8.2 19.6v-6.2h7.6v6.2")
}

/// Source control: a branch, as two nodes joined round a merge.
#[must_use]
pub fn branch() -> IconData {
    built!({
        let mut d = circle(7.0, 5.6, 2.6);
        d.push_str(&circle(7.0, 18.4, 2.6));
        d.push_str(&circle(17.0, 5.6, 2.6));
        d.push_str("M7 8.2v7.6M17 8.2v2.2a3.8 3.8 0 0 1-3.8 3.8H9.6");
        icon(&d)
    })
}

/// Export: a tray with an arrow leaving it.
#[must_use]
pub fn export() -> IconData {
    icon("M12 15.2V3.6M8.4 7.2 12 3.6l3.6 3.6M4.4 14.4v6h15.2v-6")
}

/// The safe-area toggle: a frame with its content inset.
#[must_use]
pub fn safe_area() -> IconData {
    icon("M3.6 4.6h16.8v14.8H3.6zM7.2 8.2h9.6v7.6H7.2z")
}

/// Lessons: a lightbulb.
#[must_use]
pub fn lightbulb() -> IconData {
    icon(
        "M9.4 18.4h5.2M10.4 21h3.2M12 3.4a6 6 0 0 0-3.5 10.9c.6.5 1 1.2 1 2v2.1h5v-2.1c0-.8.4-1.5 \
         1-2A6 6 0 0 0 12 3.4z",
    )
}

/// A chevron pointing down: a collapsed/expanded disclosure.
///
/// The framework ships filled chevrons and this set does not use them, for the
/// reason at the top of the file: one stroked glyph beside one filled one in
/// the same row is the mismatch a mixed set always produces.
#[must_use]
pub fn chevron_down() -> IconData {
    icon("M6.6 9.4 12 14.8l5.4-5.4")
}

/// A chevron pointing right.
#[must_use]
pub fn chevron_right() -> IconData {
    icon("M9.4 6.6 14.8 12l-5.4 5.4")
}

/// A tick.
#[must_use]
pub fn check() -> IconData {
    icon("M4.8 12.4 9.6 17.2 19.2 6.8")
}

/// A cross: close, dismiss, remove.
#[must_use]
pub fn close() -> IconData {
    icon("M6.4 6.4 17.6 17.6M17.6 6.4 6.4 17.6")
}

/// A plus: new file, add.
#[must_use]
pub fn plus() -> IconData {
    icon("M12 5.4v13.2M5.4 12h13.2")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The placeholder is a crossed box, and a real icon must not be one — this
    /// is what catches a typo in a `d` string, which otherwise draws a
    /// perfectly valid rectangle nobody looks twice at.
    #[test]
    fn every_path_parses() {
        let placeholder = fallback();
        let all = [
            ("folder", folder()),
            ("search", search()),
            ("code", code()),
            ("warning", warning()),
            ("dashboard", dashboard()),
            ("tune", tune()),
            ("swatch", swatch()),
            ("gear", gear()),
            ("play", play()),
            ("phone", phone()),
            ("clock", clock()),
            ("shield", shield()),
            ("error", error()),
            ("trash", trash()),
            ("moon", moon()),
            ("panel", panel()),
            ("preview", preview()),
            ("split", split()),
            ("collapse_all", collapse_all()),
            ("book", book()),
            ("file", file()),
            ("save", save()),
            ("branch", branch()),
            ("export", export()),
            ("safe_area", safe_area()),
            ("lightbulb", lightbulb()),
        ];
        for (name, data) in all {
            assert!(
                data.path().verbs() != placeholder.path().verbs(),
                "{name} did not parse"
            );
            assert!(!data.path().is_empty(), "{name} is empty");
        }
    }

    /// Every shape has to sit inside the grid it declares, or it is clipped by
    /// whatever box it is drawn in — and a stroke straddles its path, so the
    /// margin has to be at least half a pen.
    #[test]
    fn every_path_stays_on_the_grid() {
        for (name, data) in [
            ("folder", folder()),
            ("gear", gear()),
            ("tune", tune()),
            ("branch", branch()),
            ("clock", clock()),
            ("shield", shield()),
            ("lightbulb", lightbulb()),
        ] {
            let bounds = data.path().bounds();
            assert!(
                bounds.left >= 1.0 && bounds.top >= 1.0,
                "{name} starts at {bounds:?}"
            );
            assert!(
                bounds.right <= 23.0 && bounds.bottom <= 23.0,
                "{name} ends at {bounds:?}"
            );
        }
    }

    /// The weight is a design-grid number, so it is meaningful only against
    /// the grid the paths are drawn on. A change to either without the other
    /// is a change to the weight of every icon in the studio.
    #[test]
    fn the_weight_is_in_grid_units() {
        assert!((WEIGHT - 1.6).abs() < f32::EPSILON);
        assert_eq!(folder().viewbox().width(), 24.0);
    }
}
