//! Pending-rectangle tracking.
//!
//! Most frames change almost nothing: a caret blinks, a label counts up, one
//! button lightens under a finger. Redrawing the whole surface for that wastes
//! the entire GPU budget on pixels that are already correct. [`Damage`]
//! accumulates the regions that actually changed so a frame can repaint a corner
//! instead of a screen.
//!
//! # Why over-reporting is safe and under-reporting is not
//!
//! Every approximation here — antialiasing bleed, curve bounds taken from
//! control points, text measured from its style rather than shaped — rounds
//! *outward*. A region that is too large costs some fill rate. A region that is
//! too small leaves stale pixels on the screen, which is a visible corruption
//! bug that reproduces only on certain frames. The asymmetry is total, so every
//! judgement call in this file leans large.

use std::fmt;

use vieww_foundation::{Rect, Transform};

use crate::Scene;

/// How far outside its geometric bounds a shape can tint pixels.
///
/// An antialiased edge blends into the pixels it partially covers, so the
/// affected area is the shape's bounds grown by up to one pixel. Curves under a
/// transform can land slightly further out again; one pixel plus the outward
/// rounding in [`Rect::round_out`] covers both.
pub const AA_BLEED: f32 = 1.0;

/// How many separate regions are worth tracking before merging them.
///
/// Each region costs a scissor change and a draw call on the backend, and a
/// containment test against every command. Past roughly this many, the
/// bookkeeping costs more than the pixels it saves.
pub const MAX_REGIONS: usize = 16;

/// The fraction of the surface at which tracking is abandoned.
///
/// Once this much of the surface is pending, the regions have stopped being a
/// useful description of the frame — the remaining clean pixels are scattered
/// slivers. Collapsing to a full repaint drops all the per-region overhead for a
/// small amount of redundant fill.
pub const REPAINT_ALL_THRESHOLD: f32 = 0.65;

/// The regions of a surface that need repainting this frame.
///
/// # Invariants
///
/// Regions are pairwise non-overlapping, pixel-aligned, and clipped to the
/// surface. Overlaps are merged on insert, which keeps
/// [`covered_area`](Self::covered_area) an exact sum rather than an
/// over-estimate, and stops a hot region from being repainted several times in
/// one frame.
#[derive(Debug, Clone)]
pub struct Damage {
    surface: Rect,
    regions: Vec<Rect>,
    everything: bool,
}

impl Damage {
    /// A clean surface — nothing to repaint.
    #[must_use]
    pub fn new(surface: Rect) -> Self {
        Self {
            surface: surface.round_out(),
            regions: Vec::new(),
            everything: false,
        }
    }

    /// A surface that must be repainted in full.
    ///
    /// This is the correct starting state for the first frame, and after any
    /// event that invalidates the previous contents — a resize, a device loss, a
    /// backend swap.
    #[must_use]
    pub fn everything(surface: Rect) -> Self {
        Self {
            surface: surface.round_out(),
            regions: Vec::new(),
            everything: true,
        }
    }

    /// The damage between two recorded frames.
    ///
    /// Commands are matched from both ends, and everything between the first and
    /// last disagreement is damaged **in both scenes** — the old commands
    /// because their pixels have to be erased, the new ones because theirs have
    /// to be drawn.
    ///
    /// Matching only at the ends means an insertion near the front reports
    /// everything after it as pending, even where the drawing is unchanged. A real
    /// diff would do better, but it would also have to be right about *which*
    /// commands moved, and being wrong there under-reports. The end-matching is
    /// exact for the case that dominates — a subtree repainting in place.
    #[must_use]
    pub fn between(old: &Scene, new: &Scene, surface: Rect) -> Self {
        let mut damage = Self::new(surface);
        damage.add_between(old, new, Transform::IDENTITY);
        damage
    }

    /// Add the damage between two recorded scenes, mapped through `transform`.
    ///
    /// The transform is what a [`Layer`](crate::Layer) needs: it records in its
    /// own coordinate space, so the diff comes out in that space and has to be
    /// mapped to the screen before it means anything to a backend.
    ///
    /// See [`between`](Self::between) for how the diff itself works.
    pub fn add_between(&mut self, old: &Scene, new: &Scene, transform: Transform) {
        let (a, b) = (old.commands(), new.commands());

        let mut prefix = 0;
        while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
            prefix += 1;
        }
        // The suffix must not reach back past the prefix, or the two would
        // overlap and the divergent range would be empty when it is not.
        let available = a.len().min(b.len()) - prefix;
        let mut suffix = 0;
        while suffix < available && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
            suffix += 1;
        }

        for command in a[prefix..a.len() - suffix]
            .iter()
            .chain(&b[prefix..b.len() - suffix])
        {
            let bounds = command.bounds();
            if bounds.is_empty() {
                continue;
            }
            self.add(transform.apply_rect(bounds));
        }
    }

    /// The surface being tracked, pixel-aligned.
    #[must_use]
    pub fn surface(&self) -> Rect {
        self.surface
    }

    /// Mark a region pending.
    ///
    /// The rectangle is grown by [`AA_BLEED`], aligned outward to whole pixels,
    /// and clipped to the surface. Regions it overlaps are merged into it.
    ///
    /// An empty rectangle damages nothing. That check has to come *before* the
    /// bleed is applied: `Rect::ZERO` is the "nothing here" sentinel returned by
    /// an empty layer and by a command clipped fully out of view, and inflating it
    /// would turn it into a live 1x1 region at the origin — a pixel that repaints
    /// every frame forever and keeps [`is_clean`](Self::is_clean) false when there
    /// is genuinely nothing to do.
    pub fn add(&mut self, rect: Rect) {
        if self.everything || rect.is_empty() {
            return;
        }
        let mut rect = rect
            .inflate(AA_BLEED)
            .round_out()
            .intersect(self.surface)
            .round_out();
        if rect.is_empty() {
            return;
        }

        // Absorb every region this one touches. A union can reach a region the
        // original rectangle missed, so this repeats until nothing more merges.
        while let Some(index) = self.regions.iter().position(|region| region.overlaps(rect)) {
            rect = rect.union(self.regions.swap_remove(index));
        }
        self.regions.push(rect);

        if self.regions.len() > MAX_REGIONS {
            self.merge_cheapest_pair();
        }
        if self.covered_area() >= self.surface.area() * REPAINT_ALL_THRESHOLD {
            self.add_everything();
        }
    }

    /// Mark every command in a scene pending.
    ///
    /// For the first frame, or whenever there is no previous scene to diff
    /// against.
    pub fn add_scene(&mut self, scene: &Scene) {
        for command in scene.commands() {
            self.add(command.bounds());
        }
    }

    /// Abandon per-region tracking and repaint the whole surface.
    pub fn add_everything(&mut self) {
        self.everything = true;
        self.regions.clear();
    }

    /// The regions to repaint, or the whole surface if tracking was abandoned.
    ///
    /// Prefer this over [`regions`](Self::regions) when driving a backend: it
    /// collapses the full-repaint case into the same shape as the incremental
    /// one, so a caller needs no branch.
    #[must_use]
    pub fn repaint_regions(&self) -> Vec<Rect> {
        if self.everything {
            if self.surface.is_empty() {
                Vec::new()
            } else {
                vec![self.surface]
            }
        } else {
            self.regions.clone()
        }
    }

    /// The tracked regions. Empty when a full repaint is pending — check
    /// [`is_everything`](Self::is_everything) first, or use
    /// [`repaint_regions`](Self::repaint_regions).
    #[must_use]
    pub fn regions(&self) -> &[Rect] {
        &self.regions
    }

    /// The single rectangle enclosing all damage.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        if self.everything {
            return self.surface;
        }
        self.regions.iter().copied().fold(Rect::ZERO, Rect::union)
    }

    /// The exact pending area, in square pixels.
    ///
    /// Exact rather than an upper bound, because regions never overlap.
    #[must_use]
    pub fn covered_area(&self) -> f32 {
        if self.everything {
            return self.surface.area();
        }
        self.regions.iter().map(|region| region.area()).sum()
    }

    /// `true` if nothing needs repainting and the frame can be skipped entirely.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        !self.everything && self.regions.is_empty()
    }

    /// `true` if the whole surface is being repainted.
    #[must_use]
    pub fn is_everything(&self) -> bool {
        self.everything
    }

    /// `true` if `rect` touches any damaged region.
    ///
    /// A backend uses this to skip commands whose pixels are already correct.
    #[must_use]
    pub fn intersects(&self, rect: Rect) -> bool {
        if self.everything {
            return !self.surface.intersect(rect).is_empty();
        }
        self.regions.iter().any(|region| region.overlaps(rect))
    }

    /// Reset to clean, keeping the surface. Call once a frame is presented.
    pub fn clear(&mut self) {
        self.regions.clear();
        self.everything = false;
    }

    /// Change the tracked surface, invalidating everything.
    ///
    /// A resize discards the old contents, so there is nothing to keep.
    pub fn resize(&mut self, surface: Rect) {
        self.surface = surface.round_out();
        self.add_everything();
    }

    /// Union the two regions whose merger wastes the least new area.
    ///
    /// Merging by waste rather than collapsing everything to
    /// [`bounds`](Self::bounds) matters when the pending regions are far apart: two
    /// opposite corners of the screen would collapse into the entire screen,
    /// while this leaves them separate and merges some closer pair instead.
    fn merge_cheapest_pair(&mut self) {
        let mut best: Option<(usize, usize, f32)> = None;
        for i in 0..self.regions.len() {
            for j in (i + 1)..self.regions.len() {
                let (a, b) = (self.regions[i], self.regions[j]);
                let waste = a.union(b).area() - a.area() - b.area();
                if best.is_none_or(|(_, _, best_waste)| waste < best_waste) {
                    best = Some((i, j, waste));
                }
            }
        }
        if let Some((i, j, _)) = best {
            // Remove the later index first so the earlier one stays valid.
            let b = self.regions.swap_remove(j);
            let a = self.regions.swap_remove(i);
            self.regions.push(a.union(b));
        }
    }
}

impl fmt::Display for Damage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.everything {
            return write!(f, "everything {}", self.surface);
        }
        if self.regions.is_empty() {
            return f.write_str("clean");
        }
        let percent = 100.0 * self.covered_area() / self.surface.area().max(1.0);
        write!(
            f,
            "{} region(s), {percent:.1}% of surface",
            self.regions.len()
        )?;
        for region in &self.regions {
            write!(f, "\n  {region}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Color, Offset};

    use super::*;
    use crate::Canvas;

    const SURFACE: Rect = Rect::new(0.0, 0.0, 1000.0, 1000.0);

    fn damage() -> Damage {
        Damage::new(SURFACE)
    }

    #[test]
    fn a_fresh_damage_is_clean_and_a_clean_frame_can_be_skipped() {
        let damage = damage();
        assert!(damage.is_clean());
        assert!(damage.repaint_regions().is_empty());
        assert_eq!(damage.covered_area(), 0.0);
    }

    #[test]
    fn an_added_region_is_inflated_outward_never_inward() {
        let mut damage = damage();
        damage.add(Rect::new(10.5, 10.5, 20.5, 20.5));

        let region = damage.regions()[0];
        assert!(
            region.left <= 9.0 && region.top <= 9.0,
            "stale pixels are worse than extra fill: {region}"
        );
        assert!(region.right >= 22.0 && region.bottom >= 22.0, "{region}");
    }

    #[test]
    fn regions_are_pixel_aligned() {
        let mut damage = damage();
        damage.add(Rect::new(10.3, 10.7, 20.2, 20.9));

        let region = damage.regions()[0];
        assert_eq!(region.left.fract(), 0.0, "{region}");
        assert_eq!(region.top.fract(), 0.0, "{region}");
        assert_eq!(region.right.fract(), 0.0, "{region}");
        assert_eq!(region.bottom.fract(), 0.0, "{region}");
    }

    #[test]
    fn overlapping_regions_merge_into_one() {
        let mut damage = damage();
        damage.add(Rect::new(0.0, 0.0, 100.0, 100.0));
        damage.add(Rect::new(50.0, 50.0, 150.0, 150.0));

        assert_eq!(damage.regions().len(), 1);
        assert_eq!(
            damage.covered_area(),
            damage.regions()[0].area(),
            "the shared 50x50 corner is counted once, not twice"
        );
    }

    #[test]
    fn regions_never_overlap_so_covered_area_is_exact() {
        let mut damage = damage();
        for i in 0..10 {
            let x = i as f32 * 90.0;
            damage.add(Rect::new(x, 0.0, x + 20.0, 20.0));
            damage.add(Rect::new(x + 5.0, 5.0, x + 30.0, 30.0));
        }

        let regions = damage.regions();
        for (i, a) in regions.iter().enumerate() {
            for b in &regions[(i + 1)..] {
                assert!(!a.overlaps(*b), "{a} overlaps {b}");
            }
        }
        let summed: f32 = regions.iter().map(|r| r.area()).sum();
        assert_eq!(damage.covered_area(), summed);
    }

    #[test]
    fn a_merge_can_cascade_into_a_region_the_new_rect_did_not_touch() {
        let mut damage = damage();
        damage.add(Rect::new(0.0, 0.0, 40.0, 40.0));
        damage.add(Rect::new(200.0, 0.0, 240.0, 40.0));
        assert_eq!(damage.regions().len(), 2);

        // Bridges the two, which must collapse to one region, not three.
        damage.add(Rect::new(30.0, 10.0, 210.0, 20.0));
        assert_eq!(damage.regions().len(), 1);
    }

    #[test]
    fn distant_regions_stay_separate() {
        let mut damage = damage();
        damage.add(Rect::new(0.0, 0.0, 10.0, 10.0));
        damage.add(Rect::new(900.0, 900.0, 910.0, 910.0));

        assert_eq!(damage.regions().len(), 2);
        assert!(
            damage.covered_area() < 1000.0,
            "two small corners must not cost the screen: {damage}"
        );
    }

    #[test]
    fn damage_is_clipped_to_the_surface() {
        let mut damage = damage();
        damage.add(Rect::new(-500.0, -500.0, 50.0, 50.0));

        let region = damage.regions()[0];
        assert!(region.left >= SURFACE.left, "{region}");
        assert!(region.top >= SURFACE.top, "{region}");
    }

    #[test]
    fn damage_entirely_off_surface_is_dropped() {
        let mut damage = damage();
        damage.add(Rect::new(5000.0, 5000.0, 6000.0, 6000.0));
        assert!(damage.is_clean());
    }

    #[test]
    fn an_empty_rect_damages_nothing_even_though_the_bleed_would_inflate_it() {
        let mut damage = damage();
        // `Rect::ZERO` is what an empty layer and a fully clipped command report.
        // Inflating before testing for emptiness would leave a 1x1 region at the
        // origin that repaints every frame forever.
        damage.add(Rect::ZERO);
        damage.add(Rect::new(50.0, 50.0, 50.0, 50.0));
        damage.add(Rect::new(80.0, 80.0, 70.0, 90.0)); // inverted, so no area

        assert!(damage.is_clean(), "{damage}");
    }

    #[test]
    fn tracking_is_abandoned_once_most_of_the_surface_is_pending() {
        let mut damage = damage();
        damage.add(Rect::new(0.0, 0.0, 1000.0, 700.0)); // 70% of the surface

        assert!(damage.is_everything(), "{damage}");
        assert_eq!(damage.repaint_regions(), vec![damage.surface()]);
    }

    #[test]
    fn region_count_is_bounded_however_many_rects_arrive() {
        let mut damage = damage();
        // Small and far apart, so nothing merges on overlap and the total area
        // stays under the abandon threshold.
        for i in 0..40 {
            let x = (i % 8) as f32 * 120.0;
            let y = (i / 8) as f32 * 120.0;
            damage.add(Rect::new(x, y, x + 4.0, y + 4.0));
        }
        assert!(!damage.is_everything(), "{damage}");
        assert!(damage.regions().len() <= MAX_REGIONS, "{damage}");
    }

    #[test]
    fn adding_to_a_full_repaint_stays_a_full_repaint() {
        let mut damage = Damage::everything(SURFACE);
        damage.add(Rect::new(0.0, 0.0, 1.0, 1.0));

        assert!(damage.is_everything());
        assert!(
            damage.regions().is_empty(),
            "regions are moot once abandoned"
        );
        assert_eq!(damage.bounds(), SURFACE);
    }

    #[test]
    fn intersects_lets_a_backend_skip_untouched_commands() {
        let mut damage = damage();
        damage.add(Rect::new(100.0, 100.0, 200.0, 200.0));

        assert!(damage.intersects(Rect::new(150.0, 150.0, 160.0, 160.0)));
        assert!(!damage.intersects(Rect::new(500.0, 500.0, 600.0, 600.0)));
    }

    #[test]
    fn a_full_repaint_intersects_anything_on_the_surface() {
        let damage = Damage::everything(SURFACE);
        assert!(damage.intersects(Rect::new(1.0, 1.0, 2.0, 2.0)));
        assert!(!damage.intersects(Rect::new(-50.0, -50.0, -10.0, -10.0)));
    }

    #[test]
    fn clearing_returns_to_clean_and_keeps_the_surface() {
        let mut damage = Damage::everything(SURFACE);
        damage.clear();
        assert!(damage.is_clean());
        assert_eq!(damage.surface(), SURFACE);
    }

    #[test]
    fn a_resize_invalidates_everything() {
        let mut damage = damage();
        damage.resize(Rect::new(0.0, 0.0, 500.0, 500.0));

        assert!(damage.is_everything(), "old contents cannot be reused");
        assert_eq!(damage.surface(), Rect::new(0.0, 0.0, 500.0, 500.0));
    }

    fn scene_with(rects: &[(Rect, Color)]) -> Scene {
        let mut scene = Scene::new();
        for (rect, color) in rects {
            scene.fill_rect(*rect, (*color).into());
        }
        scene
    }

    #[test]
    fn identical_scenes_produce_no_damage() {
        let rects = [
            (Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED),
            (Rect::new(50.0, 50.0, 60.0, 60.0), Color::BLUE),
        ];
        let damage = Damage::between(&scene_with(&rects), &scene_with(&rects), SURFACE);
        assert!(damage.is_clean(), "{damage}");
    }

    #[test]
    fn a_recoloured_command_damages_only_its_own_bounds() {
        let old = scene_with(&[
            (Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED),
            (Rect::new(500.0, 500.0, 510.0, 510.0), Color::BLUE),
        ]);
        let new = scene_with(&[
            (Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED),
            (Rect::new(500.0, 500.0, 510.0, 510.0), Color::GREEN),
        ]);

        let damage = Damage::between(&old, &new, SURFACE);
        assert_eq!(damage.regions().len(), 1, "{damage}");
        assert!(damage.intersects(Rect::new(500.0, 500.0, 510.0, 510.0)));
        assert!(
            !damage.intersects(Rect::new(0.0, 0.0, 5.0, 5.0)),
            "the unchanged command must not be repainted: {damage}"
        );
    }

    #[test]
    fn a_moved_command_damages_both_where_it_was_and_where_it_is() {
        let old = scene_with(&[(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED)]);
        let new = scene_with(&[(Rect::new(400.0, 400.0, 410.0, 410.0), Color::RED)]);

        let damage = Damage::between(&old, &new, SURFACE);
        assert!(
            damage.intersects(Rect::new(0.0, 0.0, 10.0, 10.0)),
            "the vacated pixels still hold the old colour: {damage}"
        );
        assert!(damage.intersects(Rect::new(400.0, 400.0, 410.0, 410.0)));
    }

    #[test]
    fn a_removed_command_damages_the_pixels_it_leaves_behind() {
        let old = scene_with(&[
            (Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED),
            (Rect::new(300.0, 300.0, 310.0, 310.0), Color::BLUE),
        ]);
        let new = scene_with(&[(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED)]);

        let damage = Damage::between(&old, &new, SURFACE);
        assert!(
            damage.intersects(Rect::new(300.0, 300.0, 310.0, 310.0)),
            "{damage}"
        );
    }

    #[test]
    fn an_appended_command_leaves_the_matching_prefix_clean() {
        let old = scene_with(&[(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED)]);
        let new = scene_with(&[
            (Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED),
            (Rect::new(300.0, 300.0, 310.0, 310.0), Color::BLUE),
        ]);

        let damage = Damage::between(&old, &new, SURFACE);
        assert!(
            !damage.intersects(Rect::new(0.0, 0.0, 5.0, 5.0)),
            "{damage}"
        );
        assert!(damage.intersects(Rect::new(300.0, 300.0, 310.0, 310.0)));
    }

    #[test]
    fn a_trailing_match_is_recognised_when_a_command_is_prepended() {
        let tail = (Rect::new(800.0, 800.0, 810.0, 810.0), Color::BLUE);
        let old = scene_with(&[tail]);
        let new = scene_with(&[(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED), tail]);

        let damage = Damage::between(&old, &new, SURFACE);
        assert!(
            !damage.intersects(Rect::new(805.0, 805.0, 806.0, 806.0)),
            "matching from both ends should spare the shared tail: {damage}"
        );
        assert!(damage.intersects(Rect::new(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn diffing_against_an_empty_scene_damages_everything_drawn() {
        let new = scene_with(&[(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED)]);
        let damage = Damage::between(&Scene::new(), &new, SURFACE);
        assert!(damage.intersects(Rect::new(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn a_transform_change_damages_the_screen_space_bounds_not_the_local_ones() {
        let mut old = Scene::new();
        old.translate(Offset::new(100.0, 100.0));
        old.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());

        let mut new = Scene::new();
        new.translate(Offset::new(600.0, 600.0));
        new.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());

        let damage = Damage::between(&old, &new, SURFACE);
        assert!(
            damage.intersects(Rect::new(100.0, 100.0, 110.0, 110.0)),
            "{damage}"
        );
        assert!(
            damage.intersects(Rect::new(600.0, 600.0, 610.0, 610.0)),
            "{damage}"
        );
        assert!(
            !damage.intersects(Rect::new(0.0, 0.0, 5.0, 5.0)),
            "local coordinates are not where the pixels are: {damage}"
        );
    }

    #[test]
    fn add_scene_damages_every_command() {
        let mut damage = damage();
        damage.add_scene(&scene_with(&[
            (Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED),
            (Rect::new(700.0, 700.0, 710.0, 710.0), Color::BLUE),
        ]));

        assert_eq!(damage.regions().len(), 2, "{damage}");
    }
}
