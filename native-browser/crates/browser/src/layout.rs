//! Webview geometry. Pure math so it is unit-testable without a window.

use tao::dpi::PhysicalSize;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::Rect;

pub const HEADER_H: i32 = 36;
pub const RAIL_W: i32 = 220;
pub const RAIL_W_COLLAPSED: i32 = 44;
pub const FIND_BAR_H: i32 = 40;
pub const PANEL_W: i32 = 360;
pub const PALETTE_W: i32 = 640;
pub const PALETTE_H: i32 = 460;
pub const DROPDOWN_MIN_W: i32 = 480;
pub const DROPDOWN_MAX_W: i32 = 900;
pub const DROPDOWN_MAX_H: i32 = 480;

/// Which shell surface owns the docked/floating area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    None,
    Find,
    Downloads,
    History,
}

/// A floating surface drawn by the overlay webview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Closed,
    /// Anchored under the omnibox: `(x, y, width)` in logical units.
    Dropdown { x: i32, y: i32, w: i32 },
    Palette,
}

impl Overlay {
    pub fn is_open(self) -> bool {
        !matches!(self, Overlay::Closed)
    }
}

/// Geometry in logical (DIP) units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Layout {
    pub shell_w: i32,
    pub shell_h: i32,
    pub page_x: i32,
    pub page_y: i32,
    pub page_w: i32,
    pub page_h: i32,
    pub overlay_x: i32,
    pub overlay_y: i32,
    pub overlay_w: i32,
    pub overlay_h: i32,
}

/// Compute the layout for a client size, DPI scale, rail state, panel, and
/// overlay. The page webview is shrunk only for *docked* surfaces (§3.1);
/// floating ones live in the overlay webview and leave the page alone.
pub fn compute(size: PhysicalSize<u32>, scale: f64, rail_collapsed: bool, panel: Panel, overlay: Overlay) -> Layout {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let w = (size.width as f64 / scale).round() as i32;
    let h = (size.height as f64 / scale).round() as i32;
    let rail = if rail_collapsed { RAIL_W_COLLAPSED } else { RAIL_W };
    let find_open = panel == Panel::Find;
    let docked = panel == Panel::Downloads;

    let page_x = rail;
    let page_y = HEADER_H + if find_open { FIND_BAR_H } else { 0 };
    let page_w = (w - rail - if docked { PANEL_W } else { 0 }).max(0);
    let page_h = (h - page_y).max(0);

    let (ox, oy, ow, oh) = match overlay {
        Overlay::Closed => (0, 0, 0, 0),
        Overlay::Dropdown { x, y, w: dw } => {
            let width = dw.clamp(DROPDOWN_MIN_W, DROPDOWN_MAX_W).min(page_w);
            let height = DROPDOWN_MAX_H.min((h - y).max(0));
            (x, y, width, height)
        }
        Overlay::Palette => {
            let pw = PALETTE_W.min(w - 40).max(0);
            let ph = PALETTE_H.min(h - 80).max(0);
            (page_x + (page_w - pw) / 2, HEADER_H + 24, pw, ph)
        }
    };

    Layout {
        shell_w: w.max(0),
        shell_h: h.max(0),
        page_x,
        page_y,
        page_w,
        page_h,
        overlay_x: ox,
        overlay_y: oy,
        overlay_w: ow,
        overlay_h: oh,
    }
}

pub fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(w.max(0), h.max(0)).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: u32, h: u32) -> PhysicalSize<u32> {
        PhysicalSize::new(w, h)
    }

    #[test]
    fn page_sits_below_header_and_right_of_rail() {
        let l = compute(size(1280, 820), 1.0, false, Panel::None, Overlay::Closed);
        assert_eq!((l.page_x, l.page_y), (220, 36));
        assert_eq!((l.page_w, l.page_h), (1060, 784));
        assert_eq!(l.shell_w, 1280);
        assert_eq!(l.overlay_w, 0);
    }

    #[test]
    fn find_bar_shrinks_page_by_40() {
        let l = compute(size(1280, 820), 1.0, false, Panel::Find, Overlay::Closed);
        assert_eq!(l.page_y, 76);
        assert_eq!(l.page_h, 744);
    }

    #[test]
    fn downloads_docks_and_shrinks_width() {
        let l = compute(size(1280, 820), 1.0, false, Panel::Downloads, Overlay::Closed);
        assert_eq!(l.page_w, 700);
    }

    #[test]
    fn history_does_not_shrink_the_page_it_overlays() {
        let base = compute(size(1280, 820), 1.0, false, Panel::None, Overlay::Closed);
        let hist = compute(size(1280, 820), 1.0, false, Panel::History, Overlay::Closed);
        assert_eq!(base.page_w, hist.page_w);
        assert_eq!(base.page_h, hist.page_h);
    }

    #[test]
    fn collapsed_rail_is_44() {
        let l = compute(size(1280, 820), 1.0, true, Panel::None, Overlay::Closed);
        assert_eq!(l.page_x, 44);
        assert_eq!(l.page_w, 1236);
    }

    #[test]
    fn dpi_scale_maps_physical_to_logical() {
        let l = compute(size(3840, 2160), 2.0, false, Panel::None, Overlay::Closed);
        assert_eq!(l.shell_w, 1920);
        assert_eq!(l.page_w, 1700);
    }

    #[test]
    fn palette_is_centered_over_the_page_area() {
        let l = compute(size(1600, 1000), 1.0, false, Panel::None, Overlay::Palette);
        assert_eq!(l.overlay_w, 640);
        // page area is [220, 1600) → width 1380; centered: 220 + (1380-640)/2
        assert_eq!(l.overlay_x, 590);
    }

    #[test]
    fn dropdown_width_is_clamped_and_never_exceeds_page() {
        let wide = compute(size(2000, 1000), 1.0, false, Panel::None, Overlay::Dropdown { x: 200, y: 40, w: 1500 });
        assert_eq!(wide.overlay_w, DROPDOWN_MAX_W);
        let narrow = compute(size(2000, 1000), 1.0, false, Panel::None, Overlay::Dropdown { x: 200, y: 40, w: 100 });
        assert_eq!(narrow.overlay_w, DROPDOWN_MIN_W);
    }

    #[test]
    fn tiny_window_never_yields_negative_sizes() {
        let l = compute(size(200, 100), 1.0, false, Panel::Downloads, Overlay::Palette);
        assert!(l.page_w >= 0 && l.page_h >= 0 && l.overlay_w >= 0 && l.overlay_h >= 0);
    }
}