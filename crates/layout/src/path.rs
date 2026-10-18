use rhizo_core::{Point, Rect, Shape};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Line(Point),
    Cubic(Point, Point, Point),
}

impl Seg {
    pub fn end(&self) -> Point {
        match *self {
            Seg::Line(p) | Seg::Cubic(_, _, p) => p,
        }
    }
}

/// A connected path of lines and cubic Béziers.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub start: Point,
    pub segs: Vec<Seg>,
}

impl Path {
    pub fn line(a: Point, b: Point) -> Path {
        Path { start: a, segs: vec![Seg::Line(b)] }
    }

    pub fn polyline(points: &[Point]) -> Path {
        Path { start: points[0], segs: points[1..].iter().map(|&p| Seg::Line(p)).collect() }
    }

    pub fn end(&self) -> Point {
        self.segs.last().map_or(self.start, Seg::end)
    }

    fn seg_start(&self, i: usize) -> Point {
        if i == 0 { self.start } else { self.segs[i - 1].end() }
    }

    /// Direction the path leaves its start, as a unit vector.
    pub fn start_tangent(&self) -> Point {
        let Some(first) = self.segs.first() else { return Point::new(1.0, 0.0) };
        let towards = match *first {
            Seg::Line(p) => p,
            Seg::Cubic(c1, c2, p) => [c1, c2, p].into_iter().find(|q| q.distance(self.start) > 0.01).unwrap_or(p),
        };
        unit(self.start, towards)
    }

    /// Direction the path arrives at its end, as a unit vector.
    pub fn end_tangent(&self) -> Point {
        let Some(last) = self.segs.last() else { return Point::new(1.0, 0.0) };
        let end = last.end();
        let from = match *last {
            Seg::Line(_) => self.seg_start(self.segs.len() - 1),
            Seg::Cubic(c1, c2, _) => {
                [c2, c1, self.seg_start(self.segs.len() - 1)].into_iter().find(|q| q.distance(end) > 0.01).unwrap_or(c1)
            }
        };
        unit(from, end)
    }

    /// A point roughly halfway along the path (by segment, then parameter).
    pub fn midpoint(&self) -> Point {
        if self.segs.is_empty() {
            return self.start;
        }
        let lengths: Vec<f32> = (0..self.segs.len()).map(|i| self.seg_start(i).distance(self.segs[i].end())).collect();
        let total: f32 = lengths.iter().sum();
        let mut acc = 0.0;
        for (i, len) in lengths.iter().enumerate() {
            if acc + len >= total / 2.0 || i == self.segs.len() - 1 {
                let t = if *len > 0.0 { ((total / 2.0 - acc) / len).clamp(0.0, 1.0) } else { 0.5 };
                return self.eval(i, t);
            }
            acc += len;
        }
        self.end()
    }

    fn eval(&self, i: usize, t: f32) -> Point {
        let p0 = self.seg_start(i);
        match self.segs[i] {
            Seg::Line(p) => p0.lerp(p, t),
            Seg::Cubic(c1, c2, p) => {
                let u = 1.0 - t;
                let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                Point::new(
                    w[0] * p0.x + w[1] * c1.x + w[2] * c2.x + w[3] * p.x,
                    w[0] * p0.y + w[1] * c1.y + w[2] * c2.y + w[3] * p.y,
                )
            }
        }
    }

    /// Conservative bounds (control points included).
    pub fn bounds(&self) -> Rect {
        let mut pts = vec![self.start];
        for s in &self.segs {
            match *s {
                Seg::Line(p) => pts.push(p),
                Seg::Cubic(a, b, p) => pts.extend([a, b, p]),
            }
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p.x);
            y0 = y0.min(p.y);
            x1 = x1.max(p.x);
            y1 = y1.max(p.y);
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn map(&self, f: impl Fn(Point) -> Point) -> Path {
        Path {
            start: f(self.start),
            segs: self
                .segs
                .iter()
                .map(|s| match *s {
                    Seg::Line(p) => Seg::Line(f(p)),
                    Seg::Cubic(a, b, p) => Seg::Cubic(f(a), f(b), f(p)),
                })
                .collect(),
        }
    }

    pub fn reversed(&self) -> Path {
        let mut points = vec![self.start];
        points.extend(self.segs.iter().map(Seg::end));
        let mut segs = Vec::with_capacity(self.segs.len());
        for i in (0..self.segs.len()).rev() {
            let start = points[i];
            segs.push(match self.segs[i] {
                Seg::Line(_) => Seg::Line(start),
                Seg::Cubic(c1, c2, _) => Seg::Cubic(c2, c1, start),
            });
        }
        Path { start: self.end(), segs }
    }
}

fn unit(a: Point, b: Point) -> Point {
    let d = a.distance(b);
    if d < 1e-6 { Point::new(1.0, 0.0) } else { Point::new((b.x - a.x) / d, (b.y - a.y) / d) }
}

/// Where the ray from the center of `rect` toward `toward` leaves the shape.
pub fn boundary(rect: &Rect, shape: Shape, toward: Point) -> Point {
    let c = rect.center();
    let (dx, dy) = (toward.x - c.x, toward.y - c.y);
    if dx.abs() < 1e-6 && dy.abs() < 1e-6 {
        return c;
    }
    let (a, b) = (rect.width / 2.0, rect.height / 2.0);
    let t = match shape {
        Shape::Ellipse | Shape::Circle => 1.0 / ((dx / a).powi(2) + (dy / b).powi(2)).sqrt(),
        Shape::Diamond => 1.0 / (dx.abs() / a + dy.abs() / b),
        Shape::Hexagon => {
            // Flat top and bottom; the left and right points sit at mid-height.
            let inset = (rect.height * 0.35).min(a);
            let t_flat = if dy.abs() > 1e-6 { b / dy.abs() } else { f32::MAX };
            let t_side = 1.0 / (dx.abs() / a + dy.abs() * (inset / a) / b).max(1e-6);
            t_flat.min(t_side)
        }
        Shape::Pill => {
            let r = b.min(a);
            let t_rect = (if dx.abs() > 1e-6 { a / dx.abs() } else { f32::MAX }).min(if dy.abs() > 1e-6 {
                b / dy.abs()
            } else {
                f32::MAX
            });
            let hit = Point::new(dx * t_rect, dy * t_rect);
            if hit.x.abs() <= a - r {
                t_rect
            } else {
                // Intersect with the end cap circle.
                let cx = (a - r) * dx.signum();
                let (qa, qb, qc) = (dx * dx + dy * dy, -2.0 * cx * dx, cx * cx - r * r);
                let disc = (qb * qb - 4.0 * qa * qc).max(0.0);
                (-qb + disc.sqrt()) / (2.0 * qa)
            }
        }
        Shape::Rect | Shape::Rounded | Shape::Text => (if dx.abs() > 1e-6 { a / dx.abs() } else { f32::MAX })
            .min(if dy.abs() > 1e-6 { b / dy.abs() } else { f32::MAX }),
    };
    Point::new(c.x + dx * t, c.y + dy * t)
}

/// Distance from the center to the shape's edge along the depth axis, at
/// breadth offset `o` from the center. Used to place spread-out ports.
pub fn port_depth(shape: Shape, half_breadth: f32, half_depth: f32, o: f32) -> f32 {
    let r = (o.abs() / half_breadth.max(1e-6)).min(1.0);
    match shape {
        Shape::Diamond => half_depth * (1.0 - r),
        Shape::Ellipse | Shape::Circle => half_depth * (1.0 - r * r).sqrt(),
        _ => half_depth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_boundary() {
        let r = Rect::new(0.0, 0.0, 100.0, 40.0);
        let p = boundary(&r, Shape::Rect, Point::new(500.0, 20.0));
        assert!((p.x - 100.0).abs() < 1e-3 && (p.y - 20.0).abs() < 1e-3);
        let p = boundary(&r, Shape::Diamond, Point::new(50.0, 500.0));
        assert!((p.y - 40.0).abs() < 1e-3);
        let p = boundary(&r, Shape::Ellipse, Point::new(500.0, 20.0));
        assert!((p.x - 100.0).abs() < 1e-3);
        let p = boundary(&r, Shape::Pill, Point::new(500.0, 20.0));
        assert!((p.x - 100.0).abs() < 1e-3, "{p:?}");
    }

    #[test]
    fn reverse_roundtrips() {
        let p = Path {
            start: Point::new(0.0, 0.0),
            segs: vec![
                Seg::Cubic(Point::new(0.0, 5.0), Point::new(10.0, 5.0), Point::new(10.0, 10.0)),
                Seg::Line(Point::new(20.0, 10.0)),
            ],
        };
        assert_eq!(p.reversed().reversed(), p);
        assert_eq!(p.reversed().end(), p.start);
    }
}
