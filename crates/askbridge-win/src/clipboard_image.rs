use std::ptr;

use askbridge_core::{AppError, CapturedImage, Result};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HWND},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
    },
};

const CF_DIB_FORMAT: u32 = 8;

pub fn copy_image_to_clipboard(owner: HWND, image: &CapturedImage) -> Result<()> {
    let dib = rgba_to_cf_dib(image)?;
    // SAFETY: owner is a live AskBridge window and clipboard ownership is scoped by CloseClipboard.
    if unsafe { OpenClipboard(owner) } == 0 {
        return Err(AppError::ClipboardUnavailable);
    }
    let result = (|| {
        // SAFETY: Clipboard is open for this process.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(AppError::ClipboardWriteFailed);
        }
        // SAFETY: GlobalAlloc returns a moveable handle suitable for SetClipboardData.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, dib.len()) };
        if handle.is_null() {
            return Err(AppError::ClipboardWriteFailed);
        }
        // SAFETY: The handle was allocated above; lock gives writable memory of dib.len().
        let locked = unsafe { GlobalLock(handle) as *mut u8 };
        if locked.is_null() {
            // SAFETY: Clipboard does not own the handle because SetClipboardData was not called.
            unsafe {
                GlobalFree(handle);
            }
            return Err(AppError::ClipboardWriteFailed);
        }
        // SAFETY: locked points to dib.len() bytes and dib is at least that long.
        unsafe {
            ptr::copy_nonoverlapping(dib.as_ptr(), locked, dib.len());
            GlobalUnlock(handle);
        }
        // SAFETY: On success, the clipboard owns handle and it must not be freed by us.
        if unsafe { SetClipboardData(CF_DIB_FORMAT, handle) }.is_null() {
            // SAFETY: Clipboard did not take ownership when SetClipboardData failed.
            unsafe {
                GlobalFree(handle);
            }
            return Err(AppError::ClipboardWriteFailed);
        }
        Ok(())
    })();
    // SAFETY: Clipboard was opened successfully above.
    unsafe {
        CloseClipboard();
    }
    result
}

fn rgba_to_cf_dib(image: &CapturedImage) -> Result<Vec<u8>> {
    let pixel_count = (image.width as usize)
        .checked_mul(image.height as usize)
        .ok_or(AppError::ClipboardWriteFailed)?;
    let pixel_bytes = pixel_count
        .checked_mul(4)
        .ok_or(AppError::ClipboardWriteFailed)?;
    if image.rgba_bytes.len() != pixel_bytes {
        return Err(AppError::ClipboardWriteFailed);
    }
    let mut dib = Vec::with_capacity(40 + pixel_bytes);
    push_u32(&mut dib, 40);
    push_i32(
        &mut dib,
        i32::try_from(image.width).map_err(|_| AppError::ClipboardWriteFailed)?,
    );
    push_i32(
        &mut dib,
        i32::try_from(image.height).map_err(|_| AppError::ClipboardWriteFailed)?,
    );
    push_u16(&mut dib, 1);
    push_u16(&mut dib, 32);
    push_u32(&mut dib, 0);
    push_u32(
        &mut dib,
        u32::try_from(pixel_bytes).map_err(|_| AppError::ClipboardWriteFailed)?,
    );
    push_i32(&mut dib, 0);
    push_i32(&mut dib, 0);
    push_u32(&mut dib, 0);
    push_u32(&mut dib, 0);

    let row_len = image.width as usize * 4;
    for y in (0..image.height as usize).rev() {
        let row = &image.rgba_bytes[y * row_len..(y + 1) * row_len];
        for rgba in row.chunks_exact(4) {
            dib.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
    }
    Ok(dib)
}

fn push_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(buffer: &mut Vec<u8>, value: i32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use askbridge_core::ScreenRect;

    #[test]
    fn cf_dib_uses_bitmapinfo_header_and_bottom_up_bgra_pixels() {
        let image = CapturedImage::new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 255, 255, 255, 64,
            ],
            ScreenRect::new(0, 0, 2, 2),
        )
        .expect("image");

        let dib = rgba_to_cf_dib(&image).expect("dib");

        assert_eq!(&dib[0..4], &40_u32.to_le_bytes());
        assert_eq!(&dib[4..8], &2_i32.to_le_bytes());
        assert_eq!(&dib[8..12], &2_i32.to_le_bytes());
        assert_eq!(&dib[14..16], &32_u16.to_le_bytes());
        assert_eq!(
            &dib[40..56],
            &[
                255, 0, 0, 128, 255, 255, 255, 64, 0, 0, 255, 255, 0, 255, 0, 255
            ]
        );
    }
}
