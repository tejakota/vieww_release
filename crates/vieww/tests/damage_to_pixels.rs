//! The exit test for damage and layers: a change costs a corner of the screen,
//! all the way down to the pixels actually rasterised.
//!
//! Requires the `native` feature:
//!
//! ```console
//! cargo test -p vieww --features native --test damage_to_pixels
//! ```
//!
//! # What this is for
//!
//! Phase 4 built `Damage` and `LayerTree` and tested both thoroughly, but nothing
//! *used* them: the frame driver painted into one flat scene and the backend
//! rendered all of it every time. Two damage bugs sat behind green tests for
//! exactly that reason — the tests asserted the right property about a value
//! nobody consumed.
//!
//! So this test does not check that damage is *computed*. It checks that it is
//! **obeyed**: that a small change to a widget tree ends with a present
//! rasterising a small number of pixels, and that the pixels outside them are
//! the ones the previous frame left there — via
//! [`vieww_test_harness::Persistent`], the same damage-composited-onto-the-
//! previous-frame model a real presented surface uses.
//!
//! Vieww's own rasterizer needs no graphics adapter, so this runs
//! unconditionally.

#![cfg(feature = "native")]

use vieww::foundation::{Color, Rect, Size};
use vieww::paint::Damage;
use vieww::prelude::*;
use vieww::{BuildContext, Widget, WidgetKind, WidgetNode};
use vieww_test_harness::Persistent;

/// A damage-driven surface backed by [`Persistent`]: the first present is a
/// full render (nothing to keep yet), every later one composites only the
/// damaged regions onto what is already there — the same cost a real
/// presented surface pays, and the same reason `Persistent::advance` exists.
struct Surface {
    persistent: Option<Persistent>,
}

impl Surface {
    const fn new() -> Self {
        Self { persistent: None }
    }

    /// Drive one frame and push it onto the surface, honouring the damage.
    /// Returns how many pixels this present rasterised.
    fn present(&mut self, driver: &mut FrameDriver) -> u64 {
        driver.draw_frame();
        match &mut self.persistent {
            None => {
                self.persistent = Some(Persistent::new(driver, Size::square(SIDE_F), Color::WHITE));
                u64::from(SIDE) * u64::from(SIDE)
            }
            Some(persistent) => {
                let cost = damage_pixel_cost(driver.damage(), SIDE);
                persistent.advance(driver);
                cost
            }
        }
    }

    fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        self.persistent
            .as_ref()
            .expect("at least one frame was presented")
            .frame()
            .at(x, y)
    }
}

/// The pixel cost of the current damage, mirroring exactly what
/// `Persistent::advance` (and, before it, a real presented surface) actually
/// rasterises: each repaint region rounded outward and clamped to the
/// surface, not one looser union of them all.
fn damage_pixel_cost(damage: &Damage, side: u32) -> u64 {
    if damage.is_clean() {
        return 0;
    }
    if damage.is_everything() {
        return u64::from(side) * u64::from(side);
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded outward and clamped to the surface, same as Persistent::blit"
    )]
    damage
        .repaint_regions()
        .iter()
        .map(|region| {
            let region = region.round_out();
            let x0 = region.left.max(0.0) as u32;
            let y0 = region.top.max(0.0) as u32;
            let x1 = (region.right.max(0.0) as u32).min(side);
            let y1 = (region.bottom.max(0.0) as u32).min(side);
            u64::from(x1.saturating_sub(x0)) * u64::from(y1.saturating_sub(y0))
        })
        .sum()
}

/// The surface, in pixels for the rasteriser and in logical units for layout.
/// Kept as two constants rather than one cast: the pixel count is exact
/// arithmetic the assertions depend on, and rounding it out of an `f32` at
/// each use site reads worse than stating it twice.
const SIDE: u32 = 200;
const SIDE_F: f32 = 200.0;
const SWATCH: f32 = 20.0;
/// Where the swatch lands: a column fills its cross axis and centres in it, and
/// the static band above it is 120 tall.
const SWATCH_RECT: Rect = Rect::new(90.0, 120.0, 110.0, 140.0);

/// A composed widget whose colour comes from a signal.
///
/// Reading the signal in `build` is what subscribes this element to it, so
/// setting the signal rebuilds *this element only*. That is what makes the test
/// meaningful: a rebuild from the root would legitimately repaint everything, and
/// then a small damage figure would prove nothing.
#[derive(Debug)]
struct Swatch {
    color: Signal<Color>,
}

impl Widget for Swatch {
    fn debug_name(&self) -> &'static str {
        "Swatch"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        ColoredBox::new(self.color.get())
            .child(SizedBox::square(SWATCH))
            .into()
    }
}

vieww::widget::widget_node_from!(Swatch);

/// A large static band, and under it a small swatch behind a repaint boundary.
fn app(color: &Signal<Color>) -> WidgetNode {
    Flex::column()
        .children(children![
            ColoredBox::new(Color::RED).child(SizedBox::from_size(Size::new(SIDE_F, 120.0))),
            RepaintBoundary::new().child(Swatch {
                color: color.clone()
            }),
        ])
        .into()
}

#[test]
fn a_small_change_costs_a_small_number_of_pixels() {
    let mut driver = FrameDriver::new(Size::square(SIDE_F));
    let color = driver.elements().runtime().signal(Color::BLUE);
    driver.elements().set_root(app(&color));

    let mut surface = Surface::new();
    let first = surface.present(&mut driver);
    assert_eq!(
        first,
        u64::from(SIDE) * u64::from(SIDE),
        "the first frame has no previous contents to keep, so it costs all of it"
    );

    // Settle, so the next frame's damage is only what the signal changes.
    surface.present(&mut driver);

    color.set(Color::GREEN);
    let cost = surface.present(&mut driver);

    let full = u64::from(SIDE) * u64::from(SIDE);
    assert!(
        cost * 20 < full,
        "a {SWATCH}x{SWATCH} change on a {SIDE}x{SIDE} surface rasterised {cost} \
         of {full} pixels — damage is computed but not obeyed"
    );

    assert_eq!(
        surface.pixel(100, 130),
        (0, 255, 0, 255),
        "the swatch is repainted in the new colour"
    );
    assert_eq!(
        surface.pixel(100, 60),
        (255, 0, 0, 255),
        "and the band above it, which the frame never rasterised, still holds \
         what the previous frame drew"
    );
}

#[test]
fn the_root_layer_is_not_re_recorded_for_a_change_behind_a_boundary() {
    let mut driver = FrameDriver::new(Size::square(SIDE_F));
    let color = driver.elements().runtime().signal(Color::BLUE);
    driver.elements().set_root(app(&color));

    let mut surface = Surface::new();
    surface.present(&mut driver);
    surface.present(&mut driver);

    let layers = driver.layers();
    let root = layers.root().expect("a root layer");
    let boundary = *layers
        .layer(root)
        .children()
        .first()
        .expect("the boundary's layer");
    let root_before = layers.layer(root).paint_count();
    let boundary_before = layers.layer(boundary).paint_count();

    color.set(Color::GREEN);
    surface.present(&mut driver);

    let layers = driver.layers();
    assert_eq!(
        layers.layer(root).paint_count(),
        root_before,
        "the root layer must keep the commands it already had — re-recording it \
         would mean the boundary bought nothing"
    );
    assert_eq!(
        layers.layer(boundary).paint_count(),
        boundary_before + 1,
        "and the boundary's layer must be the one that re-records"
    );
}

#[test]
fn an_idle_frame_rasterises_nothing_at_all() {
    let mut driver = FrameDriver::new(Size::square(SIDE_F));
    let color = driver.elements().runtime().signal(Color::BLUE);
    driver.elements().set_root(app(&color));

    let mut surface = Surface::new();
    surface.present(&mut driver);
    surface.present(&mut driver);

    assert!(driver.damage().is_clean(), "{}", driver.damage());
    assert_eq!(
        surface.present(&mut driver),
        0,
        "a frame where nothing changed must not rasterise at all"
    );

    assert_eq!(
        surface.pixel(100, 60),
        (255, 0, 0, 255),
        "and the surface still holds the last real frame"
    );
    assert_eq!(surface.pixel(100, 130), (0, 0, 255, 255));
}

#[test]
fn damage_covers_where_the_change_actually_landed() {
    let mut driver = FrameDriver::new(Size::square(SIDE_F));
    let color = driver.elements().runtime().signal(Color::BLUE);
    driver.elements().set_root(app(&color));

    let mut surface = Surface::new();
    surface.present(&mut driver);
    surface.present(&mut driver);

    color.set(Color::GREEN);
    driver.draw_frame();

    // Under-reporting is the dangerous direction: it leaves stale pixels on the
    // screen, on some frames only. Over-reporting merely costs fill rate.
    assert!(
        driver.damage().intersects(SWATCH_RECT),
        "damage missed the thing that changed: {}",
        driver.damage()
    );
}
