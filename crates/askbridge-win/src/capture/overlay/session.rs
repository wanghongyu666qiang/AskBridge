// Overlay session orchestration: the nested message loop, per-selection
// state, the window procedure, and all mouse/keyboard message handling.

use std::{
    mem::{replace, zeroed},
    ptr,
};

use askbridge_core::{AppError, Result, ScreenRect};
use tracing::{info, warn};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::DwmFlush,
        Gdi::{InvalidateRect, UpdateWindow},
    },
    UI::{
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN},
        WindowsAndMessaging::{
            CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA,
            GetClientRect, GetMessageW, GetWindowLongPtrW, HWND_TOPMOST, LWA_ALPHA, LWA_COLORKEY,
            MSG, PostMessageW, PostQuitMessage, SW_HIDE, SWP_SHOWWINDOW, SetForegroundWindow,
            SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow,
            TranslateMessage, WM_CANCELMODE, WM_CLOSE, WM_ERASEBKGND, WM_HOTKEY, WM_KEYDOWN,
            WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY,
            WM_PAINT, WM_RBUTTONDOWN, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

use crate::{
    capture::{WM_CAPTURE_BUSY, monitor::DesktopLayout, toolbar_webview},
    util::{last_error, wide},
};

use super::{
    OVERLAY_CLASS, OverlayProviderChoice, SelectionAction, SelectionResult,
    draw::{COLOR_KEY, OVERLAY_ALPHA, paint_overlay},
    guards::{DesktopSnapshot, OverlayWindow},
    layout::{fallback_toolbar_size, hit_dropdown, point_in_rect, toolbar_layout},
};

pub(super) fn select_region_internal(
    instance: HINSTANCE,
    owner: HWND,
    layout: &DesktopLayout,
    providers: Option<Vec<OverlayProviderChoice>>,
) -> Result<Option<SelectionOutcome>> {
    let bounds = layout.virtual_bounds;
    let width = i32::try_from(bounds.width)
        .map_err(|_| AppError::CaptureFailed("virtual screen width is too large".to_owned()))?;
    let height = i32::try_from(bounds.height)
        .map_err(|_| AppError::CaptureFailed("virtual screen height is too large".to_owned()))?;
    let desktop_snapshot = DesktopSnapshot::capture(bounds)
        .inspect_err(|error| warn!(%error, "capture overlay desktop snapshot unavailable"));
    let use_snapshot_backdrop = desktop_snapshot.is_ok();
    let mut state = Box::new(OverlayState::new(
        instance,
        providers,
        desktop_snapshot.ok(),
    ));
    let class_name = wide(OVERLAY_CLASS);
    let title = wide("AskBridge 区域截图");
    // SAFETY: The class is registered and state remains allocated until the window is destroyed.
    let window = unsafe {
        CreateWindowExW(
            overlay_ex_style(use_snapshot_backdrop),
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

    if !use_snapshot_backdrop {
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

    let outcome = replace(&mut state.outcome, OverlayOutcome::Pending);
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
            .map(|rect| Some(SelectionOutcome::Quick(rect)))
            .ok_or_else(|| {
                AppError::CaptureFailed(
                    "selection coordinates overflow the virtual desktop".to_owned(),
                )
            }),
        OverlayOutcome::Action { rect, action } => rect
            .translated(bounds.left, bounds.top)
            .map(|rect| Some(SelectionOutcome::Action(SelectionResult { rect, action })))
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

pub(super) enum SelectionOutcome {
    Quick(ScreenRect),
    Action(SelectionResult),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum OverlayOutcome {
    #[default]
    Pending,
    Selected(ScreenRect),
    Action {
        rect: ScreenRect,
        action: SelectionAction,
    },
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
pub(super) struct OverlayState {
    instance: HINSTANCE,
    pub(super) desktop_snapshot: Option<DesktopSnapshot>,
    anchor: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
    pub(super) locked_selection: Option<ScreenRect>,
    pub(super) toolbar: Option<ToolbarState>,
    pub(super) web_toolbar: Option<toolbar_webview::ToolbarWebView>,
    pub(super) web_toolbar_failed: bool,
    outcome: OverlayOutcome,
}

impl OverlayState {
    fn new(
        instance: HINSTANCE,
        providers: Option<Vec<OverlayProviderChoice>>,
        desktop_snapshot: Option<DesktopSnapshot>,
    ) -> Self {
        let toolbar = providers.map(ToolbarState::new);
        Self {
            instance,
            desktop_snapshot,
            toolbar,
            ..Self::default()
        }
    }

    pub(super) fn selection(&self) -> Option<ScreenRect> {
        if let Some(selection) = self.locked_selection {
            return Some(selection);
        }
        ScreenRect::from_points(self.anchor?, self.current?)
    }

    fn update_drag(&mut self, point: (i32, i32)) -> RepaintRequest {
        if self.locked_selection.is_some() || self.anchor.is_none() || self.current == Some(point) {
            return RepaintRequest::None;
        }
        self.current = Some(point);
        RepaintRequest::Deferred
    }

    fn begin_drag(&mut self, point: (i32, i32)) {
        self.locked_selection = None;
        self.web_toolbar = None;
        self.web_toolbar_failed = false;
        if let Some(toolbar) = &mut self.toolbar {
            toolbar.dropdown_open = false;
        }
        self.anchor = Some(point);
        self.current = Some(point);
    }

    fn finish_drag(&mut self, window: HWND, point: (i32, i32)) {
        self.current = Some(point);
        let Some(selection) = self.selection() else {
            self.outcome = OverlayOutcome::Cancelled(CancelReason::EmptySelection);
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
            return;
        };
        unsafe {
            ReleaseCapture();
        }
        if self.toolbar.is_some() {
            self.locked_selection = Some(selection);
            self.anchor = None;
            self.current = None;
            self.show_web_toolbar(window, selection);
            unsafe {
                InvalidateRect(window, ptr::null(), 0);
                UpdateWindow(window);
            }
        } else {
            self.outcome = OverlayOutcome::Selected(selection);
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
        }
    }

    fn show_web_toolbar(&mut self, window: HWND, selection: ScreenRect) {
        self.web_toolbar = None;
        self.web_toolbar_failed = false;
        if self.desktop_snapshot.is_none() {
            self.web_toolbar_failed = true;
            return;
        }
        let Some(toolbar) = &self.toolbar else {
            return;
        };
        let mut client = RECT::default();
        // SAFETY: window is live and client points to writable storage.
        unsafe {
            GetClientRect(window, &mut client);
        }
        let selection_rect = RECT {
            left: selection.left,
            top: selection.top,
            right: selection.right() as i32,
            bottom: selection.bottom() as i32,
        };
        let layout = toolbar_layout(
            &client,
            &selection_rect,
            toolbar.providers.len(),
            toolbar_webview::preferred_size_for_window(window),
        );
        let providers = toolbar
            .providers
            .iter()
            .enumerate()
            .map(|(index, provider)| toolbar_webview::ToolbarProvider {
                id: provider.id.clone(),
                display_name: provider.display_name.clone(),
                selected: index == toolbar.selected_index,
            })
            .collect::<Vec<_>>();
        match toolbar_webview::show(self.instance, window, &layout.outer, providers) {
            Ok(web_toolbar) => {
                self.web_toolbar = Some(web_toolbar);
            }
            Err(error) => {
                self.web_toolbar_failed = true;
                warn!(%error, "capture toolbar webview unavailable; using fallback drawing");
            }
        }
    }
}

pub(super) struct ToolbarState {
    pub(super) providers: Vec<OverlayProviderChoice>,
    pub(super) selected_index: usize,
    pub(super) dropdown_open: bool,
}

impl ToolbarState {
    fn new(providers: Vec<OverlayProviderChoice>) -> Self {
        let selected_index = providers
            .iter()
            .position(|provider| provider.selected)
            .unwrap_or(0);
        Self {
            providers,
            selected_index,
            dropdown_open: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepaintRequest {
    None,
    Deferred,
}

const fn overlay_ex_style(use_snapshot_backdrop: bool) -> u32 {
    if use_snapshot_backdrop {
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW
    } else {
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW
    }
}

pub(super) unsafe extern "system" fn overlay_window_proc(
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
    if asks_with_enter(message, wparam) && ask_with_selected_provider(window, state) {
        return 0;
    }

    match message {
        toolbar_webview::ACTION_MESSAGE => {
            handle_web_toolbar_action(window, state, wparam, lparam);
            0
        }
        WM_LBUTTONDOWN => {
            let point = mouse_point(lparam);
            if handle_toolbar_click(window, state, point) {
                return 0;
            }
            state.begin_drag(point);
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
                state.finish_drag(window, mouse_point(lparam));
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

const fn asks_with_enter(message: u32, wparam: WPARAM) -> bool {
    message == WM_KEYDOWN && wparam as u16 == VK_RETURN
}

fn cancel_selection(window: HWND, state: &mut OverlayState, reason: CancelReason) {
    state.outcome = OverlayOutcome::Cancelled(reason);
    state.web_toolbar = None;
    // SAFETY: Releasing without capture is harmless; window is live.
    unsafe {
        ReleaseCapture();
        ShowWindow(window, SW_HIDE);
    }
}

fn handle_web_toolbar_action(
    window: HWND,
    state: &mut OverlayState,
    action: WPARAM,
    value: LPARAM,
) {
    if action == toolbar_webview::ACTION_MENU_OPEN || action == toolbar_webview::ACTION_MENU_CLOSE {
        if let Some(web_toolbar) = &state.web_toolbar
            && let Err(error) =
                web_toolbar.set_menu_open(action == toolbar_webview::ACTION_MENU_OPEN)
        {
            warn!(%error, "capture toolbar provider menu resize failed");
        }
        return;
    }
    match action {
        toolbar_webview::ACTION_COPY => {
            let Some(selection) = state.locked_selection else {
                return;
            };
            state.outcome = OverlayOutcome::Action {
                rect: selection,
                action: SelectionAction::Copy,
            };
            state.web_toolbar = None;
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
        }
        toolbar_webview::ACTION_CANCEL => {
            state.outcome = OverlayOutcome::Cancelled(CancelReason::Close);
            state.web_toolbar = None;
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
        }
        toolbar_webview::ACTION_ASK => {
            let index = value as usize;
            let Some(toolbar) = &mut state.toolbar else {
                return;
            };
            if index >= toolbar.providers.len() {
                return;
            }
            toolbar.selected_index = index;
            ask_with_selected_provider(window, state);
        }
        _ => {}
    }
}

fn ask_with_selected_provider(window: HWND, state: &mut OverlayState) -> bool {
    let Some(selection) = state.locked_selection else {
        return false;
    };
    let Some(provider_id) = state.toolbar.as_ref().and_then(|toolbar| {
        toolbar
            .providers
            .get(toolbar.selected_index)
            .map(|provider| provider.id.clone())
    }) else {
        return false;
    };
    state.outcome = OverlayOutcome::Action {
        rect: selection,
        action: SelectionAction::Ask { provider_id },
    };
    state.web_toolbar = None;
    // SAFETY: window is the live capture overlay being completed.
    unsafe {
        ShowWindow(window, SW_HIDE);
    }
    true
}

fn handle_toolbar_click(window: HWND, state: &mut OverlayState, point: (i32, i32)) -> bool {
    let Some(selection) = state.locked_selection else {
        return false;
    };
    let Some(toolbar) = &mut state.toolbar else {
        return false;
    };
    let mut client = RECT::default();
    // SAFETY: window is live and client points to writable storage.
    unsafe {
        GetClientRect(window, &mut client);
    }
    let selection_rect = RECT {
        left: selection.left,
        top: selection.top,
        right: selection.right() as i32,
        bottom: selection.bottom() as i32,
    };
    let layout = toolbar_layout(
        &client,
        &selection_rect,
        toolbar.providers.len(),
        fallback_toolbar_size(),
    );
    if toolbar.dropdown_open {
        if let Some(index) = hit_dropdown(&layout.dropdown_rects, point) {
            toolbar.selected_index = index;
            toolbar.dropdown_open = false;
            unsafe {
                InvalidateRect(window, ptr::null(), 0);
            }
            return true;
        }
    }
    if point_in_rect(point, &layout.provider) {
        toolbar.dropdown_open = !toolbar.dropdown_open;
        unsafe {
            InvalidateRect(window, ptr::null(), 0);
        }
        return true;
    }
    if point_in_rect(point, &layout.more) {
        return true;
    }
    if point_in_rect(point, &layout.copy) {
        state.outcome = OverlayOutcome::Action {
            rect: selection,
            action: SelectionAction::Copy,
        };
        unsafe {
            ShowWindow(window, SW_HIDE);
        }
        return true;
    }
    if point_in_rect(point, &layout.cancel) {
        state.outcome = OverlayOutcome::Cancelled(CancelReason::Close);
        unsafe {
            ShowWindow(window, SW_HIDE);
        }
        return true;
    }
    if point_in_rect(point, &layout.ask) {
        ask_with_selected_provider(window, state);
        return true;
    }
    if point_in_rect(point, &layout.outer)
        || (toolbar.dropdown_open && point_in_rect(point, &layout.dropdown_bounds))
    {
        return true;
    }
    false
}

const fn mouse_point(lparam: LPARAM) -> (i32, i32) {
    let value = lparam as u32;
    (
        (value as u16 as i16) as i32,
        ((value >> 16) as u16 as i16) as i32,
    )
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
            ..OverlayState::default()
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

    #[test]
    fn enter_maps_to_ask_without_changing_escape_cancellation() {
        assert!(asks_with_enter(WM_KEYDOWN, usize::from(VK_RETURN)));
        assert!(!asks_with_enter(WM_KEYDOWN, usize::from(VK_ESCAPE)));
        assert!(!asks_with_enter(WM_LBUTTONDOWN, usize::from(VK_RETURN)));
        assert_eq!(
            cancellation_reason(WM_KEYDOWN, usize::from(VK_ESCAPE)),
            Some(CancelReason::Escape)
        );
    }
}
