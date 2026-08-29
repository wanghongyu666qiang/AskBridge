// Pure layout math for the fallback toolbar and selection handles.
// No Win32 calls; everything here is unit-testable as plain data.

use windows_sys::Win32::Foundation::RECT;

use crate::capture::toolbar_webview;

const TOOLBAR_GAP: i32 = 12;
const DROPDOWN_ROW_HEIGHT: i32 = 34;
const MORE_WIDTH: i32 = 0;
const PROVIDER_WIDTH: i32 = 188;
const COPY_WIDTH: i32 = 88;
const CANCEL_WIDTH: i32 = 84;
const ASK_WIDTH: i32 = 112;
const BUTTON_GAP: i32 = 6;
const TOOLBAR_PADDING: i32 = 20;

pub(super) struct ToolbarLayout {
    pub(super) outer: RECT,
    pub(super) more: RECT,
    pub(super) provider: RECT,
    pub(super) copy: RECT,
    pub(super) cancel: RECT,
    pub(super) ask: RECT,
    pub(super) dropdown_bounds: RECT,
    pub(super) dropdown_rects: Vec<RECT>,
}

pub(super) fn toolbar_layout(
    client: &RECT,
    selection_rect: &RECT,
    provider_count: usize,
    toolbar_size: (i32, i32),
) -> ToolbarLayout {
    let (total_width, toolbar_height) = toolbar_size;
    // On clients narrower than the toolbar the clamp range inverts; pin to the
    // left edge and let the toolbar clip instead of panicking.
    let min_right = client.left + total_width.saturating_add(8);
    let max_right = client.right - 8;
    let right = if max_right < min_right {
        min_right
    } else {
        selection_rect.right.clamp(min_right, max_right)
    };
    let left = right - total_width;
    let below = selection_rect.bottom + TOOLBAR_GAP;
    let above = selection_rect.top - toolbar_height - TOOLBAR_GAP;
    let dropdown_clearance = if provider_count > 1 {
        (provider_count as i32 * DROPDOWN_ROW_HEIGHT + 12).min(180)
    } else {
        0
    };
    let preferred_top = if below + toolbar_height + dropdown_clearance <= client.bottom - 8 {
        below
    } else {
        above.max(client.top + 8)
    };
    let min_top = client.top + 8;
    let max_top = (client.bottom - toolbar_height - 8).max(min_top);
    let top = preferred_top.clamp(min_top, max_top);
    let outer = RECT {
        left,
        top,
        right: left + total_width,
        bottom: top + toolbar_height,
    };
    let button_top = top + 14;
    let button_height = 40;
    let copy = RECT {
        left: left + TOOLBAR_PADDING,
        top: button_top,
        right: left + TOOLBAR_PADDING + COPY_WIDTH,
        bottom: button_top + button_height,
    };
    let cancel = offset_rect(&copy, COPY_WIDTH + BUTTON_GAP, CANCEL_WIDTH);
    let provider = offset_rect(&cancel, CANCEL_WIDTH + BUTTON_GAP * 2, PROVIDER_WIDTH);
    let ask = offset_rect(&provider, PROVIDER_WIDTH + BUTTON_GAP, ASK_WIDTH);
    // The former fallback-only "More" placeholder had no action. Keep an
    // empty rect so the shared hit-testing shape stays stable without
    // spending visible space on a dead control.
    let more = RECT {
        left,
        top: button_top,
        right: left + MORE_WIDTH,
        bottom: button_top + button_height,
    };
    let dropdown_top =
        if outer.bottom + DROPDOWN_ROW_HEIGHT * provider_count as i32 <= client.bottom - 8 {
            outer.bottom + 4
        } else {
            outer.top - DROPDOWN_ROW_HEIGHT * provider_count as i32 - 4
        };
    let dropdown_bounds = RECT {
        left: provider.left,
        top: dropdown_top,
        right: provider.right,
        bottom: dropdown_top + DROPDOWN_ROW_HEIGHT * provider_count as i32,
    };
    let dropdown_rects = (0..provider_count)
        .map(|index| RECT {
            left: dropdown_bounds.left,
            top: dropdown_bounds.top + DROPDOWN_ROW_HEIGHT * index as i32,
            right: dropdown_bounds.right,
            bottom: dropdown_bounds.top + DROPDOWN_ROW_HEIGHT * (index as i32 + 1),
        })
        .collect::<Vec<_>>();
    ToolbarLayout {
        outer,
        more,
        provider,
        copy,
        cancel,
        ask,
        dropdown_bounds,
        dropdown_rects,
    }
}

fn offset_rect(previous: &RECT, delta_x: i32, width: i32) -> RECT {
    RECT {
        left: previous.left + delta_x,
        top: previous.top,
        right: previous.left + delta_x + width,
        bottom: previous.bottom,
    }
}

pub(super) fn inset_rect(rect: &RECT, value: i32) -> RECT {
    RECT {
        left: rect.left + value,
        top: rect.top + value,
        right: rect.right - value,
        bottom: rect.bottom - value,
    }
}

pub(super) fn hit_dropdown(rects: &[RECT], point: (i32, i32)) -> Option<usize> {
    rects.iter().position(|rect| point_in_rect(point, rect))
}

pub(super) fn point_in_rect(point: (i32, i32), rect: &RECT) -> bool {
    point.0 >= rect.left && point.0 < rect.right && point.1 >= rect.top && point.1 < rect.bottom
}

pub(super) fn fallback_toolbar_size() -> (i32, i32) {
    // The fallback is a rendering substitution, not a separate toolbar.
    // Keep its outer frame identical to the WebView surface so placement and
    // hit testing cannot jump when WebView2 is unavailable.
    toolbar_webview::preferred_size()
}

pub(super) fn selection_handle_points(rect: &RECT) -> [(i32, i32); 8] {
    let mid_x = rect.left + (rect.right - rect.left) / 2;
    let mid_y = rect.top + (rect.bottom - rect.top) / 2;
    [
        (rect.left, rect.top),
        (mid_x, rect.top),
        (rect.right, rect.top),
        (rect.right, mid_y),
        (rect.right, rect.bottom),
        (mid_x, rect.bottom),
        (rect.left, rect.bottom),
        (rect.left, mid_y),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_flips_above_when_provider_menu_would_be_clipped() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        let selection = RECT {
            left: 120,
            top: 100,
            right: 1120,
            bottom: 650,
        };

        let layout = toolbar_layout(&client, &selection, 4, toolbar_webview::preferred_size());

        assert_eq!(layout.outer.bottom, selection.top - TOOLBAR_GAP);
    }

    #[test]
    fn toolbar_right_edge_tracks_selection_right_edge() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 1440,
            bottom: 900,
        };
        let selection = RECT {
            left: 180,
            top: 120,
            right: 1180,
            bottom: 640,
        };

        let layout = toolbar_layout(&client, &selection, 4, toolbar_webview::preferred_size());

        assert_eq!(layout.outer.right, selection.right);
    }

    #[test]
    fn narrow_client_pins_toolbar_instead_of_panicking() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 400,
            bottom: 300,
        };
        let selection = RECT {
            left: 40,
            top: 60,
            right: 200,
            bottom: 120,
        };

        let layout = toolbar_layout(&client, &selection, 1, (680, 46));

        assert_eq!(layout.outer.left, client.left + 8);
        assert_eq!(layout.outer.right, layout.outer.left + 680);
    }

    #[test]
    fn negative_origin_narrow_client_pins_to_left_edge() {
        let client = RECT {
            left: -1920,
            top: 0,
            right: -1600,
            bottom: 300,
        };
        let selection = RECT {
            left: -1880,
            top: 50,
            right: -1750,
            bottom: 110,
        };

        let layout = toolbar_layout(&client, &selection, 2, (680, 46));

        assert_eq!(layout.outer.left, client.left + 8);
    }

    #[test]
    fn fallback_toolbar_matches_the_dark_web_toolbar_action_order() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        let selection = RECT {
            left: 100,
            top: 100,
            right: 1000,
            bottom: 500,
        };

        let layout = toolbar_layout(&client, &selection, 3, fallback_toolbar_size());

        assert!(layout.copy.right < layout.cancel.left);
        assert!(layout.cancel.right < layout.provider.left);
        assert!(layout.provider.right < layout.ask.left);
        assert_eq!(layout.more.left, layout.more.right);
        assert_eq!(fallback_toolbar_size(), toolbar_webview::preferred_size());
    }
}
