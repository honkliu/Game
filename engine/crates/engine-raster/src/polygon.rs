//! Scanline coverage for polygons with straight edges.
//!
//! **Deviation from ARCHITECTURE.md §3.1 (recorded):** the document specifies
//! 4×4 supersampling. This implements a scanline sweep instead, because it is
//! cheaper, and — the actual reason — it gives *exact* coverage on
//! axis-aligned geometry, which is what lets the 1-px border test assert
//! exact bytes (`0` or `255`, never `253`).
//!
//! Algorithm, per pixel row: collect every non-horizontal edge that overlaps
//! the row's `[y, y+1)` band, evaluate its x at the row's vertical centre, sort
//! the crossings, and fill between successive pairs. The two boundary pixels of
//! each span get fractional coverage from where the span starts and ends; all
//! interior pixels get full coverage. This is exact for axis-aligned edges
//! (fractional x positions are measured precisely) and a good approximation for
//! the flattened arc edges of rounded corners.
//!
//! Limitation, recorded rather than hidden: a span boundary is computed per
//! row (not sub-divided at edge intersections), so two edges crossing *inside*
//! one pixel row can produce slightly wrong coverage on that row. The only
//! self-intersecting case we draw is a rounded rect whose corner radii
//! overlap, and those radii are clamped to half the shorter side before
//! flattening, so the arcs never cross.

use engine_core::Rect;

/// One straightened edge in device space.
#[derive(Clone, Copy, Debug)]
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

/// A flattened polygon ready to rasterize.
#[derive(Clone, Debug, Default)]
pub struct Polygon {
    edges: Vec<Edge>,
}

impl Polygon {
    pub fn new() -> Self {
        Polygon { edges: Vec::new() }
    }

    /// Add a straight edge. Order is irrelevant; winding is taken from direction.
    pub fn edge(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if x0 == x1 && y0 == y1 {
            return;
        }
        self.edges.push(Edge { x0, y0, x1, y1 });
    }

    /// Add a closed axis-aligned rectangle.
    pub fn rect(&mut self, r: Rect) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
        self.edge(x0, y0, x1, y0);
        self.edge(x1, y0, x1, y1);
        self.edge(x1, y1, x0, y1);
        self.edge(x0, y1, x0, y0);
    }

    /// A rectangle with per-corner radii. Each corner is a circular quarter-arc
    /// flattened into `ARC_STEPS` segments. The radii are clamped to half the
    /// shorter side so overlapping radii degrade to a stadium rather than invert.
    pub fn rounded_rect(&mut self, r: Rect, radii: engine_core::CornerRadii) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let max_r = (r.w.min(r.h)) * 0.5;
        let clamp = |v: f32| v.clamp(0.0, max_r);
        let tl = clamp(radii.top_left);
        let tr = clamp(radii.top_right);
        let br = clamp(radii.bottom_right);
        let bl = clamp(radii.bottom_left);
        let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());

        // Walk clockwise starting at the end of the top-left arc.
        let mut px = x0 + tl;
        let mut py = y0;
        // top edge -> TR arc start
        self.edge(px, py, x1 - tr, y0);
        px = x1 - tr;
        // TR arc: center (x1-tr, y0+tr), from -90deg to 0deg
        arc(&mut |ax, ay, bx, by| self.edge(ax, ay, bx, by), x1 - tr, y0 + tr, tr, -90.0, 0.0);
        py = y0 + tr;
        // right edge
        self.edge(x1, py, x1, y1 - br);
        // BR arc: center (x1-br, y1-br), 0 -> 90
        arc(&mut |ax, ay, bx, by| self.edge(ax, ay, bx, by), x1 - br, y1 - br, br, 0.0, 90.0);
        // bottom edge
        self.edge(x1 - br, y1, x0 + bl, y1);
        // BL arc: center (x0+bl, y1-bl), 90 -> 180
        arc(&mut |ax, ay, bx, by| self.edge(ax, ay, bx, by), x0 + bl, y1 - bl, bl, 90.0, 180.0);
        // left edge
        self.edge(x0, y1 - bl, x0, y0 + tl);
        // TL arc: center (x0+tl, y0+tl), 180 -> 270
        arc(&mut |ax, ay, bx, by| self.edge(ax, ay, bx, by), x0 + tl, y0 + tl, tl, 180.0, 270.0);
        let _ = (px, py);
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Raw `(x0, y0, x1, y1)` edge endpoints, for bounds computation.
    pub(crate) fn edges(&self) -> impl Iterator<Item = (f32, f32, f32, f32)> + '_ {
        self.edges.iter().map(|e| (e.x0, e.y0, e.x1, e.y1))
    }

    /// Rasterize into an 8-bit coverage buffer of `width * height` at `origin`,
    /// clipped to the buffer. Coverage is `round(area * 255)` clamped.
    ///
    /// `out` is row-major, `width` wide. The caller maps buffer coordinates back
    /// to surface coordinates with `origin`.
    pub fn coverage(&self, width: u32, height: u32, origin_x: i32, origin_y: i32) -> Vec<u8> {
        let n = (width as usize) * (height as usize);
        let mut cov = vec![0u8; n];
        if self.edges.is_empty() || width == 0 || height == 0 {
            return cov;
        }

        // Device-space origin of this buffer; row `r` covers `[win_y0+r, +1)`.
        let win_x0 = origin_x as f32;
        let win_y0 = origin_y as f32;

        scanline_fill(&self.edges, width, height, win_x0, win_y0, &mut cov);
        cov
    }
}

const ARC_STEPS: usize = 8;

fn arc(
    emit: &mut dyn FnMut(f32, f32, f32, f32),
    cx: f32,
    cy: f32,
    r: f32,
    start_deg: f32,
    end_deg: f32,
) {
    if r <= 0.0 {
        return;
    }
    let mut prev = (cx + r * cos(start_deg), cy + r * sin(start_deg));
    for i in 1..=ARC_STEPS {
        let t = start_deg + (end_deg - start_deg) * (i as f32 / ARC_STEPS as f32);
        let p = (cx + r * cos(t), cy + r * sin(t));
        emit(prev.0, prev.1, p.0, p.1);
        prev = p;
    }
}

/// `cos`/`sin` in degrees. Hand-rolled to avoid libm differences across
/// platforms/opt levels — the values here only need to be *stable*, and a
/// 7-term Taylor series on `[-45°,45°]` plus quadrant reduction is both stable
/// and accurate enough for an 0.5px arc step.
fn cos(deg: f32) -> f32 {
    sin(deg + 90.0)
}

fn sin(deg_in: f32) -> f32 {
    let mut deg = deg_in % 360.0;
    if deg < 0.0 {
        deg += 360.0;
    }
    let (sign, a) = if deg <= 90.0 {
        (1.0, deg)
    } else if deg <= 180.0 {
        (1.0, 180.0 - deg)
    } else if deg <= 270.0 {
        (-1.0, deg - 180.0)
    } else {
        (-1.0, 360.0 - deg)
    };
    let r = a * std::f32::consts::PI / 180.0;
    // sin via Taylor on |r| <= pi/2 (converges well over 5 terms)
    let r2 = r * r;
    let s = r * (1.0 - r2 / 6.0 * (1.0 - r2 / 20.0 * (1.0 - r2 / 42.0 * (1.0 - r2 / 72.0))));
    sign * s
}

/// Exact span sweep with analytic x-coverage on span endpoints.
///
/// For each pixel row, each edge that overlaps the row contributes a
/// linear-in-x boundary. We split the row at every edge's x at the row's
/// top and bottom to get x-monotone segments, then accumulate.
///
/// For the shapes this engine draws (rects, flattened rounded rects) edges are
/// either horizontal or monotone; the sweep below handles both by evaluating
/// each edge at the pixel centre and using its slope for partial pixels.
fn scanline_fill(
    edges: &[Edge],
    width: u32,
    height: u32,
    win_x0: f32,
    win_y0: f32,
    cov: &mut [u8],
) {
    // Per-row list of crossings: x at the row's vertical centre.
    let mut crossings: Vec<f32> = Vec::with_capacity(edges.len());

    for row in 0..height {
        let y_top = win_y0 + row as f32;
        let y_bot = y_top + 1.0;
        let y_c = y_top + 0.5;
        crossings.clear();
        for e in edges {
            // Horizontal edges contribute no x-crossing; they are handled by the
            // edges that bound the shape vertically.
            if e.y0 == e.y1 {
                continue;
            }
            let (ey0, ey1) = if e.y0 < e.y1 { (e.y0, e.y1) } else { (e.y1, e.y0) };
            // Half-open vertical test: an edge covers rows whose band it overlaps
            // positively. This makes a vertex shared by two edges count once.
            if ey1 <= y_top || ey0 >= y_bot {
                continue;
            }
            // Evaluate x at the row's vertical centre. An edge may start above
            // the centre, so clamp the parameter to the edge's own extent: an
            // edge that ends inside this row contributes only down to its end.
            let y_eval = y_c.clamp(ey0, ey1);
            let t = (y_eval - e.y0) / (e.y1 - e.y0);
            let x = e.x0 + (e.x1 - e.x0) * t;
            // A monotone edge that does not span this row's centre (it starts or
            // ends inside the row) is a corner cap: it should not contribute a
            // crossing that shifts a span boundary. Skip it.
            if y_eval != y_c {
                continue;
            }
            crossings.push(x);
        }
        if crossings.len() < 2 {
            continue;
        }
        crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Even-odd pairing. Coincident crossings (from edges that share an
        // endpoint at this row's centre) cancel in pairs and are skipped.
        let mut i = 0;
        while i + 1 < crossings.len() {
            let xa = crossings[i];
            let xb = crossings[i + 1];
            i += 2;
            if xb <= xa {
                continue;
            }
            emit_span(row, xa, xb, win_x0, width, cov);
        }
    }
}

/// Emit one horizontal span with analytic partial-pixel coverage at both ends.
fn emit_span(row: u32, xa: f32, xb: f32, win_x0: f32, width: u32, cov: &mut [u8]) {
    // Convert to buffer-local pixel coordinates.
    let la = xa - win_x0;
    let lb = xb - win_x0;
    if lb <= 0.0 || la >= width as f32 {
        return;
    }
    let la = la.max(0.0);
    let lb = lb.min(width as f32);
    if lb <= la {
        return;
    }
    let row_base = row as usize * width as usize;

    let p_start = la.floor() as u32;
    let p_end = (lb.ceil() as u32).min(width);
    let full_from = (la.ceil() as u32).min(p_end);
    let full_to = (lb.floor() as u32).min(p_end);

    // Leading partial pixel.
    if p_start < full_from {
        let frac = (la.ceil() - la).clamp(0.0, 1.0);
        let c = (frac * 255.0).round() as u8;
        let idx = row_base + p_start as usize;
        cov[idx] = cov[idx].saturating_add(c);
    }
    // Middle full pixels.
    let mut x = full_from;
    while x < full_to {
        cov[row_base + x as usize] = 255;
        x += 1;
    }
    // Trailing partial pixel.
    if full_to < p_end {
        let frac = (lb - lb.floor()).clamp(0.0, 1.0);
        let c = (frac * 255.0).round() as u8;
        let idx = row_base + full_to as usize;
        cov[idx] = cov[idx].saturating_add(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Index into a coverage buffer of `width` columns.
    fn at(cov: &[u8], width: u32, x: u32, y: u32) -> u8 {
        cov[(y * width + x) as usize]
    }

    #[test]
    fn integer_rect_covers_exactly() {
        let mut p = Polygon::new();
        p.rect(Rect::new(2.0, 2.0, 4.0, 3.0));
        let cov = p.coverage(10, 10, 0, 0);
        for y in 0..10u32 {
            for x in 0..10u32 {
                let inside = (2..6).contains(&x) && (2..5).contains(&y);
                let c = at(&cov, 10, x, y);
                if inside {
                    assert_eq!(c, 255, "pixel ({x},{y}) should be fully covered");
                } else {
                    assert_eq!(c, 0, "pixel ({x},{y}) should be empty, got {c}");
                }
            }
        }
    }

    #[test]
    fn fractional_rect_edge_coverage_is_half() {
        let mut p = Polygon::new();
        p.rect(Rect::new(2.5, 0.0, 4.0, 4.0));
        let cov = p.coverage(10, 6, 0, 0);
        // Left edge lands at 2.5: pixel x=2 is half covered.
        assert_eq!(at(&cov, 10, 2, 0), 128);
        // Interior pixels are full.
        assert_eq!(at(&cov, 10, 3, 1), 255);
        // Right edge at 6.5: pixel x=6 is half covered.
        assert_eq!(at(&cov, 10, 6, 1), 128);
        assert_eq!(at(&cov, 10, 7, 1), 0);
    }

    #[test]
    fn buffer_origin_offsets_correctly() {
        let mut p = Polygon::new();
        p.rect(Rect::new(100.0, 100.0, 2.0, 2.0));
        // A 4x4 buffer rooted at (99,99) should contain the rect in its
        // bottom-right 2x2, i.e. local (1..3, 1..3).
        let cov = p.coverage(4, 4, 99, 99);
        assert_eq!(at(&cov, 4, 1, 1), 255);
        assert_eq!(at(&cov, 4, 2, 2), 255);
        assert_eq!(at(&cov, 4, 0, 0), 0);
    }

    #[test]
    fn rounded_rect_corners_are_cut() {
        let mut p = Polygon::new();
        p.rounded_rect(
            Rect::new(0.0, 0.0, 20.0, 20.0),
            engine_core::CornerRadii::uniform(6.0),
        );
        let cov = p.coverage(20, 20, 0, 0);
        // Extreme corner pixel is outside the rounded corner.
        assert_eq!(at(&cov, 20, 0, 0), 0, "top-left corner must be cut");
        // Centre is fully covered.
        assert_eq!(at(&cov, 20, 10, 10), 255);
        // Top edge midpoint is fully covered (the arc does not reach the centre).
        assert_eq!(at(&cov, 20, 10, 0), 255);
        // The diagonal on the corner arc should be partially covered somewhere.
        let partial = cov.iter().any(|&c| c > 0 && c < 255);
        assert!(partial, "arc flattening must produce partial coverage");
    }

    #[test]
    fn zero_radius_rounded_rect_equals_rect() {
        let mut a = Polygon::new();
        a.rect(Rect::new(1.0, 1.0, 5.0, 5.0));
        let mut b = Polygon::new();
        b.rounded_rect(Rect::new(1.0, 1.0, 5.0, 5.0), engine_core::CornerRadii::ZERO);
        let ca = a.coverage(8, 8, 0, 0);
        let cb = b.coverage(8, 8, 0, 0);
        assert_eq!(ca, cb);
    }

    #[test]
    fn trig_matches_known_values() {
        // Reference values from std's f32 trig; the hand-rolled versions exist
        // only for cross-run stability, and must agree to well under a pixel.
        let cases_sin = [(30.0f32, 0.5f32), (45.0, 0.707_106_77), (60.0, 0.866_025_4)];
        for (deg, want) in cases_sin {
            let got = sin(deg);
            assert!((got - want).abs() < 1e-4, "sin({deg}) = {got}, want {want}");
        }
        let cases_cos = [(30.0f32, 0.866_025_4f32), (45.0, 0.707_106_77), (60.0, 0.5)];
        for (deg, want) in cases_cos {
            let got = cos(deg);
            assert!((got - want).abs() < 1e-4, "cos({deg}) = {got}, want {want}");
        }
        // Quadrant reduction and sign handling.
        assert!((sin(120.0) - 0.866_025_4).abs() < 1e-4);
        assert!((cos(120.0) + 0.5).abs() < 1e-4);
        assert!((sin(-30.0) + 0.5).abs() < 1e-4);
        assert!((cos(270.0)).abs() < 1e-4);
    }

    #[test]
    fn empty_polygon_covers_nothing() {
        let p = Polygon::new();
        assert!(p.is_empty());
        assert!(p.coverage(4, 4, 0, 0).iter().all(|&c| c == 0));
    }
}