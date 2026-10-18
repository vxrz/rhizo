//! Layered graph drawing (Sugiyama style).
//!
//! 1. Break cycles by reversing DFS back edges.
//! 2. Assign layers by longest path, then pull sources down next to their
//!    successors.
//! 3. Split long edges with dummy nodes. If any edge has a label, every layer
//!    is doubled so each edge gets a dummy that holds its label and reserves
//!    room for it.
//! 4. Reduce crossings: barycenter sweeps plus adjacent transpositions, keeping
//!    the best ordering seen.
//! 5. Assign breadth coordinates by repeatedly projecting each layer onto its
//!    neighbors' weighted mean. Each projection is an exact weighted isotonic
//!    regression (pool adjacent violators) under minimum-separation
//!    constraints. Dummy chains weigh more, so long edges come out straight.
//!
//! Coordinates are abstract (breadth, depth); the caller maps them to x/y.

use rhizo_core::Shape;

use crate::path::port_depth;

pub struct Input<'a> {
    /// `(breadth, depth)` extent of each node.
    pub size: &'a [(f32, f32)],
    pub shape: &'a [Shape],
    pub edges: &'a [(usize, usize)],
    /// Optional `(breadth, depth)` extent of each edge's label.
    pub labels: &'a [Option<(f32, f32)>],
    pub node_sep: f32,
    pub rank_sep: f32,
}

pub struct Output {
    /// Top-left `(breadth, depth)` of each node.
    pub pos: Vec<(f32, f32)>,
    /// Per edge: points from source to target, in `(breadth, depth)`. The
    /// first and last are on the node outlines; the rest are dummy centers.
    /// Empty for self loops, which the caller draws.
    pub routes: Vec<Vec<(f32, f32)>>,
    /// Center of each edge's label, if it has one.
    pub label_at: Vec<Option<(f32, f32)>>,
}

const DUMMY_WEIGHT: f32 = 8.0;
const MIXED_WEIGHT: f32 = 2.0;

pub fn layout(input: &Input) -> Output {
    let n = input.size.len();
    let has_labels = input.labels.iter().any(Option::is_some);

    // 1. Orient edges into a DAG.
    let real: Vec<(usize, usize, usize)> =
        input.edges.iter().enumerate().filter(|(_, (a, b))| a != b).map(|(i, &(a, b))| (i, a, b)).collect();
    let reversed = back_edges(n, &real);
    let dag: Vec<(usize, usize)> = real.iter().map(|&(i, a, b)| if reversed[i] { (b, a) } else { (a, b) }).collect();

    // 2. Layers.
    let mut layer = longest_path(n, &dag);
    if has_labels {
        for l in &mut layer {
            *l *= 2;
        }
    }

    // 3. Dummies. Virtual node ids: 0..n are real, n.. are dummies.
    let mut vsize: Vec<(f32, f32)> = input.size.to_vec();
    let mut vlayer = layer.clone();
    let mut chains: Vec<Vec<usize>> = Vec::with_capacity(real.len());
    let mut label_dummy: Vec<Option<usize>> = vec![None; input.edges.len()];
    for (k, &(i, _, _)) in real.iter().enumerate() {
        let (a, b) = dag[k];
        let mut chain = vec![a];
        let span = vlayer[b] - vlayer[a];
        let mid = span / 2;
        for s in 1..span {
            let id = vsize.len();
            let size = match input.labels[i] {
                Some(sz) if s == mid => {
                    label_dummy[i] = Some(id);
                    (sz.0 + 8.0, sz.1)
                }
                _ => (0.0, 0.0),
            };
            vsize.push(size);
            vlayer.push(vlayer[a] + s);
            chain.push(id);
        }
        chain.push(b);
        chains.push(chain);
    }
    let vn = vsize.len();
    let mut up: Vec<Vec<usize>> = vec![Vec::new(); vn];
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); vn];
    for chain in &chains {
        for w in chain.windows(2) {
            down[w[0]].push(w[1]);
            up[w[1]].push(w[0]);
        }
    }
    let nlayers = vlayer.iter().copied().max().map_or(0, |m| m + 1);

    // 4. Ordering.
    let mut layers = initial_order(vn, &vlayer, &down, nlayers);
    reduce_crossings(&mut layers, &up, &down, vn);

    // 5. Coordinates.
    let is_dummy = |v: usize| v >= n;
    let x = assign_breadth(&layers, &vsize, &up, &down, input.node_sep, &is_dummy);

    let mut layer_depth = vec![0.0f32; nlayers];
    for v in 0..vn {
        layer_depth[vlayer[v]] = layer_depth[vlayer[v]].max(vsize[v].1);
    }
    let rank_sep = if has_labels { input.rank_sep / 2.0 } else { input.rank_sep };
    let mut layer_top = vec![0.0f32; nlayers];
    let mut acc = 0.0;
    for l in 0..nlayers {
        layer_top[l] = acc;
        // Layers of only unlabeled dummies take no space of their own.
        acc += layer_depth[l] + if layer_depth[l] > 0.0 || !has_labels { rank_sep } else { rank_sep * 0.5 };
    }
    let center_depth = |v: usize| layer_top[vlayer[v]] + layer_depth[vlayer[v]] / 2.0;

    let mut pos = vec![(0.0, 0.0); n];
    let min_x = (0..vn).map(|v| x[v] - vsize[v].0 / 2.0).fold(f32::INFINITY, f32::min);
    let min_x = if min_x.is_finite() { min_x } else { 0.0 };
    for v in 0..n {
        pos[v] = (x[v] - vsize[v].0 / 2.0 - min_x, center_depth(v) - vsize[v].1 / 2.0);
    }

    // Ports: spread edges along the side they leave or enter, ordered by
    // where they go so they do not cross at the node.
    let next_of = |chain: &Vec<usize>, from_end: bool| if from_end { chain[chain.len() - 2] } else { chain[1] };
    let mut out_ports: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    let mut in_ports: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    for (k, chain) in chains.iter().enumerate() {
        let (a, b) = (chain[0], *chain.last().unwrap());
        out_ports[a].push((k, x[next_of(chain, false)]));
        in_ports[b].push((k, x[next_of(chain, true)]));
    }
    let spread = |ports: &mut Vec<(usize, f32)>, v: usize| -> Vec<(usize, f32)> {
        ports.sort_by(|p, q| p.1.total_cmp(&q.1));
        let k = ports.len();
        let usable = vsize[v].0 * 0.5;
        let step = if k > 1 { (usable / (k - 1) as f32).min(12.0) } else { 0.0 };
        ports.iter().enumerate().map(|(i, &(e, _))| (e, (i as f32 - (k - 1) as f32 / 2.0) * step)).collect()
    };
    let mut start_off = vec![0.0f32; chains.len()];
    let mut end_off = vec![0.0f32; chains.len()];
    for v in 0..n {
        for (e, o) in spread(&mut out_ports[v], v) {
            start_off[e] = o;
        }
        for (e, o) in spread(&mut in_ports[v], v) {
            end_off[e] = o;
        }
    }

    let mut routes = vec![Vec::new(); input.edges.len()];
    let mut label_at = vec![None; input.edges.len()];
    for (k, &(i, _, _)) in real.iter().enumerate() {
        let chain = &chains[k];
        let (a, b) = (chain[0], *chain.last().unwrap());
        let edge_at = |v: usize, off: f32, bottom: bool| {
            let (hb, hd) = (vsize[v].0 / 2.0, vsize[v].1 / 2.0);
            let d = port_depth(input.shape[v], hb, hd, off);
            let c = center_depth(v);
            (x[v] + off - min_x, if bottom { c + d } else { c - d })
        };
        let mut pts = vec![edge_at(a, start_off[k], true)];
        let end = edge_at(b, end_off[k], false);
        let inner = &chain[1..chain.len() - 1];
        for (j, &d) in inner.iter().enumerate() {
            let p = (x[d] - min_x, center_depth(d));
            let next = inner.get(j + 1).map_or(end, |&e| (x[e] - min_x, center_depth(e)));
            // Dummies that sit on the line between their neighbors add only
            // kinks; keep the ones that bend the edge or carry its label.
            if Some(d) == label_dummy[i] || deviation(*pts.last().unwrap(), next, p) > 6.0 {
                pts.push(p);
            }
        }
        pts.push(end);
        if reversed[i] {
            pts.reverse();
        }
        routes[i] = pts;
        label_at[i] = label_dummy[i].map(|d| (x[d] - min_x, center_depth(d)));
    }
    Output { pos, routes, label_at }
}

/// Distance from `p` to the segment `a`–`b`.
fn deviation(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((a.0 + t * dx - p.0).powi(2) + (a.1 + t * dy - p.1).powi(2)).sqrt()
}

/// Marks edges that close a cycle in a DFS from sources in input order.
fn back_edges(n: usize, edges: &[(usize, usize, usize)]) -> Vec<bool> {
    let max_id = edges.iter().map(|e| e.0).max().map_or(0, |m| m + 1);
    let mut reversed = vec![false; max_id];
    let mut out: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(i, a, b) in edges {
        out[a].push((b, i));
        indeg[b] += 1;
    }
    // 0 = unvisited, 1 = on stack, 2 = done
    let mut state = vec![0u8; n];
    let starts = (0..n).filter(|&v| indeg[v] == 0).chain(0..n);
    for s in starts {
        if state[s] != 0 {
            continue;
        }
        let mut stack = vec![(s, 0usize)];
        state[s] = 1;
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            if *next < out[v].len() {
                let (w, i) = out[v][*next];
                *next += 1;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => reversed[i] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    reversed
}

fn longest_path(n: usize, dag: &[(usize, usize)]) -> Vec<usize> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut inc: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for &(a, b) in dag {
        out[a].push(b);
        inc[b].push(a);
        indeg[b] += 1;
    }
    let mut order = Vec::with_capacity(n);
    let mut queue: std::collections::VecDeque<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    while let Some(v) = queue.pop_front() {
        order.push(v);
        for &w in &out[v] {
            indeg[w] -= 1;
            if indeg[w] == 0 {
                queue.push_back(w);
            }
        }
    }
    let mut layer = vec![0usize; n];
    for &v in &order {
        for &w in &out[v] {
            layer[w] = layer[w].max(layer[v] + 1);
        }
    }
    // Pull nodes without predecessors down to just above their successors.
    for &v in order.iter().rev() {
        if inc[v].is_empty() && !out[v].is_empty() {
            layer[v] = out[v].iter().map(|&w| layer[w]).min().unwrap() - 1;
        }
    }
    layer
}

fn initial_order(vn: usize, vlayer: &[usize], down: &[Vec<usize>], nlayers: usize) -> Vec<Vec<usize>> {
    let mut layers = vec![Vec::new(); nlayers];
    let mut seen = vec![false; vn];
    let mut has_up = vec![false; vn];
    for d in down {
        for &w in d {
            has_up[w] = true;
        }
    }
    let starts: Vec<usize> = (0..vn).filter(|&v| !has_up[v]).chain(0..vn).collect();
    for s in starts {
        if seen[s] {
            continue;
        }
        let mut stack = vec![s];
        while let Some(v) = stack.pop() {
            if seen[v] {
                continue;
            }
            seen[v] = true;
            layers[vlayer[v]].push(v);
            for &w in down[v].iter().rev() {
                if !seen[w] {
                    stack.push(w);
                }
            }
        }
    }
    layers
}

fn positions(layers: &[Vec<usize>], vn: usize) -> Vec<usize> {
    let mut pos = vec![0; vn];
    for l in layers {
        for (i, &v) in l.iter().enumerate() {
            pos[v] = i;
        }
    }
    pos
}

/// Crossings between two adjacent layers, counted as inversions with a
/// Fenwick tree in O(E log V).
fn crossings_between(upper: &[usize], lower_len: usize, down: &[Vec<usize>], pos: &[usize]) -> usize {
    let mut ends: Vec<(usize, usize)> = Vec::new();
    for &u in upper {
        for &w in &down[u] {
            ends.push((pos[u], pos[w]));
        }
    }
    ends.sort_unstable();
    let mut tree = vec![0usize; lower_len + 1];
    let mut count = 0;
    for (k, &(_, w)) in ends.iter().enumerate() {
        // Edges seen so far ending strictly right of w cross this one.
        let mut i = w + 1;
        let mut le = 0;
        while i > 0 {
            le += tree[i];
            i &= i - 1;
        }
        count += k - le;
        let mut i = w + 1;
        while i <= lower_len {
            tree[i] += 1;
            i += i & i.wrapping_neg();
        }
    }
    count
}

fn total_crossings(layers: &[Vec<usize>], down: &[Vec<usize>], vn: usize) -> usize {
    let pos = positions(layers, vn);
    layers.windows(2).map(|w| crossings_between(&w[0], w[1].len(), down, &pos)).sum()
}

fn reduce_crossings(layers: &mut Vec<Vec<usize>>, up: &[Vec<usize>], down: &[Vec<usize>], vn: usize) {
    let mut best = layers.clone();
    let mut best_count = total_crossings(layers, down, vn);
    if best_count == 0 {
        return;
    }
    for round in 0..12 {
        let downward = round % 2 == 0;
        let mut pos = positions(layers, vn);
        let range: Vec<usize> =
            if downward { (1..layers.len()).collect() } else { (0..layers.len().saturating_sub(1)).rev().collect() };
        for l in range {
            let nbrs = if downward { up } else { down };
            let mut keyed: Vec<(f32, usize)> = layers[l]
                .iter()
                .map(|&v| {
                    let ns = &nbrs[v];
                    let key = if ns.is_empty() {
                        pos[v] as f32
                    } else {
                        ns.iter().map(|&w| pos[w] as f32).sum::<f32>() / ns.len() as f32
                    };
                    (key, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            layers[l] = keyed.into_iter().map(|(_, v)| v).collect();
            for (i, &v) in layers[l].iter().enumerate() {
                pos[v] = i;
            }
        }
        transpose(layers, up, down, vn);
        let c = total_crossings(layers, down, vn);
        if c < best_count {
            best_count = c;
            best = layers.clone();
            if c == 0 {
                break;
            }
        }
    }
    *layers = best;
}

/// Swaps adjacent nodes while that reduces crossings with both neighbor layers.
fn transpose(layers: &mut [Vec<usize>], up: &[Vec<usize>], down: &[Vec<usize>], vn: usize) {
    let mut pos = positions(layers, vn);
    let pair_cross = |a: usize, b: usize, nbrs: &[Vec<usize>], pos: &[usize]| -> usize {
        // Crossings among edges of a (left) and b (right) to one neighbor layer.
        let mut c = 0;
        for &x in &nbrs[a] {
            for &y in &nbrs[b] {
                if pos[x] > pos[y] {
                    c += 1;
                }
            }
        }
        c
    };
    for _ in 0..4 {
        let mut improved = false;
        for layer in layers.iter_mut() {
            for i in 0..layer.len().saturating_sub(1) {
                let (a, b) = (layer[i], layer[i + 1]);
                let now = pair_cross(a, b, up, &pos) + pair_cross(a, b, down, &pos);
                let swapped = pair_cross(b, a, up, &pos) + pair_cross(b, a, down, &pos);
                if swapped < now {
                    layer.swap(i, i + 1);
                    pos[a] = i + 1;
                    pos[b] = i;
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
}

fn assign_breadth(
    layers: &[Vec<usize>],
    vsize: &[(f32, f32)],
    up: &[Vec<usize>],
    down: &[Vec<usize>],
    node_sep: f32,
    is_dummy: &dyn Fn(usize) -> bool,
) -> Vec<f32> {
    let vn = vsize.len();
    let sep = |a: usize, b: usize| {
        let gap = match (is_dummy(a), is_dummy(b)) {
            (false, false) => node_sep,
            (true, true) => node_sep * 0.25,
            _ => node_sep * 0.5,
        };
        (vsize[a].0 + vsize[b].0) / 2.0 + gap
    };
    let weight = |a: usize, b: usize| match (is_dummy(a), is_dummy(b)) {
        (true, true) => DUMMY_WEIGHT,
        (false, false) => 1.0,
        _ => MIXED_WEIGHT,
    };

    let mut x = vec![0.0f32; vn];
    for l in layers {
        let mut cur = 0.0;
        for (i, &v) in l.iter().enumerate() {
            if i > 0 {
                cur += sep(l[i - 1], v);
            }
            x[v] = cur;
        }
        let mid = cur / 2.0;
        for &v in l {
            x[v] -= mid;
        }
    }

    let project = |l: &[usize], x: &mut Vec<f32>, use_up: bool, use_down: bool| {
        let mut target = Vec::with_capacity(l.len());
        let mut w = Vec::with_capacity(l.len());
        for &v in l {
            let mut sw = 0.0;
            let mut swx = 0.0;
            if use_up {
                for &u in &up[v] {
                    let k = weight(u, v);
                    sw += k;
                    swx += k * x[u];
                }
            }
            if use_down {
                for &d in &down[v] {
                    let k = weight(v, d);
                    sw += k;
                    swx += k * x[d];
                }
            }
            if sw > 0.0 {
                target.push(swx / sw);
                w.push(sw);
            } else {
                target.push(x[v]);
                w.push(0.05);
            }
        }
        let gaps: Vec<f32> = l.windows(2).map(|p| sep(p[0], p[1])).collect();
        let solved = isotonic_with_gaps(&target, &w, &gaps);
        for (i, &v) in l.iter().enumerate() {
            x[v] = solved[i];
        }
    };

    for round in 0..8 {
        if round % 2 == 0 {
            for l in layers.iter().skip(1) {
                project(l, &mut x, true, false);
            }
        } else {
            for l in layers.iter().rev().skip(1) {
                project(l, &mut x, false, true);
            }
        }
    }
    for _ in 0..3 {
        for l in layers {
            project(l, &mut x, true, true);
        }
    }
    x
}

/// Minimizes Σ wᵢ(xᵢ − tᵢ)² subject to xᵢ₊₁ − xᵢ ≥ gapᵢ, exactly, in O(n),
/// by substituting yᵢ = xᵢ − Σ gaps and running pool-adjacent-violators.
pub(crate) fn isotonic_with_gaps(t: &[f32], w: &[f32], gaps: &[f32]) -> Vec<f32> {
    let n = t.len();
    let mut offset = vec![0.0f32; n];
    for i in 1..n {
        offset[i] = offset[i - 1] + gaps[i - 1];
    }
    // Blocks of (weight sum, weighted value sum, count).
    let mut blocks: Vec<(f32, f32, usize)> = Vec::with_capacity(n);
    for i in 0..n {
        let u = t[i] - offset[i];
        blocks.push((w[i], w[i] * u, 1));
        while blocks.len() > 1 {
            let (w2, s2, c2) = blocks[blocks.len() - 1];
            let (w1, s1, c1) = blocks[blocks.len() - 2];
            if s1 / w1 > s2 / w2 {
                blocks.pop();
                *blocks.last_mut().unwrap() = (w1 + w2, s1 + s2, c1 + c2);
            } else {
                break;
            }
        }
    }
    let mut out = Vec::with_capacity(n);
    for (ws, s, c) in blocks {
        let y = s / ws;
        for _ in 0..c {
            let i = out.len();
            out.push(y + offset[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(n: usize, edges: &[(usize, usize)], labels: bool) -> Output {
        let size = vec![(60.0, 30.0); n];
        let shape = vec![Shape::Rounded; n];
        let labels: Vec<_> = edges.iter().map(|_| labels.then_some((30.0, 14.0))).collect();
        layout(&Input { size: &size, shape: &shape, edges, labels: &labels, node_sep: 20.0, rank_sep: 40.0 })
    }

    fn no_overlap(out: &Output) {
        for i in 0..out.pos.len() {
            for j in i + 1..out.pos.len() {
                let (a, b) = (out.pos[i], out.pos[j]);
                let apart = a.0 + 60.0 <= b.0 + 0.01
                    || b.0 + 60.0 <= a.0 + 0.01
                    || a.1 + 30.0 <= b.1 + 0.01
                    || b.1 + 30.0 <= a.1 + 0.01;
                assert!(apart, "nodes {i} and {j} overlap: {a:?} {b:?}");
            }
        }
    }

    #[test]
    fn chain_is_straight() {
        let out = run(3, &[(0, 1), (1, 2)], false);
        assert_eq!(out.pos[0].0, out.pos[1].0);
        assert_eq!(out.pos[1].0, out.pos[2].0);
        assert!(out.pos[0].1 < out.pos[1].1 && out.pos[1].1 < out.pos[2].1);
    }

    #[test]
    fn diamond_is_symmetric() {
        let out = run(4, &[(0, 1), (0, 2), (1, 3), (2, 3)], false);
        no_overlap(&out);
        let c = |i: usize| out.pos[i].0 + 30.0;
        assert!((c(0) - c(3)).abs() < 0.5);
        assert!(((c(1) + c(2)) / 2.0 - c(0)).abs() < 0.5);
    }

    #[test]
    fn cycles_and_labels() {
        let out = run(3, &[(0, 1), (1, 2), (2, 0), (1, 1)], true);
        no_overlap(&out);
        assert!(out.routes[3].is_empty());
        // The back edge still runs from 2 to 0.
        let r = &out.routes[2];
        assert!(r.first().unwrap().1 < r.last().unwrap().1 || r.len() > 2);
        assert!(out.label_at.iter().take(3).all(Option::is_some));
    }

    #[test]
    fn untangles_crossing() {
        // 0,1 on top; 2,3 below; edges cross in input order.
        let out = run(4, &[(0, 3), (1, 2)], false);
        let x = |i: usize| out.pos[i].0;
        assert_eq!(x(0) < x(1), x(3) < x(2));
    }

    #[test]
    fn isotonic_respects_gaps() {
        let x = isotonic_with_gaps(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0], &[10.0, 10.0]);
        assert_eq!(x, vec![-10.0, 0.0, 10.0]);
        let x = isotonic_with_gaps(&[0.0, 100.0], &[1.0, 1.0], &[10.0]);
        assert_eq!(x, vec![0.0, 100.0]);
    }

    #[test]
    fn larger_graph_is_fast_and_clean() {
        let n = 400;
        let mut edges = Vec::new();
        for i in 1..n {
            edges.push(((i - 1) / 3, i));
            if i % 7 == 0 {
                edges.push((i / 2, i));
            }
        }
        let t = std::time::Instant::now();
        let out = run(n, &edges, false);
        no_overlap(&out);
        assert!(t.elapsed().as_secs_f32() < 2.0);
    }
}
