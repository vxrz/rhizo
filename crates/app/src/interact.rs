//! What pointer gestures mean, as pure functions of the laid-out diagram and
//! the document (decision D19):
//!
//! - free frames: dragging moves the node and pins its `x`/`y`;
//! - mindmaps and trees: dropping onto the middle of a node makes the dragged
//!   node its child, dropping onto its leading or trailing quarter makes it a
//!   sibling before or after;
//! - layered and grid frames: the layout owns positions, nodes only select.
//!
//! Coordinates here are diagram coordinates (what layout produced).

use rhizo_core::{Direction, Document, FrameId, LayoutKind, NodeId, Point};
use rhizo_lang::Placement;
use rhizo_layout::{Diagram, EdgeBox, EdgeKind, FrameBox, NodeBox, Path, Seg};

use crate::palette::Arrange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragMode {
    /// Free frame: the node follows the pointer.
    Move,
    /// Mindmap or tree: the node is dropped into the outline.
    Outline,
    /// The layout owns the position.
    Fixed,
}

/// The topmost node under `p`.
pub fn hit(d: &Diagram, p: Point) -> Option<&NodeBox> {
    d.nodes.iter().rev().find(|n| n.rect.contains(p))
}

/// The innermost drawn frame under `p`.
pub fn hit_frame(d: &Diagram, p: Point) -> Option<&FrameBox> {
    d.frames.iter().filter(|f| f.visible && f.rect.contains(p)).max_by_key(|f| f.depth)
}

/// True when `p` is on the strip of `f` that holds its label: hovering it
/// shows the frame can be clicked, double-clicking it renames.
pub fn on_frame_title(f: &FrameBox, p: Point) -> bool {
    f.rect.contains(p) && p.y <= f.rect.y + 28.0 + f.label_size + 14.0
}

/// Where a dragged node goes between frames when dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameDrop {
    /// Into this frame, as one of its top nodes.
    Into(FrameId),
    /// Out of its frame, to the top level.
    Out,
}

/// What dropping `dragged` at `p` does between frames: into the frame
/// under it when that is another one, out to the top level when no frame
/// is under it. Not over a node (that is a connection or a re-parent).
pub fn frame_drop(d: &Diagram, doc: &Document, dragged: &NodeId, p: Point) -> Option<FrameDrop> {
    if hit(d, p).is_some() {
        return None;
    }
    let node = doc.node(dragged)?;
    match hit_frame(d, p) {
        Some(f) if f.id != node.frame => Some(FrameDrop::Into(f.id.clone())),
        Some(_) => None,
        None => doc.frame(&node.frame).is_some_and(|f| !f.implicit).then_some(FrameDrop::Out),
    }
}

/// The explicit edge under `p`, as an index into `doc.edges`: the closest
/// link (or the one whose label `p` is on) within `tolerance` diagram units.
/// Nodes win, and nesting edges never hit: they change by moving nodes.
pub fn hit_edge(d: &Diagram, doc: &Document, p: Point, tolerance: f32) -> Option<usize> {
    if hit(d, p).is_some() {
        return None;
    }
    rhizo_layout::edge_sources(doc, d)
        .into_iter()
        .zip(&d.edges)
        .filter_map(|(k, e)| {
            let on_label = e.label.as_ref().is_some_and(|l| l.rect().contains(p));
            Some((if on_label { 0.0 } else { path_distance(&e.path, p) }, k?))
        })
        .filter(|&(dist, _)| dist <= tolerance)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, k)| k)
}

/// The child whose nesting connector (branch) is under `p`, within
/// `tolerance`. Branches are not edges: clicking one selects the child it
/// leads to. Nodes win.
pub fn hit_branch(d: &Diagram, p: Point, tolerance: f32) -> Option<&NodeId> {
    if hit(d, p).is_some() {
        return None;
    }
    d.edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Branch)
        .map(|e| (path_distance(&e.path, p), &e.to))
        .filter(|&(dist, _)| dist <= tolerance)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// Where `doc.edges[k]` is drawn.
pub fn drawn_edge<'a>(d: &'a Diagram, doc: &Document, k: usize) -> Option<&'a EdgeBox> {
    let i = rhizo_layout::edge_sources(doc, d).iter().position(|s| *s == Some(k))?;
    d.edges.get(i)
}

/// Distance from `p` to `path`, with cubics flattened into short lines.
pub fn path_distance(path: &Path, p: Point) -> f32 {
    let mut prev = path.start;
    let mut best = prev.distance(p);
    for (i, seg) in path.segs.iter().enumerate() {
        let steps = if matches!(seg, Seg::Line(_)) { 1 } else { 16 };
        for s in 1..=steps {
            let q = path.eval(i, s as f32 / steps as f32);
            let (dx, dy) = (q.x - prev.x, q.y - prev.y);
            let len2 = dx * dx + dy * dy;
            let t = if len2 > 0.0 { (((p.x - prev.x) * dx + (p.y - prev.y) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            best = best.min(p.distance(Point::new(prev.x + t * dx, prev.y + t * dy)));
            prev = q;
        }
    }
    best
}

fn frame_of<'a>(d: &'a Diagram, n: &NodeBox) -> Option<&'a FrameBox> {
    d.frames.iter().find(|f| f.id == n.frame)
}

pub fn drag_mode(d: &Diagram, n: &NodeBox) -> DragMode {
    match frame_of(d, n).map(|f| f.layout) {
        Some(LayoutKind::Free) => DragMode::Move,
        Some(LayoutKind::Mindmap | LayoutKind::Tree) => DragMode::Outline,
        _ => DragMode::Fixed,
    }
}

/// True when siblings in this frame stack vertically (so before/after is
/// decided by y), false when they sit side by side.
pub fn siblings_stack_vertically(doc: &Document, n: &NodeBox, layout: LayoutKind) -> bool {
    let default = if layout == LayoutKind::Tree { Direction::Down } else { Direction::Right };
    doc.frame(&n.frame).and_then(|f| f.direction).unwrap_or(default).is_horizontal()
}

/// Where dropping `dragged` at `p` would put it in the outline, if anywhere.
pub fn drop_target(d: &Diagram, doc: &Document, dragged: &NodeId, p: Point) -> Option<Placement> {
    let target = hit(d, p)?;
    if &target.id == dragged || doc.ancestors(&target.id).any(|a| a == dragged) {
        return None;
    }
    let layout = frame_of(d, target)?.layout;
    if !matches!(layout, LayoutKind::Mindmap | LayoutKind::Tree) {
        return None;
    }
    let r = target.rect;
    let t = if siblings_stack_vertically(doc, target, layout) { (p.y - r.y) / r.height } else { (p.x - r.x) / r.width };
    // Roots have no siblings in the outline; any drop on them nests.
    let is_root = doc.node(&target.id).is_some_and(|n| n.parent.is_none());
    Some(match t {
        _ if is_root => Placement::Child(target.id.clone()),
        t if t < 0.25 => Placement::Before(target.id.clone()),
        t if t > 0.75 => Placement::After(target.id.clone()),
        _ => Placement::Child(target.id.clone()),
    })
}

/// The `x`/`y` to write for a node in a free frame dragged by `delta`.
///
/// Free layouts draw a pinned node at `at + offset`, where the offset comes
/// from frame padding and the content's top-left. Pinned siblings reveal the
/// offset exactly; without any, the content starts at 0,0 and the offset is
/// the frame's inner origin.
pub fn free_position(d: &Diagram, doc: &Document, id: &NodeId, delta: Point) -> Option<Point> {
    let nb = d.nodes.iter().find(|n| &n.id == id)?;
    let target = Point::new(nb.rect.x + delta.x, nb.rect.y + delta.y);
    let pinned = d
        .nodes
        .iter()
        .filter(|n| n.frame == nb.frame)
        .find_map(|n| doc.node(&n.id).and_then(|m| m.at).map(|at| Point::new(n.rect.x - at.x, n.rect.y - at.y)));
    let offset = match pinned {
        Some(o) => o,
        None => {
            let f = frame_of(d, nb)?;
            let (pad, title) =
                if f.visible { (28.0, if f.label.is_some() { f.label_size + 14.0 } else { 0.0 }) } else { (0.0, 0.0) };
            Point::new(f.rect.x + pad, f.rect.y + pad + title)
        }
    };
    Some(Point::new((target.x - offset.x).round(), (target.y - offset.y).round()))
}

/// The nodes whose boxes meet `r` (diagram coordinates), as a selection
/// box picks them.
pub fn in_rect(d: &Diagram, r: rhizo_core::Rect) -> Vec<NodeId> {
    let meets = |n: &NodeBox| {
        n.rect.x <= r.right() && r.x <= n.rect.right() && n.rect.y <= r.bottom() && r.y <= n.rect.bottom()
    };
    d.nodes.iter().filter(|n| meets(n)).map(|n| n.id.clone()).collect()
}

/// The `x`/`y` to write for each of `ids` in a free frame, moved together
/// by `delta`. Nodes in other frames stay. All are worked out from the
/// diagram as drawn, before any of them is written.
pub fn moved(d: &Diagram, doc: &Document, ids: &[NodeId], delta: Point) -> Vec<(NodeId, Point)> {
    let free = |id: &&NodeId| d.nodes.iter().any(|n| &n.id == *id && drag_mode(d, n) == DragMode::Move);
    ids.iter().filter(free).filter_map(|id| Some((id.clone(), free_position(d, doc, id, delta)?))).collect()
}

/// The `x`/`y` to write to line up (or spread out) the free-frame nodes
/// among `ids`, as [`Arrange`] says. Fewer than two such nodes: nothing.
pub fn arrange(d: &Diagram, doc: &Document, ids: &[NodeId], how: Arrange) -> Vec<(NodeId, Point)> {
    let mut boxes: Vec<&NodeBox> =
        d.nodes.iter().filter(|n| ids.contains(&n.id) && drag_mode(d, n) == DragMode::Move).collect();
    if boxes.len() < 2 {
        return Vec::new();
    }
    let lo = |f: fn(&NodeBox) -> f32| boxes.iter().map(|n| f(n)).fold(f32::INFINITY, f32::min);
    let hi = |f: fn(&NodeBox) -> f32| boxes.iter().map(|n| f(n)).fold(f32::NEG_INFINITY, f32::max);
    let (left, right, top, bottom) =
        (lo(|n| n.rect.x), hi(|n| n.rect.right()), lo(|n| n.rect.y), hi(|n| n.rect.bottom()));
    let x = |n: &NodeBox, to: f32| Point::new(to - n.rect.x, 0.0);
    let y = |n: &NodeBox, to: f32| Point::new(0.0, to - n.rect.y);
    let mut deltas: Vec<(NodeId, Point)> = Vec::new();
    let spread = |boxes: &mut Vec<&NodeBox>, along: fn(&NodeBox) -> (f32, f32), room: f32| {
        boxes.sort_by(|a, b| along(a).0.total_cmp(&along(b).0));
        let used: f32 = boxes.iter().map(|n| along(n).1).sum();
        let gap = (room - used) / (boxes.len() - 1) as f32;
        let mut at = along(boxes[0]).0;
        boxes
            .iter()
            .map(|n| {
                let d = at - along(n).0;
                at += along(n).1 + gap;
                (n.id.clone(), d)
            })
            .collect::<Vec<_>>()
    };
    match how {
        Arrange::SpreadX => {
            let moves = spread(&mut boxes, |n| (n.rect.x, n.rect.width), right - left);
            deltas.extend(moves.into_iter().map(|(id, dx)| (id, Point::new(dx, 0.0))));
        }
        Arrange::SpreadY => {
            let moves = spread(&mut boxes, |n| (n.rect.y, n.rect.height), bottom - top);
            deltas.extend(moves.into_iter().map(|(id, dy)| (id, Point::new(0.0, dy))));
        }
        _ => {
            for n in &boxes {
                let r = n.rect;
                let delta = match how {
                    Arrange::Left => x(n, left),
                    Arrange::Center => x(n, (left + right - r.width) / 2.0),
                    Arrange::Right => x(n, right - r.width),
                    Arrange::Top => y(n, top),
                    Arrange::Middle => y(n, (top + bottom - r.height) / 2.0),
                    _ => y(n, bottom - r.height),
                };
                deltas.push((n.id.clone(), delta));
            }
        }
    }
    deltas.into_iter().filter_map(|(id, delta)| free_position(d, doc, &id, delta).map(|p| (id, p))).collect()
}

/// `ids` without those whose parent (or its parent…) is among them: the
/// statements that hold the rest.
pub fn tops(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let under = |id: &NodeId| {
        let mut p = doc.node(id).and_then(|n| n.parent.clone());
        while let Some(q) = p {
            if ids.contains(&q) {
                return true;
            }
            p = doc.node(&q).and_then(|n| n.parent.clone());
        }
        false
    };
    ids.iter().filter(|id| doc.node(id).is_some() && !under(id)).cloned().collect()
}

/// The selected nodes as rhizo text, to copy: each top statement as
/// written in `src` (with its children), then the edges between them that
/// those statements do not hold already.
pub fn copy_text(src: &str, doc: &Document, ids: &[NodeId]) -> String {
    let (_, spans) = crate::textpane::analyze(src);
    let mut tops: Vec<(NodeId, std::ops::Range<usize>)> = tops(doc, ids)
        .into_iter()
        .filter_map(|id| {
            let r = spans.nodes.iter().find(|(n, _)| *n == id)?.1.clone();
            Some((id, r))
        })
        .collect();
    tops.sort_by_key(|(_, r)| r.start);
    let mut covered: Vec<NodeId> = tops.iter().map(|(id, _)| id.clone()).collect();
    let mut i = 0;
    while i < covered.len() {
        covered.extend(doc.node(&covered[i]).map(|n| n.children.clone()).unwrap_or_default());
        i += 1;
    }
    let mut out: Vec<String> = tops.iter().map(|(_, r)| src[r.clone()].trim().to_string()).collect();
    for (k, e) in doc.edges.iter().enumerate() {
        let held = spans
            .edges
            .get(k)
            .cloned()
            .flatten()
            .is_some_and(|er| tops.iter().any(|(_, r)| r.start <= er.start && er.end <= r.end));
        if held || !covered.contains(&e.from) || !covered.contains(&e.to) {
            continue;
        }
        let label = e.label.as_ref().map(|l| format!(" label={l:?}")).unwrap_or_default();
        out.push(format!("edge {:?} {:?}{label}", e.from.as_str(), e.to.as_str()));
    }
    out.join("\n") + "\n"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The node an arrow key moves the selection to: the closest one in that
/// direction, preferring nodes in line with the current one. Stays in the
/// current frame unless nothing there lies in that direction.
pub fn nearest(d: &Diagram, from: &NodeId, dir: Dir) -> Option<NodeId> {
    let start = d.nodes.iter().find(|n| &n.id == from)?;
    best(d, start, dir, true).or_else(|| best(d, start, dir, false))
}

fn best(d: &Diagram, start: &NodeBox, dir: Dir, same_frame: bool) -> Option<NodeId> {
    let c0 = start.rect.center();
    d.nodes
        .iter()
        .filter(|n| n.id != start.id && (!same_frame || n.frame == start.frame))
        .filter_map(|n| {
            let c = n.rect.center();
            let (dx, dy) = (c.x - c0.x, c.y - c0.y);
            let (along, across) = match dir {
                Dir::Left => (-dx, dy.abs()),
                Dir::Right => (dx, dy.abs()),
                Dir::Up => (-dy, dx.abs()),
                Dir::Down => (dy, dx.abs()),
            };
            (along > 1.0).then_some((along + 2.0 * across, &n.id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_core::Theme;
    use rhizo_lang::{Syntax, load};

    const SRC: &str = r#"
- "Root" {
    - "A" { - "A1" }
    - "B"
}
frame "Board" layout=free {
    node "Pinned" x=100 y=50
    node "Loose"
}
frame "Flow" layout=layered {
    edge "One" "Two"
}
"#;

    fn setup() -> (Document, Diagram) {
        let doc = load(SRC, Syntax::Kdl).into_result().unwrap().0;
        let d = rhizo_layout::layout(&doc, &Theme::light(), &rhizo_layout::Heuristic);
        (doc, d)
    }

    fn node<'a>(d: &'a Diagram, id: &str) -> &'a NodeBox {
        d.nodes.iter().find(|n| n.id.as_str() == id).unwrap()
    }

    fn at(n: &NodeBox, fx: f32, fy: f32) -> Point {
        Point::new(n.rect.x + n.rect.width * fx, n.rect.y + n.rect.height * fy)
    }

    #[test]
    fn modes_follow_the_frame_layout() {
        let (_, d) = setup();
        assert_eq!(drag_mode(&d, node(&d, "a")), DragMode::Outline);
        assert_eq!(drag_mode(&d, node(&d, "pinned")), DragMode::Move);
        assert_eq!(drag_mode(&d, node(&d, "one")), DragMode::Fixed);
        assert_eq!(hit(&d, at(node(&d, "b"), 0.5, 0.5)).map(|n| n.id.as_str()), Some("b"));
        assert!(hit(&d, Point::new(-10_000.0, -10_000.0)).is_none());
    }

    #[test]
    fn distance_to_lines_and_curves() {
        let line = Path::line(Point::new(0.0, 0.0), Point::new(10.0, 0.0));
        assert_eq!(path_distance(&line, Point::new(5.0, 3.0)), 3.0);
        assert_eq!(path_distance(&line, Point::new(14.0, 3.0)), 5.0);
        // A symmetric S-curve passes through its middle.
        let o = Point::new(0.0, 0.0);
        let curve = Path {
            start: o,
            segs: vec![Seg::Cubic(Point::new(50.0, 0.0), Point::new(50.0, 100.0), Point::new(100.0, 100.0))],
        };
        assert!(path_distance(&curve, Point::new(50.0, 50.0)) < 0.5);
        assert!(path_distance(&curve, Point::new(50.0, 10.0)) > 10.0);
    }

    #[test]
    fn edges_hit_near_their_path() {
        let (doc, d) = setup();
        let link = d.edges.iter().find(|e| e.from.as_str() == "one").unwrap();
        let k = doc.edges.iter().position(|e| e.from.as_str() == "one").unwrap();
        let mid = link.path.midpoint();
        assert_eq!(hit_edge(&d, &doc, mid, 4.0), Some(k));
        assert_eq!(drawn_edge(&d, &doc, k).map(|e| e.path.midpoint()), Some(mid));
        assert_eq!(hit_edge(&d, &doc, Point::new(mid.x + 3.0, mid.y + 3.0), 6.0), Some(k));
        assert_eq!(hit_edge(&d, &doc, Point::new(mid.x + 30.0, mid.y + 30.0), 6.0), None);
        // Nodes win over edges, and nesting edges are not selectable.
        assert_eq!(hit_edge(&d, &doc, at(node(&d, "one"), 0.5, 0.5), 1000.0), None);
        let branch = d.edges.iter().find(|e| e.from.as_str() == "root").unwrap();
        assert_eq!(hit_edge(&d, &doc, branch.path.midpoint(), 4.0), None);
    }

    #[test]
    fn branches_select_their_child() {
        let (_, d) = setup();
        let branch = d.edges.iter().find(|e| e.kind == EdgeKind::Branch && e.to.as_str() == "a1").unwrap();
        assert_eq!(hit_branch(&d, branch.path.midpoint(), 4.0).map(|n| n.as_str()), Some("a1"));
        assert_eq!(hit_branch(&d, Point::new(-10_000.0, -10_000.0), 4.0), None);
        // Nodes win.
        assert_eq!(hit_branch(&d, at(node(&d, "a1"), 0.5, 0.5), 1000.0), None);
    }

    #[test]
    fn outline_drops() {
        let (doc, d) = setup();
        let b = node(&d, "b");
        let id = NodeId::from("a1");
        assert_eq!(drop_target(&d, &doc, &id, at(b, 0.5, 0.5)), Some(Placement::Child("b".into())));
        assert_eq!(drop_target(&d, &doc, &id, at(b, 0.5, 0.1)), Some(Placement::Before("b".into())));
        assert_eq!(drop_target(&d, &doc, &id, at(b, 0.5, 0.9)), Some(Placement::After("b".into())));
        // Never into itself or its own subtree, never into a layered frame.
        let a = NodeId::from("a");
        assert_eq!(drop_target(&d, &doc, &a, at(node(&d, "a1"), 0.5, 0.5)), None);
        assert_eq!(drop_target(&d, &doc, &a, at(node(&d, "one"), 0.5, 0.5)), None);
        // Roots only take children.
        assert_eq!(drop_target(&d, &doc, &id, at(node(&d, "root"), 0.5, 0.05)), Some(Placement::Child("root".into())));
    }

    #[test]
    fn free_positions_round_trip_through_layout() {
        let (doc, d) = setup();
        // Pinned node: the new `at` is the old one plus the delta.
        let p = free_position(&d, &doc, &"pinned".into(), Point::new(30.0, -20.0)).unwrap();
        assert_eq!((p.x, p.y), (130.0, 30.0));
        // A loose node gets pinned where it is drawn (plus the delta).
        let loose = node(&d, "loose").rect;
        let q = free_position(&d, &doc, &"loose".into(), Point::new(0.0, 0.0)).unwrap();
        let pinned = node(&d, "pinned").rect;
        assert_eq!((q.x - 100.0, q.y - 50.0), ((loose.x - pinned.x).round(), (loose.y - pinned.y).round()));
    }

    #[test]
    fn arrows_walk_the_map() {
        let (_, d) = setup();
        let go = |from: &str, dir| nearest(&d, &from.into(), dir).map(|n| n.0);
        // Balanced mindmap: A (with A1) on the right of Root, B on the left.
        assert_eq!(go("a", Dir::Right), Some("a1".into()));
        assert_eq!(go("a1", Dir::Left), Some("a".into()));
        assert_eq!(go("root", Dir::Left), Some("b".into()));
        // Free frame: the loose node sits below the pinned one, and the
        // frame to the right does not steal the move.
        assert_eq!(go("pinned", Dir::Down), Some("loose".into()));
        // Nothing further left inside the mindmap: leave the frame? No frame
        // lies left of it either.
        assert_eq!(go("b", Dir::Left), None);
    }

    const BOARD: &str = r#"frame "Board" layout=free {
    node "Wide box here" id=a x=10 y=40
    node "B" id=b x=200 y=0
    node "C" id=c x=90 y=300
    edge a b label="ships"
}
- "Root" {
    - "Kid" shape=diamond
}
"#;

    /// `BOARD` with `moves` written, laid out again.
    fn after(moves: &[(NodeId, Point)]) -> Diagram {
        let mut e = rhizo_lang::Editor::open(BOARD);
        for (id, p) in moves {
            e.move_node(id, p.x, p.y).unwrap();
        }
        rhizo_layout::layout(e.document(), &Theme::light(), &rhizo_layout::Heuristic)
    }

    fn board() -> (Document, Diagram) {
        let doc = load(BOARD, Syntax::Kdl).into_result().unwrap().0;
        let d = rhizo_layout::layout(&doc, &Theme::light(), &rhizo_layout::Heuristic);
        (doc, d)
    }

    #[test]
    fn a_selection_box_picks_the_nodes_it_meets() {
        let (_, d) = board();
        let (a, c) = (node(&d, "a").rect, node(&d, "c").rect);
        let r = rhizo_core::Rect::new(a.x - 5.0, a.y - 5.0, c.right() - a.x, c.y + 7.0 - a.y);
        let mut got = in_rect(&d, r);
        got.sort();
        assert_eq!(got, vec![NodeId::from("a"), NodeId::from("c")]);
        assert!(in_rect(&d, rhizo_core::Rect::new(-9000.0, -9000.0, 1.0, 1.0)).is_empty());
    }

    #[test]
    fn selected_nodes_move_together_and_line_up() {
        let (doc, d) = board();
        let ids: Vec<NodeId> = ["a", "b", "c", "root"].into_iter().map(NodeId::from).collect();
        let m = moved(&d, &doc, &[ids[0].clone(), ids[1].clone(), ids[3].clone()], Point::new(10.0, 5.0));
        assert_eq!(m.len(), 2, "the mindmap node stays: its layout owns it");
        let d2 = after(&m);
        // Relative to C, which stayed (the frame's drawing starts at its content).
        let rel = |d: &Diagram, id: &str| {
            let (r, c) = (node(d, id).rect, node(d, "c").rect);
            (r.x - c.x, r.y - c.y)
        };
        for id in ["a", "b"] {
            let ((x0, y0), (x1, y1)) = (rel(&d, id), rel(&d2, id));
            assert_eq!((x1 - x0, y1 - y0), (10.0, 5.0), "{id} moved by the delta");
        }
        let left = after(&arrange(&d, &doc, &ids, Arrange::Left));
        let xs: Vec<f32> = ["a", "b", "c"].iter().map(|id| node(&left, id).rect.x).collect();
        assert!(xs.iter().all(|x| *x == xs[0]), "left edges line up: {xs:?}");
        let mid = after(&arrange(&d, &doc, &ids, Arrange::Middle));
        let cy: Vec<f32> = ["a", "b", "c"].iter().map(|id| node(&mid, id).rect.center().y).collect();
        assert!(cy.iter().all(|y| (y - cy[0]).abs() < 0.6), "middles line up: {cy:?}");
        // Spread out: the same gap between neighbours, the outer two stay.
        let spread = after(&arrange(&d, &doc, &ids, Arrange::SpreadX));
        let mut r: Vec<_> = ["a", "b", "c"].iter().map(|id| node(&spread, id).rect).collect();
        r.sort_by(|p, q| p.x.total_cmp(&q.x));
        let (g1, g2) = (r[1].x - r[0].right(), r[2].x - r[1].right());
        assert!((g1 - g2).abs() < 1.0, "even gaps {g1} {g2}");
        assert!(arrange(&d, &doc, &ids[..1], Arrange::Left).is_empty(), "one node: nothing to line up");
    }

    #[test]
    fn copying_takes_the_statements_and_the_edges_between_them() {
        let (doc, _) = board();
        let ids: Vec<NodeId> = ["a", "b", "kid", "root"].into_iter().map(NodeId::from).collect();
        assert_eq!(tops(&doc, &ids), vec![NodeId::from("a"), NodeId::from("b"), NodeId::from("root")]);
        let text = copy_text(BOARD, &doc, &ids);
        assert_eq!(
            text,
            "node \"Wide box here\" id=a x=10 y=40\nnode \"B\" id=b x=200 y=0\n- \"Root\" {\n    - \"Kid\" shape=diamond\n}\nedge \"a\" \"b\" label=\"ships\"\n"
        );
        // It reads back as the same nodes.
        let back = load(&text, Syntax::Kdl).into_result().unwrap().0;
        assert_eq!(back.nodes.len(), 4);
        assert_eq!(back.edges.len(), 1);
        assert!(!copy_text(BOARD, &doc, &[NodeId::from("c")]).contains("edge"), "an edge needs both ends");
    }
}
