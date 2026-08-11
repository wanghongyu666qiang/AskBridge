use std::{ptr, slice};

use askbridge_core::{AppError, CapturedImage, Result};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
            OpenClipboard, SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
    },
};

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;

pub struct ClipboardSession {
    owner: HWND,
    snapshot: Vec<(u32, Vec<u8>)>,
}

impl ClipboardSession {
    pub fn begin(owner: HWND) -> Result<Self> {
        let clipboard = ClipboardLock::open(owner)?;
        let mut snapshot = Vec::new();
        for format in [CF_UNICODETEXT, CF_DIB] {
            if let Some(bytes) = clipboard.read_global(format) {
                snapshot.push((format, bytes));
            }
        }
        drop(clipboard);
        Ok(Self { owner, snapshot })
    }

    pub fn copy_text(&self, text: &str) -> Result<()> {
        let mut utf16: Vec<u16> = text.encode_utf16().collect();
        utf16.push(0);
        // SAFETY: u16 has no padding and the byte slice remains live through set_formats.
        let bytes = unsafe {
            slice::from_raw_parts(utf16.as_ptr().cast::<u8>(), utf16.len() * size_of::<u16>())
        };
        self.set_formats(&[(CF_UNICODETEXT, bytes)])
    }

    pub fn copy_image(&self, image: &CapturedImage) -> Result<()> {
        let dib = dib_bytes(image)?;
        self.set_formats(&[(CF_DIB, &dib)])
    }

    fn set_formats(&self, formats: &[(u32, &[u8])]) -> Result<()> {
        let clipboard = ClipboardLock::open(self.owner)?;
        // SAFETY: The clipboard is open for this thread.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(AppError::ClipboardWriteFailed);
        }
        for (format, bytes) in formats {
            set_global(*format, bytes)?;
        }
        drop(clipboard);
        Ok(())
    }

    fn restore(&self) -> Result<()> {
        let clipboard = ClipboardLock::open(self.owner)?;
        // SAFETY: The clipboard is open for this thread.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(AppError::ClipboardRestoreFailed);
        }
        for (format, bytes) in &self.snapshot {
            set_global(*format, bytes).map_err(|_| AppError::ClipboardRestoreFailed)?;
        }
        drop(clipboard);
        Ok(())
    }
}

impl Drop for ClipboardSession {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

struct ClipboardLock;

impl ClipboardLock {
    fn open(owner: HWND) -> Result<Self> {
        // SAFETY: owner is the live hidden application window or null during tests.
        if unsafe { OpenClipboard(owner) } == 0 {
            return Err(AppError::ClipboardUnavailable);
        }
        Ok(Self)
    }

    fn read_global(&self, format: u32) -> Option<Vec<u8>> {
        // SAFETY: The clipboard is open and the handle is only borrowed while locked.
        unsafe {
            if IsClipboardFormatAvailable(format) == 0 {
                return None;
            }
            let handle = GetClipboardData(format);
            if handle.is_null() {
                return None;
            }
            let size = GlobalSize(handle as HGLOBAL);
            if size == 0 || size > 64 * 1024 * 1024 {
                return None;
            }
            let data = GlobalLock(handle as HGLOBAL);
            if data.is_null() {
                return None;
            }
            let bytes = slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
            GlobalUnlock(handle as HGLOBAL);
            Some(bytes)
        }
    }
}

impl Drop for ClipboardLock {
    fn drop(&mut self) {
        // SAFETY: This guard exists only after OpenClipboard succeeded.
        unsafe {
            CloseClipboard();
        }
    }
}

fn set_global(format: u32, bytes: &[u8]) -> Result<()> {
    // SAFETY: Allocation is movable as required by SetClipboardData. Ownership is
    // transferred only after SetClipboardData succeeds.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if memory.is_null() {
            return Err(AppError::ClipboardWriteFailed);
        }
        let destination = GlobalLock(memory);
        if destination.is_null() {
            GlobalFree(memory);
            return Err(AppError::ClipboardWriteFailed);
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast::<u8>(), bytes.len());
        GlobalUnlock(memory);
        if SetClipboardData(format, memory as HANDLE).is_null() {
            GlobalFree(memory);
            return Err(AppError::ClipboardWriteFailed);
        }
    }
    Ok(())
}

fn dib_bytes(image: &CapturedImage) -> Result<Vec<u8>> {
    let pixel_bytes = image
        .width
        .checked_mul(image.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| AppError::CaptureFailed("clipboard image is too large".to_owned()))?
        as usize;
    let mut dib = Vec::with_capacity(40 + pixel_bytes);
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(image.width as i32).to_le_bytes());
    dib.extend_from_slice(&(image.height as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    let row_bytes = image.width as usize * 4;
    for row in image.rgba_bytes.chunks_exact(row_bytes).rev() {
        for pixel in row.chunks_exact(4) {
            dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 0]);
        }
    }
    Ok(dib)
}

#[cfg(test)]
mod tests {
    use super::*;
    use askbridge_core::ScreenRect;

    #[test]
    fn builds_bottom_up_bgra_dib_without_mutating_source() {
        let image = CapturedImage::new(
            1,
            2,
            vec![1, 2, 3, 255, 10, 20, 30, 255],
            ScreenRect::new(0, 0, 1, 2),
        )
        .expect("image");
        let dib = dib_bytes(&image).expect("dib");

        assert_eq!(&dib[..4], &40u32.to_le_bytes());
        assert_eq!(&dib[40..44], &[30, 20, 10, 0]);
        assert_eq!(&dib[44..48], &[3, 2, 1, 0]);
        assert_eq!(image.rgba_bytes[0..4], [1, 2, 3, 255]);
    }
}
