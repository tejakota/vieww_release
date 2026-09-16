//! Does an interaction actually damage a small region — and would repainting
//! only that region be cheaper?
//!
//! # Why this has to be measured before anything is built
//!
//! "Wire damage through to the window backend" is only worth doing if the
//! damage a real interaction produces is genuinely small *and* the culled
//! scene is genuinely cheaper to render. Both are assumptions, and both are
//! testable in half a second with no window. If a click damages 80% of the
//! surface — because a hover state, a tooltip and a focus ring all move at
//! once, or because the layer tree reports coarse bounds — then a retained
//! surface buys a few percent and the effort belongs somewhere else.
//!
//! So this taps the real Studio shell, and reports what the framework itself
//! says changed, beside what each strategy actually costs.

use std::time::Instant;

use std::time::Duration;

use vieww_foundation::{Color, Offset, PointerEvent, PointerId, Size};
use vieww_paint::native::NativeRenderer;
use vieww_paint::Damage;
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

fn best<F: FnMut()>(runs: usize, mut f: F) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..runs {
        let start = Instant::now();
        f();
        best = best.min(start.elapsed().as_secs_f64());
    }
    best * 1000.0
}

fn main() {
    let size = viewwstudio::WINDOW;
    let (w, h) = (size.width as u32, size.height as u32);

    let mut driver = FrameDriver::new(size);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    driver.set_root(Shell {
        studio: Studio::new(&runtime),
    });

    // Settle: two frames, so the third describes the interaction rather than
    // the start-up.
    driver.draw_frame();
    driver.draw_frame();

    let mut renderer = NativeRenderer::new();
    let full = best(3, || {
        renderer
            .render_to_pixels(driver.scene(), w, h, Color::BLACK)
            .expect("full");
    });
    println!(
        "baseline: a full frame is {full:.1} ms, {} commands\n",
        driver.scene().len()
    );

    // Each interaction, from a settled tree.
    let mut previous = Offset::new(0.0, 0.0);
    let mut clock = Duration::from_millis(0);
    for (label, at) in [
        ("hover the activity bar", Offset::new(26.0, 120.0)),
        ("hover a file-tree row", Offset::new(160.0, 260.0)),
        ("move over the editor", Offset::new(800.0, 400.0)),
        ("hover the status bar", Offset::new(300.0, 880.0)),
    ] {
        clock += Duration::from_millis(16);
        driver.handle_pointer(&PointerEvent::moved(PointerId(1), previous, at, clock));
        previous = at;
        driver.draw_frame();

        let damage = driver.damage().clone();
        let surface_area = size.width * size.height;
        let ratio = damage.covered_area() / surface_area;
        let regions = damage.repaint_regions().len();

        let culled = best(3, || {
            renderer
                .render_damaged(driver.scene(), &damage, w, h, Color::BLACK)
                .expect("damaged");
        });
        let retained = best(3, || {
            renderer
                .render_retained(driver.scene(), &damage, w, h, Color::BLACK)
                .expect("retained");
        });

        println!(
            "{label:<26} {regions:>2} region(s) {:>6.1}%   culled {culled:>6.1} ms   \
             retained {retained:>6.2} ms   ({:.0}x the full frame)",
            ratio * 100.0,
            full / retained.max(1e-6)
        );
    }

    println!(
        "\n`culled` is the existing render_damaged: it skips commands but still fills\n\
         a whole framebuffer and converts every pixel on the way out. `retained` keeps\n\
         the previous frame's pixels and touches only the damaged regions."
    );

    // What the floor actually is, for comparison.
    let empty = vieww_paint::Scene::new();
    let floor = best(3, || {
        renderer
            .render_to_pixels(&empty, w, h, Color::BLACK)
            .expect("floor");
    });
    println!("an empty scene at this size still costs {floor:.1} ms");

    let _ = Damage::new;
    let _ = Size::new(0.0, 0.0);
}
