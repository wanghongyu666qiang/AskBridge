//! The settings window: a native host frame with one WebView2 child page.
//!
//! The native side owns the frame, DPI, and lifetime; the page owns layout and
//! interaction. Commands flow back as the same `WM_COMMAND` IDs the runtime
//! already handles, so `app/commands.rs` needs no knowledge of the page.

use std::{
    cell::RefCell,
    ffi::c_void,
    path::{Path, PathBuf},
    ptr,
    rc::Rc,
};

use askbridge_core::{AppConfig, AppError, Result};
use tracing::{info, warn};
use windows_sys::Win32::{
    Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
        DwmSetWindowAttribute,
    },
    UI::{
        HiDpi::AdjustWindowRectExForDpi,
        WindowsAndMessaging::{
            CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
            GetClientRect, PostMessageW, SW_HIDE, SW_SHOW, SetForegroundWindow, SetWindowLongPtrW,
            SetWindowTextW, ShowWindow, WM_CLOSE, WM_COMMAND, WM_NCCREATE, WM_NCDESTROY,
            WS_CAPTION, WS_CLIPCHILDREN, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
        },
    },
};

use wry::{
    Rect, WebContext, WebView, WebViewBuilder,
    dpi::{PhysicalPosition, PhysicalSize},
    raw_window_handle::{
        HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
    },
};

use crate::{
    adapter::ProviderHealthReport,
    single_instance::{MAIN_WINDOW_CLASS, MAIN_WINDOW_TITLE},
    util::{last_error, wide},
};

use self::{
    dpi::UiScale,
    draft::SettingsDraft,
    html::settings_html,
    view_model::{SettingsViewModel, health_status},
};

pub const SETTINGS_CLASS: &str = "AskBridgeSettingsWindow";

mod dpi;
mod draft;
mod html;
mod view_model;

/// Control IDs the runtime already understands. The page posts these verbatim.
pub const CONTROL_APPLY: u16 = 2051;
pub const CONTROL_RESTORE_DEFAULTS: u16 = 2052;
pub const CONTROL_CLOSE: u16 = 2053;
pub const CONTROL_OPEN_BROWSER: u16 = 2054;
pub const CONTROL_CHECK_BROWSER: u16 = 2055;
pub const CONTROL_OPEN_LOGIN: u16 = 2056;
pub const CONTROL_CHECK_PROVIDERS: u16 = 2057;

const WINDOW_TITLE: &str = "AskBridge 设置";
const WINDOW_WIDTH: i32 = 900;
const WINDOW_HEIGHT: i32 = 660;
/// The page background, kept in sync with `--bg` in `html.rs` and the DWM
/// caption color so the title bar and the page read as one surface.
const PAGE_BACKGROUND: COLORREF = rgb(251, 249, 246);
const BORDER_COLOR: COLORREF = rgb(232, 227, 219);

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

/// The window background; the runtime registers the class with this color so
/// the frame never flashes a system color while the page loads.
pub const fn window_background_color() -> COLORREF {
    PAGE_BACKGROUND
}

/// Everything the page and the host share.
///
/// All of this runs on the UI thread: the IPC handler is invoked from the
/// WebView2 message pump, which this thread's message loop drives.
struct SettingsState {
    /// Created on first `show`, so startup never pays for a WebView2
    /// environment the user may never open.
    webview: Option<WebView>,
    /// `WebContext` must outlive the WebView it created.
    context: Option<Box<WebContext>>,
    /// The draft last posted by the page, consumed when the runtime applies.
    draft: Option<SettingsDraft>,
    /// The page has loaded and can accept `evaluate_script` calls.
    ready: bool,
    status: String,
    status_tone: &'static str,
    health: Vec<ProviderHealthReport>,
    /// Latest serialized view model, so a page that finishes loading after a
    /// refresh still receives the current configuration.
    view_model: String,
}

pub struct SettingsWindow {
    window: HWND,
    state: Rc<RefCell<SettingsState>>,
    data_root: PathBuf,
    config: AppConfig,
}

impl SettingsWindow {
    pub fn create(
        parent: HWND,
        instance: HINSTANCE,
        config: &AppConfig,
        data_root: &Path,
    ) -> Result<Self> {
        let scale = UiScale::system();
        // The page is laid out in CSS pixels, so the *client* area must be
        // exactly the logical size once the window's DPI scaling is applied.
        let (client_width, client_height) = scale.size(WINDOW_WIDTH, WINDOW_HEIGHT);
        let (width, height) = window_size_for_client(client_width, client_height, scale.dpi());
        let class_name = wide(SETTINGS_CLASS);
        let title = wide(WINDOW_TITLE);
        // The page starts empty until the first show builds it, so the state
        // pointer is not needed at creation time.
        // SAFETY: The class is registered by the runtime and both string
        // buffers live through the call. The window starts hidden.
        let window = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
                180,
                120,
                width,
                height,
                parent,
                ptr::null_mut(),
                instance,
                ptr::null_mut(),
            )
        };
        if window.is_null() {
            return Err(AppError::Windows {
                operation: "CreateWindowExW(settings window)",
                win32_code: last_error(),
            });
        }
        apply_dwm_frame(window);

        let view_model = SettingsViewModel::build(
            config,
            data_root,
            "准备就绪。所有设置在校验后统一应用。",
            &[],
        )?;
        Ok(Self {
            window,
            state: Rc::new(RefCell::new(SettingsState {
                webview: None,
                context: None,
                draft: None,
                ready: false,
                status: "准备就绪。所有设置在校验后统一应用。".to_owned(),
                status_tone: "",
                health: Vec::new(),
                view_model: view_model.to_json()?,
            })),
            data_root: data_root.to_path_buf(),
            config: config.clone(),
        })
    }

    pub const fn hwnd(&self) -> HWND {
        self.window
    }

    /// Shows the window, building the page on first use.
    ///
    /// A page that cannot initialize is reported to the user instead of
    /// leaving an empty frame on screen: without WebView2 there is no settings
    /// UI at all, so silence would look like a hang.
    pub fn show(&mut self) {
        if let Err(error) = self.ensure_page() {
            warn!(
                error_kind = error.kind(),
                "settings page could not be created"
            );
            let message = format!(
                "设置页面无法打开：{error}\n\n\
                 AskBridge 的设置界面需要 Microsoft Edge WebView2 运行时。\n\
                 Windows 11 与多数 Windows 10 已内置；若缺失，请从微软官方下载安装后重试。\n\n\
                 在此期间仍可直接编辑配置文件：\n{}",
                self.data_root.join("config.json").display()
            );
            crate::util::show_error("AskBridge 设置无法打开", &message);
            return;
        }
        // SAFETY: window is owned by this object and remains valid.
        unsafe {
            ShowWindow(self.window, SW_SHOW);
            SetForegroundWindow(self.window);
        }
    }

    pub fn hide(&self) {
        // SAFETY: window is owned by this object and remains valid.
        unsafe {
            ShowWindow(self.window, SW_HIDE);
        }
    }

    /// Replaces the configuration the page renders.
    pub fn refresh(&mut self, config: &AppConfig) -> Result<()> {
        self.config = config.clone();
        self.rebuild_view_model()?;
        self.push_view_model()
    }

    /// Hands the page's pending draft to the runtime. Returns `None` when the
    /// page has not posted one, which the runtime treats as "nothing to apply".
    pub fn take_draft(&self) -> Option<SettingsDraft> {
        self.state.borrow_mut().draft.take()
    }

    pub fn set_status(&self, message: &str) {
        {
            let mut state = self.state.borrow_mut();
            state.status = message.to_owned();
            state.status_tone = "";
        }
        self.evaluate(format!(
            "window.applyStatus?.({}, \"\");",
            js_string_literal(message)
        ));
    }

    /// Reports the outcome of an apply so the page can show its toast.
    pub fn report_apply(&self, result: std::result::Result<&str, &str>) {
        let (ok, message) = match result {
            Ok(message) => (true, message),
            Err(message) => (false, message),
        };
        {
            let mut state = self.state.borrow_mut();
            state.status = message.to_owned();
            state.status_tone = if ok { "ok" } else { "error" };
        }
        info!(ok, "settings apply result");
        self.evaluate(format!(
            "window.applyResult?.({ok}, {});",
            js_string_literal(message)
        ));
    }

    /// Updates the provider health badges.
    pub fn set_provider_health(&self, reports: &[ProviderHealthReport]) {
        self.state.borrow_mut().health = reports.to_vec();
        self.evaluate(format!("window.applyHealth?.({});", health_json(reports)));
    }

    /// Runs a script in the page once it is loaded. Calls made before the page
    /// finishes loading are dropped; the page hydrates from the inline view
    /// model, so nothing is lost.
    fn evaluate(&self, script: String) {
        let state = self.state.borrow();
        if !state.ready {
            return;
        }
        let Some(webview) = &state.webview else {
            return;
        };
        if let Err(error) = webview.evaluate_script(&script) {
            warn!(
                error_kind = "webview_eval",
                "settings page script could not run: {error}"
            );
        }
    }

    fn ensure_page(&mut self) -> Result<()> {
        if self.state.borrow().webview.is_some() {
            return Ok(());
        }
        let html = settings_html(&self.state.borrow().view_model)?;
        let mut context = Box::new(WebContext::new(Some(self.webview_data_dir())));
        let state = Rc::clone(&self.state);
        let host = HostWindow(self.window);
        let scale = UiScale::for_window(self.window);
        let (width, height) = scale.size(WINDOW_WIDTH, WINDOW_HEIGHT);
        let webview = WebViewBuilder::new_with_web_context(&mut context)
            .with_html(html)
            .with_bounds(Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: PhysicalSize::new(width as u32, height as u32).into(),
            })
            // The page is fully self-contained: navigating away from it is
            // either a bug or an injection attempt. `with_html` loads the
            // document through `NavigateToString`, which itself reports
            // `about:blank`, so that one origin must stay allowed.
            // The page is fully self-contained: navigating away from it is
            // either a bug or an injection attempt. `with_html` loads the
            // document as an inline `data:` URL, so only that origin stays
            // allowed; anything else (including links and redirects) is
            // cancelled.
            .with_navigation_handler(|url| is_inline_document(&url))
            .with_devtools(cfg!(debug_assertions))
            .with_ipc_handler(move |request| {
                if let Err(error) = handle_page_message(&state, request.body()) {
                    warn!(
                        error_kind = error.kind(),
                        "settings page message could not be handled"
                    );
                }
            })
            .build_as_child(&host)
            .map_err(|error| {
                AppError::ConfigurationInvalid(format!(
                    "设置页面无法初始化（需要 Microsoft Edge WebView2 运行时）：{error}"
                ))
            })?;
        {
            let mut state = self.state.borrow_mut();
            state.webview = Some(webview);
            state.context = Some(context);
        }
        // Creating the WebView2 child re-creates the host's window text, so
        // the title has to be restored afterwards. It is the identity the
        // accessibility tree and the UI acceptance script look the window up
        // by, so it must not stay empty.
        let title = wide(WINDOW_TITLE);
        // SAFETY: window is live and title is a nul-terminated UTF-16 buffer.
        unsafe {
            SetWindowTextW(self.window, title.as_ptr());
        }
        self.resize_page();
        Ok(())
    }

    fn webview_data_dir(&self) -> PathBuf {
        self.data_root.join("webview")
    }

    fn rebuild_view_model(&self) -> Result<()> {
        let state = self.state.borrow();
        let view_model =
            SettingsViewModel::build(&self.config, &self.data_root, &state.status, &state.health)?;
        let json = view_model.to_json()?;
        drop(state);
        self.state.borrow_mut().view_model = json;
        Ok(())
    }

    fn push_view_model(&self) -> Result<()> {
        let payload = self.state.borrow().view_model.clone();
        self.evaluate(format!("window.hydrate?.({payload});"));
        Ok(())
    }

    /// Keeps the WebView2 surface filling the host client area. The window is
    /// fixed-size, so this only matters on the initial layout and after a DPI
    /// change.
    fn resize_page(&self) {
        let state = self.state.borrow();
        let Some(webview) = &state.webview else {
            return;
        };
        let mut client = RECT::default();
        // SAFETY: window is live and client points to writable storage.
        unsafe {
            GetClientRect(self.window, &mut client);
        }
        let width = (client.right - client.left).max(1) as u32;
        let height = (client.bottom - client.top).max(1) as u32;
        if let Err(error) = webview.set_bounds(Rect {
            position: PhysicalPosition::new(0, 0).into(),
            size: PhysicalSize::new(width, height).into(),
        }) {
            warn!(
                error_kind = "webview_bounds",
                "settings page could not be sized: {error}"
            );
        }
    }
}

impl Drop for SettingsWindow {
    fn drop(&mut self) {
        // The WebView must be released before the window it is parented to,
        // and the WebContext after the WebView.
        let mut state = self.state.borrow_mut();
        state.webview = None;
        state.context = None;
        drop(state);
        if !self.window.is_null() {
            // SAFETY: This object owns the top-level window and drops it on its UI thread.
            unsafe {
                DestroyWindow(self.window);
            }
            self.window = ptr::null_mut();
        }
    }
}

/// Converts a desired client size into the window size that produces it.
///
/// The settings page is laid out in CSS pixels, so the client area has to be
/// exactly the logical size; without this the caption and borders would eat
/// into the page and clip its right-hand column.
fn window_size_for_client(client_width: i32, client_height: i32, dpi: u32) -> (i32, i32) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_width,
        bottom: client_height,
    };
    // SAFETY: rect is writable and the style matches the window created below.
    let adjusted = unsafe {
        AdjustWindowRectExForDpi(
            &mut rect,
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            0,
            0,
            dpi,
        )
    };
    if adjusted == 0 {
        return (client_width, client_height);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// Rounds the frame and merges the caption with the page surface. Every
/// attribute is a no-op on Windows 10, where the frame keeps the system look.
fn apply_dwm_frame(window: HWND) {
    let corner = DWMWCP_ROUND;
    let caption = PAGE_BACKGROUND;
    let border = BORDER_COLOR;
    // SAFETY: Each call passes a pointer to a live local of the documented size.
    unsafe {
        for (attribute, value, size) in [
            (
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                &corner as *const _ as *const c_void,
                std::mem::size_of_val(&corner) as u32,
            ),
            (
                DWMWA_CAPTION_COLOR as u32,
                &caption as *const _ as *const c_void,
                std::mem::size_of_val(&caption) as u32,
            ),
            (
                DWMWA_BORDER_COLOR as u32,
                &border as *const _ as *const c_void,
                std::mem::size_of_val(&border) as u32,
            ),
        ] {
            let result = DwmSetWindowAttribute(window, attribute, value, size);
            if result < 0 {
                // Windows 10 rejects these; the frame just stays square, which
                // is the intended fallback.
                info!(
                    hresult = format!("0x{:08X}", result as u32),
                    "DWM frame attribute unavailable on this Windows build"
                );
            }
        }
    }
}

/// Whether a navigation target is the inline document itself.
///
/// `WebViewBuilder::with_html` hands the page to WebView2 as a `data:` URL,
/// so the initial load and any same-document reload carry that scheme. Every
/// other URL is external and must be refused.
fn is_inline_document(url: &str) -> bool {
    url.starts_with("data:text/html") || url == "about:blank"
}

/// A JS string literal for interpolation into an `evaluate_script` call.
///
/// `serde_json` already produces a valid JS string literal, so this delegates
/// instead of hand-escaping, which is the usual source of script injection.
/// `<` is additionally escaped so a value can never terminate an inline
/// `<script>` block if it is ever embedded in markup.
fn js_string_literal(value: &str) -> String {
    serde_json::to_string(value)
        .map(|literal| literal.replace('<', "\\u003c"))
        .unwrap_or_else(|_| "\"\"".to_owned())
}

fn health_json(reports: &[ProviderHealthReport]) -> String {
    let payload = reports
        .iter()
        .map(|report| {
            let (health, label) = health_status(report.health);
            serde_json::json!({
                "provider_id": report.provider_id,
                "health": health,
                "health_label": label,
            })
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(payload).to_string()
}

/// Routes a page message either to the page state or to the runtime.
fn handle_page_message(state: &Rc<RefCell<SettingsState>>, body: &str) -> Result<()> {
    if body == "ready" {
        let mut state = state.borrow_mut();
        state.ready = true;
        // A refresh may have landed while the page was still loading.
        let payload = state.view_model.clone();
        let webview = state.webview.as_ref();
        if let Some(webview) = webview
            && let Err(error) = webview.evaluate_script(&format!("window.hydrate?.({payload});"))
        {
            warn!(
                error_kind = "webview_eval",
                "settings page could not hydrate after load: {error}"
            );
        }
        return Ok(());
    }
    if body == "close" {
        forward_command(CONTROL_CLOSE);
        return Ok(());
    }
    let command = match body {
        "restore_defaults" => CONTROL_RESTORE_DEFAULTS,
        "check_providers" => CONTROL_CHECK_PROVIDERS,
        "open_browser" => CONTROL_OPEN_BROWSER,
        "open_login" => CONTROL_OPEN_LOGIN,
        _ => {
            // Only the messages the page actually sends are meaningful; a
            // stray body is ignored rather than logged, so a malformed page
            // cannot flood the log.
            let Some(payload) = body.strip_prefix("apply:") else {
                return Ok(());
            };
            let draft = serde_json::from_str::<SettingsDraft>(payload).map_err(|error| {
                AppError::ConfigurationInvalid(format!("设置草稿格式无效：{error}"))
            })?;
            state.borrow_mut().draft = Some(draft);
            CONTROL_APPLY
        }
    };
    forward_command(command);
    Ok(())
}

/// Forwards a control ID to the hidden runtime window, which owns the
/// configuration, hotkeys, and browser. The settings window never applies
/// anything itself.
fn forward_command(command: u16) {
    let class = wide(MAIN_WINDOW_CLASS);
    let title = wide(MAIN_WINDOW_TITLE);
    // SAFETY: Both search strings are valid nul-terminated UTF-16 buffers.
    let main_window = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(class.as_ptr(), title.as_ptr())
    };
    if main_window.is_null() {
        warn!(
            stage = "settings_command_dispatch",
            completed = false,
            "the runtime window was not found; settings command dropped"
        );
        return;
    }
    // SAFETY: WM_COMMAND targets our own UI thread and carries only control data.
    unsafe {
        if PostMessageW(main_window, WM_COMMAND, command as WPARAM, 0) == 0 {
            warn!(
                stage = "settings_command_dispatch",
                completed = false,
                win32_code = last_error(),
                "failed to forward a settings command to the runtime"
            );
        }
    }
}

struct HostWindow(HWND);

impl HasWindowHandle for HostWindow {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        let hwnd = std::num::NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?;
        let handle = Win32WindowHandle::new(hwnd);
        // SAFETY: The borrowed handle is valid while the owned settings window is alive.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

pub unsafe extern "system" fn settings_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: lparam points to CREATESTRUCTW for WM_NCCREATE.
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        // SAFETY: lpCreateParams carries the settings state pointer, if any.
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        return 1;
    }
    if message == WM_NCDESTROY {
        // SAFETY: Clear the borrowed state pointer before default teardown.
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            return DefWindowProcW(window, message, wparam, lparam);
        }
    }
    match message {
        WM_CLOSE => {
            // SAFETY: window is the live settings window receiving this close request.
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
            0
        }
        _ => {
            // SAFETY: Unhandled messages are forwarded exactly as received.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_background_matches_the_css_page_color() {
        // COLORREF is 0x00BBGGRR, so #fbf9f6 becomes 0x00f6f9fb.
        assert_eq!(PAGE_BACKGROUND, 0x00f6f9fb);
    }

    #[test]
    fn control_ids_stay_in_the_range_the_runtime_handles() {
        for id in [
            CONTROL_APPLY,
            CONTROL_RESTORE_DEFAULTS,
            CONTROL_CLOSE,
            CONTROL_OPEN_BROWSER,
            CONTROL_CHECK_BROWSER,
            CONTROL_OPEN_LOGIN,
            CONTROL_CHECK_PROVIDERS,
        ] {
            assert!((2051..=2057).contains(&id));
        }
    }

    #[test]
    fn js_string_literals_escape_quotes_and_newlines() {
        assert_eq!(js_string_literal("a\"b"), "\"a\\\"b\"");
        assert_eq!(js_string_literal("a\nb"), "\"a\\nb\"");
        assert_eq!(js_string_literal("设置已保存"), "\"设置已保存\"");
        // A script terminator must not survive as raw markup.
        assert!(!js_string_literal("</script>").contains("</script>"));
    }

    #[test]
    fn health_payload_uses_the_page_vocabulary() {
        let reports = vec![ProviderHealthReport {
            provider_id: "chatgpt".to_owned(),
            health: crate::adapter::ProviderHealth::ComposerMissing,
            page_accessible: true,
            logged_in: true,
            composer_found: false,
            attachment_supported: false,
            focus_supported: false,
        }];
        let json = health_json(&reports);
        assert!(json.contains("\"composer_missing\""));
        assert!(json.contains("\"provider_id\":\"chatgpt\""));
    }

    #[test]
    fn only_the_inline_document_may_navigate() {
        assert!(is_inline_document(
            "data:text/html;charset=utf-8;base64,PCFkb2N0eXBl"
        ));
        assert!(is_inline_document("about:blank"));
        for external in [
            "https://example.com/",
            "http://127.0.0.1:8080/",
            "file:///C:/Windows/notepad.exe",
            "javascript:alert(1)",
            "data:image/png;base64,AAAA",
            "about:blank#fragment",
        ] {
            assert!(
                !is_inline_document(external),
                "navigation to {external} must be refused"
            );
        }
    }

    #[test]
    fn class_and_title_are_a_stable_contract_with_the_ui_script() {
        assert_eq!(SETTINGS_CLASS, "AskBridgeSettingsWindow");
        assert_eq!(WINDOW_TITLE, "AskBridge 设置");
    }
}
