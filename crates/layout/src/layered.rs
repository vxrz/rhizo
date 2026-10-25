//! Layered graph drawing (Sugiyama style).
//!
//! 1. Break cycles by reversing DFS back edges.
//! 2. Assign layers by network simplex, minimizing total edge span.
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
//!
//! Given a previous layout (`Input::before`, after an edit in the viewer),
//! each step stays close to it instead, the way dynamic graph drawing keeps
//! the mental map (Misue, Eades, Lai & Sugiyama 1995; North 1996, "Incremental
//! layout in DynaDAG"; Branke 2001, "Dynamic graph drawing"): nodes keep
//! their layers where the edges allow, each layer starts in its previous
//! order and is only reordered wholesale when that removes crossings by a
//! clear margin, and breadth positions are pulled towards the previous ones.

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
    /// Each node's `(breadth, depth)` center in a previous layout, to stay
    /// close to; empty for a layout from scratch.
    pub before: &'a [Option<(f32, f32)>],
}

pub struct Output {
    /// Top-left `(breadth, depth)` of each node.
    pub pos: Vec<(f32, f32)>,
    /// Per edge: points from source to target, in `(breadth, depth)`. The
    /// first and last are on the node outlines; the rest are dummy centers.
    /// Empty for self loops, which the caller draws on the node's far
    /// breadth side, reaching `LOOP_REACH` out of it.
    pub routes: Vec<Vec<(f32, f32)>>,
    /// Center of each edge's label, if it has one.
    pub label_at: Vec<Option<(f32, f32)>>,
}

const DUMMY_WEIGHT: f32 = 8.0;
const MIXED_WEIGHT: f32 = 2.0;
/// How far a self loop bulges out of its node's side (see `self_loop`).
pub(crate) const LOOP_REACH: f32 = 27.0;
const LABEL_GAP: f32 = 4.0;
/// How hard a node is pulled towards its previous breadth position, against
/// an edge's pull of 1 towards its neighbor: all but pinned, so a node that
/// was there moves only when the spacing in its layer pushes it.
const KEEP_WEIGHT: f32 = 1000.0;

pub fn layout(input: &Input) -> Output {
    let n = input.size.len();
    // Labels on self loops sit beside their node and need no doubling.
    let has_labels = input.edges.iter().zip(input.labels).any(|(&(a, b), l)| a != b && l.is_some());

    // 1. Orient edges into a DAG.
    let real: Vec<(usize, usize, usize)> =
        input.edges.iter().enumerate().filter(|(_, (a, b))| a != b).map(|(i, &(a, b))| (i, a, b)).collect();
    let reversed = if input.before.is_empty() { back_edges(n, &real) } else { kept_directions(n, &real, input.before) };
    let dag: Vec<(usize, usize)> = real.iter().map(|&(i, a, b)| if reversed[i] { (b, a) } else { (a, b) }).collect();

    // 2. Layers.
    let breadth: Vec<f32> = input.size.iter().map(|s| s.0).collect();
    let mut layer = network_simplex(&breadth, &dag);
    if let Some(kept) = kept_layers(n, &dag, input.before) {
        layer = kept;
    }
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
    let keys = kept_keys(input.before, n, &real, &chains, &vlayer);
    if keys.is_empty() {
        reduce_crossings(&mut layers, &up, &down, vn);
    } else {
        layers = kept_order(layers, &keys, &up, &down, vn);
    }

    // 5. Coordinates. Self loops bulge out on a node's far breadth side;
    // reserve room there for the loop and its label so neighbors keep clear.
    // Further loops on a node lean toward its corners and need depth too.
    let mut room = vec![0.0f32; vn];
    let mut layer_depth = vec![0.0f32; nlayers];
    let mut loops = vec![0; n];
    for (i, &(a, b)) in input.edges.iter().enumerate() {
        if a == b {
            let (lb, ld) = input.labels[i].map_or((0.0, 0.0), |l| (LABEL_GAP + l.0, l.1));
            room[a] = room[a].max(LOOP_REACH + lb);
            loops[a] += 1;
            let lean = if loops[a] > 1 { input.size[a].1 + 2.0 * (LOOP_REACH + ld) } else { 0.0 };
            layer_depth[vlayer[a]] = layer_depth[vlayer[a]].max(ld).max(lean);
        }
    }
    let is_dummy = |v: usize| v >= n;
    let pull: Vec<Option<f32>> = (0..vn).map(|v| input.before.get(v).copied().flatten().map(|b| b.0)).collect();
    let pull = if input.before.is_empty() { Vec::new() } else { pull };
    let x = assign_breadth(&layers, &vsize, &room, &up, &down, input.node_sep, &is_dummy, &pull);

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
    if !input.before.is_empty() {
        let mut was = vec![None; nlayers];
        for v in (0..n).rev() {
            was[vlayer[v]] = input.before[v].map(|b| b.1).or(was[vlayer[v]]);
        }
        layer_top = kept_tops(&layer_top, &layer_depth, &was);
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
    // Only a node's first loop has its label beside the node; the caller
    // places the others along their loops.
    let mut first = vec![true; n];
    for (i, &(a, b)) in input.edges.iter().enumerate() {
        if a != b || !std::mem::replace(&mut first[a], false) {
            continue;
        }
        if let Some(l) = input.labels[i] {
            let side = x[a] + vsize[a].0 / 2.0 - min_x;
            label_at[i] = Some((side + LOOP_REACH + LABEL_GAP + l.0 / 2.0, center_depth(a)));
        }
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

/// Cycle breaking that keeps the previous drawing's directions: an edge
/// between two nodes that were there points down from the higher one. Each
/// other edge keeps its direction unless that closes a cycle (it is then
/// reversed, not an older edge).
fn kept_directions(n: usize, edges: &[(usize, usize, usize)], before: &[Option<(f32, f32)>]) -> Vec<bool> {
    let max_id = edges.iter().map(|e| e.0).max().map_or(0, |m| m + 1);
    let mut reversed = vec![false; max_id];
    let depth = |v: usize| before.get(v).copied().flatten().map(|b| b.1);
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut free = Vec::new();
    for &(i, a, b) in edges {
        match (depth(a), depth(b)) {
            (Some(da), Some(db)) if (da - db).abs() > 0.5 => {
                reversed[i] = da > db;
                let (a, b) = if da > db { (b, a) } else { (a, b) };
                out[a].push(b);
            }
            _ => free.push((i, a, b)),
        }
    }
    // shortcut: one search per edge with a new end, O(E·(V+E)) when a paste
    // brings thousands at once; switch to an incremental topological order
    // (Pearce & Kelly 2006) if that gets slow.
    let mut seen = vec![usize::MAX; n];
    for (k, (i, a, b)) in free.into_iter().enumerate() {
        // Does b reach a already? Then a → b would close a cycle.
        let mut stack = vec![b];
        seen[b] = k;
        let mut closes = false;
        while let Some(v) = stack.pop() {
            if v == a {
                closes = true;
                break;
            }
            for &w in &out[v] {
                if seen[w] != k {
                    seen[w] = k;
                    stack.push(w);
                }
            }
        }
        reversed[i] = closes;
        let (a, b) = if closes { (b, a) } else { (a, b) };
        out[a].push(b);
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

/// Layers by network simplex (Gansner et al. 1993, as in Graphviz dot):
/// minimizes the total edge span Σ (layer(b) − layer(a)) subject to every
/// span being at least 1, so short branches are not stretched the way
/// longest-path layering stretches them. Starts from the longest-path layers
/// and falls back to them if the pivot cap is hit. `breadth` is each node's
/// extent across layers.
fn network_simplex(breadth: &[f32], dag: &[(usize, usize)]) -> Vec<usize> {
    let n = breadth.len();
    let init = longest_path(n, dag);
    // Parallel edges are one constraint with their weights summed.
    let mut merged: std::collections::BTreeMap<(usize, usize), i64> = Default::default();
    for &e in dag {
        *merged.entry(e).or_default() += 1;
    }
    let edges: Vec<(usize, usize, i64)> = merged.into_iter().map(|((a, b), w)| (a, b, w)).collect();
    let m = edges.len();
    let mut inc: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, &(a, b, _)) in edges.iter().enumerate() {
        inc[a].push(k);
        inc[b].push(k);
    }
    let other = |k: usize, v: usize| if edges[k].0 == v { edges[k].1 } else { edges[k].0 };
    let slack = |rank: &[i64], k: usize| rank[edges[k].1] - rank[edges[k].0] - 1;
    let mut rank: Vec<i64> = init.iter().map(|&r| r as i64).collect();

    // Feasible spanning forest of tight edges, one tree per component: grow
    // along tight edges, then shift the tree to tighten the least-slack edge
    // leaving it.
    let mut tree = vec![false; m];
    let mut root_of = vec![usize::MAX; n];
    for s in 0..n {
        if root_of[s] != usize::MAX {
            continue;
        }
        root_of[s] = s;
        let mut members = vec![s];
        loop {
            let mut i = 0;
            while i < members.len() {
                let v = members[i];
                i += 1;
                for &k in &inc[v] {
                    let w = other(k, v);
                    if root_of[w] == usize::MAX && slack(&rank, k) == 0 {
                        root_of[w] = s;
                        tree[k] = true;
                        members.push(w);
                    }
                }
            }
            let nearest = members
                .iter()
                .flat_map(|&v| inc[v].iter().copied())
                .filter(|&k| (root_of[edges[k].0] == usize::MAX) != (root_of[edges[k].1] == usize::MAX))
                .min_by_key(|&k| slack(&rank, k));
            let Some(k) = nearest else { break };
            let delta = if root_of[edges[k].0] == s { slack(&rank, k) } else { -slack(&rank, k) };
            for &v in &members {
                rank[v] += delta;
            }
        }
    }

    // Pivot: a tree edge with negative cut value leaves, the least-slack edge
    // crossing its cut the other way enters.
    let mut low = vec![0usize; n];
    let mut lim = vec![0usize; n];
    let mut parent: Vec<Option<usize>> = vec![None; n];
    let mut cut = vec![0i64; m];
    let cap = (n * m).clamp(100, 10_000);
    for pivot in 0.. {
        // Postorder numbers: v's subtree is exactly the nodes with lim in low[v]..=lim[v].
        let mut post = Vec::with_capacity(n);
        for r in (0..n).filter(|&v| root_of[v] == v) {
            parent[r] = None;
            let mut stack = vec![(r, 0usize)];
            low[r] = post.len();
            while let Some(&mut (v, ref mut next)) = stack.last_mut() {
                let child = inc[v][*next..].iter().position(|&k| tree[k] && Some(k) != parent[v]);
                if let Some(j) = child {
                    let k = inc[v][*next + j];
                    *next += j + 1;
                    let w = other(k, v);
                    parent[w] = Some(k);
                    low[w] = post.len();
                    stack.push((w, 0));
                } else {
                    lim[v] = post.len();
                    post.push(v);
                    stack.pop();
                }
            }
        }
        // Cut values bottom-up: each from the child's edges and its children's cut values.
        for &v in &post {
            let Some(pe) = parent[v] else { continue };
            let v_is_tail = edges[pe].0 == v;
            let mut c = edges[pe].2;
            for &k in inc[v].iter().filter(|&&k| k != pe) {
                let to_head = (edges[k].0 == v) == v_is_tail;
                c += if to_head { edges[k].2 } else { -edges[k].2 };
                if tree[k] {
                    c += if to_head { -cut[k] } else { cut[k] };
                }
            }
            cut[pe] = c;
        }
        let Some(leave) = (0..m).find(|&k| tree[k] && cut[k] < 0) else { break };
        if pivot >= cap {
            return init;
        }
        let (a, b, _) = edges[leave];
        // The side below the leaving edge in the rooted tree.
        let (sub, flip) = if lim[a] < lim[b] { (a, false) } else { (b, true) };
        let below = |v: usize| low[sub] <= lim[v] && lim[v] <= lim[sub];
        let Some(enter) = (0..m)
            .filter(|&k| {
                let (x, y, _) = edges[k];
                root_of[x] == root_of[a] && below(x) == flip && below(y) != flip
            })
            .min_by_key(|&k| slack(&rank, k))
        else {
            return init;
        };
        tree[leave] = false;
        tree[enter] = true;
        // Re-derive ranks from the roots along the (all tight) tree edges.
        for r in (0..n).filter(|&v| root_of[v] == v) {
            let mut stack = vec![(r, usize::MAX)];
            while let Some((v, from)) = stack.pop() {
                for &k in inc[v].iter().filter(|&&k| tree[k] && k != from) {
                    let w = other(k, v);
                    rank[w] = if edges[k].0 == v { rank[v] + 1 } else { rank[v] - 1 };
                    stack.push((w, k));
                }
            }
        }
    }

    // Normalize each component to start at layer 0.
    let mut min = vec![i64::MAX; n];
    for v in 0..n {
        min[root_of[v]] = min[root_of[v]].min(rank[v]);
    }
    let mut layer: Vec<usize> = (0..n).map(|v| (rank[v] - min[root_of[v]]) as usize).collect();

    // Balance: a node with as much weight in as out may sit anywhere between
    // its neighbors at no cost; move it where the layers end up narrowest.
    // Crowding is measured in breadth, not node count as in dot, so a small
    // node does not move beside a wide one and widen the drawing.
    let nlayers = layer.iter().max().map_or(0, |&l| l + 1);
    let mut crowd = vec![0.0f32; nlayers];
    for v in 0..n {
        crowd[layer[v]] += breadth[v];
    }
    for v in 0..n {
        let (ins, outs): (Vec<usize>, Vec<usize>) = inc[v].iter().partition(|&&k| edges[k].1 == v);
        let weight = |ks: &[usize]| ks.iter().map(|&k| edges[k].2).sum::<i64>();
        if ins.is_empty() || outs.is_empty() || weight(&ins) != weight(&outs) {
            continue;
        }
        let lo = ins.iter().map(|&k| layer[edges[k].0] + 1).max().unwrap();
        let hi = outs.iter().map(|&k| layer[edges[k].1] - 1).min().unwrap();
        let best = (lo..=hi).min_by(|&p, &q| crowd[p].total_cmp(&crowd[q])).unwrap();
        if crowd[best] + breadth[v] < crowd[layer[v]] {
            crowd[layer[v]] -= breadth[v];
            crowd[best] += breadth[v];
            layer[v] = best;
        }
    }
    layer
}

/// Layers that keep each node's previous one (its rank among the previous
/// depths) where the edges allow: new nodes go just below their
/// predecessors (else just above their successors), then every edge that
/// would point up or sideways pushes its target down. Empty layers close.
/// `None` without a previous layout.
fn kept_layers(n: usize, dag: &[(usize, usize)], before: &[Option<(f32, f32)>]) -> Option<Vec<usize>> {
    let was = |v: usize| before.get(v).copied().flatten();
    let mut depths: Vec<f32> = (0..n).filter_map(|v| Some(was(v)?.1)).collect();
    if depths.is_empty() {
        return None;
    }
    depths.sort_by(f32::total_cmp);
    depths.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    let rank = |d: f32| depths.partition_point(|&x| x < d - 0.5) as i64;
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
    let mut layer: Vec<Option<i64>> = (0..n).map(|v| was(v).map(|b| rank(b.1))).collect();
    for &v in &order {
        if layer[v].is_none() {
            let below = inc[v].iter().filter_map(|&u| layer[u]).max().map(|m| m + 1);
            let above = || out[v].iter().filter_map(|&w| layer[w]).min().map_or(0, |m| m - 1);
            layer[v] = Some(below.unwrap_or_else(above));
        }
    }
    let mut layer: Vec<i64> = layer.into_iter().map(|l| l.unwrap_or(0)).collect();
    for &v in &order {
        for &w in &out[v] {
            layer[w] = layer[w].max(layer[v] + 1);
        }
    }
    let mut used = layer.clone();
    used.sort_unstable();
    used.dedup();
    Some(layer.iter().map(|l| used.partition_point(|u| u < l)).collect())
}

/// Layer tops that keep each layer's previous center (`was`) where there is
/// room: a layer moves only when the one before it pushes it, by growing or
/// by gaining a layer between. Layers with no previous center keep their
/// distance from the one before (the first ones, from the first kept one).
fn kept_tops(top: &[f32], depth: &[f32], was: &[Option<f32>]) -> Vec<f32> {
    let center = |l: usize| top[l] + depth[l] / 2.0;
    let Some(first) = was.iter().position(Option::is_some) else { return top.to_vec() };
    let mut c: Vec<f32> = (0..top.len()).map(|l| center(l) - center(first) + was[first].unwrap()).collect();
    for l in first + 1..c.len() {
        let pushed = c[l - 1] + center(l) - center(l - 1);
        c[l] = was[l].map_or(pushed, |w| w.max(pushed));
    }
    c.iter().zip(depth).map(|(c, d)| c - d / 2.0).collect()
}

/// Where each virtual node sat across its layer before: its previous
/// breadth; for a new node, just after the last of its siblings (nodes of
/// its layer sharing a neighbor with it), as a new child is the last one,
/// else its old neighbors' mean; a dummy's place on the line between its
/// edge's ends. `None` for what has no clue; empty without a previous layout.
fn kept_keys(
    before: &[Option<(f32, f32)>],
    n: usize,
    real: &[(usize, usize, usize)],
    chains: &[Vec<usize>],
    vlayer: &[usize],
) -> Vec<Option<f32>> {
    if before.is_empty() {
        return Vec::new();
    }
    let vn = n + chains.iter().map(|c| c.len() - 2).sum::<usize>();
    let mut key: Vec<Option<f32>> = (0..vn).map(|v| before.get(v).copied().flatten().map(|b| b.0)).collect();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(_, a, b) in real {
        adj[a].push(b);
        adj[b].push(a);
    }
    let was = key.clone();
    for v in (0..n).filter(|&v| was[v].is_none()) {
        let old: Vec<f32> = adj[v].iter().filter_map(|&u| was[u]).collect();
        let siblings = adj[v].iter().filter(|&&u| was[u].is_some()).flat_map(|&u| &adj[u]);
        let last = siblings.filter(|&&w| w != v && vlayer[w] == vlayer[v]).filter_map(|&w| was[w]).reduce(f32::max);
        key[v] = last.map(|k| k + 0.5).or((!old.is_empty()).then(|| old.iter().sum::<f32>() / old.len() as f32));
    }
    for chain in chains {
        let (a, b) = (chain[0], *chain.last().unwrap());
        let span = (chain.len() - 1) as f32;
        for (s, &d) in chain.iter().enumerate().take(chain.len() - 1).skip(1) {
            key[d] = match (key[a], key[b]) {
                (Some(ka), Some(kb)) => Some(ka + (kb - ka) * s as f32 / span),
                (k, None) | (None, k) => k,
            };
        }
    }
    key
}

/// Each layer in its previous order (by `keys`; what has none keeps its
/// place after them), then adjacent swaps that remove crossings. A full
/// reordering only wins when it removes at least two crossings and a
/// quarter of them: hysteresis, so an edit does not reshuffle a layer for
/// one crossing less.
fn kept_order(
    mut layers: Vec<Vec<usize>>,
    keys: &[Option<f32>],
    up: &[Vec<usize>],
    down: &[Vec<usize>],
    vn: usize,
) -> Vec<Vec<usize>> {
    for l in &mut layers {
        let at: Vec<(f32, usize)> = l.iter().enumerate().map(|(i, &v)| (keys[v].unwrap_or(f32::INFINITY), i)).collect();
        let mut idx: Vec<usize> = (0..l.len()).collect();
        idx.sort_by(|&p, &q| at[p].0.total_cmp(&at[q].0).then(at[p].1.cmp(&at[q].1)));
        *l = idx.iter().map(|&i| l[i]).collect();
    }
    let mut full = layers.clone();
    transpose(&mut layers, up, down, vn);
    reduce_crossings(&mut full, up, down, vn);
    let (kept, best) = (total_crossings(&layers, down, vn), total_crossings(&full, down, vn));
    if best + 2.max(kept / 4) <= kept { full } else { layers }
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

#[allow(clippy::too_many_arguments)]
fn assign_breadth(
    layers: &[Vec<usize>],
    vsize: &[(f32, f32)],
    room: &[f32],
    up: &[Vec<usize>],
    down: &[Vec<usize>],
    node_sep: f32,
    is_dummy: &dyn Fn(usize) -> bool,
    pull: &[Option<f32>],
) -> Vec<f32> {
    let vn = vsize.len();
    let sep = |a: usize, b: usize| {
        let gap = match (is_dummy(a), is_dummy(b)) {
            (false, false) => node_sep,
            (true, true) => node_sep * 0.25,
            _ => node_sep * 0.5,
        };
        (vsize[a].0 + vsize[b].0) / 2.0 + room[a] + gap
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
        // After an edit: start where the nodes were, spaced as they must be.
        if l.iter().any(|&v| pull.get(v).copied().flatten().is_some()) {
            let gaps: Vec<f32> = l.windows(2).map(|p| sep(p[0], p[1])).collect();
            let mut t: Vec<Option<f32>> = l.iter().map(|&v| pull[v]).collect();
            for i in 1..t.len() {
                t[i] = t[i].or(t[i - 1].map(|p| p + gaps[i - 1]));
            }
            for i in (0..t.len().saturating_sub(1)).rev() {
                t[i] = t[i].or(t[i + 1].map(|p| p - gaps[i]));
            }
            let w: Vec<f32> = l.iter().map(|&v| if pull[v].is_some() { 1.0 } else { 0.05 }).collect();
            let t: Vec<f32> = t.into_iter().map(|t| t.unwrap_or(0.0)).collect();
            for (&v, xv) in l.iter().zip(isotonic_with_gaps(&t, &w, &gaps)) {
                x[v] = xv;
            }
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
            if let Some(p) = pull.get(v).copied().flatten() {
                sw += KEEP_WEIGHT;
                swx += KEEP_WEIGHT * p;
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
        layout(&Input {
            size: &size,
            shape: &shape,
            edges,
            labels: &labels,
            node_sep: 20.0,
            rank_sep: 40.0,
            before: &[],
        })
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

    fn span(dag: &[(usize, usize)], layer: &[usize]) -> usize {
        dag.iter()
            .map(|&(a, b)| {
                assert!(layer[b] > layer[a], "edge {a}->{b} not downward: {layer:?}");
                layer[b] - layer[a]
            })
            .sum()
    }

    #[test]
    fn simplex_never_spans_more_than_longest_path() {
        let ladder: Vec<(usize, usize)> =
            (0..4).flat_map(|i| [(i, i + 1), (i + 5, i + 6), (i, i + 5 + (i % 2))]).collect();
        let graphs: [(usize, &[(usize, usize)]); 3] = [
            (4, &[(0, 1), (0, 2), (1, 3), (2, 3)]),
            (10, &ladder),
            // A long chain and a short branch into its end: longest path
            // leaves the branch hanging at the top.
            (7, &[(0, 1), (1, 2), (2, 3), (3, 4), (5, 6), (6, 4)]),
        ];
        for (n, dag) in graphs {
            assert!(span(dag, &network_simplex(&vec![1.0; n], dag)) <= span(dag, &longest_path(n, dag)));
        }
        let (n, dag) = graphs[2];
        assert_eq!(span(dag, &network_simplex(&vec![1.0; n], dag)), 6);
        assert!(span(dag, &longest_path(n, dag)) > 6);
    }

    #[test]
    fn simplex_is_optimal_on_small_graphs() {
        let mut seed = 7u64;
        let mut rand = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        for _ in 0..30 {
            let n = 3 + rand() % 4;
            let dag: Vec<(usize, usize)> =
                (0..n).flat_map(|a| (a + 1..n).map(move |b| (a, b))).filter(|_| rand() % 3 == 0).collect();
            // Brute force over all layerings with layers below n.
            let mut best = usize::MAX;
            let mut layer = vec![0usize; n];
            for code in 0..n.pow(n as u32) {
                let mut c = code;
                for l in layer.iter_mut() {
                    *l = c % n;
                    c /= n;
                }
                if dag.iter().all(|&(a, b)| layer[b] > layer[a]) {
                    best = best.min(dag.iter().map(|&(a, b)| layer[b] - layer[a]).sum());
                }
            }
            assert_eq!(span(&dag, &network_simplex(&vec![1.0; n], &dag)), best, "{dag:?}");
        }
    }

    #[test]
    fn simplex_handles_parts_and_is_deterministic() {
        // Two components, a parallel edge, an isolated node.
        let dag = [(0, 1), (1, 2), (3, 4), (3, 4), (4, 2)];
        let layer = network_simplex(&[1.0; 6], &dag);
        assert_eq!(layer, network_simplex(&[1.0; 6], &dag));
        assert_eq!(layer[5], 0);
        assert_eq!(span(&dag, &layer), 5);
        let parts = network_simplex(&[1.0; 4], &[(0, 1), (2, 3)]);
        assert_eq!(parts, vec![0, 1, 0, 1]);
        let a = run(6, &[(0, 1), (1, 2), (2, 0), (3, 3), (3, 4), (0, 4), (5, 2)], true);
        let b = run(6, &[(0, 1), (1, 2), (2, 0), (3, 3), (3, 4), (0, 4), (5, 2)], true);
        assert_eq!(a.pos, b.pos);
        no_overlap(&a);
    }

    #[test]
    fn simplex_is_fast_on_large_graphs() {
        let n = 400;
        let mut dag = Vec::new();
        for i in 1..n {
            dag.push(((i - 1) / 3, i));
            if i % 5 == 0 {
                dag.push((i / 4, i));
            }
            if i % 11 == 0 {
                dag.push((i - 7, i));
            }
        }
        let t = std::time::Instant::now();
        let layer = network_simplex(&vec![1.0; n], &dag);
        assert!(t.elapsed().as_secs_f32() < 2.0, "took {:?}", t.elapsed());
        assert!(span(&dag, &layer) <= span(&dag, &longest_path(n, &dag)));
    }
}
