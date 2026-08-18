use std::{
    mem::{replace, zeroed},
    ptr,
};

use askbridge_core::{AppError, Result, ScreenRect};
use tracing::{info, warn};
use windows_sys::Win32::{
    Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
        Dwm::DwmFlush,
        Gdi::{
            AC_SRC_OVER, AlphaBlend, BLENDFUNCTION, BeginPaint, BitBlt, CAPTUREBLT,
            CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush,
            DT_CENTER, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW,
            Ellipse, EndPaint, FillRect, FrameRect, GetDC, HBITMAP, HDC, InvalidateRect, LineTo,
            MoveToEx, PAINTSTRUCT, PS_SOLID, Rectangle, ReleaseDC, RoundRect, SRCCOPY,
            SelectObject, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN},
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
    capture::{WM_CAPTURE_BUSY, monitor::DesktopLayout, toolbar_webview},
    util::{last_error, wide},
};

const OVERLAY_CLASS: &str = "AskBridge.CaptureOverlay.Window.v1";
const COLOR_KEY: COLORREF = rgb(255, 0, 255);
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
const OVERLAY_ALPHA: u8 = 145;
const TOOLBAR_GAP: i32 = 12;
const DROPDOWN_ROW_HEIGHT: i32 = 34;
const MORE_WIDTH: i32 = 82;
const PROVIDER_WIDTH: i32 = 202;
const COPY_WIDTH: i32 = 86;
const CANCEL_WIDTH: i32 = 86;
const ASK_WIDTH: i32 = 188;
const BUTTON_GAP: i32 = 4;
const TOOLBAR_PADDING: i32 = 10;
const TOOLBAR_RADIUS: i32 = 18;
const HANDLE_RADIUS: i32 = 5;

#[derive(Debug, Clone)]
pub struct OverlayProviderChoice {
    pub id: String,
    pub display_name: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionAction {
    Ask { provider_id: String },
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionResult {
    pub rect: ScreenRect,
    pub action: SelectionAction,
}

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
    select_region_internal(instance, owner, layout, None).map(|outcome| match outcome {
        Some(SelectionOutcome::Quick(rect)) => Some(rect),
        Some(SelectionOutcome::Action(result)) => Some(result.rect),
        None => None,
    })
}

pub fn select_region_with_toolbar(
    instance: HINSTANCE,
    owner: HWND,
    layout: &DesktopLayout,
    providers: Vec<OverlayProviderChoice>,
) -> Result<Option<SelectionResult>> {
    select_region_internal(instance, owner, layout, Some(providers)).map(|outcome| match outcome {
        Some(SelectionOutcome::Action(result)) => Some(result),
        Some(SelectionOutcome::Quick(rect)) => Some(SelectionResult {
            rect,
            action: SelectionAction::Ask {
                provider_id: String::new(),
            },
        }),
        None => None,
    })
}

fn select_region_internal(
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

enum SelectionOutcome {
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
struct OverlayState {
    instance: HINSTANCE,
    desktop_snapshot: Option<DesktopSnapshot>,
    anchor: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
    locked_selection: Option<ScreenRect>,
    toolbar: Option<ToolbarState>,
    web_toolbar: Option<toolbar_webview::ToolbarWebView>,
    web_toolbar_failed: bool,
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

    fn selection(&self) -> Option<ScreenRect> {
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

struct ToolbarState {
    providers: Vec<OverlayProviderChoice>,
    selected_index: usize,
    dropdown_open: bool,
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

struct DesktopSnapshot {
    bitmap: HBITMAP,
    width: i32,
    height: i32,
}

impl DesktopSnapshot {
    fn capture(bounds: ScreenRect) -> Result<Self> {
        let width = i32::try_from(bounds.width).map_err(|_| {
            AppError::CaptureFailed("desktop snapshot width exceeds Win32 limits".to_owned())
        })?;
        let height = i32::try_from(bounds.height).map_err(|_| {
            AppError::CaptureFailed("desktop snapshot height exceeds Win32 limits".to_owned())
        })?;
        if width <= 0 || height <= 0 {
            return Err(AppError::CaptureFailed(
                "desktop snapshot bounds are empty".to_owned(),
            ));
        }

        // SAFETY: A null HWND requests the virtual desktop DC.
        let screen_dc = unsafe { GetDC(ptr::null_mut()) };
        if screen_dc.is_null() {
            return Err(AppError::Windows {
                operation: "GetDC(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        let screen_dc = ScreenDc(screen_dc);

        // SAFETY: screen_dc is valid for the lifetime of this function.
        let memory_dc = unsafe { CreateCompatibleDC(screen_dc.0) };
        if memory_dc.is_null() {
            return Err(AppError::Windows {
                operation: "CreateCompatibleDC(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        let memory_dc = MemoryDc(memory_dc);

        // SAFETY: screen_dc is valid and dimensions were checked.
        let bitmap = unsafe { CreateCompatibleBitmap(screen_dc.0, width, height) };
        if bitmap.is_null() {
            return Err(AppError::Windows {
                operation: "CreateCompatibleBitmap(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        let bitmap = OwnedBitmap(bitmap);

        // SAFETY: Both handles are valid GDI objects.
        let old_bitmap = unsafe { SelectObject(memory_dc.0, bitmap.0) };
        if old_bitmap.is_null() {
            return Err(AppError::Windows {
                operation: "SelectObject(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        let mut selection = SelectedObject {
            dc: memory_dc.0,
            old: old_bitmap,
        };

        // SAFETY: Source and destination DCs are valid; the selected bitmap matches dimensions.
        let copied = unsafe {
            BitBlt(
                memory_dc.0,
                0,
                0,
                width,
                height,
                screen_dc.0,
                bounds.left,
                bounds.top,
                SRCCOPY | CAPTUREBLT,
            )
        };
        if copied == 0 {
            return Err(AppError::Windows {
                operation: "BitBlt(desktop snapshot)",
                win32_code: last_error(),
            });
        }
        selection.restore();
        Ok(Self {
            bitmap: bitmap.into_raw(),
            width,
            height,
        })
    }
}

impl Drop for DesktopSnapshot {
    fn drop(&mut self) {
        if !self.bitmap.is_null() {
            // SAFETY: This guard owns the bitmap created for the desktop snapshot.
            unsafe {
                DeleteObject(self.bitmap);
            }
        }
    }
}

struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: This guard owns a screen DC returned by GetDC(NULL).
        unsafe {
            ReleaseDC(ptr::null_mut(), self.0);
        }
    }
}

struct MemoryDc(HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: This guard owns a compatible memory DC.
        unsafe {
            DeleteDC(self.0);
        }
    }
}

struct OwnedBitmap(HBITMAP);

impl OwnedBitmap {
    fn into_raw(mut self) -> HBITMAP {
        let bitmap = self.0;
        self.0 = ptr::null_mut();
        bitmap
    }
}

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This guard owns a bitmap created by CreateCompatibleBitmap.
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

struct SelectedObject {
    dc: HDC,
    old: *mut core::ffi::c_void,
}

impl SelectedObject {
    fn restore(&mut self) {
        if !self.old.is_null() {
            // SAFETY: dc is valid and old is the object returned by SelectObject.
            unsafe {
                SelectObject(self.dc, self.old);
            }
            self.old = ptr::null_mut();
        }
    }
}

impl Drop for SelectedObject {
    fn drop(&mut self) {
        self.restore();
    }
}

const fn overlay_ex_style(use_snapshot_backdrop: bool) -> u32 {
    if use_snapshot_backdrop {
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW
    } else {
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW
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

struct ToolbarLayout {
    outer: RECT,
    more: RECT,
    provider: RECT,
    copy: RECT,
    cancel: RECT,
    ask: RECT,
    dropdown_bounds: RECT,
    dropdown_rects: Vec<RECT>,
}

fn toolbar_layout(
    client: &RECT,
    selection_rect: &RECT,
    provider_count: usize,
    toolbar_size: (i32, i32),
) -> ToolbarLayout {
    let (total_width, toolbar_height) = toolbar_size;
    let right = selection_rect
        .right
        .clamp(client.left + total_width + 8, client.right - 8);
    let left = right - total_width;
    let below = selection_rect.bottom + TOOLBAR_GAP;
    let above = selection_rect.top - toolbar_height - TOOLBAR_GAP;
    let dropdown_clearance = if provider_count > 1 {
        (provider_count as i32 * DROPDOWN_ROW_HEIGHT + 12).min(180)
    } else {
        0
    };
    let preferred_top = if below + toolbar_height + dropdown_clearance <= client.bottom - 8 {
        below
    } else {
        above.max(client.top + 8)
    };
    let min_top = client.top + 8;
    let max_top = (client.bottom - toolbar_height - 8).max(min_top);
    let top = preferred_top.clamp(min_top, max_top);
    let outer = RECT {
        left,
        top,
        right: left + total_width,
        bottom: top + toolbar_height,
    };
    let button_top = top + 7;
    let button_height = 32;
    let more = RECT {
        left: left + TOOLBAR_PADDING,
        top: button_top,
        right: left + TOOLBAR_PADDING + MORE_WIDTH,
        bottom: button_top + button_height,
    };
    let provider = offset_rect(&more, MORE_WIDTH + BUTTON_GAP, PROVIDER_WIDTH);
    let copy = offset_rect(&provider, PROVIDER_WIDTH + BUTTON_GAP, COPY_WIDTH);
    let cancel = offset_rect(&copy, COPY_WIDTH + BUTTON_GAP, CANCEL_WIDTH);
    let ask = offset_rect(&cancel, CANCEL_WIDTH + BUTTON_GAP, ASK_WIDTH);
    let dropdown_top =
        if outer.bottom + DROPDOWN_ROW_HEIGHT * provider_count as i32 <= client.bottom - 8 {
            outer.bottom + 4
        } else {
            outer.top - DROPDOWN_ROW_HEIGHT * provider_count as i32 - 4
        };
    let dropdown_bounds = RECT {
        left: provider.left,
        top: dropdown_top,
        right: provider.right,
        bottom: dropdown_top + DROPDOWN_ROW_HEIGHT * provider_count as i32,
    };
    let dropdown_rects = (0..provider_count)
        .map(|index| RECT {
            left: dropdown_bounds.left,
            top: dropdown_bounds.top + DROPDOWN_ROW_HEIGHT * index as i32,
            right: dropdown_bounds.right,
            bottom: dropdown_bounds.top + DROPDOWN_ROW_HEIGHT * (index as i32 + 1),
        })
        .collect::<Vec<_>>();
    ToolbarLayout {
        outer,
        more,
        provider,
        copy,
        cancel,
        ask,
        dropdown_bounds,
        dropdown_rects,
    }
}

fn offset_rect(previous: &RECT, delta_x: i32, width: i32) -> RECT {
    RECT {
        left: previous.left + delta_x,
        top: previous.top,
        right: previous.left + delta_x + width,
        bottom: previous.bottom,
    }
}

fn inset_rect(rect: &RECT, value: i32) -> RECT {
    RECT {
        left: rect.left + value,
        top: rect.top + value,
        right: rect.right - value,
        bottom: rect.bottom - value,
    }
}

fn hit_dropdown(rects: &[RECT], point: (i32, i32)) -> Option<usize> {
    rects.iter().position(|rect| point_in_rect(point, rect))
}

fn point_in_rect(point: (i32, i32), rect: &RECT) -> bool {
    point.0 >= rect.left && point.0 < rect.right && point.1 >= rect.top && point.1 < rect.bottom
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

fn draw_toolbar_antialiased(gdi: &GdiPlusSession, layout: &ToolbarLayout, toolbar: &ToolbarState) {
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
    layout: &ToolbarLayout,
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

fn fallback_toolbar_size() -> (i32, i32) {
    (
        TOOLBAR_PADDING * 2
            + MORE_WIDTH
            + PROVIDER_WIDTH
            + COPY_WIDTH
            + CANCEL_WIDTH
            + ASK_WIDTH
            + BUTTON_GAP * 4,
        toolbar_webview::preferred_size().1,
    )
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

fn selection_handle_points(rect: &RECT) -> [(i32, i32); 8] {
    let mid_x = rect.left + (rect.right - rect.left) / 2;
    let mid_y = rect.top + (rect.bottom - rect.top) / 2;
    [
        (rect.left, rect.top),
        (mid_x, rect.top),
        (rect.right, rect.top),
        (rect.right, mid_y),
        (rect.right, rect.bottom),
        (mid_x, rect.bottom),
        (rect.left, rect.bottom),
        (rect.left, mid_y),
    ]
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

struct GdiPlusSession {
    token: usize,
    graphics: *mut core::ffi::c_void,
}

impl GdiPlusSession {
    unsafe fn start(hdc: *mut core::ffi::c_void) -> Option<Self> {
        let input = GdiplusStartupInput {
            gdiplus_version: 1,
            debug_event_callback: ptr::null_mut(),
            suppress_background_thread: 0,
            suppress_external_codecs: 0,
        };
        let mut token = 0usize;
        if unsafe { GdiplusStartup(&mut token, &input, ptr::null_mut()) } != GDIP_OK {
            return None;
        }
        let mut graphics = ptr::null_mut();
        if unsafe { GdipCreateFromHDC(hdc, &mut graphics) } != GDIP_OK || graphics.is_null() {
            unsafe {
                GdiplusShutdown(token);
            }
            return None;
        }
        unsafe {
            GdipSetSmoothingMode(graphics, GDIP_SMOOTHING_ANTIALIAS);
            GdipSetPixelOffsetMode(graphics, GDIP_PIXEL_OFFSET_HALF);
        }
        Some(Self { token, graphics })
    }

    fn rounded_rect_rect(
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
    fn rounded_rect(
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

    fn line(&self, x1: f32, y1: f32, x2: f32, y2: f32, color: u32, width: f32) {
        unsafe {
            if let Some(pen) = self.pen(color, width) {
                GdipDrawLine(self.graphics, pen, x1, y1, x2, y2);
                GdipDeletePen(pen);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ellipse(&self, x: f32, y: f32, width: f32, height: f32, fill: u32, stroke: u32, sw: f32) {
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

impl Drop for GdiPlusSession {
    fn drop(&mut self) {
        unsafe {
            if !self.graphics.is_null() {
                GdipDeleteGraphics(self.graphics);
            }
            if self.token != 0 {
                GdiplusShutdown(self.token);
            }
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

const fn argb(alpha: u8, red: u8, green: u8, blue: u8) -> u32 {
    ((alpha as u32) << 24) | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
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

    #[test]
    fn toolbar_flips_above_when_provider_menu_would_be_clipped() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        let selection = RECT {
            left: 120,
            top: 100,
            right: 1120,
            bottom: 650,
        };

        let layout = toolbar_layout(&client, &selection, 4, toolbar_webview::preferred_size());

        assert_eq!(layout.outer.bottom, selection.top - TOOLBAR_GAP);
    }

    #[test]
    fn toolbar_right_edge_tracks_selection_right_edge() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 1440,
            bottom: 900,
        };
        let selection = RECT {
            left: 180,
            top: 120,
            right: 1180,
            bottom: 640,
        };

        let layout = toolbar_layout(&client, &selection, 4, toolbar_webview::preferred_size());

        assert_eq!(layout.outer.right, selection.right);
    }
}
