// Overlay painting: double-buffered frame composition, toolbar drawing
// (GDI+ with a plain GDI fallback), and the shared color palette.
//
// GDI objects are expensive to create, and WM_PAINT fires on every mouse
// move during a drag. `PaintCache` keeps the per-window resources (double
// buffer, snapshot DC, font, brushes, pens, alpha-blend source) alive for
// the whole overlay session instead of rebuilding them per paint.

use std::{mem::zeroed, ptr};

use askbridge_core::ScreenRect;
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, RECT},
    Graphics::Gdi::{
        AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, BeginPaint, BitBlt, CreateCompatibleBitmap,
        CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DT_CENTER, DT_LEFT,
        DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint, FillRect,
        FrameRect, HBITMAP, HBRUSH, HFONT, HPEN, LineTo, MoveToEx, PAINTSTRUCT, PS_SOLID,
        Rectangle, RoundRect, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::WindowsAndMessaging::GetClientRect,
};

use crate::util::wide;

use super::{
    gdiplus::GdiPlusSession,
    guards::DesktopSnapshot,
    layout::{fallback_toolbar_size, inset_rect, selection_handle_points, toolbar_layout},
    session::ToolbarState,
};

pub(super) const COLOR_KEY: COLORREF = rgb(255, 0, 255);
const COLOR_OVERLAY: COLORREF = rgb(0, 0, 0);
const COLOR_BORDER: COLORREF = rgb(255, 255, 255);
const COLOR_LABEL: COLORREF = rgb(15, 23, 42);
const COLOR_TOOLBAR: COLORREF = rgb(28, 28, 31);
const COLOR_TOOLBAR_BORDER: COLORREF = rgb(75, 75, 80);
const COLOR_TOOLBAR_TEXT: COLORREF = rgb(245, 245, 247);
const COLOR_TOOLBAR_HOVER: COLORREF = rgb(43, 43, 47);
const COLOR_DROPDOWN_SELECTED: COLORREF = rgb(55, 55, 60);
const COLOR_TOOLBAR_ACCENT: COLORREF = rgb(153, 60, 29);
const ARGB_BORDER: u32 = argb(255, 255, 255, 255);
const ARGB_LABEL: u32 = argb(255, 15, 23, 42);
const ARGB_TOOLBAR: u32 = argb(255, 28, 28, 31);
const ARGB_TOOLBAR_BORDER: u32 = argb(255, 75, 75, 80);
const ARGB_TOOLBAR_TEXT: u32 = argb(255, 245, 245, 247);
const ARGB_TOOLBAR_HOVER: u32 = argb(255, 43, 43, 47);
const ARGB_DROPDOWN_SELECTED: u32 = argb(255, 55, 55, 60);
const ARGB_TOOLBAR_ACCENT: u32 = argb(255, 153, 60, 29);
pub(super) const OVERLAY_ALPHA: u8 = 145;
const TOOLBAR_RADIUS: i32 = 18;
const HANDLE_RADIUS: i32 = 5;

/// Everything the frame needs from the session, resolved once per paint so
/// painting never has to reach back into [`OverlayState`](super::session::OverlayState).
struct FrameInputs<'a> {
    snapshot: Option<&'a DesktopSnapshot>,
    selection: Option<ScreenRect>,
    locked: bool,
    fallback_toolbar: Option<&'a ToolbarState>,
}

/// GDI resources reused across paints for one overlay window.
///
/// All handles are created and used on the UI thread only. The struct must
/// be dropped before the session's [`DesktopSnapshot`]: the cached snapshot
/// DC keeps that bitmap selected, and a bitmap must not be deleted while a
/// DC holds it. `OverlayState` therefore declares this field first.
pub(super) struct PaintCache {
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
    unsafe fn ensure_buffer(
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
    unsafe fn ensure_snapshot_dc(
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
    unsafe fn ensure_font(&mut self) -> HFONT {
        if self.font.is_null() {
            let face = wide("Microsoft YaHei UI");
            // SAFETY: face is nul-terminated and valid for this synchronous call.
            self.font =
                unsafe { CreateFontW(-16, 0, 0, 0, 500, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) };
        }
        self.font
    }

    fn brush(&mut self, color: COLORREF) -> HBRUSH {
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

    fn pen(&mut self, color: COLORREF, width: i32) -> HPEN {
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
    unsafe fn ensure_alpha_source(
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

pub(super) fn paint_overlay(window: HWND, state: &mut super::session::OverlayState) {
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

        let inputs = FrameInputs {
            snapshot: state.desktop_snapshot.as_ref(),
            selection: state.selection(),
            locked: state.locked_selection.is_some(),
            fallback_toolbar: if state.locked_selection.is_some()
                && state.web_toolbar.is_none()
                && state.web_toolbar_failed
            {
                state.toolbar.as_ref()
            } else {
                None
            },
        };

        let buffer_dc = state
            .paint_cache
            .ensure_buffer(device_context, width, height);
        let (target_context, direct_draw) = if !buffer_dc.is_null() {
            (buffer_dc, false)
        } else {
            (device_context, true)
        };
        // One graphics object per paint, bound to the context actually drawn
        // into. The expensive process-wide GDI+ startup lives in the session
        // state and is not repeated per frame.
        let gdi_session = state
            .gdi_runtime
            .as_ref()
            .and_then(|runtime| GdiPlusSession::start(target_context, runtime));

        draw_overlay_frame(
            target_context,
            &client,
            &inputs,
            &mut state.paint_cache,
            gdi_session.as_ref(),
        );

        if !direct_draw {
            BitBlt(
                device_context,
                client.left,
                client.top,
                width,
                height,
                target_context,
                0,
                0,
                SRCCOPY,
            );
        }
        drop(gdi_session);
        EndPaint(window, &paint);
    }
}

unsafe fn draw_overlay_frame(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    inputs: &FrameInputs<'_>,
    cache: &mut PaintCache,
    gdi: Option<&GdiPlusSession>,
) {
    // SAFETY: device_context is a live paint or compatible memory DC.
    unsafe {
        let has_snapshot = draw_desktop_snapshot(device_context, inputs.snapshot, cache);
        if has_snapshot {
            dim_selection_backdrop(device_context, client, inputs.selection, cache);
        } else {
            fill(device_context, client, COLOR_OVERLAY, cache);
        }

        if let Some(selection) = inputs.selection {
            let selection_rect = RECT {
                left: selection.left,
                top: selection.top,
                right: selection.right() as i32,
                bottom: selection.bottom() as i32,
            };
            if !has_snapshot {
                fill(device_context, &selection_rect, COLOR_KEY, cache);
            }
            if has_snapshot {
                if let Some(gdi) = gdi {
                    draw_selection_frame_antialiased(gdi, &selection_rect);
                    draw_selection_handles_antialiased(gdi, &selection_rect);
                } else {
                    frame(device_context, &selection_rect, COLOR_BORDER, cache);
                    draw_selection_handles(device_context, &selection_rect, gdi, cache);
                }
            } else {
                frame(device_context, &selection_rect, COLOR_BORDER, cache);
                draw_selection_handles(device_context, &selection_rect, gdi, cache);
            }
            draw_size_label(
                device_context,
                client,
                &selection_rect,
                selection,
                cache,
                gdi,
            );
            if let Some(toolbar) = inputs.fallback_toolbar {
                draw_toolbar(device_context, client, &selection_rect, toolbar, cache, gdi);
            }
        }
        if !inputs.locked {
            draw_instructions(device_context, client, cache);
        }
    }
}

unsafe fn draw_toolbar(
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
    layout: &super::layout::ToolbarLayout,
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

unsafe fn draw_desktop_snapshot(
    device_context: *mut core::ffi::c_void,
    snapshot: Option<&DesktopSnapshot>,
    cache: &mut PaintCache,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    if snapshot.bitmap.is_null() || snapshot.width <= 0 || snapshot.height <= 0 {
        return false;
    }

    // SAFETY: device_context is live and the cached DC keeps only the snapshot selected.
    let snapshot_context = unsafe { cache.ensure_snapshot_dc(device_context, snapshot) };
    if snapshot_context.is_null() {
        return false;
    }
    // SAFETY: Source and destination DCs are valid with matching dimensions.
    unsafe {
        BitBlt(
            device_context,
            0,
            0,
            snapshot.width,
            snapshot.height,
            snapshot_context,
            0,
            0,
            SRCCOPY,
        ) != 0
    }
}

unsafe fn dim_selection_backdrop(
    device_context: *mut core::ffi::c_void,
    client: &RECT,
    selection: Option<ScreenRect>,
    cache: &mut PaintCache,
) {
    let Some(selection) = selection else {
        unsafe {
            alpha_fill(device_context, client, COLOR_OVERLAY, OVERLAY_ALPHA, cache);
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
            alpha_fill(device_context, &rect, COLOR_OVERLAY, OVERLAY_ALPHA, cache);
        }
    }
}

unsafe fn alpha_fill(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    color: COLORREF,
    alpha: u8,
    cache: &mut PaintCache,
) {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }
    // SAFETY: device_context is a live paint or compatible memory DC.
    let source_context = unsafe { cache.ensure_alpha_source(device_context) };
    if source_context.is_null() {
        // SAFETY: device_context and rect are valid for the current paint.
        unsafe {
            fill(device_context, rect, color, cache);
        }
        return;
    }
    let source_rect = RECT {
        left: 0,
        top: 0,
        right: 1,
        bottom: 1,
    };
    // SAFETY: source_context owns the cached 1×1 bitmap for this paint.
    unsafe {
        fill(source_context, &source_rect, color, cache);
    }
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: 0,
    };
    // SAFETY: Source and destination DCs are valid for this synchronous blend.
    let blended = unsafe {
        AlphaBlend(
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
        )
    };
    if blended == 0 {
        // SAFETY: device_context and rect are valid for the current paint.
        unsafe {
            fill(device_context, rect, color, cache);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ToolbarIcon {
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

unsafe fn draw_text_in_rect(
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

unsafe fn fill(
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

unsafe fn frame(
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

unsafe fn rounded_rect(
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

unsafe fn draw_selection_handles(
    device_context: *mut core::ffi::c_void,
    rect: &RECT,
    gdi: Option<&GdiPlusSession>,
    cache: &mut PaintCache,
) {
    let points = selection_handle_points(rect);
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

unsafe fn draw_size_label(
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

unsafe fn draw_instructions(
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

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

const fn argb(alpha: u8, red: u8, green: u8, blue: u8) -> u32 {
    ((alpha as u32) << 24) | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}
