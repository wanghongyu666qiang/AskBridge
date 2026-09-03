#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("askbridge-win supports Windows only");

mod adapter;
mod app;
mod app_icon;
mod browser;
mod capture;
mod clipboard_image;
mod data_dir;
mod hotkey_manager;
mod logging;
mod paste_mode;
mod settings_v2;
mod single_instance;
#[cfg(not(feature = "store"))]
mod startup;
#[cfg(feature = "store")]
#[path = "startup_store.rs"]
mod startup;
#[cfg(feature = "store")]
mod store_runtime;
mod tray;
mod update;
mod util;

fn main() {
    if let Err(error) = app::run() {
        #[cfg(debug_assertions)]
        eprintln!("AskBridge startup failed: {error}");
        util::show_error("AskBridge 无法启动", &error.to_string());
    }
}
