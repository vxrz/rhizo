//! Network layout by stress majorization (Gansner, Koren & North 2004).
//!
//! Each pair of nodes gets an ideal distance: their graph distance times an
//! edge length. Stress, Σ w·(|xᵢ−xⱼ| − dᵢⱼ)² with w = d⁻², is minimized by
//! repeated weighted averaging (SMACOF), which only ever lowers it, so the
//! result is deterministic and settles without tuning. Components are laid
//! out apart and packed in a row; a last pass pushes overlapping boxes apart.
//!
//! O(n²) per iteration: fine to a few thousand nodes.
// shortcut: all-pairs stress; switch to sparse stress (pivots) for 10k+ nodes (K7).

/// Iterations after an edit, from the previous layout: enough to make room
/// for what changed, few enough that the rest stays put.
const KEPT_ROUNDS: usize = 30;
/// How hard a node is held where it was, against all its stress terms.
const KEEP: f64 = 3.0;

pub struct Spacing {
    /// Gap between two connected nodes' borders.
    pub edge: f32,
    /// Gap between components and between boxes after overlap removal.
    pub gap: f32,
}

/// Centers for nodes of the given `(width, height)`, top-left of the whole
/// drawing at 0,0.
pub fn layout(size: &[(f32, f32)], edges: &[(usize, usize)], sp: &Spacing) -> Vec<(f32, f32)> {
    layout_from(size, edges, sp, &[])
}

/// After an edit: iterations that start from the previous layout's centers
/// (`before`, where there is one), so the drawing settles near where it
/// was instead of from a circle, and does not turn or mirror. New nodes
/// start beside their neighbors. Components stay where they were; new ones
/// go to the right. Fewer iterations: an edit is a small change.
pub fn layout_from(
    size: &[(f32, f32)],
    edges: &[(usize, usize)],
    sp: &Spacing,
    before: &[Option<(f32, f32)>],
) -> Vec<(f32, f32)> {
    let n = size.len();
    let mut adj = vec![Vec::new(); n];
    for &(a, b) in edges {
        if a != b {
            adj[a].push(b);
            adj[b].push(a);
        }
    }
    let mut pos = vec![(0.0f32, 0.0f32); n];
    let mut seen = vec![false; n];
    let mut x_cursor = 0.0f32;
    let was = |v: usize| before.get(v).copied().flatten();
    if (0..n).any(|v| was(v).is_some()) {
        // New components go right of everything that was there.
        let right = (0..n).filter_map(|v| Some(was(v)?.0 + size[v].0 / 2.0)).fold(f32::MIN, f32::max);
        let top = (0..n).filter_map(|v| Some(was(v)?.1 - size[v].1 / 2.0)).fold(f32::MAX, f32::min);
        x_cursor = right + sp.gap * 2.0;
        for start in 0..n {
            if seen[start] {
                continue;
            }
            let comp = bfs_order(&adj, start, &mut seen);
            let kept = comp.iter().any(|&v| was(v).is_some());
            let local = if kept {
                let anchor: Vec<_> = comp.iter().map(|&v| was(v).map(|(x, y)| (x as f64, y as f64))).collect();
                component(&comp, &adj, size, sp, Some(starts(&comp, &adj, size, sp, &was)), &anchor)
            } else {
                component(&comp, &adj, size, sp, None, &[])
            };
            if kept {
                for (k, &v) in comp.iter().enumerate() {
                    pos[v] = local[k];
                }
            } else {
                let bb = bounds(&comp, &local, size);
                for (k, &v) in comp.iter().enumerate() {
                    pos[v] = (local[k].0 - bb.0 + x_cursor, local[k].1 - bb.1 + top);
                }
                x_cursor += bb.2 - bb.0 + sp.gap * 2.0;
            }
        }
        remove_overlaps(&mut pos, size, sp.gap);
        return pos;
    }
    for start in 0..n {
        if seen[start] {
            continue;
        }
        let comp = bfs_order(&adj, start, &mut seen);
        let local = component(&comp, &adj, size, sp, None, &[]);
        // Pack components left to right, tops aligned.
        let bb = bounds(&comp, &local, size);
        for (k, &v) in comp.iter().enumerate() {
            pos[v] = (local[k].0 - bb.0 + x_cursor, local[k].1 - bb.1);
        }
        x_cursor += bb.2 - bb.0 + sp.gap * 2.0;
    }
    remove_overlaps(&mut pos, size, sp.gap);
    let min_x = (0..n).map(|i| pos[i].0 - size[i].0 / 2.0).fold(f32::INFINITY, f32::min);
    let min_y = (0..n).map(|i| pos[i].1 - size[i].1 / 2.0).fold(f32::INFINITY, f32::min);
    if n > 0 {
        for p in &mut pos {
            p.0 -= min_x;
            p.1 -= min_y;
        }
    }
    pos
}

fn bfs_order(adj: &[Vec<usize>], start: usize, seen: &mut [bool]) -> Vec<usize> {
    let mut order = vec![start];
    seen[start] = true;
    let mut i = 0;
    while i < order.len() {
        for &w in &adj[order[i]] {
            if !seen[w] {
                seen[w] = true;
                order.push(w);
            }
        }
        i += 1;
    }
    order
}

/// Where a component's nodes start from after an edit, in `comp` order:
/// where they were, or (new) a step from the neighbors placed before them.
fn starts(
    comp: &[usize],
    adj: &[Vec<usize>],
    size: &[(f32, f32)],
    sp: &Spacing,
    was: &dyn Fn(usize) -> Option<(f32, f32)>,
) -> Vec<(f64, f64)> {
    let mut at: std::collections::HashMap<usize, (f64, f64)> =
        comp.iter().filter_map(|&v| Some((v, was(v).map(|(x, y)| (x as f64, y as f64))?))).collect();
    // BFS order reaches every new node through a placed one, except when
    // the component's first node is new too; that one starts at the mean.
    let n = at.len().max(1) as f64;
    let mean = at.values().fold((0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n));
    for (k, &v) in comp.iter().enumerate() {
        if at.contains_key(&v) {
            continue;
        }
        let near: Vec<(f64, f64)> = adj[v].iter().filter_map(|w| at.get(w).copied()).collect();
        let base = if near.is_empty() {
            mean
        } else {
            let c = near.len() as f64;
            near.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0 / c, a.1 + p.1 / c))
        };
        // A fixed step in a direction set by the node's place, so new
        // siblings do not start on top of each other.
        let step = (size[v].0.max(size[v].1) + sp.edge) as f64;
        let angle = k as f64 * 2.399_963; // the golden angle
        at.insert(v, (base.0 + step * angle.cos(), base.1 + step * angle.sin()));
    }
    comp.iter().map(|v| at[v]).collect()
}

/// Stress majorization for one connected component, from a circle or from
/// `start`. Returns centers in `comp` order.
fn component(
    comp: &[usize],
    adj: &[Vec<usize>],
    size: &[(f32, f32)],
    sp: &Spacing,
    start: Option<Vec<(f64, f64)>>,
    anchor: &[Option<(f64, f64)>],
) -> Vec<(f32, f32)> {
    let m = comp.len();
    if m == 1 {
        let (x, y) = start.map_or((0.0, 0.0), |s| s[0]);
        return vec![(x as f32, y as f32)];
    }
    let local: std::collections::HashMap<usize, usize> = comp.iter().enumerate().map(|(k, &v)| (v, k)).collect();
    // Edge length from typical box size, so big labels get room.
    let diag = comp.iter().map(|&v| (size[v].0.powi(2) + size[v].1.powi(2)).sqrt()).sum::<f32>() / m as f32;
    let unit = (diag + sp.edge) as f64;

    // All-pairs graph distances by BFS.
    let mut d = vec![0u32; m * m];
    for s in 0..m {
        let mut dist = vec![u32::MAX; m];
        dist[s] = 0;
        let mut queue = std::collections::VecDeque::from([s]);
        while let Some(u) = queue.pop_front() {
            for &w in &adj[comp[u]] {
                let k = local[&w];
                if dist[k] == u32::MAX {
                    dist[k] = dist[u] + 1;
                    queue.push_back(k);
                }
            }
        }
        d[s * m..(s + 1) * m].copy_from_slice(&dist);
    }

    // Deterministic start: a circle in BFS order, sized to the edge length.
    let r = unit * m as f64 / (2.0 * std::f64::consts::PI);
    let rounds = if start.is_some() { KEPT_ROUNDS } else { 300 };
    let mut x: Vec<(f64, f64)> = start.unwrap_or_else(|| {
        (0..m).map(|k| (k as f64 / m as f64) * std::f64::consts::TAU).map(|a| (r * a.cos(), r * a.sin())).collect()
    });

    let target = |i: usize, j: usize| d[i * m + j] as f64 * unit;
    let stress = |x: &[(f64, f64)]| -> f64 {
        let mut s = 0.0;
        for i in 0..m {
            for j in i + 1..m {
                let t = target(i, j);
                let dist = ((x[i].0 - x[j].0).powi(2) + (x[i].1 - x[j].1).powi(2)).sqrt();
                s += (dist - t).powi(2) / (t * t);
            }
        }
        s
    };
    let mut last = stress(&x);
    for _ in 0..rounds {
        for i in 0..m {
            let (mut nx, mut ny, mut wsum) = (0.0, 0.0, 0.0);
            for j in 0..m {
                if i == j {
                    continue;
                }
                let t = target(i, j);
                let w = 1.0 / (t * t);
                let (dx, dy) = (x[i].0 - x[j].0, x[i].1 - x[j].1);
                let dist = (dx * dx + dy * dy).sqrt().max(1e-6);
                nx += w * (x[j].0 + t * dx / dist);
                ny += w * (x[j].1 + t * dy / dist);
                wsum += w;
            }
            // After an edit, nodes that were there are also pulled to where
            // they were (anchored stress), so a new edge bends the drawing
            // locally instead of reshaping it.
            if let Some(Some(p)) = anchor.get(i) {
                let a = KEEP * wsum;
                nx += a * p.0;
                ny += a * p.1;
                wsum += a;
            }
            x[i] = (nx / wsum, ny / wsum);
        }
        let now = stress(&x);
        if anchor.is_empty() && last - now < 1e-5 * last {
            break;
        }
        last = now;
    }
    x.into_iter().map(|(a, b)| (a as f32, b as f32)).collect()
}

/// `(min_x, min_y, max_x, max_y)` of the boxes.
fn bounds(comp: &[usize], centers: &[(f32, f32)], size: &[(f32, f32)]) -> (f32, f32, f32, f32) {
    let mut b = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (k, &v) in comp.iter().enumerate() {
        let (cx, cy) = centers[k];
        let (hw, hh) = (size[v].0 / 2.0, size[v].1 / 2.0);
        b = (b.0.min(cx - hw), b.1.min(cy - hh), b.2.max(cx + hw), b.3.max(cy + hh));
    }
    b
}

/// Pushes overlapping boxes apart along the axis of least overlap until none
/// overlap (or a pass limit is hit).
fn remove_overlaps(pos: &mut [(f32, f32)], size: &[(f32, f32)], gap: f32) {
    let n = pos.len();
    for _ in 0..200 {
        let mut moved = false;
        for i in 0..n {
            for j in i + 1..n {
                let ox = (size[i].0 + size[j].0) / 2.0 + gap - (pos[i].0 - pos[j].0).abs();
                let oy = (size[i].1 + size[j].1) / 2.0 + gap - (pos[i].1 - pos[j].1).abs();
                if ox <= 0.0 || oy <= 0.0 {
                    continue;
                }
                moved = true;
                // Ties (same spot) split by index, so the result stays deterministic.
                if ox < oy {
                    let s = if pos[i].0 < pos[j].0 || (pos[i].0 == pos[j].0 && i < j) { -1.0 } else { 1.0 };
                    pos[i].0 += s * ox / 2.0;
                    pos[j].0 -= s * ox / 2.0;
                } else {
                    let s = if pos[i].1 < pos[j].1 || (pos[i].1 == pos[j].1 && i < j) { -1.0 } else { 1.0 };
                    pos[i].1 += s * oy / 2.0;
                    pos[j].1 -= s * oy / 2.0;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp() -> Spacing {
        Spacing { edge: 40.0, gap: 16.0 }
    }

    fn dist(p: &[(f32, f32)], a: usize, b: usize) -> f32 {
        ((p[a].0 - p[b].0).powi(2) + (p[a].1 - p[b].1).powi(2)).sqrt()
    }

    fn overlaps(p: &[(f32, f32)], s: &[(f32, f32)]) -> bool {
        (0..p.len()).any(|i| {
            (i + 1..p.len()).any(|j| {
                (p[i].0 - p[j].0).abs() < (s[i].0 + s[j].0) / 2.0 - 0.01
                    && (p[i].1 - p[j].1).abs() < (s[i].1 + s[j].1) / 2.0 - 0.01
            })
        })
    }

    #[test]
    fn neighbors_sit_closer_than_strangers() {
        // A 6-cycle with one chord: graph distance should show in the drawing.
        let size = vec![(80.0, 30.0); 6];
        let edges = [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 0), (0, 3)];
        let p = layout(&size, &edges, &sp());
        assert!(!overlaps(&p, &size));
        assert!(dist(&p, 0, 1) < dist(&p, 1, 4), "1 and 4 are 3 hops apart, 0 and 1 are neighbors");
        assert!(p.iter().all(|&(x, y)| x >= 0.0 && y >= 0.0));
    }

    #[test]
    fn deterministic_and_components_apart() {
        let size = vec![(60.0, 30.0); 5];
        let edges = [(0, 1), (1, 2), (3, 4)];
        let a = layout(&size, &edges, &sp());
        let b = layout(&size, &edges, &sp());
        assert_eq!(a, b);
        assert!(!overlaps(&a, &size));
        let max_left = (0..3).map(|i| a[i].0 + 30.0).fold(f32::MIN, f32::max);
        let min_right = (3..5).map(|i| a[i].0 - 30.0).fold(f32::MAX, f32::min);
        assert!(min_right > max_left, "second component sits to the right");
    }

    #[test]
    fn handles_edge_cases() {
        assert!(layout(&[], &[], &sp()).is_empty());
        assert_eq!(layout(&[(10.0, 10.0)], &[(0, 0)], &sp()), vec![(5.0, 5.0)]);
        let size = vec![(50.0, 20.0); 4];
        let p = layout(&size, &[], &sp());
        assert!(!overlaps(&p, &size));
    }

    #[test]
    fn medium_graph_is_quick() {
        let n = 300;
        let size = vec![(90.0, 32.0); n];
        let edges: Vec<(usize, usize)> =
            (1..n).map(|i| ((i - 1) / 3, i)).chain((0..n / 10).map(|i| (i * 7 % n, i * 13 % n))).collect();
        let t = std::time::Instant::now();
        let p = layout(&size, &edges, &sp());
        assert!(!overlaps(&p, &size));
        assert!(t.elapsed().as_secs_f32() < 10.0, "debug build, took {:?}", t.elapsed());
    }
}
