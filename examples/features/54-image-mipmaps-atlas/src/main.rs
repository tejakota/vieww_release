//! `vieww-image`'s two pure-CPU pieces, run over synthetic pixels and
//! reported to the terminal — no window, no GPU, nothing to look at, only
//! to read.
//!
//! ```console
//! cargo run -p feature-image-mipmaps-atlas
//! ```
//!
//! Two things are demonstrated:
//!
//! 1. **Mipmap generation** ([`vieww_image::mipmap`]): an 8x8 checkerboard
//!    is built in code, its full mip chain is generated, and each level's
//!    size and top-left pixel are printed — watch the size halve every
//!    level down to 1x1, and the top-left pixel drift from pure red toward
//!    the image's overall average as more of the checkerboard gets folded
//!    into that one texel.
//! 2. **Atlas packing** ([`vieww_image::atlas`]): three small, distinctly
//!    coloured images are packed into one atlas and composited into a
//!    single output buffer, whose pixel at each reported placement is
//!    printed back out to confirm it landed exactly where the packer said.

use vieww_foundation::Image;
use vieww_image::atlas::{self, AtlasPacker};
use vieww_image::mipmap::generate_mip_chain;

fn checkerboard(size: u32) -> Image {
    let mut pixels = vec![0u8; size as usize * size as usize * 4];
    for y in 0..size {
        for x in 0..size {
            let index = ((y * size + x) * 4) as usize;
            let on = (x / 2 + y / 2) % 2 == 0;
            let color = if on {
                [220u8, 30, 30, 255]
            } else {
                [20, 30, 220, 255]
            };
            pixels[index..index + 4].copy_from_slice(&color);
        }
    }
    Image::from_rgba8(pixels, size, size)
}

fn solid(width: u32, height: u32, color: [u8; 4]) -> Image {
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.copy_from_slice(&color);
    }
    Image::from_rgba8(pixels, width, height)
}

fn main() {
    println!("== mipmap chain ==");
    let source = checkerboard(8);
    let chain = generate_mip_chain(&source);
    for (level, image) in chain.iter().enumerate() {
        let top_left = &image.pixels()[0..4];
        println!(
            "  level {level}: {:>2}x{:<2}  top-left rgba = {top_left:?}",
            image.width(),
            image.height()
        );
    }
    assert_eq!(chain.last().unwrap().width(), 1, "the chain must reach 1x1");
    assert_eq!(
        chain.last().unwrap().height(),
        1,
        "the chain must reach 1x1"
    );

    println!("\n== atlas packing ==");
    let red = solid(4, 4, [255, 0, 0, 255]);
    let green = solid(3, 5, [0, 255, 0, 255]);
    let blue = solid(2, 2, [0, 0, 255, 255]);
    let requests = [
        (red.width(), red.height()),
        (green.width(), green.height()),
        (blue.width(), blue.height()),
    ];

    let packer = AtlasPacker::new(16, 16);
    let result = packer.pack(&requests);
    println!(
        "  packed {} requests into a {}x{} atlas, all fit: {}",
        requests.len(),
        packer.width(),
        packer.height(),
        result.all_fit()
    );

    let placements: Vec<(&Image, atlas::AtlasRect)> = [&red, &green, &blue]
        .into_iter()
        .zip(&result.placements)
        .map(|(image, rect)| {
            (
                image,
                rect.expect("small enough to always fit a 16x16 atlas"),
            )
        })
        .collect();

    for (name, (image, rect)) in ["red", "green", "blue"].iter().zip(&placements) {
        println!(
            "  {name}: {}x{} placed at ({}, {})",
            image.width(),
            image.height(),
            rect.x,
            rect.y
        );
    }

    let atlas_image = atlas::composite(packer.width(), packer.height(), &placements);
    println!(
        "  composited atlas is {}x{} ({} bytes)",
        atlas_image.width(),
        atlas_image.height(),
        atlas_image.pixels().len()
    );

    for (name, (_, rect)) in ["red", "green", "blue"].iter().zip(&placements) {
        let index = ((rect.y * atlas_image.width() + rect.x) * 4) as usize;
        let pixel = &atlas_image.pixels()[index..index + 4];
        println!("  atlas pixel at {name}'s placement origin = {pixel:?}");
    }
}
