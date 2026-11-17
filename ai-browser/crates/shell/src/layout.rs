//! Webview geometry. Pure math so it is unit-testable without a window.

use tao::dpi::PhysicalSize;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::Rect;

pub const HEADER_H: i32 = 36;
/// The tab strip above the toolbar in the default (horizontal) tab layout.
pub const TABSTRIP_H: i32 = 36;
pub const RAIL_W: i32 = 220;
pub const RAIL_W_COLLAPSED: i32 = 44;

/// Where the tabs live: a Chrome/Edge-style strip above the toolbar (the default), or a
/// vertical rail at the left (expanded or icon-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tabs {
    Strip,
    Rail,
    RailCollapsed,
}

impl From<bool> for Tabs {
    /// A vertical rail; the flag says whether it is collapsed.
    fn from(collapsed: bool) -> Self {
        if collapsed { Tabs::RailCollapsed } else { Tabs::Rail }
    }
}

impl Tabs {
    pub fn rail_w(self) -> i32 {
        match self {
            Tabs::Strip => 0,
            Tabs::Rail => RAIL_W,
            Tabs::RailCollapsed => RAIL_W_COLLAPSED,
        }
    }
    /// Height of everything above the page: the toolbar, plus the strip when horizontal.
    pub fn header_h(self) -> i32 {
        match self {
            Tabs::Strip => TABSTRIP_H + HEADER_H,
            _ => HEADER_H,
        }
    }
}
pub const PANEL_W: i32 = 360;
pub const AI_W: i32 = 420;
pub const AI_MIN_W: i32 = 320;
/// The page keeps a sliver beside the docked pane so it never collapses to nothing.
pub const PAGE_MIN_W: i32 = 80;

static AI_WIDTH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(AI_W);

/// The docked AI pane's current width in logical units (user-resizable).
pub fn ai_width() -> i32 {
    AI_WIDTH.load(std::sync::atomic::Ordering::Relaxed)
}

/// Clamp a wanted pane width for a window `shell_w` wide with a `rail` of that width,
/// store it, and return the stored value. There is no fixed maximum: the pane may
/// take everything except the rail and a sliver of page.
pub fn set_ai_width(wanted: i32, shell_w: i32, rail: i32) -> i32 {
    let max = (shell_w - rail - PAGE_MIN_W).max(AI_MIN_W);
    let w = wanted.clamp(AI_MIN_W, max);
    AI_WIDTH.store(w, std::sync::atomic::Ordering::Relaxed);
    w
}
pub const PALETTE_W: i32 = 640;
pub const PALETTE_H: i32 = 460;
pub const ASK_W: i32 = 560;
pub const ASK_H: i32 = 420;
pub const DROPDOWN_MIN_W: i32 = 480;
pub const DROPDOWN_MAX_W: i32 = 900;
pub const DROPDOWN_MAX_H: i32 = 480;

/// Which shell surface owns the docked/floating area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    None,
    Downloads,
    History,
    /// The AI side pane: docked on the right, the page shrinks beside it.
    Ai,
}

/// A floating surface drawn by the overlay webview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Closed,
    /// Anchored under the omnibox: `(x, y, width)` in logical units.
    Dropdown { x: i32, y: i32, w: i32 },
    Palette,
    /// The AI composer: question field, answer, citation chips. Centred on the
    /// same column as the palette and docked near the bottom edge, so it never
    /// lands on top of the dropdown that opens under the omnibox.
    Ask,
    /// A surface pinned beside the page text it is about: the selection toolbar and
    /// the inline answer card. `(x, y)` is the wanted top-left in logical units
    /// relative to the window; the size is the surface's own.
    Anchored { x: i32, y: i32, w: i32, h: i32 },
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
    pub header_h: i32,
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
pub fn compute(size: PhysicalSize<u32>, scale: f64, tabs: impl Into<Tabs>, panel: Panel, overlay: Overlay) -> Layout {
    let tabs = tabs.into();
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let w = (size.width as f64 / scale).round() as i32;
    let h = (size.height as f64 / scale).round() as i32;
    let rail = tabs.rail_w();
    let header_h = tabs.header_h();
    let dock_w = match panel {
        Panel::Downloads => PANEL_W,
        Panel::Ai => ai_width(),
        _ => 0,
    };

    let page_x = rail;
    let page_y = header_h;
    let page_w = (w - rail - dock_w).max(0);
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
            (page_x + (page_w - pw) / 2, header_h + 24, pw, ph)
        }
        Overlay::Anchored { x, y, w: aw, h: ah } => {
            // Kept entirely inside the page area, so it can never cover the rail
            // or the header, and shrinks rather than overflowing a small window.
            let width = aw.min(page_w).max(0);
            let height = ah.min(page_h).max(0);
            let max_x = (page_x + page_w - width).max(page_x);
            let max_y = (page_y + page_h - height).max(page_y);
            (x.clamp(page_x, max_x), y.clamp(page_y, max_y), width, height)
        }
        Overlay::Ask => {
            let aw = ASK_W.min(w - 40).min(page_w).max(0);
            // A window shorter than the card gets a shorter card rather than a card
            // under the omnibox, so the top clamp is the header plus its own margin.
            let ah = ASK_H.min((h - header_h - 48).max(0)).max(0);
            let ay = (h - ah - 24).max(header_h);
            (page_x + (page_w - aw) / 2, ay, aw, ah)
        }
    };

    Layout {
        shell_w: w.max(0),
        shell_h: h.max(0),
        header_h,
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

/// Width of the ring the shell draws around each cell (it shows the focused cell). The
/// page webview sits inside it; a cell has no chrome of its own.
pub const CELL_RING: i32 = 2;
/// Frames touch: two neighbouring pages are separated by exactly two rings (4 px), the
/// strip that doubles as the drag handle between them.
pub const CELL_GAP: i32 = 0;

/// A plain rectangle in logical units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rc {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One canvas cell: the frame (with its focus ring) and the page body inside it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellGeom {
    pub frame: Rc,
    pub body: Rc,
    /// A zoomed-out sibling: it keeps its page alive but takes no space.
    pub hidden: bool,
}

/// Smallest share a column or row can be dragged down to.
pub const MIN_SHARE: f64 = 0.12;

/// Normalised shares for `n` tracks: `weights` when it fits, otherwise equal.
pub fn shares(weights: &[f64], n: usize) -> Vec<f64> {
    if weights.len() == n && weights.iter().all(|w| w.is_finite() && *w > 0.0) {
        let sum: f64 = weights.iter().sum();
        weights.iter().map(|w| w / sum).collect()
    } else {
        vec![1.0 / n.max(1) as f64; n]
    }
}

/// Where track `i` of `n` starts and how long it is. Edges are cumulative so tracks
/// tile exactly.
fn split(origin: i32, total: i32, shares: &[f64], i: usize) -> (i32, i32) {
    let total = total.max(0);
    let edge = |k: usize| -> i32 {
        if k >= shares.len() {
            total
        } else {
            (shares[..k].iter().sum::<f64>() * f64::from(total)).round() as i32
        }
    };
    let (start, end) = (edge(i), edge(i + 1));
    (origin + start, (end - start).max(0))
}

/// A draggable boundary between two tracks. `index` is the track before it. A vertical
/// divider belongs to one `row`; a horizontal one spans the whole width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    pub vertical: bool,
    pub index: usize,
    pub row: usize,
    pub rect: Rc,
}

fn row_shares(col_ws: &[Vec<f64>], row: usize, cols: usize) -> Vec<f64> {
    shares(col_ws.get(row).map_or(&[][..], Vec::as_slice), cols)
}

/// The dividers of a `cols × rows` grid laid over `area` (not for a zoomed canvas).
/// Every row has its own column dividers, so rows resize independently.
pub fn canvas_dividers(area: Rc, cols: usize, rows: usize, col_ws: &[Vec<f64>], row_h: &[f64]) -> Vec<Divider> {
    let (cols, rows) = (cols.max(1), rows.max(1));
    let rs = shares(row_h, rows);
    let mut out = Vec::new();
    for r in 0..rows {
        let (y, h) = split(area.y, area.h, &rs, r);
        let cs = row_shares(col_ws, r, cols);
        for i in 0..cols - 1 {
            let (x, w) = split(area.x, area.w, &cs, i);
            out.push(Divider { vertical: true, index: i, row: r, rect: Rc { x: x + w - CELL_RING, y, w: 2 * CELL_RING, h } });
        }
    }
    for i in 0..rows - 1 {
        let (y, h) = split(area.y, area.h, &rs, i);
        out.push(Divider { vertical: false, index: i, row: 0, rect: Rc { x: area.x, y: y + h - CELL_RING, w: area.w, h: 2 * CELL_RING } });
    }
    out
}

/// Move the boundary after track `index` to `pos` (a coordinate along the axis, in the
/// same units as `origin`/`total`), keeping every track above `MIN_SHARE`. Returns the
/// new weights.
pub fn drag_divider(weights: &[f64], n: usize, index: usize, pos: f64, origin: i32, total: i32) -> Vec<f64> {
    let mut s = shares(weights, n);
    if index + 1 >= n || total <= 0 {
        return s;
    }
    let before: f64 = s[..index].iter().sum();
    let pair = s[index] + s[index + 1];
    let want = ((pos - f64::from(origin)) / f64::from(total) - before).clamp(MIN_SHARE, (pair - MIN_SHARE).max(MIN_SHARE));
    s[index] = want;
    s[index + 1] = pair - want;
    s
}

/// Tile `cols × rows` cells over the page area, row-major. With `zoom = Some(i)` that cell
/// fills the whole area and the others are hidden. The cells always tile exactly: no
/// cell overlaps another and none leaves the area.
pub fn canvas_cells(
    area: Rc,
    cols: usize,
    rows: usize,
    zoom: Option<usize>,
    col_ws: &[Vec<f64>],
    row_h: &[f64],
) -> Vec<CellGeom> {
    let (cols, rows) = (cols.max(1), rows.max(1));
    let rs = shares(row_h, rows);
    let make = |frame: Rc, ring: i32| {
        let ring = ring.min(frame.w / 2).min(frame.h / 2).max(0);
        CellGeom {
            frame,
            body: Rc { x: frame.x + ring, y: frame.y + ring, w: frame.w - 2 * ring, h: frame.h - 2 * ring },
            hidden: false,
        }
    };
    (0..cols * rows)
        .map(|n| match zoom {
            Some(z) if z == n => make(area, 0),
            Some(_) => CellGeom { hidden: true, ..Default::default() },
            None => {
                let (c, r) = (n % cols, n / cols);
                let cs = row_shares(col_ws, r, cols);
                let (x, w) = split(area.x, area.w, &cs, c);
                let (y, h) = split(area.y, area.h, &rs, r);
                make(Rc { x, y, w, h }, CELL_RING)
            }
        })
        .collect()
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
    fn horizontal_tabs_put_the_strip_above_the_toolbar_and_use_the_full_width() {
        let l = compute(size(1280, 820), 1.0, Tabs::Strip, Panel::None, Overlay::Closed);
        assert_eq!((l.page_x, l.page_y), (0, 72));
        assert_eq!((l.page_w, l.page_h), (1280, 748));
        assert_eq!(l.header_h, 72);
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

    /// The composer floats: same centred column as the palette, docked near the
    /// bottom edge so it cannot land on the dropdown that opens under the omnibox,
    /// and it never reflows the page it is answering about.
    #[test]
    fn ask_card_is_centred_bottom_docked_and_never_shrinks_the_page() {
        let closed = compute(size(1600, 1000), 1.0, false, Panel::None, Overlay::Closed);
        let ask = compute(size(1600, 1000), 1.0, false, Panel::None, Overlay::Ask);
        assert_eq!((ask.overlay_w, ask.overlay_h), (ASK_W, ASK_H));
        // page area is [220, 1600) → 1380 wide; a 560 card centres at 220 + 410.
        assert_eq!(ask.overlay_x, 630, "centred on the page, like the palette");
        assert_eq!(ask.overlay_y, 1000 - ASK_H - 24);
        assert_eq!(
            (ask.page_w, ask.page_h),
            (closed.page_w, closed.page_h),
            "an ask card floats; the page it cites is not resized"
        );
    }

    /// A window shorter than the card shrinks the card instead of pushing it under
    /// the header, and a window narrower than the card shrinks it to the page width.
    #[test]
    fn ask_card_shrinks_instead_of_covering_the_header() {
        let l = compute(size(600, 300), 1.0, false, Panel::None, Overlay::Ask);
        assert!(l.overlay_h > 0 && l.overlay_h < ASK_H);
        assert!(l.overlay_y >= HEADER_H, "the omnibox stays reachable");
        assert_eq!(l.overlay_w, l.page_w, "capped at the page width");
        assert!(l.overlay_x >= 0 && l.overlay_y + l.overlay_h <= l.shell_h);
    }

    #[test]
    fn dropdown_width_is_clamped_and_never_exceeds_page() {
        let wide = compute(size(2000, 1000), 1.0, false, Panel::None, Overlay::Dropdown { x: 200, y: 40, w: 1500 });
        assert_eq!(wide.overlay_w, DROPDOWN_MAX_W);
        let narrow = compute(size(2000, 1000), 1.0, false, Panel::None, Overlay::Dropdown { x: 200, y: 40, w: 100 });
        assert_eq!(narrow.overlay_w, DROPDOWN_MIN_W);
    }

    #[test]
    fn canvas_cells_tile_without_overlap_inside_the_area() {
        let area = Rc { x: 220, y: 36, w: 1060, h: 784 };
        let cells = canvas_cells(area, 3, 2, None, &[], &[]);
        assert_eq!(cells.len(), 6);
        for (i, a) in cells.iter().enumerate() {
            let f = a.frame;
            assert!(f.w > 0 && f.h > 0);
            assert!(f.x >= area.x && f.y >= area.y && f.x + f.w <= area.x + area.w && f.y + f.h <= area.y + area.h);
            assert_eq!(a.body.x, f.x + CELL_RING);
            assert_eq!(a.body.h, f.h - 2 * CELL_RING);
            for b in cells.iter().skip(i + 1) {
                let g = b.frame;
                let apart = f.x + f.w <= g.x || g.x + g.w <= f.x || f.y + f.h <= g.y || g.y + g.h <= f.y;
                assert!(apart, "cells overlap: {f:?} {g:?}");
            }
        }
        // Row-major: cell 1 is right of cell 0; cell 3 is below cell 0.
        assert!(cells[1].frame.x > cells[0].frame.x && cells[1].frame.y == cells[0].frame.y);
        assert!(cells[3].frame.y > cells[0].frame.y && cells[3].frame.x == cells[0].frame.x);
    }

    #[test]
    fn zoomed_canvas_gives_one_cell_everything() {
        let area = Rc { x: 0, y: 0, w: 900, h: 600 };
        let cells = canvas_cells(area, 3, 2, Some(4), &[], &[]);
        assert_eq!(cells[4].frame, area);
        assert_eq!(cells[4].body, area);
        assert!(!cells[4].hidden);
        assert!(cells.iter().enumerate().all(|(i, c)| i == 4 || c.hidden));
    }

    #[test]
    fn canvas_in_a_tiny_area_never_goes_negative() {
        for c in canvas_cells(Rc { x: 0, y: 0, w: 10, h: 10 }, 4, 3, None, &[], &[]) {
            assert!(c.frame.w >= 0 && c.frame.h >= 0 && c.body.w >= 0 && c.body.h >= 0);
        }
    }

    #[test]
    fn tiny_window_never_yields_negative_sizes() {
        let l = compute(size(200, 100), 1.0, false, Panel::Downloads, Overlay::Palette);
        assert!(l.page_w >= 0 && l.page_h >= 0 && l.overlay_w >= 0 && l.overlay_h >= 0);
    }
    #[test]
    fn dragging_a_divider_resizes_the_two_neighbours_and_keeps_a_minimum() {
        let area = Rc { x: 100, y: 0, w: 1000, h: 500 };
        let w = drag_divider(&[], 2, 0, 700.0, area.x, area.w);
        let cells = canvas_cells(area, 2, 1, None, &[w], &[]);
        assert_eq!(cells[0].frame.w, 600);
        assert_eq!(cells[0].frame.w + cells[1].frame.w, 1000);
        let w = drag_divider(&[], 2, 0, 0.0, area.x, area.w);
        assert!((w[0] - MIN_SHARE).abs() < 1e-9);
        let d = canvas_dividers(area, 2, 2, &[], &[]);
        assert_eq!(d.iter().filter(|d| d.vertical).count(), 2);
        assert!(d.iter().any(|d| d.vertical && d.rect.x == 100 + 500 - CELL_RING));
    }
    #[test]
    fn each_row_splits_its_columns_independently() {
        let area = Rc { x: 0, y: 0, w: 1000, h: 400 };
        let top = drag_divider(&[], 2, 0, 700.0, area.x, area.w);
        let cells = canvas_cells(area, 2, 2, None, &[top], &[]);
        assert_eq!(cells[0].frame.w, 700);
        assert_eq!(cells[2].frame.w, 500, "the second row keeps equal columns");
        assert_eq!(cells[3].frame.w, 500);
    }
}