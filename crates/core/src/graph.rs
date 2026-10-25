//! Graph helpers over node indices (`0..n`, usually document order), shared
//! by everything that orders a diagram's parts.

use std::collections::BTreeSet;

/// Orders `0..n` so that every edge `(a, b)` puts `a` before `b`, taking at
/// each step the lowest index whose predecessors are all placed (document
/// order breaks ties). On a cycle, the lowest index left goes next, so every
/// index appears once and the result depends only on the input. Edges count
/// with multiplicity; a self-loop keeps a node waiting like a cycle does.
/// O((n + edges) log n).
pub fn stable_toposort(n: usize, edges: impl IntoIterator<Item = (usize, usize)>) -> Vec<usize> {
    let mut succ = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for (a, b) in edges {
        succ[a].push(b);
        indeg[b] += 1;
    }
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut done = vec![false; n];
    let mut first_left = 0;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let i = match ready.pop_first() {
            Some(i) => i,
            None => {
                while done[first_left] {
                    first_left += 1;
                }
                first_left
            }
        };
        done[i] = true;
        out.push(i);
        for &b in &succ[i] {
            indeg[b] = indeg[b].saturating_sub(1);
            if indeg[b] == 0 && !done[b] {
                ready.insert(b);
            }
        }
    }
    out
}

/// A directed acyclic graph over `0..n` that refuses edges closing a cycle.
/// It keeps a topological order as it grows (Pearce and Kelly's dynamic
/// algorithm), so a check only searches between the two ends' positions
/// instead of the whole graph.
#[derive(Clone, Debug)]
pub struct Dag {
    succ: Vec<Vec<usize>>,
    pred: Vec<Vec<usize>>,
    /// Position of each node in the current topological order.
    pos: Vec<usize>,
}

impl Dag {
    pub fn new(n: usize) -> Self {
        Dag { succ: vec![Vec::new(); n], pred: vec![Vec::new(); n], pos: (0..n).collect() }
    }

    /// Whether `from` reaches `to` along edges (every node reaches itself).
    pub fn reaches(&self, from: usize, to: usize) -> bool {
        if from == to {
            return true;
        }
        if self.pos[from] > self.pos[to] {
            return false;
        }
        let ub = self.pos[to];
        self.collect(from, |d, x| &d.succ[x], |p| p <= ub).contains(&to)
    }

    /// Adds the edge `a → b` unless `b` already reaches `a` (which includes
    /// `a == b`). Returns whether it was added.
    pub fn add(&mut self, a: usize, b: usize) -> bool {
        if self.reaches(b, a) {
            return false;
        }
        if self.pos[a] > self.pos[b] {
            self.reorder(a, b);
        }
        self.succ[a].push(b);
        self.pred[b].push(a);
        true
    }

    /// Moves what `b` reaches (within the window) after what reaches `a`,
    /// so `a` comes before `b`.
    fn reorder(&mut self, a: usize, b: usize) {
        let (lb, ub) = (self.pos[b], self.pos[a]);
        let forward = self.collect(b, |d, x| &d.succ[x], |p| p <= ub);
        let backward = self.collect(a, |d, x| &d.pred[x], |p| p >= lb);
        let mut slots: Vec<usize> = forward.iter().chain(&backward).map(|&x| self.pos[x]).collect();
        slots.sort_unstable();
        let by_pos = |v: Vec<usize>, pos: &[usize]| {
            let mut v = v;
            v.sort_unstable_by_key(|&x| pos[x]);
            v
        };
        let moved: Vec<usize> = by_pos(backward, &self.pos).into_iter().chain(by_pos(forward, &self.pos)).collect();
        for (x, p) in moved.into_iter().zip(slots) {
            self.pos[x] = p;
        }
    }

    /// `start` and every node reachable from it through `next` whose
    /// position is `inside` the window.
    fn collect(
        &self,
        start: usize,
        next: impl Fn(&Self, usize) -> &Vec<usize>,
        inside: impl Fn(usize) -> bool,
    ) -> Vec<usize> {
        let mut seen = std::collections::HashSet::from([start]);
        let mut out = vec![start];
        let mut stack = vec![start];
        while let Some(x) = stack.pop() {
            for &y in next(self, x) {
                if inside(self.pos[y]) && seen.insert(y) {
                    out.push(y);
                    stack.push(y);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator (no dependency on `rand`).
    fn lcg(seed: &mut u64) -> usize {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 33) as usize
    }

    /// The quadratic reference the brief and the flow compiler used.
    fn naive(n: usize, edges: &[(usize, usize)]) -> Vec<usize> {
        let mut indeg = vec![0usize; n];
        for &(_, b) in edges {
            indeg[b] += 1;
        }
        let mut done = vec![false; n];
        let mut out = Vec::new();
        while out.len() < n {
            let i = (0..n)
                .find(|&i| !done[i] && indeg[i] == 0)
                .or_else(|| (0..n).find(|&i| !done[i]))
                .expect("a node is left");
            done[i] = true;
            out.push(i);
            for &(a, b) in edges {
                if a == i {
                    indeg[b] = indeg[b].saturating_sub(1);
                }
            }
        }
        out
    }

    #[test]
    fn document_order_breaks_ties() {
        assert_eq!(stable_toposort(4, []), vec![0, 1, 2, 3]);
        assert_eq!(stable_toposort(4, [(3, 0), (2, 1)]), vec![2, 1, 3, 0]);
        assert_eq!(stable_toposort(3, [(2, 0), (2, 0)]), vec![1, 2, 0]);
    }

    #[test]
    fn cycles_are_broken_at_the_first_node_left() {
        // 0 ⇄ 1, and 2 waits for 1.
        assert_eq!(stable_toposort(3, [(0, 1), (1, 0), (1, 2)]), vec![0, 1, 2]);
        // A self-loop waits until nothing else is ready.
        assert_eq!(stable_toposort(3, [(0, 0), (1, 2)]), vec![1, 2, 0]);
    }

    #[test]
    fn matches_the_reference_and_is_deterministic() {
        let mut seed = 7;
        for _ in 0..300 {
            let n = 1 + lcg(&mut seed) % 12;
            let edges: Vec<(usize, usize)> =
                (0..lcg(&mut seed) % 30).map(|_| (lcg(&mut seed) % n, lcg(&mut seed) % n)).collect();
            let got = stable_toposort(n, edges.iter().copied());
            assert_eq!(got, naive(n, &edges), "{n} {edges:?}");
            assert_eq!(got, stable_toposort(n, edges.iter().copied()));
            let mut sorted = got.clone();
            sorted.sort();
            assert_eq!(sorted, (0..n).collect::<Vec<_>>());
        }
    }

    #[test]
    fn dag_refuses_exactly_the_cycles() {
        let mut seed = 11;
        for _ in 0..300 {
            let n = 1 + lcg(&mut seed) % 10;
            let mut dag = Dag::new(n);
            let mut edges: Vec<(usize, usize)> = Vec::new();
            for _ in 0..lcg(&mut seed) % 40 {
                let (a, b) = (lcg(&mut seed) % n, lcg(&mut seed) % n);
                // Reference: does b reach a through the edges so far?
                let mut stack = vec![b];
                let mut seen = vec![false; n];
                let mut cycle = false;
                while let Some(x) = stack.pop() {
                    if x == a {
                        cycle = true;
                        break;
                    }
                    if !std::mem::replace(&mut seen[x], true) {
                        stack.extend(edges.iter().filter(|e| e.0 == x).map(|e| e.1));
                    }
                }
                assert_eq!(dag.add(a, b), !cycle, "{a}→{b} after {edges:?}");
                if !cycle {
                    edges.push((a, b));
                }
                // The kept order is topological.
                assert!(edges.iter().all(|&(x, y)| dag.pos[x] < dag.pos[y]));
            }
        }
    }
}
