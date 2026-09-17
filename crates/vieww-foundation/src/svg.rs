//! Reading SVG path data into a [`Path`].
//!
//! The `d` attribute of an SVG `<path>`, which is where an icon's shape actually
//! lives. Everything else about an icon — the box it was drawn in, how it scales
//! into a widget — is already [`IconData`](crate::IconData)'s job, so this is the
//! one piece that was missing.
//!
//! ```
//! use vieww_foundation::{parse_path_data, IconData};
//!
//! // A tick, on the 24x24 grid the built-in icons use.
//! let path = parse_path_data("M9 16.17 4.83 12 3.41 13.41 9 19 21 7l-1.41-1.41z")
//!     .expect("valid path data");
//! let tick = IconData::square24(path);
//! ```
//!
//! # Curves are cubics, because [`Path`] only has cubics
//!
//! Quadratics (`Q`, `T`) are converted on the way in, exactly as `Path`'s own
//! documentation says they should be: one curve type instead of two halves the
//! match arms in every backend for no loss of shape. The conversion is exact —
//! every quadratic *is* a cubic — so nothing is approximated here.
//!
//! # Elliptical arcs, which are the one command that is not a translation
//!
//! `A` gives the endpoint it wants and two radii, and leaves the centre to be
//! worked out — so it is converted to centre parameters (SVG F.6.5), split into
//! pieces of at most a quarter turn, and each piece approximated by a cubic. A
//! cubic tracks a circular quarter to within about one part in a thousand and
//! degrades visibly past a half turn, which is where the split comes from.
//!
//! Three repairs the specification asks for, all of which are silent by design
//! and none of which is an error:
//!
//! - **A zero radius is a straight line** (F.6.6.1), not a degenerate curve.
//! - **Radii too small to span the endpoints are scaled up** until they exactly
//!   reach (F.6.6.2), rather than the arc being dropped.
//! - **An arc whose endpoints coincide is not drawn at all** (F.6.2) — it has no
//!   defined centre, and a `0`-length arc is what an author meant by writing
//!   one.
//!
//! **The last segment lands on the commanded endpoint exactly**, rather than on
//! whatever the trigonometry arrives at. Otherwise a following command starts a
//! fraction away from where the author said, and closed shapes show hairline
//! gaps that look like a rasteriser bug.
//!
//! # What this deliberately does not do
//!
//! Parse XML. `<svg>`, `<g>`, transforms, strokes, gradients and CSS are a
//! document format, not a shape, and pulling them in means pulling in a parser
//! and a styling model. An application that needs them can read the `d` out with
//! any XML crate and hand it here.

use core::f32::consts::{FRAC_PI_2, TAU};
use core::fmt;

use crate::{Offset, Path};

/// Why some path data could not be read.
///
/// Every variant names something specific enough to fix. A parser that returned
/// one opaque "invalid path" for all of these would be telling an author their
/// icon is broken without saying where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgPathError {
    /// Numbers appeared with no command to apply them to.
    NumberWithoutCommand,
    /// Path data must begin with a moveto — `M` or `m`.
    MissingInitialMoveTo,
    /// A letter that is not an SVG path command.
    UnknownCommand(char),
    /// An arc's large-arc or sweep flag was neither `0` nor `1`.
    ///
    /// Its own variant because the flags are the one place the grammar is not
    /// numbers: they are single characters and may run straight into the
    /// coordinates after them, so a wrong one is a different mistake from a
    /// missing number.
    MalformedArcFlag,
    /// A command that did not get all the numbers it needs.
    TruncatedCommand(char),
}

impl fmt::Display for SvgPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NumberWithoutCommand => {
                f.write_str("coordinates with no command to apply them to")
            }
            Self::MissingInitialMoveTo => {
                f.write_str("path data must begin with a moveto (M or m)")
            }
            Self::UnknownCommand(command) => {
                write!(f, "'{command}' is not an SVG path command")
            }
            Self::MalformedArcFlag => {
                f.write_str("an arc's large-arc and sweep flags must be 0 or 1")
            }
            Self::TruncatedCommand(command) => {
                write!(f, "'{command}' ran out of numbers")
            }
        }
    }
}

impl std::error::Error for SvgPathError {}

/// Read SVG path data into a [`Path`].
///
/// # Errors
///
/// [`SvgPathError`], which names what went wrong rather than reporting that
/// something did.
pub fn parse_path_data(data: &str) -> Result<Path, SvgPathError> {
    let mut scanner = Scanner::new(data);
    let mut path = Path::new();

    // Where the pen is, and where the current subpath began — `Z` returns to
    // the latter, and so does the point a following command starts from.
    let mut cursor = Offset::ZERO;
    let mut subpath_start = Offset::ZERO;

    // The control point a smooth command reflects. Set only by the curve that
    // owns it and cleared by everything else, which is exactly the spec's rule
    // that `S` reflects only after `C` or `S`, and `T` only after `Q` or `T`.
    let mut cubic_control: Option<Offset> = None;
    let mut quad_control: Option<Offset> = None;

    // The command to repeat when numbers keep coming. `M` repeats as `L`,
    // because "moveto, then more pairs" means a polyline and not a series of
    // moves — the single most surprising rule in the grammar.
    let mut repeat: Option<char> = None;
    let mut started = false;

    loop {
        scanner.skip_separators();
        if scanner.at_end() {
            break;
        }

        let command = match scanner.take_command() {
            Some(letter) => {
                repeat = Some(match letter {
                    'M' => 'L',
                    'm' => 'l',
                    other => other,
                });
                letter
            }
            None => repeat.ok_or(SvgPathError::NumberWithoutCommand)?,
        };

        if !started && !matches!(command, 'M' | 'm') {
            return Err(SvgPathError::MissingInitialMoveTo);
        }

        let relative = command.is_ascii_lowercase();
        let base = if relative { cursor } else { Offset::ZERO };

        match command.to_ascii_uppercase() {
            'M' => {
                let point = base + scanner.point(command)?;
                path.move_to(point);
                cursor = point;
                subpath_start = point;
                started = true;
                cubic_control = None;
                quad_control = None;
            }
            'L' => {
                let point = base + scanner.point(command)?;
                path.line_to(point);
                cursor = point;
                cubic_control = None;
                quad_control = None;
            }
            'H' => {
                let x = scanner.number(command)?;
                let point = Offset::new(base.dx + x, cursor.dy);
                path.line_to(point);
                cursor = point;
                cubic_control = None;
                quad_control = None;
            }
            'V' => {
                let y = scanner.number(command)?;
                let point = Offset::new(cursor.dx, base.dy + y);
                path.line_to(point);
                cursor = point;
                cubic_control = None;
                quad_control = None;
            }
            'C' => {
                let first = base + scanner.point(command)?;
                let second = base + scanner.point(command)?;
                let end = base + scanner.point(command)?;
                path.cubic_to(first, second, end);
                cursor = end;
                cubic_control = Some(second);
                quad_control = None;
            }
            'S' => {
                // No preceding cubic means the first control point sits on the
                // current point, which is the spec's answer and makes the curve
                // leave the pen in the direction of its second control.
                let first = reflect(cubic_control, cursor);
                let second = base + scanner.point(command)?;
                let end = base + scanner.point(command)?;
                path.cubic_to(first, second, end);
                cursor = end;
                cubic_control = Some(second);
                quad_control = None;
            }
            'Q' => {
                let control = base + scanner.point(command)?;
                let end = base + scanner.point(command)?;
                let (first, second) = quadratic_as_cubic(cursor, control, end);
                path.cubic_to(first, second, end);
                cursor = end;
                quad_control = Some(control);
                cubic_control = None;
            }
            'T' => {
                let control = reflect(quad_control, cursor);
                let end = base + scanner.point(command)?;
                let (first, second) = quadratic_as_cubic(cursor, control, end);
                path.cubic_to(first, second, end);
                cursor = end;
                quad_control = Some(control);
                cubic_control = None;
            }
            'Z' => {
                path.close();
                cursor = subpath_start;
                cubic_control = None;
                quad_control = None;
                // `Z` takes no numbers, so nothing may repeat it. Anything but a
                // command next is an error rather than an infinite loop.
                repeat = None;
            }
            'A' => {
                let rx = scanner.number(command)?;
                let ry = scanner.number(command)?;
                let rotation = scanner.number(command)?;
                // Flags before coordinates, and read as single characters:
                // `0110 0` is two flags and then a 10, which a number scanner
                // would swallow whole.
                let large = scanner.flag()?;
                let sweep = scanner.flag()?;
                let end = base + scanner.point(command)?;
                Arc {
                    from: cursor,
                    to: end,
                    rx,
                    ry,
                    rotation: rotation.to_radians(),
                    large,
                    sweep,
                }
                .append_to(&mut path);
                cursor = end;
                cubic_control = None;
                quad_control = None;
            }
            // `command`, not the uppercased letter the match arm sees: an
            // author who wrote `x` should be told about `x`.
            _ => return Err(SvgPathError::UnknownCommand(command)),
        }
    }

    Ok(path)
}

/// The control point a smooth command uses: the previous one mirrored through
/// the current point, or the current point when there was no previous one.
fn reflect(previous: Option<Offset>, cursor: Offset) -> Offset {
    match previous {
        Some(control) => Offset::new(
            2.0f32.mul_add(cursor.dx, -control.dx),
            2.0f32.mul_add(cursor.dy, -control.dy),
        ),
        None => cursor,
    }
}

/// The two cubic controls that draw exactly the quadratic `start`-`control`-`end`.
///
/// Exact rather than approximate: every quadratic is a cubic whose controls sit
/// two thirds of the way from each endpoint toward the quadratic's own control.
fn quadratic_as_cubic(start: Offset, control: Offset, end: Offset) -> (Offset, Offset) {
    const TWO_THIRDS: f32 = 2.0 / 3.0;
    let toward = |from: Offset| {
        Offset::new(
            TWO_THIRDS.mul_add(control.dx - from.dx, from.dx),
            TWO_THIRDS.mul_add(control.dy - from.dy, from.dy),
        )
    };
    (toward(start), toward(end))
}

/// One `A` command, resolved from endpoint parameters into cubics.
///
/// A struct rather than seven arguments, which is also what keeps this readable:
/// the specification names these, and the names are the only way to follow
/// F.6.5 alongside it.
struct Arc {
    from: Offset,
    to: Offset,
    rx: f32,
    ry: f32,
    /// The x-axis rotation, in radians.
    rotation: f32,
    large: bool,
    sweep: bool,
}

impl Arc {
    fn append_to(&self, path: &mut Path) {
        // F.6.2: coincident endpoints mean no arc at all. There is no centre to
        // find, and it is what an author writing one meant.
        if (self.from.dx - self.to.dx).abs() < f32::EPSILON
            && (self.from.dy - self.to.dy).abs() < f32::EPSILON
        {
            return;
        }

        // F.6.6.1: a zero radius is a line, not a curve that happens to be flat.
        let (mut rx, mut ry) = (self.rx.abs(), self.ry.abs());
        if rx < f32::EPSILON || ry < f32::EPSILON {
            path.line_to(self.to);
            return;
        }

        let (sin_phi, cos_phi) = self.rotation.sin_cos();

        // F.6.5.1: the half-difference of the endpoints, un-rotated.
        let half_dx = (self.from.dx - self.to.dx) / 2.0;
        let half_dy = (self.from.dy - self.to.dy) / 2.0;
        let x1 = cos_phi.mul_add(half_dx, sin_phi * half_dy);
        let y1 = cos_phi.mul_add(half_dy, -(sin_phi * half_dx));

        // F.6.6.2: radii that cannot span the endpoints are grown until they
        // exactly reach. The spec repairs rather than refuses, because an
        // exported drawing rounding its radii down is ordinary.
        let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
        if lambda > 1.0 {
            let scale = lambda.sqrt();
            rx *= scale;
            ry *= scale;
        }

        // F.6.5.2: the centre, still un-rotated.
        let rx2 = rx * rx;
        let ry2 = ry * ry;
        let denominator = ry2.mul_add(x1 * x1, rx2 * y1 * y1);
        if denominator < f32::EPSILON {
            path.line_to(self.to);
            return;
        }
        let numerator = (rx2 * ry2 - rx2 * y1 * y1 - ry2 * x1 * x1).max(0.0);
        // The two flags together pick which of the four arcs through these two
        // points is meant, and this sign is the whole of that choice.
        let sign = if self.large == self.sweep { -1.0 } else { 1.0 };
        let coefficient = sign * (numerator / denominator).sqrt();
        let cx1 = coefficient * rx * y1 / ry;
        let cy1 = -coefficient * ry * x1 / rx;

        // F.6.5.3: and back into the frame the caller drew in.
        let centre = Offset::new(
            cos_phi.mul_add(cx1, -(sin_phi * cy1)) + (self.from.dx + self.to.dx) / 2.0,
            sin_phi.mul_add(cx1, cos_phi * cy1) + (self.from.dy + self.to.dy) / 2.0,
        );

        // F.6.5.5 and F.6.5.6: where the arc starts on the ellipse, and how far
        // it turns.
        let start_x = (x1 - cx1) / rx;
        let start_y = (y1 - cy1) / ry;
        let end_x = (-x1 - cx1) / rx;
        let end_y = (-y1 - cy1) / ry;

        let theta = angle_between(1.0, 0.0, start_x, start_y);
        let mut swept = angle_between(start_x, start_y, end_x, end_y);
        if !self.sweep && swept > 0.0 {
            swept -= TAU;
        } else if self.sweep && swept < 0.0 {
            swept += TAU;
        }

        // A cubic follows a circular quarter closely and a half badly, so the
        // arc is cut until no piece is more than a quarter turn.
        // See `quarter_pieces` for why this is not a bare `ceil`.
        let count = quarter_pieces(swept);
        let step = swept / count as f32;
        // The tangent scale that makes a cubic meet a circular segment at both
        // ends with the right slope.
        let reach = (4.0 / 3.0) * (step / 4.0).tan();

        let mut angle = theta;
        for index in 0..count {
            let next = angle + step;
            let (from, from_slope) = ellipse_at(centre, rx, ry, sin_phi, cos_phi, angle);
            let (to, to_slope) = ellipse_at(centre, rx, ry, sin_phi, cos_phi, next);

            // The last piece lands on the point the author actually wrote,
            // rather than where the trigonometry arrived. A subpath that closes
            // a fraction short shows a hairline gap that reads as a rasteriser
            // bug.
            let to = if index + 1 == count { self.to } else { to };

            path.cubic_to(
                Offset::new(
                    reach.mul_add(from_slope.dx, from.dx),
                    reach.mul_add(from_slope.dy, from.dy),
                ),
                Offset::new(
                    reach.mul_add(-to_slope.dx, to.dx),
                    reach.mul_add(-to_slope.dy, to.dy),
                ),
                to,
            );
            angle = next;
        }
    }
}

/// How many pieces of at most a quarter turn an arc sweeping `swept` radians
/// is cut into.
///
/// The small allowance before `ceil` is load-bearing. A semicircle whose sweep
/// came from `acos(-1) - TAU` lands one ulp either side of exactly two
/// quarters depending on the platform's libm — Apple's `acosf(-1)` rounds the
/// other way from glibc's — and without it macOS cut the same semicircle into
/// three pieces where Linux cut two. A piece a hair past a quarter turn is
/// still followed closely by a cubic.
fn quarter_pieces(swept: f32) -> usize {
    (swept.abs() / FRAC_PI_2 - 1.0e-4).ceil().max(1.0) as usize
}

/// A point on the ellipse at `angle`, and the tangent there.
///
/// Both already rotated into the frame the caller drew in, so the segment loop
/// needs no trigonometry of its own.
fn ellipse_at(
    centre: Offset,
    rx: f32,
    ry: f32,
    sin_phi: f32,
    cos_phi: f32,
    angle: f32,
) -> (Offset, Offset) {
    let (sin_a, cos_a) = angle.sin_cos();
    let x = rx * cos_a;
    let y = ry * sin_a;
    let slope_x = -rx * sin_a;
    let slope_y = ry * cos_a;
    (
        Offset::new(
            cos_phi.mul_add(x, -(sin_phi * y)) + centre.dx,
            sin_phi.mul_add(x, cos_phi * y) + centre.dy,
        ),
        Offset::new(
            cos_phi.mul_add(slope_x, -(sin_phi * slope_y)),
            sin_phi.mul_add(slope_x, cos_phi * slope_y),
        ),
    )
}

/// The signed angle from one vector to another, as F.6.5.4 defines it.
fn angle_between(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    let dot = ux.mul_add(vx, uy * vy);
    let lengths = (ux.mul_add(ux, uy * uy) * vx.mul_add(vx, vy * vy)).sqrt();
    let angle = (dot / lengths).clamp(-1.0, 1.0).acos();
    if ux.mul_add(vy, -(uy * vx)) < 0.0 {
        -angle
    } else {
        angle
    }
}

/// A cursor over path data, which is not tokenisable ahead of time.
///
/// SVG numbers may run together with no separator at all — `10-20` is two
/// numbers, and `1.5.5` is `1.5` then `.5`, because a second decimal point can
/// only begin a new one. Scanning on demand is what gets those right.
struct Scanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Scanner<'a> {
    const fn new(data: &'a str) -> Self {
        Self {
            bytes: data.as_bytes(),
            pos: 0,
        }
    }

    const fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn skip_separators(&mut self) {
        while let Some(byte) = self.bytes.get(self.pos) {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | b',' => self.pos += 1,
                _ => break,
            }
        }
    }

    /// The next command letter, if the next thing is one.
    fn take_command(&mut self) -> Option<char> {
        self.skip_separators();
        let byte = *self.bytes.get(self.pos)?;
        byte.is_ascii_alphabetic().then(|| {
            self.pos += 1;
            byte as char
        })
    }

    /// One number, or the error for `command` having run out.
    fn number(&mut self, command: char) -> Result<f32, SvgPathError> {
        self.scan_number()
            .ok_or(SvgPathError::TruncatedCommand(command))
    }

    /// One arc flag: a single `0` or `1`.
    ///
    /// **Not a number**, and that is the whole point. The grammar allows
    /// `0110 0` — two flags then a coordinate — so reading these with the number
    /// scanner would take `0110` as one value and quietly draw a different arc.
    fn flag(&mut self) -> Result<bool, SvgPathError> {
        self.skip_separators();
        match self.bytes.get(self.pos) {
            Some(b'0') => {
                self.pos += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err(SvgPathError::MalformedArcFlag),
        }
    }

    /// A coordinate pair.
    fn point(&mut self, command: char) -> Result<Offset, SvgPathError> {
        let x = self.number(command)?;
        let y = self.number(command)?;
        Ok(Offset::new(x, y))
    }

    fn scan_number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.pos;

        if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
            self.pos += 1;
        }

        let mut digits = self.take_digits();
        if matches!(self.bytes.get(self.pos), Some(b'.')) {
            self.pos += 1;
            digits |= self.take_digits();
        }
        if !digits {
            // A sign or a point with nothing after it is not a number, and the
            // cursor must not move or the caller's error would point elsewhere.
            self.pos = start;
            return None;
        }

        // An exponent only counts if it actually has digits: `1e` is the number
        // 1 followed by a command that happens to be invalid, not a broken
        // number, and rewinding is what lets the caller say so.
        if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
            let before_exponent = self.pos;
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.take_digits() {
                self.pos = before_exponent;
            }
        }

        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }

    /// Consume a run of digits, reporting whether there were any.
    fn take_digits(&mut self) -> bool {
        let start = self.pos;
        while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos > start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PathVerb;

    fn verbs(data: &str) -> Vec<PathVerb> {
        parse_path_data(data)
            .expect("valid path data")
            .verbs()
            .to_vec()
    }

    fn at(x: f32, y: f32) -> Offset {
        Offset::new(x, y)
    }

    #[test]
    fn a_move_and_a_line() {
        assert_eq!(
            verbs("M10 20 L30 40"),
            vec![
                PathVerb::MoveTo(at(10.0, 20.0)),
                PathVerb::LineTo(at(30.0, 40.0))
            ]
        );
    }

    #[test]
    fn lowercase_commands_are_relative_to_the_pen() {
        assert_eq!(
            verbs("M10 10 l5 5 l-5 0"),
            vec![
                PathVerb::MoveTo(at(10.0, 10.0)),
                PathVerb::LineTo(at(15.0, 15.0)),
                PathVerb::LineTo(at(10.0, 15.0)),
            ]
        );
    }

    #[test]
    fn extra_pairs_after_a_moveto_are_lines_and_not_moves() {
        // **The most surprising rule in the grammar**, and the one that silently
        // produces a scatter of disconnected points when it is missed.
        assert_eq!(
            verbs("M0 0 10 0 10 10"),
            vec![
                PathVerb::MoveTo(at(0.0, 0.0)),
                PathVerb::LineTo(at(10.0, 0.0)),
                PathVerb::LineTo(at(10.0, 10.0)),
            ]
        );
    }

    #[test]
    fn a_relative_moveto_repeats_as_a_relative_line() {
        assert_eq!(
            verbs("m5 5 5 0"),
            vec![
                PathVerb::MoveTo(at(5.0, 5.0)),
                PathVerb::LineTo(at(10.0, 5.0))
            ]
        );
    }

    #[test]
    fn repeated_numbers_repeat_the_last_command() {
        assert_eq!(
            verbs("M0 0 L1 1 2 2 3 3"),
            vec![
                PathVerb::MoveTo(at(0.0, 0.0)),
                PathVerb::LineTo(at(1.0, 1.0)),
                PathVerb::LineTo(at(2.0, 2.0)),
                PathVerb::LineTo(at(3.0, 3.0)),
            ]
        );
    }

    #[test]
    fn horizontal_and_vertical_keep_the_other_axis() {
        assert_eq!(
            verbs("M5 5 H20 V30 h-5 v-5"),
            vec![
                PathVerb::MoveTo(at(5.0, 5.0)),
                PathVerb::LineTo(at(20.0, 5.0)),
                PathVerb::LineTo(at(20.0, 30.0)),
                PathVerb::LineTo(at(15.0, 30.0)),
                PathVerb::LineTo(at(15.0, 25.0)),
            ]
        );
    }

    #[test]
    fn close_returns_the_pen_to_the_start_of_the_subpath() {
        // Not to the origin, and not staying where it was: a command after `Z`
        // continues from where the subpath began.
        assert_eq!(
            verbs("M10 10 L20 20 Z l5 0"),
            vec![
                PathVerb::MoveTo(at(10.0, 10.0)),
                PathVerb::LineTo(at(20.0, 20.0)),
                PathVerb::Close,
                PathVerb::LineTo(at(15.0, 10.0)),
            ]
        );
    }

    #[test]
    fn a_cubic_is_carried_through_unchanged() {
        assert_eq!(
            verbs("M0 0 C1 2 3 4 5 6"),
            vec![
                PathVerb::MoveTo(at(0.0, 0.0)),
                PathVerb::CubicTo(at(1.0, 2.0), at(3.0, 4.0), at(5.0, 6.0)),
            ]
        );
    }

    #[test]
    fn a_smooth_cubic_mirrors_the_previous_control() {
        // The second control of the first curve is (8,0); reflected through the
        // end point (10,0) that is (12,0), which must be the next curve's first.
        assert_eq!(
            verbs("M0 0 C2 0 8 0 10 0 S18 0 20 0"),
            vec![
                PathVerb::MoveTo(at(0.0, 0.0)),
                PathVerb::CubicTo(at(2.0, 0.0), at(8.0, 0.0), at(10.0, 0.0)),
                PathVerb::CubicTo(at(12.0, 0.0), at(18.0, 0.0), at(20.0, 0.0)),
            ]
        );
    }

    #[test]
    fn a_smooth_cubic_with_nothing_to_mirror_starts_at_the_pen() {
        assert_eq!(
            verbs("M4 4 S8 8 12 4"),
            vec![
                PathVerb::MoveTo(at(4.0, 4.0)),
                PathVerb::CubicTo(at(4.0, 4.0), at(8.0, 8.0), at(12.0, 4.0)),
            ]
        );
    }

    #[test]
    fn a_line_between_two_curves_stops_the_reflection() {
        // The spec's rule, and the reason the control is cleared by every
        // command that is not a cubic: after the `L`, `S` has nothing to mirror
        // and its first control sits on the pen at (10,10).
        assert_eq!(
            verbs("M0 0 C2 0 8 0 10 0 L10 10 S18 10 20 10").last(),
            Some(&PathVerb::CubicTo(
                at(10.0, 10.0),
                at(18.0, 10.0),
                at(20.0, 10.0)
            ))
        );
    }

    #[test]
    fn a_quadratic_becomes_the_cubic_that_draws_the_same_curve() {
        // Controls two thirds of the way from each end toward (6,6): from (0,0)
        // that is (4,4), and from (12,0) it is (8,4).
        assert_eq!(
            verbs("M0 0 Q6 6 12 0"),
            vec![
                PathVerb::MoveTo(at(0.0, 0.0)),
                PathVerb::CubicTo(at(4.0, 4.0), at(8.0, 4.0), at(12.0, 0.0)),
            ]
        );
    }

    #[test]
    fn a_smooth_quadratic_mirrors_the_previous_quadratic_control() {
        // The quadratic control (6,6) reflected through (12,0) is (18,-6); the
        // cubic controls then sit two thirds of the way toward that.
        assert_eq!(
            verbs("M0 0 Q6 6 12 0 T24 0").last(),
            Some(&PathVerb::CubicTo(
                at(16.0, -4.0),
                at(20.0, -4.0),
                at(24.0, 0.0)
            ))
        );
    }

    // -------------------------------------------------------- the scanner

    #[test]
    fn numbers_may_run_together_with_no_separator() {
        // `10-20` is two numbers, because a sign can only start one.
        assert_eq!(
            verbs("M10-20L-5-5"),
            vec![
                PathVerb::MoveTo(at(10.0, -20.0)),
                PathVerb::LineTo(at(-5.0, -5.0)),
            ]
        );
    }

    #[test]
    fn a_second_decimal_point_begins_a_new_number() {
        // **The scanner's sharpest edge.** `1.5.5` is `1.5` then `.5`, and a
        // parser that split on whitespace reads it as one broken number.
        assert_eq!(verbs("M1.5.5"), vec![PathVerb::MoveTo(at(1.5, 0.5))]);
    }

    #[test]
    fn exponents_are_numbers_and_a_bare_e_is_not() {
        assert_eq!(verbs("M1e2 2e-1"), vec![PathVerb::MoveTo(at(100.0, 0.2))]);

        // **`1e` is the number 1 and a leftover `e`**, and the rewind is what
        // makes that true — without it the whole thing would fail to parse as a
        // number and the error would point at the wrong place.
        //
        // Where the `e` then surfaces depends on what the command still wanted.
        // Here `M` is one number short of its pair, so the honest report is that
        // `M` ran out rather than that `e` is a strange command: the author's
        // mistake is the missing coordinate.
        assert_eq!(
            parse_path_data("M1e 2"),
            Err(SvgPathError::TruncatedCommand('M'))
        );

        // Once the command has all its numbers, the same leftover `e` is read
        // as the command it looks like — and is refused as one.
        assert_eq!(
            parse_path_data("M1 2e"),
            Err(SvgPathError::UnknownCommand('e'))
        );
    }

    #[test]
    fn commas_and_newlines_separate_as_well_as_spaces() {
        assert_eq!(
            verbs("M0,0\n L10,\t10"),
            vec![
                PathVerb::MoveTo(at(0.0, 0.0)),
                PathVerb::LineTo(at(10.0, 10.0))
            ]
        );
    }

    // ----------------------------------------------------------- refusals

    // --------------------------------------------------------------- arcs

    /// Where the pen finished.
    fn ends_at(data: &str) -> Offset {
        match parse_path_data(data)
            .expect("valid path data")
            .verbs()
            .last()
        {
            Some(PathVerb::MoveTo(point) | PathVerb::LineTo(point)) => *point,
            Some(PathVerb::CubicTo(_, _, point)) => *point,
            other => panic!("no point to finish at: {other:?}"),
        }
    }

    /// The end point of every cubic, in order.
    fn cubic_ends(data: &str) -> Vec<Offset> {
        parse_path_data(data)
            .expect("valid path data")
            .verbs()
            .iter()
            .filter_map(|verb| match verb {
                PathVerb::CubicTo(_, _, end) => Some(*end),
                _ => None,
            })
            .collect()
    }

    fn distance(a: Offset, b: Offset) -> f32 {
        (a.dx - b.dx).hypot(a.dy - b.dy)
    }

    #[test]
    fn an_arc_finishes_exactly_where_it_was_told_to() {
        // **The assertion that holds whatever the geometry does.** However many
        // cubics an arc becomes and however the flags are set, the pen must end
        // on the written point — exactly, not nearly, because a subpath closing
        // a fraction short shows a hairline gap that reads as a rasteriser bug.
        for large in ['0', '1'] {
            for sweep in ['0', '1'] {
                let data = format!("M0 0 A5 5 0 {large} {sweep} 8 4");
                assert_eq!(
                    ends_at(&data),
                    at(8.0, 4.0),
                    "flags {large}{sweep} finished elsewhere"
                );
            }
        }
    }

    #[test]
    fn every_piece_of_a_circular_arc_ends_on_the_circle() {
        // Endpoints ten apart with radius five: the circle is pinned, centred at
        // (5, 0), so each piece's end is checkable without trusting any of the
        // arithmetic that produced it.
        let centre = at(5.0, 0.0);
        for end in cubic_ends("M0 0 A5 5 0 0 1 10 0") {
            let radius = distance(end, centre);
            assert!(
                (radius - 5.0).abs() < 0.01,
                "a piece ended {radius} from the centre rather than 5"
            );
        }
    }

    #[test]
    fn the_sweep_flag_chooses_which_side_the_arc_bulges() {
        // A semicircle splits into two quarters, so the first piece ends at the
        // very top or the very bottom — the one place the two arcs differ most.
        let clockwise = cubic_ends("M0 0 A5 5 0 0 1 10 0");
        let widdershins = cubic_ends("M0 0 A5 5 0 0 0 10 0");

        let one = clockwise.first().expect("a first piece");
        let other = widdershins.first().expect("a first piece");

        assert!(
            (one.dx - 5.0).abs() < 0.01,
            "the midpoint is above the centre"
        );
        assert!((other.dx - 5.0).abs() < 0.01);
        assert!(
            (one.dy.abs() - 5.0).abs() < 0.01 && (other.dy.abs() - 5.0).abs() < 0.01,
            "and a full radius away from it"
        );
        assert!(
            one.dy * other.dy < 0.0,
            "the two arcs must bulge to opposite sides; both went to {}",
            one.dy
        );
    }

    #[test]
    fn a_semicircle_is_two_quarters_whichever_way_the_last_ulp_rounds() {
        // The macOS failure, reproduced on every platform: Apple's `acosf(-1)`
        // is one ulp below glibc's, so `acos(-1) - TAU` sweeps a hair more
        // than a half turn.
        let below_pi = f32::from_bits(std::f32::consts::PI.to_bits() - 1);
        for swept in [
            std::f32::consts::PI,
            -std::f32::consts::PI,
            below_pi - TAU,
            TAU - below_pi,
        ] {
            assert_eq!(quarter_pieces(swept), 2, "sweep {swept}");
        }
        assert_eq!(
            quarter_pieces(FRAC_PI_2 * 1.01),
            2,
            "a real overrun still splits"
        );
        assert_eq!(quarter_pieces(TAU), 4);
    }

    #[test]
    fn the_large_arc_flag_takes_the_long_way_round() {
        // Six apart with radius five: the short way is about 74 degrees and fits
        // in one piece, the long way is about 286 and needs four.
        assert_eq!(cubic_ends("M0 0 A5 5 0 0 1 6 0").len(), 1);
        assert_eq!(cubic_ends("M0 0 A5 5 0 1 1 6 0").len(), 4);
    }

    #[test]
    fn a_zero_radius_is_a_straight_line() {
        // F.6.6.1, and the reason it is not an error: an exported drawing that
        // rounded a radius to nothing still means "go there".
        assert_eq!(
            verbs("M0 0 A0 5 0 0 1 10 0"),
            vec![
                PathVerb::MoveTo(at(0.0, 0.0)),
                PathVerb::LineTo(at(10.0, 0.0))
            ]
        );
    }

    #[test]
    fn an_arc_that_ends_where_it_began_draws_nothing() {
        // F.6.2. There is no centre to find, and a zero-length arc is what the
        // author meant by writing one.
        assert_eq!(
            verbs("M5 5 A3 3 0 1 1 5 5"),
            vec![PathVerb::MoveTo(at(5.0, 5.0))]
        );
    }

    #[test]
    fn radii_too_small_to_reach_are_scaled_until_they_do() {
        // F.6.6.2. A radius of one cannot span ten, so both radii grow to five
        // and the arc becomes the semicircle that just reaches.
        assert_eq!(ends_at("M0 0 A1 1 0 0 1 10 0"), at(10.0, 0.0));
        let centre = at(5.0, 0.0);
        for end in cubic_ends("M0 0 A1 1 0 0 1 10 0") {
            assert!(
                (distance(end, centre) - 5.0).abs() < 0.01,
                "the repaired radius should be exactly half the span"
            );
        }
    }

    #[test]
    fn arc_flags_may_run_straight_into_the_numbers_after_them() {
        // **The arc grammar's own sharp edge**, and the reason flags are read as
        // characters. `0110 0` is flag 0, flag 1, then 10 and 0 — a number
        // scanner takes `0110` whole and draws a different arc entirely.
        assert_eq!(ends_at("M0 0 a5 5 0 0110 0"), at(10.0, 0.0));
    }

    #[test]
    fn a_flag_that_is_not_zero_or_one_is_refused() {
        assert_eq!(
            parse_path_data("M0 0 A5 5 0 2 1 10 0"),
            Err(SvgPathError::MalformedArcFlag)
        );
    }

    #[test]
    fn path_data_must_begin_with_a_moveto() {
        assert_eq!(
            parse_path_data("L10 10"),
            Err(SvgPathError::MissingInitialMoveTo)
        );
    }

    #[test]
    fn numbers_with_no_command_are_refused() {
        assert_eq!(
            parse_path_data("10 10"),
            Err(SvgPathError::NumberWithoutCommand)
        );
        // And after `Z`, which takes none and so cannot be repeated.
        assert_eq!(
            parse_path_data("M0 0 Z 5 5"),
            Err(SvgPathError::NumberWithoutCommand)
        );
    }

    #[test]
    fn a_command_that_runs_out_of_numbers_names_itself() {
        assert_eq!(
            parse_path_data("M0 0 L10"),
            Err(SvgPathError::TruncatedCommand('L'))
        );
        assert_eq!(
            parse_path_data("M0 0 C1 1 2 2 3"),
            Err(SvgPathError::TruncatedCommand('C'))
        );
    }

    #[test]
    fn an_unknown_letter_names_itself_in_the_case_it_was_written() {
        assert_eq!(
            parse_path_data("M0 0 X5 5"),
            Err(SvgPathError::UnknownCommand('X'))
        );
        // **The half that proves the mechanism.** Commands are matched
        // uppercased, so reporting the matched letter would turn every `x` into
        // an `X` — and the uppercase case above would still pass. This is the
        // one that fails if the error echoes the wrong char.
        assert_eq!(
            parse_path_data("M0 0 x5 5"),
            Err(SvgPathError::UnknownCommand('x'))
        );
    }

    #[test]
    fn empty_data_is_an_empty_path_rather_than_an_error() {
        // Nothing was asked for and nothing went wrong, which is what an
        // absent `d` attribute means.
        assert!(parse_path_data("").expect("empty is valid").is_empty());
        assert!(parse_path_data("   \n ")
            .expect("blank is valid")
            .is_empty());
    }

    #[test]
    fn every_error_says_something_specific() {
        // A parser whose messages all read "invalid path" tells an author their
        // icon is broken without saying where.
        for error in [
            SvgPathError::NumberWithoutCommand,
            SvgPathError::MissingInitialMoveTo,
            SvgPathError::UnknownCommand('X'),
            SvgPathError::MalformedArcFlag,
            SvgPathError::TruncatedCommand('C'),
        ] {
            let message = error.to_string();
            assert!(message.len() > 20, "{error:?} said only {message:?}");
        }
    }
}
