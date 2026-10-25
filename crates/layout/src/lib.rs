//! Turns a [`Document`] into a [`Diagram`]: positioned, styled boxes and
//! routed edge paths, ready for any renderer.
//!
//! Each frame runs its own engine ([`tidy`] for trees and mindmaps,
//! [`layered`] for graphs, [`sequence`] for sequence diagrams, plus grid and
//! free placement). Frames are then
//! placed on one canvas and edges between frames are routed last.

mod decor;
pub mod force;
pub mod layered;
pub mod path;
pub mod resolve;
mod route;
pub mod sequence;
#[cfg(test)]
mod stability_tests;
pub mod stable;
pub mod text;
pub mod tidy;

use std::collections::HashMap;

use rhizo_core::{
    Arrow, Color, DecorState, Direction, Document, Edge, Frame, FrameId, LayoutKind, LineStyle, MAX_FRAME_DEPTH, Node,
    NodeId, Point, Rect, Shape, Size, Theme,
};

pub use path::{Path, Seg};

pub use text::{Heuristic, TextMeasure};

use resolve::{EdgeRole, NodeCtx};

/// A laid-out document.
#[derive(Clone, Debug)]
pub struct Diagram {
    pub title: Option<String>,
    /// Everything, plus a margin.
    pub bounds: Rect,
    pub background: Color,
    pub font_family: String,
    pub frames: Vec<FrameBox>,
    pub nodes: Vec<NodeBox>,
    pub edges: Vec<EdgeBox>,
}

#[derive(Clone, Debug)]
pub struct FrameBox {
    pub id: FrameId,
    /// The frame this one is drawn inside.
    pub parent: Option<FrameId>,
    /// How many frames this one is inside.
    pub depth: usize,
    pub label: Option<String>,
    pub layout: LayoutKind,
    pub rect: Rect,
    /// Implicit frames are not drawn.
    pub visible: bool,
    pub fill: Color,
    pub stroke: Color,
    pub label_color: Color,
    pub label_size: f32,
}

#[derive(Clone, Debug)]
pub struct NodeBox {
    pub id: NodeId,
    pub frame: FrameId,
    pub label: String,
    pub lines: Vec<String>,
    pub rect: Rect,
    pub shape: Shape,
    pub fill: Color,
    pub stroke: Color,
    pub stroke_width: f32,
    pub line: LineStyle,
    pub text: Color,
    pub font_size: f32,
    pub font_weight: u16,
    pub line_height: f32,
    pub depth: usize,
    pub branch: Option<usize>,
    pub class: Vec<String>,
    pub note: Option<String>,
    pub link: Option<String>,
    /// What a kit's look adds; `None` for plain nodes.
    pub typed: Option<Box<Typed>>,
}

/// A typed node's drawing beyond its shape.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Typed {
    /// `data.entity`; `None` for untyped rows (an enum's values).
    pub ty: Option<String>,
    /// The box this one is drawn inside: the card for a row, a container.
    pub host: Option<NodeId>,
    pub row: bool,
    pub card: bool,
    pub container: bool,
    pub state: DecorState,
    /// Drawn instead of the centered label when not empty.
    pub ink: Vec<Ink>,
}

/// One mark of a typed node's drawing, relative to its box's top-left.
#[derive(Clone, Debug, PartialEq)]
pub enum Ink {
    /// A line of text from `x`, or ending at `x` when `end`.
    Text { text: String, x: f32, baseline: f32, size: f32, weight: u16, color: Color, end: bool, class: &'static str },
    /// A filled tag with centered bold text: a badge or a marker.
    Tag { rect: Rect, fill: Color, text: String, size: f32, color: Color, class: &'static str },
    /// A straight line: a card's dividers, a strike-through.
    Rule { from: Point, to: Point, color: Color, width: f32 },
    /// A card's title band: a rectangle with its top corners rounded.
    Band { rect: Rect, radius: f32, fill: Color },
}

impl Ink {
    pub fn offset(mut self, d: Point) -> Ink {
        match &mut self {
            Ink::Text { x, baseline, .. } => (*x, *baseline) = (*x + d.x, *baseline + d.y),
            Ink::Tag { rect, .. } | Ink::Band { rect, .. } => *rect = rect.translate(d.x, d.y),
            Ink::Rule { from, to, .. } => (*from, *to) = (from.offset(d.x, d.y), to.offset(d.x, d.y)),
        }
        self
    }
}

impl NodeBox {
    /// The kit's drawing to put on top of the shape, if any.
    pub fn ink(&self) -> &[Ink] {
        self.typed.as_ref().map_or(&[], |t| &t.ink)
    }
}

#[derive(Clone, Debug)]
pub struct LabelBox {
    pub lines: Vec<String>,
    pub center: Point,
    pub size: Size,
    pub font_size: f32,
    pub line_height: f32,
    pub color: Color,
    pub background: Color,
}

impl LabelBox {
    pub fn rect(&self) -> Rect {
        Rect::from_center(self.center, self.size)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// Parent → child nesting.
    Branch,
    /// An explicit edge.
    Link,
    /// A sequence participant's lifeline. Drawn like an edge, but it is not a
    /// document edge: `from` and `to` are both the participant.
    Lifeline,
}

#[derive(Clone, Debug)]
pub struct EdgeBox {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub path: Path,
    pub color: Color,
    pub width: f32,
    pub line: LineStyle,
    pub arrow: Arrow,
    pub label: Option<LabelBox>,
    pub class: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Labels wrap beyond this width.
    pub max_text_width: f32,
    pub margin: f32,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_text_width: 220.0, margin: 32.0 }
    }
}

/// What `auto` means for a frame.
pub fn effective_layout(doc: &Document, frame: &Frame) -> LayoutKind {
    if frame.layout != LayoutKind::Auto {
        return frame.layout;
    }
    if doc.edges_in(&frame.id).next().is_some() || lifted(doc, &frame.id).next().is_some() {
        LayoutKind::Layered
    } else if doc
        .nodes_in(&frame.id)
        .any(|n| n.children.iter().any(|c| doc.node(c).is_some_and(|c| !decor::hosted(doc, c))))
    {
        LayoutKind::Mindmap
    } else if doc.nodes_in(&frame.id).any(|n| n.at.is_some()) {
        LayoutKind::Free
    } else {
        LayoutKind::Grid
    }
}

/// The index in `doc.edges` behind each drawn edge (`None` for nesting),
/// matched per (from, to) pair in document order. Every layout pass keeps
/// that order within a pair.
pub fn edge_sources(doc: &Document, d: &Diagram) -> Vec<Option<usize>> {
    let mut queues: HashMap<(&str, &str), std::collections::VecDeque<usize>> = HashMap::new();
    for (k, e) in doc.edges.iter().enumerate() {
        queues.entry((e.from.as_str(), e.to.as_str())).or_default().push_back(k);
    }
    d.edges
        .iter()
        .map(|eb| match eb.kind {
            EdgeKind::Link => queues.get_mut(&(eb.from.as_str(), eb.to.as_str())).and_then(|q| q.pop_front()),
            EdgeKind::Branch | EdgeKind::Lifeline => None,
        })
        .collect()
}

pub fn layout(doc: &Document, theme: &Theme, measure: &dyn TextMeasure) -> Diagram {
    layout_with(doc, theme, measure, &Options::default())
}

/// The frame `frame` is laid out inside: its parent, unless that parent
/// cannot hold frames (a sequence) or the nesting is broken (a cycle, too
/// deep). Such frames are laid out at the top level instead.
pub fn layout_parent<'a>(doc: &'a Document, frame: &'a Frame) -> Option<&'a FrameId> {
    let p = frame.parent.as_ref()?;
    let ok = doc.frame(p).is_some_and(|pf| pf.layout != LayoutKind::Sequence)
        && doc.frame_ancestors(&frame.id).take(MAX_FRAME_DEPTH + 1).all(|a| a != &frame.id)
        && doc.frame_ancestors(&frame.id).take(MAX_FRAME_DEPTH + 1).count() <= MAX_FRAME_DEPTH;
    ok.then_some(p)
}

/// What stands for node `n` in `frame`'s layout: `frame` itself when the
/// node is one of its own, else the frame directly inside `frame` that
/// holds it, at any depth. `None` when the node is outside `frame`.
fn member<'a>(doc: &'a Document, frame: &FrameId, n: &NodeId) -> Option<&'a FrameId> {
    let mut cur = doc.frame(&doc.node(n)?.frame)?;
    if &cur.id == frame {
        return Some(&cur.id);
    }
    for _ in 0..=MAX_FRAME_DEPTH {
        let p = layout_parent(doc, cur)?;
        if p == frame {
            return Some(&cur.id);
        }
        cur = doc.frame(p)?;
    }
    None
}

/// Edges that run between two different boxes of `frame`'s layout with at
/// least one end in a nested frame, as the pair of members they connect.
/// They are drawn later, between frames, but shape the parent's layout.
fn lifted<'a>(doc: &'a Document, frame: &'a FrameId) -> impl Iterator<Item = (&'a FrameId, &'a FrameId, &'a Edge)> {
    let nested = doc.frames.iter().any(|f| f.parent.as_ref() == Some(frame));
    doc.edges.iter().filter(move |_| nested).filter_map(move |e| {
        let (a, b) = (member(doc, frame, &e.from)?, member(doc, frame, &e.to)?);
        (a != b).then_some((a, b, e))
    })
}

/// The invisible node standing for a nested frame in its parent's layout.
fn stand_in(f: &Frame, parent: &FrameId) -> Node {
    // A control character: no real id has one (ids are checked for them).
    let mut n = Node::new(format!("\u{1}{}", f.id), "", parent.clone());
    n.at = f.at;
    n
}

/// A frame laid out with everything inside it, in its own coordinates: the
/// frame's box first, at the origin, then the frames nested in it.
struct Laid {
    frames: Vec<FrameBox>,
    nodes: Vec<NodeBox>,
    edges: Vec<EdgeBox>,
    /// Where sequence frames draw their messages.
    message_areas: Vec<Rect>,
}

impl Laid {
    fn translate(&mut self, dx: f32, dy: f32) {
        for f in &mut self.frames {
            f.rect = f.rect.translate(dx, dy);
        }
        for n in &mut self.nodes {
            n.rect = n.rect.translate(dx, dy);
        }
        for e in &mut self.edges {
            e.path = e.path.map(|p| p.offset(dx, dy));
            if let Some(l) = &mut e.label {
                l.center = l.center.offset(dx, dy);
            }
        }
        for r in &mut self.message_areas {
            *r = r.translate(dx, dy);
        }
    }

    fn append(&mut self, other: Laid) {
        self.frames.extend(other.frames);
        self.nodes.extend(other.nodes);
        self.edges.extend(other.edges);
        self.message_areas.extend(other.message_areas);
    }
}

/// Lays out `frame`: the frames nested in it first, then its own nodes
/// with each nested frame as one box among them.
fn lay_frame(
    doc: &Document,
    theme: &Theme,
    measure: &dyn TextMeasure,
    opts: &Options,
    frame: &Frame,
    depth: usize,
    prev: Option<&stable::Prev>,
) -> Option<Laid> {
    let mut kids = Vec::new();
    for c in doc.frames.iter().filter(|c| layout_parent(doc, c) == Some(&frame.id)) {
        kids.extend(lay_frame(doc, theme, measure, opts, c, depth + 1, prev).map(|l| (stand_in(c, &frame.id), l)));
    }
    let blocks: Vec<(&Node, Size)> = kids.iter().map(|(n, l)| (n, l.frames[0].rect.size())).collect();
    let mut fl = FrameLayout::new(doc, theme, measure, opts, frame, &blocks, prev);
    if fl.items.is_empty() && frame.implicit {
        return None;
    }
    fl.run();
    fl.reattach_inner();
    let (mut fbox, nodes, edges, spots) = fl.finish();
    fbox.parent = layout_parent(doc, frame).cloned();
    fbox.depth = depth;
    if depth % 2 == 1 && frame.style.fill.is_none() {
        // Nested frames alternate with their parent's fill so each level reads as its own region.
        fbox.fill = theme.background;
    }
    let sequence = fbox.layout == LayoutKind::Sequence;
    let mut out = Laid { frames: vec![fbox], nodes, edges, message_areas: Vec::new() };
    if sequence {
        let parts = out.edges.iter().flat_map(|e| [Some(e.path.bounds()), e.label.as_ref().map(LabelBox::rect)]);
        out.message_areas.extend(parts.flatten().reduce(|a, b| a.union(&b)));
    }
    for ((_, mut laid), at) in kids.into_iter().zip(spots) {
        laid.translate(at.x, at.y);
        out.append(laid);
    }
    Some(out)
}

pub fn layout_with(doc: &Document, theme: &Theme, measure: &dyn TextMeasure, opts: &Options) -> Diagram {
    layout_inner(doc, theme, measure, opts, None)
}

/// The layout, from scratch or (with `prev`) starting from a previous one.
fn layout_inner(
    doc: &Document,
    theme: &Theme,
    measure: &dyn TextMeasure,
    opts: &Options,
    prev: Option<&stable::Prev>,
) -> Diagram {
    let mut all = Laid { frames: Vec::new(), nodes: Vec::new(), edges: Vec::new(), message_areas: Vec::new() };
    let mut cursor = 0.0f32;
    let mut placed_any = false;

    for frame in doc.frames.iter().filter(|f| layout_parent(doc, f).is_none()) {
        let Some(mut laid) = lay_frame(doc, theme, measure, opts, frame, 0, prev) else { continue };
        let origin = match frame.at {
            Some(p) => p,
            None => {
                let next = if placed_any { cursor + FRAME_GAP } else { cursor };
                // After an edit, a frame stays where its nodes were, pushed
                // right only as far as the frame before it needs.
                let p = match prev.and_then(|pr| pr.offset(&laid.nodes)) {
                    Some(d) if placed_any => Point::new(d.x.max(next), d.y),
                    Some(d) => d,
                    None => Point::new(next, 0.0),
                };
                cursor = p.x + laid.frames[0].rect.width;
                placed_any = true;
                p
            }
        };
        laid.translate(origin.x, origin.y);
        all.append(laid);
    }
    let Laid { frames, nodes, mut edges, message_areas } = all;

    // Edges whose ends are in different frames. Their labels keep clear of
    // every node and of the labels already placed.
    let index: HashMap<&NodeId, usize> = nodes.iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
    let mut taken: Vec<Rect> = nodes
        .iter()
        .map(|n| n.rect)
        .chain(edges.iter().filter_map(|e| e.label.as_ref().map(LabelBox::rect)))
        .chain(message_areas.iter().copied())
        .chain(frames.iter().filter(|f| f.visible && f.label.is_some()).map(|f| {
            // The frame's title line.
            Rect::new(f.rect.x, f.rect.y, f.rect.width, 16.0 + f.label_size * 1.2)
        }))
        .collect();
    // They detour around nodes and around frames they do not belong to.
    let cross: Vec<(&Edge, usize, usize)> = doc
        .edges
        .iter()
        .filter(|e| doc.node(&e.from).zip(doc.node(&e.to)).is_some_and(|(a, b)| a.frame != b.frame))
        .filter_map(|e| Some((e, *index.get(&e.from)?, *index.get(&e.to)?)))
        .collect();
    let mut obstacles = if cross.is_empty() {
        route::Obstacles::new(Vec::new())
    } else {
        route::Obstacles::new(
            nodes.iter().map(|n| n.rect).chain(frames.iter().map(|f| f.rect)).chain(message_areas).collect(),
        )
    };
    // Obstacle indices of the drawn frames holding a node, innermost first.
    let frame_index: HashMap<&FrameId, usize> = frames.iter().enumerate().map(|(i, f)| (&f.id, i)).collect();
    let frames_of = |n: &NodeBox| {
        let mut out = Vec::new();
        let mut cur = frame_index.get(&n.frame).copied();
        while let Some(i) = cur.filter(|_| out.len() <= MAX_FRAME_DEPTH) {
            if frames[i].visible {
                out.push(nodes.len() + i);
            }
            cur = frames[i].parent.as_ref().and_then(|p| frame_index.get(p)).copied();
        }
        out
    };
    let hidden: Vec<usize> = (0..frames.len()).filter(|&i| !frames[i].visible).map(|i| nodes.len() + i).collect();
    let mut lanes = Lanes::default();
    // Nodes drawn inside another route to the outermost box holding them.
    let host_of = |i: usize| nodes[i].typed.as_ref().and_then(|t| t.host.as_ref()).and_then(|h| index.get(h)).copied();
    let outer = |mut i: usize| {
        while let Some(h) = host_of(i) {
            i = h;
        }
        i
    };
    for (e, ia, ib) in cross {
        let look = resolve::edge(doc, theme, Some(e), EdgeRole::Link { secondary: false });
        let (ta, tb) = (outer(ia), outer(ib));
        let (na, nb) = (&nodes[ta], &nodes[tb]);
        let (lane, bend) = lanes.next(ta, tb);
        let mut skip = vec![ta, tb];
        skip.extend(frames_of(na));
        skip.extend(frames_of(nb));
        skip.extend(&hidden);
        let mut path = avoid(curved_link(na, nb, 0.12 + bend), &mut obstacles, &skip, na, nb, lane);
        if (ta, tb) != (ia, ib) {
            let end = |i: usize| (nodes[i].clone(), host_of(i).map(|h| nodes[h].rect));
            // The hosts between each end and its outermost box may be crossed too.
            for mut i in [ia, ib] {
                while let Some(h) = host_of(i).filter(|_| skip.len() < nodes.len() + frames.len()) {
                    skip.push(h);
                    i = h;
                }
            }
            skip.extend([ia, ib]);
            path = reattach_around(&end(ia), &end(ib), &mut obstacles, &skip);
        }
        let eb = link_box(e, path, look, theme, measure, opts, &taken);
        taken.extend(eb.label.as_ref().map(LabelBox::rect));
        edges.push(eb);
    }

    let mut bounds: Option<Rect> = None;
    let mut grow = |r: Rect| bounds = Some(bounds.map_or(r, |b| b.union(&r)));
    for f in frames.iter().filter(|f| f.visible) {
        grow(f.rect);
    }
    for n in &nodes {
        grow(n.rect);
    }
    for e in &edges {
        grow(e.path.bounds());
        if let Some(l) = &e.label {
            grow(l.rect());
        }
    }
    let bounds = bounds.unwrap_or_default().inflate(opts.margin, opts.margin);

    Diagram {
        title: doc.title.clone(),
        bounds,
        background: theme.background,
        font_family: theme.font_family.to_string(),
        frames,
        nodes,
        edges,
    }
}

const FRAME_GAP: f32 = 64.0;
const FRAME_PAD: f32 = 28.0;

struct Item<'a> {
    node: &'a Node,
    look: resolve::NodeLook,
    lines: Vec<String>,
    line_height: f32,
    size: Size,
    depth: usize,
    branch: Option<usize>,
    rect: Rect,
    ink: Vec<Ink>,
}

struct FrameLayout<'a> {
    doc: &'a Document,
    theme: &'a Theme,
    measure: &'a dyn TextMeasure,
    opts: &'a Options,
    frame: &'a Frame,
    kind: LayoutKind,
    items: Vec<Item<'a>>,
    index: HashMap<&'a NodeId, usize>,
    /// Nodes drawn inside an item (card rows, contained nodes): the item
    /// that holds them, and the box itself.
    inner: Vec<(usize, decor::Inner<'a>)>,
    inner_index: HashMap<&'a NodeId, usize>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    edges: Vec<EdgeBox>,
    /// Items from here on stand for nested frames: placed like nodes, not drawn.
    first_block: usize,
    /// The layout shown before an edit, to stay close to.
    prev: Option<&'a stable::Prev>,
}

impl<'a> FrameLayout<'a> {
    fn new(
        doc: &'a Document,
        theme: &'a Theme,
        measure: &'a dyn TextMeasure,
        opts: &'a Options,
        frame: &'a Frame,
        blocks: &[(&'a Node, Size)],
        prev: Option<&'a stable::Prev>,
    ) -> Self {
        let kind = effective_layout(doc, frame);
        let kind = match prev {
            Some(p) if frame.layout == LayoutKind::Auto => p.kind(&frame.id, kind),
            _ => kind,
        };
        // Rows and contained nodes are drawn by the node holding them, not placed by the engine.
        let mut list: Vec<&Node> = doc.nodes_in(&frame.id).filter(|n| !decor::hosted(doc, n)).collect();
        let first_block = list.len();
        list.extend(blocks.iter().map(|(n, _)| *n));
        let index: HashMap<&NodeId, usize> = list.iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
        let children: Vec<Vec<usize>> =
            list.iter().map(|n| n.children.iter().filter_map(|c| index.get(c).copied()).collect()).collect();
        let is_root = |n: &Node| n.parent.as_ref().is_none_or(|p| doc.node(p).is_some_and(|p| decor::hosted(doc, p)));
        let roots: Vec<usize> = (0..list.len()).filter(|&i| is_root(list[i])).collect();

        // Depth and branch (index of the depth-1 ancestor among its siblings).
        let mut depth = vec![0usize; list.len()];
        let mut branch: Vec<Option<usize>> = vec![None; list.len()];
        let mut stack: Vec<usize> = roots.clone();
        while let Some(v) = stack.pop() {
            for (k, &c) in children[v].iter().enumerate() {
                depth[c] = depth[v] + 1;
                branch[c] = if depth[v] == 0 { Some(k) } else { branch[v] };
                stack.push(c);
            }
        }

        let env = decor::Env { doc, theme, measure, opts };
        let mut inner = Vec::new();
        let items = list
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let cx =
                    NodeCtx { layout: kind, depth: depth[i], branch: branch[i], has_children: !children[i].is_empty() };
                let b = env.build(n, &cx);
                inner.extend(b.inner.into_iter().map(|x| (i, x)));
                Item {
                    node: n,
                    look: b.look,
                    lines: b.lines,
                    line_height: b.line_height,
                    size: b.size,
                    depth: depth[i],
                    branch: branch[i],
                    rect: Rect::default(),
                    ink: b.ink,
                }
            })
            .collect();
        let mut items: Vec<Item> = items;
        for (it, (_, size)) in items[first_block..].iter_mut().zip(blocks) {
            it.size = *size;
        }
        let inner_index =
            inner.iter().enumerate().map(|(k, (_, x)): (usize, &(usize, decor::Inner))| (&x.node.id, k)).collect();

        FrameLayout {
            doc,
            theme,
            measure,
            opts,
            frame,
            kind,
            items,
            index,
            inner,
            inner_index,
            children,
            roots,
            edges: Vec::new(),
            first_block,
            prev,
        }
    }

    /// Edges between this frame's boxes that run into nested frames, as
    /// item pairs, each pair once. They place the boxes; they are drawn later.
    fn lifted(&self) -> Vec<(usize, usize)> {
        let item = |f: &FrameId, n: &NodeId| {
            if f == &self.frame.id {
                self.item_of(n)
            } else {
                let stub = format!("\u{1}{f}");
                (self.first_block..self.items.len()).find(|&i| self.items[i].node.id.as_str() == stub)
            }
        };
        let mut out: Vec<(usize, usize)> = lifted(self.doc, &self.frame.id)
            .filter_map(|(a, b, e)| Some((item(a, &e.from)?, item(b, &e.to)?)))
            .collect();
        let mut seen = std::collections::HashSet::new();
        out.retain(|p| seen.insert(*p));
        out
    }

    /// The item an edge end attaches to in the engine: the node's own, or
    /// the one holding it.
    fn item_of(&self, id: &NodeId) -> Option<usize> {
        self.index.get(id).or_else(|| self.inner_index.get(id).map(|&k| &self.inner[k].0)).copied()
    }

    fn run(&mut self) {
        match self.kind {
            LayoutKind::Mindmap => self.mindmap(),
            LayoutKind::Tree => self.tree(),
            LayoutKind::Layered => self.layered(),
            LayoutKind::Grid => self.grid(),
            LayoutKind::Force => self.force(),
            LayoutKind::Sequence => self.sequence(),
            LayoutKind::Free | LayoutKind::Auto => self.free(),
        }
    }

    fn sizes_for(&self, dir: Direction) -> Vec<(f32, f32)> {
        self.items
            .iter()
            .map(
                |it| {
                    if dir.is_horizontal() { (it.size.width, it.size.height) } else { (it.size.height, it.size.width) }
                },
            )
            .collect()
    }

    fn place_tree(&mut self, pos: &[(f32, f32)], members: &[usize], dir: Direction) {
        for &i in members {
            let (d, b) = pos[i];
            let s = self.items[i].size;
            self.items[i].rect = match dir {
                Direction::Right => Rect::new(d, b, s.width, s.height),
                Direction::Left => Rect::new(-(d + s.width), b, s.width, s.height),
                Direction::Down => Rect::new(b, d, s.width, s.height),
                Direction::Up => Rect::new(b, -(d + s.height), s.width, s.height),
            };
        }
    }

    fn subtree(&self, root: usize) -> Vec<usize> {
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            out.extend(self.children[out[i]].iter().copied());
            i += 1;
        }
        out
    }

    /// Each item's center in the previous layout; empty without one (or
    /// when none of the items was in it: a new frame is laid out fresh).
    fn before(&self) -> Vec<Option<Point>> {
        let Some(prev) = self.prev else { return Vec::new() };
        let out: Vec<Option<Point>> = self.items.iter().map(|it| prev.center(&it.node.id)).collect();
        if out.iter().all(Option::is_none) { Vec::new() } else { out }
    }

    /// Each item's previous breadth center for a tree growing in `dir`.
    fn before_breadth(&self, dir: Direction) -> Vec<Option<f32>> {
        self.before().iter().map(|p| p.map(|p| if dir.is_horizontal() { p.y } else { p.x })).collect()
    }

    fn mindmap(&mut self) {
        let spacing = tidy::Spacing { level: 56.0, sibling: 10.0 };
        if let Some(dir) = self.frame.direction {
            let sizes = self.sizes_for(dir);
            let pos = tidy::layout_kept(
                &sizes,
                &self.children,
                &self.roots,
                &tidy::Spacing { level: 56.0, sibling: if dir.is_horizontal() { 10.0 } else { 20.0 } },
                &self.before_breadth(dir),
            );
            let all: Vec<usize> = (0..self.items.len()).collect();
            self.place_tree(&pos, &all, dir);
        } else {
            let mut y_cursor = 0.0f32;
            let before = self.before();
            let breadth: Vec<Option<f32>> = before.iter().map(|p| p.map(|p| p.y)).collect();
            for r in self.roots.clone() {
                let kids = self.children[r].clone();
                // Split by subtree weight; right side reads top to bottom,
                // left side continues clockwise (bottom to top).
                let weight: Vec<f32> = kids.iter().map(|&k| self.subtree(k).len() as f32).collect();
                let half = weight.iter().sum::<f32>() / 2.0;
                let mut acc = 0.0;
                let mut split = kids.len();
                for (k, w) in weight.iter().enumerate() {
                    if k > 0 && acc + w / 2.0 > half {
                        split = k;
                        break;
                    }
                    acc += w;
                }
                let (right, left) = match kept_sides(&kids, r, &before) {
                    Some(sides) => sides,
                    None => (kids[..split].to_vec(), kids[split..].to_vec()),
                };
                let mut children = self.children.clone();
                let mut members = Vec::new();

                children[r] = right.clone();
                let sizes = self.sizes_for(Direction::Right);
                let pos_r = tidy::layout_kept(&sizes, &children, &[r], &spacing, &breadth);
                let right_members: Vec<usize> =
                    std::iter::once(r).chain(right.iter().flat_map(|&k| self.subtree(k))).collect();
                self.place_tree(&pos_r, &right_members, Direction::Right);
                let root_rect = self.items[r].rect;
                members.extend(right_members);

                if !left.is_empty() {
                    children[r] = left.iter().rev().copied().collect();
                    let pos_l = tidy::layout_kept(&sizes, &children, &[r], &spacing, &breadth);
                    let left_members: Vec<usize> = left.iter().flat_map(|&k| self.subtree(k)).collect();
                    let mut with_root = left_members.clone();
                    with_root.push(r);
                    self.place_tree(&pos_l, &with_root, Direction::Left);
                    let moved = self.items[r].rect;
                    let (dx, dy) = (root_rect.x - moved.x, root_rect.y - moved.y);
                    for &i in &left_members {
                        self.items[i].rect = self.items[i].rect.translate(dx, dy);
                    }
                    self.items[r].rect = root_rect;
                    members.extend(left_members);
                }

                // Stack multiple maps vertically, roots aligned on x = 0.
                let bbox = Rect::bounding(members.iter().map(|&i| &self.items[i].rect)).unwrap();
                let dx = -root_rect.center().x;
                let dy = y_cursor - bbox.y;
                for &i in &members {
                    self.items[i].rect = self.items[i].rect.translate(dx, dy);
                }
                y_cursor += bbox.height + 56.0;
            }
        }
        self.branch_edges(true);
        self.links(true);
    }

    fn tree(&mut self) {
        let dir = self.frame.direction.unwrap_or(Direction::Down);
        let sizes = self.sizes_for(dir);
        let sibling = if dir.is_horizontal() { 10.0 } else { 20.0 };
        let pos = tidy::layout_kept(
            &sizes,
            &self.children,
            &self.roots,
            &tidy::Spacing { level: 44.0, sibling },
            &self.before_breadth(dir),
        );
        let all: Vec<usize> = (0..self.items.len()).collect();
        self.place_tree(&pos, &all, dir);
        self.branch_edges(false);
        self.links(true);
    }

    /// Parent → child connectors for trees and mindmaps.
    fn branch_edges(&mut self, mindmap: bool) {
        for p in 0..self.items.len() {
            for &c in &self.children[p] {
                let (pr, cr) = (self.items[p].rect, self.items[c].rect);
                let (pc, cc) = (pr.center(), cr.center());
                let horizontal = self.frame.direction.is_none_or(Direction::is_horizontal);
                let (a, b) = if horizontal {
                    if cc.x >= pc.x {
                        (Point::new(pr.right(), pc.y), Point::new(cr.x, cc.y))
                    } else {
                        (Point::new(pr.x, pc.y), Point::new(cr.right(), cc.y))
                    }
                } else if cc.y >= pc.y {
                    (Point::new(pc.x, pr.bottom()), Point::new(cc.x, cr.y))
                } else {
                    (Point::new(pc.x, pr.y), Point::new(cc.x, cr.bottom()))
                };
                let path = if mindmap {
                    if horizontal {
                        let mx = (b.x - a.x) * 0.5;
                        Path { start: a, segs: vec![Seg::Cubic(a.offset(mx, 0.0), b.offset(-mx, 0.0), b)] }
                    } else {
                        let my = (b.y - a.y) * 0.5;
                        Path { start: a, segs: vec![Seg::Cubic(a.offset(0.0, my), b.offset(0.0, -my), b)] }
                    }
                } else {
                    let pts = if horizontal {
                        let mx = (a.x + b.x) / 2.0;
                        [a, Point::new(mx, a.y), Point::new(mx, b.y), b]
                    } else {
                        let my = (a.y + b.y) / 2.0;
                        [a, Point::new(a.x, my), Point::new(b.x, my), b]
                    };
                    rounded_polyline(&pts, 6.0)
                };
                let it = &self.items[c];
                let look = resolve::edge(
                    self.doc,
                    self.theme,
                    None,
                    EdgeRole::Branch { branch: it.branch, depth: it.depth, mindmap },
                );
                self.edges.push(EdgeBox {
                    from: self.items[p].node.id.clone(),
                    to: it.node.id.clone(),
                    kind: EdgeKind::Branch,
                    path,
                    color: look.color,
                    width: look.width,
                    line: look.line,
                    arrow: Arrow::None,
                    label: None,
                    class: Vec::new(),
                });
            }
        }
    }

    /// Explicit edges inside the frame, drawn as gentle curves that detour
    /// around nodes in their way. Their labels keep clear of the frame's
    /// nodes and of each other.
    fn links(&mut self, secondary: bool) {
        let doc = self.doc;
        if doc.edges_in(&self.frame.id).next().is_none() {
            return;
        }
        let mut taken: Vec<Rect> = self.items.iter().map(|it| it.rect).collect();
        let mut obstacles = route::Obstacles::new(taken.clone());
        let mut lanes = Lanes::default();
        let mut loops = vec![0; self.items.len()];
        for e in doc.edges_in(&self.frame.id) {
            let (Some(a), Some(b)) = (self.item_of(&e.from), self.item_of(&e.to)) else { continue };
            let look = resolve::edge(doc, self.theme, Some(e), EdgeRole::Link { secondary });
            let (na, nb) = (self.node_box(a), self.node_box(b));
            let path = if a == b {
                loops[a] += 1;
                self_loop(&na.rect, na.shape, false, loops[a] - 1)
            } else {
                let (lane, bend) = lanes.next(a, b);
                let path = curved_link(&na, &nb, (if secondary { 0.18 } else { 0.0 }) + bend);
                avoid(path, &mut obstacles, &[a, b], &na, &nb, lane)
            };
            let eb = link_box(e, path, look, self.theme, self.measure, self.opts, &taken);
            taken.extend(eb.label.as_ref().map(LabelBox::rect));
            self.edges.push(eb);
        }
    }

    fn layered(&mut self) {
        let dir = self.frame.direction.unwrap_or(Direction::Down);
        let doc = self.doc;
        let size: Vec<(f32, f32)> =
            self.items
                .iter()
                .map(|it| {
                    if dir.is_horizontal() { (it.size.height, it.size.width) } else { (it.size.width, it.size.height) }
                })
                .collect();
        let shape: Vec<Shape> = self.items.iter().map(|it| it.look.shape).collect();

        // Nesting first, then explicit edges.
        let mut edges: Vec<(usize, usize)> = Vec::new();
        let mut sources: Vec<Option<&Edge>> = Vec::new();
        for p in 0..self.items.len() {
            for &c in &self.children[p] {
                edges.push((p, c));
                sources.push(None);
            }
        }
        for e in doc.edges_in(&self.frame.id) {
            if let (Some(a), Some(b)) = (self.item_of(&e.from), self.item_of(&e.to)) {
                edges.push((a, b));
                sources.push(Some(e));
            }
        }
        let looks: Vec<resolve::EdgeLook> = sources
            .iter()
            .map(|e| match e {
                Some(e) => resolve::edge(doc, self.theme, Some(e), EdgeRole::Link { secondary: false }),
                None => resolve::edge(doc, self.theme, None, EdgeRole::Nesting),
            })
            .collect();
        let label_lines: Vec<Option<(Vec<String>, Size)>> = sources
            .iter()
            .zip(&looks)
            .map(|(e, look)| {
                let text = e.and_then(|e| e.label.as_deref())?;
                Some(self.label_size(text, look.label_size))
            })
            .collect();
        let mut labels: Vec<Option<(f32, f32)>> = label_lines
            .iter()
            .map(|l| {
                l.as_ref().map(|(_, s)| if dir.is_horizontal() { (s.height, s.width) } else { (s.width, s.height) })
            })
            .collect();
        // Edges into nested frames rank the frames; they are routed later.
        let drawn = edges.len();
        for pair in self.lifted() {
            edges.push(pair);
            labels.push(None);
        }

        let before: Vec<Option<(f32, f32)>> = self
            .before()
            .iter()
            .map(|p| {
                p.map(|p| match dir {
                    Direction::Down => (p.x, p.y),
                    Direction::Up => (p.x, -p.y),
                    Direction::Right => (p.y, p.x),
                    Direction::Left => (p.y, -p.x),
                })
            })
            .collect();
        let out = layered::layout(&layered::Input {
            size: &size,
            shape: &shape,
            edges: &edges,
            labels: &labels,
            node_sep: if dir.is_horizontal() { 18.0 } else { 28.0 },
            rank_sep: if dir.is_horizontal() { 64.0 } else { 52.0 },
            before: &before,
        });

        let to_xy = |b: f32, d: f32| match dir {
            Direction::Down => Point::new(b, d),
            Direction::Up => Point::new(b, -d),
            Direction::Right => Point::new(d, b),
            Direction::Left => Point::new(-d, b),
        };
        for (i, &(b, d)) in out.pos.iter().enumerate() {
            let s = self.items[i].size;
            self.items[i].rect = match dir {
                Direction::Down => Rect::new(b, d, s.width, s.height),
                Direction::Up => Rect::new(b, -(d + s.height), s.width, s.height),
                Direction::Right => Rect::new(d, b, s.width, s.height),
                Direction::Left => Rect::new(-(d + s.width), b, s.width, s.height),
            };
        }

        // Labels the engine did not place (on second and later self loops)
        // keep clear of nodes and of the labels it did.
        let mut taken: Vec<Rect> = self.items.iter().map(|it| it.rect).collect();
        taken.extend(label_lines.iter().zip(&out.label_at).filter_map(|(l, at)| {
            let ((_, size), &(b, d)) = (l.as_ref()?, at.as_ref()?);
            Some(Rect::from_center(to_xy(b, d), *size))
        }));
        let mut loops = vec![0; self.items.len()];
        for (k, &(a, b)) in edges[..drawn].iter().enumerate() {
            let route = &out.routes[k];
            let path = if route.is_empty() {
                loops[a] += 1;
                self_loop(&self.items[a].rect, self.items[a].look.shape, dir.is_horizontal(), loops[a] - 1)
            } else {
                smooth_layered(route, &to_xy)
            };
            let look = &looks[k];
            let label = label_lines[k].as_ref().map(|(lines, size)| {
                let center = match out.label_at[k] {
                    Some((b, d)) => to_xy(b, d),
                    None => {
                        let c = label_spot(&path, *size, &taken);
                        taken.push(Rect::from_center(c, *size));
                        c
                    }
                };
                self.label_box(lines.clone(), *size, center, look)
            });
            // Links name the document edge's own ends (they may be drawn inside an item).
            let (arrow, class, kind, from, to) = match sources[k] {
                Some(e) => (e.arrow, e.class.clone(), EdgeKind::Link, e.from.clone(), e.to.clone()),
                None => {
                    let (from, to) = (self.items[a].node.id.clone(), self.items[b].node.id.clone());
                    (Arrow::None, Vec::new(), EdgeKind::Branch, from, to)
                }
            };
            self.edges.push(EdgeBox {
                from,
                to,
                kind,
                path,
                color: look.color,
                width: look.width,
                line: look.line,
                arrow,
                label,
                class,
            });
        }
    }

    /// Participants are the frame's nodes, nesting ignored; messages are its
    /// edges. Lifelines come first so messages draw over them.
    fn sequence(&mut self) {
        let doc = self.doc;
        let (messages, sources): (Vec<(usize, usize)>, Vec<&Edge>) = doc
            .edges_in(&self.frame.id)
            .filter_map(|e| Some(((self.item_of(&e.from)?, self.item_of(&e.to)?), e)))
            .unzip();
        let looks: Vec<resolve::EdgeLook> =
            sources.iter().map(|e| resolve::edge(doc, self.theme, Some(e), EdgeRole::Message)).collect();
        let labels: Vec<Option<(Vec<String>, Size)>> = sources
            .iter()
            .zip(&looks)
            .map(|(e, look)| Some(self.label_size(e.label.as_deref()?, look.label_size)))
            .collect();
        let size: Vec<Size> = self.items.iter().map(|it| it.size).collect();
        let label_sizes: Vec<Option<Size>> = labels.iter().map(|l| l.as_ref().map(|(_, s)| *s)).collect();
        let out = sequence::layout(&sequence::Input { size: &size, messages: &messages, labels: &label_sizes });
        // Labels hide the lifelines they cross, so they take the frame's fill.
        let paper = if self.frame.implicit {
            self.theme.background
        } else {
            self.frame.style.fill.unwrap_or(self.theme.frame_fill)
        };

        for (it, (rect, &(top, bottom))) in self.items.iter_mut().zip(out.boxes.iter().zip(&out.lifelines)) {
            it.rect = *rect;
            self.edges.push(EdgeBox {
                from: it.node.id.clone(),
                to: it.node.id.clone(),
                kind: EdgeKind::Lifeline,
                path: Path::line(top, bottom),
                color: it.look.stroke,
                width: 1.0,
                line: LineStyle::Dashed,
                arrow: Arrow::None,
                label: None,
                class: Vec::new(),
            });
        }
        for (k, (path, e)) in out.paths.into_iter().zip(&sources).enumerate() {
            let look = &looks[k];
            let label = labels[k].as_ref().zip(out.label_at[k]).map(|((lines, size), center)| LabelBox {
                background: paper,
                ..self.label_box(lines.clone(), *size, center, look)
            });
            self.edges.push(EdgeBox {
                from: e.from.clone(),
                to: e.to.clone(),
                kind: EdgeKind::Link,
                path,
                color: look.color,
                width: look.width,
                line: look.line,
                arrow: e.arrow,
                label,
                class: e.class.clone(),
            });
        }
    }

    fn grid(&mut self) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        // After an edit, as many columns as before (the most in a row), so
        // the grid does not reflow when the count passes a square.
        let mut rows: Vec<f32> = self.before().iter().flatten().map(|p| p.y).collect();
        rows.sort_by(f32::total_cmp);
        let kept = rows.chunk_by(|a, b| (a - b).abs() < 0.5).map(<[f32]>::len).max();
        let cols =
            self.frame.columns.map(|c| c.max(1) as usize).or(kept).unwrap_or_else(|| (n as f32).sqrt().ceil() as usize);
        let rows = n.div_ceil(cols);
        let mut col_w = vec![0.0f32; cols];
        let mut row_h = vec![0.0f32; rows];
        for (i, it) in self.items.iter().enumerate() {
            col_w[i % cols] = col_w[i % cols].max(it.size.width);
            row_h[i / cols] = row_h[i / cols].max(it.size.height);
        }
        // Gaps wide and tall enough for the largest edge label to fit between neighbors.
        let doc = self.doc;
        let label = doc
            .edges_in(&self.frame.id)
            .filter_map(|e| {
                let look = resolve::edge(doc, self.theme, Some(e), EdgeRole::Link { secondary: false });
                Some(self.label_size(e.label.as_deref()?, look.label_size).1)
            })
            .fold(Size::new(0.0, 0.0), |a, b| Size::new(a.width.max(b.width), a.height.max(b.height)));
        let (col_gap, row_gap) = ((label.width + 16.0).max(28.0), (label.height + 16.0).max(28.0));
        let col_x: Vec<f32> = col_w
            .iter()
            .scan(0.0, |acc, w| {
                let x = *acc;
                *acc += w + col_gap;
                Some(x)
            })
            .collect();
        let row_y: Vec<f32> = row_h
            .iter()
            .scan(0.0, |acc, h| {
                let y = *acc;
                *acc += h + row_gap;
                Some(y)
            })
            .collect();
        for (i, it) in self.items.iter_mut().enumerate() {
            let (c, r) = (i % cols, i / cols);
            let center = Point::new(col_x[c] + col_w[c] / 2.0, row_y[r] + row_h[r] / 2.0);
            it.rect = Rect::from_center(center, it.size);
        }
        self.straight_edges();
    }

    fn free(&mut self) {
        let placed = Rect::bounding(
            self.items
                .iter()
                .filter_map(|it| it.node.at.map(|p| Rect::new(p.x, p.y, it.size.width, it.size.height)))
                .collect::<Vec<_>>()
                .iter(),
        );
        let mut x = placed.map_or(0.0, |r| r.x);
        let y = placed.map_or(0.0, |r| r.bottom() + 40.0);
        for it in &mut self.items {
            it.rect = match it.node.at {
                Some(p) => Rect::new(p.x, p.y, it.size.width, it.size.height),
                None => {
                    let r = Rect::new(x, y, it.size.width, it.size.height);
                    x += it.size.width + 24.0;
                    r
                }
            };
        }
        self.straight_edges();
    }

    fn force(&mut self) {
        let size: Vec<(f32, f32)> = self.items.iter().map(|it| (it.size.width, it.size.height)).collect();
        let mut edges: Vec<(usize, usize)> =
            (0..self.items.len()).flat_map(|p| self.children[p].iter().map(move |&c| (p, c))).collect();
        for e in self.doc.edges_in(&self.frame.id) {
            if let (Some(a), Some(b)) = (self.item_of(&e.from), self.item_of(&e.to)) {
                edges.push((a, b));
            }
        }
        edges.extend(self.lifted());
        let before: Vec<Option<(f32, f32)>> = self.before().iter().map(|p| p.map(|p| (p.x, p.y))).collect();
        let centers = force::layout_from(&size, &edges, &force::Spacing { edge: 48.0, gap: 20.0 }, &before);
        for (it, (cx, cy)) in self.items.iter_mut().zip(centers) {
            it.rect = Rect::from_center(Point::new(cx, cy), it.size);
        }
        self.straight_edges();
    }

    fn straight_edges(&mut self) {
        for p in 0..self.items.len() {
            for &c in &self.children[p] {
                let (a, b) = (self.node_box(p), self.node_box(c));
                let look = resolve::edge(self.doc, self.theme, None, EdgeRole::Nesting);
                self.edges.push(EdgeBox {
                    from: a.id.clone(),
                    to: b.id.clone(),
                    kind: EdgeKind::Branch,
                    path: curved_link(&a, &b, 0.0),
                    color: look.color,
                    width: look.width,
                    line: look.line,
                    arrow: Arrow::None,
                    label: None,
                    class: Vec::new(),
                });
            }
        }
        self.links(false);
    }

    fn label_size(&self, text: &str, font_size: f32) -> (Vec<String>, Size) {
        let lines = text::wrap(text, self.opts.max_text_width * 0.75, font_size, 400, self.measure);
        let lh = self.measure.line_height(font_size);
        let w = lines.iter().map(|l| self.measure.width(l, font_size, 400)).fold(0.0, f32::max);
        (lines.clone(), Size::new(w + 10.0, lh * lines.len() as f32 + 4.0))
    }

    fn label_box(&self, lines: Vec<String>, size: Size, center: Point, look: &resolve::EdgeLook) -> LabelBox {
        LabelBox {
            lines,
            center,
            size,
            font_size: look.label_size,
            line_height: self.measure.line_height(look.label_size),
            color: look.label_color,
            background: self.theme.background,
        }
    }

    fn node_box(&self, i: usize) -> NodeBox {
        let it = &self.items[i];
        NodeBox {
            id: it.node.id.clone(),
            frame: self.frame.id.clone(),
            label: it.node.label.clone(),
            lines: it.lines.clone(),
            rect: it.rect,
            shape: it.look.shape,
            fill: it.look.fill,
            stroke: it.look.stroke,
            stroke_width: it.look.stroke_width,
            line: it.look.line,
            text: it.look.text,
            font_size: it.look.font_size,
            font_weight: it.look.font_weight,
            line_height: it.line_height,
            depth: it.depth,
            branch: it.branch,
            class: it.node.class.clone(),
            note: it.node.note.clone(),
            link: it.node.link.clone(),
            typed: typed(self.doc, it.node, None, false, it.ink.clone()),
        }
    }

    /// Where a node drawn inside an item is, in frame coordinates.
    fn inner_box(&self, k: usize) -> NodeBox {
        let (host, x) = &self.inner[k];
        let h = &self.items[*host];
        NodeBox {
            id: x.node.id.clone(),
            frame: self.frame.id.clone(),
            label: x.node.label.clone(),
            lines: x.lines.clone(),
            rect: x.rect.translate(h.rect.x, h.rect.y),
            shape: x.look.shape,
            fill: x.look.fill,
            stroke: x.look.stroke,
            stroke_width: x.look.stroke_width,
            line: x.look.line,
            text: x.look.text,
            font_size: x.look.font_size,
            font_weight: x.look.font_weight,
            line_height: x.line_height,
            depth: h.depth + x.depth,
            branch: h.branch,
            class: x.node.class.clone(),
            note: x.node.note.clone(),
            link: x.node.link.clone(),
            typed: typed(self.doc, x.node, Some(x.host.clone()), x.row, x.ink.clone()),
        }
    }

    /// The drawn box of a node of this frame, inner ones included.
    fn any_box(&self, id: &NodeId) -> Option<NodeBox> {
        match self.index.get(id) {
            Some(&i) => Some(self.node_box(i)),
            None => self.inner_index.get(id).map(|&k| self.inner_box(k)),
        }
    }

    /// Edges to nodes drawn inside an item: the engine routed them to the
    /// item; now they go to the row or the contained node itself.
    fn reattach_inner(&mut self) {
        if self.inner.is_empty() {
            return;
        }
        // Every box of the frame, items then inner ones, for routing around.
        let n = self.items.len();
        let rects: Vec<Rect> =
            self.items.iter().map(|it| it.rect).chain((0..self.inner.len()).map(|k| self.inner_box(k).rect)).collect();
        let mut obstacles = route::Obstacles::new(rects);
        let slot = |id: &NodeId| self.index.get(id).copied().or_else(|| self.inner_index.get(id).map(|&k| n + k));
        // A box and every box it is drawn inside.
        let chain = |id: &NodeId| {
            let mut out = Vec::new();
            let mut cur = Some(id.clone());
            while let Some(i) = cur.as_ref().and_then(slot).filter(|_| out.len() <= MAX_FRAME_DEPTH) {
                out.push(i);
                cur = i.checked_sub(n).map(|k| self.inner[k].1.host.clone());
            }
            out
        };
        for k in 0..self.edges.len() {
            let e = &self.edges[k];
            if e.kind != EdgeKind::Link
                || !(self.inner_index.contains_key(&e.from) || self.inner_index.contains_key(&e.to))
            {
                continue;
            }
            if e.from == e.to {
                continue;
            }
            let end = |id: &NodeId| {
                let b = self.any_box(id)?;
                let host = b.typed.as_ref().and_then(|t| t.host.as_ref()).and_then(|h| self.any_box(h)).map(|h| h.rect);
                Some((b, host))
            };
            let (Some(a), Some(b)) = (end(&e.from), end(&e.to)) else { continue };
            let mut skip = chain(&e.from);
            skip.extend(chain(&e.to));
            let path = reattach_around(&a, &b, &mut obstacles, &skip);
            let e = &mut self.edges[k];
            e.path = path;
            if let Some(l) = &mut e.label {
                l.center = label_spot(&e.path, l.size, &[]);
            }
        }
    }

    /// Normalizes contents to the frame's origin and builds the frame box.
    fn finish(self) -> (FrameBox, Vec<NodeBox>, Vec<EdgeBox>, Vec<Point>) {
        let mut nodes: Vec<NodeBox> = (0..self.first_block).map(|i| self.node_box(i)).collect();
        let mut blocks: Vec<Rect> = self.items[self.first_block..].iter().map(|it| it.rect).collect();
        // Inner boxes after every item: drawn on top, hit first.
        nodes.extend((0..self.inner.len()).map(|k| self.inner_box(k)));
        let mut edges = self.edges;
        let mut content: Option<Rect> = None;
        let mut grow = |r: Rect| content = Some(content.map_or(r, |c| c.union(&r)));
        for n in &nodes {
            grow(n.rect);
        }
        for r in &blocks {
            grow(*r);
        }
        for e in &edges {
            grow(e.path.bounds());
            if let Some(l) = &e.label {
                grow(l.rect());
            }
        }
        let content = content.unwrap_or(Rect::new(0.0, 0.0, 120.0, 60.0));
        let visible = !self.frame.implicit;
        let label_size = (self.theme.font_size - 1.0).max(9.0);
        let title_h = if visible && self.frame.label.is_some() { label_size + 14.0 } else { 0.0 };
        let pad = if visible { FRAME_PAD } else { 0.0 };
        let (dx, dy) = (pad - content.x, pad + title_h - content.y);
        for n in &mut nodes {
            n.rect = n.rect.translate(dx, dy);
        }
        for r in &mut blocks {
            *r = r.translate(dx, dy);
        }
        for e in &mut edges {
            e.path = e.path.map(|p| p.offset(dx, dy));
            if let Some(l) = &mut e.label {
                l.center = l.center.offset(dx, dy);
            }
        }
        let style = &self.frame.style;
        let fill = style.fill.unwrap_or(self.theme.frame_fill);
        let fbox = FrameBox {
            id: self.frame.id.clone(),
            parent: None,
            depth: 0,
            label: self.frame.label.clone(),
            layout: self.kind,
            rect: Rect::new(0.0, 0.0, content.width + 2.0 * pad, content.height + 2.0 * pad + title_h),
            visible,
            fill,
            stroke: style.stroke.unwrap_or(self.theme.frame_stroke),
            label_color: style.text.unwrap_or(self.theme.frame_label),
            label_size,
        };
        (fbox, nodes, edges, blocks.iter().map(|r| Point::new(r.x, r.y)).collect())
    }
}

/// A mindmap's branches on the side each was on before: right when its
/// center was right of the root's. A new branch goes on the side of the one
/// before it (else after it). `None` without a previous layout of the root.
fn kept_sides(kids: &[usize], root: usize, before: &[Option<Point>]) -> Option<(Vec<usize>, Vec<usize>)> {
    let rx = before.get(root).copied().flatten()?.x;
    let was: Vec<Option<bool>> = kids.iter().map(|&k| before[k].map(|p| p.x >= rx)).collect();
    if was.iter().all(Option::is_none) {
        return None;
    }
    let side = |k: usize| was[k].or_else(|| was[..k].iter().rev().chain(&was[k + 1..]).find_map(|s| *s));
    let side = |k: usize| side(k).unwrap_or(true);
    let (right, left): (Vec<(usize, &usize)>, _) = kids.iter().enumerate().partition(|&(k, _)| side(k));
    Some((right.into_iter().map(|(_, &c)| c).collect(), left.into_iter().map(|(_, &c)| c).collect()))
}

/// Outer size of a node whose text block is `tw × th`.
fn node_size(shape: Shape, tw: f32, th: f32, font_size: f32) -> Size {
    let (px, py) = (font_size, font_size * 0.65);
    let base_w = tw.max(font_size * 1.5);
    let s = match shape {
        Shape::Rect | Shape::Rounded => Size::new(base_w + 2.0 * px, th + 2.0 * py),
        Shape::Pill => {
            let h = th + 2.0 * py;
            Size::new(base_w + h * 0.9 + px * 0.5, h)
        }
        Shape::Ellipse => Size::new(base_w * 1.42 + px, th * 1.42 + py),
        Shape::Circle => {
            let d = (base_w.max(th) * 1.42 + px).max(th * 2.2);
            Size::new(d, d)
        }
        Shape::Diamond => {
            let w = base_w + (th + py) / 0.62 + px;
            Size::new(w, (w * 0.62).max(th + 3.0 * py))
        }
        Shape::Hexagon => {
            let h = th + 2.0 * py;
            Size::new(base_w + 2.0 * px + h * 0.7, h)
        }
        Shape::Text => Size::new(base_w + 8.0, th + 6.0),
    };
    Size::new(s.width.ceil(), s.height.ceil())
}

/// The `typed` part of a node's box, or `None` for a plain node.
fn typed(doc: &Document, n: &Node, host: Option<NodeId>, row: bool, ink: Vec<Ink>) -> Option<Box<Typed>> {
    let d = doc.decor.get(&n.id);
    if n.ty.is_none() && host.is_none() && d.is_none() && ink.is_empty() {
        return None;
    }
    Some(Box::new(Typed {
        ty: n.ty.as_ref().map(|t| t.as_str().to_string()),
        host,
        row,
        card: d.is_some_and(|d| d.card),
        container: d.is_some_and(|d| d.container),
        state: d.map_or(DecorState::Normal, |d| d.state),
        ink,
    }))
}

/// An edge end's box and, for a node drawn inside another, its host's.
type InnerEnd = (NodeBox, Option<Rect>);

/// A connection between two boxes, one of them (at least) drawn inside
/// another. With a card row at either end it runs ER style: out of the
/// sides, level with the row, facing sides when the boxes sit apart and
/// both right sides when they are stacked. Else outline to outline.
fn reattach(a: &InnerEnd, b: &InnerEnd) -> Path {
    let is_row = |e: &InnerEnd| e.0.typed.as_ref().is_some_and(|t| t.row);
    if !is_row(a) && !is_row(b) {
        let (ca, cb) = (a.0.rect.center(), b.0.rect.center());
        return Path::line(path::boundary(&a.0.rect, a.0.shape, cb), path::boundary(&b.0.rect, b.0.shape, ca));
    }
    // The horizontal span each end leaves from: a row's card, else the box.
    let span = |e: &InnerEnd| if is_row(e) { e.1.unwrap_or(e.0.rect) } else { e.0.rect };
    let (sa, sb) = (span(a), span(b));
    let (ra, rb) = if sa.right() < sb.x {
        (true, false)
    } else if sb.right() < sa.x {
        (false, true)
    } else {
        (true, true)
    };
    let side = |r: Rect, right: bool, y: f32| {
        (Point::new(if right { r.right() } else { r.x }, y), if right { 1.0 } else { -1.0 })
    };
    let (p, dp) = side(sa, ra, a.0.rect.center().y);
    let (q, dq) = side(sb, rb, b.0.rect.center().y);
    let k = (p.distance(q) * 0.4).max(36.0);
    Path { start: p, segs: vec![Seg::Cubic(p.offset(dp * k, 0.0), q.offset(dq * k, 0.0), q)] }
}

/// [`reattach`], but around the boxes in its way when neither end is a
/// card row (rows keep their ER-style connectors). `skip` holds both ends
/// and every box they are drawn inside, which the edge may cross.
fn reattach_around(a: &InnerEnd, b: &InnerEnd, obstacles: &mut route::Obstacles, skip: &[usize]) -> Path {
    let path = reattach(a, b);
    let is_row = |e: &InnerEnd| e.0.typed.as_ref().is_some_and(|t| t.row);
    if is_row(a) || is_row(b) || !obstacles.hits(&path, skip) {
        return path;
    }
    route::route(obstacles, skip, (&a.0.rect, a.0.shape), (&b.0.rect, b.0.shape), 0).unwrap_or(path)
}

/// A curve (or straight line when `bend` is 0) between two node outlines.
fn curved_link(a: &NodeBox, b: &NodeBox, bend: f32) -> Path {
    let (ca, cb) = (a.rect.center(), b.rect.center());
    let len = ca.distance(cb).max(1.0);
    let normal = Point::new(-(cb.y - ca.y) / len, (cb.x - ca.x) / len);
    let off = len * bend;
    let ctrl = ca.lerp(cb, 0.5).offset(normal.x * off, normal.y * off);
    let start = path::boundary(&a.rect, a.shape, if bend == 0.0 { cb } else { ctrl });
    let end = path::boundary(&b.rect, b.shape, if bend == 0.0 { ca } else { ctrl });
    if bend == 0.0 {
        return Path::line(start, end);
    }
    let c1 = start.lerp(ctrl, 0.66);
    let c2 = end.lerp(ctrl, 0.66);
    Path { start, segs: vec![Seg::Cubic(c1, c2, end)] }
}

/// The `nth` self loop on a node: out of its right side, or its bottom when
/// `below`. The first reaches `layered::LOOP_REACH` out of the node; later
/// ones tilt toward either corner, then reach further, so none coincide.
fn self_loop(r: &Rect, shape: Shape, below: bool, nth: usize) -> Path {
    let out = layered::LOOP_REACH / 0.75 * (1.0 + 0.5 * (nth / 3) as f32);
    let tilt = [0.0, -1.0, 1.0][nth % 3] * std::f32::consts::FRAC_PI_4;
    // Side midpoint, outward direction, direction along the side, side length.
    let (mid, d, t, side) = if below {
        (Point::new(r.center().x, r.bottom()), Point::new(0.0, 1.0), Point::new(1.0, 0.0), r.width)
    } else {
        (Point::new(r.right(), r.center().y), Point::new(1.0, 0.0), Point::new(0.0, 1.0), r.height)
    };
    let h = (side * 0.3).min(10.0);
    let (sin, cos) = tilt.sin_cos();
    let rot = |p: Point| Point::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos);
    let along = |p: Point, k: f32, v: Point| p.offset(v.x * k, v.y * k);
    let (dt, tt) = (rot(d), rot(t));
    // Tilted loops wrap the corner they lean to.
    let anchor = along(mid, (dt.x * t.x + dt.y * t.y) * std::f32::consts::SQRT_2 * side / 2.0, t);
    let tip = along(anchor, out, dt);
    let (mut start, mut end) = (along(anchor, -h, tt), along(anchor, h, tt));
    if tilt != 0.0 {
        start = path::boundary(r, shape, start);
        end = path::boundary(r, shape, end);
    }
    Path {
        start,
        segs: vec![Seg::Cubic(along(along(tip, -h, tt), -22.0, tt), along(along(tip, h, tt), 22.0, tt), end)],
    }
}

/// `path` as is if it crosses no obstacle but those in `skip` (its own ends
/// and frames), else a route around them.
fn avoid(path: Path, obstacles: &mut route::Obstacles, skip: &[usize], a: &NodeBox, b: &NodeBox, lane: usize) -> Path {
    if !obstacles.hits(&path, skip) {
        return path;
    }
    route::route(obstacles, skip, (&a.rect, a.shape), (&b.rect, b.shape), lane).unwrap_or(path)
}

/// Counts the edges between each pair of nodes so parallel ones can take
/// separate lanes.
#[derive(Default)]
struct Lanes(HashMap<(usize, usize), usize>);

impl Lanes {
    /// How many edges between `a` and `b` (either way) came before this one,
    /// and the extra bend that gives it a lane of its own: 0, then
    /// alternating sides of the first, further out each time.
    fn next(&mut self, a: usize, b: usize) -> (usize, f32) {
        let seen = self.0.entry((a.min(b), a.max(b))).or_default();
        let n = *seen;
        *seen += 1;
        let side = n.div_ceil(2) as f32 * if n % 2 == 1 { 1.0 } else { -1.0 };
        // Bends are relative to the edge's direction; lanes to the pair's.
        (n, if a > b { -side } else { side } * 0.15)
    }
}

/// Where to center a label of `size` on `path` so it overlaps none of
/// `taken`: the midpoint if free, else the nearest free spot toward either
/// end, else the same just beside the path. Falls back to the midpoint.
fn label_spot(path: &Path, size: Size, taken: &[Rect]) -> Point {
    let (a, b) = (path.start, path.end());
    let len = a.distance(b).max(1.0);
    let normal = Point::new(-(b.y - a.y) / len, (b.x - a.x) / len);
    let beside = normal.x.abs() * size.width / 2.0 + normal.y.abs() * size.height / 2.0 + 4.0;
    for shift in [0.0, beside, -beside] {
        for frac in [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8] {
            let c = path.at(frac).offset(normal.x * shift, normal.y * shift);
            if !taken.iter().any(|r| r.intersects(&Rect::from_center(c, size))) {
                return c;
            }
        }
    }
    path.midpoint()
}

fn link_box(
    e: &Edge,
    path: Path,
    look: resolve::EdgeLook,
    theme: &Theme,
    m: &dyn TextMeasure,
    opts: &Options,
    taken: &[Rect],
) -> EdgeBox {
    let label = e.label.as_deref().map(|text| {
        let lines = text::wrap(text, opts.max_text_width * 0.75, look.label_size, 400, m);
        let lh = m.line_height(look.label_size);
        let w = lines.iter().map(|l| m.width(l, look.label_size, 400)).fold(0.0, f32::max);
        let size = Size::new(w + 10.0, lh * lines.len() as f32 + 4.0);
        LabelBox {
            size,
            lines,
            center: label_spot(&path, size, taken),
            font_size: look.label_size,
            line_height: lh,
            color: look.label_color,
            background: theme.background,
        }
    });
    EdgeBox {
        from: e.from.clone(),
        to: e.to.clone(),
        kind: EdgeKind::Link,
        path,
        color: look.color,
        width: look.width,
        line: look.line,
        arrow: e.arrow,
        label,
        class: e.class.clone(),
    }
}

/// Smooth curve through layered route points. Ends leave and enter along
/// the depth axis so edges meet nodes head-on; interior points use
/// Catmull-Rom tangents. Nearly vertical hops stay straight lines.
fn smooth_layered(route: &[(f32, f32)], to_xy: &dyn Fn(f32, f32) -> Point) -> Path {
    let n = route.len();
    let start = to_xy(route[0].0, route[0].1);
    if n == 2 && (route[0].0 - route[1].0).abs() < 12.0 {
        return Path::line(start, to_xy(route[1].0, route[1].1));
    }
    let tangent = |i: usize| -> (f32, f32) {
        if i == 0 {
            (0.0, route[1].1 - route[0].1)
        } else if i == n - 1 {
            (0.0, route[n - 1].1 - route[n - 2].1)
        } else {
            ((route[i + 1].0 - route[i - 1].0) / 2.0, (route[i + 1].1 - route[i - 1].1) / 2.0)
        }
    };
    let mut segs = Vec::with_capacity(n - 1);
    for i in 0..n - 1 {
        let (p, q) = (route[i], route[i + 1]);
        let (tp, tq) = (tangent(i), tangent(i + 1));
        let k = if i == 0 || i == n - 2 { 0.5 } else { 1.0 / 3.0 };
        let (kp, kq) = (if i == 0 { 0.5 } else { k }, if i == n - 2 { 0.5 } else { k });
        segs.push(Seg::Cubic(
            to_xy(p.0 + tp.0 * kp, p.1 + tp.1 * kp),
            to_xy(q.0 - tq.0 * kq, q.1 - tq.1 * kq),
            to_xy(q.0, q.1),
        ));
    }
    Path { start, segs }
}

/// Orthogonal polyline with rounded corners.
fn rounded_polyline(pts: &[Point], radius: f32) -> Path {
    let mut segs = Vec::new();
    for i in 1..pts.len() - 1 {
        let (p, c, n) = (pts[i - 1], pts[i], pts[i + 1]);
        let r = radius.min(p.distance(c) / 2.0).min(c.distance(n) / 2.0);
        if r < 0.5 {
            segs.push(Seg::Line(c));
            continue;
        }
        let a = c.lerp(p, r / p.distance(c));
        let b = c.lerp(n, r / c.distance(n));
        segs.push(Seg::Line(a));
        segs.push(Seg::Cubic(a.lerp(c, 0.55), b.lerp(c, 0.55), b));
    }
    segs.push(Seg::Line(*pts.last().unwrap()));
    Path { start: pts[0], segs }
}

#[cfg(test)]
mod tests;
