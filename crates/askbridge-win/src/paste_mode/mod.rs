//! Clipboard-paste dispatch target: locate an AI website or desktop-client window, bring it to
//! the foreground, and synthesize exactly one Ctrl+V. Nothing is ever typed
//! beyond that shortcut, no page content is read, and sending stays with the
//! user. A provider-neutral UI Automation receipt verifies that the page added
//! attachment structure after the paste.

mod discover;
mod focus;
mod keystroke;
mod receipt;

pub(crate) use discover::{find_provider_windows, provider_title_keywords};
pub(crate) use focus::prepare_paste_target;
pub(crate) use keystroke::{open_default_browser, send_paste};
pub(crate) use receipt::{paste_attachment_baseline, wait_for_paste_attachment};

pub(super) const S_OK: i32 = 0;
pub(super) const S_FALSE: i32 = 1;
pub(super) const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;
