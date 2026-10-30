//! DPI scaling for the settings host window.
//!
//! The settings page itself is laid out in CSS pixels and scales with the
//! WebView2 zoom, so only the host window's size and the DWM chrome need
//! logical-to-physical conversion.

use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};

#[derive(Clone, Copy)]
pub(super) struct UiScale {
    dpi: u32,
}

impl UiScale {
    pub(super) fn system() -> Self {
        // SAFETY: The process selects Per-Monitor V2 awareness before creating any window.
        let dpi = unsafe { GetDpiForSystem() };
        Self {
            dpi: if dpi == 0 { 96 } else { dpi },
        }
    }

    /// Reads the DPI of an existing window so the host matches the monitor it
    /// is about to appear on.
    pub(super) fn for_window(window: windows_sys::Win32::Foundation::HWND) -> Self {
        // SAFETY: window is a live HWND owned by this process.
        let dpi = unsafe { GetDpiForWindow(window) };
        Self {
            dpi: if dpi == 0 { 96 } else { dpi },
        }
    }

    pub(super) fn px(self, value: i32) -> i32 {
        ((i64::from(value) * i64::from(self.dpi) + 48) / 96) as i32
    }

    pub(super) const fn dpi(self) -> u32 {
        self.dpi
    }

    /// Physical size for a logical size, as `(width, height)`.
    pub(super) fn size(self, width: i32, height: i32) -> (i32, i32) {
        (self.px(width), self.px(height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale(dpi: u32) -> UiScale {
        UiScale { dpi }
    }

    #[test]
    fn scales_logical_pixels_at_common_dpi_values() {
        assert_eq!(scale(96).px(900), 900);
        assert_eq!(scale(120).px(900), 1125);
        assert_eq!(scale(144).px(900), 1350);
        assert_eq!(scale(192).px(660), 1320);
    }

    #[test]
    fn scales_the_window_size_as_a_pair() {
        assert_eq!(scale(144).size(900, 660), (1350, 990));
        assert_eq!(scale(96).size(900, 660), (900, 660));
    }
}
