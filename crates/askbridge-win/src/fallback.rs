use std::{mem::size_of, ptr};

use askbridge_core::{AppError, DispatchRequest, PreparationOutcome, RecoveryHint, Result};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::Controls::{
        TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TDCBF_CANCEL_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
        TDF_SIZE_TO_CONTENT, TDF_USE_COMMAND_LINKS, TDN_BUTTON_CLICKED, TaskDialogIndirect,
    },
};

use crate::{clipboard::ClipboardSession, util::wide};

const COPY_IMAGE: i32 = 1001;
const COPY_PROMPT: i32 = 1002;
const RETRY: i32 = 1003;
const IDCANCEL: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackAction {
    Retry,
    Cancel,
}

pub fn show(
    owner: HWND,
    request: &DispatchRequest,
    outcome: &PreparationOutcome,
) -> Result<FallbackAction> {
    let clipboard = ClipboardSession::begin(owner)?;
    if let Some(image) = &request.image {
        clipboard.copy_image(image)?;
    } else {
        clipboard.copy_text(&request.prompt)?;
    }

    let title = wide("AskBridge 人工兜底");
    let instruction = wide("自动准备已安全停止，内容仍由你确认发送");
    let content = if outcome.recovery_hint == Some(RecoveryHint::LoginInBrowser) {
        wide(
            "检测到供应商登录跳转。请在当前浏览器中自行完成登录，然后选择重试；AskBridge 不会读取密码、验证码或登录 Cookie。",
        )
    } else if outcome.recovery_hint == Some(RecoveryHint::ProviderPageChanged) {
        wide(
            "内置供应商规则和通用定位都无法可靠确认输入框，网页可能已经改版。请手动粘贴内容，或确认页面恢复正常后重试。",
        )
    } else if request.image.is_some() {
        wide(
            "当前剪贴板中是截图。请在网页输入框中粘贴；也可在下方切换复制问题、重试自动准备或取消。关闭后 AskBridge 会尽力恢复原剪贴板。",
        )
    } else {
        wide(
            "当前剪贴板中是问题文字。请在网页输入框中粘贴；也可重试自动准备或取消。关闭后 AskBridge 会尽力恢复原剪贴板。",
        )
    };
    let copy_image = wide("复制图片\n将本次截图放入剪贴板");
    let copy_prompt = wide("复制问题\n将本次问题文字放入剪贴板");
    let retry = wide("重试自动投递\n重新定位页面并准备内容");
    let mut buttons = Vec::new();
    if request.image.is_some() {
        buttons.push(TASKDIALOG_BUTTON {
            nButtonID: COPY_IMAGE,
            pszButtonText: copy_image.as_ptr(),
        });
    }
    buttons.push(TASKDIALOG_BUTTON {
        nButtonID: COPY_PROMPT,
        pszButtonText: copy_prompt.as_ptr(),
    });
    buttons.push(TASKDIALOG_BUTTON {
        nButtonID: RETRY,
        pszButtonText: retry.as_ptr(),
    });

    let mut context = FallbackContext {
        clipboard: &clipboard,
        request,
        copy_failed: false,
    };
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        hInstance: ptr::null_mut(),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT | TDF_USE_COMMAND_LINKS,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: title.as_ptr(),
        Anonymous1: Default::default(),
        pszMainInstruction: instruction.as_ptr(),
        pszContent: content.as_ptr(),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: COPY_PROMPT,
        cRadioButtons: 0,
        pRadioButtons: ptr::null(),
        nDefaultRadioButton: 0,
        pszVerificationText: ptr::null(),
        pszExpandedInformation: ptr::null(),
        pszExpandedControlText: ptr::null(),
        pszCollapsedControlText: ptr::null(),
        Anonymous2: Default::default(),
        pszFooter: ptr::null(),
        pfCallback: Some(fallback_callback),
        lpCallbackData: (&mut context as *mut FallbackContext<'_>) as isize,
        cxWidth: 0,
    };
    let mut pressed = IDCANCEL;
    // SAFETY: All strings, button data and callback context remain live for the
    // synchronous TaskDialogIndirect call.
    let status =
        unsafe { TaskDialogIndirect(&config, &mut pressed, ptr::null_mut(), ptr::null_mut()) };
    if status < 0 {
        return Err(AppError::InvalidPreparation(
            "fallback dialog could not be displayed".to_owned(),
        ));
    }
    if context.copy_failed {
        return Err(AppError::ClipboardWriteFailed);
    }
    match pressed {
        RETRY => Ok(FallbackAction::Retry),
        IDCANCEL => Ok(FallbackAction::Cancel),
        _ => Ok(FallbackAction::Cancel),
    }
}

unsafe extern "system" fn fallback_callback(
    _window: HWND,
    message: u32,
    button: WPARAM,
    _parameter: LPARAM,
    context: isize,
) -> i32 {
    if message != TDN_BUTTON_CLICKED as u32 || context == 0 {
        return 0;
    }
    // SAFETY: TaskDialogIndirect invokes this callback synchronously while the
    // FallbackContext passed in lpCallbackData remains live.
    let context = unsafe { &mut *(context as *mut FallbackContext<'_>) };
    let result = match button as i32 {
        COPY_IMAGE => context
            .request
            .image
            .as_ref()
            .map_or(Ok(()), |image| context.clipboard.copy_image(image)),
        COPY_PROMPT => context.clipboard.copy_text(&context.request.prompt),
        _ => return 0,
    };
    if result.is_err() {
        context.copy_failed = true;
    }
    1
}

struct FallbackContext<'a> {
    clipboard: &'a ClipboardSession,
    request: &'a DispatchRequest,
    copy_failed: bool,
}
