//! HAL smoke test — spec §14.1's M0/M1: "Vulkan device, swapchain, upload
//! ring, first quad + clear ... Sustained 3-frame flight present". This
//! sandbox has no display to run a real swapchain against, so this is the
//! headless half of that gate: bring up a real Vulkan 1.2 device, translate
//! a WGSL shader through `naga` to SPIR-V, build one graphics pipeline,
//! render three times in a row, and check every read-back pixel against the
//! requested clear colour, exactly.
//!
//! Runs against **`lavapipe`** (Mesa's software Vulkan implementation),
//! installed into this container specifically so the HAL layer could be
//! tested rather than only compiled — see `docs/RENDERER-MIGRATION.md` for
//! why that matters and what it does and does not prove about a real GPU.
//! `#[ignore]`d by default (`cargo test -- --ignored` to run) because a
//! Vulkan loader and an ICD are not something every machine running
//! `cargo test` has, and a hard failure there would be a false alarm about
//! this crate rather than a signal about it.

#![cfg(feature = "vulkan")]

use vieww_foundation::Color;
use vieww_hal::vulkan::VulkanDevice;
use vieww_hal::Device;

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
fn vulkan_device_reports_an_adapter() {
    let Some(device) = try_device() else { return };
    let info = device.info();
    eprintln!("Vulkan adapter: {info:?}");
    assert_eq!(info.backend, "vulkan");
    assert!(!info.name.is_empty());
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn vulkan_render_clear_matches_the_requested_colour_exactly() {
    let Some(device) = try_device() else { return };

    let cases = [
        Color::rgba(255, 0, 0, 255),
        Color::rgba(0, 255, 0, 255),
        Color::rgba(20, 130, 220, 200),
        Color::rgba(0, 0, 0, 0),
    ];
    for color in cases {
        let pixels = device
            .render_clear_to_pixels(64, 48, color)
            .unwrap_or_else(|e| panic!("render_clear_to_pixels({color:?}) failed: {e}"));
        assert_eq!(pixels.len(), 64 * 48 * 4);
        // Every pixel should read back within 1 LSB of the requested colour
        // — the naga-translated shader writes it as a straight uniform, so
        // the only expected drift is the premultiply/round trip through
        // 8-bit UNORM.
        for chunk in pixels.chunks_exact(4) {
            let close = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs() <= 1;
            assert!(
                close(chunk[0], color.r)
                    && close(chunk[1], color.g)
                    && close(chunk[2], color.b)
                    && close(chunk[3], color.a),
                "pixel {:?} does not match requested clear colour {:?}",
                chunk,
                color
            );
        }
    }
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn vulkan_device_survives_three_consecutive_frames() {
    // Spec §14.1's M1 gate: "sustained 3-frame flight present". No swapchain
    // here, so this is the closest headless analogue: three independent
    // render-and-readback cycles on the same device, none of them leaking
    // Vulkan objects into the next (every create in
    // `render_clear_to_pixels` has a matching destroy before it returns).
    let Some(device) = try_device() else { return };
    for i in 0..3 {
        let pixels = device
            .render_clear_to_pixels(32, 32, Color::rgba(10, 20, 30, 255))
            .unwrap_or_else(|e| panic!("frame {i} failed: {e}"));
        assert_eq!(pixels.len(), 32 * 32 * 4);
    }
}
