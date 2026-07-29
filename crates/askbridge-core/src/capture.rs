use serde::{Deserialize, Serialize};

use crate::{AppError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenRect {
    pub const fn new(left: i32, top: i32, width: u32, height: u32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    pub fn from_points(start: (i32, i32), end: (i32, i32)) -> Option<Self> {
        let left = start.0.min(end.0);
        let top = start.1.min(end.1);
        let width = u32::try_from(i64::from(start.0).abs_diff(i64::from(end.0))).ok()?;
        let height = u32::try_from(i64::from(start.1).abs_diff(i64::from(end.1))).ok()?;
        (width > 0 && height > 0).then_some(Self::new(left, top, width, height))
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn translated(self, delta_x: i32, delta_y: i32) -> Option<Self> {
        Some(Self::new(
            self.left.checked_add(delta_x)?,
            self.top.checked_add(delta_y)?,
            self.width,
            self.height,
        ))
    }

    pub const fn right(self) -> i64 {
        self.left as i64 + self.width as i64
    }

    pub const fn bottom(self) -> i64 {
        self.top as i64 + self.height as i64
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba_bytes: Vec<u8>,
    pub source_rect: ScreenRect,
}

impl CapturedImage {
    pub fn new(
        width: u32,
        height: u32,
        rgba_bytes: Vec<u8>,
        source_rect: ScreenRect,
    ) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(AppError::CaptureFailed(
                "captured image dimensions must be non-zero".to_owned(),
            ));
        }
        if source_rect.width != width || source_rect.height != height {
            return Err(AppError::CaptureFailed(
                "captured image dimensions do not match the source rectangle".to_owned(),
            ));
        }
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| AppError::CaptureFailed("captured image is too large".to_owned()))?;
        if rgba_bytes.len() != expected_len {
            return Err(AppError::CaptureFailed(
                "captured image RGBA buffer length is invalid".to_owned(),
            ));
        }
        Ok(Self {
            width,
            height,
            rgba_bytes,
            source_rect,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_reverse_drag_with_negative_coordinates() {
        let rect = ScreenRect::from_points((240, 100), (-160, -80)).expect("non-empty selection");

        assert_eq!(rect, ScreenRect::new(-160, -80, 400, 180));
        assert_eq!(rect.right(), 240);
        assert_eq!(rect.bottom(), 100);
    }

    #[test]
    fn rejects_zero_area_selection() {
        assert_eq!(ScreenRect::from_points((10, 10), (10, 40)), None);
        assert_eq!(ScreenRect::from_points((10, 10), (40, 10)), None);
    }

    #[test]
    fn translates_client_selection_to_virtual_screen_coordinates() {
        let local = ScreenRect::new(20, 30, 640, 480);

        assert_eq!(
            local.translated(-1920, -200),
            Some(ScreenRect::new(-1900, -170, 640, 480))
        );
    }

    #[test]
    fn rejects_translation_overflow() {
        assert_eq!(ScreenRect::new(i32::MAX, 0, 1, 1).translated(1, 0), None);
    }

    #[test]
    fn captured_image_requires_matching_non_empty_data() {
        let rect = ScreenRect::new(-100, 20, 2, 3);
        let image = CapturedImage::new(2, 3, vec![0; 24], rect).expect("valid captured image");

        assert_eq!(image.source_rect, rect);
        assert_eq!(image.rgba_bytes.len(), 24);
        assert!(CapturedImage::new(3, 2, vec![0; 24], rect).is_err());
        assert!(CapturedImage::new(2, 3, vec![0; 23], rect).is_err());
        assert!(CapturedImage::new(2, 3, Vec::new(), rect).is_err());
    }
}
