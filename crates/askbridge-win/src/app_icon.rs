//! Loads the AskBridge icon embedded in the executable resources by `askbridge.rc`.

use std::ptr;

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, HICON, IDI_APPLICATION, IMAGE_ICON, LR_SHARED, LoadIconW, LoadImageW,
    SM_CXICON, SM_CXSMICON,
};

/// Resource identifier of the `RT_GROUP_ICON` declared in `askbridge.rc`.
const APP_ICON_RESOURCE_ID: usize = 1;

/// Loads the embedded application icon sized for the tray (`small`) or for
/// window classes. Falls back to the generic application icon when the resource
/// is unavailable so the tray and window chrome stay functional; a null return
/// means even the fallback failed and callers decide how to degrade. Both the
/// resource icon (`LR_SHARED`) and the fallback are shared handles that must
/// not be destroyed.
pub fn load_app_icon(small: bool) -> HICON {
    let metric = if small { SM_CXSMICON } else { SM_CXICON };
    // SAFETY: GetSystemMetrics takes no pointers.
    let mut size = unsafe { GetSystemMetrics(metric) };
    if size <= 0 {
        size = if small { 16 } else { 32 };
    }
    // SAFETY: A null module name requests the executable's own module handle.
    let module = unsafe { GetModuleHandleW(ptr::null()) };
    if !module.is_null() {
        // SAFETY: Resource id 1 is embedded at link time by askbridge.rc; the
        // LR_SHARED handle is process-wide shared and must not be destroyed.
        let icon = unsafe {
            LoadImageW(
                module,
                APP_ICON_RESOURCE_ID as _,
                IMAGE_ICON,
                size,
                size,
                LR_SHARED,
            )
        };
        if !icon.is_null() {
            return icon as HICON;
        }
    }
    tracing::warn!("embedded application icon unavailable; using the generic icon");
    // SAFETY: Loading the shared application icon with a null module handle is supported.
    unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) }
}
