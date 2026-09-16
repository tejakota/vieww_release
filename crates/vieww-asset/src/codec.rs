//! Bytes to pixels.
//!
//! Pure, synchronous, and with no idea where the bytes came from — which is what
//! makes it testable with no platform attached and safe to hand to a worker
//! thread.

use vieww_foundation::Image;

use crate::AssetError;

/// What an encoded image turned out to be.
///
/// Reported rather than asked for: the format is read from the bytes, because a
/// file extension is a claim by whoever named the file and the bytes are the
/// truth. A `.png` that is really a JPEG is common enough to be worth not
/// caring about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
    Gif,
    Bmp,
    /// Decoded, but not one of the formats this enum names.
    Other,
}

impl ImageFormat {
    fn of(format: image::ImageFormat) -> Self {
        match format {
            image::ImageFormat::Png => Self::Png,
            image::ImageFormat::Jpeg => Self::Jpeg,
            image::ImageFormat::WebP => Self::WebP,
            image::ImageFormat::Gif => Self::Gif,
            image::ImageFormat::Bmp => Self::Bmp,
            _ => Self::Other,
        }
    }
}

/// Decode an encoded image into straight-alpha RGBA8 pixels.
///
/// # Straight alpha, not premultiplied
///
/// [`Image::from_rgba8`] documents its input as straight alpha and the GPU
/// backend tags it that way for vello. `image`'s `to_rgba8` is also straight, so
/// no conversion happens here — but the agreement is worth stating, because
/// premultiplying by mistake darkens every semi-transparent pixel and looks like
/// a colour-space bug rather than a decode one.
///
/// # Errors
///
/// [`AssetError::Decode`] if the bytes are not an image this build can read.
/// Note "this build": WebP and GIF are behind features, so the same bytes can
/// decode in one configuration and not another, and the message says so.
pub fn decode(bytes: &[u8]) -> Result<(Image, ImageFormat), AssetError> {
    let format = image::guess_format(bytes)
        .map_err(|error| AssetError::Decode(format!("unrecognised image: {error}")))?;

    let decoded = image::load_from_memory(bytes).map_err(|error| {
        AssetError::Decode(format!(
            "decoding {format:?}: {error} — if this format is behind a \
             `vieww-asset` feature, it may simply not be enabled in this build"
        ))
    })?;

    let rgba = decoded.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Ok((
        Image::from_rgba8(rgba.into_raw(), width, height),
        ImageFormat::of(format),
    ))
}

/// Decode, and scale down to fit within `max_width` x `max_height`.
///
/// # Why this exists rather than letting layout scale it
///
/// A camera photograph is 4000 pixels wide and a thumbnail is 80. Decoding the
/// full image costs 64MB of RGBA and then hands the GPU a texture 50 times
/// larger than the space it occupies, every frame it is on screen. Scaling at
/// decode time is the only place the cost can be avoided rather than moved.
///
/// Only ever scales **down** — an image smaller than the box is left alone,
/// because upscaling at decode time bakes in blur that a GPU would have done
/// better and cheaper.
///
/// # Errors
///
/// As [`decode`].
pub fn decode_sized(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
) -> Result<(Image, ImageFormat), AssetError> {
    let format = image::guess_format(bytes)
        .map_err(|error| AssetError::Decode(format!("unrecognised image: {error}")))?;
    let decoded = image::load_from_memory(bytes)
        .map_err(|error| AssetError::Decode(format!("decoding {format:?}: {error}")))?;

    let scaled = if decoded.width() > max_width.max(1) || decoded.height() > max_height.max(1) {
        // `thumbnail` rather than `resize`: it is a box filter with a
        // nearest-neighbour prepass, which is several times faster and visually
        // indistinguishable at the sizes a thumbnail is displayed at. A
        // full-quality Lanczos here would be paying for detail the destination
        // rectangle cannot show.
        decoded.thumbnail(max_width.max(1), max_height.max(1))
    } else {
        decoded
    };

    let rgba = scaled.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Ok((
        Image::from_rgba8(rgba.into_raw(), width, height),
        ImageFormat::of(format),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2x2 PNG: red, green / blue, transparent.
    ///
    /// Encoded here rather than checked in as a file, so the test has no
    /// fixture to lose and the encoder and decoder are exercised against each
    /// other.
    fn png_2x2() -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let pixels: Vec<u8> = vec![
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            0, 0, 0, 0, // transparent
        ];
        let image = image::RgbaImage::from_raw(2, 2, pixels).expect("a 2x2 image");
        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .expect("encode");
        buffer.into_inner()
    }

    #[test]
    fn a_png_decodes_to_the_pixels_it_was_made_from() {
        let (decoded, format) = decode(&png_2x2()).expect("decode");
        assert_eq!(format, ImageFormat::Png);
        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 2);
        assert_eq!(
            &decoded.pixels()[0..4],
            &[255, 0, 0, 255],
            "top left is red"
        );
    }

    #[test]
    fn transparency_survives_as_straight_alpha() {
        // Premultiplying here would make this pixel `0,0,0,0` too — which it
        // already is — so the test uses a *half*-transparent colour, where the
        // two conventions genuinely differ.
        let mut buffer = std::io::Cursor::new(Vec::new());
        let image = image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 128]).expect("a 1x1 image");
        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .expect("encode");

        let (decoded, _) = decode(&buffer.into_inner()).expect("decode");
        assert_eq!(
            decoded.pixels(),
            &[255, 0, 0, 128],
            "premultiplied would be 128,0,0,128 and every fade would look wrong"
        );
    }

    #[test]
    fn bytes_that_are_not_an_image_fail_rather_than_producing_noise() {
        let error = decode(b"this is not a picture").unwrap_err();
        assert!(matches!(error, AssetError::Decode(_)), "{error}");
    }

    #[test]
    fn the_format_is_read_from_the_bytes_rather_than_trusted() {
        // No filename is involved anywhere in this API, which is the point.
        let (_, format) = decode(&png_2x2()).expect("decode");
        assert_eq!(format, ImageFormat::Png);
    }

    #[test]
    fn decoding_to_a_size_scales_a_large_image_down() {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let image = image::RgbaImage::from_raw(64, 64, vec![255; 64 * 64 * 4]).expect("image");
        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .expect("encode");

        let (decoded, _) = decode_sized(&buffer.into_inner(), 8, 8).expect("decode");
        assert!(
            decoded.width() <= 8 && decoded.height() <= 8,
            "got {}x{}",
            decoded.width(),
            decoded.height()
        );
    }

    #[test]
    fn decoding_to_a_size_never_scales_a_small_image_up() {
        // Upscaling at decode time bakes in blur the GPU would have done better.
        let (decoded, _) = decode_sized(&png_2x2(), 512, 512).expect("decode");
        assert_eq!((decoded.width(), decoded.height()), (2, 2));
    }

    #[test]
    fn a_zero_target_size_does_not_divide_by_zero() {
        let (decoded, _) = decode_sized(&png_2x2(), 0, 0).expect("decode");
        assert!(decoded.width() >= 1 && decoded.height() >= 1);
    }
}
