//! Radial wheel geometry (SPEC §3): petal angles, hit testing, petal outlines and label positions.
//!
//! Coordinates are relative to the wheel centre with **y pointing up** (AppKit's default).

use std::f64::consts::{FRAC_PI_2, PI, TAU};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    fn polar(r: f64, angle: f64) -> Self {
        Point::new(r * angle.cos(), r * angle.sin())
    }
}

/// Path segments; arcs are emitted as cubic Béziers so any 2D API can draw them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCmd {
    MoveTo(Point),
    LineTo(Point),
    CubicTo { c1: Point, c2: Point, to: Point },
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelGeometry {
    /// Radius of the empty centre.
    pub inner: f64,
    /// Outer radius of the petals.
    pub outer: f64,
    /// Constant width of the gap between neighbouring petals.
    pub gap: f64,
    /// Corner rounding on the outer and inner edges.
    pub outer_corner: f64,
    pub inner_corner: f64,
    /// Radius of the glass backdrop disc.
    pub backdrop: f64,
}

impl Default for WheelGeometry {
    fn default() -> Self {
        WheelGeometry {
            inner: 46.0,
            outer: 120.0,
            gap: 4.5,
            outer_corner: 16.0,
            inner_corner: 6.0,
            backdrop: 127.0,
        }
    }
}

/// Smallest rectangle (in points) holding the whole wheel.
pub const WHEEL_VIEW_SIZE: f64 = 272.0;

impl WheelGeometry {
    /// Centre angle of petal `i` of `n`: the first petal is at the top (on the left when n = 3),
    /// and the rest follow clockwise.
    pub fn petal_angle(i: usize, n: usize) -> f64 {
        let start = if n == 3 { PI } else { FRAC_PI_2 };
        start - TAU * i as f64 / n as f64
    }

    fn half_gap_angle(&self, r: f64) -> f64 {
        ((self.gap / 2.0) / r.max(self.gap / 2.0 + 0.01)).min(0.95).asin()
    }

    /// Which petal (if any) lies under the offset `(dx, dy)` from the centre.
    pub fn hit_test(&self, dx: f64, dy: f64, n: usize) -> Option<usize> {
        if n == 0 {
            return None;
        }
        let d = dx.hypot(dy);
        if d < self.inner || d > self.outer {
            return None;
        }
        if n == 1 {
            return Some(0);
        }
        let angle = dy.atan2(dx);
        let step = TAU / n as f64;
        let from_first = (Self::petal_angle(0, n) - angle).rem_euclid(TAU);
        let i = ((from_first / step).round() as usize) % n;
        let delta = angle_diff(angle, Self::petal_angle(i, n)).abs();
        if delta > step / 2.0 - self.half_gap_angle(d) {
            None
        } else {
            Some(i)
        }
    }

    /// Where petal `i`'s label is centred.
    pub fn label_position(&self, i: usize, n: usize) -> Point {
        let r = if n == 1 {
            (self.inner + self.outer) / 2.0
        } else {
            self.inner + (self.outer - self.inner) * 0.5
        };
        Point::polar(r, Self::petal_angle(i, n))
    }

    /// Outline of petal `i` of `n` as a closed path.
    pub fn petal_path(&self, i: usize, n: usize) -> Vec<PathCmd> {
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            // A full ring: outer circle counter-clockwise, inner circle clockwise (non-zero hole).
            let mut cmds = Vec::new();
            cmds.push(PathCmd::MoveTo(Point::polar(self.outer, 0.0)));
            arc(&mut cmds, self.outer, 0.0, TAU);
            cmds.push(PathCmd::Close);
            cmds.push(PathCmd::MoveTo(Point::polar(self.inner, 0.0)));
            arc(&mut cmds, self.inner, 0.0, -TAU);
            cmds.push(PathCmd::Close);
            return cmds;
        }

        let mid = Self::petal_angle(i, n);
        let half = PI / n as f64;
        let lo = mid - half; // clockwise-most edge (smaller angle)
        let hi = mid + half;
        let (ro, ri) = (self.outer, self.inner);
        // Keep corners from swallowing narrow petals.
        let span_out = (hi - lo) - 2.0 * self.half_gap_angle(ro);
        let co = self.outer_corner.min(span_out * ro * 0.3).min((ro - ri) * 0.4);
        let span_in = (hi - lo) - 2.0 * self.half_gap_angle(ri);
        let ci = self.inner_corner.min(span_in * ri * 0.3).min((ro - ri) * 0.2);

        // Edge point at radius r on the `lo` side (offset parallel to the ray by half the gap).
        let lo_at = |r: f64| Point::polar(r, lo + self.half_gap_angle(r));
        let hi_at = |r: f64| Point::polar(r, hi - self.half_gap_angle(r));

        let mut cmds = Vec::new();
        cmds.push(PathCmd::MoveTo(lo_at(ri + ci)));
        cmds.push(PathCmd::LineTo(lo_at(ro - co)));
        // Outer corner on the lo side.
        let a0 = lo + self.half_gap_angle(ro);
        quad(&mut cmds, lo_at(ro - co), lo_at(ro), Point::polar(ro, a0 + co / ro));
        // Outer arc (counter-clockwise).
        let a1 = hi - self.half_gap_angle(ro);
        arc(&mut cmds, ro, a0 + co / ro, a1 - co / ro);
        quad(&mut cmds, Point::polar(ro, a1 - co / ro), hi_at(ro), hi_at(ro - co));
        cmds.push(PathCmd::LineTo(hi_at(ri + ci)));
        // Inner corner on the hi side.
        let b1 = hi - self.half_gap_angle(ri);
        quad(&mut cmds, hi_at(ri + ci), hi_at(ri), Point::polar(ri, b1 - ci / ri));
        // Inner arc (clockwise).
        let b0 = lo + self.half_gap_angle(ri);
        arc(&mut cmds, ri, b1 - ci / ri, b0 + ci / ri);
        quad(&mut cmds, Point::polar(ri, b0 + ci / ri), lo_at(ri), lo_at(ri + ci));
        cmds.push(PathCmd::Close);
        cmds
    }

    /// Moves `anchor` so the whole wheel fits inside `bounds` (x, y, width, height).
    pub fn clamp_anchor(&self, anchor: Point, bounds: (f64, f64, f64, f64)) -> Point {
        let margin = WHEEL_VIEW_SIZE / 2.0;
        let (bx, by, bw, bh) = bounds;
        let clamp = |v: f64, lo: f64, hi: f64| if lo > hi { (lo + hi) / 2.0 } else { v.clamp(lo, hi) };
        Point::new(
            clamp(anchor.x, bx + margin, bx + bw - margin),
            clamp(anchor.y, by + margin, by + bh - margin),
        )
    }
}

/// Signed smallest difference a − b, in (−π, π].
pub fn angle_diff(a: f64, b: f64) -> f64 {
    (a - b).sin().atan2((a - b).cos())
}

fn quad(cmds: &mut Vec<PathCmd>, from: Point, ctrl: Point, to: Point) {
    let c1 = Point::new(
        from.x + 2.0 / 3.0 * (ctrl.x - from.x),
        from.y + 2.0 / 3.0 * (ctrl.y - from.y),
    );
    let c2 = Point::new(
        to.x + 2.0 / 3.0 * (ctrl.x - to.x),
        to.y + 2.0 / 3.0 * (ctrl.y - to.y),
    );
    cmds.push(PathCmd::CubicTo { c1, c2, to });
}

/// Appends a circular arc from angle `a` to `b` (radians, either direction) as cubic segments
/// of at most 90°. Assumes the current point is already at the arc start.
fn arc(cmds: &mut Vec<PathCmd>, r: f64, a: f64, b: f64) {
    let total = b - a;
    let segments = (total.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
    let step = total / segments as f64;
    for s in 0..segments {
        let s0 = a + step * s as f64;
        let s1 = s0 + step;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let p0 = Point::polar(r, s0);
        let p3 = Point::polar(r, s1);
        let c1 = Point::new(p0.x - k * r * s0.sin(), p0.y + k * r * s0.cos());
        let c2 = Point::new(p3.x + k * r * s1.sin(), p3.y - k * r * s1.cos());
        cmds.push(PathCmd::CubicTo { c1, c2, to: p3 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g() -> WheelGeometry {
        WheelGeometry::default()
    }

    #[test]
    fn first_petal_is_at_top_and_order_is_clockwise() {
        let a0 = WheelGeometry::petal_angle(0, 6);
        let a1 = WheelGeometry::petal_angle(1, 6);
        assert!((a0 - FRAC_PI_2).abs() < 1e-9);
        // Clockwise means the next petal sits to the right of the top one.
        assert!(a1.cos() > 0.0 && a1 < a0);
        assert!((WheelGeometry::petal_angle(0, 3) - PI).abs() < 1e-9);
    }

    #[test]
    fn petal_centres_hit_their_own_index() {
        for n in 1..=12 {
            for i in 0..n {
                let p = Point::polar((g().inner + g().outer) / 2.0, WheelGeometry::petal_angle(i, n));
                assert_eq!(g().hit_test(p.x, p.y, n), Some(i), "n={n} i={i}");
            }
        }
    }

    #[test]
    fn centre_outside_and_gaps_hit_nothing() {
        assert_eq!(g().hit_test(0.0, 0.0, 6), None);
        assert_eq!(g().hit_test(10.0, 10.0, 6), None);
        assert_eq!(g().hit_test(200.0, 0.0, 6), None);
        // Exactly on the boundary ray between petal 0 and 1.
        let boundary = WheelGeometry::petal_angle(0, 6) - PI / 6.0;
        let p = Point::polar(80.0, boundary);
        assert_eq!(g().hit_test(p.x, p.y, 6), None);
        assert_eq!(g().hit_test(1.0, 80.0, 0), None);
    }

    #[test]
    fn real_demo_layout_six_audio_formats() {
        // E3: for a WAV file, MP3 is on top, M4A upper right, … AIFF upper left.
        let up = g().hit_test(0.0, 90.0, 6);
        let upper_right = g().hit_test(70.0, 45.0, 6);
        let upper_left = g().hit_test(-70.0, 45.0, 6);
        assert_eq!(up, Some(0));
        assert_eq!(upper_right, Some(1));
        assert_eq!(upper_left, Some(5));
    }

    #[test]
    fn petal_paths_are_closed_and_within_outer_radius() {
        for n in 1..=10 {
            for i in 0..n {
                let path = g().petal_path(i, n);
                assert!(matches!(path.first(), Some(PathCmd::MoveTo(_))));
                assert!(matches!(path.last(), Some(PathCmd::Close)));
                for cmd in &path {
                    let pts: Vec<Point> = match *cmd {
                        PathCmd::MoveTo(p) | PathCmd::LineTo(p) => vec![p],
                        PathCmd::CubicTo { to, .. } => vec![to],
                        PathCmd::Close => vec![],
                    };
                    for p in pts {
                        let r = p.x.hypot(p.y);
                        assert!(r <= g().outer + 1e-6, "n={n} i={i} r={r}");
                        assert!(r >= g().inner - 1e-6, "n={n} i={i} r={r}");
                    }
                }
            }
        }
    }

    #[test]
    fn labels_sit_inside_their_petals() {
        for n in 1..=10 {
            for i in 0..n {
                let p = g().label_position(i, n);
                assert_eq!(g().hit_test(p.x, p.y, n), Some(i));
            }
        }
    }

    #[test]
    fn anchor_is_clamped_on_screen() {
        let p = g().clamp_anchor(Point::new(5.0, 5.0), (0.0, 0.0, 1000.0, 800.0));
        assert_eq!(p, Point::new(136.0, 136.0));
        let p = g().clamp_anchor(Point::new(500.0, 400.0), (0.0, 0.0, 1000.0, 800.0));
        assert_eq!(p, Point::new(500.0, 400.0));
        let p = g().clamp_anchor(Point::new(999.0, 799.0), (0.0, 0.0, 1000.0, 800.0));
        assert_eq!(p, Point::new(864.0, 664.0));
    }

    #[test]
    fn angle_diff_wraps() {
        assert!((angle_diff(0.1, TAU - 0.1) - 0.2).abs() < 1e-9);
        assert!((angle_diff(-PI + 0.1, PI - 0.1) - 0.2).abs() < 1e-9);
    }
}
