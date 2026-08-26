pub(crate) mod encoder;
mod monitor;
mod overlay;
pub(crate) mod screen;
mod toolbar_webview;

use askbridge_core::{CapturedImage, Result};
use tracing::info;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND},
    UI::WindowsAndMessaging::WM_APP,
};

use crate::clipboard_image;

pub const WM_CAPTURE_BUSY: u32 = WM_APP + 3;

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
        let Some(source_rect) = overlay::select_region(self.instance, self.owner, &layout)? else {
            return Ok(CaptureOutcome::Cancelled);
        };
        let raw = screen::capture_screen_rect(source_rect)?;
        let captured = encoder::captured_image(raw, source_rect)?;
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
        let raw = screen::capture_screen_rect(selection.rect)?;
        let captured = encoder::captured_image(raw, selection.rect)?;
        match selection.action {
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
