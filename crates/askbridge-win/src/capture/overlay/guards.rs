// RAII guards for every Win32 resource the overlay session touches.
// All handles are used on the creating (UI) thread only.

use std::ptr;

use askbridge_core::{AppError, Result, ScreenRect};
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::{
        BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
        GetDC, HBITMAP, HDC, ReleaseDC, SRCCOPY, SelectObject,
    },
    UI::WindowsAndMessaging::DestroyWindow,
};

use crate::{
    capture::screen::{self, RawBgraImage},
    util::last_error,
};

fn snapshot_relative_rect(bounds: ScreenRect, selection: ScreenRect) -> Result<ScreenRect> {
    if selection.is_empty()
        || selection.left < bounds.left
        || selection.top < bounds.top
        || selection.right() > bounds.right()
        || selection.bottom() > bounds.bottom()
    {
        return Err(AppError::CaptureFailed(
            "selected rectangle is outside the desktop snapshot".to_owned(),
        ));
    }
    let left = i32::try_from(i64::from(selection.left) - i64::from(bounds.left)).map_err(|_| {
        AppError::CaptureFailed("snapshot selection x offset exceeds Win32 limits".to_owned())
    })?;
    let top = i32::try_from(i64::from(selection.top) - i64::from(bounds.top)).map_err(|_| {
        AppError::CaptureFailed("snapshot selection y offset exceeds Win32 limits".to_owned())
    })?;
    Ok(ScreenRect::new(
        left,
        top,
        selection.width,
        selection.height,
    ))
}

pub(super) struct OverlayWindow(pub(super) HWND);

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This guard owns the overlay window on its creating thread.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
}

pub(super) struct DesktopSnapshot {
    pub(super) bitmap: HBITMAP,
    pub(super) width: i32,
    pub(super) height: i32,
    bounds: ScreenRect,
}

impl DesktopSnapshot {
    pub(super) fn capture(bounds: ScreenRect) -> Result<Self> {
        let width = i32::try_from(bounds.width).map_err(|_| {
            AppError::CaptureFailed("desktop snapshot width exceeds Win32 limits".to_owned())
        })?;
        let height = i32::try_from(bounds.height).map_err(|_| {
            AppError::CaptureFailed("desktop snapshot height exceeds Win32 limits".to_owned())
        })?;
        if width <= 0 || height <= 0 {
            return Err(AppError::CaptureFailed(
                "desktop snapshot bounds are empty".to_owned(),
            ));
        }

        // SAFETY: A null HWND requests the virtual desktop DC.
        let screen_dc = unsafe { GetDC(ptr::null_mut()) };
        if screen_dc.is_null() {
            return Err(AppError::Windows {
                operation: "GetDC(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        let screen_dc = ScreenDc(screen_dc);

        // SAFETY: screen_dc is valid for the lifetime of this function.
        let memory_dc = unsafe { CreateCompatibleDC(screen_dc.0) };
        if memory_dc.is_null() {
            return Err(AppError::Windows {
                operation: "CreateCompatibleDC(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        let memory_dc = MemoryDc(memory_dc);

        // SAFETY: screen_dc is valid and dimensions were checked.
        let bitmap = unsafe { CreateCompatibleBitmap(screen_dc.0, width, height) };
        if bitmap.is_null() {
            return Err(AppError::Windows {
                operation: "CreateCompatibleBitmap(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        let bitmap = OwnedBitmap(bitmap);

        // SAFETY: Both handles are valid GDI objects.
        let old_bitmap = unsafe { SelectObject(memory_dc.0, bitmap.0) };
        if old_bitmap.is_null() {
            return Err(AppError::Windows {
                operation: "SelectObject(desktop snapshot)",
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
                bounds.left,
                bounds.top,
                SRCCOPY | CAPTUREBLT,
            )
        };
        if copied == 0 {
            return Err(AppError::Windows {
                operation: "BitBlt(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        selection.restore();
        Ok(Self {
            bitmap: bitmap.into_raw(),
            width,
            height,
            bounds,
        })
    }

    pub(super) fn capture_rect(&self, selection: ScreenRect) -> Result<RawBgraImage> {
        let relative = snapshot_relative_rect(self.bounds, selection)?;
        screen::capture_bitmap_rect(self.bitmap, self.width, self.height, relative)
    }
}

impl Drop for DesktopSnapshot {
    fn drop(&mut self) {
        if !self.bitmap.is_null() {
            // SAFETY: This guard owns the bitmap created for the desktop snapshot.
            unsafe {
                DeleteObject(self.bitmap);
            }
        }
    }
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

impl OwnedBitmap {
    fn into_raw(mut self) -> HBITMAP {
        let bitmap = self.0;
        self.0 = ptr::null_mut();
        bitmap
    }
}

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This guard owns a bitmap created by CreateCompatibleBitmap.
            unsafe {
                DeleteObject(self.0);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_negative_desktop_selection_to_snapshot_offset() {
        let bounds = ScreenRect::new(-1920, -200, 3840, 1280);
        let selection = ScreenRect::new(-1850, -150, 320, 240);

        assert_eq!(
            snapshot_relative_rect(bounds, selection).expect("selection is inside snapshot"),
            ScreenRect::new(70, 50, 320, 240)
        );
    }

    #[test]
    fn accepts_selection_on_snapshot_edges() {
        let bounds = ScreenRect::new(-1920, -200, 3840, 1280);

        assert_eq!(
            snapshot_relative_rect(bounds, bounds).expect("full snapshot is a valid selection"),
            ScreenRect::new(0, 0, 3840, 1280)
        );
        assert_eq!(
            snapshot_relative_rect(bounds, ScreenRect::new(1919, 1079, 1, 1))
                .expect("last pixel is inside snapshot"),
            ScreenRect::new(3839, 1279, 1, 1)
        );
    }

    #[test]
    fn rejects_selection_crossing_snapshot_boundary() {
        let bounds = ScreenRect::new(-1920, -200, 3840, 1280);

        assert!(snapshot_relative_rect(bounds, ScreenRect::new(1919, 1079, 2, 1)).is_err());
        assert!(snapshot_relative_rect(bounds, ScreenRect::new(-1921, -200, 1, 1)).is_err());
    }
}
