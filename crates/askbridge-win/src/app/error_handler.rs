use askbridge_core::{AppCommand, AppError, AppState, PreparationRecovery};
use tracing::error;

use super::controller::Runtime;

pub(super) fn user_facing_error(error: &AppError) -> String {
    match error {
        AppError::HotkeyRegistrationFailed {
            binding,
            win32_code,
        } => format!("快捷键 {binding} 已被其他程序占用（Windows 错误 {win32_code}）"),
        AppError::HotkeyConflict(details) => format!("快捷键设置互相冲突：{details}"),
        AppError::InvalidHotkey(details) => format!("快捷键格式无效：{details}"),
        AppError::ConfigurationInvalid(details) => format!("配置无效：{details}"),
        AppError::CaptureFailed(details) => format!("截图失败：{details}"),
        AppError::InvalidDispatchRequest(details) => format!("问题无法继续：{details}"),
        AppError::InvalidProvider(details) => format!("供应商无效：{details}"),
        AppError::WorkflowBusy(_) => "另一个操作尚未完成。".to_owned(),
        AppError::ClipboardUnavailable => "剪贴板正被其他程序占用，请稍后重试。".to_owned(),
        AppError::ClipboardWriteFailed => "无法将截图写入剪贴板。".to_owned(),
        AppError::ChromeNotFound => "未找到 Google Chrome，请在设置中选择 chrome.exe。".to_owned(),
        AppError::BrowserProfileRejected(details) => format!("专用浏览器目录不安全：{details}"),
        AppError::BrowserProfileInUse => {
            "另一个 AskBridge 专用 Chrome 正在使用该目录；请先正常关闭它后重试。".to_owned()
        }
        AppError::BrowserLaunchFailed => "专用 Chrome 启动失败。".to_owned(),
        AppError::DesktopShortcutNotFound(_) => {
            "未找到桌面 ChatGPT 快捷方式，请确认 ChatGPT.lnk 仍在桌面。".to_owned()
        }
        AppError::DesktopShortcutRejected(details) => format!("桌面快捷方式不安全：{details}"),
        AppError::DesktopLaunchFailed(_) => "ChatGPT 桌面网页端启动失败。".to_owned(),
        AppError::BrowserEndpointUnavailable => "专用 Chrome 未能提供调试端点，请重试。".to_owned(),
        AppError::BrowserConnectionFailed(_) | AppError::BrowserProtocol(_) => {
            "无法连接 AskBridge 专用 Chrome，请重试。".to_owned()
        }
        AppError::BrowserCancelled => "浏览器操作已取消。".to_owned(),
        AppError::PasteTargetUnavailable => {
            "未能激活匹配的浏览器页面或 AI 桌面客户端。请先打开目标窗口，或在设置中改用其他打开方式。".to_owned()
        }
        AppError::TargetNotFound => "未找到可用的目标页面。".to_owned(),
        AppError::TargetTimeout => "目标页面加载超时，请检查浏览器后重试。".to_owned(),
        AppError::PreparationFailed { recovery, .. } => match recovery {
            PreparationRecovery::LoginInBrowser => {
                "请先在所选浏览器中登录当前供应商，然后重试。".to_owned()
            }
            PreparationRecovery::UseDedicatedChrome => {
                "当前网页端不能自动上传截图；请在设置中为该供应商选择 AskBridge 专用 Chrome。"
                    .to_owned()
            }
            PreparationRecovery::ReopenProviderPage => {
                "页面已经离开目标供应商，请重新打开供应商页面后重试。".to_owned()
            }
            PreparationRecovery::ProviderPageChanged => {
                "供应商页面结构已经变化，AskBridge 暂时无法定位输入区。".to_owned()
            }
            PreparationRecovery::Retry => "网页输入区准备失败，请检查页面后重试。".to_owned(),
        },
        _ => error.to_string(),
    }
}

impl Runtime {
    pub(super) fn browser_workflow_failed(&mut self, error: AppError) {
        let request_id = self
            .pending_dispatch
            .as_ref()
            .map_or("", |request| request.id.as_str());
        let provider_id = self
            .pending_dispatch
            .as_ref()
            .map_or("", |request| request.provider_id.as_str());
        error!(
            request_id = %request_id,
            provider_id = %provider_id,
            stage = "browser_workflow",
            completed = false,
            "browser surface workflow failed"
        );
        self.pending_dispatch = None;
        self.tray
            .notify("AskBridge 无法打开目标页面", &user_facing_error(&error));
        if !self.workflow.is_idle() && self.workflow.state() != AppState::Error {
            let _ = self.workflow.fail();
        }
        if self.workflow.state() == AppState::Error {
            let _ = self.workflow.recover();
        }
    }

    pub(super) fn workflow_failed(&mut self, _command: AppCommand, error: AppError) {
        let request_id = self
            .pending_dispatch
            .as_ref()
            .map_or("", |request| request.id.as_str());
        let provider_id = self
            .pending_dispatch
            .as_ref()
            .map_or("", |request| request.provider_id.as_str());
        error!(
            request_id = %request_id,
            provider_id = %provider_id,
            stage = "workflow",
            completed = false,
            "workflow failed"
        );
        self.pending_dispatch = None;
        self.tray
            .notify("AskBridge 无法继续", &user_facing_error(&error));
        if !self.workflow.is_idle() && self.workflow.state() != AppState::Error {
            let _ = self.workflow.fail();
        }
        if self.workflow.state() == AppState::Error {
            let _ = self.workflow.recover();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_in_use_error_has_actionable_message() {
        let message = user_facing_error(&AppError::BrowserProfileInUse);
        assert!(message.contains("AskBridge 专用 Chrome"));
        assert!(message.contains("关闭"));
        assert!(message.contains("重试"));
    }
}
