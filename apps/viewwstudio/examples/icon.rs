//! Draw the application icon, at every size a packager asks for.
//!
//! # Why the icon is generated rather than committed as a binary
//!
//! An application with no icon is an application with the operating system's
//! grey placeholder in the dock, the alt-tab strip, the taskbar and the
//! installer — and the studio had no icon file of **any** kind: no `.icns`, no
//! `.ico`, no PNG, nothing for a `.desktop` entry to point at.
//!
//! Committing a PNG would fix that and leave the repository with a picture
//! nobody can edit. This draws it instead, from the studio's own theme colours,
//! with the same `png` writer the screenshot harness uses — so the icon is
//! recoloured by changing a constant, reproducible from source, and reviewable
//! as a diff.
//!
//! ```text
//! cargo run -p viewwstudio --example icon -- packaging/icons
//! ```
//!
//! Writes `icon-16.png` through `icon-1024.png`. `packaging/package.sh` turns
//! those into the per-platform containers (`.icns`, `.ico`) where the tools to
//! do so exist, and falls back to shipping the PNGs where they do not.
//!
//! # The mark
//!
//! Two overlapping rounded panels on a dark ground — the editor and the
//! preview, which is what the application *is*. It reads at 16 pixels because
//! it is two shapes and one accent, which is the only thing that survives that
//! size. Anything with a glyph in it would be a smudge.

use std::path::PathBuf;

/// The sizes every desktop platform between them asks for.
const SIZES: [u32; 8] = [16, 32, 48, 64, 128, 256, 512, 1024];

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "packaging/icons".to_owned()),
    );
    std::fs::create_dir_all(&out).expect("the output directory");

    for size in SIZES {
        let pixels = draw(size);
        let path = out.join(format!("icon-{size}.png"));
        viewwstudio::png::write(&path, &pixels, size, size).expect("writing the PNG");
        println!("{}", path.display());
    }
}

/// The ground, from `StudioTheme::dark().chrome_0`.
const GROUND: [u8; 4] = [0x14, 0x16, 0x1A, 0xFF];
/// The editor panel.
const PANEL: [u8; 4] = [0x46, 0x4E, 0x5E, 0xFF];
/// The preview panel, and the one piece of colour.
const ACCENT: [u8; 4] = [0x7E, 0x5C, 0xE8, 0xFF];

/// One icon, as RGBA.
///
/// Everything is in fractions of `size` so the shape is identical at 16 and at
/// 1024 — an icon hand-tuned per size is an icon that drifts.
#[must_use]
pub fn draw(size: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    #[expect(
        clippy::cast_precision_loss,
        reason = "icon sizes are small powers of two"
    )]
    let side = size as f32;

    // A rounded square for the ground, so the icon does not have four hard
    // corners on the platforms that do not mask it themselves.
    let ground_radius = side * 0.22;
    let panel_radius = side * 0.08;

    for y in 0..size {
        for x in 0..size {
            #[expect(clippy::cast_precision_loss, reason = "coordinates within an icon")]
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);

            let mut colour = [0u8; 4];
            if inside_rounded(fx, fy, 0.0, 0.0, side, side, ground_radius) {
                colour = GROUND;
                // The editor: a tall panel on the left, inset from the ground.
                if inside_rounded(
                    fx,
                    fy,
                    side * 0.16,
                    side * 0.20,
                    side * 0.40,
                    side * 0.60,
                    panel_radius,
                ) {
                    colour = PANEL;
                }
                // The preview: a shorter panel on the right, overlapping, and
                // drawn second so it sits on top — which is the relationship
                // the two panes actually have on screen.
                if inside_rounded(
                    fx,
                    fy,
                    side * 0.44,
                    side * 0.32,
                    side * 0.40,
                    side * 0.48,
                    panel_radius,
                ) {
                    colour = ACCENT;
                }
            }
            let at = ((y * size + x) * 4) as usize;
            pixels[at..at + 4].copy_from_slice(&colour);
        }
    }
    pixels
}

/// Whether `(x, y)` is inside the rounded rectangle at `(left, top)`.
fn inside_rounded(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    radius: f32,
) -> bool {
    let (right, bottom) = (left + width, top + height);
    if x < left || x > right || y < top || y > bottom {
        return false;
    }
    // Clamp the point into the inner rectangle; the distance from there to the
    // point is the distance to the nearest corner arc.
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let cx = x.clamp(left + radius, right - radius);
    let cy = y.clamp(top + radius, bottom - radius);
    let (dx, dy) = (x - cx, y - cy);
    dx.mul_add(dx, dy * dy) <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_size_produces_the_right_number_of_bytes() {
        for size in [16, 64, 256] {
            assert_eq!(draw(size).len(), (size * size * 4) as usize);
        }
    }

    /// The shape has to be the same at every size — an icon hand-tuned per
    /// size is an icon that drifts.
    #[test]
    fn the_centre_is_the_accent_at_every_size() {
        for size in SIZES {
            let pixels = draw(size);
            let middle = ((size / 2 * size + size / 2) * 4) as usize;
            assert_eq!(
                &pixels[middle..middle + 4],
                &ACCENT,
                "the preview panel covers the centre at {size}"
            );
        }
    }

    /// A corner outside the rounded ground must be transparent, or the icon is
    /// a square on every platform that does not mask it.
    #[test]
    fn the_corners_are_transparent() {
        let size = 256;
        let pixels = draw(size);
        assert_eq!(pixels[3], 0, "top-left is outside the rounded ground");
    }

    #[test]
    fn the_ground_is_opaque_where_it_is_drawn() {
        let size = 256;
        let pixels = draw(size);
        // A point just inside the left edge, vertically centred.
        let at = ((size / 2 * size + 4) * 4) as usize;
        assert_eq!(pixels[at + 3], 0xFF);
    }

    #[test]
    fn a_rounded_rectangle_excludes_its_corners_and_includes_its_middle() {
        assert!(inside_rounded(50.0, 50.0, 0.0, 0.0, 100.0, 100.0, 20.0));
        assert!(!inside_rounded(1.0, 1.0, 0.0, 0.0, 100.0, 100.0, 20.0));
        assert!(!inside_rounded(-1.0, 50.0, 0.0, 0.0, 100.0, 100.0, 20.0));
    }
}
