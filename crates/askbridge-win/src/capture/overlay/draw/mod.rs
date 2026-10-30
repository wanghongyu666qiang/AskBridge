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
const COLOR_ACCENT_BRIGHT: COLORREF = rgb(226, 109, 69);
const COLOR_LABEL: COLORREF = rgb(32, 32, 32);
const COLOR_LABEL_BORDER: COLORREF = rgb(64, 64, 64);
/// The measurement readout uses a warm off-white number so it reads as part of
/// AskBridge rather than as a generic tooltip.
const COLOR_LABEL_TEXT: COLORREF = rgb(255, 250, 246);
const COLOR_TOOLBAR: COLORREF = rgb(32, 32, 32);
const COLOR_TOOLBAR_BORDER: COLORREF = rgb(61, 61, 61);
const COLOR_TOOLBAR_TEXT: COLORREF = rgb(245, 245, 247);
const COLOR_TOOLBAR_HOVER: COLORREF = rgb(52, 52, 52);
const COLOR_DROPDOWN_SELECTED: COLORREF = rgb(61, 61, 61);
const COLOR_TOOLBAR_ACCENT: COLORREF = rgb(153, 60, 29);
const ARGB_ACCENT_BRIGHT: u32 = argb(255, 226, 109, 69);
const ARGB_LABEL: u32 = argb(255, 32, 32, 32);
/// Accent hairline around the measurement readout.
const ARGB_LABEL_ACCENT: u32 = argb(200, 226, 109, 69);
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
/// Arm length of each brand corner bracket, in pixels.
const BRACKET_ARM: i32 = 26;
/// Stroke width of the bracket's solid core.
const BRACKET_STROKE: f32 = 3.4;
/// Successive halo strokes: (extra outset, stroke width, alpha). GDI+ has no
/// blur, so a soft glow is layered outward instead. These alphas are the
/// peak of the breathing pulse; see `selection_glow_scale`.
const BRACKET_GLOW: [(f32, f32, u8); 4] = [
    (1.6, 4.0, 92),
    (3.6, 5.0, 58),
    (6.5, 6.5, 30),
    (10.5, 9.0, 14),
];
/// Outward drop shadow that makes the selection read as lifted off the page:
/// (outset, stroke width, alpha), darkest first.
const SELECTION_SHADOW: [(f32, f32, u8); 3] = [(2.0, 7.0, 40), (5.0, 9.0, 24), (9.0, 12.0, 11)];
/// A hairline inside the selection edge. It separates the frozen content from
/// the dimmed surroundings on light backgrounds, where the accent alone does
/// not carry enough contrast.
const ARGB_INNER_HAIRLINE: u32 = argb(110, 255, 255, 255);

use windows_sys::Win32::Foundation::COLORREF;

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

const fn argb(alpha: u8, red: u8, green: u8, blue: u8) -> u32 {
    ((alpha as u32) << 24) | ((red as u32) << 16) | ((green as u32) << 8) | blue as u32
}
