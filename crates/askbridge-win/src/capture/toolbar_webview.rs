use std::{num::NonZeroIsize, ptr};

use askbridge_core::{AppError, Result};
use serde::Serialize;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::UpdateWindow,
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
            GetClientRect, GetWindowLongPtrW, HWND_TOP, PostMessageW, RegisterClassW,
            SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_CLOSE,
            WM_NCCREATE, WM_NCDESTROY, WNDCLASSW, WS_CHILD, WS_VISIBLE,
        },
    },
};
use wry::{
    Rect, WebView, WebViewBuilder,
    dpi::{PhysicalPosition, PhysicalSize},
    raw_window_handle::{
        HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle,
        WindowHandle as BorrowedWindowHandle,
    },
};

use crate::util::{last_error, wide};

pub const ACTION_MESSAGE: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 6;
pub const ACTION_COPY: usize = 1;
pub const ACTION_CANCEL: usize = 2;
pub const ACTION_ASK: usize = 3;
pub const ACTION_MENU_OPEN: usize = 4;
pub const ACTION_MENU_CLOSE: usize = 5;

const CLASS_NAME: &str = "AskBridge.CaptureToolbar.WebView.v1";
const TOOLBAR_LOGICAL_WIDTH: i32 = 466;
const TOOLBAR_LOGICAL_HEIGHT: i32 = 68;

#[derive(Clone)]
pub struct ToolbarProvider {
    pub id: String,
    pub display_name: String,
    pub selected: bool,
}

pub struct ToolbarWebView {
    webview: WebView,
    window: OwnedWindow,
    closed_rect: RECT,
    menu_height: i32,
    menu_above: bool,
}

pub fn preferred_size() -> (i32, i32) {
    (TOOLBAR_LOGICAL_WIDTH, TOOLBAR_LOGICAL_HEIGHT)
}

pub fn preferred_size_for_window(window: HWND) -> (i32, i32) {
    let scale = dpi_scale_for_window(window);
    (
        scaled_dimension(TOOLBAR_LOGICAL_WIDTH, scale),
        scaled_dimension(TOOLBAR_LOGICAL_HEIGHT, scale),
    )
}

pub fn show(
    instance: HINSTANCE,
    overlay_window: HWND,
    rect: &RECT,
    providers: Vec<ToolbarProvider>,
) -> Result<ToolbarWebView> {
    register_class(instance)?;
    let width = (rect.right - rect.left).max(1);
    let height = (rect.bottom - rect.top).max(1);
    let (menu_height, menu_above) = menu_geometry(overlay_window, rect, providers.len());
    let window =
        create_toolbar_window(instance, overlay_window, rect.left, rect.top, width, height)?;
    let host = WebViewHost(window.0);
    let overlay_target = overlay_window as isize;
    let provider_ids = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>();
    let html = toolbar_html(&providers, menu_height, menu_above)?;
    let webview = WebViewBuilder::new()
        .with_transparent(true)
        .with_focused(false)
        .with_bounds(Rect {
            position: PhysicalPosition::new(0, 0).into(),
            size: PhysicalSize::new(width as u32, height as u32).into(),
        })
        .with_html(html)
        .with_ipc_handler(move |request| {
            let overlay = overlay_target as HWND;
            let body = request.body();
            if body == "copy" {
                post_action(overlay, ACTION_COPY, 0);
            } else if body == "cancel" {
                post_action(overlay, ACTION_CANCEL, 0);
            } else if body == "menu:open" {
                post_action(overlay, ACTION_MENU_OPEN, 0);
            } else if body == "menu:close" {
                post_action(overlay, ACTION_MENU_CLOSE, 0);
            } else if let Some(index) = body
                .strip_prefix("ask:")
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|index| *index < provider_ids.len())
            {
                post_action(overlay, ACTION_ASK, index as isize);
            }
        })
        .build_as_child(&host)
        .map_err(|error| AppError::CaptureFailed(format!("toolbar webview failed: {error}")))?;
    // SAFETY: window and its child WebView are fully initialized and sized.
    unsafe {
        SetWindowPos(
            window.0,
            HWND_TOP,
            rect.left,
            rect.top,
            width,
            height,
            SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        UpdateWindow(window.0);
    }
    Ok(ToolbarWebView {
        webview,
        window,
        closed_rect: *rect,
        menu_height,
        menu_above,
    })
}

impl ToolbarWebView {
    pub fn set_menu_open(&self, open: bool) -> Result<()> {
        let width = self.closed_rect.right - self.closed_rect.left;
        let closed_height = self.closed_rect.bottom - self.closed_rect.top;
        let height = closed_height + if open { self.menu_height } else { 0 };
        let top = if open && self.menu_above {
            self.closed_rect.top - self.menu_height
        } else {
            self.closed_rect.top
        };
        // SAFETY: the host is a live child of the overlay and all coordinates are client-relative.
        unsafe {
            SetWindowPos(
                self.window.0,
                HWND_TOP,
                self.closed_rect.left,
                top,
                width,
                height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
        }
        self.webview
            .set_bounds(Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: PhysicalSize::new(width as u32, height as u32).into(),
            })
            .map_err(|error| AppError::CaptureFailed(format!("toolbar resize failed: {error}")))
    }
}

fn menu_geometry(overlay_window: HWND, rect: &RECT, provider_count: usize) -> (i32, bool) {
    let desired = (provider_count.clamp(1, 8) as i32 * 38 + 12).min(316);
    let mut client = RECT::default();
    // SAFETY: overlay_window is live and client is writable for the synchronous call.
    unsafe {
        GetClientRect(overlay_window, &mut client);
    }
    let above = (rect.top - client.top - 8).max(0);
    let below = (client.bottom - rect.bottom - 8).max(0);
    let menu_above = above >= desired || above > below;
    let available = if menu_above { above } else { below };
    (desired.min(available.max(76)), menu_above)
}

fn register_class(instance: HINSTANCE) -> Result<()> {
    let class_name = wide(CLASS_NAME);
    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(toolbar_window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: ptr::null_mut(),
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    // SAFETY: All pointers are valid for the synchronous class registration call.
    if unsafe { RegisterClassW(&class) } == 0 {
        let code = last_error();
        if code != 1410 {
            return Err(AppError::Windows {
                operation: "RegisterClassW(capture toolbar webview)",
                win32_code: code,
            });
        }
    }
    Ok(())
}

fn create_toolbar_window(
    instance: HINSTANCE,
    owner: HWND,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
) -> Result<OwnedWindow> {
    let class_name = wide(CLASS_NAME);
    let title = wide("AskBridge 截图工具条");
    // SAFETY: The class is registered and all string pointers live through the call.
    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            left,
            top,
            width,
            height,
            owner,
            ptr::null_mut(),
            instance,
            owner,
        )
    };
    if window.is_null() {
        return Err(AppError::Windows {
            operation: "CreateWindowExW(capture toolbar webview)",
            win32_code: last_error(),
        });
    }
    Ok(OwnedWindow(window))
}

fn dpi_scale_for_window(window: HWND) -> f64 {
    // SAFETY: window is a live HWND while the overlay is selecting.
    let dpi = unsafe { GetDpiForWindow(window) };
    if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
}

fn scaled_dimension(value: i32, scale: f64) -> i32 {
    ((value as f64 * scale).round() as i32).max(value)
}

fn post_action(overlay: HWND, action: usize, value: isize) {
    // SAFETY: Posting to a window on the same UI thread; failure is non-fatal because the
    // overlay can still be cancelled via Esc/right-click.
    unsafe {
        PostMessageW(overlay, ACTION_MESSAGE, action, value);
    }
}

#[derive(Serialize)]
struct ProviderPayload<'a> {
    name: &'a str,
    selected: bool,
}

fn toolbar_html(
    providers: &[ToolbarProvider],
    menu_height: i32,
    menu_above: bool,
) -> Result<String> {
    let payload = providers
        .iter()
        .map(|provider| ProviderPayload {
            name: &provider.display_name,
            selected: provider.selected,
        })
        .collect::<Vec<_>>();
    let providers_json = serde_json::to_string(&payload)
        .map_err(|error| AppError::CaptureFailed(format!("toolbar payload failed: {error}")))?;
    let menu_top = if menu_above {
        4
    } else {
        TOOLBAR_LOGICAL_HEIGHT - 1
    };
    let menu_content_height = (menu_height - 8).max(68);
    let above_bar_rule = if menu_above {
        format!("body.menu-open .bar {{ top: {}px; }}", menu_height + 7)
    } else {
        String::new()
    };
    Ok(format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<style>
* {{ box-sizing: border-box; }}
html, body {{
  width: 100%;
  height: 100%;
  margin: 0;
  overflow: hidden;
  background: transparent;
  font-family: "Segoe UI Variable Text", "Microsoft YaHei UI", "Segoe UI", sans-serif;
  color: #18181b;
  user-select: none;
}}
body {{ position: relative; }}
.bar {{
  position: absolute;
  left: 7px;
  right: 7px;
  top: 7px;
  height: 50px;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 5px 7px;
  border: 1px solid rgba(24, 24, 27, .12);
  border-radius: 14px;
  background: rgba(255, 255, 255, .985);
  box-shadow: 0 5px 18px rgba(0, 0, 0, .22), 0 1px 3px rgba(0, 0, 0, .10);
}}
{above_bar_rule}
button {{
  height: 38px;
  border: 0;
  border-radius: 9px;
  background: transparent;
  color: #18181b;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  padding: 0 14px;
  font: 400 15px "Segoe UI Variable Text", "Microsoft YaHei UI", "Segoe UI", sans-serif;
  letter-spacing: 0;
  white-space: nowrap;
  cursor: default;
}}
button:hover {{ background: #f4f4f5; }}
button:active {{ background: #e9e9ec; }}
.copy {{ width: 92px; }}
.cancel {{ width: 88px; }}
.divider {{ width: 1px; height: 24px; margin: 0 5px; background: #e4e4e7; }}
.ask-wrap {{ position: relative; display: flex; flex: 1; min-width: 0; height: 38px; }}
.ask {{ width: 100%; justify-content: flex-start; padding-left: 16px; padding-right: 42px; overflow: hidden; }}
.ask-label {{ overflow: hidden; text-overflow: ellipsis; }}
.provider-toggle {{ position: absolute; right: 0; top: 0; width: 40px; padding: 0; border-radius: 8px; }}
.chevron {{ width: 16px; height: 16px; transition: transform 120ms ease; }}
body.menu-open .chevron {{ transform: rotate(180deg); }}
.icon {{ width: 19px; height: 19px; flex: 0 0 19px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }}
.provider-menu {{
  position: absolute;
  right: 7px;
  top: {menu_top}px;
  width: 248px;
  max-height: {menu_content_height}px;
  display: none;
  overflow-y: auto;
  padding: 5px;
  border: 1px solid rgba(24, 24, 27, .12);
  border-radius: 12px;
  background: rgba(255, 255, 255, .99);
  box-shadow: 0 8px 24px rgba(0, 0, 0, .22), 0 1px 3px rgba(0, 0, 0, .10);
}}
body.menu-open .provider-menu {{ display: block; }}
.provider-option {{ width: 100%; height: 36px; justify-content: flex-start; padding: 0 10px; gap: 9px; font-size: 14px; }}
.provider-option.selected {{ background: #f1f1f3; font-weight: 500; }}
.provider-check {{ width: 17px; height: 17px; opacity: 0; }}
.provider-option.selected .provider-check {{ opacity: 1; }}
</style>
</head>
<body>
<div class="bar">
  <button class="copy" id="copy" title="复制截图">
    <svg class="icon" viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="12" rx="2"></rect><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"></path></svg>
    <span>复制</span>
  </button>
  <button class="cancel" id="cancel" title="取消截图 (Esc)">
    <svg class="icon" viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"></path></svg>
    <span>取消</span>
  </button>
  <div class="divider"></div>
  <div class="ask-wrap">
    <button class="ask" id="ask" title="用当前模型提问 (Enter)">
      <svg class="icon" viewBox="0 0 24 24"><path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"></path><path d="M8 10h8M8 14h5"></path></svg>
      <span class="ask-label" id="ask-label">问问</span>
    </button>
    <button class="provider-toggle" id="provider-toggle" title="切换模型" aria-label="切换模型" aria-expanded="false">
      <svg class="chevron" viewBox="0 0 24 24"><path d="m7 10 5 5 5-5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"></path></svg>
    </button>
  </div>
</div>
<div class="provider-menu" id="provider-menu" role="menu"></div>
<script>
const providers = {providers_json};
let selectedIndex = Math.max(0, providers.findIndex(provider => provider.selected));
let menuOpen = false;
const askLabel = document.getElementById('ask-label');
const menu = document.getElementById('provider-menu');
const toggle = document.getElementById('provider-toggle');
function selectedName() {{
  const provider = providers[selectedIndex] || providers[0];
  return provider ? provider.name : "";
}}
function updateAsk() {{
  const name = selectedName();
  askLabel.textContent = name ? `问问 ${{name}}` : "问问";
}}
function renderMenu() {{
  menu.replaceChildren();
  providers.forEach((provider, index) => {{
    const option = document.createElement('button');
    option.className = `provider-option${{index === selectedIndex ? ' selected' : ''}}`;
    option.type = 'button';
    option.role = 'menuitemradio';
    option.setAttribute('aria-checked', String(index === selectedIndex));
    option.innerHTML = '<svg class="provider-check icon" viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"></path></svg><span></span>';
    option.querySelector('span').textContent = provider.name;
    option.addEventListener('click', () => {{
      selectedIndex = index;
      updateAsk();
      renderMenu();
      setMenuOpen(false);
    }});
    menu.appendChild(option);
  }});
}}
function setMenuOpen(open) {{
  if (menuOpen === open) return;
  menuOpen = open;
  document.body.classList.toggle('menu-open', open);
  toggle.setAttribute('aria-expanded', String(open));
  window.ipc.postMessage(open ? 'menu:open' : 'menu:close');
}}
document.getElementById('copy').addEventListener('click', () => window.ipc.postMessage('copy'));
document.getElementById('cancel').addEventListener('click', () => window.ipc.postMessage('cancel'));
document.getElementById('ask').addEventListener('click', () => window.ipc.postMessage(`ask:${{selectedIndex}}`));
toggle.addEventListener('click', event => {{
  event.stopPropagation();
  setMenuOpen(!menuOpen);
}});
document.addEventListener('keydown', event => {{
  if (event.key === 'Escape') {{
    event.preventDefault();
    window.ipc.postMessage('cancel');
  }}
  if (event.key === 'Enter') {{
    event.preventDefault();
    window.ipc.postMessage(`ask:${{selectedIndex}}`);
  }}
}});
document.addEventListener('contextmenu', event => event.preventDefault());
updateAsk();
renderMenu();
</script>
</body>
</html>"#
    ))
}

struct OwnedWindow(HWND);

impl Drop for OwnedWindow {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This guard owns the toolbar host window.
            unsafe {
                ShowWindow(self.0, windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE);
                DestroyWindow(self.0);
            }
        }
    }
}

struct WebViewHost(HWND);

impl HasWindowHandle for WebViewHost {
    fn window_handle(&self) -> std::result::Result<BorrowedWindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?;
        let handle = Win32WindowHandle::new(hwnd);
        // SAFETY: The borrowed handle is valid while the owned toolbar window is alive.
        Ok(unsafe { BorrowedWindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

unsafe extern "system" fn toolbar_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: lparam points to CREATESTRUCTW for WM_NCCREATE.
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        // SAFETY: lpCreateParams contains the owner overlay HWND passed at creation.
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        return 1;
    }
    if message == WM_CLOSE {
        // SAFETY: GWLP_USERDATA holds the owner overlay HWND while this window is alive.
        let overlay = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as HWND };
        if !overlay.is_null() {
            post_action(overlay, ACTION_CANCEL, 0);
        }
        return 0;
    }
    if message == WM_NCDESTROY {
        // SAFETY: Clear stale user data before the default teardown.
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        }
    }
    // SAFETY: Unhandled messages use the default window procedure.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_html_contains_selected_provider() {
        let html = toolbar_html(
            &[
                ToolbarProvider {
                    id: "chatgpt".to_owned(),
                    display_name: "ChatGPT".to_owned(),
                    selected: false,
                },
                ToolbarProvider {
                    id: "doubao".to_owned(),
                    display_name: "豆包".to_owned(),
                    selected: true,
                },
            ],
            88,
            true,
        )
        .expect("toolbar html");

        assert!(html.contains("ChatGPT"));
        assert!(html.contains("豆包"));
        assert!(html.contains("window.ipc.postMessage"));
        assert!(html.contains("provider-menu"));
        assert!(html.contains("取消截图 (Esc)"));
        assert!(html.contains("用当前模型提问 (Enter)"));
        assert!(html.contains("event.key === 'Escape'"));
        assert!(html.contains("event.key === 'Enter'"));
        assert!(!html.contains("event.key === 'Escape' && menuOpen"));
        assert!(!html.contains("event.key === 'Enter' && !menuOpen"));
        assert!(!html.contains("<select"));
    }
}
