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

use rhizo_core::{Direction, Document, LayoutKind, NodeId, Point};
use rhizo_lang::Placement;
use rhizo_layout::{Diagram, FrameBox, NodeBox};

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
}
