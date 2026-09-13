// Overlay painting: double-buffered frame composition, toolbar drawing
// (GDI+ with a plain GDI fallback), and the shared color palette.
//
// GDI objects are expensive to create, and WM_PAINT fires on every mouse
// move during a drag. `PaintCache` keeps the per-window resources (double
// buffer, snapshot DC, font, brushes, pens, alpha-blend source) alive for
// the whole overlay session instead of rebuilding them per paint.

mod cache;
mod frame;
mod primitives;
mod toolbar;

pub(super) use cache::PaintCache;
pub(super) use frame::paint_overlay;

pub(super) const COLOR_KEY: COLORREF = rgb(255, 0, 255);
const COLOR_OVERLAY: COLORREF = rgb(0, 0, 0);
const COLOR_BORDER: COLORREF = rgb(255, 255, 255);
const COLOR_ACCENT_BRIGHT: COLORREF = rgb(226, 109, 69);
const COLOR_LABEL: COLORREF = rgb(32, 32, 32);
const COLOR_LABEL_BORDER: COLORREF = rgb(64, 64, 64);
const COLOR_TOOLBAR: COLORREF = rgb(32, 32, 32);
const COLOR_TOOLBAR_BORDER: COLORREF = rgb(61, 61, 61);
const COLOR_TOOLBAR_TEXT: COLORREF = rgb(245, 245, 247);
const COLOR_TOOLBAR_HOVER: COLORREF = rgb(52, 52, 52);
const COLOR_DROPDOWN_SELECTED: COLORREF = rgb(61, 61, 61);
const COLOR_TOOLBAR_ACCENT: COLORREF = rgb(153, 60, 29);
const ARGB_BORDER: u32 = argb(255, 255, 255, 255);
const ARGB_ACCENT_BRIGHT: u32 = argb(255, 226, 109, 69);
const ARGB_LABEL: u32 = argb(255, 32, 32, 32);
const ARGB_LABEL_BORDER: u32 = argb(255, 64, 64, 64);
const ARGB_TOOLBAR: u32 = argb(255, 32, 32, 32);
const ARGB_TOOLBAR_BORDER: u32 = argb(255, 61, 61, 61);
const ARGB_TOOLBAR_TEXT: u32 = argb(255, 245, 245, 247);
const ARGB_TOOLBAR_HOVER: u32 = argb(255, 52, 52, 52);
const ARGB_DROPDOWN_SELECTED: u32 = argb(255, 61, 61, 61);
const ARGB_TOOLBAR_ACCENT: u32 = argb(255, 153, 60, 29);
pub(super) const OVERLAY_ALPHA: u8 = 145;
// GDI RoundRect takes the corner *diameter*; 24 matches the 12px radius used
// by the GDI+ antialiased path.
const TOOLBAR_RADIUS: i32 = 24;
const HANDLE_RADIUS: i32 = 5;

use windows_sys::Win32::Foundation::COLORREF;

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

const fn argb(alpha: u8, red: u8, green: u8, blue: u8) -> u32 {
    ((alpha as u32) << 24) | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}
