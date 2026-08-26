// Hand-rolled GDI+ FFI bindings. Kept self-contained: nothing outside this
// module touches Gdip* directly. The process-level startup token lives in
// [`GdiPlusRuntime`] (one per overlay session); each paint only creates a
// cheap graphics object bound to the paint DC through [`GdiPlusSession`].

use std::ptr;

use windows_sys::Win32::Foundation::RECT;

type GpStatus = i32;
type GpUnit = i32;

const GDIP_OK: GpStatus = 0;
const GDIP_UNIT_PIXEL: GpUnit = 2;
const GDIP_SMOOTHING_ANTIALIAS: i32 = 4;
const GDIP_PIXEL_OFFSET_HALF: i32 = 4;

#[repr(C)]
struct GdiplusStartupInput {
    gdiplus_version: u32,
    debug_event_callback: *mut core::ffi::c_void,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}

#[link(name = "gdiplus")]
unsafe extern "system" {
    fn GdiplusStartup(
        token: *mut usize,
        input: *const GdiplusStartupInput,
        output: *mut core::ffi::c_void,
    ) -> GpStatus;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateFromHDC(
        hdc: *mut core::ffi::c_void,
        graphics: *mut *mut core::ffi::c_void,
    ) -> GpStatus;
    fn GdipDeleteGraphics(graphics: *mut core::ffi::c_void) -> GpStatus;
    fn GdipSetSmoothingMode(graphics: *mut core::ffi::c_void, smoothing_mode: i32) -> GpStatus;
    fn GdipSetPixelOffsetMode(graphics: *mut core::ffi::c_void, pixel_offset_mode: i32)
    -> GpStatus;
    fn GdipCreateSolidFill(color: u32, brush: *mut *mut core::ffi::c_void) -> GpStatus;
    fn GdipDeleteBrush(brush: *mut core::ffi::c_void) -> GpStatus;
    fn GdipCreatePen1(
        color: u32,
        width: f32,
        unit: GpUnit,
        pen: *mut *mut core::ffi::c_void,
    ) -> GpStatus;
    fn GdipDeletePen(pen: *mut core::ffi::c_void) -> GpStatus;
    fn GdipCreatePath(brush_mode: i32, path: *mut *mut core::ffi::c_void) -> GpStatus;
    fn GdipDeletePath(path: *mut core::ffi::c_void) -> GpStatus;
    fn GdipAddPathArc(
        path: *mut core::ffi::c_void,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        start_angle: f32,
        sweep_angle: f32,
    ) -> GpStatus;
    fn GdipAddPathLine(
        path: *mut core::ffi::c_void,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> GpStatus;
    fn GdipClosePathFigure(path: *mut core::ffi::c_void) -> GpStatus;
    fn GdipFillPath(
        graphics: *mut core::ffi::c_void,
        brush: *mut core::ffi::c_void,
        path: *mut core::ffi::c_void,
    ) -> GpStatus;
    fn GdipDrawPath(
        graphics: *mut core::ffi::c_void,
        pen: *mut core::ffi::c_void,
        path: *mut core::ffi::c_void,
    ) -> GpStatus;
    fn GdipDrawLine(
        graphics: *mut core::ffi::c_void,
        pen: *mut core::ffi::c_void,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> GpStatus;
    fn GdipDrawEllipse(
        graphics: *mut core::ffi::c_void,
        pen: *mut core::ffi::c_void,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> GpStatus;
    fn GdipFillEllipse(
        graphics: *mut core::ffi::c_void,
        brush: *mut core::ffi::c_void,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> GpStatus;
}

/// Owns the process-wide GDI+ startup token. `GdiplusStartup`/`GdiplusShutdown`
/// are documented as once-per-process calls; tearing the library down on every
/// paint also rebuilds its background thread pool each frame.
pub(super) struct GdiPlusRuntime {
    token: usize,
}

impl GdiPlusRuntime {
    pub(super) fn start() -> Option<Self> {
        let input = GdiplusStartupInput {
            gdiplus_version: 1,
            debug_event_callback: ptr::null_mut(),
            suppress_background_thread: 0,
            suppress_external_codecs: 0,
        };
        let mut token = 0usize;
        // SAFETY: input describes version 1 and no callbacks or output are used.
        if unsafe { GdiplusStartup(&mut token, &input, ptr::null_mut()) } != GDIP_OK {
            return None;
        }
        Some(Self { token })
    }
}

impl Drop for GdiPlusRuntime {
    fn drop(&mut self) {
        // SAFETY: token came from a successful GdiplusStartup above.
        unsafe {
            GdiplusShutdown(self.token);
        }
    }
}

pub(super) struct GdiPlusSession<'runtime> {
    _runtime: &'runtime GdiPlusRuntime,
    graphics: *mut core::ffi::c_void,
}

impl<'runtime> GdiPlusSession<'runtime> {
    pub(super) unsafe fn start(
        hdc: *mut core::ffi::c_void,
        runtime: &'runtime GdiPlusRuntime,
    ) -> Option<Self> {
        let mut graphics = ptr::null_mut();
        // SAFETY: hdc is a live paint or memory DC for this call.
        if unsafe { GdipCreateFromHDC(hdc, &mut graphics) } != GDIP_OK || graphics.is_null() {
            return None;
        }
        unsafe {
            GdipSetSmoothingMode(graphics, GDIP_SMOOTHING_ANTIALIAS);
            GdipSetPixelOffsetMode(graphics, GDIP_PIXEL_OFFSET_HALF);
        }
        Some(Self {
            _runtime: runtime,
            graphics,
        })
    }

    pub(super) fn rounded_rect_rect(
        &self,
        rect: &RECT,
        radius: f32,
        fill: u32,
        stroke: u32,
        stroke_width: f32,
    ) {
        self.rounded_rect(
            rect.left as f32,
            rect.top as f32,
            (rect.right - rect.left) as f32,
            (rect.bottom - rect.top) as f32,
            radius,
            fill,
            stroke,
            stroke_width,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn rounded_rect(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        fill: u32,
        stroke: u32,
        stroke_width: f32,
    ) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        unsafe {
            let mut path = ptr::null_mut();
            if GdipCreatePath(0, &mut path) != GDIP_OK || path.is_null() {
                return;
            }
            let radius = radius.min(width / 2.0).min(height / 2.0).max(0.0);
            let diameter = radius * 2.0;
            GdipAddPathArc(path, x, y, diameter, diameter, 180.0, 90.0);
            GdipAddPathLine(path, x + radius, y, x + width - radius, y);
            GdipAddPathArc(
                path,
                x + width - diameter,
                y,
                diameter,
                diameter,
                270.0,
                90.0,
            );
            GdipAddPathLine(path, x + width, y + radius, x + width, y + height - radius);
            GdipAddPathArc(
                path,
                x + width - diameter,
                y + height - diameter,
                diameter,
                diameter,
                0.0,
                90.0,
            );
            GdipAddPathLine(path, x + width - radius, y + height, x + radius, y + height);
            GdipAddPathArc(
                path,
                x,
                y + height - diameter,
                diameter,
                diameter,
                90.0,
                90.0,
            );
            GdipAddPathLine(path, x, y + height - radius, x, y + radius);
            GdipClosePathFigure(path);
            if fill != 0 {
                if let Some(brush) = self.solid_brush(fill) {
                    GdipFillPath(self.graphics, brush, path);
                    GdipDeleteBrush(brush);
                }
            }
            if stroke != 0 && stroke_width > 0.0 {
                if let Some(pen) = self.pen(stroke, stroke_width) {
                    GdipDrawPath(self.graphics, pen, path);
                    GdipDeletePen(pen);
                }
            }
            GdipDeletePath(path);
        }
    }

    pub(super) fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, color: u32, width: f32) {
        unsafe {
            if let Some(pen) = self.pen(color, width) {
                GdipDrawLine(self.graphics, pen, x1, y1, x2, y2);
                GdipDeletePen(pen);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn ellipse(
        &self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        fill: u32,
        stroke: u32,
        sw: f32,
    ) {
        unsafe {
            if fill != 0 {
                if let Some(brush) = self.solid_brush(fill) {
                    GdipFillEllipse(self.graphics, brush, x, y, width, height);
                    GdipDeleteBrush(brush);
                }
            }
            if stroke != 0 && sw > 0.0 {
                if let Some(pen) = self.pen(stroke, sw) {
                    GdipDrawEllipse(self.graphics, pen, x, y, width, height);
                    GdipDeletePen(pen);
                }
            }
        }
    }

    unsafe fn solid_brush(&self, color: u32) -> Option<*mut core::ffi::c_void> {
        let mut brush = ptr::null_mut();
        if unsafe { GdipCreateSolidFill(color, &mut brush) } == GDIP_OK && !brush.is_null() {
            Some(brush)
        } else {
            None
        }
    }

    unsafe fn pen(&self, color: u32, width: f32) -> Option<*mut core::ffi::c_void> {
        let mut pen = ptr::null_mut();
        if unsafe { GdipCreatePen1(color, width, GDIP_UNIT_PIXEL, &mut pen) } == GDIP_OK
            && !pen.is_null()
        {
            Some(pen)
        } else {
            None
        }
    }
}

impl Drop for GdiPlusSession<'_> {
    fn drop(&mut self) {
        unsafe {
            if !self.graphics.is_null() {
                GdipDeleteGraphics(self.graphics);
            }
        }
    }
}
