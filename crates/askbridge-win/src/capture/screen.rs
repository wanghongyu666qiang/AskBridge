use std::{mem::size_of, ptr};

use askbridge_core::{AppError, Result, ScreenRect};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC,
    ReleaseDC, SRCCOPY, SelectObject,
};

use crate::util::last_error;

pub struct RawBgraImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn capture_screen_rect(rect: ScreenRect) -> Result<RawBgraImage> {
    if rect.is_empty() {
        return Err(AppError::CaptureFailed(
            "cannot capture an empty screen rectangle".to_owned(),
        ));
    }
    let width = i32::try_from(rect.width)
        .map_err(|_| AppError::CaptureFailed("capture width exceeds Win32 limits".to_owned()))?;
    let height = i32::try_from(rect.height)
        .map_err(|_| AppError::CaptureFailed("capture height exceeds Win32 limits".to_owned()))?;
    let byte_count = (rect.width as usize)
        .checked_mul(rect.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| AppError::CaptureFailed("capture buffer is too large".to_owned()))?;
    let size_image = u32::try_from(byte_count)
        .map_err(|_| AppError::CaptureFailed("capture buffer exceeds DIB limits".to_owned()))?;

    // SAFETY: A null HWND requests a device context for the entire virtual screen.
    let screen_dc = unsafe { GetDC(ptr::null_mut()) };
    if screen_dc.is_null() {
        return Err(AppError::Windows {
            operation: "GetDC(screen)",
            win32_code: last_error(),
        });
    }
    let screen_dc = ScreenDc(screen_dc);

    // SAFETY: screen_dc is valid for the lifetime of this function.
    let memory_dc = unsafe { CreateCompatibleDC(screen_dc.0) };
    if memory_dc.is_null() {
        return Err(AppError::Windows {
            operation: "CreateCompatibleDC",
            win32_code: last_error(),
        });
    }
    let memory_dc = MemoryDc(memory_dc);

    // SAFETY: screen_dc is valid and dimensions were checked for Win32.
    let bitmap = unsafe { CreateCompatibleBitmap(screen_dc.0, width, height) };
    if bitmap.is_null() {
        return Err(AppError::Windows {
            operation: "CreateCompatibleBitmap",
            win32_code: last_error(),
        });
    }
    let bitmap = OwnedBitmap(bitmap);

    // SAFETY: Both handles are valid GDI objects.
    let old_bitmap = unsafe { SelectObject(memory_dc.0, bitmap.0) };
    if old_bitmap.is_null() {
        return Err(AppError::Windows {
            operation: "SelectObject(capture bitmap)",
            win32_code: last_error(),
        });
    }
    let mut selection = SelectedObject {
        dc: memory_dc.0,
        old: old_bitmap,
    };

    // SAFETY: Source and destination DCs are valid; the selected bitmap matches dimensions.
    let copied = unsafe {
        BitBlt(
            memory_dc.0,
            0,
            0,
            width,
            height,
            screen_dc.0,
            rect.left,
            rect.top,
            SRCCOPY | CAPTUREBLT,
        )
    };
    if copied == 0 {
        return Err(AppError::Windows {
            operation: "BitBlt(screen capture)",
            win32_code: last_error(),
        });
    }

    selection.restore();

    let mut bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: size_image,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [Default::default()],
    };
    let mut pixels = vec![0_u8; byte_count];
    // SAFETY: bitmap is not selected into a DC; pixels and bitmap_info describe the buffer.
    let scan_lines = unsafe {
        GetDIBits(
            memory_dc.0,
            bitmap.0,
            0,
            rect.height,
            pixels.as_mut_ptr().cast(),
            &mut bitmap_info,
            DIB_RGB_COLORS,
        )
    };
    if scan_lines != height {
        return Err(AppError::Windows {
            operation: "GetDIBits",
            win32_code: last_error(),
        });
    }

    Ok(RawBgraImage {
        width: rect.width,
        height: rect.height,
        pixels,
    })
}

struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: This guard owns a screen DC returned by GetDC(NULL).
        unsafe {
            ReleaseDC(ptr::null_mut(), self.0);
        }
    }
}

struct MemoryDc(HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: This guard owns a compatible memory DC.
        unsafe {
            DeleteDC(self.0);
        }
    }
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        // SAFETY: This guard owns a bitmap created by CreateCompatibleBitmap.
        unsafe {
            DeleteObject(self.0);
        }
    }
}

struct SelectedObject {
    dc: HDC,
    old: *mut core::ffi::c_void,
}

impl SelectedObject {
    fn restore(&mut self) {
        if !self.old.is_null() {
            // SAFETY: dc is valid and old is the object returned by SelectObject.
            unsafe {
                SelectObject(self.dc, self.old);
            }
            self.old = ptr::null_mut();
        }
    }
}

impl Drop for SelectedObject {
    fn drop(&mut self) {
        self.restore();
    }
}
