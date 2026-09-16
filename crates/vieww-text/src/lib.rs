//! The text layer — turning strings into positioned glyphs.
//!
//! Text is the hardest part of a UI toolkit and the usual weak point of Rust
//! ones, so it gets its own layer rather than being a corner of the canvas. What
//! it owns: loading fonts, choosing fallbacks, shaping, breaking lines, and
//! ordering bidirectional runs.
//!
//! # What comes out
//!
//! [`GlyphRun`](vieww_foundation::GlyphRun)s — positioned glyph ids, ready to
//! draw. Nothing downstream sees a character again. That boundary is deliberate:
//! if the paint layer received strings it would have to shape them to draw them,
//! and any disagreement with what layout measured shows up as text overflowing
//! its box.
//!
//! # Bidirectionality is not a feature
//!
//! A run carries its own direction, and a single line can hold runs of both. This
//! is built in from the start rather than added later because it decides the shape
//! of the data model: "a line is a string plus an x position" cannot represent
//! Hebrew with an embedded English phrase, and discovering that after the fact
//! means rewriting layout, hit testing, caret movement and selection together.
//!
//! # Where the work actually happens
//!
//! `cosmic-text` does the shaping, line breaking, bidi resolution and font
//! fallback. It is the single best reuse candidate in this project — a correct
//! implementation of all four is years of work and a permanent maintenance
//! obligation. This crate's job is to adapt it to the framework's vocabulary, and
//! to be a seam that could be swapped for a lower-level stack (`rustybuzz` +
//! `swash`) if a specific need ever demanded it.
//!
//! ```
//! use vieww_foundation::{TextStyle, Size};
//! use vieww_text::{FontStore, Paragraph, TextSpan};
//!
//! let mut fonts = FontStore::embedded_only();
//! let spans = [TextSpan::new("hello there world", TextStyle::new(16.0))];
//!
//! // Unbounded: one line.
//! let wide = Paragraph::layout(&mut fonts, &spans, f32::INFINITY);
//! assert_eq!(wide.line_count(), 1);
//!
//! // Narrow enough to force a break.
//! let narrow = Paragraph::layout(&mut fonts, &spans, 60.0);
//! assert!(narrow.line_count() > 1);
//! assert!(narrow.size().width <= 60.0);
//! ```
//!
//! # The second stack that used to be here
//!
//! This crate shipped a `shaping`/`layout`/`selection`/`editing`/`bidi`/`fonts`
//! set of modules alongside the adapter above: a shaper-agnostic interface
//! "intended to eventually sit under" it. It was deleted, and the reason is
//! worth keeping so it is not started again in the same shape.
//!
//! It was never reached. `layout::Paragraph` hard-coded `NaiveShaper` — one
//! glyph per code point, no font metrics — and no widget, render object or
//! test outside those six files ever called into any of them; the crate root
//! re-exported a second `FontFamily` and `FontWeight` that shadowed
//! `vieww-foundation`'s in every file that glob-imported this crate. Its own
//! module doc said it was in progress, which is honest, and the previous
//! `TRACKER.md` entry warned readers not to mistake `NaiveShaper` for the
//! framework's shaping — which is a warning label on a hazard rather than the
//! removal of one.
//!
//! Two smaller things went with it, and both are the reason this is a deletion
//! rather than a `#[deprecated]`. `bidi`'s module comment claimed
//! `direction_of` was "still public, and is used by the real path" — nothing
//! called it, in this crate or any other. And `RichSpan`/`SpanStyle`/
//! `LineBreak` at the crate root existed only to serve `layout`.
//!
//! If a lower-level stack is ever wanted — `rustybuzz` and `swash` under this
//! crate's own vocabulary — the seam to build it behind is `Paragraph`, and
//! the way to know it works is to make the existing text tests pass through
//! it. A parallel module tree that nothing calls cannot be known to work at
//! all, which is what this one demonstrated for as long as it existed.

mod elide;
mod font;
mod lines;
mod paragraph;
mod shape_cache;

// `shaping_improved` — a second `Shaper` implementation, `SmartShaper` — was
// deleted on 2026-08-30 rather than wired in, and the reason is worth keeping
// here so it is not written again. (The `shaping` module it would have joined
// is itself gone now; see the crate doc's "The second stack that used to be
// here". The argument below is why, applied one implementation earlier.)
//
// It emitted **Unicode code points as glyph identifiers** and a flat
// `size * 0.6` advance for every glyph. Both are stand-ins, and its own module
// header said so. A glyph id is an index into a particular font's `glyf`/`CFF`
// table and has no relationship to a code point in any font, so every glyph it
// produced would have drawn whatever happened to live at that index — and a
// fixed advance means no font's actual metrics reach the line breaker.
//
// What made it dangerous rather than merely unused is that its **Arabic joining
// tests passed**. They asserted on the joining forms it selected, which is real
// logic and was correct, against glyph ids that could never render. A reader
// finding a green Arabic shaping suite in this crate would reasonably conclude
// the framework shapes Arabic. Shaping is delegated to `cosmic-text`, which does
// it properly; the joining logic here was a second, worse implementation of what
// that library already contains.

pub use elide::elide_to_width;
pub use font::{
    FontStore, EMBEDDED_CJK, EMBEDDED_FACES, EMBEDDED_FAMILY, EMBEDDED_FONT, EMBEDDED_FONT_BOLD,
    EMBEDDED_FONT_ITALIC, EMBEDDED_MONO, EMBEDDED_MONO_BOLD, EMBEDDED_MONO_FAMILY,
};
pub use paragraph::{Paragraph, TextSpan};

pub use vieww_foundation as foundation;

/// What to do when text does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextOverflow {
    /// Let it run past the boundary.
    #[default]
    Visible,
    /// Clip at the boundary.
    Clip,
    /// Truncate and append "…". Requires `max_lines` to be meaningful.
    Ellipsis,
    /// Take the ellipsis out of the *middle*, keeping both ends.
    ///
    /// # Why a second ellipsis
    ///
    /// For names rather than prose. `login_screen.rs` and `login_screen_test.rs`
    /// differ in the middle and agree at both ends, so cutting at the right
    /// leaves two tabs both reading `login_screen…`; and the end of a file name
    /// is the part that says what it is. Anything with a distinguishing head and
    /// a meaningful tail — file names, paths, identifiers — wants this and not
    /// [`TextOverflow::Ellipsis`].
    ///
    /// Single-line by nature: a middle cut across several lines has no meaning.
    EllipsisMiddle,
    /// Fade the last visible line out.
    Fade,
}

impl TextOverflow {
    /// Whether this mode replaces text that does not fit with an ellipsis.
    #[must_use]
    pub const fn elides(self) -> bool {
        matches!(self, Self::Ellipsis | Self::EllipsisMiddle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TextOverflow::elides` is the only thing left at this level with a
    /// rule rather than a shape, and both ellipsis modes have to answer to
    /// it: `EllipsisMiddle` was added after `elides` existed, and a mode that
    /// truncates without saying so reaches the render objects as text that
    /// quietly loses its tail.
    #[test]
    fn both_ellipsis_modes_report_that_they_elide() {
        assert!(TextOverflow::Ellipsis.elides());
        assert!(TextOverflow::EllipsisMiddle.elides());
        assert!(!TextOverflow::Visible.elides());
        assert!(!TextOverflow::Clip.elides());
        assert!(!TextOverflow::Fade.elides());
    }
}
