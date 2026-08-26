use std::path::Path;

use tracing::{error, info, warn};
use windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage;

use crate::{
    update::{AvailableUpdate, UpdateAction, UpdateEvent},
    util::confirm_update,
};

use super::{controller::Runtime, error_handler::user_facing_error};

impl Runtime {
    pub(super) fn check_for_updates(&mut self) {
        if self.update_busy {
            self.tray
                .notify("AskBridge 更新", "更新检查或下载正在进行，请稍后再试。");
            return;
        }
        self.update_busy = true;
        self.settings.set_status("正在检查 AskBridge 更新…");
        if self.updater.check_now().is_err() {
            self.update_busy = false;
            self.settings.set_status("无法启动更新检查。");
            self.tray
                .notify("AskBridge 更新", "无法启动更新检查，请稍后重试。");
        }
    }

    pub(super) fn prompt_and_download_update(&mut self) {
        if self.update_busy {
            return;
        }
        let Some(release) = self.available_update.clone() else {
            self.check_for_updates();
            return;
        };
        if !self.updater.supports_in_place_update() {
            self.settings.set_status(&format!(
                "发现 AskBridge {}；便携版请从官方 GitHub Releases 手动更新。",
                release.version()
            ));
            self.tray.notify(
                "AskBridge 便携版更新",
                &format!(
                    "AskBridge {} 已发布，请从官方 GitHub Releases 手动更新。",
                    release.version()
                ),
            );
            return;
        }
        // A previously downloaded and verified setup is installed directly;
        // the user already consented when it was downloaded.
        if let Some(setup_path) = self.updater.cached_verified_setup(release.version()) {
            self.install_downloaded_update(&release, &setup_path);
            return;
        }
        if !confirm_update(
            self._main_window.hwnd(),
            release.version(),
            release.notes(),
            release.release_url(),
        ) {
            self.settings
                .set_status(&format!("已暂缓安装 AskBridge {}。", release.version()));
            return;
        }
        if !self.workflow.is_idle() {
            self.tray
                .notify("AskBridge 正在处理", "请先完成或取消当前操作，再安装更新。");
            return;
        }
        self.update_busy = true;
        self.settings
            .set_status(&format!("正在下载 AskBridge {}…", release.version()));
        self.tray.notify(
            "AskBridge 更新",
            &format!("正在下载并校验 AskBridge {}。", release.version()),
        );
        if self.updater.download(release).is_err() {
            self.update_busy = false;
            self.settings.set_status("无法启动更新下载。");
            self.tray
                .notify("AskBridge 更新", "无法启动更新下载，请稍后重试。");
        }
    }

    pub(super) fn handle_update_events(&mut self) {
        for event in self.updater.drain_events() {
            match event {
                UpdateEvent::Checked { available, manual } => {
                    if manual {
                        self.update_busy = false;
                    }
                    match available {
                        Some(release) => {
                            let version = release.version().to_owned();
                            self.available_update = Some(release);
                            if self.updater.supports_in_place_update() {
                                self.settings.set_status(&format!(
                                    "发现 AskBridge {version}；可从托盘菜单安装。"
                                ));
                                self.tray.notify(
                                    "AskBridge 有新版本",
                                    &format!("AskBridge {version} 已发布，可从托盘菜单安装。"),
                                );
                            } else {
                                self.settings.set_status(&format!(
                                    "发现 AskBridge {version}；便携版请手动更新。"
                                ));
                                self.tray.notify(
                                    "AskBridge 便携版有新版本",
                                    &format!(
                                        "AskBridge {version} 已发布，请从官方 GitHub Releases 手动更新。"
                                    ),
                                );
                            }
                            info!(
                                stage = "update_check",
                                completed = true,
                                update_available = true,
                                "application update check completed"
                            );
                            if manual {
                                self.prompt_and_download_update();
                            }
                        }
                        None => {
                            if manual {
                                self.settings.set_status("当前已经是最新版本。");
                                self.tray.notify("AskBridge 更新", "当前已经是最新版本。");
                            }
                            info!(
                                stage = "update_check",
                                completed = true,
                                update_available = false,
                                "application update check completed"
                            );
                        }
                    }
                }
                UpdateEvent::DownloadProgress {
                    version,
                    received,
                    total,
                } => {
                    let percent = received
                        .min(total)
                        .checked_mul(100)
                        .and_then(|scaled| scaled.checked_div(total))
                        .unwrap_or(100);
                    self.settings
                        .set_status(&format!("正在下载并校验 AskBridge {version}… {percent}%"));
                }
                UpdateEvent::Downloaded {
                    release,
                    setup_path,
                } => {
                    self.update_busy = false;
                    self.install_downloaded_update(&release, &setup_path);
                }
                UpdateEvent::Failed {
                    action,
                    manual,
                    message,
                } => {
                    if manual || action == UpdateAction::Download {
                        self.update_busy = false;
                        self.settings
                            .set_status(&format!("更新操作失败：{message}"));
                        self.tray.notify(
                            "AskBridge 更新失败",
                            "无法完成更新；当前版本未被修改，请稍后重试。",
                        );
                    }
                    warn!(
                        stage = "application_update",
                        completed = false,
                        action = ?action,
                        "application update operation failed"
                    );
                }
            }
        }
    }

    /// Installs a previously downloaded and hash-verified setup, either right
    /// after its download finishes or later from the tray menu retry entry.
    fn install_downloaded_update(&mut self, release: &AvailableUpdate, setup_path: &Path) {
        if !self.workflow.is_idle() {
            self.tray
                .notify("AskBridge 正在处理", "请先完成或取消当前操作，再安装更新。");
            return;
        }
        self.settings.set_status(&format!(
            "正在安装已校验的 AskBridge {}…",
            release.version()
        ));
        if let Err(install_error) = self.updater.launch_installer(setup_path) {
            error!(
                stage = "update_install",
                completed = false,
                "verified updater could not be launched"
            );
            self.settings.set_status(&format!(
                "AskBridge {} 已通过校验，但安装程序无法启动：{}。可从托盘菜单重试安装。",
                release.version(),
                user_facing_error(&install_error)
            ));
            self.tray.notify(
                "AskBridge 更新失败",
                "安装程序无法启动；可从托盘菜单再次选择“安装”重试，无需重新下载。",
            );
            return;
        }
        info!(
            stage = "update_install",
            completed = false,
            "verified updater launched; application will exit"
        );
        self.browser.cancel();
        // SAFETY: Called on the UI thread to allow owned resources to drop cleanly;
        // the updater waits for this process before replacing installed files.
        unsafe {
            PostQuitMessage(0);
        }
    }
}
