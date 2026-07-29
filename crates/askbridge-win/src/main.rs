#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("askbridge-win supports Windows only");

mod app;
mod capture;
mod hotkey_manager;
mod prompt;
mod settings;
mod single_instance;
mod tray;
mod util;

fn main() {
    if let Err(error) = app::run() {
        util::show_error("AskBridge 无法启动", &error.to_string());
    }
}
