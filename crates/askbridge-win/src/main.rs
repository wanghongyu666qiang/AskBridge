#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("askbridge-win supports Windows only");

mod adapter;
mod app;
mod browser;
mod capture;
mod clipboard_image;
mod data_dir;
mod hotkey_manager;
mod logging;
mod settings_v2;
mod single_instance;
mod startup;
mod tray;
mod util;

fn main() {
    if let Err(error) = app::run() {
        #[cfg(debug_assertions)]
        eprintln!("AskBridge startup failed: {error}");
        util::show_error("AskBridge 无法启动", &error.to_string());
    }
}
