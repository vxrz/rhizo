//! Motion: diagrams easing from one layout to the next, and the camera
//! easing between views. Pure functions of time `t` in 0..=1, so they test
//! without a window.

use std::collections::HashMap;
use std::time::Duration;

use rhizo_core::{Color, Point, Rect};
use rhizo_layout::{Diagram, EdgeBox, EdgeKind, Path, Seg};
use vello::kurbo::Vec2;

/// How long a relayout or a camera move takes.
pub const DURATION: Duration = Duration::from_millis(180);

/// shortcut: relayouts of bigger diagrams snap instead of animating; a tween
/// frame plus scene redraw costs ~6.5 ms per 5k nodes (release), so this keeps
/// a frame near 13 ms. Upgrade by tweening cached per-node sub-scenes.
pub const MAX_ANIMATED_NODES: usize = 10_000;

/// Fast start, gentle landing (cubic).
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// The diagram `t` (already eased) of the way from `old` to `new`, in `new`'s
/// coordinates; `shift` moves `old` there (see [`rhizo_layout::stable::anchor_shift`]). Nodes and
/// frames are matched by id, edges by endpoints. What only `new` has fades
/// and grows in, what only `old` has fades out.
pub fn frame(old: &Diagram, new: &Diagram, shift: Point, t: f32) -> Diagram {
    let mv = |p: Point| Point::new(p.x + shift.x, p.y + shift.y);
    let mv_rect = |r: Rect| r.translate(shift.x, shift.y);
    let mut out = new.clone();

    let old_frames: HashMap<&str, _> = old.frames.iter().map(|f| (f.id.as_str(), f)).collect();
    for f in &mut out.frames {
        match old_frames.get(f.id.as_str()) {
            Some(o) => f.rect = lerp_rect(mv_rect(o.rect), f.rect, t),
            None => (f.fill, f.stroke, f.label_color) = (fade(f.fill, t), fade(f.stroke, t), fade(f.label_color, t)),
        }
    }
    let new_frames: HashMap<&str, ()> = new.frames.iter().map(|f| (f.id.as_str(), ())).collect();
    for f in old.frames.iter().filter(|f| !new_frames.contains_key(f.id.as_str())) {
        let mut f = f.clone();
        f.rect = mv_rect(f.rect);
        (f.fill, f.stroke, f.label_color) =
            (fade(f.fill, 1.0 - t), fade(f.stroke, 1.0 - t), fade(f.label_color, 1.0 - t));
        out.frames.push(f);
    }

    let old_nodes: HashMap<&str, _> = old.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    for n in &mut out.nodes {
        match old_nodes.get(n.id.as_str()) {
            Some(o) => {
                n.rect = lerp_rect(mv_rect(o.rect), n.rect, t);
                n.fill = o.fill.mix(n.fill, t);
                n.stroke = o.stroke.mix(n.stroke, t);
                n.text = o.text.mix(n.text, t);
            }
            None => grow(n, 0.7 + 0.3 * t, t),
        }
    }
    let new_nodes: HashMap<&str, ()> = new.nodes.iter().map(|n| (n.id.as_str(), ())).collect();
    for n in old.nodes.iter().filter(|n| !new_nodes.contains_key(n.id.as_str())) {
        let mut n = n.clone();
        n.rect = mv_rect(n.rect);
        grow(&mut n, 1.0 - 0.15 * t, 1.0 - t);
        out.nodes.push(n);
    }

    // Edges pair up per (from, to, kind) in order, like `edge_sources`.
    let key = |e: &EdgeBox| (e.from.as_str().to_string(), e.to.as_str().to_string(), e.kind == EdgeKind::Branch);
    let mut old_edges: HashMap<_, Vec<&EdgeBox>> = HashMap::new();
    for e in old.edges.iter().rev() {
        old_edges.entry(key(e)).or_default().push(e);
    }
    for e in &mut out.edges {
        match old_edges.get_mut(&key(e)).and_then(Vec::pop) {
            Some(o) => {
                let from = translate(&o.path, shift);
                let label_from = o.label.as_ref().map(|l| mv(l.center));
                tween_edge(e, &from, label_from, t);
            }
            None => fade_edge(e, t),
        }
    }
    for e in old_edges.into_values().flatten() {
        let mut e = e.clone();
        e.path = translate(&e.path, shift);
        if let Some(l) = &mut e.label {
            l.center = mv(l.center);
        }
        fade_edge(&mut e, 1.0 - t);
        out.edges.push(e);
    }
    out
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    let l = |x: f32, y: f32| x + (y - x) * t;
    Rect::new(l(a.x, b.x), l(a.y, b.y), l(a.width, b.width), l(a.height, b.height))
}

/// `c` with its alpha scaled by `k`.
fn fade(c: Color, k: f32) -> Color {
    Color { a: (c.a as f32 * k.clamp(0.0, 1.0)).round() as u8, ..c }
}

/// Scales a node about its center (text too) and fades it.
fn grow(n: &mut rhizo_layout::NodeBox, scale: f32, alpha: f32) {
    let c = n.rect.center();
    n.rect = Rect::from_center(c, rhizo_core::Size::new(n.rect.width * scale, n.rect.height * scale));
    n.font_size *= scale;
    n.line_height *= scale;
    (n.fill, n.stroke, n.text) = (fade(n.fill, alpha), fade(n.stroke, alpha), fade(n.text, alpha));
}

fn fade_edge(e: &mut EdgeBox, k: f32) {
    e.color = fade(e.color, k);
    if let Some(l) = &mut e.label {
        (l.color, l.background) = (fade(l.color, k), fade(l.background, k));
    }
}

fn translate(p: &Path, d: Point) -> Path {
    map_path(p, |q| Point::new(q.x + d.x, q.y + d.y))
}

fn map_path(p: &Path, mut f: impl FnMut(Point) -> Point) -> Path {
    let start = f(p.start);
    let segs = p
        .segs
        .iter()
        .map(|s| match *s {
            Seg::Line(q) => Seg::Line(f(q)),
            Seg::Cubic(a, b, q) => Seg::Cubic(f(a), f(b), f(q)),
        })
        .collect();
    Path { start, segs }
}

/// Moves `e` from `old` towards where it is now. Same-shaped paths blend
/// point by point; otherwise the new shape rides on the blended endpoints.
fn tween_edge(e: &mut EdgeBox, old: &Path, old_label: Option<Point>, t: f32) {
    let same_shape = old.segs.len() == e.path.segs.len()
        && old.segs.iter().zip(&e.path.segs).all(|(a, b)| std::mem::discriminant(a) == std::mem::discriminant(b));
    if same_shape {
        let mut olds = points(old).into_iter();
        e.path = map_path(&e.path, |q| olds.next().map_or(q, |p| p.lerp(q, t)));
        if let (Some(l), Some(o)) = (&mut e.label, old_label) {
            l.center = o.lerp(l.center, t);
        }
    } else {
        let (a, b) = (e.path.start, e.path.end());
        let place = similarity(a, b, old.start.lerp(a, t), old.end().lerp(b, t));
        e.path = map_path(&e.path, place);
        if let Some(l) = &mut e.label {
            l.center = place(l.center);
        }
    }
}

fn points(p: &Path) -> Vec<Point> {
    let mut v = vec![p.start];
    for s in &p.segs {
        match *s {
            Seg::Line(q) => v.push(q),
            Seg::Cubic(a, b, q) => v.extend([a, b, q]),
        }
    }
    v
}

/// The rotation, scale and translation taking `a` to `a2` and `b` to `b2`
/// (just the translation when `a` and `b` coincide).
fn similarity(a: Point, b: Point, a2: Point, b2: Point) -> impl Fn(Point) -> Point + Copy {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (ex, ey) = (b2.x - a2.x, b2.y - a2.y);
    let len2 = dx * dx + dy * dy;
    // As complex numbers: k = (b2 - a2) / (b - a); p ↦ a2 + k (p - a).
    let (kr, ki) = if len2 < 1e-3 { (1.0, 0.0) } else { ((ex * dx + ey * dy) / len2, (ey * dx - ex * dy) / len2) };
    move |p: Point| {
        let (px, py) = (p.x - a.x, p.y - a.y);
        Point::new(a2.x + kr * px - ki * py, a2.y + ki * px + kr * py)
    }
}

/// Diagram (scene) units → physical pixels: `pixel = offset + scene * zoom`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub zoom: f64,
    pub offset: Vec2,
}

impl Camera {
    /// `t` of the way to `to`, keeping the window `center` steady: the point
    /// under it glides in a straight line while zoom changes geometrically.
    pub fn lerp(self, to: Camera, t: f64, center: Vec2) -> Camera {
        let (c0, c1) = ((center - self.offset) / self.zoom, (center - to.offset) / to.zoom);
        let zoom = self.zoom * (to.zoom / self.zoom).powf(t);
        Camera { zoom, offset: center - c0.lerp(c1, t) * zoom }
    }
}

/// How far to pan so `r` (pixels) sits inside the window `size` with
/// `margin` to spare; zero when it already does. Too big to fit: show its
/// top-left.
pub fn reveal(r: vello::kurbo::Rect, size: Vec2, margin: f64) -> Vec2 {
    let axis = |lo: f64, hi: f64, max: f64| {
        if lo < margin || hi - lo > max - 2.0 * margin {
            margin - lo
        } else if hi > max - margin {
            max - margin - hi
        } else {
            0.0
        }
    };
    Vec2::new(axis(r.x0, r.x1, size.x), axis(r.y0, r.y1, size.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_core::Theme;
    use rhizo_lang::{Syntax, load};

    fn diagram(src: &str) -> Diagram {
        let doc = load(src, Syntax::Kdl).into_result().unwrap().0;
        rhizo_layout::layout(&doc, &Theme::light(), &rhizo_layout::Heuristic)
    }

    fn node<'a>(d: &'a Diagram, id: &str) -> &'a rhizo_layout::NodeBox {
        d.nodes.iter().find(|n| n.id.as_str() == id).unwrap()
    }

    #[test]
    fn easing_starts_fast_and_lands() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(2.0), 1.0);
        assert!(ease_out(0.5) > 0.8, "most of the way by half time");
        assert!((0..10).all(|i| ease_out(i as f32 / 10.0) < ease_out((i + 1) as f32 / 10.0)));
    }

    #[test]
    fn frames_run_from_old_to_new() {
        let old = diagram(r#"- "Root" { - "A"; - "B" }"#);
        let new = diagram(r#"- "Root" { - "A" { - "A1" }; - "B" }"#);
        let shift = rhizo_layout::stable::anchor_shift(&old, &new, &[]);
        // Ends: exactly the new layout, plus nothing left over at t = 1.
        let end = frame(&old, &new, shift, 1.0);
        assert_eq!(node(&end, "a").rect, node(&new, "a").rect);
        let gone: Vec<_> = end.nodes.iter().filter(|n| n.fill.a == 0).collect();
        assert!(gone.is_empty());
        // Start: shared nodes where they were, the new one invisible.
        let start = frame(&old, &new, shift, 0.0);
        let was = node(&old, "a").rect.translate(shift.x, shift.y);
        assert_eq!(node(&start, "a").rect, was);
        assert_eq!(node(&start, "a1").fill.a, 0);
        assert!(node(&start, "a1").rect.width < node(&new, "a1").rect.width);
        // Halfway: in between.
        let mid = node(&frame(&old, &new, shift, 0.5), "a").rect;
        let n = node(&new, "a").rect;
        assert!((mid.x - (was.x + n.x) / 2.0).abs() < 0.01 && (mid.y - (was.y + n.y) / 2.0).abs() < 0.01);
        assert_eq!(frame(&old, &new, shift, 0.5).edges.len(), new.edges.len());
    }

    #[test]
    fn removed_things_fade_out() {
        let old = diagram(r#"- "Root" { - "A"; - "B" }"#);
        let new = diagram(r#"- "Root" { - "A" }"#);
        let f = frame(&old, &new, rhizo_layout::stable::anchor_shift(&old, &new, &[]), 0.5);
        let b = node(&f, "b");
        assert!(b.fill.a > 0 && b.fill.a < 255);
        assert_eq!(f.edges.len(), 2, "the branch to B is still fading");
        let end = frame(&old, &new, Point::new(0.0, 0.0), 1.0);
        assert_eq!(node(&end, "b").fill.a, 0);
    }

    #[test]
    fn reshaped_edges_keep_their_ends_on_the_nodes() {
        let a = Path::line(Point::new(0.0, 0.0), Point::new(10.0, 0.0));
        let b = Path {
            start: Point::new(0.0, 10.0),
            segs: vec![Seg::Line(Point::new(5.0, 10.0)), Seg::Line(Point::new(20.0, 20.0))],
        };
        let place = similarity(b.start, b.end(), a.start, a.end());
        let (s, e) = (place(b.start), place(b.end()));
        assert!(s.distance(a.start) < 1e-4 && e.distance(a.end()) < 1e-4);
    }

    #[test]
    fn camera_glides_between_views() {
        let center = Vec2::new(400.0, 300.0);
        let a = Camera { zoom: 1.0, offset: Vec2::new(0.0, 0.0) };
        let b = Camera { zoom: 4.0, offset: Vec2::new(-200.0, 100.0) };
        assert_eq!(a.lerp(b, 0.0, center), a);
        let end = a.lerp(b, 1.0, center);
        assert!((end.zoom - 4.0).abs() < 1e-9 && (end.offset - b.offset).hypot() < 1e-9);
        assert!((a.lerp(b, 0.5, center).zoom - 2.0).abs() < 1e-9, "zoom moves geometrically");
    }

    #[test]
    fn reveal_pans_just_enough() {
        let size = Vec2::new(800.0, 600.0);
        let r = |x0, y0, x1, y1| vello::kurbo::Rect::new(x0, y0, x1, y1);
        assert_eq!(reveal(r(100.0, 100.0, 200.0, 150.0), size, 20.0), Vec2::ZERO);
        assert_eq!(reveal(r(780.0, 100.0, 880.0, 150.0), size, 20.0), Vec2::new(-100.0, 0.0));
        assert_eq!(reveal(r(-50.0, 590.0, 50.0, 640.0), size, 20.0), Vec2::new(70.0, -60.0));
    }
}
