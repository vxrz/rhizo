//! Turns a [`Document`] into a [`Diagram`]: positioned, styled boxes and
//! routed edge paths, ready for any renderer.
//!
//! Each frame runs its own engine ([`tidy`] for trees and mindmaps,
//! [`layered`] for graphs, plus grid and free placement). Frames are then
//! placed on one canvas and edges between frames are routed last.

pub mod layered;
pub mod path;
pub mod resolve;
pub mod text;
pub mod tidy;

use std::collections::HashMap;

use rhizo_core::{
    Arrow, Color, Direction, Document, Edge, Frame, FrameId, LayoutKind, LineStyle, Node, NodeId, Point, Rect, Shape,
    Size, Theme,
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
    if doc.edges_in(&frame.id).next().is_some() {
        LayoutKind::Layered
    } else if doc.nodes_in(&frame.id).any(|n| !n.children.is_empty()) {
        LayoutKind::Mindmap
    } else if doc.nodes_in(&frame.id).any(|n| n.at.is_some()) {
        LayoutKind::Free
    } else {
        LayoutKind::Grid
    }
}

pub fn layout(doc: &Document, theme: &Theme, measure: &dyn TextMeasure) -> Diagram {
    layout_with(doc, theme, measure, &Options::default())
}

pub fn layout_with(doc: &Document, theme: &Theme, measure: &dyn TextMeasure, opts: &Options) -> Diagram {
    let mut frames = Vec::new();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut cursor = 0.0f32;
    let mut placed_any = false;

    for frame in &doc.frames {
        let mut fl = FrameLayout::new(doc, theme, measure, opts, frame);
        if fl.items.is_empty() && frame.implicit {
            continue;
        }
        fl.run();
        let (mut fbox, fnodes, fedges) = fl.finish();
        let origin = match frame.at {
            Some(p) => p,
            None => {
                let p = Point::new(if placed_any { cursor + FRAME_GAP } else { cursor }, 0.0);
                cursor = p.x + fbox.rect.width;
                placed_any = true;
                p
            }
        };
        fbox.rect = fbox.rect.translate(origin.x, origin.y);
        frames.push(fbox);
        nodes.extend(fnodes.into_iter().map(|mut n| {
            n.rect = n.rect.translate(origin.x, origin.y);
            n
        }));
        edges.extend(fedges.into_iter().map(|mut e| {
            e.path = e.path.map(|p| p.offset(origin.x, origin.y));
            if let Some(l) = &mut e.label {
                l.center = l.center.offset(origin.x, origin.y);
            }
            e
        }));
    }

    // Edges whose ends are in different frames.
    let index: HashMap<&NodeId, usize> = nodes.iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
    for e in &doc.edges {
        let (Some(a), Some(b)) = (doc.node(&e.from), doc.node(&e.to)) else { continue };
        if a.frame == b.frame {
            continue;
        }
        let (Some(&ia), Some(&ib)) = (index.get(&e.from), index.get(&e.to)) else { continue };
        let look = resolve::edge(doc, theme, Some(e), EdgeRole::Link { secondary: false });
        let path = curved_link(&nodes[ia], &nodes[ib], 0.12);
        edges.push(link_box(e, path, look, theme, measure, opts));
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
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    edges: Vec<EdgeBox>,
}

impl<'a> FrameLayout<'a> {
    fn new(
        doc: &'a Document,
        theme: &'a Theme,
        measure: &'a dyn TextMeasure,
        opts: &'a Options,
        frame: &'a Frame,
    ) -> Self {
        let kind = effective_layout(doc, frame);
        let list: Vec<&Node> = doc.nodes_in(&frame.id).collect();
        let index: HashMap<&NodeId, usize> = list.iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
        let children: Vec<Vec<usize>> =
            list.iter().map(|n| n.children.iter().filter_map(|c| index.get(c).copied()).collect()).collect();
        let roots: Vec<usize> = (0..list.len()).filter(|&i| list[i].parent.is_none()).collect();

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

        let items = list
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let cx =
                    NodeCtx { layout: kind, depth: depth[i], branch: branch[i], has_children: !children[i].is_empty() };
                let look = resolve::node(doc, theme, n, &cx);
                let max_w = if depth[i] == 0 && !children[i].is_empty() {
                    opts.max_text_width * 1.2
                } else {
                    opts.max_text_width
                };
                let lines = text::wrap(&n.label, max_w, look.font_size, look.font_weight, measure);
                let line_height = measure.line_height(look.font_size);
                let tw = lines.iter().map(|l| measure.width(l, look.font_size, look.font_weight)).fold(0.0, f32::max);
                let th = line_height * lines.len() as f32;
                let size = node_size(look.shape, tw, th, look.font_size);
                Item {
                    node: n,
                    look,
                    lines,
                    line_height,
                    size,
                    depth: depth[i],
                    branch: branch[i],
                    rect: Rect::default(),
                }
            })
            .collect();

        FrameLayout { doc, theme, measure, opts, frame, kind, items, index, children, roots, edges: Vec::new() }
    }

    fn run(&mut self) {
        match self.kind {
            LayoutKind::Mindmap => self.mindmap(),
            LayoutKind::Tree => self.tree(),
            LayoutKind::Layered => self.layered(),
            LayoutKind::Grid => self.grid(),
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

    fn mindmap(&mut self) {
        let spacing = tidy::Spacing { level: 56.0, sibling: 10.0 };
        if let Some(dir) = self.frame.direction {
            let sizes = self.sizes_for(dir);
            let pos = tidy::layout(
                &sizes,
                &self.children,
                &self.roots,
                &tidy::Spacing { level: 56.0, sibling: if dir.is_horizontal() { 10.0 } else { 20.0 } },
            );
            let all: Vec<usize> = (0..self.items.len()).collect();
            self.place_tree(&pos, &all, dir);
        } else {
            let mut y_cursor = 0.0f32;
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
                let (right, left) = kids.split_at(split);
                let mut children = self.children.clone();
                let mut members = Vec::new();

                children[r] = right.to_vec();
                let sizes = self.sizes_for(Direction::Right);
                let pos_r = tidy::layout(&sizes, &children, &[r], &spacing);
                let right_members: Vec<usize> =
                    std::iter::once(r).chain(right.iter().flat_map(|&k| self.subtree(k))).collect();
                self.place_tree(&pos_r, &right_members, Direction::Right);
                let root_rect = self.items[r].rect;
                members.extend(right_members);

                if !left.is_empty() {
                    children[r] = left.iter().rev().copied().collect();
                    let pos_l = tidy::layout(&sizes, &children, &[r], &spacing);
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
        let pos = tidy::layout(&sizes, &self.children, &self.roots, &tidy::Spacing { level: 44.0, sibling });
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

    /// Explicit edges inside the frame, drawn as gentle curves.
    fn links(&mut self, secondary: bool) {
        let doc = self.doc;
        for e in doc.edges_in(&self.frame.id) {
            let (Some(&a), Some(&b)) = (self.index.get(&e.from), self.index.get(&e.to)) else { continue };
            let look = resolve::edge(doc, self.theme, Some(e), EdgeRole::Link { secondary });
            let (na, nb) = (self.node_box(a), self.node_box(b));
            let path =
                if a == b { self_loop(&na.rect) } else { curved_link(&na, &nb, if secondary { 0.18 } else { 0.0 }) };
            let eb = link_box(e, path, look, self.theme, self.measure, self.opts);
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
            if let (Some(&a), Some(&b)) = (self.index.get(&e.from), self.index.get(&e.to)) {
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
        let labels: Vec<Option<(f32, f32)>> = label_lines
            .iter()
            .map(|l| {
                l.as_ref().map(|(_, s)| if dir.is_horizontal() { (s.height, s.width) } else { (s.width, s.height) })
            })
            .collect();

        let out = layered::layout(&layered::Input {
            size: &size,
            shape: &shape,
            edges: &edges,
            labels: &labels,
            node_sep: if dir.is_horizontal() { 18.0 } else { 28.0 },
            rank_sep: if dir.is_horizontal() { 64.0 } else { 52.0 },
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

        for (k, &(a, b)) in edges.iter().enumerate() {
            let route = &out.routes[k];
            let path = if route.is_empty() { self_loop(&self.items[a].rect) } else { smooth_layered(route, &to_xy) };
            let look = &looks[k];
            let label = label_lines[k].as_ref().map(|(lines, size)| {
                let center = match out.label_at[k] {
                    Some((b, d)) => to_xy(b, d),
                    None => path.midpoint(),
                };
                self.label_box(lines.clone(), *size, center, look)
            });
            let (arrow, class, kind) = match sources[k] {
                Some(e) => (e.arrow, e.class.clone(), EdgeKind::Link),
                None => (Arrow::End, Vec::new(), EdgeKind::Branch),
            };
            self.edges.push(EdgeBox {
                from: self.items[a].node.id.clone(),
                to: self.items[b].node.id.clone(),
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

    fn grid(&mut self) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        let cols = self.frame.columns.map(|c| c.max(1) as usize).unwrap_or_else(|| (n as f32).sqrt().ceil() as usize);
        let rows = n.div_ceil(cols);
        let mut col_w = vec![0.0f32; cols];
        let mut row_h = vec![0.0f32; rows];
        for (i, it) in self.items.iter().enumerate() {
            col_w[i % cols] = col_w[i % cols].max(it.size.width);
            row_h[i / cols] = row_h[i / cols].max(it.size.height);
        }
        let gap = 28.0;
        let col_x: Vec<f32> = col_w
            .iter()
            .scan(0.0, |acc, w| {
                let x = *acc;
                *acc += w + gap;
                Some(x)
            })
            .collect();
        let row_y: Vec<f32> = row_h
            .iter()
            .scan(0.0, |acc, h| {
                let y = *acc;
                *acc += h + gap;
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
        }
    }

    /// Normalizes contents to the frame's origin and builds the frame box.
    fn finish(self) -> (FrameBox, Vec<NodeBox>, Vec<EdgeBox>) {
        let mut nodes: Vec<NodeBox> = (0..self.items.len()).map(|i| self.node_box(i)).collect();
        let mut edges = self.edges;
        let mut content: Option<Rect> = None;
        let mut grow = |r: Rect| content = Some(content.map_or(r, |c| c.union(&r)));
        for n in &nodes {
            grow(n.rect);
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
            label: self.frame.label.clone(),
            layout: self.kind,
            rect: Rect::new(0.0, 0.0, content.width + 2.0 * pad, content.height + 2.0 * pad + title_h),
            visible,
            fill,
            stroke: style.stroke.unwrap_or(self.theme.frame_stroke),
            label_color: style.text.unwrap_or(self.theme.frame_label),
            label_size,
        };
        (fbox, nodes, edges)
    }
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

fn self_loop(r: &Rect) -> Path {
    let (x, cy) = (r.right(), r.center().y);
    let h = (r.height * 0.3).min(10.0);
    Path {
        start: Point::new(x, cy - h),
        segs: vec![Seg::Cubic(
            Point::new(x + 36.0, cy - h - 22.0),
            Point::new(x + 36.0, cy + h + 22.0),
            Point::new(x, cy + h),
        )],
    }
}

fn link_box(
    e: &Edge,
    path: Path,
    look: resolve::EdgeLook,
    theme: &Theme,
    m: &dyn TextMeasure,
    opts: &Options,
) -> EdgeBox {
    let label = e.label.as_deref().map(|text| {
        let lines = text::wrap(text, opts.max_text_width * 0.75, look.label_size, 400, m);
        let lh = m.line_height(look.label_size);
        let w = lines.iter().map(|l| m.width(l, look.label_size, 400)).fold(0.0, f32::max);
        LabelBox {
            size: Size::new(w + 10.0, lh * lines.len() as f32 + 4.0),
            lines,
            center: path.midpoint(),
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
