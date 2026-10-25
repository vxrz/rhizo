//! Non-layered tidy trees.
//!
//! Each node sits at its parent's far edge plus a gap (so depths vary with node
//! size, unlike layered tidy trees). Subtrees are packed using their contours,
//! the outline of the space they occupy at every depth. That lets a small
//! subtree tuck in next to the narrow part of a big neighbor instead of being
//! stacked as a bounding box. Runs in O(n · contour size) without recursion,
//! so very deep trees do not overflow the stack.
//!
//! Coordinates are abstract: "depth" grows away from the root, "breadth" runs
//! across siblings. The caller maps them onto x/y for a direction.

#[derive(Clone, Copy, Debug, PartialEq)]
struct Seg {
    d0: f32,
    d1: f32,
    lo: f32,
    hi: f32,
}

type Contour = Vec<Seg>;

fn shift(c: &mut Contour, dd: f32, db: f32) {
    for s in c {
        s.d0 += dd;
        s.d1 += dd;
        s.lo += db;
        s.hi += db;
    }
}

/// The smallest breadth shift for `lower` that keeps it `gap` below `upper`
/// wherever their depth ranges overlap.
fn separation(upper: &Contour, lower: &Contour, gap: f32) -> f32 {
    let (mut i, mut j) = (0, 0);
    let mut need = f32::NEG_INFINITY;
    while i < upper.len() && j < lower.len() {
        let (u, l) = (upper[i], lower[j]);
        if u.d0 < l.d1 && l.d0 < u.d1 {
            need = need.max(u.hi - l.lo + gap);
        }
        if u.d1 < l.d1 { i += 1 } else { j += 1 }
    }
    if need == f32::NEG_INFINITY { 0.0 } else { need }
}

fn union(a: &Contour, b: &Contour) -> Contour {
    let mut cuts: Vec<f32> = a.iter().chain(b).flat_map(|s| [s.d0, s.d1]).collect();
    cuts.sort_by(f32::total_cmp);
    cuts.dedup_by(|x, y| (*x - *y).abs() < 1e-4);
    let mut out: Contour = Vec::with_capacity(cuts.len());
    let (mut i, mut j) = (0, 0);
    for w in cuts.windows(2) {
        let (x0, x1) = (w[0], w[1]);
        let mid = (x0 + x1) / 2.0;
        while i < a.len() && a[i].d1 <= mid {
            i += 1;
        }
        while j < b.len() && b[j].d1 <= mid {
            j += 1;
        }
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for s in [a.get(i), b.get(j)].into_iter().flatten() {
            if s.d0 <= mid && mid < s.d1 {
                lo = lo.min(s.lo);
                hi = hi.max(s.hi);
            }
        }
        if lo > hi {
            continue;
        }
        match out.last_mut() {
            Some(l) if (l.d1 - x0).abs() < 1e-4 && l.lo == lo && l.hi == hi => l.d1 = x1,
            _ => out.push(Seg { d0: x0, d1: x1, lo, hi }),
        }
    }
    out
}

pub struct Spacing {
    /// Between a parent's far edge and its children.
    pub level: f32,
    /// Between neighboring siblings.
    pub sibling: f32,
}

/// Lays out a forest. `size[i]` is `(depth extent, breadth extent)`.
/// Returns each node's top-left as `(depth, breadth)`, with the forest's
/// minimum breadth at 0 and the roots at depth 0. Roots are packed like
/// siblings.
pub fn layout(size: &[(f32, f32)], children: &[Vec<usize>], roots: &[usize], sp: &Spacing) -> Vec<(f32, f32)> {
    layout_kept(size, children, roots, sp, &[])
}

/// [`layout`], staying close to a previous layout: `before[i]` is node
/// `i`'s breadth center in it. Siblings keep their distance from the first
/// of them that was there before, unless they must move apart to clear each
/// other (gaps stay; only what would overlap is pushed). A parent keeps its
/// offset from that first child (clamped to its children's span) instead of
/// centering on them. An edit in one subtree so moves the siblings after it,
/// and only as far as they must; nothing above it recenters.
pub fn layout_kept(
    size: &[(f32, f32)],
    children: &[Vec<usize>],
    roots: &[usize],
    sp: &Spacing,
    before: &[Option<f32>],
) -> Vec<(f32, f32)> {
    let n = size.len();
    // Per node: breadth of the node's top within its own subtree frame, and
    // its contour in that frame.
    let mut node_b = vec![0.0f32; n];
    let mut contour: Vec<Option<Contour>> = vec![None; n];
    // Offset of each child's subtree frame within its parent's frame.
    let mut rel = vec![(0.0f32, 0.0f32); n];

    let order = post_order(children, roots);
    for &v in &order {
        let (dv, bv) = size[v];
        let kids = &children[v];
        if kids.is_empty() {
            node_b[v] = 0.0;
            contour[v] = Some(vec![Seg { d0: 0.0, d1: dv, lo: 0.0, hi: bv }]);
            continue;
        }
        let off = dv + sp.level;
        let want = |k: usize, offsets: &[f32]| kept_offset(kids, k, offsets, &node_b, size, before);
        let (merged, offsets) = pack(kids, &mut contour, sp.sibling, off, &want);
        let (first, last) = (kids[0], *kids.last().unwrap());
        let first_c = offsets[0] + node_b[first] + size[first].1 / 2.0;
        let last_c = offsets[offsets.len() - 1] + node_b[last] + size[last].1 / 2.0;
        let kept = before.get(v).copied().flatten().and_then(|was| {
            let k = kids.iter().position(|&c| before.get(c).copied().flatten().is_some())?;
            let c = kids[k];
            let child = offsets[k] + node_b[c] + size[c].1 / 2.0;
            Some((child + was - before[c]?).clamp(first_c.min(last_c), first_c.max(last_c)))
        });
        let top = match kept {
            Some(center) => center - bv / 2.0,
            None => (first_c + last_c) / 2.0 - bv / 2.0,
        };
        let min = merged.iter().map(|s| s.lo).fold(top, f32::min);
        node_b[v] = top - min;
        for (k, &c) in kids.iter().enumerate() {
            rel[c] = (off, offsets[k] - min);
        }
        let mut merged = merged;
        shift(&mut merged, 0.0, -min);
        let own = vec![Seg { d0: 0.0, d1: dv, lo: node_b[v], hi: node_b[v] + bv }];
        contour[v] = Some(union(&own, &merged));
    }

    // Pack the roots as siblings of a virtual parent at depth 0.
    let want = |k: usize, offsets: &[f32]| kept_offset(roots, k, offsets, &node_b, size, before);
    let (_, root_offsets) = pack(roots, &mut contour, sp.sibling, 0.0, &want);
    let mut frame = vec![(0.0f32, 0.0f32); n];
    let mut pos = vec![(0.0f32, 0.0f32); n];
    for (k, &r) in roots.iter().enumerate() {
        frame[r] = (0.0, root_offsets[k]);
    }
    for &v in order.iter().rev() {
        pos[v] = (frame[v].0, frame[v].1 + node_b[v]);
        for &c in &children[v] {
            frame[c] = (frame[v].0 + rel[c].0, frame[v].1 + rel[c].1);
        }
    }
    let min_b = pos.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    if min_b.is_finite() {
        for p in &mut pos {
            p.1 -= min_b;
        }
    }
    pos
}

/// Where subtree `k` of `kids` goes to keep its node's previous distance
/// from the first sibling before it that was there before; `None` when
/// either is new (or there is no previous layout).
fn kept_offset(
    kids: &[usize],
    k: usize,
    offsets: &[f32],
    node_b: &[f32],
    size: &[(f32, f32)],
    before: &[Option<f32>],
) -> Option<f32> {
    let was = |c: usize| before.get(c).copied().flatten();
    let (c, bc) = (kids[k], was(kids[k])?);
    let j = (0..k).find(|&j| was(kids[j]).is_some())?;
    let r = kids[j];
    let center_r = offsets[j] + node_b[r] + size[r].1 / 2.0;
    Some(center_r + bc - was(r)? - node_b[c] - size[c].1 / 2.0)
}

/// Stacks subtrees along the breadth axis as tightly as their contours allow,
/// or further where `want` asks for it. Consumes their contours; returns the
/// merged contour (in the parent's frame, depth-shifted by `off`) and each
/// subtree's breadth offset.
fn pack(
    kids: &[usize],
    contour: &mut [Option<Contour>],
    gap: f32,
    off: f32,
    want: &dyn Fn(usize, &[f32]) -> Option<f32>,
) -> (Contour, Vec<f32>) {
    let mut merged: Contour = Vec::new();
    let mut offsets = Vec::with_capacity(kids.len());
    for (k, &c) in kids.iter().enumerate() {
        let mut cc = contour[c].take().unwrap_or_default();
        shift(&mut cc, off, 0.0);
        let s = if k == 0 {
            0.0
        } else {
            let s = separation(&merged, &cc, gap).max(*offsets.last().unwrap());
            want(k, &offsets).map_or(s, |w| s.max(w))
        };
        shift(&mut cc, 0.0, s);
        merged = if k == 0 { cc } else { union(&merged, &cc) };
        offsets.push(s);
    }
    (merged, offsets)
}

fn post_order(children: &[Vec<usize>], roots: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(children.len());
    let mut stack: Vec<(usize, bool)> = roots.iter().rev().map(|&r| (r, false)).collect();
    while let Some((v, expanded)) = stack.pop() {
        if expanded {
            out.push(v);
        } else {
            stack.push((v, true));
            for &c in children[v].iter().rev() {
                stack.push((c, false));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(pos: &[(f32, f32)], size: &[(f32, f32)]) -> bool {
        for i in 0..pos.len() {
            for j in i + 1..pos.len() {
                let (a, b) = (pos[i], pos[j]);
                let (sa, sb) = (size[i], size[j]);
                if a.0 < b.0 + sb.0 && b.0 < a.0 + sa.0 && a.1 < b.1 + sb.1 && b.1 < a.1 + sa.1 {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn simple_tree() {
        // 0 -> 1, 2 ; 1 -> 3
        let size = vec![(40.0, 20.0); 4];
        let children = vec![vec![1, 2], vec![3], vec![], vec![]];
        let pos = layout(&size, &children, &[0], &Spacing { level: 10.0, sibling: 5.0 });
        assert_eq!(pos[1].0, 50.0);
        assert_eq!(pos[3].0, 100.0);
        assert_eq!(pos[2].1, 25.0);
        // Parent centered between its children.
        assert_eq!(pos[0].1, 12.5);
        assert!(!overlaps(&pos, &size));
    }

    #[test]
    fn contours_tuck_small_subtrees() {
        // Child 1 is short but has a tall deep subtree; child 2 is a leaf.
        // The leaf should sit right under child 1, not under its subtree.
        let mut size = vec![(40.0, 20.0); 3];
        let mut children = vec![vec![1, 2], vec![], vec![]];
        for _ in 0..6 {
            size.push((40.0, 20.0));
            let id = size.len() - 1;
            children.push(vec![]);
            children[1].push(id);
        }
        let pos = layout(&size, &children, &[0], &Spacing { level: 10.0, sibling: 5.0 });
        assert!(!overlaps(&pos, &size));
        // Child 1 is centered on its 6 kids (breadth 0..145); the leaf only
        // has to clear child 1 itself at its depth.
        assert!(pos[2].1 < pos[8].1, "leaf {:?} should be above the last grandchild {:?}", pos[2], pos[8]);
    }

    #[test]
    fn forest_and_deep_chain() {
        let n = 5000;
        let size = vec![(10.0, 10.0); n];
        let mut children: Vec<Vec<usize>> = (0..n).map(|i| if i + 1 < n { vec![i + 1] } else { vec![] }).collect();
        children[n - 1] = vec![];
        let pos = layout(&size, &children, &[0], &Spacing { level: 5.0, sibling: 5.0 });
        assert_eq!(pos[n - 1].0, (n - 1) as f32 * 15.0);

        let size = vec![(10.0, 10.0); 3];
        let pos = layout(&size, &[vec![], vec![], vec![]], &[0, 1, 2], &Spacing { level: 5.0, sibling: 5.0 });
        assert_eq!(pos.iter().map(|p| p.1).collect::<Vec<_>>(), vec![0.0, 15.0, 30.0]);
    }
}
