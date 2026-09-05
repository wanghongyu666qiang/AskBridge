//! Fallback (non-WebView) screenshot toolbar painted with GDI, plus the
//! GDI+ antialiased variants of every toolbar element.

use std::ptr;

use windows_sys::Win32::{
    Foundation::{COLORREF, RECT},
    Graphics::Gdi::{
        DT_LEFT, DT_SINGLELINE, DT_VCENTER, LineTo, MoveToEx, Rectangle, RoundRect, SelectObject,
    },
};

use super::super::gdiplus::GdiPlusSession;
use super::super::layout::{fallback_toolbar_size, inset_rect, toolbar_layout};
use super::super::session::ToolbarState;
use super::cache::PaintCache;
use super::primitives::{draw_text_in_rect, rounded_rect};
use super::{
    ARGB_DROPDOWN_SELECTED, ARGB_TOOLBAR, ARGB_TOOLBAR_ACCENT, ARGB_TOOLBAR_BORDER,
    ARGB_TOOLBAR_HOVER, ARGB_TOOLBAR_TEXT, COLOR_DROPDOWN_SELECTED, COLOR_TOOLBAR,
    COLOR_TOOLBAR_ACCENT, COLOR_TOOLBAR_BORDER, COLOR_TOOLBAR_HOVER, COLOR_TOOLBAR_TEXT,
    TOOLBAR_RADIUS,
};

#[derive(Debug, Clone, Copy)]
enum ToolbarIcon {
    Provider,
    Copy,
    Cancel,
    Ask,
}

pub(super) unsafe fn draw_toolbar(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    selection_rect: &RECT,
    toolbar: &ToolbarState,
    cache: &mut PaintCache,
    gdi: Option<&GdiPlusSession>,
) {
    let layout = toolbar_layout(
        client,
        selection_rect,
        toolbar.providers.len(),
        fallback_toolbar_size(),
    );
    let Some(selected) = toolbar.providers.get(toolbar.selected_index) else {
        return;
    };
    unsafe {
        if let Some(gdi) = gdi {
            draw_toolbar_antialiased(gdi, &layout, toolbar);
            draw_toolbar_labels(device_context, &layout, toolbar, cache);
            return;
        }
        rounded_rect(
            device_context,
            &layout.outer,
            COLOR_TOOLBAR,
            COLOR_TOOLBAR_BORDER,
            TOOLBAR_RADIUS,
            cache,
        );
        draw_toolbar_item(
            device_context,
            &layout.copy,
            "复制",
            ToolbarIcon::Copy,
            false,
            cache,
        );
        draw_toolbar_item(
            device_context,
            &layout.cancel,
            "取消",
            ToolbarIcon::Cancel,
            false,
            cache,
        );
        draw_toolbar_item(
            device_context,
            &layout.provider,
            &selected.display_name,
            ToolbarIcon::Provider,
            true,
            cache,
        );
        rounded_rect(
            device_context,
            &layout.ask,
            COLOR_TOOLBAR_ACCENT,
            COLOR_TOOLBAR_ACCENT,
            10,
            cache,
        );
        draw_toolbar_item(
            device_context,
            &layout.ask,
            "问问",
            ToolbarIcon::Ask,
            false,
            cache,
        );
        if toolbar.dropdown_open {
            rounded_rect(
                device_context,
                &layout.dropdown_bounds,
                COLOR_TOOLBAR,
                COLOR_TOOLBAR_BORDER,
                12,
                cache,
            );
            for (index, rect) in layout.dropdown_rects.iter().enumerate() {
                if index == toolbar.selected_index {
                    rounded_rect(
                        device_context,
                        &inset_rect(rect, 4),
                        COLOR_DROPDOWN_SELECTED,
                        COLOR_DROPDOWN_SELECTED,
                        8,
                        cache,
                    );
                }
                let Some(provider) = toolbar.providers.get(index) else {
                    continue;
                };
                draw_text_in_rect(
                    device_context,
                    rect,
                    &provider.display_name,
                    COLOR_TOOLBAR_TEXT,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                    cache,
                );
            }
        }
    }
}

fn draw_toolbar_antialiased(
    gdi: &GdiPlusSession,
    layout: &super::super::layout::ToolbarLayout,
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
    gdi.rounded_rect_rect(
        &layout.ask,
        10.0,
        ARGB_TOOLBAR_ACCENT,
        ARGB_TOOLBAR_ACCENT,
        1.0,
    );
    draw_toolbar_icon_antialiased(gdi, &layout.copy, ToolbarIcon::Copy);
    draw_toolbar_icon_antialiased(gdi, &layout.cancel, ToolbarIcon::Cancel);
    draw_toolbar_icon_antialiased(gdi, &layout.provider, ToolbarIcon::Provider);
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
    layout: &super::super::layout::ToolbarLayout,
    toolbar: &ToolbarState,
    cache: &mut PaintCache,
) {
    unsafe {
        let selected = toolbar.providers.get(toolbar.selected_index);
        draw_toolbar_label(device_context, &layout.copy, "复制", cache);
        draw_toolbar_label(device_context, &layout.cancel, "取消", cache);
        if let Some(selected) = selected {
            draw_toolbar_label(
                device_context,
                &layout.provider,
                &selected.display_name,
                cache,
            );
        }
        draw_toolbar_label(device_context, &layout.ask, "问问", cache);
        if toolbar.dropdown_open {
            for (index, rect) in layout.dropdown_rects.iter().enumerate() {
                let Some(provider) = toolbar.providers.get(index) else {
                    continue;
                };
                draw_text_in_rect(
                    device_context,
                    rect,
                    &provider.display_name,
                    COLOR_TOOLBAR_TEXT,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                    cache,
                );
            }
        }
    }
}

unsafe fn draw_toolbar_label(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    label: &str,
    cache: &mut PaintCache,
) {
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
            cache,
        );
    }
}

unsafe fn draw_toolbar_item(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    label: &str,
    icon: ToolbarIcon,
    highlighted: bool,
    cache: &mut PaintCache,
) {
    unsafe {
        if highlighted {
            rounded_rect(
                device_context,
                rect,
                COLOR_TOOLBAR_HOVER,
                COLOR_TOOLBAR_HOVER,
                10,
                cache,
            );
        }
        draw_toolbar_icon(device_context, rect, icon, COLOR_TOOLBAR_TEXT, cache);
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
            cache,
        );
    }
}

unsafe fn draw_toolbar_icon(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    icon: ToolbarIcon,
    color: COLORREF,
    cache: &mut PaintCache,
) {
    let cx = rect.left + 18;
    let cy = rect.top + (rect.bottom - rect.top) / 2;
    // SAFETY: device_context is live; the cached pen stays selected only here.
    unsafe {
        let pen = cache.pen(color, 2);
        let previous_pen = if pen.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(device_context, pen)
        };
        match icon {
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
    }
}

fn draw_toolbar_icon_antialiased(gdi: &GdiPlusSession, rect: &RECT, icon: ToolbarIcon) {
    let cx = rect.left as f32 + 18.0;
    let cy = rect.top as f32 + (rect.bottom - rect.top) as f32 / 2.0;
    match icon {
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
