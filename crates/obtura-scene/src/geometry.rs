use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance(self, o: Point) -> f64 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }

    pub fn offset(self, dx: f64, dy: f64) -> Point {
        Point::new(self.x + dx, self.y + dy)
    }
}

/// A rectangle with a non-negative width and height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectF {
    /// The rectangle between two opposite corners, in any order.
    pub fn from_points(a: Point, b: Point) -> Self {
        Self { x: a.x.min(b.x), y: a.y.min(b.y), w: (a.x - b.x).abs(), h: (a.y - b.y).abs() }
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }

    pub fn inflate(&self, d: f64) -> RectF {
        RectF { x: self.x - d, y: self.y - d, w: self.w + 2.0 * d, h: self.h + 2.0 * d }
    }

    /// Whether the two rectangles share any point (touching edges count).
    pub fn intersects(&self, o: &RectF) -> bool {
        self.x <= o.x + o.w && o.x <= self.x + self.w && self.y <= o.y + o.h && o.y <= self.y + self.h
    }

    pub fn union(&self, o: &RectF) -> RectF {
        let (x0, y0) = (self.x.min(o.x), self.y.min(o.y));
        let (x1, y1) = ((self.x + self.w).max(o.x + o.w), (self.y + self.h).max(o.y + o.h));
        RectF { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
}

/// The control point of the quadratic curve from `a` to `b` that passes through `via` in
/// the middle (at t = 0.5). Dragging `via` is how the user bends a curve.
pub fn quad_control(a: Point, via: Point, b: Point) -> Point {
    Point::new(2.0 * via.x - (a.x + b.x) / 2.0, 2.0 * via.y - (a.y + b.y) / 2.0)
}

/// The point at `t` (0 to 1) on the quadratic curve `a`, control `c`, `b`.
pub fn quad_at(a: Point, c: Point, b: Point, t: f64) -> Point {
    let u = 1.0 - t;
    Point::new(u * u * a.x + 2.0 * u * t * c.x + t * t * b.x, u * u * a.y + 2.0 * u * t * c.y + t * t * b.y)
}

/// The curve as a short list of points, good enough for hit testing and bounds.
pub fn flatten_quad(a: Point, c: Point, b: Point) -> Vec<Point> {
    const STEPS: usize = 24;
    (0..=STEPS).map(|i| quad_at(a, c, b, i as f64 / STEPS as f64)).collect()
}

/// A cubic Bézier piece of a line: from `a` to `b`, pulled by `c1` and `c2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cubic {
    pub a: Point,
    pub c1: Point,
    pub c2: Point,
    pub b: Point,
}

impl Cubic {
    /// A straight piece.
    pub fn straight(a: Point, b: Point) -> Cubic {
        Cubic { a, c1: lerp(a, b, 1.0 / 3.0), c2: lerp(a, b, 2.0 / 3.0), b }
    }

    /// The quadratic curve `a`, control `c`, `b`, exactly.
    pub fn from_quad(a: Point, c: Point, b: Point) -> Cubic {
        Cubic { a, c1: lerp(a, c, 2.0 / 3.0), c2: lerp(b, c, 2.0 / 3.0), b }
    }

    pub fn at(&self, t: f64) -> Point {
        let u = 1.0 - t;
        let (k0, k1, k2, k3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        Point::new(k0 * self.a.x + k1 * self.c1.x + k2 * self.c2.x + k3 * self.b.x, k0 * self.a.y + k1 * self.c1.y + k2 * self.c2.y + k3 * self.b.y)
    }

    /// The same piece run backwards.
    pub fn reversed(&self) -> Cubic {
        Cubic { a: self.b, c1: self.c2, c2: self.c1, b: self.a }
    }

    /// The part from `t = 0` to `t` (de Casteljau).
    pub fn head(&self, t: f64) -> Cubic {
        let (p01, p12, p23) = (lerp(self.a, self.c1, t), lerp(self.c1, self.c2, t), lerp(self.c2, self.b, t));
        let (p012, p123) = (lerp(p01, p12, t), lerp(p12, p23, t));
        Cubic { a: self.a, c1: p01, c2: p012, b: lerp(p012, p123, t) }
    }

    /// Length of the control polygon (never less than the curve's own length).
    pub fn hull_length(&self) -> f64 {
        self.a.distance(self.c1) + self.c1.distance(self.c2) + self.c2.distance(self.b)
    }

    /// The exact box of the piece: its ends and the turning points between.
    pub fn bounds(&self) -> RectF {
        let mut pts = vec![self.a, self.b];
        let roots = |p0: f64, p1: f64, p2: f64, p3: f64| -> Vec<f64> {
            // Derivative / 3: a t^2 + b t + c.
            let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
            let b = 2.0 * (p0 - 2.0 * p1 + p2);
            let c = p1 - p0;
            let mut out = Vec::new();
            if a.abs() < 1e-12 {
                if b.abs() > 1e-12 {
                    out.push(-c / b);
                }
            } else {
                let disc = b * b - 4.0 * a * c;
                if disc >= 0.0 {
                    let r = disc.sqrt();
                    out.push((-b + r) / (2.0 * a));
                    out.push((-b - r) / (2.0 * a));
                }
            }
            out.into_iter().filter(|t| *t > 0.0 && *t < 1.0).collect()
        };
        for t in roots(self.a.x, self.c1.x, self.c2.x, self.b.x).into_iter().chain(roots(self.a.y, self.c1.y, self.c2.y, self.b.y)) {
            pts.push(self.at(t));
        }
        let first = pts[0];
        pts.iter().fold(RectF { x: first.x, y: first.y, w: 0.0, h: 0.0 }, |r, p| r.union(&RectF { x: p.x, y: p.y, w: 0.0, h: 0.0 }))
    }
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// The pieces of a line from `from` to `to` bending through `via`, in order. No bend: one
/// straight piece. One bend: the quadratic curve through it at its middle (see
/// [`quad_control`]). More: a smooth curve through every point (Catmull-Rom).
pub fn line_pieces(from: Point, via: &[Point], to: Point) -> Vec<Cubic> {
    match via {
        [] => vec![Cubic::straight(from, to)],
        [v] => vec![Cubic::from_quad(from, quad_control(from, *v, to), to)],
        _ => {
            let mut pts = Vec::with_capacity(via.len() + 2);
            pts.push(from);
            pts.extend_from_slice(via);
            pts.push(to);
            let n = pts.len();
            // The direction at each point: along its neighbours (the ends: toward the next one).
            let tangent = |i: usize| -> Point {
                let (p, q) = match i {
                    0 => (pts[0], pts[1]),
                    i if i == n - 1 => (pts[n - 2], pts[n - 1]),
                    i => {
                        let (p, q) = (pts[i - 1], pts[i + 1]);
                        return Point::new((q.x - p.x) / 2.0, (q.y - p.y) / 2.0);
                    }
                };
                Point::new(q.x - p.x, q.y - p.y)
            };
            (0..n - 1)
                .map(|i| {
                    let (m0, m1) = (tangent(i), tangent(i + 1));
                    Cubic { a: pts[i], c1: pts[i].offset(m0.x / 3.0, m0.y / 3.0), c2: pts[i + 1].offset(-m1.x / 3.0, -m1.y / 3.0), b: pts[i + 1] }
                })
                .collect()
        }
    }
}

/// The pieces as a list of points, good enough for hit testing.
pub fn flatten_pieces(pieces: &[Cubic]) -> Vec<Point> {
    const STEPS: usize = 24;
    let mut out = Vec::with_capacity(pieces.len() * STEPS + 1);
    for (k, c) in pieces.iter().enumerate() {
        let start = if k == 0 { 0 } else { 1 };
        out.extend((start..=STEPS).map(|i| c.at(i as f64 / STEPS as f64)));
    }
    out
}

/// `p` turned by `angle` (radians, clockwise on the screen) around `center`.
pub fn rotate_point(p: Point, center: Point, angle: f64) -> Point {
    if angle == 0.0 {
        return p;
    }
    let (s, c) = angle.sin_cos();
    let (dx, dy) = (p.x - center.x, p.y - center.y);
    Point::new(center.x + dx * c - dy * s, center.y + dx * s + dy * c)
}

/// The upright box around `r` turned by `angle` around `center`.
pub fn rotated_bounds(r: RectF, center: Point, angle: f64) -> RectF {
    if angle == 0.0 {
        return r;
    }
    let corners = [Point::new(r.x, r.y), Point::new(r.x + r.w, r.y), Point::new(r.x + r.w, r.y + r.h), Point::new(r.x, r.y + r.h)].map(|p| rotate_point(p, center, angle));
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in corners {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    RectF { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/// The middle of the side of `r` that is nearest to `p`: where a callout's line meets its box.
pub fn nearest_side_midpoint(r: RectF, p: Point) -> Point {
    let sides = [Point::new(r.x + r.w / 2.0, r.y), Point::new(r.x + r.w, r.y + r.h / 2.0), Point::new(r.x + r.w / 2.0, r.y + r.h), Point::new(r.x, r.y + r.h / 2.0)];
    sides.into_iter().min_by(|a, b| a.distance(p).total_cmp(&b.distance(p))).expect("four sides")
}

/// Distance from `p` to the segment `a`-`b`.
pub fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_from_any_corners() {
        let r = RectF::from_points(Point::new(10.0, 2.0), Point::new(4.0, 8.0));
        assert_eq!(r, RectF { x: 4.0, y: 2.0, w: 6.0, h: 6.0 });
    }

    #[test]
    fn a_curve_passes_through_its_via_point() {
        let (a, b, via) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(50.0, 30.0));
        let c = quad_control(a, via, b);
        assert_eq!(quad_at(a, c, b, 0.5), via);
        assert_eq!(quad_at(a, c, b, 0.0), a);
        assert_eq!(quad_at(a, c, b, 1.0), b);
        let pts = flatten_quad(a, c, b);
        assert!(pts.iter().any(|p| p.distance(via) < 1.0));
    }

    #[test]
    fn one_bend_is_the_same_curve_as_before_and_more_bends_pass_through_every_point() {
        let (a, b, v) = (Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(50.0, 30.0));
        let one = line_pieces(a, &[v], b);
        let c = quad_control(a, v, b);
        for t in [0.0, 0.25, 0.5, 0.8, 1.0] {
            assert!(one[0].at(t).distance(quad_at(a, c, b, t)) < 1e-9);
        }
        let bends = [Point::new(30.0, 40.0), Point::new(60.0, -20.0), Point::new(80.0, 10.0)];
        let many = line_pieces(a, &bends, b);
        assert_eq!(many.len(), 4);
        assert_eq!(many[0].a, a);
        assert_eq!(many[3].b, b);
        for (i, p) in bends.iter().enumerate() {
            assert_eq!(many[i].b, *p);
            assert_eq!(many[i + 1].a, *p);
        }
        // Smooth: the pieces meet with the same direction.
        let d_in = (many[1].b.x - many[1].c2.x, many[1].b.y - many[1].c2.y);
        let d_out = (many[2].c1.x - many[2].a.x, many[2].c1.y - many[2].a.y);
        assert!((d_in.0 * d_out.1 - d_in.1 * d_out.0).abs() < 1e-9);
    }

    #[test]
    fn a_cubic_box_holds_every_point_of_it() {
        let c = Cubic { a: Point::new(0.0, 0.0), c1: Point::new(10.0, 80.0), c2: Point::new(90.0, -60.0), b: Point::new(100.0, 0.0) };
        let b = c.bounds();
        for i in 0..=1000 {
            let p = c.at(i as f64 / 1000.0);
            assert!(b.inflate(1e-9).contains(p), "{p:?} outside {b:?}");
        }
        // Tight: a point of the curve touches the top and the bottom.
        let ys: Vec<f64> = (0..=1000).map(|i| c.at(i as f64 / 1000.0).y).collect();
        assert!((ys.iter().cloned().fold(f64::MIN, f64::max) - (b.y + b.h)).abs() < 0.01);
        // Cutting the head keeps its start and lies on the curve.
        let h = c.head(0.4);
        assert_eq!(h.a, c.a);
        assert!(h.b.distance(c.at(0.4)) < 1e-9);
        assert!(h.at(0.5).distance(c.at(0.2)) < 1e-9);
    }

    #[test]
    fn rotating_a_point_and_a_box() {
        let c = Point::new(10.0, 10.0);
        let p = rotate_point(Point::new(20.0, 10.0), c, std::f64::consts::FRAC_PI_2);
        assert!(p.distance(Point::new(10.0, 20.0)) < 1e-9, "clockwise on the screen: {p:?}");
        let r = rotated_bounds(RectF { x: 0.0, y: 5.0, w: 20.0, h: 10.0 }, c, std::f64::consts::FRAC_PI_2);
        assert!((r.w - 10.0).abs() < 1e-9 && (r.h - 20.0).abs() < 1e-9 && (r.x - 5.0).abs() < 1e-9);
    }

    #[test]
    fn the_callout_line_meets_the_nearest_side() {
        let r = RectF { x: 100.0, y: 100.0, w: 60.0, h: 20.0 };
        assert_eq!(nearest_side_midpoint(r, Point::new(0.0, 110.0)), Point::new(100.0, 110.0));
        assert_eq!(nearest_side_midpoint(r, Point::new(130.0, 0.0)), Point::new(130.0, 100.0));
    }

    #[test]
    fn segment_distance() {
        let (a, b) = (Point::new(0.0, 0.0), Point::new(10.0, 0.0));
        assert_eq!(dist_to_segment(Point::new(5.0, 3.0), a, b), 3.0);
        assert_eq!(dist_to_segment(Point::new(-4.0, 3.0), a, b), 5.0); // beyond the end
        assert_eq!(dist_to_segment(Point::new(1.0, 1.0), a, a), 2f64.sqrt());
    }
}
