//! Smoke test for [`vieww_hal::vulkan::VulkanDevice::render_mesh_to_pixels`]:
//! tessellate a real path with `vieww-gpu`, rasterize it on a real Vulkan
//! device, and check that the drawn triangle actually landed where the
//! geometry says it should.
//!
//! Same honesty bar as `vulkan_smoke.rs`: `#[ignore]`d because a Vulkan
//! loader + ICD (`lavapipe` or a real GPU) is not something every machine
//! running `cargo test` has. In the sandbox this delivery was built in, the
//! loader is present but no ICD is registered — confirmed directly:
//! `VulkanDevice::new()` fails with `ERROR_INCOMPATIBLE_DRIVER`, not a
//! missing-library error, which is exactly the "no GPU here" case this
//! delivery's task description anticipated. On a GPU-backed machine (or one
//! with `lavapipe` installed) this test runs for real.

#![cfg(feature = "vulkan")]

use vieww_foundation::{Color, Offset, Path};
use vieww_gpu::tessellate_fill;
use vieww_hal::vulkan::VulkanDevice;

fn try_device() -> Option<VulkanDevice> {
    match VulkanDevice::new() {
        Ok(device) => Some(device),
        Err(error) => {
            eprintln!("skipping: no Vulkan device available ({error})");
            None
        }
    }
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn a_tessellated_square_rasterizes_into_its_own_bounds() {
    let Some(device) = try_device() else { return };

    // **Deliberately asymmetric about both axes.** This test drew a square
    // spanning 20..80 in a 100x100 frame — centred, and therefore identical
    // to its own reflection in either axis. That made it blind to a mirrored
    // frame, and the shader it exercises (`solid.wgsl`) was in fact emitting
    // one: it wrote the Vulkan Y convention while `naga`'s SPIR-V writer was
    // already flipping Y for it. Every assertion below passed throughout.
    //
    // The rectangle is now near the top-left, so a flip in either axis moves
    // the "inside" sample outside it.
    let mut path = Path::new();
    path.move_to(Offset::new(10.0, 10.0));
    path.line_to(Offset::new(60.0, 10.0));
    path.line_to(Offset::new(60.0, 35.0));
    path.line_to(Offset::new(10.0, 35.0));
    path.close();

    let mesh = tessellate_fill(&path, 0.1);
    assert!(mesh.triangle_count() >= 2);

    let pixels = device
        .render_mesh_to_pixels(
            100,
            100,
            &mesh.positions,
            &mesh.indices,
            Color::rgba(255, 0, 0, 255),
        )
        .expect("render_mesh_to_pixels");

    let at = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * 100 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };

    // Inside the rectangle: opaque red.
    assert_eq!(
        at(35, 22),
        [255, 0, 0, 255],
        "inside the rectangle should be opaque red"
    );

    // Outside it, on every side — and specifically at the points its own
    // mirror images would occupy, which is what makes a flip fail here.
    for (x, y, why) in [
        (5, 5, "left of and above it"),
        (90, 90, "right of and below it"),
        (35, 78, "where a vertical flip would put it"),
        (85, 22, "where a horizontal flip would put it"),
    ] {
        let outside = at(x, y);
        assert_eq!(
            outside[3], 0,
            "({x}, {y}) is {why} and should be transparent, got {outside:?}"
        );
    }
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn an_empty_mesh_draws_a_fully_transparent_frame() {
    let Some(device) = try_device() else { return };
    let pixels = device
        .render_mesh_to_pixels(16, 16, &[], &[], Color::rgba(0, 255, 0, 255))
        .expect("render_mesh_to_pixels");
    assert!(pixels.iter().all(|&b| b == 0));
}
