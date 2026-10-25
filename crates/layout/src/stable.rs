//! Layouts that keep the person's mental map: after an edit, the viewer lays
//! out again starting from the layout on screen, so nodes the edit did not
//! touch stay where they were (see `docs/design/stable-layout.md`).
//!
//! Each engine takes the previous layout as a hint: trees and mindmaps keep
//! parents beside the child they sat beside and branches on their side,
//! layered graphs keep layers, orders and positions unless a change pays off
//! clearly, force layouts start from the old positions. Without a previous
//! layout (`rhizo render`, export, compile, "Tidy layout") nothing here runs
//! and the layout is the from-scratch one, byte for byte.
//!
//! The same previous layout and the same edit always give the same result.

use std::collections::HashMap;

use rhizo_core::{Document, FrameId, LayoutKind, NodeId, Point, Rect, Theme};

use crate::{Diagram, NodeBox, Options, TextMeasure};

/// Where everything was in the previous layout: node boxes, nested frames
/// under their stand-in's id, and how each frame was laid out.
pub(crate) struct Prev {
    rects: HashMap<NodeId, Rect>,
    kinds: HashMap<FrameId, LayoutKind>,
}

impl Prev {
    pub(crate) fn new(d: &Diagram) -> Prev {
        let nodes = d.nodes.iter().map(|n| (n.id.clone(), n.rect));
        let frames = d.frames.iter().map(|f| (NodeId::new(format!("\u{1}{}", f.id)), f.rect));
        let kinds = d.frames.iter().map(|f| (f.id.clone(), f.layout)).collect();
        Prev { rects: nodes.chain(frames).collect(), kinds }
    }

    /// The center of `id` in the previous layout, in its coordinates.
    pub(crate) fn center(&self, id: &NodeId) -> Option<Point> {
        self.rects.get(id).map(Rect::center)
    }

    /// The engine for an `auto` frame that `effective` now picks: the one it
    /// had, when the edit would only flip it between mindmap and layered
    /// (adding the first link to a mindmap, removing the last edge of a
    /// graph with nesting). Grid and free give way to either as before.
    pub(crate) fn kind(&self, frame: &FrameId, effective: LayoutKind) -> LayoutKind {
        let flip = [LayoutKind::Mindmap, LayoutKind::Layered];
        match self.kinds.get(frame) {
            Some(&was) if flip.contains(&was) && flip.contains(&effective) => was,
            _ => effective,
        }
    }

    /// How far to move `nodes` (laid out in their own coordinates) to put
    /// them back where they were: the median move of those that were there.
    pub(crate) fn offset(&self, nodes: &[NodeBox]) -> Option<Point> {
        let (mut dx, mut dy): (Vec<f32>, Vec<f32>) = nodes
            .iter()
            .filter_map(|n| {
                let (a, b) = (self.center(&n.id)?, n.rect.center());
                Some((a.x - b.x, a.y - b.y))
            })
            .unzip();
        (!dx.is_empty()).then(|| Point::new(median(&mut dx), median(&mut dy)))
    }
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// Lays `doc` out starting from `prev`, the layout shown before an edit.
pub fn layout_from(doc: &Document, theme: &Theme, measure: &dyn TextMeasure, prev: &Diagram) -> Diagram {
    crate::layout_inner(doc, theme, measure, &Options::default(), Some(&Prev::new(prev)))
}

/// The nodes to hold still on screen after `old` became `new`, best first:
/// the selection, else its ancestors when the edit made it, renamed it (it
/// grows from one side, its center moves) or moved it under a new parent.
pub fn anchors(old: &Document, new: &Document, selected: Option<&NodeId>) -> Vec<NodeId> {
    let Some(sel) = selected.and_then(|s| new.node(s)) else { return Vec::new() };
    let mut out = Vec::new();
    if old.node(&sel.id).is_some_and(|o| o.parent == sel.parent && o.label == sel.label) {
        out.push(sel.id.clone());
    }
    let mut cur = sel.parent.as_ref();
    while let Some(p) = cur.filter(|_| out.len() < new.nodes.len()) {
        out.push(p.clone());
        cur = new.node(p).and_then(|n| n.parent.as_ref());
    }
    out
}

/// How far to move `old` so it lines up with `new`: the move most of the
/// nodes they share made, when most made the same (the layout kept them
/// still), else the move of the first of `prefer` both have, else the
/// median move. Zero when they share none.
pub fn anchor_shift(old: &Diagram, new: &Diagram, prefer: &[NodeId]) -> Point {
    let before: HashMap<&NodeId, Point> = old.nodes.iter().map(|n| (&n.id, n.rect.center())).collect();
    let after: HashMap<&NodeId, Point> = new.nodes.iter().map(|n| (&n.id, n.rect.center())).collect();
    let pair = |id: &NodeId| Some((*before.get(id)?, *after.get(id)?));
    let moves: Vec<Point> =
        new.nodes.iter().filter_map(|n| pair(&n.id)).map(|(a, b)| Point::new(b.x - a.x, b.y - a.y)).collect();
    let (mut dx, mut dy): (Vec<f32>, Vec<f32>) = moves.iter().map(|p| (p.x, p.y)).unzip();
    let median = Point::new(median(&mut dx), median(&mut dy));
    // A layout that kept most nodes in place (a stable one) is held by them.
    let still = moves.iter().filter(|p| (p.x - median.x).abs() < 0.5 && (p.y - median.y).abs() < 0.5).count();
    if still * 2 > moves.len() {
        return median;
    }
    match prefer.iter().find_map(pair) {
        Some((a, b)) => Point::new(b.x - a.x, b.y - a.y),
        None => median,
    }
}
