//! Geometry: points, sizes, edges, rects, corner radii.
//!
//! All coordinates are `f32`. The architecture (§3.2) fixes the unit: after the
//! cascade and DPR scaling, every layout coordinate is an absolute device pixel.
//! Nothing here knows about CSS.

/// A 2-D point / offset. Used for origins and deltas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }
}

/// A width/height pair.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f32,
    pub h: f32,
}

impl Size {
    pub const ZERO: Size = Size { w: 0.0, h: 0.0 };
    pub const fn new(w: f32, h: f32) -> Self {
        Size { w, h }
    }
}

/// Per-edge lengths in paint order-independent CSS order: top/right/bottom/left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeSizes {
    pub const ZERO: EdgeSizes = EdgeSizes { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 };

    pub const fn uniform(v: f32) -> Self {
        EdgeSizes { top: v, right: v, bottom: v, left: v }
    }

    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        EdgeSizes { top, right, bottom, left }
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }

    /// The 4-value CSS shorthand expansion: t (r=l=t) (b=t).
    pub fn from_shorthand(t: f32, r: f32, b: f32, l: f32) -> Self {
        EdgeSizes { top: t, right: r, bottom: b, left: l }
    }
}

/// An axis-aligned rectangle. `x`/`y` is the top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn from_origin_size(p: Point, s: Size) -> Self {
        Rect::new(p.x, p.y, s.w, s.h)
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    pub fn is_empty(&self) -> bool {
        !(self.w > 0.0 && self.h > 0.0)
    }

    /// The rect inset by `e` on each side. May produce negative extents; callers
    /// that need a non-empty rect must clamp with `max(0.0)` themselves.
    pub fn deflate(&self, e: EdgeSizes) -> Rect {
        Rect::new(self.x + e.left, self.y + e.top, self.w - e.horizontal(), self.h - e.vertical())
    }

    pub fn inflate(&self, e: EdgeSizes) -> Rect {
        self.deflate(EdgeSizes::new(-e.top, -e.right, -e.bottom, -e.left))
    }

    /// Intersection; empty (`w == h == 0`) if the rects do not overlap.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= x0 || y1 <= y0 {
            Rect::new(x0, y0, 0.0, 0.0)
        } else {
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    /// True when the two rects share any positive-area region.
    pub fn intersects(&self, other: &Rect) -> bool {
        !self.intersect(other).is_empty()
    }

    /// Smallest rect containing both. An empty input does not extend the result
    /// (used so a union with `Rect::ZERO` does not drag the origin to 0,0).
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self.right().max(other.right());
        let y1 = self.bottom().max(other.bottom());
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn contains_point(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Snap to whole device pixels, covering the original rect fully
    /// (floor the origin, ceil the far edge). Used at blit time only.
    pub fn to_pixel_bounds(&self) -> Rect {
        let x0 = self.x.floor();
        let y0 = self.y.floor();
        let x1 = self.right().ceil();
        let y1 = self.bottom().ceil();
        Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }

    /// True if every component is finite.
    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.w.is_finite() && self.h.is_finite()
    }
}

/// Corner radii, clockwise from top-left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerRadii {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl CornerRadii {
    pub const ZERO: CornerRadii =
        CornerRadii { top_left: 0.0, top_right: 0.0, bottom_right: 0.0, bottom_left: 0.0 };

    pub const fn uniform(r: f32) -> Self {
        CornerRadii { top_left: r, top_right: r, bottom_right: r, bottom_left: r }
    }

    pub fn is_zero(&self) -> bool {
        self.top_left == 0.0
            && self.top_right == 0.0
            && self.bottom_right == 0.0
            && self.bottom_left == 0.0
    }
}

/// A minimal non-empty region check helper used by damage tracking.
pub fn union_all(rects: impl IntoIterator<Item = Rect>) -> Rect {
    rects.into_iter().fold(Rect::ZERO, |acc, r| acc.union(&r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_arithmetic() {
        let e = EdgeSizes::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(e.horizontal(), 6.0);
        assert_eq!(e.vertical(), 4.0);
        assert_eq!(EdgeSizes::uniform(2.5).horizontal(), 5.0);
    }

    #[test]
    fn rect_intersection() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        let i = a.intersect(&b);
        assert_eq!(i, Rect::new(5.0, 5.0, 5.0, 5.0));
        assert!(a.intersects(&b));
    }

    #[test]
    fn rect_disjoint_intersection_is_empty() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(20.0, 20.0, 5.0, 5.0);
        let i = a.intersect(&b);
        assert!(i.is_empty());
        assert_eq!(i.w, 0.0);
        assert!(!a.intersects(&b));
    }

    #[test]
    fn rect_touching_edges_do_not_intersect() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(10.0, 0.0, 10.0, 10.0);
        assert!(!a.intersects(&b), "shared edge is zero area");
    }

    #[test]
    fn rect_union() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
    }

    #[test]
    fn rect_union_ignores_empty() {
        let a = Rect::new(3.0, 4.0, 10.0, 10.0);
        assert_eq!(a.union(&Rect::ZERO), a);
        assert_eq!(Rect::ZERO.union(&a), a);
    }

    #[test]
    fn rect_deflate_inflate_roundtrip() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        let e = EdgeSizes::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(r.deflate(e).inflate(e), r);
    }

    #[test]
    fn rect_deflate_to_content_box() {
        let border_box = Rect::new(0.0, 0.0, 100.0, 50.0);
        let padding = EdgeSizes::uniform(10.0);
        let border = EdgeSizes::uniform(2.0);
        let content = border_box.deflate(border).deflate(padding);
        assert_eq!(content, Rect::new(12.0, 12.0, 76.0, 26.0));
    }

    #[test]
    fn rect_contains_point_half_open() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains_point(Point::new(0.0, 0.0)));
        assert!(r.contains_point(Point::new(9.99, 9.99)));
        assert!(!r.contains_point(Point::new(10.0, 5.0)));
    }

    #[test]
    fn pixel_bounds_cover_fractional_rect() {
        let r = Rect::new(1.4, 2.6, 3.3, 4.1);
        let p = r.to_pixel_bounds();
        assert_eq!(p, Rect::new(1.0, 2.0, 4.0, 5.0));
        assert!(p.x <= r.x && p.y <= r.y);
        assert!(p.right() >= r.right() && p.bottom() >= r.bottom());
    }

    #[test]
    fn union_all_folds() {
        let rs = [Rect::new(0.0, 0.0, 1.0, 1.0), Rect::new(10.0, 10.0, 1.0, 1.0)];
        assert_eq!(union_all(rs), Rect::new(0.0, 0.0, 11.0, 11.0));
    }
}