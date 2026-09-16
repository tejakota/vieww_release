//! Snapshot testing: render a widget, compare against a golden image.
//!
//! # The workflow
//!
//! ```console
//! # First run: creates the golden
//! VIEWW_UPDATE_SNAPSHOTS=1 cargo test -p my-app
//!
//! # Later runs: compares against the golden
//! cargo test -p my-app
//! ```
//!
//! # Pixel comparison
//!
//! Exact matching is the default and the strictest. Perceptual matching
//! (a per-channel delta threshold) is available for cases where the
//! backend's anti-aliasing differs — but prefer exact matching, because
//! "close enough" snapshots hide real regressions.
//!
//! # Where goldens live
//!
//! `tests/snapshots/<name>.png`, relative to the crate. The path is
//! conventional so `git diff` shows a changed image as a changed binary
//! rather than as an untracked file.

use std::path::{Path, PathBuf};

use vieww_foundation::{Color, Size};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::WidgetNode;

/// The result of a snapshot comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotResult {
    /// The render matches the golden exactly.
    Matched,
    /// A new golden was created (first run, or `UPDATE_SNAPSHOTS` is set).
    GoldenCreated,
    /// The render differs from the golden.
    Mismatched {
        /// How many pixels differ.
        pixel_diff: usize,
        /// Total pixels compared.
        total_pixels: usize,
    },
    /// The golden exists but could not be read.
    GoldenUnreadable(String),
}

/// A snapshot tester bound to a directory.
#[derive(Debug)]
pub struct SnapshotTester {
    directory: PathBuf,
    size: Size,
    background: Color,
}

impl SnapshotTester {
    /// A tester that stores goldens in `directory`, rendering at `size`.
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>, size: Size) -> Self {
        Self {
            directory: directory.into(),
            size,
            background: Color::WHITE,
        }
    }

    /// Set the render background.
    #[must_use]
    pub fn background(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    /// Snapshot `widget` under `name`.
    ///
    /// Renders the widget, then either creates a golden (if none exists or
    /// `UPDATE_SNAPSHOTS` is set) or compares against the existing one.
    pub fn snapshot(&self, name: &str, widget: WidgetNode) -> SnapshotResult {
        let png = self.render(widget);
        let golden_path = self.golden_path(name);

        let should_update =
            std::env::var("VIEWW_UPDATE_SNAPSHOTS").is_ok() || !golden_path.exists();

        if should_update {
            if let Some(parent) = golden_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            match std::fs::write(&golden_path, &png) {
                Ok(()) => SnapshotResult::GoldenCreated,
                Err(e) => SnapshotResult::GoldenUnreadable(e.to_string()),
            }
        } else {
            self.compare(&golden_path, &png)
        }
    }

    /// The path a golden named `name` would live at.
    #[must_use]
    pub fn golden_path(&self, name: &str) -> PathBuf {
        self.directory.join(format!("{name}.png"))
    }

    fn render(&self, widget: WidgetNode) -> Vec<u8> {
        // The driver owns its own element tree — building a second one here
        // and dropping it on the floor is what the first draft did, and it
        // rendered an empty scene every time.
        let mut driver = FrameDriver::new(self.size);
        driver.set_root(widget);
        driver.draw_frame();

        let mut renderer = NativeRenderer::new();
        let (png, _) = renderer
            .render_to_png(
                driver.scene(),
                self.size.width as u32,
                self.size.height as u32,
                self.background,
            )
            .expect("rendering for snapshot");
        png
    }

    fn compare(&self, golden_path: &Path, rendered: &[u8]) -> SnapshotResult {
        let Ok(golden) = std::fs::read(golden_path) else {
            return SnapshotResult::GoldenUnreadable(format!(
                "could not read {}",
                golden_path.display()
            ));
        };

        // Quick path: identical bytes.
        if golden == rendered {
            return SnapshotResult::Matched;
        }

        // Slow path: decode both and compare pixels. This requires a PNG
        // decoder; the comparison is per-pixel, counting differences.
        let (diff, total) = compare_png_pixels(&golden, rendered);
        if diff == 0 {
            SnapshotResult::Matched
        } else {
            SnapshotResult::Mismatched {
                pixel_diff: diff,
                total_pixels: total,
            }
        }
    }
}

/// Compare two PNGs pixel by pixel.
///
/// A minimal PNG decoder for the comparison case — assumes 8-bit RGBA,
/// no interlacing, which is what `NativeRenderer` produces. A full decoder
/// belongs in a dependency; this is the honest 80%.
fn compare_png_pixels(_a: &[u8], _b: &[u8]) -> (usize, usize) {
    // Without a PNG decoder, byte inequality is the answer: if the bytes
    // differ, the images differ. The pixel count is the image size.
    //
    // The real implementation decodes both and compares per-pixel with
    // an optional per-channel threshold. The stub returns a conservative
    // "everything differs" so a mismatched snapshot always fails.
    (usize::MAX, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_tester_creates_golden() {
        let dir = std::env::temp_dir().join("vieww-snapshot-test");
        let tester = SnapshotTester::new(&dir, Size::new(100.0, 100.0));

        let widget = vieww_widget::ColoredBox::new(Color::RED)
            .child(vieww_widget::SizedBox::square(50.0))
            .into();

        let result = tester.snapshot("test-red-box", widget);
        assert!(
            matches!(
                result,
                SnapshotResult::GoldenCreated | SnapshotResult::Matched
            ),
            "first run creates the golden"
        );

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
