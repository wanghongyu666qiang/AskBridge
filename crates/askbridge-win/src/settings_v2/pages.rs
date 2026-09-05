use super::controls::*;
use super::theme::{UiFonts, UiScale};
use super::*;

pub(super) fn create_page(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    id: u16,
) -> Result<HWND> {
    create_control(
        parent,
        instance,
        scale,
        SETTINGS_CLASS,
        "",
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
        24,
        100,
        794,
        482,
        0,
        id,
    )
}

pub(super) fn create_hotkey_page(
    page: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    fonts: &UiFonts,
) -> Result<Vec<HotkeyRow>> {
    create_group(page, instance, scale, fonts, "全局快捷键", 12, 6, 770, 344)?;
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
        let y = 40 + index as i32 * 120;
        create_label(page, instance, scale, label, 28, y, 240, 26, 0)?;
        create_label(page, instance, scale, description, 28, y + 28, 280, 24, 0)?;
        let edit = create_framed_edit(page, instance, scale, "", 320, y, 270, 34, 0, edit_id)?;
        set_limit(edit, MAX_SINGLE_LINE);
        let enabled = create_control(
            page,
            instance,
            scale,
            "BUTTON",
            "启用",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            612,
            y + 2,
            100,
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
    create_label(
        page,
        instance,
        scale,
        "修改后立即生效；注册或保存失败时会保留原绑定。",
        12,
        362,
        570,
        24,
        0,
    )?;
    create_button(
        page,
        instance,
        scale,
        fonts,
        "恢复默认快捷键",
        596,
        356,
        186,
        CONTROL_RESTORE_DEFAULTS,
    )?;
    Ok(rows)
}

pub(super) fn create_provider_page(
    page: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    fonts: &UiFonts,
) -> Result<(HWND, Vec<ProviderRow>, HWND)> {
    create_label(page, instance, scale, "默认供应商", 12, 14, 132, 26, 0)?;
    let default_provider = create_control(
        page,
        instance,
        scale,
        "COMBOBOX",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST as u32,
        154,
        8,
        260,
        220,
        0,
        COMBO_DEFAULT_PROVIDER,
    )?;

    let defaults = built_in_providers();
    let group_height = 34 + defaults.len() as i32 * 36 + 12;
    create_group(
        page,
        instance,
        scale,
        fonts,
        "内置供应商（入口只匹配同一 HTTPS 域名）",
        12,
        48,
        770,
        group_height,
    )?;
    let mut rows = Vec::new();
    for (index, provider) in defaults.into_iter().enumerate() {
        let y = 34 + index as i32 * 36;
        let enabled = create_control(
            page,
            instance,
            scale,
            "BUTTON",
            &provider.display_name,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            28,
            y + 2,
            128,
            30,
            0,
            CHECK_PROVIDER_BASE + index as u16,
        )?;
        let start_url = create_framed_edit(
            page,
            instance,
            scale,
            "",
            170,
            y,
            350,
            34,
            0,
            EDIT_PROVIDER_URL_BASE + index as u16,
        )?;
        set_limit(start_url, MAX_SINGLE_LINE);
        let health = create_control(
            page,
            instance,
            scale,
            "STATIC",
            "○ 未检测",
            WS_CHILD | WS_VISIBLE,
            536,
            y + 6,
            220,
            24,
            0,
            PROVIDER_HEALTH_BASE + index as u16,
        )?;
        rows.push(ProviderRow {
            id: provider.id,
            enabled,
            start_url,
            health,
        });
    }

    let custom_y = 48 + group_height + 12;
    create_group(
        page,
        instance,
        scale,
        fonts,
        "自定义供应商",
        12,
        custom_y,
        770,
        482 - custom_y - 6,
    )?;
    create_label(
        page,
        instance,
        scale,
        "每行一条：id | 名称 | 起始网址 | 匹配前缀；多个前缀用逗号分隔",
        28,
        24,
        740,
        24,
        0,
    )?;
    let custom = create_framed_edit(
        page,
        instance,
        scale,
        "",
        28,
        50,
        740,
        482 - custom_y - 66,
        WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32,
        EDIT_CUSTOM_PROVIDERS,
    )?;
    set_limit(custom, MAX_MULTI_LINE);
    Ok((default_provider, rows, custom))
}

pub(super) fn create_browser_page(
    page: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    data_root: &Path,
    fonts: &UiFonts,
) -> Result<(HWND, HWND, HWND, HWND, HWND, HWND)> {
    create_group(
        page,
        instance,
        scale,
        fonts,
        "ChatGPT 打开方式",
        12,
        8,
        770,
        196,
    )?;
    let pwa = create_control(
        page,
        instance,
        scale,
        "BUTTON",
        "桌面网页端：复用现有登录，但截图需要手动上传",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_GROUP | BS_AUTORADIOBUTTON as u32,
        28,
        32,
        740,
        30,
        0,
        RADIO_CHATGPT_DESKTOP_PWA,
    )?;
    set_font(pwa, fonts.body.handle());
    let dedicated = create_control(
        page,
        instance,
        scale,
        "BUTTON",
        "AskBridge 专用 Chrome：支持自动上传图片，需要单独登录",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTORADIOBUTTON as u32,
        28,
        66,
        740,
        30,
        0,
        RADIO_CHATGPT_DEDICATED_CHROME,
    )?;
    set_font(dedicated, fonts.body.handle());
    let clipboard_paste = create_control(
        page,
        instance,
        scale,
        "BUTTON",
        "通用粘贴：支持浏览器或 AI 桌面端，仅模拟 Ctrl+V（不验证结果）",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTORADIOBUTTON as u32,
        28,
        100,
        740,
        30,
        0,
        RADIO_CHATGPT_CLIPBOARD_PASTE,
    )?;
    set_font(clipboard_paste, fonts.body.handle());
    let dedicated_then_clipboard = create_control(
        page,
        instance,
        scale,
        "BUTTON",
        "专用 Chrome 优先：安全失败后自动用 Ctrl+V 粘贴截图",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTORADIOBUTTON as u32,
        28,
        134,
        740,
        30,
        0,
        RADIO_CHATGPT_DEDICATED_THEN_CLIPBOARD,
    )?;
    set_font(dedicated_then_clipboard, fonts.body.handle());
    create_label(
        page,
        instance,
        scale,
        "自动降级只用于截图；纯文字仍只走专用 Chrome。",
        28,
        168,
        740,
        24,
        0,
    )?;

    create_group(
        page,
        instance,
        scale,
        fonts,
        "专用 Chrome",
        12,
        212,
        770,
        206,
    )?;
    create_label(
        page,
        instance,
        scale,
        "Chrome 可执行文件",
        28,
        26,
        170,
        24,
        0,
    )?;
    create_label(
        page,
        instance,
        scale,
        "留空自动检测；填写时必须是现有 chrome.exe 的绝对路径。",
        216,
        26,
        550,
        24,
        0,
    )?;
    let chrome = create_framed_edit(
        page,
        instance,
        scale,
        "",
        28,
        52,
        740,
        34,
        0,
        EDIT_CHROME_PATH,
    )?;
    set_limit(chrome, MAX_SINGLE_LINE);
    create_label(page, instance, scale, "生命周期", 28, 100, 170, 24, 0)?;
    let lifecycle = create_control(
        page,
        instance,
        scale,
        "COMBOBOX",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST as u32,
        216,
        96,
        350,
        220,
        0,
        COMBO_LIFECYCLE,
    )?;
    create_button(
        page,
        instance,
        scale,
        fonts,
        "打开 AskBridge 浏览器",
        28,
        134,
        190,
        CONTROL_OPEN_BROWSER,
    )?;
    create_button(
        page,
        instance,
        scale,
        fonts,
        "检查连接",
        228,
        134,
        130,
        CONTROL_CHECK_BROWSER,
    )?;
    create_button(
        page,
        instance,
        scale,
        fonts,
        "打开默认供应商登录页面",
        368,
        134,
        226,
        CONTROL_OPEN_LOGIN,
    )?;
    create_label(
        page,
        instance,
        scale,
        "浏览器工具只控制 AskBridge 专用配置，不连接日常 Chrome。",
        28,
        176,
        740,
        24,
        0,
    )?;

    create_group(
        page,
        instance,
        scale,
        fonts,
        "AskBridge 数据目录",
        12,
        426,
        770,
        50,
    )?;
    let data_path = create_framed_edit(
        page,
        instance,
        scale,
        &data_root.to_string_lossy(),
        28,
        22,
        740,
        26,
        ES_READONLY as u32,
        EDIT_DATA_PATH,
    )?;
    set_limit(data_path, MAX_SINGLE_LINE);
    Ok((
        pwa,
        dedicated,
        clipboard_paste,
        dedicated_then_clipboard,
        chrome,
        lifecycle,
    ))
}

pub(super) fn create_general_page(
    page: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    fonts: &UiFonts,
) -> Result<(HWND, HWND, HWND)> {
    create_group(
        page,
        instance,
        scale,
        fonts,
        "截图快速投递",
        12,
        8,
        770,
        172,
    )?;
    create_label(page, instance, scale, "提示词", 28, 26, 300, 24, 0)?;
    let quick_prompt = create_framed_edit(
        page,
        instance,
        scale,
        "",
        28,
        52,
        740,
        104,
        WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32,
        EDIT_QUICK_PROMPT,
    )?;
    set_limit(quick_prompt, MAX_MULTI_LINE);

    create_group(
        page,
        instance,
        scale,
        fonts,
        "启动与日志",
        12,
        188,
        770,
        168,
    )?;
    let start_on_login = create_check(
        page,
        instance,
        scale,
        fonts,
        "登录 Windows 后启动 AskBridge（当前用户，不需管理员权限）",
        28,
        32,
        CHECK_START_ON_LOGIN,
    )?;
    let debug = create_check(
        page,
        instance,
        scale,
        fonts,
        "启用调试日志（立即生效；日志仍不记录问题、截图或网页正文）",
        28,
        78,
        CHECK_DEBUG_LOGGING,
    )?;
    create_label(
        page,
        instance,
        scale,
        "AskBridge 没有自动发送开关；所有请求始终由你在网页中确认发送。",
        28,
        126,
        740,
        24,
        0,
    )?;
    Ok((quick_prompt, start_on_login, debug))
}
