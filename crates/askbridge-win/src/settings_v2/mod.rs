use std::{ffi::c_void, path::Path, ptr};

use askbridge_core::{
    AppCommand, AppConfig, AppError, BrowserLifecycle, BrowserTargetPreference, HotkeyBinding,
    HotkeyConfig, ProviderConfig, ProviderOverride, Result, provider::built_in_providers,
};
use tracing::error;
use windows_sys::Win32::{
    Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontW, CreateSolidBrush,
        DEFAULT_CHARSET, DEFAULT_GUI_FONT, DEFAULT_PITCH, DeleteObject, DrawTextW, FF_DONTCARE,
        FW_NORMAL, FW_SEMIBOLD, FillRect, FrameRect, GetStockObject, GetSysColorBrush,
        OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::{
        Controls::{
            BST_CHECKED, BST_UNCHECKED, DRAWITEMSTRUCT, EM_SETLIMITTEXT, EM_SETMARGINS,
            ODS_DISABLED, ODS_SELECTED,
        },
        HiDpi::GetDpiForSystem,
        WindowsAndMessaging::{
            BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_OWNERDRAW,
            CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CBS_DROPDOWNLIST,
            CreateWindowExW, DefWindowProcW, DestroyWindow, EC_LEFTMARGIN, EC_RIGHTMARGIN,
            ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, ES_WANTRETURN, FindWindowW,
            GetDlgCtrlID, GetDlgItem, GetWindowTextLengthW, GetWindowTextW, IsChild,
            IsWindowVisible, PostMessageW, SW_HIDE, SW_SHOW, SendMessageW, SetForegroundWindow,
            SetWindowTextW, ShowWindow, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DRAWITEM,
            WM_GETFONT, WM_SETFONT, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
            WS_EX_CLIENTEDGE, WS_GROUP, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP,
            WS_VISIBLE, WS_VSCROLL,
        },
    },
};

use crate::{
    adapter::{ProviderHealth, ProviderHealthReport},
    single_instance::{MAIN_WINDOW_CLASS, MAIN_WINDOW_TITLE},
    util::{last_error, wide},
};

mod controls;
mod pages;
mod provider_config;

use controls::{
    combo_add, combo_reset, combo_select, combo_selection, create_button, create_control, get_text,
    is_checked, set_checked, set_font, set_text,
};
use pages::{
    create_browser_page, create_general_page, create_hotkey_page, create_page, create_provider_page,
};
use provider_config::{origin_pattern, parse_custom_providers};

pub const SETTINGS_CLASS: &str = "AskBridgeSettingsWindow";

const TAB_HOTKEYS: u16 = 2001;
const TAB_PROVIDERS: u16 = 2002;
const TAB_BROWSER: u16 = 2003;
const TAB_GENERAL: u16 = 2004;
const PAGE_HOTKEYS: u16 = 2011;
const PAGE_PROVIDERS: u16 = 2012;
const PAGE_BROWSER: u16 = 2013;
const PAGE_GENERAL: u16 = 2014;
const TITLE_LABEL: u16 = 2021;
const SUBTITLE_LABEL: u16 = 2022;

pub const CONTROL_APPLY: u16 = 2051;
pub const CONTROL_RESTORE_DEFAULTS: u16 = 2052;
pub const CONTROL_CLOSE: u16 = 2053;
pub const CONTROL_OPEN_BROWSER: u16 = 2054;
pub const CONTROL_CHECK_BROWSER: u16 = 2055;
pub const CONTROL_OPEN_LOGIN: u16 = 2056;
pub const CONTROL_CHECK_PROVIDERS: u16 = 2057;

const STATUS_LABEL: u16 = 2060;
const EDIT_CAPTURE: u16 = 2101;
const CHECK_CAPTURE: u16 = 2102;
const EDIT_QUICK: u16 = 2103;
const CHECK_QUICK: u16 = 2104;
const EDIT_TEXT: u16 = 2105;
const CHECK_TEXT: u16 = 2106;

const COMBO_DEFAULT_PROVIDER: u16 = 2201;
const CHECK_PROVIDER_BASE: u16 = 2210;
const EDIT_PROVIDER_URL_BASE: u16 = 2220;
const EDIT_CUSTOM_PROVIDERS: u16 = 2230;
const PROVIDER_HEALTH_BASE: u16 = 2240;

const RADIO_CHATGPT_DESKTOP_PWA: u16 = 2301;
const RADIO_CHATGPT_DEDICATED_CHROME: u16 = 2305;
const EDIT_CHROME_PATH: u16 = 2302;
const COMBO_LIFECYCLE: u16 = 2303;
const EDIT_DATA_PATH: u16 = 2304;

const EDIT_QUICK_PROMPT: u16 = 2401;
const CHECK_START_ON_LOGIN: u16 = 2402;
const CHECK_DEBUG_LOGGING: u16 = 2405;

const WINDOW_WIDTH: i32 = 840;
const WINDOW_HEIGHT: i32 = 720;
const MAX_SINGLE_LINE: WPARAM = 2048;
const MAX_MULTI_LINE: WPARAM = 16_384;

const COLOR_TEXT: COLORREF = rgb(30, 41, 59);
const COLOR_MUTED: COLORREF = rgb(100, 116, 139);
const COLOR_ACCENT: COLORREF = rgb(37, 99, 235);
const COLOR_ACCENT_PRESSED: COLORREF = rgb(29, 78, 216);
const COLOR_SECONDARY: COLORREF = rgb(241, 245, 249);
const COLOR_SECONDARY_PRESSED: COLORREF = rgb(226, 232, 240);
const COLOR_BORDER: COLORREF = rgb(203, 213, 225);
const COLOR_DISABLED: COLORREF = rgb(148, 163, 184);
const COLOR_WHITE: COLORREF = rgb(255, 255, 255);

const LIFECYCLES: [(&str, BrowserLifecycle); 4] = [
    ("按需启动，保持运行", BrowserLifecycle::OnDemandKeepRunning),
    (
        "按需启动，空闲十分钟关闭",
        BrowserLifecycle::OnDemandIdleClose,
    ),
    ("每次准备后询问关闭", BrowserLifecycle::CloseAfterDispatch),
    ("随 AskBridge 启动", BrowserLifecycle::OnStartup),
];

#[derive(Clone, Copy)]
struct UiScale {
    dpi: u32,
}

impl UiScale {
    fn system() -> Self {
        // SAFETY: The process selects Per-Monitor V2 awareness before creating settings UI.
        let dpi = unsafe { GetDpiForSystem() };
        Self {
            dpi: if dpi == 0 { 96 } else { dpi },
        }
    }

    fn px(self, value: i32) -> i32 {
        ((i64::from(value) * i64::from(self.dpi) + 48) / 96) as i32
    }
}

struct OwnedFont(*mut c_void);

impl OwnedFont {
    fn create(height: i32, weight: i32, scale: UiScale) -> Result<Self> {
        let family = wide("Microsoft YaHei UI");
        // SAFETY: All metrics are ordinary font attributes and family is nul-terminated.
        let font = unsafe {
            CreateFontW(
                -scale.px(height),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                family.as_ptr(),
            )
        };
        if font.is_null() {
            return Err(AppError::Windows {
                operation: "CreateFontW(settings)",
                win32_code: last_error(),
            });
        }
        Ok(Self(font))
    }

    const fn handle(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for OwnedFont {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: The top-level settings window is destroyed before the fonts are dropped.
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

struct UiFonts {
    title: OwnedFont,
    body: OwnedFont,
    label: OwnedFont,
    small: OwnedFont,
}

impl UiFonts {
    fn create(scale: UiScale) -> Result<Self> {
        Ok(Self {
            title: OwnedFont::create(24, FW_SEMIBOLD as i32, scale)?,
            body: OwnedFont::create(16, FW_NORMAL as i32, scale)?,
            label: OwnedFont::create(16, FW_SEMIBOLD as i32, scale)?,
            small: OwnedFont::create(14, FW_NORMAL as i32, scale)?,
        })
    }
}

struct HotkeyRow {
    command: AppCommand,
    edit: HWND,
    enabled: HWND,
}

struct ProviderRow {
    id: String,
    enabled: HWND,
    start_url: HWND,
    health: HWND,
}

pub struct SettingsWindow {
    window: HWND,
    rows: Vec<HotkeyRow>,
    provider_rows: Vec<ProviderRow>,
    provider_ids: Vec<String>,
    default_provider: HWND,
    custom_providers: HWND,
    chatgpt_desktop_pwa: HWND,
    chatgpt_dedicated_chrome: HWND,
    chrome_path: HWND,
    lifecycle: HWND,
    quick_prompt: HWND,
    start_on_login: HWND,
    debug_logging: HWND,
    status: HWND,
    _fonts: UiFonts,
}

impl SettingsWindow {
    pub fn create(
        parent: HWND,
        instance: HINSTANCE,
        config: &AppConfig,
        data_root: &Path,
    ) -> Result<Self> {
        let scale = UiScale::system();
        let fonts = UiFonts::create(scale)?;
        let window = create_control(
            parent,
            instance,
            scale,
            SETTINGS_CLASS,
            "AskBridge 设置",
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            180,
            120,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            0,
            0,
        )?;

        let title = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "AskBridge",
            WS_CHILD | WS_VISIBLE,
            24,
            18,
            180,
            32,
            0,
            TITLE_LABEL,
        )?;
        set_font(title, fonts.title.handle());
        let subtitle = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "轻量问答路由设置",
            WS_CHILD | WS_VISIBLE,
            24,
            50,
            240,
            24,
            0,
            SUBTITLE_LABEL,
        )?;
        set_font(subtitle, fonts.small.handle());

        for (index, (id, label)) in [
            (TAB_HOTKEYS, "快捷键"),
            (TAB_PROVIDERS, "供应商"),
            (TAB_BROWSER, "浏览器"),
            (TAB_GENERAL, "常规"),
        ]
        .into_iter()
        .enumerate()
        {
            let tab = create_control(
                window,
                instance,
                scale,
                "BUTTON",
                label,
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | if index == 0 { WS_GROUP } else { 0 }
                    | BS_AUTORADIOBUTTON as u32,
                282 + index as i32 * 122,
                34,
                112,
                30,
                0,
                id,
            )?;
            set_font(tab, fonts.label.handle());
        }

        let hotkey_page = create_page(window, instance, scale, PAGE_HOTKEYS)?;
        let provider_page = create_page(window, instance, scale, PAGE_PROVIDERS)?;
        let browser_page = create_page(window, instance, scale, PAGE_BROWSER)?;
        let general_page = create_page(window, instance, scale, PAGE_GENERAL)?;

        let rows = create_hotkey_page(hotkey_page, instance, scale, &fonts)?;
        let (default_provider, provider_rows, custom_providers) =
            create_provider_page(provider_page, instance, scale, &fonts)?;
        let (chatgpt_desktop_pwa, chatgpt_dedicated_chrome, chrome_path, lifecycle) =
            create_browser_page(browser_page, instance, scale, data_root, &fonts)?;
        let (quick_prompt, start_on_login, debug_logging) =
            create_general_page(general_page, instance, scale, &fonts)?;

        create_button(
            window,
            instance,
            scale,
            &fonts,
            "应用更改",
            494,
            604,
            104,
            CONTROL_APPLY,
        )?;
        create_button(
            window,
            instance,
            scale,
            &fonts,
            "恢复默认快捷键",
            610,
            604,
            132,
            CONTROL_RESTORE_DEFAULTS,
        )?;
        create_button(
            window,
            instance,
            scale,
            &fonts,
            "关闭",
            754,
            604,
            64,
            CONTROL_CLOSE,
        )?;
        let status = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "准备就绪。所有设置在校验后统一应用。",
            WS_CHILD | WS_VISIBLE,
            24,
            660,
            794,
            28,
            0,
            STATUS_LABEL,
        )?;
        set_font(status, fonts.small.handle());

        let mut settings = Self {
            window,
            rows,
            provider_rows,
            provider_ids: Vec::new(),
            default_provider,
            custom_providers,
            chatgpt_desktop_pwa,
            chatgpt_dedicated_chrome,
            chrome_path,
            lifecycle,
            quick_prompt,
            start_on_login,
            debug_logging,
            status,
            _fonts: fonts,
        };
        settings.refresh(config)?;
        switch_page(window, PAGE_HOTKEYS);
        Ok(settings)
    }

    pub const fn hwnd(&self) -> HWND {
        self.window
    }

    pub fn show(&self) {
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

    pub fn is_visible(&self) -> bool {
        // SAFETY: window is owned by this object.
        unsafe { IsWindowVisible(self.window) != 0 }
    }

    pub fn contains(&self, window: HWND) -> bool {
        window == self.window || {
            // SAFETY: both handles are UI handles on the same thread.
            unsafe { IsChild(self.window, window) != 0 }
        }
    }

    pub fn refresh(&mut self, config: &AppConfig) -> Result<()> {
        for row in &self.rows {
            let binding = config.hotkeys.binding(row.command);
            set_text(row.edit, &binding.to_string())?;
            set_checked(row.enabled, binding.enabled);
        }

        let providers = config.merged_providers()?;
        self.provider_ids.clear();
        combo_reset(self.default_provider);
        let mut selected = 0;
        for provider in &providers {
            let index = combo_add(self.default_provider, &provider.display_name)?;
            if provider.id == config.default_provider_id {
                selected = index;
            }
            self.provider_ids.push(provider.id.clone());
        }
        combo_select(self.default_provider, selected);

        for row in &self.provider_rows {
            let provider = providers
                .iter()
                .find(|provider| provider.id == row.id)
                .ok_or_else(|| AppError::InvalidProvider(row.id.clone()))?;
            set_checked(row.enabled, provider.enabled);
            set_text(row.start_url, &provider.start_url)?;
            set_text(row.health, "○ 未检测")?;
        }
        let custom_text = config
            .custom_providers
            .iter()
            .map(|provider| {
                format!(
                    "{} | {} | {} | {}",
                    provider.id,
                    provider.display_name,
                    provider.start_url,
                    provider.url_patterns.join(",")
                )
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        set_text(self.custom_providers, &custom_text)?;

        set_text(
            self.chrome_path,
            config.browser.chrome_path.as_deref().unwrap_or_default(),
        )?;
        set_checked(
            self.chatgpt_desktop_pwa,
            config.browser.target_preference("chatgpt") == BrowserTargetPreference::DesktopPwa,
        );
        set_checked(
            self.chatgpt_dedicated_chrome,
            config.browser.target_preference("chatgpt") == BrowserTargetPreference::DedicatedChrome,
        );
        combo_reset(self.lifecycle);
        let mut lifecycle_selected = 0;
        for (index, (label, value)) in LIFECYCLES.into_iter().enumerate() {
            combo_add(self.lifecycle, label)?;
            if config.browser.lifecycle == value {
                lifecycle_selected = index;
            }
        }
        combo_select(self.lifecycle, lifecycle_selected);

        set_text(self.quick_prompt, &config.quick_prompt)?;
        set_checked(self.start_on_login, config.general.start_on_login);
        set_checked(self.debug_logging, config.general.debug_logging);
        Ok(())
    }

    pub fn read_config(&self, base: &AppConfig) -> Result<AppConfig> {
        let mut candidate = base.clone();
        candidate.hotkeys = self.read_hotkeys()?;
        candidate.default_provider_id = self.read_default_provider()?;
        candidate.quick_prompt = get_text(self.quick_prompt)?.trim().to_owned();
        candidate.general.start_on_login = is_checked(self.start_on_login);
        candidate.general.debug_logging = is_checked(self.debug_logging);
        candidate.general.auto_submit = false;

        let chrome_path = get_text(self.chrome_path)?;
        candidate.browser.chrome_path =
            (!chrome_path.trim().is_empty()).then(|| chrome_path.trim().to_owned());
        candidate.browser.lifecycle = self.read_lifecycle()?;
        candidate.browser.target_preferences.insert(
            "chatgpt".to_owned(),
            if is_checked(self.chatgpt_desktop_pwa) {
                BrowserTargetPreference::DesktopPwa
            } else {
                BrowserTargetPreference::DedicatedChrome
            },
        );
        candidate.provider_overrides = self.read_provider_overrides(base)?;
        candidate.custom_providers = parse_custom_providers(&get_text(self.custom_providers)?)?;
        candidate.validate()?;
        Ok(candidate)
    }

    pub fn set_status(&self, message: &str) {
        let _ = set_text(self.status, message);
    }

    /// Updates the provider rows with the latest no-send capability results.
    pub fn set_provider_health(&self, reports: &[ProviderHealthReport]) {
        for row in &self.provider_rows {
            let Some(report) = reports.iter().find(|report| report.provider_id == row.id) else {
                continue;
            };
            let message = match report.health {
                ProviderHealth::Healthy => "✓ 页面/输入框/图片",
                ProviderHealth::LoginRequired => "⚠ 需要登录",
                ProviderHealth::ComposerMissing => "⚠ 输入框检测失败",
                ProviderHealth::AttachmentUnsupported => "⚠ 输入框正常；无图片",
                ProviderHealth::NetworkError => "✕ 页面访问失败",
            };
            let _ = set_text(row.health, message);
        }
    }

    fn read_hotkeys(&self) -> Result<HotkeyConfig> {
        let mut config = HotkeyConfig::default();
        for row in &self.rows {
            let text = get_text(row.edit)?;
            let mut binding = text
                .parse::<HotkeyBinding>()
                .map_err(|error| AppError::InvalidHotkey(format!("{text}: {error}")))?;
            binding.enabled = is_checked(row.enabled);
            *config.binding_mut(row.command) = binding;
        }
        config.validate()?;
        Ok(config)
    }

    fn read_default_provider(&self) -> Result<String> {
        let selected = combo_selection(self.default_provider)?;
        self.provider_ids.get(selected).cloned().ok_or_else(|| {
            AppError::InvalidProvider("selected default provider is unavailable".to_owned())
        })
    }

    fn read_lifecycle(&self) -> Result<BrowserLifecycle> {
        let selected = combo_selection(self.lifecycle)?;
        LIFECYCLES
            .get(selected)
            .map(|(_, value)| *value)
            .ok_or_else(|| {
                AppError::ConfigurationInvalid("browser lifecycle selection is invalid".to_owned())
            })
    }

    fn read_provider_overrides(&self, base: &AppConfig) -> Result<Vec<ProviderOverride>> {
        let defaults = built_in_providers();
        let current = base.merged_providers()?;
        let mut overrides = Vec::new();
        for row in &self.provider_rows {
            let original = defaults
                .iter()
                .find(|provider| provider.id == row.id)
                .ok_or_else(|| AppError::InvalidProvider(row.id.clone()))?;
            let existing = base
                .provider_overrides
                .iter()
                .find(|provider| provider.id == row.id);
            let previous = current
                .iter()
                .find(|provider| provider.id == row.id)
                .ok_or_else(|| AppError::InvalidProvider(row.id.clone()))?;
            let enabled = is_checked(row.enabled);
            let start_url = get_text(row.start_url)?.trim().to_owned();
            let url_patterns = if start_url == previous.start_url {
                previous.url_patterns.clone()
            } else {
                vec![origin_pattern(&start_url)?]
            };
            let verification = ProviderConfig {
                id: row.id.clone(),
                display_name: original.display_name.clone(),
                enabled,
                start_url: start_url.clone(),
                url_patterns: url_patterns.clone(),
                is_custom: false,
                adapter_override: original.adapter_override.clone(),
            };
            verification.validate()?;
            let provider_override = ProviderOverride {
                id: row.id.clone(),
                display_name: existing.and_then(|value| value.display_name.clone()),
                enabled: (enabled != original.enabled).then_some(enabled),
                start_url: (start_url != original.start_url).then_some(start_url),
                url_patterns: (url_patterns != original.url_patterns).then_some(url_patterns),
                adapter_override: existing.and_then(|value| value.adapter_override.clone()),
            };
            if provider_override.display_name.is_some()
                || provider_override.enabled.is_some()
                || provider_override.start_url.is_some()
                || provider_override.url_patterns.is_some()
                || provider_override.adapter_override.is_some()
            {
                overrides.push(provider_override);
            }
        }
        Ok(overrides)
    }
}

impl Drop for SettingsWindow {
    fn drop(&mut self) {
        if !self.window.is_null() {
            // SAFETY: This object owns the top-level window and drops it on its UI thread.
            unsafe {
                DestroyWindow(self.window);
            }
            self.window = ptr::null_mut();
        }
    }
}

fn switch_page(window: HWND, page: u16) {
    for (page_id, tab_id) in [
        (PAGE_HOTKEYS, TAB_HOTKEYS),
        (PAGE_PROVIDERS, TAB_PROVIDERS),
        (PAGE_BROWSER, TAB_BROWSER),
        (PAGE_GENERAL, TAB_GENERAL),
    ] {
        // SAFETY: window is the live outer settings window and IDs are its child controls.
        let page_window = unsafe { GetDlgItem(window, i32::from(page_id)) };
        if !page_window.is_null() {
            // SAFETY: page_window is a live child window.
            unsafe {
                ShowWindow(page_window, if page_id == page { SW_SHOW } else { SW_HIDE });
            }
        }
        // SAFETY: window is live and tab_id names an optional direct child.
        let tab = unsafe { GetDlgItem(window, i32::from(tab_id)) };
        if !tab.is_null() {
            set_checked(tab, page_id == page);
        }
    }
}

fn draw_owner_button(item: &DRAWITEMSTRUCT) -> LRESULT {
    let primary = item.CtlID == CONTROL_APPLY as u32
        || item.CtlID == CONTROL_OPEN_BROWSER as u32
        || item.CtlID == CONTROL_OPEN_LOGIN as u32;
    let disabled = item.itemState & ODS_DISABLED != 0;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let fill_color = if primary {
        if pressed {
            COLOR_ACCENT_PRESSED
        } else {
            COLOR_ACCENT
        }
    } else if pressed {
        COLOR_SECONDARY_PRESSED
    } else {
        COLOR_SECONDARY
    };
    let border_color = if primary { fill_color } else { COLOR_BORDER };
    let text_color = if disabled {
        COLOR_DISABLED
    } else if primary {
        COLOR_WHITE
    } else {
        COLOR_TEXT
    };

    // SAFETY: item contains the valid HDC and RECT supplied by WM_DRAWITEM.
    unsafe {
        let fill = CreateSolidBrush(fill_color);
        if !fill.is_null() {
            FillRect(item.hDC, &item.rcItem, fill);
            DeleteObject(fill);
        }

        let border = CreateSolidBrush(border_color);
        if !border.is_null() {
            FrameRect(item.hDC, &item.rcItem, border);
            DeleteObject(border);
        }

        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, text_color);

        let font = SendMessageW(item.hwndItem, WM_GETFONT, 0, 0) as *mut c_void;
        let previous_font = if font.is_null() {
            ptr::null_mut()
        } else {
            SelectObject(item.hDC, font)
        };

        let mut text = [0_u16; 96];
        let copied = GetWindowTextW(item.hwndItem, text.as_mut_ptr(), text.len() as i32);
        let mut text_rect: RECT = item.rcItem;
        DrawTextW(
            item.hDC,
            text.as_ptr(),
            copied,
            &mut text_rect,
            0x0000_0001 | 0x0000_0004 | 0x0000_0020,
        );

        if !previous_font.is_null() {
            SelectObject(item.hDC, previous_font);
        }
    }
    1
}

fn static_control_color(window: HWND, device_context: *mut c_void) -> LRESULT {
    // SAFETY: window and device_context are supplied by WM_CTLCOLORSTATIC for a live control.
    unsafe {
        SetBkMode(device_context, TRANSPARENT as i32);
        let id = GetDlgCtrlID(window);
        let color = match id as u16 {
            SUBTITLE_LABEL | STATUS_LABEL => COLOR_MUTED,
            _ => COLOR_TEXT,
        };
        SetTextColor(device_context, color);
        GetSysColorBrush(COLOR_WINDOW) as LRESULT
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

const fn make_lparam(low: u16, high: u16) -> LPARAM {
    (low as usize | ((high as usize) << 16)) as LPARAM
}

pub unsafe extern "system" fn settings_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND => {
            let command = (wparam & 0xffff) as u16;
            let page = match command {
                TAB_HOTKEYS => Some(PAGE_HOTKEYS),
                TAB_PROVIDERS => Some(PAGE_PROVIDERS),
                TAB_BROWSER => Some(PAGE_BROWSER),
                TAB_GENERAL => Some(PAGE_GENERAL),
                _ => None,
            };
            if let Some(page) = page {
                switch_page(window, page);
                if command != TAB_PROVIDERS {
                    return 0;
                }
            }
            let forwarded_wparam = if command == TAB_PROVIDERS {
                CONTROL_CHECK_PROVIDERS as WPARAM
            } else {
                wparam
            };
            let forwarded_lparam = if command == TAB_PROVIDERS { 0 } else { lparam };
            let class = wide(MAIN_WINDOW_CLASS);
            let title = wide(MAIN_WINDOW_TITLE);
            // SAFETY: Both search strings are valid nul-terminated UTF-16 buffers.
            let main_window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
            if !main_window.is_null() {
                // SAFETY: WM_COMMAND targets our own UI thread and carries only control data.
                if unsafe { PostMessageW(main_window, message, forwarded_wparam, forwarded_lparam) }
                    == 0
                {
                    error!(
                        stage = "settings_command_dispatch",
                        completed = false,
                        win32_code = last_error(),
                        "failed to forward a settings command to the runtime"
                    );
                }
            }
            0
        }
        WM_DRAWITEM => {
            if lparam == 0 {
                return 0;
            }
            // SAFETY: For WM_DRAWITEM, lparam points to a DRAWITEMSTRUCT valid for this call.
            let item = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            draw_owner_button(item)
        }
        WM_CTLCOLORSTATIC => static_control_color(lparam as HWND, wparam as *mut c_void),
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
    fn custom_provider_editor_requires_safe_complete_rows() {
        let providers = parse_custom_providers(
            "example | Example | https://example.com/chat | https://example.com/",
        )
        .expect("custom provider");
        assert_eq!(providers.len(), 1);
        assert!(providers[0].is_custom);
        assert!(parse_custom_providers("bad | Missing fields").is_err());
        assert!(
            parse_custom_providers("bad | Bad | javascript:alert(1) | https://example.com/")
                .is_err()
        );
    }

    #[test]
    fn built_in_url_edits_are_scoped_to_their_https_origin() {
        assert_eq!(
            origin_pattern("https://example.com/app/new?x=1").expect("origin"),
            "https://example.com/"
        );
        assert!(origin_pattern("http://example.com/").is_err());
    }

    #[test]
    fn settings_page_exposes_no_auto_submit_control() {
        let source = concat!(include_str!("mod.rs"), include_str!("pages.rs"));
        let forbidden_control = ["CHECK_", "AUTO_", "SUBMIT"].concat();
        assert!(!source.contains(&forbidden_control));
    }

    #[test]
    fn browser_page_exposes_explicit_chatgpt_target_choices() {
        let source = concat!(include_str!("mod.rs"), include_str!("pages.rs"));
        assert!(source.contains("桌面网页端：复用现有登录，但截图需要手动上传"));
        assert!(source.contains("AskBridge 专用 Chrome：支持自动上传图片，需要单独登录"));
        let forbidden = ["截图提问会", "自动使用 AskBridge 专用 Chrome"].concat();
        assert!(!source.contains(&forbidden));
    }
}
