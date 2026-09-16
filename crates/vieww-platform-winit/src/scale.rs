//! Logical pixels above, physical pixels below.
//!
//! Every coordinate in `vieww` is a **logical** pixel: `Offset`, `Size`, `Rect`,
//! a font's em size, `kTouchSlop`. Every coordinate a window deals in is a
//! **physical** one. The ratio between them is a property of the display, and it
//! is 3 on a modern phone — so getting this wrong is not a rounding error, it is
//! a UI drawn at a third of its size in the corner of the screen.
//!
//! This is the only place the two meet, and that is deliberate: the conversion
//! is a platform concern, so no core crate has a notion of it, and there is one
//! function per direction rather than a `* scale` scattered through the event
//! handlers.

use vieww_foundation::{Offset, Rect, Size, Transform};
use vieww_paint::{Damage, Scene};

/// How many physical pixels make one logical pixel.
///
/// Never zero and never negative: a display with no pixels does not exist, and a
/// window reporting one is a compositor bug that would otherwise turn into a
/// divide by zero several layers away from the cause.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale(f32);

impl Scale {
    /// The scale of a display where one logical pixel is one physical pixel.
    pub const ONE: Self = Self(1.0);

    /// A scale, or [`ONE`](Self::ONE) if the platform reported something absurd.
    ///
    /// Clamping rather than panicking is the right answer here: the value comes
    /// from outside, an application cannot do anything about it, and rendering
    /// at 1:1 is wrong in a way the user can see and recover from, where a crash
    /// on window creation is not.
    #[must_use]
    pub fn new(factor: f64) -> Self {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "f32 is the geometry scalar; §4"
        )]
        let factor = factor as f32;
        if factor.is_finite() && factor > 0.0 {
            Self(factor)
        } else {
            Self::ONE
        }
    }

    /// The raw ratio.
    #[must_use]
    pub const fn factor(self) -> f32 {
        self.0
    }

    /// `true` when logical and physical pixels are the same thing, and every
    /// conversion below can be skipped outright.
    #[must_use]
    pub fn is_one(self) -> bool {
        (self.0 - 1.0).abs() < f32::EPSILON
    }

    /// A physical position as the logical one the tree hit tests against.
    #[must_use]
    pub fn to_logical(self, physical: Offset) -> Offset {
        Offset::new(physical.dx / self.0, physical.dy / self.0)
    }

    /// A physical window size as the logical size to lay the root out in.
    #[must_use]
    pub fn to_logical_size(self, width: u32, height: u32) -> Size {
        Size::new(width as f32 / self.0, height as f32 / self.0)
    }

    /// The transform that takes a recorded scene into physical pixels.
    #[must_use]
    pub fn to_physical_transform(self) -> Transform {
        Transform::scale(self.0, self.0)
    }

    /// A logical rectangle in physical pixels.
    #[must_use]
    pub fn to_physical_rect(self, rect: Rect) -> Rect {
        self.to_physical_transform().apply_rect(rect)
    }
}

/// A frame's scene and damage, converted to the pixels the surface is measured
/// in.
///
/// At 1:1 — every desktop display without HiDPI, and the case tests run in —
/// this borrows both and copies nothing. Anywhere else it pays for one scene
/// copy per frame, which is the honest cost of not having a root transform in
/// the backend: `GpuRenderer::render_damaged` already appends the scene once per
/// damaged region, so the scale could ride along on that append instead and this
/// copy could go. Doing that means teaching the paint layer about a coordinate
/// space it currently has no reason to know about, so it waits until a device
/// with a real DPI says it is worth it.
#[derive(Debug)]
pub struct Physical<'a> {
    /// Always borrowed. The scene is never rewritten — see
    /// [`root`](Self::root).
    scene: &'a Scene,
    damage: DamageCow<'a>,
    root: Transform,
}

#[derive(Debug)]
enum DamageCow<'a> {
    Borrowed(&'a Damage),
    Scaled(Damage),
}

impl<'a> Physical<'a> {
    /// Convert a frame, or pass it through untouched at 1:1.
    #[must_use]
    pub fn new(scene: &'a Scene, damage: &'a Damage, scale: Scale, surface: Rect) -> Self {
        if scale.is_one() {
            return Self {
                scene,
                damage: DamageCow::Borrowed(damage),
                root: Transform::IDENTITY,
            };
        }

        // Built from the regions rather than scaled in place, because a
        // `Damage` is defined against a surface and this one belongs to a
        // different, larger surface than the frame's did.
        let mut regions = Damage::new(surface);
        if damage.is_everything() {
            regions.add_everything();
        } else {
            for region in damage.regions() {
                regions.add(scale.to_physical_rect(*region));
            }
        }

        Self {
            scene,
            damage: DamageCow::Scaled(regions),
            root: scale.to_physical_transform(),
        }
    }

    /// The scene, unchanged, in *logical* pixels.
    ///
    /// Paired with [`root`](Self::root), which is what puts it in physical ones.
    /// Rewriting the scene here is what this used to do, and it copied every
    /// command in the frame on every display that was not exactly 1:1.
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        self.scene
    }

    /// The transform that turns the scene into physical pixels.
    ///
    /// Handed to `GpuRenderer::present_with`,
    /// which composes it with the per-region translation it was already
    /// applying — so the scale costs one more matrix multiply per region rather
    /// than one copy of the frame.
    #[must_use]
    pub const fn root(&self) -> Transform {
        self.root
    }

    /// The damage in physical pixels.
    #[must_use]
    pub const fn damage(&self) -> &Damage {
        match &self.damage {
            DamageCow::Borrowed(damage) => damage,
            DamageCow::Scaled(damage) => damage,
        }
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::Color;
    use vieww_paint::Canvas;

    use super::*;

    #[test]
    fn an_absurd_scale_falls_back_to_one_rather_than_panicking() {
        for absurd in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Scale::new(absurd),
                Scale::ONE,
                "{absurd} should not survive"
            );
        }
    }

    #[test]
    fn a_physical_position_becomes_the_logical_one_the_tree_uses() {
        let scale = Scale::new(3.0);
        assert_eq!(
            scale.to_logical(Offset::new(300.0, 150.0)),
            Offset::new(100.0, 50.0)
        );
        assert_eq!(scale.to_logical_size(1080, 2400), Size::new(360.0, 800.0));
    }

    #[test]
    fn a_one_to_one_frame_passes_through_untouched() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
        let damage = Damage::everything(Rect::new(0.0, 0.0, 100.0, 100.0));

        let physical = Physical::new(&scene, &damage, Scale::ONE, damage.surface());

        assert!(physical.root().is_identity());
        assert_eq!(
            physical.scene().fills()[0].0,
            Rect::new(0.0, 0.0, 10.0, 10.0)
        );
    }

    #[test]
    fn a_scaled_frame_moves_both_the_ink_and_the_damage() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(10.0, 20.0, 30.0, 40.0), Color::RED.into());

        let logical = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut damage = Damage::new(logical);
        damage.add(Rect::new(10.0, 20.0, 30.0, 40.0));

        let surface = Rect::new(0.0, 0.0, 200.0, 200.0);
        let physical = Physical::new(&scene, &damage, Scale::new(2.0), surface);

        // The scene itself is *not* rewritten — that copied every command in
        // the frame on every display that was not exactly 1:1. The scale is a
        // matrix now, composed into the per-region transform the rasteriser was
        // already applying.
        assert_eq!(
            physical.scene().fills()[0].0,
            Rect::new(10.0, 20.0, 30.0, 40.0),
            "the frame is handed over as it was described"
        );
        assert_eq!(
            physical
                .root()
                .apply_rect(Rect::new(10.0, 20.0, 30.0, 40.0)),
            Rect::new(20.0, 40.0, 60.0, 80.0),
            "and the transform is what puts the ink where the bigger surface \
             wants it"
        );
        // The rectangle is 20,40..60,80 once scaled, and both `Damage::add`s —
        // the caller's and this one's — grow it by `AA_BLEED`. So a one-pixel
        // logical bleed arrives as three physical pixels rather than two.
        // Conservative in the safe direction: the alternative is building a
        // `Damage` without going through `add`, which means re-implementing the
        // merging it does.
        assert_eq!(
            physical.damage().regions(),
            [Rect::new(17.0, 37.0, 63.0, 83.0)],
            "damage that did not scale would repaint the wrong quarter of the screen"
        );
        let scaled = Rect::new(20.0, 40.0, 60.0, 80.0);
        assert_eq!(
            physical.damage().bounds().intersect(scaled),
            scaled,
            "whatever the bleed, the repainted region must cover all of the scaled one"
        );
        assert_eq!(physical.damage().surface(), surface);
    }

    #[test]
    fn scaling_everything_stays_everything() {
        let logical = Rect::new(0.0, 0.0, 100.0, 100.0);
        let scene = Scene::new();
        let damage = Damage::everything(logical);
        let surface = Rect::new(0.0, 0.0, 300.0, 300.0);

        let physical = Physical::new(&scene, &damage, Scale::new(3.0), surface);

        assert!(
            physical.damage().is_everything(),
            "a full repaint of the logical surface is a full repaint of the physical one"
        );
        assert_eq!(physical.damage().bounds(), surface);
    }

    #[test]
    fn a_scaled_frame_is_never_copied_however_big_it_is() {
        // The regression this exists to catch: a `Physical` that rewrote the
        // scene would be indistinguishable in every other assertion here, and
        // would cost a copy of every command on every frame of every non-1:1
        // display — which is most of them.
        let mut scene = Scene::new();
        for i in 0..64 {
            let x = i as f32;
            scene.fill_rect(Rect::new(x, x, x + 1.0, x + 1.0), Color::RED.into());
        }
        let logical = Rect::new(0.0, 0.0, 100.0, 100.0);
        let damage = Damage::everything(logical);

        let physical = Physical::new(&scene, &damage, Scale::new(2.0), logical);

        assert!(std::ptr::eq(physical.scene(), &scene));
    }
}
