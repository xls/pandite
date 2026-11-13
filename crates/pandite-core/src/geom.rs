//! Points, sizes, rectangles and the fit math shared by the window, the
//! viewer and the resize tools. All values are `f32` logical pixels unless a
//! function says otherwise.

/// A position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal coordinate, growing right.
    pub x: f32,
    /// Vertical coordinate, growing down.
    pub y: f32,
}

impl Point {
    /// Build a point.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A width and height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Size {
    /// Build a size.
    pub const fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }

    /// True when either side is zero, negative or not finite.
    pub fn is_empty(self) -> bool {
        !(self.w > 0.0 && self.h > 0.0 && self.w.is_finite() && self.h.is_finite())
    }

    /// The scale that makes `self` fit inside `bounds` and touch it on at
    /// least one axis ("best fit"). Returns 1.0 when either size is empty.
    pub fn fit_scale(self, bounds: Size) -> f32 {
        if self.is_empty() || bounds.is_empty() {
            return 1.0;
        }
        (bounds.w / self.w).min(bounds.h / self.h)
    }

    /// `self` scaled to best fit inside `bounds`, keeping the aspect ratio.
    pub fn fit_within(self, bounds: Size) -> Size {
        let s = self.fit_scale(bounds);
        Size::new(self.w * s, self.h * s)
    }

    /// Multiply both sides by `s`.
    pub fn scaled(self, s: f32) -> Size {
        Size::new(self.w * s, self.h * s)
    }
}

/// An axis-aligned rectangle given by its top-left corner and size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// Build a rectangle.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Build a rectangle from two opposite corners in any order.
    pub fn from_corners(a: Point, b: Point) -> Self {
        let (x0, x1) = if a.x <= b.x { (a.x, b.x) } else { (b.x, a.x) };
        let (y0, y1) = if a.y <= b.y { (a.y, b.y) } else { (b.y, a.y) };
        Self::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Right edge.
    pub fn right(self) -> f32 {
        self.x + self.w
    }

    /// Bottom edge.
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    /// Size of the rectangle.
    pub fn size(self) -> Size {
        Size::new(self.w, self.h)
    }

    /// Center point.
    pub fn center(self) -> Point {
        Point::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    /// True when the rectangle has no area.
    pub fn is_empty(self) -> bool {
        self.size().is_empty()
    }

    /// True when `p` lies inside (left and top inclusive, right and bottom exclusive).
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }

    /// The overlap of two rectangles, or `None` when they do not overlap.
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
    }

    /// Shrink by `d` on every side. Never returns a negative size.
    pub fn inset(self, d: f32) -> Rect {
        let w = (self.w - 2.0 * d).max(0.0);
        let h = (self.h - 2.0 * d).max(0.0);
        Rect::new(self.x + d, self.y + d, w, h)
    }

    /// A rectangle of `size` centered inside `self`.
    pub fn centered(self, size: Size) -> Rect {
        Rect::new(
            self.x + (self.w - size.w) * 0.5,
            self.y + (self.h - size.h) * 0.5,
            size.w,
            size.h,
        )
    }
}

/// A two-dimensional affine transform: a point `(x, y)` maps to
/// `(a * x + c * y + e, b * x + d * y + f)`. It describes how a view edit
/// (a quarter turn, a mirror, a crop, a resize) moves image pixels, and how
/// image pixels land on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    /// Weight of `x` in the new `x`.
    pub a: f32,
    /// Weight of `x` in the new `y`.
    pub b: f32,
    /// Weight of `y` in the new `x`.
    pub c: f32,
    /// Weight of `y` in the new `y`.
    pub d: f32,
    /// Added to the new `x`.
    pub e: f32,
    /// Added to the new `y`.
    pub f: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    /// The transform that changes nothing.
    pub const IDENTITY: Affine = Affine::new(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);

    /// Build a transform from its six coefficients.
    pub const fn new(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self { a, b, c, d, e, f }
    }

    /// Move by `dx`, `dy`.
    pub const fn translate(dx: f32, dy: f32) -> Self {
        Self::new(1.0, 0.0, 0.0, 1.0, dx, dy)
    }

    /// Scale about the origin.
    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self::new(sx, 0.0, 0.0, sy, 0.0, 0.0)
    }

    /// The transform that maps rectangle `from` onto rectangle `to`
    /// (scale and move, no rotation). An empty `from` gives the identity.
    pub fn rect_to_rect(from: Rect, to: Rect) -> Self {
        if from.is_empty() {
            return Self::IDENTITY;
        }
        let sx = to.w / from.w;
        let sy = to.h / from.h;
        Self::new(sx, 0.0, 0.0, sy, to.x - from.x * sx, to.y - from.y * sy)
    }

    /// Where `p` lands.
    pub fn apply(&self, p: Point) -> Point {
        Point::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }

    /// First `self`, then `next`.
    pub fn then(&self, next: &Affine) -> Affine {
        Affine::new(
            next.a * self.a + next.c * self.b,
            next.b * self.a + next.d * self.b,
            next.a * self.c + next.c * self.d,
            next.b * self.c + next.d * self.d,
            next.a * self.e + next.c * self.f + next.e,
            next.b * self.e + next.d * self.f + next.f,
        )
    }

    /// The transform that undoes `self`, or `None` when it collapses the
    /// plane (a zero scale) or holds values that are not finite.
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let inv = Affine::new(
            self.d / det,
            -self.b / det,
            -self.c / det,
            self.a / det,
            (self.c * self.f - self.d * self.e) / det,
            (self.b * self.e - self.a * self.f) / det,
        );
        let finite = [inv.a, inv.b, inv.c, inv.d, inv.e, inv.f]
            .iter()
            .all(|v| v.is_finite());
        finite.then_some(inv)
    }
}

/// The window rectangle for a first-open image.
///
/// `work_area` is the usable monitor area. `fraction` (0.1 to 1.0) is the
/// share of that area the window may take on each axis. The image is best
/// fitted by width or height inside that sub-region, never enlarged above
/// 1:1, never smaller than `min_size`, and the result is centered.
pub fn initial_window_rect(work_area: Rect, image: Size, fraction: f32, min_size: Size) -> Rect {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.1, 1.0)
    } else {
        0.8
    };
    let region = work_area.size().scaled(fraction);
    let scale = image.fit_scale(region).min(1.0);
    let fitted = if image.is_empty() {
        region
    } else {
        image.scaled(scale)
    };
    let size = Size::new(
        fitted.w.max(min_size.w).min(work_area.w),
        fitted.h.max(min_size.h).min(work_area.h),
    );
    work_area.centered(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_touches_one_axis() {
        let s = Size::new(4000.0, 1000.0).fit_within(Size::new(800.0, 600.0));
        assert_eq!(s, Size::new(800.0, 200.0));
        let s = Size::new(1000.0, 4000.0).fit_within(Size::new(800.0, 600.0));
        assert_eq!(s, Size::new(150.0, 600.0));
        assert_eq!(Size::new(0.0, 5.0).fit_scale(Size::new(1.0, 1.0)), 1.0);
    }

    #[test]
    fn rect_intersect_and_contains() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(b), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
        assert_eq!(a.intersect(Rect::new(10.0, 0.0, 1.0, 1.0)), None);
        assert!(a.contains(Point::new(0.0, 0.0)));
        assert!(!a.contains(Point::new(10.0, 5.0)));
        let c = Rect::from_corners(Point::new(8.0, 9.0), Point::new(2.0, 1.0));
        assert_eq!(c, Rect::new(2.0, 1.0, 6.0, 8.0));
    }

    #[test]
    fn affine_composes_and_inverts() {
        // A quarter turn clockwise of a 30 x 20 image: (x, y) -> (20 - y, x).
        let turn = Affine::new(0.0, 1.0, -1.0, 0.0, 20.0, 0.0);
        assert_eq!(turn.apply(Point::new(0.0, 0.0)), Point::new(20.0, 0.0));
        assert_eq!(turn.apply(Point::new(30.0, 20.0)), Point::new(0.0, 30.0));
        let then = turn.then(&Affine::translate(5.0, -1.0));
        assert_eq!(then.apply(Point::new(10.0, 4.0)), Point::new(21.0, 9.0));
        let inv = then.inverse().unwrap();
        let p = inv.apply(then.apply(Point::new(7.5, 3.25)));
        assert!(
            (p.x - 7.5).abs() < 1e-4 && (p.y - 3.25).abs() < 1e-4,
            "{p:?}"
        );
        assert!(Affine::scale(0.0, 1.0).inverse().is_none());
        let fit = Affine::rect_to_rect(
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Rect::new(10.0, 20.0, 50.0, 25.0),
        );
        assert_eq!(fit.apply(Point::new(100.0, 50.0)), Point::new(60.0, 45.0));
        assert_eq!(Affine::default(), Affine::IDENTITY);
    }

    #[test]
    fn initial_window_fits_fraction_of_work_area() {
        let work = Rect::new(0.0, 0.0, 1920.0, 1040.0);
        let min = Size::new(320.0, 240.0);
        // Wide image: limited by width of the 80% region.
        let r = initial_window_rect(work, Size::new(8000.0, 2000.0), 0.8, min);
        assert_eq!(r.w, 1536.0);
        assert_eq!(r.h, 384.0);
        assert_eq!(r.center(), work.center());
        // Tall image: limited by height.
        let r = initial_window_rect(work, Size::new(1000.0, 4000.0), 0.5, min);
        assert_eq!(r.h, 520.0);
        // Small image: never enlarged, but not below the minimum.
        let r = initial_window_rect(work, Size::new(100.0, 50.0), 0.8, min);
        assert_eq!(r.size(), min);
        // Bad fraction falls back to a sane default.
        let r = initial_window_rect(work, Size::new(8000.0, 8000.0), f32::NAN, min);
        assert_eq!(r.h, 832.0);
    }
}
