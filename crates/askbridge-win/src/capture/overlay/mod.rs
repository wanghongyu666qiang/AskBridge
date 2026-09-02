// Capture overlay facade. The heavy lifting lives in the sibling
// submodules: session (message loop + state), guards (RAII Win32
// handles), layout (pure math), draw (painting), gdiplus (FFI).

mod draw;
mod gdiplus;
mod guards;
mod layout;
mod session;

use std::ptr;

use askbridge_core::{AppError, Result, ScreenRect};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND},
    UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, IDC_CROSS, LoadCursorW, RegisterClassW, WNDCLASSW,
    },
};

use crate::{
    capture::{monitor::DesktopLayout, screen::RawBgraImage},
    util::{last_error, wide},
};

use session::{SelectionOutcome, select_region_internal};

const OVERLAY_CLASS: &str = "AskBridge.CaptureOverlay.Window.v1";

#[derive(Debug, Clone)]
pub struct OverlayProviderChoice {
    pub id: String,
    pub display_name: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionAction {
    Ask { provider_id: String },
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedRegion {
    pub rect: ScreenRect,
    pub frozen_pixels: Option<RawBgraImage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionResult {
    pub rect: ScreenRect,
    pub action: SelectionAction,
    pub frozen_pixels: Option<RawBgraImage>,
}

pub fn register_class(instance: HINSTANCE) -> Result<()> {
    let class_name = wide(OVERLAY_CLASS);
    // SAFETY: Loading a shared system cursor with a null instance is supported.
    let cursor = unsafe { LoadCursorW(ptr::null_mut(), IDC_CROSS) };
    if cursor.is_null() {
        return Err(AppError::Windows {
            operation: "LoadCursorW(IDC_CROSS)",
            win32_code: last_error(),
        });
    }
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(session::overlay_window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: cursor,
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    // SAFETY: All pointers remain valid for this synchronous registration call.
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(AppError::Windows {
            operation: "RegisterClassW(capture overlay)",
            win32_code: last_error(),
        });
    }
    Ok(())
}

pub fn select_region(
    instance: HINSTANCE,
    owner: HWND,
    layout: &DesktopLayout,
) -> Result<Option<SelectedRegion>> {
    select_region_internal(instance, owner, layout, None).map(|outcome| match outcome {
        Some(SelectionOutcome::Quick(selection)) => Some(selection),
        Some(SelectionOutcome::Action(result)) => Some(SelectedRegion {
            rect: result.rect,
            frozen_pixels: result.frozen_pixels,
        }),
        None => None,
    })
}

pub fn select_region_with_toolbar(
    instance: HINSTANCE,
    owner: HWND,
    layout: &DesktopLayout,
    providers: Vec<OverlayProviderChoice>,
) -> Result<Option<SelectionResult>> {
    select_region_internal(instance, owner, layout, Some(providers)).map(|outcome| match outcome {
        Some(SelectionOutcome::Action(result)) => Some(result),
        Some(SelectionOutcome::Quick(selection)) => Some(SelectionResult {
            rect: selection.rect,
            action: SelectionAction::Ask {
                provider_id: String::new(),
            },
            frozen_pixels: selection.frozen_pixels,
        }),
        None => None,
    })
}
