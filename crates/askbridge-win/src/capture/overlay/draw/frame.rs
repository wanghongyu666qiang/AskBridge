//! Frame composition: one pass that layers the desktop snapshot, the dimmed
//! backdrop, the selection chrome, and the fallback toolbar.

use std::mem::zeroed;

use askbridge_core::ScreenRect;
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, RECT},
    Graphics::Gdi::{
        AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, BeginPaint, BitBlt, EndPaint, PAINTSTRUCT, SRCCOPY,
    },
    UI::WindowsAndMessaging::GetClientRect,
};

use super::super::gdiplus::GdiPlusSession;
use super::super::guards::DesktopSnapshot;
use super::cache::{FrameInputs, PaintCache};
use super::primitives::{
    draw_instructions, draw_selection_frame_antialiased, draw_selection_handles, draw_size_label,
    fill, frame,
};
use super::toolbar::draw_toolbar;
use super::{COLOR_BORDER, COLOR_KEY, COLOR_OVERLAY, OVERLAY_ALPHA};

pub(in crate::capture::overlay) fn paint_overlay(
    window: HWND,
    state: &mut super::super::session::OverlayState,
) {
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
                    super::primitives::draw_selection_handles_antialiased(gdi, &selection_rect);
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
