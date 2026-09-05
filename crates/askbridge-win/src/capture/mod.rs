pub(crate) mod encoder;
mod monitor;
mod overlay;
pub(crate) mod screen;
mod toolbar_html;
mod toolbar_webview;

use askbridge_core::{CapturedImage, Result};
use tracing::info;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND},
    UI::WindowsAndMessaging::WM_APP,
};

use crate::clipboard_image;

pub const WM_CAPTURE_BUSY: u32 = WM_APP + 3;

fn selected_pixels(
    source_rect: askbridge_core::ScreenRect,
    frozen_pixels: Option<screen::RawBgraImage>,
    capture_live: impl FnOnce(askbridge_core::ScreenRect) -> Result<screen::RawBgraImage>,
) -> Result<screen::RawBgraImage> {
    match frozen_pixels {
        Some(raw) => Ok(raw),
        None => capture_live(source_rect),
    }
}

pub enum CaptureOutcome {
    Captured(CapturedImage),
    CapturedForProvider {
        image: CapturedImage,
        provider_id: String,
    },
    CopiedToClipboard,
    Cancelled,
}

pub struct CaptureProviderChoice {
    pub id: String,
    pub display_name: String,
    pub selected: bool,
}

pub struct CaptureService {
    instance: HINSTANCE,
    owner: HWND,
}

impl CaptureService {
    pub fn new(instance: HINSTANCE, owner: HWND) -> Result<Self> {
        overlay::register_class(instance)?;
        Ok(Self { instance, owner })
    }

    /// Displays the region selector and returns the selected pixels in memory as RGBA.
    ///
    /// Capture never mutates the clipboard. Clipboard fallback belongs to the later dispatch flow.
    pub fn capture(&self) -> Result<CaptureOutcome> {
        let layout = monitor::DesktopLayout::enumerate()?;
        info!(
            monitor_count = layout.monitors.len(),
            virtual_left = layout.virtual_bounds.left,
            virtual_top = layout.virtual_bounds.top,
            virtual_width = layout.virtual_bounds.width,
            virtual_height = layout.virtual_bounds.height,
            "capture overlay starting"
        );
        let Some(selection) = overlay::select_region(self.instance, self.owner, &layout)? else {
            return Ok(CaptureOutcome::Cancelled);
        };
        let raw = selected_pixels(
            selection.rect,
            selection.frozen_pixels,
            screen::capture_screen_rect,
        )?;
        let captured = encoder::captured_image(raw, selection.rect)?;
        Ok(CaptureOutcome::Captured(captured))
    }

    pub fn capture_with_toolbar(
        &self,
        providers: Vec<CaptureProviderChoice>,
    ) -> Result<CaptureOutcome> {
        let layout = monitor::DesktopLayout::enumerate()?;
        info!(
            monitor_count = layout.monitors.len(),
            virtual_left = layout.virtual_bounds.left,
            virtual_top = layout.virtual_bounds.top,
            virtual_width = layout.virtual_bounds.width,
            virtual_height = layout.virtual_bounds.height,
            "capture overlay starting with toolbar"
        );
        let providers = providers
            .into_iter()
            .map(|provider| overlay::OverlayProviderChoice {
                id: provider.id,
                display_name: provider.display_name,
                selected: provider.selected,
            })
            .collect::<Vec<_>>();
        let Some(selection) =
            overlay::select_region_with_toolbar(self.instance, self.owner, &layout, providers)?
        else {
            return Ok(CaptureOutcome::Cancelled);
        };
        let overlay::SelectionResult {
            rect,
            action,
            frozen_pixels,
        } = selection;
        let raw = selected_pixels(rect, frozen_pixels, screen::capture_screen_rect)?;
        let captured = encoder::captured_image(raw, rect)?;
        match action {
            overlay::SelectionAction::Ask { provider_id } => {
                Ok(CaptureOutcome::CapturedForProvider {
                    image: captured,
                    provider_id,
                })
            }
            overlay::SelectionAction::Copy => {
                clipboard_image::copy_image_to_clipboard(self.owner, &captured)?;
                Ok(CaptureOutcome::CopiedToClipboard)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_pixel(value: u8) -> screen::RawBgraImage {
        screen::RawBgraImage {
            width: 1,
            height: 1,
            pixels: vec![value; 4],
        }
    }

    #[test]
    fn frozen_selection_pixels_are_preferred_over_live_capture() {
        let rect = askbridge_core::ScreenRect::new(-1500, -100, 1, 1);

        let raw = selected_pixels(rect, Some(raw_pixel(7)), |_| {
            panic!("live screen must not be captured when frozen pixels exist")
        })
        .expect("frozen selection is returned");

        assert_eq!(raw.pixels, vec![7; 4]);
    }

    #[test]
    fn missing_frozen_selection_pixels_fall_back_to_live_capture() {
        let rect = askbridge_core::ScreenRect::new(-1500, -100, 1, 1);

        let raw = selected_pixels(rect, None, |requested| {
            assert_eq!(requested, rect);
            Ok(raw_pixel(9))
        })
        .expect("live fallback is returned");

        assert_eq!(raw.pixels, vec![9; 4]);
    }
}
