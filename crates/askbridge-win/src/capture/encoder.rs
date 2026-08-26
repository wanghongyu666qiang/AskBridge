use askbridge_core::{AppError, CapturedImage, Result, ScreenRect};
use png::{BitDepth, ColorType, Compression, Encoder};

use super::screen::RawBgraImage;

pub fn captured_image(mut raw: RawBgraImage, source_rect: ScreenRect) -> Result<CapturedImage> {
    bgra_to_rgba_in_place(&mut raw.pixels)?;
    CapturedImage::new(raw.width, raw.height, raw.pixels, source_rect)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn encode_png(image: &CapturedImage) -> Result<Vec<u8>> {
    let mut png_bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut png_bytes, image.width, image.height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_compression(Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|error| AppError::CaptureFailed(format!("PNG header failed: {error}")))?;
        writer
            .write_image_data(&image.rgba_bytes)
            .map_err(|error| AppError::CaptureFailed(format!("PNG encoding failed: {error}")))?;
    }
    Ok(png_bytes)
}

fn bgra_to_rgba_in_place(pixels: &mut [u8]) -> Result<()> {
    if pixels.len() % 4 != 0 {
        return Err(AppError::CaptureFailed(
            "BGRA capture buffer length is invalid".to_owned(),
        ));
    }
    // In-place channel swap avoids a full-size copy of multi-monitor captures.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bgra_pixels_to_opaque_rgba() {
        let mut pixels = vec![3, 2, 1, 0, 30, 20, 10, 128];
        bgra_to_rgba_in_place(&mut pixels).expect("valid pixels");
        assert_eq!(pixels, vec![1, 2, 3, 255, 10, 20, 30, 255]);
    }

    #[test]
    fn rejects_incomplete_bgra_pixels() {
        let mut pixels = vec![1, 2, 3];
        assert!(bgra_to_rgba_in_place(&mut pixels).is_err());
    }

    #[test]
    fn encodes_png_in_memory() {
        let raw = RawBgraImage {
            width: 1,
            height: 1,
            pixels: vec![30, 20, 10, 0],
        };
        let rect = ScreenRect::new(-5, 7, 1, 1);

        let image = captured_image(raw, rect).expect("capture conversion succeeds");
        let png_bytes = encode_png(&image).expect("PNG encoding succeeds");

        assert_eq!(image.source_rect, rect);
        assert_eq!(image.rgba_bytes, vec![10, 20, 30, 255]);
        assert!(png_bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));
    }
}
