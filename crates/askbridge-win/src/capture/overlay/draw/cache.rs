//! GDI resources reused across paints for one overlay window.

use std::ptr;

use askbridge_core::ScreenRect;
use windows_sys::Win32::{
    Foundation::COLORREF,
    Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
        DeleteDC, DeleteObject, HBITMAP, HBRUSH, HFONT, HPEN, PS_SOLID, SelectObject,
    },
};

use crate::util::wide;

use super::super::guards::DesktopSnapshot;

/// Everything the frame needs from the session, resolved once per paint so
/// painting never has to reach back into [`OverlayState`](super::super::session::OverlayState).
pub(super) struct FrameInputs<'a> {
    pub snapshot: Option<&'a DesktopSnapshot>,
    pub selection: Option<ScreenRect>,
    pub locked: bool,
    pub fallback_toolbar: Option<&'a super::super::session::ToolbarState>,
}

/// GDI resources reused across paints for one overlay window.
///
/// All handles are created and used on the UI thread only. The struct must
/// be dropped before the session's [`DesktopSnapshot`]: the cached snapshot
/// DC keeps that bitmap selected, and a bitmap must not be deleted while a
/// DC holds it. `OverlayState` therefore declares this field first.
pub(in crate::capture::overlay) struct PaintCache {
    buffer_dc: *mut core::ffi::c_void,
    buffer_bitmap: HBITMAP,
    buffer_old_bitmap: *mut core::ffi::c_void,
    buffer_width: i32,
    buffer_height: i32,
    snapshot_dc: *mut core::ffi::c_void,
    snapshot_old_bitmap: *mut core::ffi::c_void,
    snapshot_failed: bool,
    alpha_dc: *mut core::ffi::c_void,
    alpha_bitmap: HBITMAP,
    alpha_old_bitmap: *mut core::ffi::c_void,
    alpha_failed: bool,
    font: HFONT,
    brushes: Vec<(COLORREF, HBRUSH)>,
    pens: Vec<(COLORREF, i32, HPEN)>,
}

impl Default for PaintCache {
    fn default() -> Self {
        Self {
            buffer_dc: ptr::null_mut(),
            buffer_bitmap: ptr::null_mut(),
            buffer_old_bitmap: ptr::null_mut(),
            buffer_width: 0,
            buffer_height: 0,
            snapshot_dc: ptr::null_mut(),
            snapshot_old_bitmap: ptr::null_mut(),
            snapshot_failed: false,
            alpha_dc: ptr::null_mut(),
            alpha_bitmap: ptr::null_mut(),
            alpha_old_bitmap: ptr::null_mut(),
            alpha_failed: false,
            font: ptr::null_mut(),
            brushes: Vec::new(),
            pens: Vec::new(),
        }
    }
}

impl PaintCache {
    /// Returns the double-buffer memory DC with a client-size bitmap
    /// selected, recreating the bitmap when the client size changes.
    /// Returns null when double buffering is unavailable for this paint.
    pub(super) unsafe fn ensure_buffer(
        &mut self,
        device_context: *mut core::ffi::c_void,
        width: i32,
        height: i32,
    ) -> *mut core::ffi::c_void {
        // SAFETY: device_context is a live paint DC; handles stay on the UI thread.
        unsafe {
            if width <= 0 || height <= 0 {
                return ptr::null_mut();
            }
            if self.buffer_dc.is_null() {
                let dc = CreateCompatibleDC(device_context);
                if dc.is_null() {
                    return ptr::null_mut();
                }
                self.buffer_dc = dc;
            }
            if self.buffer_width != width || self.buffer_height != height {
                if !self.buffer_bitmap.is_null() {
                    if !self.buffer_old_bitmap.is_null() {
                        SelectObject(self.buffer_dc, self.buffer_old_bitmap);
                    }
                    DeleteObject(self.buffer_bitmap);
                    self.buffer_bitmap = ptr::null_mut();
                    self.buffer_old_bitmap = ptr::null_mut();
                }
                let bitmap = CreateCompatibleBitmap(device_context, width, height);
                if bitmap.is_null() {
                    self.buffer_width = 0;
                    self.buffer_height = 0;
                    return ptr::null_mut();
                }
                let old_bitmap = SelectObject(self.buffer_dc, bitmap);
                if old_bitmap.is_null() {
                    DeleteObject(bitmap);
                    return ptr::null_mut();
                }
                self.buffer_bitmap = bitmap;
                self.buffer_old_bitmap = old_bitmap;
                self.buffer_width = width;
                self.buffer_height = height;
            }
            self.buffer_dc
        }
    }

    /// Returns a memory DC with the desktop snapshot bitmap kept selected
    /// for the lifetime of the cache. Returns null when unavailable.
    pub(super) unsafe fn ensure_snapshot_dc(
        &mut self,
        device_context: *mut core::ffi::c_void,
        snapshot: &DesktopSnapshot,
    ) -> *mut core::ffi::c_void {
        if self.snapshot_failed {
            return ptr::null_mut();
        }
        if !self.snapshot_dc.is_null() {
            return self.snapshot_dc;
        }
        // SAFETY: device_context is live and snapshot.bitmap outlives the cache.
        unsafe {
            let dc = CreateCompatibleDC(device_context);
            if dc.is_null() {
                self.snapshot_failed = true;
                return ptr::null_mut();
            }
            let old_bitmap = SelectObject(dc, snapshot.bitmap);
            if old_bitmap.is_null() {
                DeleteDC(dc);
                self.snapshot_failed = true;
                return ptr::null_mut();
            }
            self.snapshot_dc = dc;
            self.snapshot_old_bitmap = old_bitmap;
            dc
        }
    }

    /// Returns the shared label font, creating it on first use.
    pub(super) unsafe fn ensure_font(&mut self) -> HFONT {
        if self.font.is_null() {
            let face = wide("Microsoft YaHei UI");
            // SAFETY: face is nul-terminated and valid for this synchronous call.
            self.font =
                unsafe { CreateFontW(-16, 0, 0, 0, 500, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) };
        }
        self.font
    }

    pub(super) fn brush(&mut self, color: COLORREF) -> HBRUSH {
        if let Some((_, brush)) = self.brushes.iter().find(|(cached, _)| *cached == color) {
            return *brush;
        }
        // SAFETY: color is a plain value; the handle joins the cache.
        let brush = unsafe { CreateSolidBrush(color) };
        if !brush.is_null() {
            self.brushes.push((color, brush));
        }
        brush
    }

    pub(super) fn pen(&mut self, color: COLORREF, width: i32) -> HPEN {
        if let Some((_, _, pen)) = self.pens.iter().find(|(cached_color, cached_width, _)| {
            *cached_color == color && *cached_width == width
        }) {
            return *pen;
        }
        // SAFETY: arguments are plain values; the handle joins the cache.
        let pen = unsafe { CreatePen(PS_SOLID, width, color) };
        if !pen.is_null() {
            self.pens.push((color, width, pen));
        }
        pen
    }

    /// Returns the 1×1 source DC used for alpha-blended dim rectangles.
    pub(super) unsafe fn ensure_alpha_source(
        &mut self,
        device_context: *mut core::ffi::c_void,
    ) -> *mut core::ffi::c_void {
        if self.alpha_failed {
            return ptr::null_mut();
        }
        if !self.alpha_dc.is_null() {
            return self.alpha_dc;
        }
        // SAFETY: device_context is a live paint DC; handles stay on the UI thread.
        unsafe {
            let dc = CreateCompatibleDC(device_context);
            if dc.is_null() {
                self.alpha_failed = true;
                return ptr::null_mut();
            }
            let bitmap = CreateCompatibleBitmap(device_context, 1, 1);
            if bitmap.is_null() {
                DeleteDC(dc);
                self.alpha_failed = true;
                return ptr::null_mut();
            }
            let old_bitmap = SelectObject(dc, bitmap);
            if old_bitmap.is_null() {
                DeleteObject(bitmap);
                DeleteDC(dc);
                self.alpha_failed = true;
                return ptr::null_mut();
            }
            self.alpha_dc = dc;
            self.alpha_bitmap = bitmap;
            self.alpha_old_bitmap = old_bitmap;
            dc
        }
    }
}

impl Drop for PaintCache {
    fn drop(&mut self) {
        // SAFETY: Every handle here was created by this cache on the UI thread.
        unsafe {
            if !self.buffer_dc.is_null() {
                if !self.buffer_bitmap.is_null() && !self.buffer_old_bitmap.is_null() {
                    SelectObject(self.buffer_dc, self.buffer_old_bitmap);
                }
                DeleteDC(self.buffer_dc);
            }
            if !self.buffer_bitmap.is_null() {
                DeleteObject(self.buffer_bitmap);
            }
            if !self.snapshot_dc.is_null() {
                if !self.snapshot_old_bitmap.is_null() {
                    SelectObject(self.snapshot_dc, self.snapshot_old_bitmap);
                }
                DeleteDC(self.snapshot_dc);
            }
            if !self.alpha_dc.is_null() {
                if !self.alpha_old_bitmap.is_null() {
                    SelectObject(self.alpha_dc, self.alpha_old_bitmap);
                }
                DeleteDC(self.alpha_dc);
            }
            if !self.alpha_bitmap.is_null() {
                DeleteObject(self.alpha_bitmap);
            }
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
            for (_, brush) in self.brushes.drain(..) {
                if !brush.is_null() {
                    DeleteObject(brush);
                }
            }
            for (_, _, pen) in self.pens.drain(..) {
                if !pen.is_null() {
                    DeleteObject(pen);
                }
            }
        }
    }
}
