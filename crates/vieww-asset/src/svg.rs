//! Decoding an SVG document into a [`VectorImage`].
//!
//! # The subset this covers, stated rather than discovered by surprise
//!
//! Structure: `<svg viewBox>`, `<g>` (for grouping and `fill` inheritance
//! only — a group's own `transform` is **not** applied; see below), `<path>`,
//! `<rect>` (with `rx`/`ry`), `<circle>`, `<ellipse>`, `<polygon>`. Each
//! element's own `fill` (a `#rrggbb`/`#rgb` hex colour, or `none`) is read,
//! falling back to the nearest ancestor's, defaulting to black.
//!
//! `<path>`'s `d` is not parsed here at all: it is handed to
//! [`vieww_foundation::parse_path_data`], the same routine the built-in icon
//! set is decoded with, so there is one implementation of that mini-language
//! in the workspace rather than two that can drift. That covers the whole
//! command set — `M`/`L`/`H`/`V`/`C`/`S`/`Z` and also `Q`/`T` (converted
//! exactly to cubics, since every quadratic *is* one) and `A` (elliptical
//! arcs). A command outside it fails with a `Decode` naming the command,
//! rather than silently drawing the wrong shape.
//!
//! # What is deliberately not here, and why one line each
//!
//! - **`<line>`/`<polyline>`, and stroking generally** — this framework's
//!   canvas has no stroke primitive at all yet ([`Canvas::fill_path`] is the
//!   only paint operation a path reaches), so nothing here could honour
//!   `stroke-width` even if it parsed one. Filling would be silently wrong,
//!   not merely incomplete, so these elements are skipped.
//! - **Group `transform`, gradients, `<use>`/`<defs>`/`<clipPath>`, CSS
//!   `style=`, `<text>`, embedded raster images** — each is either a second
//!   parser (CSS) or a second paint primitive (gradients) this workspace
//!   does not have yet. Silently ignored, the same as an unknown attribute;
//!   the shapes that *are* understood still decode correctly around them.
//!
//! This is an icon-and-illustration importer, not a browser's rendering
//! engine — the same scope an icon-SVG importer settles for in
//! practice, made explicit here instead of discovered by a shape that came
//! out wrong.

use std::f32::consts::TAU;

use vieww_foundation::{Color, Offset, Path, Rect, Transform, VectorImage, VectorShape};

use crate::AssetError;

/// Parse an SVG document's text into a [`VectorImage`].
///
/// # Errors
///
/// [`AssetError::Decode`] if the document has no `<svg>` root, no usable
/// `viewBox` (or `width`/`height` to build one from), or a `<path d="…">`
/// using a command this parser does not implement.
pub fn parse_svg(source: &str) -> Result<VectorImage, AssetError> {
    let document = roxmltree::Document::parse(source)
        .map_err(|error| AssetError::Decode(format!("malformed XML: {error}")))?;

    let root = document
        .descendants()
        .find(|node| node.has_tag_name("svg"))
        .ok_or_else(|| AssetError::Decode("no <svg> root element".to_owned()))?;

    let viewbox = viewbox_of(&root)
        .ok_or_else(|| AssetError::Decode("no viewBox and no usable width/height".to_owned()))?;

    let mut shapes = Vec::new();
    for child in root.children() {
        collect_shapes(child, Color::BLACK, &mut shapes)?;
    }

    Ok(VectorImage::new(shapes, viewbox))
}

fn viewbox_of(svg: &roxmltree::Node) -> Option<Rect> {
    if let Some(attr) = svg.attribute("viewBox") {
        let nums: Vec<f32> = attr
            .split([' ', ','])
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if let [x, y, w, h] = nums[..] {
            return Some(Rect::new(x, y, x + w, y + h));
        }
        return None;
    }
    let width: f32 = svg
        .attribute("width")?
        .trim_end_matches("px")
        .parse()
        .ok()?;
    let height: f32 = svg
        .attribute("height")?
        .trim_end_matches("px")
        .parse()
        .ok()?;
    Some(Rect::new(0.0, 0.0, width, height))
}

fn collect_shapes(
    node: roxmltree::Node,
    inherited_fill: Color,
    out: &mut Vec<VectorShape>,
) -> Result<(), AssetError> {
    if !node.is_element() {
        return Ok(());
    }
    let fill = fill_of(&node).unwrap_or(inherited_fill);

    match node.tag_name().name() {
        "g" => {
            for child in node.children() {
                collect_shapes(child, fill, out)?;
            }
        }
        "path" => {
            if let Some(d) = node.attribute("d") {
                if fill != Color::TRANSPARENT {
                    out.push(VectorShape {
                        path: parse_path_data(d)?,
                        color: fill,
                    });
                }
            }
        }
        "rect" => {
            if fill != Color::TRANSPARENT {
                let x = attr_f32(&node, "x").unwrap_or(0.0);
                let y = attr_f32(&node, "y").unwrap_or(0.0);
                let w = attr_f32(&node, "width").unwrap_or(0.0);
                let h = attr_f32(&node, "height").unwrap_or(0.0);
                let radius = attr_f32(&node, "rx")
                    .or_else(|| attr_f32(&node, "ry"))
                    .unwrap_or(0.0);
                let rect = Rect::new(x, y, x + w, y + h);
                let path = if radius > 0.0 {
                    Path::rounded_rect(rect, radius)
                } else {
                    Path::rect(rect)
                };
                out.push(VectorShape { path, color: fill });
            }
        }
        "circle" => {
            if fill != Color::TRANSPARENT {
                let cx = attr_f32(&node, "cx").unwrap_or(0.0);
                let cy = attr_f32(&node, "cy").unwrap_or(0.0);
                let r = attr_f32(&node, "r").unwrap_or(0.0);
                if r > 0.0 {
                    let mut path = Path::arc(Offset::new(cx, cy), r, 0.0, TAU);
                    path.close();
                    out.push(VectorShape { path, color: fill });
                }
            }
        }
        "ellipse" => {
            if fill != Color::TRANSPARENT {
                let cx = attr_f32(&node, "cx").unwrap_or(0.0);
                let cy = attr_f32(&node, "cy").unwrap_or(0.0);
                let rx = attr_f32(&node, "rx").unwrap_or(0.0);
                let ry = attr_f32(&node, "ry").unwrap_or(0.0);
                if rx > 0.0 && ry > 0.0 {
                    // A unit circle at the origin, squashed to the two radii
                    // and moved into place — cheaper than a bespoke ellipse
                    // walker and exact, since an affine map takes a circle's
                    // cubic approximation to an ellipse's.
                    let mut unit = Path::arc(Offset::ZERO, 1.0, 0.0, TAU);
                    unit.close();
                    let path = unit.transformed(
                        Transform::scale(rx, ry).then(Transform::translate(Offset::new(cx, cy))),
                    );
                    out.push(VectorShape { path, color: fill });
                }
            }
        }
        "polygon" => {
            if fill != Color::TRANSPARENT {
                if let Some(points) = node.attribute("points") {
                    if let Some(path) = polygon_path(points) {
                        out.push(VectorShape { path, color: fill });
                    }
                }
            }
        }
        _ => {
            // An unrecognised element (`<defs>`, `<title>`, `<metadata>`,
            // `<use>`, …) contributes nothing, but its children might be
            // meaningful in a document that nests shapes somewhere this
            // parser does not specifically expect — walking them costs
            // nothing when there is nothing there.
            for child in node.children() {
                collect_shapes(child, fill, out)?;
            }
        }
    }
    Ok(())
}

fn attr_f32(node: &roxmltree::Node, name: &str) -> Option<f32> {
    node.attribute(name)?.parse().ok()
}

/// `fill="#rrggbb"` / `#rgb` / `none`. Anything else (a named CSS colour, a
/// `url(#gradient)` reference, `style="fill:…"`) is not handled — the
/// element falls back to whatever it inherits rather than guessing.
fn fill_of(node: &roxmltree::Node) -> Option<Color> {
    let value = node.attribute("fill")?;
    if value.eq_ignore_ascii_case("none") {
        return Some(Color::TRANSPARENT);
    }
    parse_hex_color(value)
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some(Color::rgb(r, g, b))
        }
        3 => {
            let double = |c: char| u8::from_str_radix(&format!("{c}{c}"), 16).ok();
            let mut chars = hex.chars();
            let r = double(chars.next()?)?;
            let g = double(chars.next()?)?;
            let b = double(chars.next()?)?;
            Some(Color::rgb(r, g, b))
        }
        _ => None,
    }
}

fn polygon_path(points: &str) -> Option<Path> {
    let nums: Vec<f32> = points
        .split([' ', ',', '\n', '\t'])
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() < 4 || nums.len() % 2 != 0 {
        return None;
    }
    let mut path = Path::new();
    path.move_to(Offset::new(nums[0], nums[1]));
    for pair in nums[2..].chunks(2) {
        path.line_to(Offset::new(pair[0], pair[1]));
    }
    path.close();
    Some(path)
}

// --------------------------------------------------------------- path data

/// A `<path d="…">` attribute, converted into this crate's own [`Path`].
///
/// The `d` mini-language is parsed by [`vieww_foundation::parse_path_data`] —
/// the same routine the built-in icon set is decoded with — rather than a
/// second implementation living here. That one already covers the commands
/// this module previously rejected: quadratics (`Q`/`T`, converted exactly to
/// the cubics [`Path`] stores, since every quadratic *is* a cubic) and
/// elliptical arcs (`A`). An `<svg>` carrying either now draws the right
/// shape instead of failing to decode.
fn parse_path_data(d: &str) -> Result<Path, AssetError> {
    vieww_foundation::parse_path_data(d)
        .map_err(|err| AssetError::Decode(format!("path data: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_with_no_svg_root_is_a_decode_error() {
        assert!(matches!(
            parse_svg("<not-svg/>"),
            Err(AssetError::Decode(_))
        ));
    }

    #[test]
    fn a_viewbox_is_read_into_a_rect() {
        let image = parse_svg(r#"<svg viewBox="0 0 24 24"></svg>"#).unwrap();
        assert_eq!(image.viewbox(), Rect::new(0.0, 0.0, 24.0, 24.0));
    }

    #[test]
    fn width_and_height_stand_in_for_a_missing_viewbox() {
        let image = parse_svg(r#"<svg width="16" height="16"></svg>"#).unwrap();
        assert_eq!(image.viewbox(), Rect::new(0.0, 0.0, 16.0, 16.0));
    }

    #[test]
    fn a_rect_element_becomes_one_shape() {
        let image = parse_svg(r##"<svg viewBox="0 0 10 10"><rect x="1" y="1" width="8" height="8" fill="#ff0000"/></svg>"##).unwrap();
        assert_eq!(image.shapes().len(), 1);
        assert_eq!(image.shapes()[0].color, Color::rgb(0xff, 0, 0));
        assert_eq!(
            image.shapes()[0].path.bounds(),
            Rect::new(1.0, 1.0, 9.0, 9.0)
        );
    }

    #[test]
    fn a_three_digit_hex_colour_expands_each_channel() {
        let image =
            parse_svg(r##"<svg viewBox="0 0 1 1"><rect width="1" height="1" fill="#0f0"/></svg>"##)
                .unwrap();
        assert_eq!(image.shapes()[0].color, Color::rgb(0, 0xff, 0));
    }

    #[test]
    fn fill_none_drops_the_shape_rather_than_drawing_it_black() {
        let image = parse_svg(
            r#"<svg viewBox="0 0 10 10"><rect width="10" height="10" fill="none"/></svg>"#,
        )
        .unwrap();
        assert!(image.shapes().is_empty());
    }

    #[test]
    fn a_group_fill_is_inherited_by_children_that_do_not_override_it() {
        let image = parse_svg(
            r##"<svg viewBox="0 0 10 10"><g fill="#00ff00"><rect width="4" height="4"/><rect x="5" width="4" height="4" fill="#0000ff"/></g></svg>"##,
        )
        .unwrap();
        assert_eq!(image.shapes().len(), 2);
        assert_eq!(image.shapes()[0].color, Color::rgb(0, 0xff, 0), "inherited");
        assert_eq!(
            image.shapes()[1].color,
            Color::rgb(0, 0, 0xff),
            "overridden locally"
        );
    }

    #[test]
    fn a_circle_is_a_closed_path_centred_on_cx_cy() {
        let image =
            parse_svg(r#"<svg viewBox="0 0 20 20"><circle cx="10" cy="10" r="5"/></svg>"#).unwrap();
        let bounds = image.shapes()[0].path.bounds();
        assert!((bounds.width() - 10.0).abs() < 1e-2, "{bounds:?}");
        assert!((bounds.height() - 10.0).abs() < 1e-2, "{bounds:?}");
    }

    #[test]
    fn a_polygon_closes_back_to_its_first_point() {
        let path = polygon_path("0,0 10,0 10,10 0,10").unwrap();
        assert!(matches!(
            path.verbs().last(),
            Some(vieww_foundation::PathVerb::Close)
        ));
    }

    #[test]
    fn path_data_moveto_lineto_builds_two_points() {
        let path = parse_path_data("M0 0 L10 10").unwrap();
        assert_eq!(
            path.verbs(),
            &[
                vieww_foundation::PathVerb::MoveTo(Offset::ZERO),
                vieww_foundation::PathVerb::LineTo(Offset::new(10.0, 10.0)),
            ]
        );
    }

    #[test]
    fn path_data_relative_commands_accumulate_from_the_cursor() {
        let path = parse_path_data("m5 5 l1 1 l1 1").unwrap();
        assert_eq!(
            path.verbs(),
            &[
                vieww_foundation::PathVerb::MoveTo(Offset::new(5.0, 5.0)),
                vieww_foundation::PathVerb::LineTo(Offset::new(6.0, 6.0)),
                vieww_foundation::PathVerb::LineTo(Offset::new(7.0, 7.0)),
            ]
        );
    }

    #[test]
    fn path_data_implicit_lineto_follows_a_bare_moveto_pair() {
        // "M0 0 10 10" — the second pair has no command letter of its own,
        // and the spec says it inherits an implicit `L`.
        let path = parse_path_data("M0 0 10 10").unwrap();
        assert_eq!(path.verbs().len(), 2);
        assert!(matches!(
            path.verbs()[1],
            vieww_foundation::PathVerb::LineTo(_)
        ));
    }

    #[test]
    fn path_data_h_and_v_move_along_one_axis_only() {
        let path = parse_path_data("M5 5 H20 V30").unwrap();
        assert_eq!(
            path.verbs(),
            &[
                vieww_foundation::PathVerb::MoveTo(Offset::new(5.0, 5.0)),
                vieww_foundation::PathVerb::LineTo(Offset::new(20.0, 5.0)),
                vieww_foundation::PathVerb::LineTo(Offset::new(20.0, 30.0)),
            ]
        );
    }

    #[test]
    fn path_data_s_reflects_the_previous_cubics_control_point() {
        // After `C … 10,0 10,10`, an `S` with target `20,20` and its own
        // second control `20,10` should reflect `10,0` through `10,10` to
        // get its own first control: `(2*10-10, 2*10-0) = (10, 20)`.
        let path = parse_path_data("M0 0 C0 0 10 0 10 10 S20 10 20 20").unwrap();
        let vieww_foundation::PathVerb::CubicTo(c1, _, _) = path.verbs()[2] else {
            panic!("expected a cubic");
        };
        assert_eq!(c1, Offset::new(10.0, 20.0));
    }

    #[test]
    fn path_data_z_closes_and_returns_the_cursor_to_the_subpath_start() {
        let path = parse_path_data("M0 0 L10 0 L10 10 Z L5 5").unwrap();
        // After Z the cursor is back at (0,0), so the trailing L lands
        // relative to *that* start, not to (10,10).
        assert_eq!(
            path.verbs().last(),
            Some(&vieww_foundation::PathVerb::LineTo(Offset::new(5.0, 5.0)))
        );
    }

    #[test]
    fn an_elliptical_arc_is_parsed_rather_than_rejected() {
        // This used to be a `Decode` error, back when this module carried its
        // own path parser covering only M/L/H/V/C/S/Z. Delegating to
        // `vieww_foundation::parse_path_data` means `A` is a real shape now:
        // approximated as cubics, which is the only curve `Path` stores.
        let path = parse_path_data("M0 0 A5 5 0 0 1 10 10").expect("arcs parse");
        assert!(
            path.verbs()
                .iter()
                .any(|verb| matches!(verb, vieww_foundation::PathVerb::CubicTo(..))),
            "an arc should land as cubics, got {:?}",
            path.verbs()
        );
    }

    #[test]
    fn a_genuinely_unknown_command_is_a_named_decode_error_not_a_wrong_shape() {
        let err = parse_path_data("M0 0 K5 5").unwrap_err();
        assert!(
            matches!(&err, AssetError::Decode(msg) if msg.contains('K')),
            "{err}"
        );
    }
}
