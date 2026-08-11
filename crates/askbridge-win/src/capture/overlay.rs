use std::{mem::zeroed, ptr};

use askbridge_core::{AppError, Result, ScreenRect};
use tracing::info;
use windows_sys::Win32::{
    Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::DwmFlush,
        Gdi::{
            BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush,
            DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, EndPaint,
            FillRect, FrameRect, InvalidateRect, PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode,
            SetTextColor, TRANSPARENT, UpdateWindow,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE},
        WindowsAndMessaging::{
            CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
            DispatchMessageW, GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW,
            HWND_TOPMOST, IDC_CROSS, LWA_ALPHA, LWA_COLORKEY, LoadCursorW, MSG, PostMessageW,
            PostQuitMessage, RegisterClassW, SW_HIDE, SWP_SHOWWINDOW, SetForegroundWindow,
            SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow,
            TranslateMessage, WM_CANCELMODE, WM_CLOSE, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN,
            WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY,
            WM_PAINT, WM_RBUTTONDOWN, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
            WS_POPUP,
        },
    },
};

use crate::{
    capture::{WM_CAPTURE_BUSY, monitor::DesktopLayout},
    util::{last_error, wide},
};

const OVERLAY_CLASS: &str = "AskBridge.CaptureOverlay.Window.v1";
const COLOR_KEY: COLORREF = rgb(255, 0, 255);
const COLOR_OVERLAY: COLORREF = rgb(0, 0, 0);
const COLOR_BORDER: COLORREF = rgb(255, 255, 255);
const COLOR_LABEL: COLORREF = rgb(15, 23, 42);
const OVERLAY_ALPHA: u8 = 145;

pub fn register_class(instance: HINSTANCE) -> Result<()> {
    let class_name = wide(OVERLAY_CLASS);
    // SAFETY: Loading a shared system cursor with a null instance is supported.
    let cursor = unsafe { LoadCursorW(ptr::null_mut(), IDC_CROSS) };
    if cursor.is_null() {
        return Err(AppError::Windows {
            operation: "LoadCursorW(IDC_CROSS)",
            win32_code: last_error(),
        });
    }
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(overlay_window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: cursor,
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    // SAFETY: All pointers remain valid for this synchronous registration call.
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(AppError::Windows {
            operation: "RegisterClassW(capture overlay)",
            win32_code: last_error(),
        });
    }
    Ok(())
}

pub fn select_region(
    instance: HINSTANCE,
    owner: HWND,
    layout: &DesktopLayout,
) -> Result<Option<ScreenRect>> {
    let bounds = layout.virtual_bounds;
    let width = i32::try_from(bounds.width)
        .map_err(|_| AppError::CaptureFailed("virtual screen width is too large".to_owned()))?;
    let height = i32::try_from(bounds.height)
        .map_err(|_| AppError::CaptureFailed("virtual screen height is too large".to_owned()))?;
    let mut state = Box::<OverlayState>::default();
    let class_name = wide(OVERLAY_CLASS);
    let title = wide("AskBridge 区域截图");
    // SAFETY: The class is registered and state remains allocated until the window is destroyed.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            bounds.left,
            bounds.top,
            width,
            height,
            owner,
            ptr::null_mut(),
            instance,
            state.as_mut() as *mut OverlayState as *mut _,
        )
    };
    if window.is_null() {
        return Err(AppError::Windows {
            operation: "CreateWindowExW(capture overlay)",
            win32_code: last_error(),
        });
    }
    let window = OverlayWindow(window);

    // SAFETY: window is a live layered window.
    if unsafe {
        SetLayeredWindowAttributes(window.0, COLOR_KEY, OVERLAY_ALPHA, LWA_ALPHA | LWA_COLORKEY)
    } == 0
    {
        return Err(AppError::Windows {
            operation: "SetLayeredWindowAttributes",
            win32_code: last_error(),
        });
    }
    // SAFETY: Bounds are validated and window is live.
    if unsafe {
        SetWindowPos(
            window.0,
            HWND_TOPMOST,
            bounds.left,
            bounds.top,
            width,
            height,
            SWP_SHOWWINDOW,
        )
    } == 0
    {
        return Err(AppError::Windows {
            operation: "SetWindowPos(capture overlay)",
            win32_code: last_error(),
        });
    }
    // SAFETY: The hotkey-triggered overlay is intended to receive immediate keyboard input.
    unsafe {
        SetForegroundWindow(window.0);
        SetFocus(window.0);
        UpdateWindow(window.0);
    }

    let mut quit_code = None;
    let mut ignored_hotkey = false;
    while state.outcome == OverlayOutcome::Pending {
        // SAFETY: Zero is the documented initial state for MSG.
        let mut message: MSG = unsafe { zeroed() };
        // SAFETY: message points to writable storage and the null HWND reads this thread queue.
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result == -1 {
            return Err(AppError::Windows {
                operation: "GetMessageW(capture overlay)",
                win32_code: last_error(),
            });
        }
        if result == 0 {
            quit_code = Some(message.wParam as i32);
            state.outcome = OverlayOutcome::Cancelled(CancelReason::Quit);
            break;
        }
        if message.message == WM_HOTKEY {
            ignored_hotkey = true;
            continue;
        }
        // SAFETY: message was populated by GetMessageW.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    // SAFETY: Hiding before DwmFlush ensures the overlay is not captured.
    unsafe {
        ShowWindow(window.0, SW_HIDE);
    }
    // SAFETY: DwmFlush synchronizes pending desktop composition commands.
    let flush_result = unsafe { DwmFlush() };
    if flush_result < 0 {
        return Err(AppError::CaptureFailed(format!(
            "desktop composition synchronization failed (HRESULT 0x{:08X})",
            flush_result as u32
        )));
    }

    let outcome = state.outcome;
    drop(window);
    if let Some(code) = quit_code {
        // SAFETY: Preserve the quit message consumed by the nested selection loop.
        unsafe {
            PostQuitMessage(code);
        }
    }
    if ignored_hotkey {
        // SAFETY: owner is the live AskBridge main window on the same UI thread.
        unsafe {
            PostMessageW(owner, WM_CAPTURE_BUSY, 0, 0);
        }
    }

    match outcome {
        OverlayOutcome::Selected(local_rect) => local_rect
            .translated(bounds.left, bounds.top)
            .map(Some)
            .ok_or_else(|| {
                AppError::CaptureFailed(
                    "selection coordinates overflow the virtual desktop".to_owned(),
                )
            }),
        OverlayOutcome::Cancelled(reason) => {
            info!(reason = ?reason, "capture overlay cancelled");
            Ok(None)
        }
        OverlayOutcome::Pending => Ok(None),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum OverlayOutcome {
    #[default]
    Pending,
    Selected(ScreenRect),
    Cancelled(CancelReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancelReason {
    Quit,
    EmptySelection,
    Escape,
    RightClick,
    Close,
    CancelMode,
}

#[derive(Default)]
struct OverlayState {
    anchor: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
    outcome: OverlayOutcome,
}

impl OverlayState {
    fn selection(&self) -> Option<ScreenRect> {
        ScreenRect::from_points(self.anchor?, self.current?)
    }

    fn update_drag(&mut self, point: (i32, i32)) -> RepaintRequest {
        if self.anchor.is_none() || self.current == Some(point) {
            return RepaintRequest::None;
        }
        self.current = Some(point);
        RepaintRequest::Deferred
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepaintRequest {
    None,
    Deferred,
}

struct OverlayWindow(HWND);

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

unsafe extern "system" fn overlay_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: WM_NCCREATE lparam points to a valid CREATESTRUCTW for this call.
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        // SAFETY: lpCreateParams is the OverlayState pointer supplied to CreateWindowExW.
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        return 1;
    }
    // SAFETY: This value is set during WM_NCCREATE and cleared during WM_NCDESTROY.
    let state_ptr = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut OverlayState };
    if message == WM_NCDESTROY {
        // SAFETY: The window no longer needs access to the caller-owned state.
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            return DefWindowProcW(window, message, wparam, lparam);
        }
    }
    if state_ptr.is_null() {
        // SAFETY: Uninitialized messages use the default window procedure.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    // SAFETY: Window messages are dispatched serially on the creating thread.
    let state = unsafe { &mut *state_ptr };

    if let Some(reason) = cancellation_reason(message, wparam) {
        cancel_selection(window, state, reason);
        return 0;
    }

    match message {
        WM_LBUTTONDOWN => {
            let point = mouse_point(lparam);
            state.anchor = Some(point);
            state.current = Some(point);
            // SAFETY: window is live and will release capture when selection ends.
            unsafe {
                SetCapture(window);
                InvalidateRect(window, ptr::null(), 0);
            }
            0
        }
        WM_MOUSEMOVE => {
            match state.update_drag(mouse_point(lparam)) {
                RepaintRequest::None => {}
                RepaintRequest::Deferred => {
                    // SAFETY: window is live; consecutive invalidations may be coalesced.
                    unsafe {
                        InvalidateRect(window, ptr::null(), 0);
                    }
                }
            }
            0
        }
        WM_LBUTTONUP => {
            if state.anchor.is_some() {
                state.current = Some(mouse_point(lparam));
                state.outcome = state
                    .selection()
                    .map(OverlayOutcome::Selected)
                    .unwrap_or(OverlayOutcome::Cancelled(CancelReason::EmptySelection));
                // SAFETY: The window owns mouse capture during a drag.
                unsafe {
                    ReleaseCapture();
                    ShowWindow(window, SW_HIDE);
                }
            }
            0
        }
        WM_PAINT => {
            paint_overlay(window, state);
            0
        }
        WM_ERASEBKGND => 1,
        _ => {
            // SAFETY: Unhandled messages are forwarded exactly as received.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

const fn cancellation_reason(message: u32, wparam: WPARAM) -> Option<CancelReason> {
    match message {
        WM_KEYDOWN if wparam as u16 == VK_ESCAPE => Some(CancelReason::Escape),
        WM_RBUTTONDOWN => Some(CancelReason::RightClick),
        WM_CLOSE => Some(CancelReason::Close),
        WM_CANCELMODE => Some(CancelReason::CancelMode),
        // A hotkey-created topmost overlay can receive a normal focus transition before the
        // user's first click. Treating that transition as cancellation makes every real drag
        // disappear before WM_LBUTTONUP can commit the selection.
        WM_KILLFOCUS => None,
        _ => None,
    }
}

fn cancel_selection(window: HWND, state: &mut OverlayState, reason: CancelReason) {
    state.outcome = OverlayOutcome::Cancelled(reason);
    // SAFETY: Releasing without capture is harmless; window is live.
    unsafe {
        ReleaseCapture();
        ShowWindow(window, SW_HIDE);
    }
}

fn paint_overlay(window: HWND, state: &OverlayState) {
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
        fill(device_context, client, COLOR_OVERLAY);

        if let Some(selection) = state.selection() {
            let selection_rect = RECT {
                left: selection.left,
                top: selection.top,
                right: selection.right() as i32,
                bottom: selection.bottom() as i32,
            };
            fill(device_context, &selection_rect, COLOR_KEY);
            frame(device_context, &selection_rect, COLOR_BORDER);
            draw_size_label(device_context, client, &selection_rect, selection);
        }
        draw_instructions(device_context, client);
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
        fill(device_context, &label_rect, COLOR_LABEL);
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

const fn mouse_point(lparam: LPARAM) -> (i32, i32) {
    let value = lparam as u32;
    (
        (value as u16 as i16) as i32,
        ((value >> 16) as u16 as i16) as i32,
    )
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_signed_mouse_coordinates() {
        let packed =
            ((20_u32 << 16) | u16::from_ne_bytes((-30_i16).to_ne_bytes()) as u32) as LPARAM;

        assert_eq!(mouse_point(packed), (-30, 20));
    }

    #[test]
    fn drag_updates_request_deferred_repaint() {
        let mut state = OverlayState {
            anchor: Some((10, 10)),
            current: Some((10, 10)),
            outcome: OverlayOutcome::Pending,
        };

        assert_eq!(state.update_drag((20, 30)), RepaintRequest::Deferred);
        assert_eq!(state.current, Some((20, 30)));
        assert_eq!(state.update_drag((20, 30)), RepaintRequest::None);
    }

    #[test]
    fn focus_loss_does_not_cancel_a_hotkey_created_overlay() {
        assert_eq!(cancellation_reason(WM_KILLFOCUS, 0), None);
        assert_eq!(
            cancellation_reason(WM_KEYDOWN, usize::from(VK_ESCAPE)),
            Some(CancelReason::Escape)
        );
        assert_eq!(
            cancellation_reason(WM_RBUTTONDOWN, 0),
            Some(CancelReason::RightClick)
        );
    }
}
