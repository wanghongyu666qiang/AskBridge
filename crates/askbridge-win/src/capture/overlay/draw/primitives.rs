//! Shared drawing primitives: cached fills, frames, rounded rectangles,
//! text, and the selection chrome (frame, handles, size label, instructions).

use std::ptr;

use askbridge_core::ScreenRect;
use windows_sys::Win32::{
    Foundation::{COLORREF, RECT},
    Graphics::Gdi::{
        DT_CENTER, DT_SINGLELINE, DT_VCENTER, DrawTextW, FillRect, FrameRect, RoundRect,
        SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
};

use crate::util::wide;

use super::super::gdiplus::GdiPlusSession;
use super::cache::PaintCache;
use super::{
    ARGB_ACCENT_BRIGHT, ARGB_INNER_HAIRLINE, ARGB_LABEL, ARGB_LABEL_ACCENT, BRACKET_ARM,
    BRACKET_GLOW, BRACKET_STROKE, COLOR_LABEL, COLOR_LABEL_BORDER, COLOR_LABEL_TEXT,
    SELECTION_SHADOW,
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

/// Draws the brand selection frame: four corner brackets with a soft glow.
///
/// The four-corner bracket is the AskBridge mark itself (see
/// `assets/branding/askbridge-final/docs/BRAND_GUIDELINES.md`), so the
/// selection reads as "this is what AskBridge will capture" rather than as a
/// generic marquee.
pub(super) fn draw_selection_brackets(gdi: &GdiPlusSession, rect: &RECT, pulse: f32) {
    let left = rect.left as f32 + 1.0;
    let top = rect.top as f32 + 1.0;
    let right = rect.right as f32 - 1.0;
    let bottom = rect.bottom as f32 - 1.0;
    // The brackets must not overlap on a small selection.
    let arm = BRACKET_ARM as f32;
    let horizontal_arm = arm.min(((right - left) / 2.0).max(0.0));
    let vertical_arm = arm.min(((bottom - top) / 2.0).max(0.0));
    if horizontal_arm <= 0.0 || vertical_arm <= 0.0 {
        return;
    }

    // Lift the selection off the page first, so the glow and brackets sit on
    // top of the shadow rather than inside it.
    for (outset, width, alpha) in SELECTION_SHADOW {
        gdi.rounded_rect(
            left - outset,
            top - outset,
            (right - left) + outset * 2.0,
            (bottom - top) + outset * 2.0,
            3.0,
            0,
            argb_alpha(0x0000_0000, alpha),
            width,
        );
    }
    // A hairline inside the edge keeps the frozen content distinct from the
    // dimmed backdrop on light pages.
    gdi.rounded_rect(
        left + 0.5,
        top + 0.5,
        (right - left) - 1.0,
        (bottom - top) - 1.0,
        2.0,
        0,
        ARGB_INNER_HAIRLINE,
        1.0,
    );

    // Glow next, from the outside in, so the solid core stays crisp. The
    // pulse keeps a locked selection feeling alive without ever moving.
    for (outset, width, alpha) in BRACKET_GLOW {
        let color = argb_alpha(ARGB_ACCENT_BRIGHT, scale_alpha(alpha, pulse));
        draw_brackets(
            gdi,
            left - outset,
            top - outset,
            right + outset,
            bottom + outset,
            horizontal_arm + outset,
            vertical_arm + outset,
            color,
            width,
        );
    }
    draw_brackets(
        gdi,
        left,
        top,
        right,
        bottom,
        horizontal_arm,
        vertical_arm,
        ARGB_ACCENT_BRIGHT,
        BRACKET_STROKE,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_brackets(
    gdi: &GdiPlusSession,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    horizontal_arm: f32,
    vertical_arm: f32,
    color: u32,
    width: f32,
) {
    // Top-left
    gdi.line(left, top, left + horizontal_arm, top, color, width);
    gdi.line(left, top, left, top + vertical_arm, color, width);
    // Top-right
    gdi.line(right, top, right - horizontal_arm, top, color, width);
    gdi.line(right, top, right, top + vertical_arm, color, width);
    // Bottom-right
    gdi.line(right, bottom, right - horizontal_arm, bottom, color, width);
    gdi.line(right, bottom, right, bottom - vertical_arm, color, width);
    // Bottom-left
    gdi.line(left, bottom, left + horizontal_arm, bottom, color, width);
    gdi.line(left, bottom, left, bottom - vertical_arm, color, width);
}

/// Replaces the alpha channel of an ARGB color.
const fn argb_alpha(color: u32, alpha: u8) -> u32 {
    (color & 0x00FF_FFFF) | ((alpha as u32) << 24)
}

/// Scales a glow alpha by the breathing factor, clamped to a valid byte.
fn scale_alpha(alpha: u8, pulse: f32) -> u8 {
    (f32::from(alpha) * pulse.clamp(0.0, 2.0)).clamp(0.0, 255.0) as u8
}

pub(super) unsafe fn draw_size_label(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    selection_rect: &RECT,
    selection: ScreenRect,
    cache: &mut PaintCache,
    gdi: Option<&GdiPlusSession>,
) {
    // A compact pill that hugs the number instead of a wide fixed plate: it
    // reads as a measurement readout rather than as a dialog.
    let label_height = 28;
    let label_width = 116;
    let gap = 8;
    let left = selection_rect
        .left
        .clamp(client.left, (client.right - label_width).max(client.left));
    let top = if selection_rect.top - label_height - gap >= client.top {
        selection_rect.top - label_height - gap
    } else {
        (selection_rect.bottom + gap).min(client.bottom - label_height)
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
            // Drop shadow, then the dark plate with an accent hairline, so the
            // readout separates from both light and dark desktop content.
            gdi.rounded_rect(
                label_rect.left as f32 - 1.0,
                label_rect.top as f32,
                label_width as f32 + 2.0,
                label_height as f32 + 2.0,
                10.0,
                0,
                argb_alpha(0x0000_0000, 60),
                5.0,
            );
            gdi.rounded_rect_rect(&label_rect, 9.0, ARGB_LABEL, ARGB_LABEL_ACCENT, 1.2);
        } else {
            rounded_rect(
                device_context,
                &label_rect,
                COLOR_LABEL,
                COLOR_LABEL_BORDER,
                18,
                cache,
            );
        }
        SetBkMode(device_context, TRANSPARENT as i32);
        SetTextColor(device_context, COLOR_LABEL_TEXT);
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
    gdi: Option<&GdiPlusSession>,
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
        if let Some(gdi) = gdi {
            // Same treatment as the measurement readout: a lifted dark pill
            // with an accent hairline, so both overlays share one look.
            gdi.rounded_rect(
                rect.left as f32 - 1.0,
                rect.top as f32,
                (rect.right - rect.left) as f32 + 2.0,
                (rect.bottom - rect.top) as f32 + 3.0,
                20.0,
                0,
                argb_alpha(0x0000_0000, 56),
                6.0,
            );
            gdi.rounded_rect_rect(&rect, 18.0, ARGB_LABEL, ARGB_LABEL_ACCENT, 1.0);
        } else {
            rounded_rect(
                device_context,
                &rect,
                COLOR_LABEL,
                COLOR_LABEL_BORDER,
                36,
                cache,
            );
        }
        SetBkMode(device_context, TRANSPARENT as i32);
        SetTextColor(device_context, COLOR_LABEL_TEXT);
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
