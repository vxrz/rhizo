//! Routes edges around the boxes they would otherwise cross.
//!
//! Used for edges no layout engine routes: links inside tree, mindmap, grid
//! and free frames, and edges between frames. Waypoints are the corners of
//! obstacles grown by [`MARGIN`]; A* over their visibility graph, with a
//! cost for turning, finds a short path with few bends, which is then
//! smoothed into a curve.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::f32::consts::FRAC_1_SQRT_2;

use rhizo_core::{Point, Rect, Shape};

use crate::path::{self, Path, Seg, unit};

/// Clearance a routed edge keeps from the boxes it passes.
const MARGIN: f32 = 10.0;
/// Extra clearance per earlier edge that bent around the same corner.
const LANE: f32 = 8.0;
/// Cost of turning, in pixels per radian.
const BEND: f32 = 12.0;
/// How many grid cells around a waypoint the search looks for the next.
const RING: usize = 2;
/// Segment tests all routing in one layout may spend. Past it, edges keep
/// their plain curves.
// shortcut: a fixed budget caps huge, densely linked documents at plain
// curves; upgrade to an incremental orthogonal visibility graph if those
// need routing too.
const BUDGET: usize = 2_000_000;

/// A waypoint: position, direction its corner points (zero at the ends),
/// and which box and corner it is.
type Vertex = (Point, Point, Option<(usize, usize)>);

/// Boxes to keep clear of, bucketed on a uniform grid.
pub(crate) struct Obstacles {
    rects: Vec<Rect>,
    bounds: Rect,
    cell: f32,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
    /// Per box and corner, how many routes already bend there.
    used: Vec<[u16; 4]>,
    /// Segment tests left; see `BUDGET`.
    budget: Cell<usize>,
}

impl Obstacles {
    pub fn new(rects: Vec<Rect>) -> Self {
        let bounds = Rect::bounding(&rects).unwrap_or_default().inflate(MARGIN, MARGIN);
        // About two boxes per cell, and at most 256 cells a side.
        let cell = (bounds.width * bounds.height * 2.0 / rects.len().max(1) as f32)
            .sqrt()
            .max(bounds.width.max(bounds.height) / 256.0)
            .max(16.0);
        let cols = (bounds.width / cell).ceil().max(1.0) as usize;
        let rows = (bounds.height / cell).ceil().max(1.0) as usize;
        let used = vec![[0; 4]; rects.len()];
        let mut o = Obstacles {
            rects: Vec::new(),
            bounds,
            cell,
            cols,
            rows,
            cells: vec![Vec::new(); cols * rows],
            used,
            budget: Cell::new(BUDGET),
        };
        for (i, r) in rects.iter().enumerate() {
            let (c0, c1, r0, r1) = o.span(&r.inflate(MARGIN, MARGIN));
            for row in r0..=r1 {
                for col in c0..=c1 {
                    o.cells[row * cols + col].push(i as u32);
                }
            }
        }
        o.rects = rects;
        o
    }

    /// Column and row ranges of the cells `r` touches.
    fn span(&self, r: &Rect) -> (usize, usize, usize, usize) {
        let col = |x: f32| (((x - self.bounds.x) / self.cell).floor().max(0.0) as usize).min(self.cols - 1);
        let row = |y: f32| (((y - self.bounds.y) / self.cell).floor().max(0.0) as usize).min(self.rows - 1);
        (col(r.x), col(r.right()), row(r.y), row(r.bottom()))
    }

    /// Boxes that, grown by `MARGIN`, may meet `r`. Sorted.
    fn near(&self, r: &Rect) -> Vec<usize> {
        let (c0, c1, r0, r1) = self.span(r);
        let mut out: Vec<usize> = (r0..=r1)
            .flat_map(|row| (c0..=c1).flat_map(move |col| &self.cells[row * self.cols + col]))
            .map(|&i| i as usize)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Whether the segment `p`–`q` passes through a box grown by `grow`
    /// (at most `MARGIN`), other than those in `skip`.
    fn blocked(&self, p: Point, q: Point, grow: f32, skip: &[usize]) -> bool {
        let (_, _, r0, r1) = self.span(&Rect::new(p.x, p.y.min(q.y), 0.0, (p.y - q.y).abs()));
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let x_at = |y: f32| if dy == 0.0 { p.x } else { p.x + dx * ((y - p.y) / dy).clamp(0.0, 1.0) };
        for row in r0..=r1 {
            // Only the cells the segment's stretch within this row touches.
            let top = self.bounds.y + row as f32 * self.cell;
            let (xa, xb) = if dy == 0.0 { (p.x, q.x) } else { (x_at(top), x_at(top + self.cell)) };
            let (c0, c1, _, _) = self.span(&Rect::new(xa.min(xb) - 1.0, top, (xa - xb).abs() + 2.0, 0.0));
            for col in c0..=c1 {
                for &i in &self.cells[row * self.cols + col] {
                    let i = i as usize;
                    if !skip.contains(&i) && crosses(p, q, &self.rects[i].inflate(grow, grow)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether `p` lies inside a box grown by `grow`, other than those in `skip`.
    fn inside(&self, p: Point, grow: f32, skip: &[usize]) -> bool {
        let (c, _, r, _) = self.span(&Rect::new(p.x, p.y, 0.0, 0.0));
        self.cells[r * self.cols + c].iter().any(|&i| {
            let b = self.rects[i as usize].inflate(grow, grow);
            !skip.contains(&(i as usize)) && p.x > b.x && p.x < b.right() && p.y > b.y && p.y < b.bottom()
        })
    }

    /// Whether `path` passes through any box other than those in `skip`.
    pub fn hits(&self, path: &Path, skip: &[usize]) -> bool {
        path.flatten(12).windows(2).any(|w| self.blocked(w[0], w[1], 0.0, skip))
    }
}

/// Whether the segment `p`–`q` passes through the interior of `r`. Touching
/// its outline does not count.
pub(crate) fn crosses(p: Point, q: Point, r: &Rect) -> bool {
    let (dx, dy) = (q.x - p.x, q.y - p.y);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (pk, qk) in [(-dx, p.x - r.x), (dx, r.right() - p.x), (-dy, p.y - r.y), (dy, r.bottom() - p.y)] {
        if pk == 0.0 {
            if qk <= 0.0 {
                return false;
            }
        } else if pk < 0.0 {
            t0 = t0.max(qk / pk);
        } else {
            t1 = t1.min(qk / pk);
        }
    }
    t1 - t0 > 1e-4
}

/// A path from `a`'s outline to `b`'s that keeps clear of every obstacle not
/// in `skip`. Routes bending around a corner an earlier route used pass
/// further out, so they do not coincide; `lane` does the same for the n-th
/// of several parallel edges that need no bend. `None` if no way around was
/// found.
pub(crate) fn route(
    obs: &mut Obstacles,
    skip: &[usize],
    (a, a_shape): (&Rect, Shape),
    (b, b_shape): (&Rect, Shape),
    lane: usize,
) -> Option<Path> {
    if obs.budget.get() == 0 {
        return None;
    }
    let span = a.union(b);
    // Bend only around boxes in a corridor along the straight line, widened
    // until a way through is found.
    let (from, to) = (a.center(), b.center());
    let mut pad = MARGIN * 6.0 + from.distance(to) * 0.1;
    let (route, skip) = loop {
        let region = span.inflate(pad, pad);
        let near: Vec<usize> = obs
            .near(&region)
            .into_iter()
            .filter(|&i| {
                let r = &obs.rects[i];
                to_segment(r.center(), from, to) <= pad + (r.width + r.height) / 2.0
            })
            .collect();
        if let Some(found) = search(obs, &near, skip, from, to) {
            break found;
        }
        if region.union(&obs.bounds) == region || obs.budget.get() == 0 {
            return None;
        }
        pad *= 3.0;
    };

    let mut pts: Vec<Point> = route
        .iter()
        .map(|&(p, s, corner)| {
            let Some((i, c)) = corner else { return p };
            let shift = obs.used[i][c] as f32 * LANE * FRAC_1_SQRT_2;
            obs.used[i][c] = obs.used[i][c].saturating_add(1);
            p.offset(s.x * shift, s.y * shift)
        })
        .collect();
    let k = pts.len();
    pts[0] = path::boundary(a, a_shape, pts[1]);
    pts[k - 1] = path::boundary(b, b_shape, pts[k - 2]);
    if k == 2 {
        let (p, q) = (pts[0], pts[1]);
        let n = unit(p, q);
        let off = lane as f32 * LANE * 2.0;
        let mid = p.lerp(q, 0.5).offset(-n.y * off, n.x * off);
        return Some(Path { start: p, segs: vec![Seg::Cubic(p.lerp(mid, 0.66), q.lerp(mid, 0.66), q)] });
    }
    [smooth(&pts), crate::rounded_polyline(&pts, 12.0)]
        .into_iter()
        .find(|p| !obs.hits(p, &skip))
        .or_else(|| Some(Path::polyline(&pts)))
}

/// A short route from `from` to `to` bending only at corners of the
/// obstacles in `near`, each with the direction its corner points (zero at
/// the ends) and which box and corner it is. Also returns the obstacles it
/// ignored: `skip`, plus any too close to an end to get around.
fn search(
    obs: &Obstacles,
    near: &[usize],
    skip: &[usize],
    from: Point,
    to: Point,
) -> Option<(Vec<Vertex>, Vec<usize>)> {
    let close = near.iter().copied().filter(|&i| {
        let r = obs.rects[i].inflate(MARGIN, MARGIN);
        !skip.contains(&i) && (r.contains(from) || r.contains(to))
    });
    let skip: Vec<usize> = skip.iter().copied().chain(close).collect();
    let mut verts: Vec<Vertex> = vec![(from, Point::default(), None), (to, Point::default(), None)];
    for &i in near.iter().filter(|i| !skip.contains(i)) {
        let r = obs.rects[i].inflate(MARGIN, MARGIN);
        for (c, (sx, sy)) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].into_iter().enumerate() {
            let p = Point::new(if sx < 0.0 { r.x } else { r.right() }, if sy < 0.0 { r.y } else { r.bottom() });
            if !obs.inside(p, MARGIN - 0.5, &skip) {
                verts.push((p, Point::new(sx, sy), Some((i, c))));
            }
        }
    }

    // Each vertex links to the corners in nearby cells, to corners of boxes
    // too big for that (frames, mostly) and to the goal. Paths are
    // straightened afterwards, so the short hops cost little.
    let n = verts.len();
    let mut bucket: Vec<Vec<u32>> = vec![Vec::new(); obs.cols * obs.rows];
    let mut wide: Vec<u32> = vec![1];
    for (v, &(p, _, corner)) in verts.iter().enumerate().skip(2) {
        let r = &obs.rects[corner.map_or(0, |(i, _)| i)];
        if r.width.max(r.height) > obs.cell * RING as f32 {
            wide.push(v as u32);
        } else {
            let (c, _, row, _) = obs.span(&Rect::new(p.x, p.y, 0.0, 0.0));
            bucket[row * obs.cols + c].push(v as u32);
        }
    }
    let mut g = vec![f32::INFINITY; n];
    let mut parent = vec![usize::MAX; n];
    let mut done = vec![false; n];
    let mut open = BinaryHeap::new();
    g[0] = 0.0;
    open.push(Open(from.distance(to), 0));
    while let Some(Open(_, u)) = open.pop() {
        if std::mem::replace(&mut done[u], true) {
            continue;
        }
        if u == 1 {
            let mut path = vec![1];
            while parent[*path.last().unwrap()] != usize::MAX {
                path.push(parent[*path.last().unwrap()]);
            }
            path.reverse();
            return Some((straighten(obs, &verts, &path, &skip), skip));
        }
        if obs.budget.get() == 0 {
            return None;
        }
        let pu = verts[u].0;
        let heading = (parent[u] != usize::MAX).then(|| unit(verts[parent[u]].0, pu));
        let (c, _, row, _) = obs.span(&Rect::new(pu.x, pu.y, 0.0, 0.0));
        let bucket = &bucket;
        let ring = (row.saturating_sub(RING)..=(row + RING).min(obs.rows - 1)).flat_map(|row| {
            (c.saturating_sub(RING)..=(c + RING).min(obs.cols - 1)).flat_map(move |col| &bucket[row * obs.cols + col])
        });
        for &v in wide.iter().chain(ring) {
            let v = v as usize;
            let (pv, s, _) = verts[v];
            let (wx, wy) = (pv.x - pu.x, pv.y - pu.y);
            // A corner is only worth bending at if the line through it
            // grazes its obstacle instead of entering it.
            if done[v] || (v > 1 && (wy * s.x) * (-wx * s.y) < 0.0) {
                continue;
            }
            let len = (wx * wx + wy * wy).sqrt();
            if len < 1e-3 {
                continue;
            }
            let turn = heading.map_or(0.0, |h| ((h.x * wx + h.y * wy) / len).clamp(-1.0, 1.0).acos());
            let cost = g[u] + len + BEND * turn;
            if cost >= g[v] {
                continue;
            }
            obs.budget.set(obs.budget.get().saturating_sub(1));
            if obs.blocked(pu, pv, MARGIN - 0.5, &skip) {
                continue;
            }
            g[v] = cost;
            parent[v] = u;
            // Slightly greedy: far fewer expansions for a near-shortest route.
            open.push(Open(cost + 1.2 * pv.distance(to), v));
        }
    }
    None
}

/// Drops waypoints the route can skip: from each kept one, jump to the
/// furthest one in plain view.
fn straighten(obs: &Obstacles, verts: &[Vertex], path: &[usize], skip: &[usize]) -> Vec<Vertex> {
    let mut out = vec![verts[path[0]]];
    let mut i = 0;
    while i < path.len() - 1 {
        let mut j = path.len() - 1;
        while j > i + 1 && obs.blocked(verts[path[i]].0, verts[path[j]].0, MARGIN - 0.5, skip) {
            j -= 1;
        }
        out.push(verts[path[j]]);
        i = j;
    }
    out
}

/// Distance from `p` to the segment `a`–`b`.
fn to_segment(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a.offset(dx * t, dy * t))
}

/// An open A* vertex: estimated total cost, index. Pops cheapest first, ties
/// by index so the result never depends on heap internals.
struct Open(f32, usize);

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

/// A curve through `pts`. Each inner point's tangent follows its neighbors
/// (Catmull-Rom direction); handles scale with their own segment so short
/// hops do not overshoot.
fn smooth(pts: &[Point]) -> Path {
    let n = pts.len();
    let dir = |i: usize| unit(pts[i.saturating_sub(1)], pts[(i + 1).min(n - 1)]);
    let segs = (0..n - 1)
        .map(|i| {
            let (p, q) = (pts[i], pts[i + 1]);
            let k = p.distance(q) / 3.0;
            let (tp, tq) = (dir(i), dir(i + 1));
            Seg::Cubic(p.offset(tp.x * k, tp.y * k), q.offset(-tq.x * k, -tq.y * k), q)
        })
        .collect();
    Path { start: pts[0], segs }
}
