// Overlay painting: double-buffered frame composition, toolbar drawing
// (GDI+ with a plain GDI fallback), and the shared color palette.

use std::{mem::zeroed, ptr};

use askbridge_core::ScreenRect;
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, RECT},
    Graphics::Gdi::{
        AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, BeginPaint, BitBlt, CreateCompatibleBitmap,
        CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DT_CENTER, DT_LEFT,
        DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint, FillRect,
        FrameRect, LineTo, MoveToEx, PAINTSTRUCT, PS_SOLID, Rectangle, RoundRect, SRCCOPY,
        SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::WindowsAndMessaging::GetClientRect,
};

use crate::util::wide;

use super::{
    gdiplus::GdiPlusSession,
    guards::DesktopSnapshot,
    layout::{fallback_toolbar_size, inset_rect, selection_handle_points, toolbar_layout},
    session::{OverlayState, ToolbarState},
};

pub(super) const COLOR_KEY: COLORREF = rgb(255, 0, 255);
const COLOR_OVERLAY: COLORREF = rgb(0, 0, 0);
const COLOR_BORDER: COLORREF = rgb(255, 255, 255);
const COLOR_LABEL: COLORREF = rgb(15, 23, 42);
const COLOR_TOOLBAR: COLORREF = rgb(248, 250, 252);
const COLOR_TOOLBAR_BORDER: COLORREF = rgb(203, 213, 225);
const COLOR_TOOLBAR_TEXT: COLORREF = rgb(15, 23, 42);
const COLOR_TOOLBAR_HOVER: COLORREF = rgb(241, 245, 249);
const COLOR_DROPDOWN_SELECTED: COLORREF = rgb(229, 231, 235);
const ARGB_BORDER: u32 = argb(255, 255, 255, 255);
const ARGB_LABEL: u32 = argb(255, 15, 23, 42);
const ARGB_TOOLBAR: u32 = argb(255, 248, 250, 252);
const ARGB_TOOLBAR_BORDER: u32 = argb(255, 203, 213, 225);
const ARGB_TOOLBAR_TEXT: u32 = argb(255, 15, 23, 42);
const ARGB_TOOLBAR_HOVER: u32 = argb(255, 241, 245, 249);
const ARGB_DROPDOWN_SELECTED: u32 = argb(255, 229, 231, 235);
pub(super) const OVERLAY_ALPHA: u8 = 145;
const TOOLBAR_RADIUS: i32 = 18;
const HANDLE_RADIUS: i32 = 5;

pub(super) fn paint_overlay(window: HWND, state: &OverlayState) {
    // SAFETY: window is handling WM_PAINT and paint remains valid through EndPaint.
    unsafe {
        let mut paint: PAINTSTRUCT = zeroed();
        let device_context = BeginPaint(window, &mut paint);
        if device_context.is_null() {
            return;
        }
        let mut client = RECT::default();
        GetClientRect(window, &mut client);
        let width = client.right - client.left;
        let height = client.bottom - client.top;
        let memory_context = CreateCompatibleDC(device_context);
        let bitmap = if memory_context.is_null() || width <= 0 || height <= 0 {
            ptr::null_mut()
        } else {
            CreateCompatibleBitmap(device_context, width, height)
        };
        if memory_context.is_null() || bitmap.is_null() {
            if !memory_context.is_null() {
                DeleteDC(memory_context);
            }
            draw_overlay_frame(device_context, &client, state);
            EndPaint(window, &paint);
            return;
        }

        let previous_bitmap = SelectObject(memory_context, bitmap);
        if previous_bitmap.is_null() {
            DeleteObject(bitmap);
            DeleteDC(memory_context);
            draw_overlay_frame(device_context, &client, state);
            EndPaint(window, &paint);
            return;
        }

        draw_overlay_frame(memory_context, &client, state);
        BitBlt(
            device_context,
            client.left,
            client.top,
            width,
            height,
            memory_context,
            0,
            0,
            SRCCOPY,
        );
        SelectObject(memory_context, previous_bitmap);
        DeleteObject(bitmap);
        DeleteDC(memory_context);
        EndPaint(window, &paint);
    }
}

unsafe fn draw_overlay_frame(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    state: &OverlayState,
) {
    // SAFETY: device_context is a live paint or compatible memory DC.
    unsafe {
        let has_snapshot = draw_desktop_snapshot(device_context, state.desktop_snapshot.as_ref());
        if has_snapshot {
            dim_selection_backdrop(device_context, client, state.selection());
        } else {
            fill(device_context, client, COLOR_OVERLAY);
        }

        if let Some(selection) = state.selection() {
            let selection_rect = RECT {
                left: selection.left,
                top: selection.top,
                right: selection.right() as i32,
                bottom: selection.bottom() as i32,
            };
            if !has_snapshot {
                fill(device_context, &selection_rect, COLOR_KEY);
            }
            if has_snapshot {
                if let Some(gdi) = GdiPlusSession::start(device_context) {
                    draw_selection_frame_antialiased(&gdi, &selection_rect);
                    draw_selection_handles_antialiased(&gdi, &selection_rect);
                } else {
                    frame(device_context, &selection_rect, COLOR_BORDER);
                    draw_selection_handles(device_context, &selection_rect);
                }
            } else {
                frame(device_context, &selection_rect, COLOR_BORDER);
                draw_selection_handles(device_context, &selection_rect);
            }
            draw_size_label(device_context, client, &selection_rect, selection);
            if state.locked_selection.is_some()
                && let Some(toolbar) = &state.toolbar
                && state.web_toolbar.is_none()
                && state.web_toolbar_failed
            {
                draw_toolbar(device_context, client, &selection_rect, toolbar);
            }
        }
        if state.locked_selection.is_none() {
            draw_instructions(device_context, client);
        }
    }
}

unsafe fn draw_toolbar(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    selection_rect: &RECT,
    toolbar: &ToolbarState,
) {
    let layout = toolbar_layout(
        client,
        selection_rect,
        toolbar.providers.len(),
        fallback_toolbar_size(),
    );
    unsafe {
        if let Some(gdi) = GdiPlusSession::start(device_context) {
            draw_toolbar_antialiased(&gdi, &layout, toolbar);
            drop(gdi);
            draw_toolbar_labels(device_context, &layout, toolbar);
            return;
        }
        rounded_rect(
            device_context,
            &layout.outer,
            COLOR_TOOLBAR,
            COLOR_TOOLBAR_BORDER,
            TOOLBAR_RADIUS,
        );
        draw_toolbar_item(
            device_context,
            &layout.more,
            "更多",
            ToolbarIcon::More,
            false,
        );
        draw_toolbar_item(
            device_context,
            &layout.provider,
            &toolbar.providers[toolbar.selected_index].display_name,
            ToolbarIcon::Provider,
            true,
        );
        draw_toolbar_item(
            device_context,
            &layout.copy,
            "复制",
            ToolbarIcon::Copy,
            false,
        );
        draw_toolbar_item(
            device_context,
            &layout.cancel,
            "取消",
            ToolbarIcon::Cancel,
            false,
        );
        draw_toolbar_item(
            device_context,
            &layout.ask,
            &format!(
                "问问 {}",
                toolbar.providers[toolbar.selected_index].display_name
            ),
            ToolbarIcon::Ask,
            false,
        );
        if toolbar.dropdown_open {
            rounded_rect(
                device_context,
                &layout.dropdown_bounds,
                COLOR_TOOLBAR,
                COLOR_TOOLBAR_BORDER,
                12,
            );
            for (index, rect) in layout.dropdown_rects.iter().enumerate() {
                if index == toolbar.selected_index {
                    rounded_rect(
                        device_context,
                        &inset_rect(rect, 4),
                        COLOR_DROPDOWN_SELECTED,
                        COLOR_DROPDOWN_SELECTED,
                        8,
                    );
                }
                draw_text_in_rect(
                    device_context,
                    rect,
                    &toolbar.providers[index].display_name,
                    COLOR_TOOLBAR_TEXT,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
    }
}

fn draw_toolbar_antialiased(
    gdi: &GdiPlusSession,
    layout: &super::layout::ToolbarLayout,
    toolbar: &ToolbarState,
) {
    gdi.rounded_rect_rect(&layout.outer, 18.0, ARGB_TOOLBAR, ARGB_TOOLBAR_BORDER, 1.0);
    gdi.rounded_rect_rect(
        &layout.provider,
        10.0,
        ARGB_TOOLBAR_HOVER,
        ARGB_TOOLBAR_HOVER,
        1.0,
    );
    draw_toolbar_icon_antialiased(gdi, &layout.more, ToolbarIcon::More);
    draw_toolbar_icon_antialiased(gdi, &layout.provider, ToolbarIcon::Provider);
    draw_toolbar_icon_antialiased(gdi, &layout.copy, ToolbarIcon::Copy);
    draw_toolbar_icon_antialiased(gdi, &layout.cancel, ToolbarIcon::Cancel);
    draw_toolbar_icon_antialiased(gdi, &layout.ask, ToolbarIcon::Ask);
    if toolbar.dropdown_open {
        gdi.rounded_rect_rect(
            &layout.dropdown_bounds,
            12.0,
            ARGB_TOOLBAR,
            ARGB_TOOLBAR_BORDER,
            1.0,
        );
        for (index, rect) in layout.dropdown_rects.iter().enumerate() {
            if index == toolbar.selected_index {
                gdi.rounded_rect_rect(
                    &inset_rect(rect, 4),
                    8.0,
                    ARGB_DROPDOWN_SELECTED,
                    ARGB_DROPDOWN_SELECTED,
                    1.0,
                );
            }
        }
    }
}

unsafe fn draw_toolbar_labels(
    device_context: *mut core::ffi::c_void,
    layout: &super::layout::ToolbarLayout,
    toolbar: &ToolbarState,
) {
    unsafe {
        draw_toolbar_label(device_context, &layout.more, "更多");
        draw_toolbar_label(
            device_context,
            &layout.provider,
            &toolbar.providers[toolbar.selected_index].display_name,
        );
        draw_toolbar_label(device_context, &layout.copy, "复制");
        draw_toolbar_label(device_context, &layout.cancel, "取消");
        draw_toolbar_label(
            device_context,
            &layout.ask,
            &format!(
                "问问 {}",
                toolbar.providers[toolbar.selected_index].display_name
            ),
        );
        if toolbar.dropdown_open {
            for (index, rect) in layout.dropdown_rects.iter().enumerate() {
                draw_text_in_rect(
                    device_context,
                    rect,
                    &toolbar.providers[index].display_name,
                    COLOR_TOOLBAR_TEXT,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
    }
}

unsafe fn draw_toolbar_label(device_context: *mut core::ffi::c_void, rect: &RECT, label: &str) {
    let text_rect = RECT {
        left: rect.left + 34,
        top: rect.top,
        right: rect.right - 12,
        bottom: rect.bottom,
    };
    unsafe {
        draw_text_in_rect(
            device_context,
            &text_rect,
            label,
            COLOR_TOOLBAR_TEXT,
            DT_LEFT,
        );
    }
}

unsafe fn draw_desktop_snapshot(
    device_context: *mut core::ffi::c_void,
    snapshot: Option<&DesktopSnapshot>,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    if snapshot.bitmap.is_null() || snapshot.width <= 0 || snapshot.height <= 0 {
        return false;
    }

    unsafe {
        let snapshot_context = CreateCompatibleDC(device_context);
        if snapshot_context.is_null() {
            return false;
        }
        let old_bitmap = SelectObject(snapshot_context, snapshot.bitmap);
        if old_bitmap.is_null() {
            DeleteDC(snapshot_context);
            return false;
        }
        let copied = BitBlt(
            device_context,
            0,
            0,
            snapshot.width,
            snapshot.height,
            snapshot_context,
            0,
            0,
            SRCCOPY,
        ) != 0;
        SelectObject(snapshot_context, old_bitmap);
        DeleteDC(snapshot_context);
        copied
    }
}

unsafe fn dim_selection_backdrop(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    selection: Option<ScreenRect>,
) {
    let Some(selection) = selection else {
        unsafe {
            alpha_fill(device_context, client, COLOR_OVERLAY, OVERLAY_ALPHA);
        }
        return;
    };
    let selection_rect = RECT {
        left: selection.left.clamp(client.left, client.right),
        top: selection.top.clamp(client.top, client.bottom),
        right: (selection.right() as i32).clamp(client.left, client.right),
        bottom: (selection.bottom() as i32).clamp(client.top, client.bottom),
    };
    for rect in [
        RECT {
            left: client.left,
            top: client.top,
            right: client.right,
            bottom: selection_rect.top,
        },
        RECT {
            left: client.left,
            top: selection_rect.bottom,
            right: client.right,
            bottom: client.bottom,
        },
        RECT {
            left: client.left,
            top: selection_rect.top,
            right: selection_rect.left,
            bottom: selection_rect.bottom,
        },
        RECT {
            left: selection_rect.right,
            top: selection_rect.top,
            right: client.right,
            bottom: selection_rect.bottom,
        },
    ] {
        unsafe {
            alpha_fill(device_context, &rect, COLOR_OVERLAY, OVERLAY_ALPHA);
        }
    }
}

unsafe fn alpha_fill(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    color: COLORREF,
    alpha: u8,
) {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }
    unsafe {
        let source_context = CreateCompatibleDC(device_context);
        if source_context.is_null() {
            fill(device_context, rect, color);
            return;
        }
        let source_bitmap = CreateCompatibleBitmap(device_context, 1, 1);
        if source_bitmap.is_null() {
            DeleteDC(source_context);
            fill(device_context, rect, color);
            return;
        }
        let old_bitmap = SelectObject(source_context, source_bitmap);
        let source_rect = RECT {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        fill(source_context, &source_rect, color);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: 0,
        };
        let blended = AlphaBlend(
            device_context,
            rect.left,
            rect.top,
            width,
            height,
            source_context,
            0,
            0,
            1,
            1,
            blend,
        );
        if blended == 0 {
            fill(device_context, rect, color);
        }
        if !old_bitmap.is_null() {
            SelectObject(source_context, old_bitmap);
        }
        DeleteObject(source_bitmap);
        DeleteDC(source_context);
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolbarIcon {
    More,
    Provider,
    Copy,
    Cancel,
    Ask,
}

unsafe fn draw_toolbar_item(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    label: &str,
    icon: ToolbarIcon,
    highlighted: bool,
) {
    unsafe {
        if highlighted {
            rounded_rect(
                device_context,
                rect,
                COLOR_TOOLBAR_HOVER,
                COLOR_TOOLBAR_HOVER,
                10,
            );
        }
        draw_toolbar_icon(device_context, rect, icon, COLOR_TOOLBAR_TEXT);
        let text_rect = RECT {
            left: rect.left + 34,
            top: rect.top,
            right: rect.right - 12,
            bottom: rect.bottom,
        };
        draw_text_in_rect(
            device_context,
            &text_rect,
            label,
            COLOR_TOOLBAR_TEXT,
            DT_LEFT,
        );
    }
}

unsafe fn draw_text_in_rect(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    text: &str,
    color: COLORREF,
    format: u32,
) {
    let mut text_rect = RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    let text = wide(text);
    let face = wide("Microsoft YaHei UI");
    unsafe {
        let font = CreateFontW(-16, 0, 0, 0, 600, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
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
        if !font.is_null() {
            DeleteObject(font);
        }
    }
}

unsafe fn draw_toolbar_icon(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    icon: ToolbarIcon,
    color: COLORREF,
) {
    let cx = rect.left + 18;
    let cy = rect.top + (rect.bottom - rect.top) / 2;
    unsafe {
        let pen = CreatePen(PS_SOLID, 2, color);
        let previous_pen = if pen.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(device_context, pen)
        };
        match icon {
            ToolbarIcon::More => {
                for (x, y) in [(cx - 7, cy - 7), (cx + 1, cy - 7), (cx - 7, cy + 1)] {
                    Rectangle(device_context, x, y, x + 5, y + 5);
                }
                MoveToEx(device_context, cx + 4, cy + 5, ptr::null_mut());
                LineTo(device_context, cx + 8, cy + 5);
                MoveToEx(device_context, cx + 6, cy + 3, ptr::null_mut());
                LineTo(device_context, cx + 6, cy + 7);
            }
            ToolbarIcon::Provider => {
                Rectangle(device_context, cx - 7, cy - 7, cx + 7, cy + 7);
                MoveToEx(device_context, cx - 4, cy - 2, ptr::null_mut());
                LineTo(device_context, cx + 4, cy - 2);
                MoveToEx(device_context, cx - 4, cy + 3, ptr::null_mut());
                LineTo(device_context, cx + 4, cy + 3);
            }
            ToolbarIcon::Copy => {
                Rectangle(device_context, cx - 6, cy - 7, cx + 5, cy + 4);
                Rectangle(device_context, cx - 2, cy - 3, cx + 9, cy + 8);
            }
            ToolbarIcon::Cancel => {
                MoveToEx(device_context, cx - 6, cy - 6, ptr::null_mut());
                LineTo(device_context, cx + 6, cy + 6);
                MoveToEx(device_context, cx + 6, cy - 6, ptr::null_mut());
                LineTo(device_context, cx - 6, cy + 6);
            }
            ToolbarIcon::Ask => {
                RoundRect(device_context, cx - 8, cy - 7, cx + 8, cy + 5, 4, 4);
                MoveToEx(device_context, cx - 2, cy + 5, ptr::null_mut());
                LineTo(device_context, cx - 5, cy + 9);
                MoveToEx(device_context, cx - 3, cy - 1, ptr::null_mut());
                LineTo(device_context, cx + 3, cy - 1);
            }
        }
        if !previous_pen.is_null() {
            SelectObject(device_context, previous_pen);
        }
        if !pen.is_null() {
            DeleteObject(pen);
        }
    }
}

fn draw_toolbar_icon_antialiased(gdi: &GdiPlusSession, rect: &RECT, icon: ToolbarIcon) {
    let cx = rect.left as f32 + 18.0;
    let cy = rect.top as f32 + (rect.bottom - rect.top) as f32 / 2.0;
    match icon {
        ToolbarIcon::More => {
            for (x, y) in [
                (cx - 7.0, cy - 7.0),
                (cx + 1.0, cy - 7.0),
                (cx - 7.0, cy + 1.0),
            ] {
                gdi.rounded_rect(x, y, 5.0, 5.0, 1.4, 0, ARGB_TOOLBAR_TEXT, 1.6);
            }
            gdi.line(
                cx + 4.0,
                cy + 5.0,
                cx + 8.0,
                cy + 5.0,
                ARGB_TOOLBAR_TEXT,
                1.8,
            );
            gdi.line(
                cx + 6.0,
                cy + 3.0,
                cx + 6.0,
                cy + 7.0,
                ARGB_TOOLBAR_TEXT,
                1.8,
            );
        }
        ToolbarIcon::Provider => {
            gdi.rounded_rect(
                cx - 7.0,
                cy - 7.0,
                14.0,
                14.0,
                2.2,
                0,
                ARGB_TOOLBAR_TEXT,
                1.7,
            );
            gdi.line(
                cx - 4.0,
                cy - 2.0,
                cx + 4.0,
                cy - 2.0,
                ARGB_TOOLBAR_TEXT,
                1.7,
            );
            gdi.line(
                cx - 4.0,
                cy + 3.0,
                cx + 4.0,
                cy + 3.0,
                ARGB_TOOLBAR_TEXT,
                1.7,
            );
        }
        ToolbarIcon::Copy => {
            gdi.rounded_rect(
                cx - 6.0,
                cy - 7.0,
                11.0,
                11.0,
                1.8,
                0,
                ARGB_TOOLBAR_TEXT,
                1.7,
            );
            gdi.rounded_rect(
                cx - 2.0,
                cy - 3.0,
                11.0,
                11.0,
                1.8,
                0,
                ARGB_TOOLBAR_TEXT,
                1.7,
            );
        }
        ToolbarIcon::Cancel => {
            gdi.line(
                cx - 6.0,
                cy - 6.0,
                cx + 6.0,
                cy + 6.0,
                ARGB_TOOLBAR_TEXT,
                1.9,
            );
            gdi.line(
                cx + 6.0,
                cy - 6.0,
                cx - 6.0,
                cy + 6.0,
                ARGB_TOOLBAR_TEXT,
                1.9,
            );
        }
        ToolbarIcon::Ask => {
            gdi.rounded_rect(
                cx - 8.0,
                cy - 7.0,
                16.0,
                12.0,
                3.2,
                0,
                ARGB_TOOLBAR_TEXT,
                1.6,
            );
            gdi.line(
                cx - 2.0,
                cy + 5.0,
                cx - 5.0,
                cy + 9.0,
                ARGB_TOOLBAR_TEXT,
                1.6,
            );
            gdi.line(
                cx - 3.0,
                cy - 1.0,
                cx + 3.0,
                cy - 1.0,
                ARGB_TOOLBAR_TEXT,
                1.6,
            );
        }
    }
}

unsafe fn fill(device_context: *mut core::ffi::c_void, rect: &RECT, color: COLORREF) {
    // SAFETY: device_context and rect are valid for the current paint.
    let brush = unsafe { CreateSolidBrush(color) };
    if !brush.is_null() {
        // SAFETY: brush is live for the synchronous FillRect call.
        unsafe {
            FillRect(device_context, rect, brush);
            DeleteObject(brush);
        }
    }
}

unsafe fn frame(device_context: *mut core::ffi::c_void, rect: &RECT, color: COLORREF) {
    // SAFETY: device_context and rect are valid for the current paint.
    let brush = unsafe { CreateSolidBrush(color) };
    if !brush.is_null() {
        // SAFETY: brush is live for the synchronous FrameRect call.
        unsafe {
            FrameRect(device_context, rect, brush);
            DeleteObject(brush);
        }
    }
}

unsafe fn rounded_rect(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    fill_color: COLORREF,
    border_color: COLORREF,
    radius: i32,
) {
    unsafe {
        let brush = CreateSolidBrush(fill_color);
        let pen = CreatePen(PS_SOLID, 1, border_color);
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
        if !brush.is_null() {
            DeleteObject(brush);
        }
        if !pen.is_null() {
            DeleteObject(pen);
        }
    }
}

unsafe fn draw_selection_handles(device_context: *mut core::ffi::c_void, rect: &RECT) {
    let points = selection_handle_points(rect);
    if let Some(gdi) = unsafe { GdiPlusSession::start(device_context) } {
        draw_selection_handles_antialiased(&gdi, rect);
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
            );
        }
    }
}

fn draw_selection_frame_antialiased(gdi: &GdiPlusSession, rect: &RECT) {
    let left = rect.left as f32 + 0.5;
    let top = rect.top as f32 + 0.5;
    let right = rect.right as f32 - 0.5;
    let bottom = rect.bottom as f32 - 0.5;
    gdi.line(left, top, right, top, ARGB_BORDER, 1.2);
    gdi.line(right, top, right, bottom, ARGB_BORDER, 1.2);
    gdi.line(right, bottom, left, bottom, ARGB_BORDER, 1.2);
    gdi.line(left, bottom, left, top, ARGB_BORDER, 1.2);
}

fn draw_selection_handles_antialiased(gdi: &GdiPlusSession, rect: &RECT) {
    for (x, y) in selection_handle_points(rect) {
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

unsafe fn filled_ellipse(
    device_context: *mut core::ffi::c_void,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    fill_color: COLORREF,
    border_color: COLORREF,
) {
    unsafe {
        let brush = CreateSolidBrush(fill_color);
        let pen = CreatePen(PS_SOLID, 1, border_color);
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
        if !brush.is_null() {
            DeleteObject(brush);
        }
        if !pen.is_null() {
            DeleteObject(pen);
        }
    }
}

unsafe fn draw_size_label(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    selection_rect: &RECT,
    selection: ScreenRect,
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
        if let Some(gdi) = GdiPlusSession::start(device_context) {
            gdi.rounded_rect_rect(&label_rect, 5.0, ARGB_LABEL, ARGB_LABEL, 1.0);
        } else {
            fill(device_context, &label_rect, COLOR_LABEL);
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

unsafe fn draw_instructions(device_context: *mut core::ffi::c_void, client: &RECT) {
    let width = 300.min(client.right - client.left);
    let mut rect = RECT {
        left: client.left + (client.right - client.left - width) / 2,
        top: client.top + 24,
        right: client.left + (client.right - client.left + width) / 2,
        bottom: client.top + 60,
    };
    // SAFETY: device_context and rect are valid for the current paint.
    unsafe {
        fill(device_context, &rect, COLOR_LABEL);
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

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

const fn argb(alpha: u8, red: u8, green: u8, blue: u8) -> u32 {
    ((alpha as u32) << 24) | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}
