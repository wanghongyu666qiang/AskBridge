use std::{path::Path, ptr};

use askbridge_core::{
    AppCommand, AppConfig, AppError, BrowserLifecycle, BrowserTargetPreference, HotkeyBinding,
    HotkeyConfig, ProviderConfig, ProviderOverride, Result, provider::built_in_providers,
};
use tracing::error;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, GetSysColorBrush},
    UI::{
        Controls::{BST_CHECKED, BST_UNCHECKED, EM_SETLIMITTEXT},
        HiDpi::GetDpiForSystem,
        WindowsAndMessaging::{
            BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, CB_ADDSTRING,
            CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CBS_DROPDOWNLIST, CreateWindowExW,
            DefWindowProcW, DestroyWindow, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE,
            ES_READONLY, ES_WANTRETURN, FindWindowW, GetDlgItem, GetWindowTextLengthW,
            GetWindowTextW, IsChild, IsWindowVisible, PostMessageW, SW_HIDE, SW_SHOW, SendMessageW,
            SetForegroundWindow, SetWindowTextW, ShowWindow, WM_CLOSE, WM_COMMAND,
            WM_CTLCOLORSTATIC, WM_SETFONT, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
            WS_EX_CLIENTEDGE, WS_GROUP, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP,
            WS_VISIBLE, WS_VSCROLL,
        },
    },
};

use crate::{
    single_instance::{MAIN_WINDOW_CLASS, MAIN_WINDOW_TITLE},
    util::{last_error, wide},
};

pub const SETTINGS_CLASS: &str = "AskBridgeSettingsWindow";

const TAB_HOTKEYS: u16 = 2001;
const TAB_PROVIDERS: u16 = 2002;
const TAB_BROWSER: u16 = 2003;
const TAB_GENERAL: u16 = 2004;
const PAGE_HOTKEYS: u16 = 2011;
const PAGE_PROVIDERS: u16 = 2012;
const PAGE_BROWSER: u16 = 2013;
const PAGE_GENERAL: u16 = 2014;

pub const CONTROL_APPLY: u16 = 2051;
pub const CONTROL_RESTORE_DEFAULTS: u16 = 2052;
pub const CONTROL_CLOSE: u16 = 2053;
pub const CONTROL_OPEN_BROWSER: u16 = 2054;
pub const CONTROL_CHECK_BROWSER: u16 = 2055;
pub const CONTROL_OPEN_LOGIN: u16 = 2056;

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

const CHECK_CHATGPT_DESKTOP_PWA: u16 = 2301;
const EDIT_CHROME_PATH: u16 = 2302;
const COMBO_LIFECYCLE: u16 = 2303;
const EDIT_DATA_PATH: u16 = 2304;

const EDIT_QUICK_PROMPT: u16 = 2401;
const CHECK_START_ON_LOGIN: u16 = 2402;
const CHECK_CLIPBOARD_FALLBACK: u16 = 2403;
const CHECK_HIDE_PROMPT: u16 = 2404;
const CHECK_DEBUG_LOGGING: u16 = 2405;

const WINDOW_WIDTH: i32 = 800;
const WINDOW_HEIGHT: i32 = 660;
const MAX_SINGLE_LINE: WPARAM = 2048;
const MAX_MULTI_LINE: WPARAM = 16_384;

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

struct HotkeyRow {
    command: AppCommand,
    edit: HWND,
    enabled: HWND,
}

struct ProviderRow {
    id: String,
    enabled: HWND,
    start_url: HWND,
}

pub struct SettingsWindow {
    window: HWND,
    rows: Vec<HotkeyRow>,
    provider_rows: Vec<ProviderRow>,
    provider_ids: Vec<String>,
    default_provider: HWND,
    custom_providers: HWND,
    chatgpt_desktop_pwa: HWND,
    chrome_path: HWND,
    lifecycle: HWND,
    quick_prompt: HWND,
    start_on_login: HWND,
    clipboard_fallback: HWND,
    hide_prompt_after_prepare: HWND,
    debug_logging: HWND,
    status: HWND,
}

impl SettingsWindow {
    pub fn create(
        parent: HWND,
        instance: HINSTANCE,
        config: &AppConfig,
        data_root: &Path,
    ) -> Result<Self> {
        let scale = UiScale::system();
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

        for (index, (id, label)) in [
            (TAB_HOTKEYS, "快捷键"),
            (TAB_PROVIDERS, "供应商"),
            (TAB_BROWSER, "浏览器"),
            (TAB_GENERAL, "常规"),
        ]
        .into_iter()
        .enumerate()
        {
            create_control(
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
                24 + index as i32 * 122,
                18,
                112,
                30,
                0,
                id,
            )?;
        }

        let hotkey_page = create_page(window, instance, scale, PAGE_HOTKEYS)?;
        let provider_page = create_page(window, instance, scale, PAGE_PROVIDERS)?;
        let browser_page = create_page(window, instance, scale, PAGE_BROWSER)?;
        let general_page = create_page(window, instance, scale, PAGE_GENERAL)?;

        let rows = create_hotkey_page(hotkey_page, instance, scale)?;
        let (default_provider, provider_rows, custom_providers) =
            create_provider_page(provider_page, instance, scale)?;
        let (chatgpt_desktop_pwa, chrome_path, lifecycle) =
            create_browser_page(browser_page, instance, scale, data_root)?;
        let (
            quick_prompt,
            start_on_login,
            clipboard_fallback,
            hide_prompt_after_prepare,
            debug_logging,
        ) = create_general_page(general_page, instance, scale)?;

        create_button(
            window,
            instance,
            scale,
            "应用更改",
            454,
            544,
            104,
            CONTROL_APPLY,
        )?;
        create_button(
            window,
            instance,
            scale,
            "恢复默认快捷键",
            570,
            544,
            132,
            CONTROL_RESTORE_DEFAULTS,
        )?;
        create_button(window, instance, scale, "关闭", 714, 544, 64, CONTROL_CLOSE)?;
        let status = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "准备就绪。所有设置在校验后统一应用。",
            WS_CHILD | WS_VISIBLE,
            24,
            592,
            754,
            28,
            0,
            STATUS_LABEL,
        )?;

        let mut settings = Self {
            window,
            rows,
            provider_rows,
            provider_ids: Vec::new(),
            default_provider,
            custom_providers,
            chatgpt_desktop_pwa,
            chrome_path,
            lifecycle,
            quick_prompt,
            start_on_login,
            clipboard_fallback,
            hide_prompt_after_prepare,
            debug_logging,
            status,
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
        set_checked(self.clipboard_fallback, config.general.clipboard_fallback);
        set_checked(
            self.hide_prompt_after_prepare,
            config.general.hide_prompt_after_prepare,
        );
        set_checked(self.debug_logging, config.general.debug_logging);
        Ok(())
    }

    pub fn read_config(&self, base: &AppConfig) -> Result<AppConfig> {
        let mut candidate = base.clone();
        candidate.hotkeys = self.read_hotkeys()?;
        candidate.default_provider_id = self.read_default_provider()?;
        candidate.quick_prompt = get_text(self.quick_prompt)?.trim().to_owned();
        candidate.general.start_on_login = is_checked(self.start_on_login);
        candidate.general.clipboard_fallback = is_checked(self.clipboard_fallback);
        candidate.general.hide_prompt_after_prepare = is_checked(self.hide_prompt_after_prepare);
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

fn create_page(parent: HWND, instance: HINSTANCE, scale: UiScale, id: u16) -> Result<HWND> {
    create_control(
        parent,
        instance,
        scale,
        SETTINGS_CLASS,
        "",
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
        18,
        58,
        762,
        466,
        0,
        id,
    )
}

fn create_hotkey_page(page: HWND, instance: HINSTANCE, scale: UiScale) -> Result<Vec<HotkeyRow>> {
    create_label(page, instance, scale, "全局快捷键", 12, 8, 720, 28, 0)?;
    create_label(
        page,
        instance,
        scale,
        "修改后立即生效；注册或保存失败时会保留原绑定。",
        12,
        38,
        720,
        24,
        0,
    )?;
    let definitions = [
        (
            AppCommand::CaptureWithPrompt,
            "截图并提问",
            "截取区域后输入问题",
            EDIT_CAPTURE,
            CHECK_CAPTURE,
        ),
        (
            AppCommand::CaptureQuickDispatch,
            "截图快速投递",
            "使用默认供应商和快速提示词",
            EDIT_QUICK,
            CHECK_QUICK,
        ),
        (
            AppCommand::TextOnlyPrompt,
            "直接文字提问",
            "不截图，直接打开输入框",
            EDIT_TEXT,
            CHECK_TEXT,
        ),
    ];
    let mut rows = Vec::new();
    for (index, (command, label, description, edit_id, check_id)) in
        definitions.into_iter().enumerate()
    {
        let y = 82 + index as i32 * 104;
        create_label(page, instance, scale, label, 12, y, 190, 26, 0)?;
        create_label(page, instance, scale, description, 12, y + 28, 280, 24, 0)?;
        let edit = create_control(
            page,
            instance,
            scale,
            "EDIT",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            310,
            y,
            252,
            34,
            WS_EX_CLIENTEDGE,
            edit_id,
        )?;
        set_limit(edit, MAX_SINGLE_LINE);
        let enabled = create_control(
            page,
            instance,
            scale,
            "BUTTON",
            "启用",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            590,
            y + 2,
            92,
            30,
            0,
            check_id,
        )?;
        rows.push(HotkeyRow {
            command,
            edit,
            enabled,
        });
    }
    Ok(rows)
}

fn create_provider_page(
    page: HWND,
    instance: HINSTANCE,
    scale: UiScale,
) -> Result<(HWND, Vec<ProviderRow>, HWND)> {
    create_label(page, instance, scale, "默认供应商", 12, 8, 132, 26, 0)?;
    let default_provider = create_control(
        page,
        instance,
        scale,
        "COMBOBOX",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST as u32,
        154,
        4,
        260,
        220,
        0,
        COMBO_DEFAULT_PROVIDER,
    )?;
    create_label(
        page,
        instance,
        scale,
        "内置供应商（修改入口时会把匹配边界收敛到同一 HTTPS 域名）",
        12,
        48,
        710,
        24,
        0,
    )?;
    let defaults = built_in_providers();
    let mut rows = Vec::new();
    for (index, provider) in defaults.into_iter().enumerate() {
        let y = 78 + index as i32 * 48;
        let enabled = create_control(
            page,
            instance,
            scale,
            "BUTTON",
            &provider.display_name,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            12,
            y + 3,
            128,
            30,
            0,
            CHECK_PROVIDER_BASE + index as u16,
        )?;
        let start_url = create_control(
            page,
            instance,
            scale,
            "EDIT",
            "",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            154,
            y,
            568,
            34,
            WS_EX_CLIENTEDGE,
            EDIT_PROVIDER_URL_BASE + index as u16,
        )?;
        set_limit(start_url, MAX_SINGLE_LINE);
        rows.push(ProviderRow {
            id: provider.id,
            enabled,
            start_url,
        });
    }
    create_label(
        page,
        instance,
        scale,
        "自定义供应商（每行：id | 名称 | 起始网址 | 匹配前缀；多个前缀用逗号分隔）",
        12,
        278,
        720,
        24,
        0,
    )?;
    let custom = create_control(
        page,
        instance,
        scale,
        "EDIT",
        "",
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_WANTRETURN as u32,
        12,
        308,
        710,
        132,
        WS_EX_CLIENTEDGE,
        EDIT_CUSTOM_PROVIDERS,
    )?;
    set_limit(custom, MAX_MULTI_LINE);
    Ok((default_provider, rows, custom))
}

fn create_browser_page(
    page: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    data_root: &Path,
) -> Result<(HWND, HWND, HWND)> {
    let pwa = create_control(
        page,
        instance,
        scale,
        "BUTTON",
        "ChatGPT 使用桌面网页端并复用现有登录",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        12,
        10,
        520,
        30,
        0,
        CHECK_CHATGPT_DESKTOP_PWA,
    )?;
    create_label(
        page,
        instance,
        scale,
        "Chrome 可执行文件",
        12,
        58,
        210,
        24,
        0,
    )?;
    create_label(
        page,
        instance,
        scale,
        "留空自动检测；填写时必须是现有 chrome.exe 的绝对路径。",
        230,
        58,
        492,
        24,
        0,
    )?;
    let chrome = create_control(
        page,
        instance,
        scale,
        "EDIT",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
        12,
        86,
        710,
        34,
        WS_EX_CLIENTEDGE,
        EDIT_CHROME_PATH,
    )?;
    set_limit(chrome, MAX_SINGLE_LINE);
    create_label(
        page,
        instance,
        scale,
        "专用 Chrome 生命周期",
        12,
        142,
        210,
        24,
        0,
    )?;
    let lifecycle = create_control(
        page,
        instance,
        scale,
        "COMBOBOX",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST as u32,
        230,
        136,
        350,
        220,
        0,
        COMBO_LIFECYCLE,
    )?;
    create_label(
        page,
        instance,
        scale,
        "AskBridge 数据目录",
        12,
        196,
        210,
        24,
        0,
    )?;
    let data_path = create_control(
        page,
        instance,
        scale,
        "EDIT",
        &data_root.to_string_lossy(),
        WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL as u32 | ES_READONLY as u32,
        12,
        224,
        710,
        34,
        WS_EX_CLIENTEDGE,
        EDIT_DATA_PATH,
    )?;
    set_limit(data_path, MAX_SINGLE_LINE);
    create_label(
        page,
        instance,
        scale,
        "浏览器工具只控制 AskBridge 专用配置，不连接日常 Chrome。",
        12,
        278,
        710,
        24,
        0,
    )?;
    create_button(
        page,
        instance,
        scale,
        "打开 AskBridge 浏览器",
        12,
        316,
        190,
        CONTROL_OPEN_BROWSER,
    )?;
    create_button(
        page,
        instance,
        scale,
        "检查连接",
        216,
        316,
        130,
        CONTROL_CHECK_BROWSER,
    )?;
    create_button(
        page,
        instance,
        scale,
        "打开默认供应商登录页面",
        360,
        316,
        226,
        CONTROL_OPEN_LOGIN,
    )?;
    Ok((pwa, chrome, lifecycle))
}

fn create_general_page(
    page: HWND,
    instance: HINSTANCE,
    scale: UiScale,
) -> Result<(HWND, HWND, HWND, HWND, HWND)> {
    create_label(
        page,
        instance,
        scale,
        "截图快速投递提示词",
        12,
        8,
        250,
        24,
        0,
    )?;
    let quick_prompt = create_control(
        page,
        instance,
        scale,
        "EDIT",
        "",
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_WANTRETURN as u32,
        12,
        38,
        710,
        112,
        WS_EX_CLIENTEDGE,
        EDIT_QUICK_PROMPT,
    )?;
    set_limit(quick_prompt, MAX_MULTI_LINE);
    let start_on_login = create_check(
        page,
        instance,
        scale,
        "登录 Windows 后启动 AskBridge（当前用户，不需管理员权限）",
        12,
        176,
        CHECK_START_ON_LOGIN,
    )?;
    let clipboard = create_check(
        page,
        instance,
        scale,
        "自动化失败时启用剪贴板兜底，并在关闭后尽力恢复",
        12,
        220,
        CHECK_CLIPBOARD_FALLBACK,
    )?;
    let hide_prompt = create_check(
        page,
        instance,
        scale,
        "内容准备完成后隐藏输入框",
        12,
        264,
        CHECK_HIDE_PROMPT,
    )?;
    let debug = create_check(
        page,
        instance,
        scale,
        "启用调试日志（立即生效；日志仍不记录问题、截图或网页正文）",
        12,
        308,
        CHECK_DEBUG_LOGGING,
    )?;
    create_label(
        page,
        instance,
        scale,
        "AskBridge 1.0 没有自动发送开关；所有请求始终由用户在网页中确认发送。",
        12,
        364,
        710,
        44,
        0,
    )?;
    Ok((quick_prompt, start_on_login, clipboard, hide_prompt, debug))
}

#[allow(clippy::too_many_arguments)]
fn create_label(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: u16,
) -> Result<HWND> {
    create_control(
        parent,
        instance,
        scale,
        "STATIC",
        text,
        WS_CHILD | WS_VISIBLE,
        x,
        y,
        width,
        height,
        0,
        id,
    )
}

fn create_check(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    text: &str,
    x: i32,
    y: i32,
    id: u16,
) -> Result<HWND> {
    create_control(
        parent,
        instance,
        scale,
        "BUTTON",
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        x,
        y,
        710,
        32,
        0,
        id,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_button(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    id: u16,
) -> Result<HWND> {
    create_control(
        parent,
        instance,
        scale,
        "BUTTON",
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        x,
        y,
        width,
        36,
        0,
        id,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_control(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    extended_style: u32,
    id: u16,
) -> Result<HWND> {
    let class = wide(class);
    let text = wide(text);
    // SAFETY: Class/text buffers are valid for the call, and parent/instance are live.
    let control = unsafe {
        CreateWindowExW(
            extended_style,
            class.as_ptr(),
            text.as_ptr(),
            style,
            scale.px(x),
            scale.px(y),
            scale.px(width),
            scale.px(height),
            parent,
            id as usize as _,
            instance,
            ptr::null(),
        )
    };
    if control.is_null() {
        return Err(AppError::Windows {
            operation: "CreateWindowExW(settings control)",
            win32_code: last_error(),
        });
    }
    // SAFETY: DEFAULT_GUI_FONT is a process-lifetime stock object and control is live.
    unsafe {
        SendMessageW(
            control,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as WPARAM,
            1,
        );
    }
    Ok(control)
}

fn set_limit(edit: HWND, limit: WPARAM) {
    // SAFETY: edit is a live EDIT control.
    unsafe {
        SendMessageW(edit, EM_SETLIMITTEXT, limit, 0);
    }
}

fn set_text(window: HWND, value: &str) -> Result<()> {
    let value = wide(value);
    // SAFETY: window is live and value is a nul-terminated UTF-16 string.
    if unsafe { SetWindowTextW(window, value.as_ptr()) } == 0 {
        return Err(AppError::Windows {
            operation: "SetWindowTextW(settings)",
            win32_code: last_error(),
        });
    }
    Ok(())
}

fn get_text(window: HWND) -> Result<String> {
    // SAFETY: window is a live control and the query is read-only.
    let length = unsafe { GetWindowTextLengthW(window) };
    if length < 0 {
        return Err(AppError::Windows {
            operation: "GetWindowTextLengthW(settings)",
            win32_code: last_error(),
        });
    }
    let mut buffer = vec![0u16; length as usize + 1];
    // SAFETY: buffer has room for the reported text and terminating nul.
    let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied < 0 {
        return Err(AppError::Windows {
            operation: "GetWindowTextW(settings)",
            win32_code: last_error(),
        });
    }
    buffer.truncate(copied as usize);
    Ok(String::from_utf16_lossy(&buffer))
}

fn set_checked(control: HWND, checked: bool) {
    // SAFETY: control is a live checkbox or radio button.
    unsafe {
        SendMessageW(
            control,
            BM_SETCHECK,
            if checked {
                BST_CHECKED as WPARAM
            } else {
                BST_UNCHECKED as WPARAM
            },
            0,
        );
    }
}

fn is_checked(control: HWND) -> bool {
    // SAFETY: control is a live checkbox.
    unsafe { SendMessageW(control, BM_GETCHECK, 0, 0) == BST_CHECKED as isize }
}

fn combo_reset(combo: HWND) {
    // SAFETY: combo is a live COMBOBOX.
    unsafe {
        SendMessageW(combo, CB_RESETCONTENT, 0, 0);
    }
}

fn combo_add(combo: HWND, value: &str) -> Result<usize> {
    let value = wide(value);
    // SAFETY: combo is live and value remains valid for the synchronous call.
    let index = unsafe { SendMessageW(combo, CB_ADDSTRING, 0, value.as_ptr() as LPARAM) };
    if index < 0 {
        return Err(AppError::Windows {
            operation: "CB_ADDSTRING(settings)",
            win32_code: last_error(),
        });
    }
    Ok(index as usize)
}

fn combo_select(combo: HWND, index: usize) {
    // SAFETY: combo is live; invalid indices are detected later by read_config.
    unsafe {
        SendMessageW(combo, CB_SETCURSEL, index, 0);
    }
}

fn combo_selection(combo: HWND) -> Result<usize> {
    // SAFETY: combo is a live COMBOBOX.
    let selected = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) };
    if selected < 0 {
        Err(AppError::ConfigurationInvalid(
            "a required setting has no selection".to_owned(),
        ))
    } else {
        Ok(selected as usize)
    }
}

fn parse_custom_providers(text: &str) -> Result<Vec<ProviderConfig>> {
    let mut providers = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields = line.split('|').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(AppError::InvalidProvider(format!(
                "custom provider line {} must contain id | name | start URL | match prefixes",
                index + 1
            )));
        }
        let url_patterns = fields[3]
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let provider = ProviderConfig {
            id: fields[0].to_owned(),
            display_name: fields[1].to_owned(),
            enabled: true,
            start_url: fields[2].to_owned(),
            url_patterns,
            is_custom: true,
            adapter_override: None,
        };
        provider.validate()?;
        providers.push(provider);
    }
    Ok(providers)
}

fn origin_pattern(url: &str) -> Result<String> {
    let Some(remainder) = url.strip_prefix("https://") else {
        return Err(AppError::InvalidProviderUrl(url.to_owned()));
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(AppError::InvalidProviderUrl(url.to_owned()));
    }
    Ok(format!("https://{authority}/"))
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
                return 0;
            }
            let class = wide(MAIN_WINDOW_CLASS);
            let title = wide(MAIN_WINDOW_TITLE);
            // SAFETY: Both search strings are valid nul-terminated UTF-16 buffers.
            let main_window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
            if !main_window.is_null() {
                // SAFETY: WM_COMMAND targets our own UI thread and carries only control data.
                if unsafe { PostMessageW(main_window, message, wparam, lparam) } == 0 {
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
        WM_CTLCOLORSTATIC => {
            // SAFETY: COLOR_WINDOW always yields a process-lifetime system brush.
            unsafe { GetSysColorBrush(COLOR_WINDOW) as LRESULT }
        }
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
        let source = include_str!("settings_v2.rs");
        let forbidden_control = ["CHECK_", "AUTO_", "SUBMIT"].concat();
        assert!(!source.contains(&forbidden_control));
    }
}
