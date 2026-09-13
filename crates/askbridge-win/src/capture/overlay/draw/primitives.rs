//! Shared drawing primitives: cached fills, frames, rounded rectangles,
//! text, and the selection chrome (frame, handles, size label, instructions).

use std::ptr;

use askbridge_core::ScreenRect;
use windows_sys::Win32::{
    Foundation::{COLORREF, RECT},
    Graphics::Gdi::{
        DT_CENTER, DT_SINGLELINE, DT_VCENTER, DrawTextW, Ellipse, FillRect, FrameRect, RoundRect,
        SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
};

use crate::util::wide;

use super::super::gdiplus::GdiPlusSession;
use super::cache::PaintCache;
use super::{
    ARGB_BORDER, ARGB_LABEL, ARGB_TOOLBAR_BORDER, COLOR_BORDER, COLOR_LABEL, COLOR_TOOLBAR_BORDER,
    HANDLE_RADIUS,
};

pub(super) unsafe fn draw_text_in_rect(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    text: &str,
    color: COLORREF,
    format: u32,
    cache: &mut PaintCache,
) {
    let mut text_rect = RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    let text = wide(text);
    // SAFETY: device_context is live; the cached font outlives this call.
    unsafe {
        let font = cache.ensure_font();
        let previous_font = if font.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(device_context, font)
        };
        SetBkMode(device_context, TRANSPARENT as i32);
        SetTextColor(device_context, color);
        DrawTextW(
            device_context,
            text.as_ptr(),
            text.len().saturating_sub(1) as i32,
            &mut text_rect,
            format | DT_VCENTER | DT_SINGLELINE,
        );
        if !previous_font.is_null() {
            SelectObject(device_context, previous_font);
        }
    }
}

pub(super) unsafe fn fill(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    color: COLORREF,
    cache: &mut PaintCache,
) {
    // SAFETY: device_context and rect are valid for the current paint, and
    // the cached brush is live for the synchronous FillRect call.
    unsafe {
        let brush = cache.brush(color);
        if !brush.is_null() {
            FillRect(device_context, rect, brush);
        }
    }
}

pub(super) unsafe fn frame(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    color: COLORREF,
    cache: &mut PaintCache,
) {
    // SAFETY: device_context and rect are valid for the current paint, and
    // the cached brush is live for the synchronous FrameRect call.
    unsafe {
        let brush = cache.brush(color);
        if !brush.is_null() {
            FrameRect(device_context, rect, brush);
        }
    }
}

pub(super) unsafe fn rounded_rect(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    fill_color: COLORREF,
    border_color: COLORREF,
    radius: i32,
    cache: &mut PaintCache,
) {
    unsafe {
        let brush = cache.brush(fill_color);
        let pen = cache.pen(border_color, 1);
        let previous_brush = if brush.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(device_context, brush)
        };
        let previous_pen = if pen.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(device_context, pen)
        };
        RoundRect(
            device_context,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        if !previous_brush.is_null() {
            SelectObject(device_context, previous_brush);
        }
        if !previous_pen.is_null() {
            SelectObject(device_context, previous_pen);
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn filled_ellipse(
    device_context: *mut core::ffi::c_void,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    fill_color: COLORREF,
    border_color: COLORREF,
    cache: &mut PaintCache,
) {
    unsafe {
        let brush = cache.brush(fill_color);
        let pen = cache.pen(border_color, 1);
        let previous_brush = if brush.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(device_context, brush)
        };
        let previous_pen = if pen.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(device_context, pen)
        };
        Ellipse(device_context, left, top, right, bottom);
        if !previous_brush.is_null() {
            SelectObject(device_context, previous_brush);
        }
        if !previous_pen.is_null() {
            SelectObject(device_context, previous_pen);
        }
    }
}

pub(super) unsafe fn draw_selection_handles(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    gdi: Option<&GdiPlusSession>,
    cache: &mut PaintCache,
) {
    let points = super::super::layout::selection_handle_points(rect);
    if let Some(gdi) = gdi {
        draw_selection_handles_antialiased(gdi, rect);
        return;
    }
    for (x, y) in points {
        unsafe {
            filled_ellipse(
                device_context,
                x - HANDLE_RADIUS,
                y - HANDLE_RADIUS,
                x + HANDLE_RADIUS,
                y + HANDLE_RADIUS,
                COLOR_BORDER,
                COLOR_TOOLBAR_BORDER,
                cache,
            );
        }
    }
}

pub(super) fn draw_selection_frame_antialiased(gdi: &GdiPlusSession, rect: &RECT) {
    let left = rect.left as f32 + 0.5;
    let top = rect.top as f32 + 0.5;
    let right = rect.right as f32 - 0.5;
    let bottom = rect.bottom as f32 - 0.5;
    gdi.line(left, top, right, top, ARGB_BORDER, 1.2);
    gdi.line(right, top, right, bottom, ARGB_BORDER, 1.2);
    gdi.line(right, bottom, left, bottom, ARGB_BORDER, 1.2);
    gdi.line(left, bottom, left, top, ARGB_BORDER, 1.2);
}

pub(super) fn draw_selection_handles_antialiased(gdi: &GdiPlusSession, rect: &RECT) {
    for (x, y) in super::super::layout::selection_handle_points(rect) {
        gdi.ellipse(
            (x - HANDLE_RADIUS) as f32,
            (y - HANDLE_RADIUS) as f32,
            (HANDLE_RADIUS * 2) as f32,
            (HANDLE_RADIUS * 2) as f32,
            ARGB_BORDER,
            ARGB_TOOLBAR_BORDER,
            1.0,
        );
    }
}

pub(super) unsafe fn draw_size_label(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    selection_rect: &RECT,
    selection: ScreenRect,
    cache: &mut PaintCache,
    gdi: Option<&GdiPlusSession>,
) {
    let label_width = 132;
    let label_height = 30;
    let left = selection_rect
        .left
        .clamp(client.left, (client.right - label_width).max(client.left));
    let top = if selection_rect.top - label_height - 6 >= client.top {
        selection_rect.top - label_height - 6
    } else {
        (selection_rect.bottom + 6).min(client.bottom - label_height)
    };
    let mut label_rect = RECT {
        left,
        top,
        right: left + label_width,
        bottom: top + label_height,
    };
    // SAFETY: device_context and label_rect are valid for the current paint.
    unsafe {
        if let Some(gdi) = gdi {
            gdi.rounded_rect_rect(&label_rect, 5.0, ARGB_LABEL, ARGB_LABEL, 1.0);
        } else {
            fill(device_context, &label_rect, COLOR_LABEL, cache);
        }
        SetBkMode(device_context, TRANSPARENT as i32);
        SetTextColor(device_context, COLOR_BORDER);
    }
    let text = wide(&format!("{} × {}", selection.width, selection.height));
    // SAFETY: text is nul-terminated and the explicit length excludes the terminator.
    unsafe {
        DrawTextW(
            device_context,
            text.as_ptr(),
            text.len().saturating_sub(1) as i32,
            &mut label_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

pub(super) unsafe fn draw_instructions(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    cache: &mut PaintCache,
) {
    let width = 300.min(client.right - client.left);
    let mut rect = RECT {
        left: client.left + (client.right - client.left - width) / 2,
        top: client.top + 24,
        right: client.left + (client.right - client.left + width) / 2,
        bottom: client.top + 60,
    };
    // SAFETY: device_context and rect are valid for the current paint.
    unsafe {
        fill(device_context, &rect, COLOR_LABEL, cache);
        SetBkMode(device_context, TRANSPARENT as i32);
        SetTextColor(device_context, COLOR_BORDER);
    }
    let text = wide("拖动选择区域 · Esc / 右键取消");
    // SAFETY: text is nul-terminated and the explicit length excludes the terminator.
    unsafe {
        DrawTextW(
            device_context,
            text.as_ptr(),
            text.len().saturating_sub(1) as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
}
