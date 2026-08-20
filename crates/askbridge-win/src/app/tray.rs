use tracing::{error, info};

use super::{controller::Runtime, error_handler::user_facing_error};

impl Runtime {
    pub(super) fn toggle_paused(&mut self) {
        if self.paused {
            let errors = self.hotkeys.register_initial(&self.config.hotkeys);
            if errors.is_empty() {
                self.paused = false;
                self.tray.notify("AskBridge", "全局快捷键已恢复。");
                info!("global hotkeys resumed");
            } else {
                let _ = self.hotkeys.pause();
                let summary = errors
                    .iter()
                    .map(user_facing_error)
                    .collect::<Vec<_>>()
                    .join("; ");
                self.settings
                    .set_status(&format!("无法恢复快捷键：{summary}"));
                self.tray
                    .notify("AskBridge 快捷键冲突", "快捷键仍保持暂停，请检查设置。");
            }
        } else {
            let errors = self.hotkeys.pause();
            if !errors.is_empty() {
                error!(
                    stage = "hotkeys",
                    completed = false,
                    "one or more hotkeys could not be paused"
                );
                self.tray
                    .notify("AskBridge 快捷键暂停失败", "部分快捷键仍然处于活动状态。");
                return;
            }
            self.paused = true;
            self.tray.notify("AskBridge", "全局快捷键已暂停。");
            info!("global hotkeys paused");
        }
    }
}
