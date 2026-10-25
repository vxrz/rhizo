//! Mental-map preservation, measured: typical single edits on the example
//! diagrams, and how far the nodes the edit did not touch move on screen.
//!
//! `cargo test -p rhizo-layout stability -- --nocapture` prints the table.
//! "before" is a from-scratch relayout held still at the viewer's old anchor
//! (the selection, else the first node); "anchored" the same layout held at
//! the new anchor (the edited node, else the median move); "after" the
//! stable layout (from the previous one) held at the new anchor.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use rhizo_core::{Document, LayoutKind, NodeId, Point, Theme};
use rhizo_lang::{Editor, Placement};

use crate::stable::{anchor_shift, anchors, layout_from};
use crate::{Diagram, Heuristic, decor, effective_layout, layout};

/// Unchanged nodes moving more than this count as moved.
const MOVED: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Edit {
    AddChild,
    AddSibling,
    AddEdge,
    Rename,
    DeleteLeaf,
    Reparent,
}

const EDITS: [Edit; 6] =
    [Edit::AddChild, Edit::AddSibling, Edit::AddEdge, Edit::Rename, Edit::DeleteLeaf, Edit::Reparent];

/// Movement of unchanged nodes, summed over cases.
#[derive(Clone, Copy, Debug, Default)]
struct Movement {
    nodes: usize,
    sum: f32,
    max: f32,
    moved: usize,
}

impl Movement {
    fn add(&mut self, old: &Diagram, new: &Diagram, shift: Point, skip: &HashSet<NodeId>) {
        for n in &new.nodes {
            let Some(o) = old.nodes.iter().find(|o| o.id == n.id).filter(|_| !skip.contains(&n.id)) else { continue };
            let (a, b) = (o.rect.center(), n.rect.center());
            let d = ((b.x - a.x - shift.x).powi(2) + (b.y - a.y - shift.y).powi(2)).sqrt();
            self.nodes += 1;
            self.sum += d;
            self.max = self.max.max(d);
            self.moved += usize::from(d > MOVED);
        }
    }

    fn mean(&self) -> f32 {
        if self.nodes == 0 { 0.0 } else { self.sum / self.nodes as f32 }
    }

    fn share(&self) -> f32 {
        if self.nodes == 0 { 0.0 } else { self.moved as f32 / self.nodes as f32 }
    }
}

#[derive(Default)]
struct Row {
    cases: usize,
    before: Movement,
    anchored: Movement,
    after: Movement,
}

fn examples() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut out = Vec::new();
    let mut dirs = vec![root];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rhizo") {
                out.push((p.clone(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// The viewer's anchor before this change: the selection, else the first
/// node both layouts have.
fn old_shift(old: &Diagram, new: &Diagram, sel: Option<&NodeId>) -> Point {
    let find = |d: &Diagram, id: &NodeId| d.nodes.iter().find(|n| &n.id == id).map(|n| n.rect.center());
    let pair = |id: &NodeId| Some((find(old, id)?, find(new, id)?));
    sel.and_then(pair)
        .or_else(|| new.nodes.iter().find_map(|n| pair(&n.id)))
        .map_or(Point::new(0.0, 0.0), |(a, b)| Point::new(b.x - a.x, b.y - a.y))
}

/// Staying close to the previous layout never puts two boxes on each other.
fn assert_no_overlap(d: &Diagram, what: &str) {
    for (i, a) in d.nodes.iter().enumerate() {
        for b in &d.nodes[i + 1..] {
            let hit = a.rect.inflate(-0.5, -0.5).intersects(&b.rect);
            assert!(!hit, "{what}: `{}` overlaps `{}`: {:?} {:?}", a.id, b.id, a.rect, b.rect);
        }
    }
}

/// Up to four nodes spread through the document, for edits on `pick`ed ones.
fn targets(doc: &Document, pick: impl Fn(&rhizo_core::Node) -> bool) -> Vec<NodeId> {
    let all: Vec<&NodeId> = doc.nodes.values().filter(|n| !decor::hosted(doc, n) && pick(n)).map(|n| &n.id).collect();
    let mut out: Vec<NodeId> = (0..4).filter_map(|k| all.get(k * all.len() / 4)).map(|&id| id.clone()).collect();
    out.dedup();
    out
}

/// Another node of `t`'s frame, a fixed way along the document from it,
/// that `ok` accepts.
fn other(doc: &Document, t: &NodeId, ok: impl Fn(&NodeId) -> bool) -> Option<NodeId> {
    let frame = &doc.node(t)?.frame;
    let same: Vec<&NodeId> =
        doc.nodes.values().filter(|n| &n.frame == frame && !decor::hosted(doc, n)).map(|n| &n.id).collect();
    let at = same.iter().position(|id| *id == t)?;
    (1..same.len()).map(|k| same[(at + k + same.len() / 3) % same.len()]).find(|id| *id != t && ok(id)).cloned()
}

/// Applies `edit` on `t`: what is selected afterwards in the viewer, and
/// the nodes the edit touched (left out of the measure).
fn apply(ed: &mut Editor, edit: Edit, t: &NodeId) -> Option<(Option<NodeId>, Vec<NodeId>)> {
    let doc = ed.document().clone();
    let new = |ed: &Editor| ed.document().nodes.keys().filter(|k| doc.node(k).is_none()).cloned().collect::<Vec<_>>();
    match edit {
        Edit::AddChild | Edit::AddSibling => {
            let id = if edit == Edit::AddChild { ed.add_child(t, "New node") } else { ed.add_after(t, "New node") };
            let id = id.ok()?;
            Some((Some(id), new(ed)))
        }
        Edit::AddEdge => {
            let linked = |a: &NodeId, b: &NodeId| doc.edges.iter().any(|e| (&e.from, &e.to) == (a, b));
            let to = other(&doc, t, |o| !linked(t, o) && !linked(o, t))?;
            ed.connect(t, &to).ok()?;
            Some((Some(t.clone()), new(ed)))
        }
        Edit::Rename => {
            let label = format!("{} and a much longer label", doc.node(t)?.label);
            ed.rename(t, &label).ok()?;
            Some((Some(t.clone()), vec![t.clone()]))
        }
        Edit::DeleteLeaf => {
            let parent = doc.node(t)?.parent.clone();
            ed.delete(t).ok()?;
            Some((parent, vec![t.clone()]))
        }
        Edit::Reparent => {
            let below = |o: &NodeId| {
                let mut cur = Some(o);
                std::iter::from_fn(|| {
                    let c = cur?;
                    cur = doc.node(c).and_then(|n| n.parent.as_ref());
                    Some(c.clone())
                })
                .any(|a| &a == t)
            };
            let to = other(&doc, t, |o| !below(o) && doc.node(t).is_some_and(|n| n.parent.as_ref() != Some(o)))?;
            ed.reparent(t, &Placement::Child(to)).ok()?;
            // The moved subtree goes with it.
            let moved = doc.nodes.keys().filter(|o| below(o)).cloned().collect();
            Some((Some(t.clone()), moved))
        }
    }
}

fn measure_all() -> BTreeMap<(String, Edit), Row> {
    let theme = Theme::light();
    let mut rows: BTreeMap<(String, Edit), Row> = BTreeMap::new();
    for (_, src) in examples() {
        let base = Editor::open(src.clone());
        if base.diagnostics().iter().any(rhizo_lang::Diagnostic::is_error) {
            continue;
        }
        let doc = base.document().clone();
        let old = layout(&doc, &theme, &Heuristic);
        for edit in EDITS {
            let ts = match edit {
                Edit::DeleteLeaf => targets(&doc, |n| n.children.is_empty()),
                Edit::Reparent => targets(&doc, |n| n.parent.is_some() || doc.nodes.len() > 2),
                _ => targets(&doc, |_| true),
            };
            for t in ts {
                let mut ed = Editor::open(src.clone());
                let Some((sel, touched)) = apply(&mut ed, edit, &t) else { continue };
                let new_doc = ed.document();
                if ed.diagnostics().iter().any(rhizo_lang::Diagnostic::is_error) {
                    continue;
                }
                let kind = doc.frame(&doc.nodes[&t].frame).map_or(LayoutKind::Free, |f| effective_layout(&doc, f));
                let scratch = layout(new_doc, &theme, &Heuristic);
                let stable = layout_from(new_doc, &theme, &Heuristic, &old);
                if edit == Edit::AddChild {
                    let again = layout_from(new_doc, &theme, &Heuristic, &old);
                    let rects = |d: &Diagram| d.nodes.iter().map(|n| (n.id.clone(), n.rect)).collect::<Vec<_>>();
                    assert_eq!(rects(&stable), rects(&again), "the same edit on the same layout lays out the same");
                }
                if kind != LayoutKind::Free {
                    // Free layouts keep the positions people gave them, overlapping or not.
                    assert_no_overlap(&stable, &format!("{edit:?} on {t}"));
                }
                let skip: HashSet<NodeId> = touched.into_iter().collect();
                let prefer = anchors(&doc, new_doc, sel.as_ref());
                let row = rows.entry((format!("{kind:?}"), edit)).or_default();
                row.cases += 1;
                row.before.add(&old, &scratch, old_shift(&old, &scratch, sel.as_ref()), &skip);
                row.anchored.add(&old, &scratch, anchor_shift(&old, &scratch, &prefer), &skip);
                row.after.add(&old, &stable, anchor_shift(&old, &stable, &prefer), &skip);
            }
        }
    }
    rows
}

fn print(rows: &BTreeMap<(String, Edit), Row>) {
    let cell = |m: &Movement| format!("{:6.1} {:6.1} {:4.0}%", m.mean(), m.max, m.share() * 100.0);
    eprintln!(
        "{:<9} {:<10} {:>5} | {:^19} | {:^19} | {:^19}",
        "layout", "edit", "cases", "before mean/max/>8", "anchored", "after"
    );
    for ((kind, edit), r) in rows {
        eprintln!(
            "{:<9} {:<10} {:>5} | {} | {} | {}",
            kind,
            format!("{edit:?}"),
            r.cases,
            cell(&r.before),
            cell(&r.anchored),
            cell(&r.after)
        );
    }
}

#[test]
fn anchors_hold_what_the_edit_left_alone() {
    let doc = |src: &str| Editor::open(src).document().clone();
    let old = doc(r#"- "Root" { - "A" { - "A1" }; - "B" }"#);
    let id = |s: &str| NodeId::new(s);
    // A new node: its nearest older ancestor, then the rest of its line.
    let grown = doc(r#"- "Root" { - "A" { - "A1"; - "New" }; - "B" }"#);
    assert_eq!(anchors(&old, &grown, Some(&id("new"))), vec![id("a"), id("root")]);
    // Selected and untouched: itself first.
    assert_eq!(anchors(&old, &grown, Some(&id("b"))), vec![id("b"), id("root")]);
    // Renamed: it grows from one side, so its parent holds.
    let renamed = doc(r#"- "Root" { - "A" { - "A1" }; - "Bee" id=b }"#);
    assert_eq!(anchors(&old, &renamed, Some(&id("b"))), vec![id("root")]);
    // Moved under a new parent: that parent.
    let moved = doc(r#"- "Root" { - "A"; - "B" { - "A1" } }"#);
    assert_eq!(anchors(&old, &moved, Some(&id("a1"))), vec![id("b"), id("root")]);

    // Most nodes kept their place: they hold, not the preferred node.
    let d = layout(&old, &Theme::light(), &Heuristic);
    let mut nudged = d.clone();
    nudged.nodes.iter_mut().filter(|n| n.id == id("b")).for_each(|n| n.rect = n.rect.translate(0.0, 40.0));
    assert_eq!(anchor_shift(&d, &nudged, &[id("b")]), Point::new(0.0, 0.0));
    // A layout that moved everything differently: the preferred node holds.
    let mut spread = d.clone();
    for (k, n) in spread.nodes.iter_mut().enumerate() {
        n.rect = n.rect.translate(10.0 * k as f32, 0.0);
    }
    assert_eq!(anchor_shift(&d, &spread, &[id("b")]), Point::new(30.0, 0.0));
}

/// The most the unchanged nodes may move on average, in px, after an edit
/// in a stable layout. What remains is what must move: siblings pushed
/// aside, a subtree pushed out by a wider label, a layer pushed down.
fn bound(kind: &str, edit: Edit) -> f32 {
    match (kind, edit) {
        ("Tree" | "Mindmap", Edit::AddChild) => 2.0,
        // A parent left with one child moves over it.
        ("Tree" | "Mindmap", Edit::DeleteLeaf) => 5.0,
        ("Tree" | "Mindmap", _) => 25.0,
        ("Layered", Edit::Rename) => 30.0,
        ("Layered", _) => 8.0,
        ("Force", Edit::AddEdge) => 40.0,
        ("Force", _) => 3.0,
        // Grid, free and sequence: held by the anchor only, measured here.
        _ => f32::INFINITY,
    }
}

#[test]
fn stability_of_single_edits() {
    let rows = measure_all();
    print(&rows);
    assert!(rows.len() > 30, "the examples cover most layouts and edits");
    for ((kind, edit), r) in &rows {
        let (after, before) = (r.after.mean(), r.before.mean());
        assert!(after <= bound(kind, *edit), "{kind} {edit:?}: unchanged nodes move {after:.1} px on average");
        assert!(after <= before + 4.0, "{kind} {edit:?}: {after:.1} px, worse than {before:.1} px before");
    }
}
