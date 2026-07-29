use askbridge_core::{AppError, CapturedImage, Result, ScreenRect};
use png::{BitDepth, ColorType, Compression, Encoder};

use super::screen::RawBgraImage;

pub fn captured_image(raw: &RawBgraImage, source_rect: ScreenRect) -> Result<CapturedImage> {
    let rgba_bytes = bgra_to_rgba(&raw.pixels)?;
    CapturedImage::new(raw.width, raw.height, rgba_bytes, source_rect)
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

fn bgra_to_rgba(pixels: &[u8]) -> Result<Vec<u8>> {
    if pixels.len() % 4 != 0 {
        return Err(AppError::CaptureFailed(
            "BGRA capture buffer length is invalid".to_owned(),
        ));
    }
    let mut rgba = pixels.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bgra_pixels_to_opaque_rgba() {
        assert_eq!(
            bgra_to_rgba(&[3, 2, 1, 0, 30, 20, 10, 128]).expect("valid pixels"),
            vec![1, 2, 3, 255, 10, 20, 30, 255]
        );
    }

    #[test]
    fn rejects_incomplete_bgra_pixels() {
        assert!(bgra_to_rgba(&[1, 2, 3]).is_err());
    }

    #[test]
    fn encodes_png_in_memory() {
        let raw = RawBgraImage {
            width: 1,
            height: 1,
            pixels: vec![30, 20, 10, 0],
        };
        let rect = ScreenRect::new(-5, 7, 1, 1);

        let image = captured_image(&raw, rect).expect("capture conversion succeeds");
        let png_bytes = encode_png(&image).expect("PNG encoding succeeds");

        assert_eq!(image.source_rect, rect);
        assert_eq!(image.rgba_bytes, vec![10, 20, 30, 255]);
        assert!(png_bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));
    }
}
