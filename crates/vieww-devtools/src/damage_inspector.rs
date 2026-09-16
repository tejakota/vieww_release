//! A devtools-facing summary of a [`vieww_paint::Damage`] value.
//!
//! `Damage` already tracks exactly what a backend needs to know — which
//! regions are pending, their exact non-overlapping area, whether tracking
//! was abandoned in favor of a full repaint (see that module's own docs on
//! why over-reporting is safe and under-reporting is not). This module adds
//! nothing to that arithmetic; it packages the same numbers into the
//! shape a devtools panel wants — a region count, a coverage percentage, a
//! one-line verdict — rather than making every caller re-derive
//! `covered_area() / surface.area()` by hand.

use vieww_paint::Damage;

/// A snapshot of one [`Damage`] value, as a devtools panel would show it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageReport {
    /// How many separate regions are pending. Zero once
    /// [`Damage::is_everything`] is true — see that field's own doc.
    pub region_count: usize,
    /// The exact pending area, in square logical pixels
    /// ([`Damage::covered_area`]).
    pub covered_area: f32,
    /// The surface's total area, in square logical pixels.
    pub surface_area: f32,
    /// `covered_area / surface_area` as a percentage in `0.0..=100.0`. `0.0`
    /// for a zero-area surface rather than `NaN` — a torn-down window
    /// reporting "0% damaged" is a more honest devtools value than a
    /// division's undefined result leaking into a panel that will try to
    /// print it.
    pub coverage_percent: f32,
    /// `true` if [`Damage::is_everything`] — per-region tracking was
    /// abandoned and the whole surface will repaint.
    pub is_everything: bool,
    /// `true` if [`Damage::is_clean`] — nothing to repaint, the frame can be
    /// skipped.
    pub is_clean: bool,
}

impl DamageReport {
    /// Summarize `damage` as it stands right now.
    ///
    /// `damage` already knows its own surface (`Damage::surface`), so this
    /// takes no separate size argument — the "for a given surface size" in
    /// this crate's own task description is exactly what `Damage::surface`
    /// already is, since [`Damage::new`]/[`Damage::everything`] are
    /// constructed with one and [`Damage::resize`] is the only way to change
    /// it.
    #[must_use]
    pub fn build(damage: &Damage) -> Self {
        let surface_area = damage.surface().area();
        let covered_area = damage.covered_area();
        let coverage_percent = if surface_area > 0.0 {
            (100.0 * covered_area / surface_area).clamp(0.0, 100.0)
        } else {
            0.0
        };
        Self {
            region_count: damage.regions().len(),
            covered_area,
            surface_area,
            coverage_percent,
            is_everything: damage.is_everything(),
            is_clean: damage.is_clean(),
        }
    }

    /// A one-line human summary — what
    /// [`vieww_paint::Damage`]'s own `Display` says, plus the
    /// coverage percentage spelled out for a reader who has not memorized
    /// the arithmetic.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.is_clean {
            return "clean: nothing to repaint".to_owned();
        }
        if self.is_everything {
            return format!(
                "full repaint: {:.0}px2 (100% of {:.0}px2 surface)",
                self.covered_area, self.surface_area
            );
        }
        format!(
            "{} region(s), {:.0}px2 of {:.0}px2 surface ({:.1}%)",
            self.region_count, self.covered_area, self.surface_area, self.coverage_percent
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Rect;

    /// Total surface area, hand-computed here so the tests below read as
    /// arithmetic rather than as trusting `Rect::area` to agree with
    /// itself.
    fn area(rect: Rect) -> f32 {
        rect.width() * rect.height()
    }

    const SURFACE: Rect = Rect::new(0.0, 0.0, 200.0, 100.0); // 20,000px2

    #[test]
    fn a_fresh_damage_reports_clean_with_zero_coverage() {
        let damage = Damage::new(SURFACE);
        let report = DamageReport::build(&damage);

        assert!(report.is_clean);
        assert!(!report.is_everything);
        assert_eq!(report.region_count, 0);
        assert_eq!(report.covered_area, 0.0);
        assert_eq!(report.coverage_percent, 0.0);
        assert_eq!(report.describe(), "clean: nothing to repaint");
    }

    #[test]
    fn a_full_repaint_reports_100_percent_and_no_regions() {
        let damage = Damage::everything(SURFACE);
        let report = DamageReport::build(&damage);

        assert!(report.is_everything);
        assert_eq!(report.region_count, 0, "regions are moot once abandoned");
        assert_eq!(report.covered_area, area(SURFACE));
        assert_eq!(report.coverage_percent, 100.0);
    }

    #[test]
    fn a_single_region_reports_its_exact_hand_computed_percentage() {
        let mut damage = Damage::new(SURFACE);
        // A clean 50x50 region, inflated by AA_BLEED and rounded outward by
        // `Damage::add` — so rather than assume the caller's exact numbers
        // survive untouched, this reads back the region `Damage` actually
        // stored and computes the expected percentage from that.
        damage.add(Rect::new(10.0, 10.0, 60.0, 60.0));
        let stored_region = damage.regions()[0];
        let expected_area = area(stored_region);
        let expected_percent = 100.0 * expected_area / area(SURFACE);

        let report = DamageReport::build(&damage);
        assert_eq!(report.region_count, 1);
        assert_eq!(report.covered_area, expected_area);
        assert!(
            (report.coverage_percent - expected_percent).abs() < 1e-4,
            "{} vs {expected_percent}",
            report.coverage_percent
        );
    }

    #[test]
    fn two_non_overlapping_regions_sum_exactly() {
        let mut damage = Damage::new(SURFACE);
        damage.add(Rect::new(0.0, 0.0, 10.0, 10.0));
        damage.add(Rect::new(150.0, 50.0, 160.0, 60.0));

        let report = DamageReport::build(&damage);
        assert_eq!(report.region_count, 2);
        let expected: f32 = damage.regions().iter().map(|r| area(*r)).sum();
        assert_eq!(report.covered_area, expected);
    }

    #[test]
    fn coverage_percent_is_clamped_and_finite_for_a_degenerate_surface() {
        let damage = Damage::new(Rect::ZERO);
        let report = DamageReport::build(&damage);
        assert_eq!(report.surface_area, 0.0);
        assert_eq!(report.coverage_percent, 0.0, "must not be NaN");
    }

    #[test]
    fn describe_mentions_region_count_and_percentage() {
        let mut damage = Damage::new(SURFACE);
        damage.add(Rect::new(0.0, 0.0, 100.0, 50.0)); // 5,000 / 20,000 = 25%
        let report = DamageReport::build(&damage);
        let text = report.describe();
        assert!(text.contains("1 region"), "{text}");
        assert!(text.contains('%'), "{text}");
    }
}
