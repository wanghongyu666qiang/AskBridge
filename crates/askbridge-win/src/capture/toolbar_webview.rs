use std::{num::NonZeroIsize, ptr};

use askbridge_core::{AppError, Result};
use serde::Serialize;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{CreateRectRgn, DeleteObject, SetWindowRgn, UpdateWindow},
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA,
            GetClientRect, GetWindowLongPtrW, HWND_TOP, PostMessageW, RegisterClassW,
            SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_CLOSE,
            WM_NCCREATE, WM_NCDESTROY, WNDCLASSW, WS_CHILD,
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
const TOOLBAR_LOGICAL_WIDTH: i32 = 536;
const TOOLBAR_LOGICAL_HEIGHT: i32 = 68;
const MENU_ROW_LOGICAL_HEIGHT: i32 = 32;
const MENU_VISIBLE_ROWS: usize = 5;
const MENU_VERTICAL_CHROME: i32 = 10;
const MENU_MIN_LOGICAL_HEIGHT: i32 = MENU_ROW_LOGICAL_HEIGHT + MENU_VERTICAL_CHROME;

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
    let menu_geometry = menu_geometry(overlay_window, rect, providers.len());
    let frame = toolbar_host_frame(
        rect,
        menu_geometry.physical_height,
        menu_geometry.above,
        false,
    );
    let width = (frame.right - frame.left).max(1);
    let height = (frame.bottom - frame.top).max(1);
    let window = create_toolbar_window(
        instance,
        overlay_window,
        frame.left,
        frame.top,
        width,
        height,
    )?;
    let host = WebViewHost(window.0);
    let overlay_target = overlay_window as isize;
    let provider_ids = providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>();
    let html = toolbar_html(
        &providers,
        menu_geometry.logical_height,
        menu_geometry.above,
    )?;
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
        set_toolbar_clip(
            window.0,
            rect,
            menu_geometry.physical_height,
            menu_geometry.above,
            false,
        )?;
        SetWindowPos(
            window.0,
            HWND_TOP,
            frame.left,
            frame.top,
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
        menu_height: menu_geometry.physical_height,
        menu_above: menu_geometry.above,
    })
}

impl ToolbarWebView {
    pub fn set_menu_open(&self, open: bool) -> Result<()> {
        if !open {
            self.update_menu_visual(false)?;
        }
        set_toolbar_clip(
            self.window.0,
            &self.closed_rect,
            self.menu_height,
            self.menu_above,
            open,
        )?;
        if open {
            self.update_menu_visual(true)?;
        }
        Ok(())
    }

    fn update_menu_visual(&self, open: bool) -> Result<()> {
        self.webview
            .evaluate_script(if open {
                "window.applyMenuOpen?.(true);"
            } else {
                "window.applyMenuOpen?.(false);"
            })
            .map_err(|error| {
                AppError::CaptureFailed(format!("toolbar menu visual update failed: {error}"))
            })
    }
}

fn toolbar_host_frame(closed: &RECT, menu_height: i32, menu_above: bool, _open: bool) -> RECT {
    RECT {
        left: closed.left,
        top: if menu_above {
            closed.top - menu_height
        } else {
            closed.top
        },
        right: closed.right,
        bottom: if menu_above {
            closed.bottom
        } else {
            closed.bottom + menu_height
        },
    }
}

fn toolbar_clip_rect(closed: &RECT, menu_height: i32, menu_above: bool, open: bool) -> RECT {
    let width = (closed.right - closed.left).max(1);
    let closed_height = (closed.bottom - closed.top).max(1);
    let full_height = closed_height + menu_height;
    if open {
        RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: full_height,
        }
    } else if menu_above {
        RECT {
            left: 0,
            top: menu_height,
            right: width,
            bottom: full_height,
        }
    } else {
        RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: closed_height,
        }
    }
}

fn set_toolbar_clip(
    window: HWND,
    closed: &RECT,
    menu_height: i32,
    menu_above: bool,
    open: bool,
) -> Result<()> {
    let clip = toolbar_clip_rect(closed, menu_height, menu_above, open);
    // SAFETY: the region is created with finite client coordinates and ownership transfers to
    // the live toolbar window only when SetWindowRgn succeeds.
    unsafe {
        let region = CreateRectRgn(clip.left, clip.top, clip.right, clip.bottom);
        if region.is_null() {
            return Err(AppError::Windows {
                operation: "CreateRectRgn(capture toolbar clip)",
                win32_code: last_error(),
            });
        }
        if SetWindowRgn(window, region, 1) == 0 {
            DeleteObject(region);
            return Err(AppError::Windows {
                operation: "SetWindowRgn(capture toolbar clip)",
                win32_code: last_error(),
            });
        }
    }
    Ok(())
}

struct MenuGeometry {
    physical_height: i32,
    logical_height: i32,
    above: bool,
}

fn menu_geometry(overlay_window: HWND, rect: &RECT, provider_count: usize) -> MenuGeometry {
    let scale = dpi_scale_for_window(overlay_window);
    let desired_logical = menu_logical_height(provider_count);
    let desired = scaled_dimension(desired_logical, scale);
    let minimum = scaled_dimension(MENU_MIN_LOGICAL_HEIGHT, scale);
    let mut client = RECT::default();
    // SAFETY: overlay_window is live and client is writable for the synchronous call.
    unsafe {
        GetClientRect(overlay_window, &mut client);
    }
    let above = (rect.top - client.top - 8).max(0);
    let below = (client.bottom - rect.bottom - 8).max(0);
    let menu_above = above >= desired || above > below;
    let available = if menu_above { above } else { below };
    let physical_height = desired.min(available.max(minimum));
    MenuGeometry {
        physical_height,
        logical_height: logical_dimension(physical_height, scale),
        above: menu_above,
    }
}

fn menu_logical_height(provider_count: usize) -> i32 {
    let rows = provider_count.clamp(1, MENU_VISIBLE_ROWS) as i32;
    (rows * MENU_ROW_LOGICAL_HEIGHT + MENU_VERTICAL_CHROME).max(MENU_MIN_LOGICAL_HEIGHT)
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
            WS_CHILD,
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

fn logical_dimension(value: i32, scale: f64) -> i32 {
    ((value as f64 / scale).round() as i32).max(1)
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
    let bar_top = if menu_above { menu_height + 7 } else { 7 };
    let menu_placement_rule = if menu_above {
        "top: auto; bottom: 44px;"
    } else {
        "top: 44px; bottom: auto;"
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
  color: #f5f5f7;
  user-select: none;
}}
body {{ position: relative; }}
.bar {{
  position: absolute;
  left: 7px;
  right: 7px;
  top: {bar_top}px;
  height: 50px;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 5px 7px;
  border: 1px solid rgba(255, 255, 255, .16);
  border-radius: 18px;
  background: #1c1c1f;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, .08), 0 8px 24px rgba(0, 0, 0, .34), 0 2px 6px rgba(0, 0, 0, .24);
}}
button {{
  height: 40px;
  border: 0;
  border-radius: 11px;
  background: transparent;
  color: #f5f5f7;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 0 13px;
  font: 400 14px "Segoe UI Variable Text", "Microsoft YaHei UI", "Segoe UI", sans-serif;
  letter-spacing: 0;
  white-space: nowrap;
  cursor: default;
}}
button:hover {{ background: rgba(255, 255, 255, .09); }}
button:active {{ background: rgba(255, 255, 255, .14); transform: translateY(1px); }}
button:focus-visible {{ outline: 2px solid rgba(226, 109, 69, .92); outline-offset: -2px; }}
.copy {{ width: 88px; }}
.cancel {{ width: 84px; }}
.divider {{ width: 1px; height: 26px; margin: 0 4px; background: rgba(255, 255, 255, .14); }}
.provider-picker {{ position: relative; width: 188px; height: 40px; flex: 0 0 188px; }}
.provider-toggle {{
  width: 100%;
  justify-content: flex-start;
  padding-left: 13px;
  padding-right: 12px;
  border: 1px solid rgba(255, 255, 255, .11);
  background: rgba(255, 255, 255, .055);
}}
.provider-toggle:hover {{ background: rgba(255, 255, 255, .10); }}
.provider-label {{ flex: 1; overflow: hidden; text-align: left; text-overflow: ellipsis; }}
.ask {{
  width: 112px;
  margin-left: 2px;
  background: #993c1d;
  color: #fffaf6;
  font-weight: 600;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, .16), 0 2px 7px rgba(0, 0, 0, .22);
}}
.ask:hover {{ background: #ad4928; }}
.ask:active {{ background: #8c3419; }}
.chevron {{ width: 16px; height: 16px; transition: transform 120ms ease; }}
body.menu-open .chevron {{ transform: rotate(180deg); }}
.icon {{ width: 19px; height: 19px; flex: 0 0 19px; fill: none; stroke: currentColor; stroke-width: 1.75; stroke-linecap: round; stroke-linejoin: round; }}
.provider-menu {{
  position: absolute;
  left: 0;
  {menu_placement_rule}
  width: 188px;
  max-height: {menu_height}px;
  display: none;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 4px;
  border: 1px solid rgba(255, 255, 255, .15);
  border-radius: 12px;
  background: #1c1c1f;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, .07), 0 10px 28px rgba(0, 0, 0, .38), 0 2px 6px rgba(0, 0, 0, .22);
  color-scheme: dark;
  scrollbar-color: rgba(255, 255, 255, .24) transparent;
  scrollbar-width: thin;
  scrollbar-gutter: stable;
}}
.provider-menu::-webkit-scrollbar {{ width: 8px; }}
.provider-menu::-webkit-scrollbar-thumb {{ border: 2px solid transparent; border-radius: 999px; background: rgba(255, 255, 255, .22); background-clip: padding-box; }}
body.menu-open .provider-menu {{ display: block; }}
.provider-option {{ width: 100%; height: 32px; justify-content: flex-start; padding: 0 9px; gap: 8px; border-radius: 8px; font-size: 13.5px; }}
.provider-option:hover {{ background: rgba(255, 255, 255, .075); }}
.provider-option.selected {{ background: rgba(255, 255, 255, .105); font-weight: 500; }}
.provider-option.selected:hover {{ background: rgba(255, 255, 255, .13); }}
.provider-check {{ width: 16px; height: 16px; margin-left: auto; opacity: 0; }}
.provider-option.selected .provider-check {{ opacity: 1; color: #e77a54; }}
.provider-option span {{ overflow: hidden; text-overflow: ellipsis; }}
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
  <div class="provider-picker">
    <button class="provider-toggle" id="provider-toggle" title="切换模型" aria-label="切换模型" aria-haspopup="menu" aria-controls="provider-menu" aria-expanded="false">
      <svg class="icon" viewBox="0 0 24 24"><path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"></path><path d="M9 10h6M9 14h4"></path></svg>
      <span class="provider-label" id="provider-label"></span>
      <svg class="chevron" viewBox="0 0 24 24"><path d="m7 10 5 5 5-5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"></path></svg>
    </button>
    <div class="provider-menu" id="provider-menu" role="menu"></div>
  </div>
  <button class="ask" id="ask" title="用当前模型提问 (Enter)">
    <svg class="icon" viewBox="0 0 24 24"><path d="M12 3v4M12 17v4M3 12h4M17 12h4"></path><path d="m7.5 7.5 2 2M14.5 14.5l2 2M16.5 7.5l-2 2M9.5 14.5l-2 2"></path></svg>
    <span>问问</span>
  </button>
</div>
<script>
const providers = {providers_json};
let selectedIndex = Math.max(0, providers.findIndex(provider => provider.selected));
let menuOpen = false;
let menuRequestedOpen = false;
const providerLabel = document.getElementById('provider-label');
const menu = document.getElementById('provider-menu');
const toggle = document.getElementById('provider-toggle');
function selectedName() {{
  const provider = providers[selectedIndex] || providers[0];
  return provider ? provider.name : "";
}}
function updateProvider() {{
  const name = selectedName();
  providerLabel.textContent = name || "选择模型";
}}
function renderMenu() {{
  menu.replaceChildren();
  providers.forEach((provider, index) => {{
    const option = document.createElement('button');
    option.className = `provider-option${{index === selectedIndex ? ' selected' : ''}}`;
    option.type = 'button';
    option.role = 'menuitemradio';
    option.setAttribute('aria-checked', String(index === selectedIndex));
    option.innerHTML = '<span></span><svg class="provider-check icon" viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"></path></svg>';
    option.querySelector('span').textContent = provider.name;
    option.addEventListener('click', () => {{
      selectedIndex = index;
      updateProvider();
      renderMenu();
      requestMenuOpen(false);
    }});
    menu.appendChild(option);
  }});
}}
function requestMenuOpen(open) {{
  if (menuRequestedOpen === open) return;
  menuRequestedOpen = open;
  if (open) {{
    window.ipc.postMessage('menu:open');
  }} else {{
    applyMenuOpen(false);
    requestAnimationFrame(() => window.ipc.postMessage('menu:close'));
  }}
}}
function applyMenuOpen(open) {{
  menuOpen = open;
  menuRequestedOpen = open;
  document.body.classList.toggle('menu-open', open);
  toggle.setAttribute('aria-expanded', String(open));
  if (open) {{
    requestAnimationFrame(() => menu.querySelector('.selected')?.scrollIntoView({{ block: 'nearest' }}));
  }}
}}
window.applyMenuOpen = applyMenuOpen;
document.getElementById('copy').addEventListener('click', () => window.ipc.postMessage('copy'));
document.getElementById('cancel').addEventListener('click', () => window.ipc.postMessage('cancel'));
document.getElementById('ask').addEventListener('click', () => window.ipc.postMessage(`ask:${{selectedIndex}}`));
toggle.addEventListener('click', event => {{
  event.stopPropagation();
  requestMenuOpen(!menuRequestedOpen);
}});
document.addEventListener('click', event => {{
  if (menuOpen && !menu.contains(event.target)) requestMenuOpen(false);
}});
document.addEventListener('keydown', event => {{
  if (event.key === 'Escape') {{
    event.preventDefault();
    if (menuOpen) {{
      requestMenuOpen(false);
    }} else {{
      window.ipc.postMessage('cancel');
    }}
  }}
  if (event.key === 'Enter') {{
    event.preventDefault();
    window.ipc.postMessage(`ask:${{selectedIndex}}`);
  }}
}});
document.addEventListener('contextmenu', event => event.preventDefault());
updateProvider();
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
        assert!(html.contains("if (menuOpen)"));
        assert!(!html.contains("event.key === 'Enter' && !menuOpen"));
        assert!(!html.contains("<select"));
        assert!(html.contains("background: #1c1c1f"));
        assert!(html.contains("background: #993c1d"));
        assert!(html.contains("class=\"provider-picker\""));
        assert!(html.contains("left: 0;"));
        assert!(!html.contains("right: 140px"));
        assert!(html.contains("id=\"provider-label\""));
        assert!(!html.contains("id=\"ask-label\""));
    }

    #[test]
    fn provider_menu_caps_at_five_compact_rows() {
        assert_eq!(menu_logical_height(0), MENU_MIN_LOGICAL_HEIGHT);
        assert_eq!(menu_logical_height(1), 42);
        assert_eq!(menu_logical_height(5), 170);
        assert_eq!(menu_logical_height(12), 170);
        assert_eq!(logical_dimension(scaled_dimension(170, 1.5), 1.5), 170);
    }

    #[test]
    fn provider_menu_waits_for_native_clip_before_visual_open() {
        let html = toolbar_html(
            &[ToolbarProvider {
                id: "chatgpt".to_owned(),
                display_name: "ChatGPT".to_owned(),
                selected: true,
            }],
            170,
            true,
        )
        .expect("toolbar html");

        let resize_request = html
            .find("window.ipc.postMessage('menu:open')")
            .expect("native resize request");
        let visual_change = html
            .find("document.body.classList.toggle('menu-open', open)")
            .expect("visual menu change");
        assert!(
            resize_request < visual_change,
            "the host resize request must precede the visual open"
        );
        assert!(html.contains("window.applyMenuOpen"));
    }

    #[test]
    fn toolbar_surfaces_do_not_leak_background_content() {
        let html = toolbar_html(&[], 170, false).expect("toolbar html");

        assert!(html.matches("background: #1c1c1f;").count() >= 2);
        assert!(!html.contains("background: rgba(28, 28, 31, .975)"));
        assert!(!html.contains("background: rgba(28, 28, 31, .985)"));
        assert!(!html.contains("backdrop-filter"));
    }

    #[test]
    fn provider_menu_does_not_move_or_resize_the_webview_surface() {
        let closed = RECT {
            left: 300,
            top: 500,
            right: 836,
            bottom: 568,
        };

        for menu_above in [false, true] {
            let closed_frame = toolbar_host_frame(&closed, 170, menu_above, false);
            let open_frame = toolbar_host_frame(&closed, 170, menu_above, true);
            assert_eq!(
                (
                    closed_frame.left,
                    closed_frame.top,
                    closed_frame.right,
                    closed_frame.bottom,
                ),
                (
                    open_frame.left,
                    open_frame.top,
                    open_frame.right,
                    open_frame.bottom,
                ),
                "opening the menu must not move or resize the WebView surface"
            );
        }

        let closed_below = toolbar_clip_rect(&closed, 170, false, false);
        let closed_above = toolbar_clip_rect(&closed, 170, true, false);
        let open = toolbar_clip_rect(&closed, 170, true, true);
        assert_eq!(
            (
                closed_below.left,
                closed_below.top,
                closed_below.right,
                closed_below.bottom,
            ),
            (0, 0, 536, 68)
        );
        assert_eq!(
            (
                closed_above.left,
                closed_above.top,
                closed_above.right,
                closed_above.bottom,
            ),
            (0, 170, 536, 238)
        );
        assert_eq!(
            (open.left, open.top, open.right, open.bottom),
            (0, 0, 536, 238)
        );
    }

    #[test]
    fn upward_provider_menu_keeps_closed_toolbar_inside_its_clip() {
        let html = toolbar_html(
            &[ToolbarProvider {
                id: "chatgpt".to_owned(),
                display_name: "ChatGPT".to_owned(),
                selected: true,
            }],
            170,
            true,
        )
        .expect("toolbar html");
        let bar_rule = html
            .split_once(".bar {")
            .and_then(|(_, rest)| rest.split_once('}'))
            .map(|(rule, _)| rule)
            .expect("bar rule");

        assert!(
            bar_rule.contains("top: 177px;"),
            "the closed toolbar must stay inside the fixed host's visible clip"
        );
        assert!(!html.contains("body.menu-open .bar"));
    }
}
