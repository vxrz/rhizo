//! AST ⇄ document model.

use std::collections::{HashMap, HashSet};

use rhizo_core::{Document, Edge, Frame, FrameId, LayoutKind, Node, NodeId, Point, Theme, slugify};

use crate::ast::{EdgeDef, File, FrameDef, NodeDef, Span};
use crate::diag::{Diagnostic, suggest_name};

pub const IMPLICIT_FRAME: &str = "main";

/// Resolves ids, label references and styles. The document is complete even
/// when there are errors (bad edges are dropped), so tools can still show
/// something while the user is typing.
pub fn lower(file: &File) -> (Document, Vec<Diagnostic>) {
    let mut cx = Lower::default();
    cx.run(file);
    let mut seen = HashSet::new();
    cx.diags.retain(|d| seen.insert((d.message.clone(), d.span)));
    (cx.doc, cx.diags)
}

#[derive(Default)]
struct Lower {
    doc: Document,
    diags: Vec<Diagnostic>,
    /// Every explicit id, collected up front so derived ids never take one.
    reserved: HashSet<String>,
    taken: HashSet<String>,
    by_label: HashMap<String, Vec<NodeId>>,
}

impl Lower {
    fn run(&mut self, file: &File) {
        self.doc.title = file.title.clone();
        if let Some(t) = &file.theme {
            if Theme::by_name(t).is_none() {
                let mut d = Diagnostic::warning(format!("unknown theme `{t}`, using `light`"), None);
                d.help = Some(format!("available: {}", Theme::NAMES.join(", ")));
                self.diags.push(d);
            }
        }
        self.doc.theme = file.theme.clone();
        self.doc.styles = file.styles.clone();

        fn reserve(nodes: &[NodeDef], out: &mut Vec<(String, Span)>) {
            for n in nodes {
                if let Some(id) = &n.id {
                    out.push((id.clone(), n.span));
                }
                reserve(&n.children, out);
            }
        }
        let mut explicit = Vec::new();
        reserve(&file.nodes, &mut explicit);
        for f in &file.frames {
            reserve(&f.nodes, &mut explicit);
        }
        for (id, span) in explicit {
            if !self.reserved.insert(id.clone()) {
                self.diags.push(Diagnostic::error(format!("node id `{id}` is used more than once"), span));
            }
        }

        let mut frame_ids = HashSet::new();
        let has_implicit = !file.nodes.is_empty() || file.frames.is_empty();
        let mut frames: Vec<(FrameId, &FrameDef)> = Vec::new();
        for (i, f) in file.frames.iter().enumerate() {
            let base = f.id.clone().or_else(|| f.label.as_deref().map(slugify)).unwrap_or(format!("frame-{}", i + 1));
            let id = unique(&base, |c| frame_ids.contains(c) || (has_implicit && c == IMPLICIT_FRAME));
            if f.id.as_deref().is_some_and(|given| given != id) {
                self.diags.push(Diagnostic::error(format!("frame id `{base}` is used more than once"), f.span));
            }
            frame_ids.insert(id.clone());
            frames.push((FrameId(id), f));
        }

        if has_implicit {
            let mut frame = Frame::new(IMPLICIT_FRAME);
            frame.implicit = true;
            frame.layout = file.layout.unwrap_or_default();
            frame.direction = file.direction;
            self.doc.frames.push(frame);
            let fid = FrameId::from(IMPLICIT_FRAME);
            for n in &file.nodes {
                self.node(n, &fid, None);
            }
        } else if file.layout.is_some() || file.direction.is_some() {
            self.diags.push(Diagnostic::warning(
                "top-level `layout`/`direction` only apply to nodes outside frames, and there are none",
                None,
            ));
        }

        for (fid, f) in &frames {
            let frame = Frame {
                id: fid.clone(),
                label: f.label.clone(),
                layout: f.layout.unwrap_or_default(),
                direction: f.direction,
                at: point(f.x, f.y),
                columns: f.columns,
                style: f.style.clone(),
                implicit: false,
            };
            self.doc.frames.push(frame);
            for n in &f.nodes {
                self.node(n, fid, None);
            }
        }

        for (fid, f) in &frames {
            for e in &f.edges {
                self.edge(e, Some(fid));
            }
        }
        for e in &file.edges {
            self.edge(e, None);
        }
        self.check_classes();
    }

    fn node(&mut self, def: &NodeDef, frame: &FrameId, parent: Option<&NodeId>) {
        let id = match &def.id {
            Some(id) => {
                if !self.taken.insert(id.clone()) {
                    return; // duplicate, already reported
                }
                id.clone()
            }
            None => {
                let id = unique(&slugify(&def.label), |c| self.reserved.contains(c) || self.taken.contains(c));
                self.taken.insert(id.clone());
                id
            }
        };
        let mut node = Node::new(id.clone(), def.label.clone(), frame.clone());
        node.parent = parent.cloned();
        node.class = def.class.clone();
        node.style = def.style.clone();
        node.at = point(def.x, def.y);
        node.note = def.note.clone();
        node.link = def.link.clone();
        node.tags = def.tags.clone();
        let nid = NodeId(id);
        self.by_label.entry(def.label.clone()).or_default().push(nid.clone());
        self.doc.add_node(node).expect("ids are unique and parents exist");
        for c in &def.children {
            self.node(c, frame, Some(&nid));
        }
    }

    /// Resolves a reference: exact id, then exact label, then the slug of it.
    /// `Err(true)` means an error was reported; `Err(false)` means nothing
    /// matches, so the caller creates the node (warning first if the name is
    /// suspiciously close to an existing one).
    fn resolve(&mut self, r: &str, span: Span) -> Result<NodeId, bool> {
        let id = NodeId::from(r);
        if self.doc.nodes.contains_key(&id) {
            return Ok(id);
        }
        if let Some(ids) = self.by_label.get(r) {
            if ids.len() == 1 {
                return Ok(ids[0].clone());
            }
            let list: Vec<&str> = ids.iter().map(|i| i.as_str()).collect();
            self.diags.push(
                Diagnostic::error(format!("`{r}` is ambiguous: {} nodes have that label", ids.len()), span)
                    .with_help(format!("refer to one by id: {}", list.join(", "))),
            );
            return Err(true);
        }
        let slug = NodeId(slugify(r));
        if self.doc.nodes.contains_key(&slug) {
            return Ok(slug);
        }
        let candidates: Vec<&str> =
            self.doc.nodes.keys().map(|k| k.as_str()).chain(self.by_label.keys().map(String::as_str)).collect();
        if let Some(close) = suggest_name(r, candidates) {
            let help = format!("did you mean `{close}`? If not, declare it with `node` to silence this");
            self.diags
                .push(Diagnostic::warning(format!("`{r}` is new, so a node was created for it"), span).with_help(help));
        }
        Err(false)
    }

    /// Resolves an endpoint, creating the node if the reference is new. New
    /// nodes go in the edge's frame, or the other endpoint's frame for
    /// top-level edges.
    fn endpoint(&mut self, r: &str, span: Span, frame: Option<&FrameId>, other: Option<&NodeId>) -> Option<NodeId> {
        match self.resolve(r, span) {
            Ok(id) => Some(id),
            Err(true) => None,
            Err(false) => {
                let frame = frame
                    .cloned()
                    .or_else(|| other.and_then(|o| self.doc.node(o)).map(|n| n.frame.clone()))
                    .unwrap_or_else(|| self.implicit_frame());
                let def = NodeDef { span, ..NodeDef::new(r) };
                self.node(&def, &frame, None);
                self.resolve(r, span).ok()
            }
        }
    }

    fn implicit_frame(&mut self) -> FrameId {
        let id = FrameId::from(IMPLICIT_FRAME);
        if self.doc.frame(&id).is_none() {
            let mut frame = Frame::new(IMPLICIT_FRAME);
            frame.implicit = true;
            self.doc.frames.insert(0, frame);
        }
        id
    }

    fn edge(&mut self, def: &EdgeDef, frame: Option<&FrameId>) {
        let from = self.endpoint(&def.from, def.span, frame, None);
        let to = self.endpoint(&def.to, def.span, frame, from.as_ref());
        let (Some(from), Some(to)) = (from, to) else { return };
        let edge = Edge {
            from,
            to,
            label: def.label.clone(),
            arrow: def.arrow.unwrap_or_default(),
            class: def.class.clone(),
            style: def.style.clone(),
        };
        self.doc.add_edge(edge).expect("endpoints resolved");
    }

    fn check_classes(&mut self) {
        let used = self
            .doc
            .nodes
            .values()
            .flat_map(|n| n.class.iter())
            .chain(self.doc.edges.iter().flat_map(|e| e.class.iter()));
        let mut missing: Vec<&String> = used.filter(|c| !self.doc.styles.contains_key(*c)).collect();
        missing.sort();
        missing.dedup();
        let missing: Vec<String> = missing.into_iter().cloned().collect();
        for c in missing {
            let mut d = Diagnostic::warning(format!("class `{c}` has no style"), None);
            d.help = Some(format!("define it: style \"{c}\" fill=\"#...\""));
            self.diags.push(d);
        }
    }
}

fn point(x: Option<f32>, y: Option<f32>) -> Option<Point> {
    match (x, y) {
        (None, None) => None,
        (x, y) => Some(Point::new(x.unwrap_or(0.0), y.unwrap_or(0.0))),
    }
}

fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}-{i}")).find(|c| !taken(c)).unwrap()
}

/// The inverse of [`lower`]: nests children, drops ids that would be derived
/// anyway, and puts each edge in the frame that holds both ends.
pub fn raise(doc: &Document) -> File {
    let mut file =
        File { title: doc.title.clone(), theme: doc.theme.clone(), styles: doc.styles.clone(), ..Default::default() };
    let node_def = |id: &NodeId| -> NodeDef { raise_node(doc, id) };
    for frame in &doc.frames {
        let roots: Vec<NodeDef> = doc.roots(&frame.id).map(|n| node_def(&n.id)).collect();
        if frame.implicit {
            file.layout = Some(frame.layout).filter(|l| *l != LayoutKind::Auto);
            file.direction = frame.direction;
            file.nodes = roots;
        } else {
            file.frames.push(FrameDef {
                id: Some(frame.id.0.clone())
                    .filter(|id| Some(id.as_str()) != frame.label.as_deref().map(slugify).as_deref()),
                label: frame.label.clone(),
                layout: Some(frame.layout).filter(|l| *l != LayoutKind::Auto),
                direction: frame.direction,
                x: frame.at.map(|p| p.x),
                y: frame.at.map(|p| p.y),
                columns: frame.columns,
                style: frame.style.clone(),
                nodes: roots,
                edges: Vec::new(),
                span: None,
            });
        }
    }
    for e in &doc.edges {
        let def = EdgeDef {
            from: e.from.0.clone(),
            to: e.to.0.clone(),
            label: e.label.clone(),
            arrow: Some(e.arrow).filter(|a| *a != rhizo_core::Arrow::End),
            class: e.class.clone(),
            style: e.style.clone(),
            span: None,
        };
        let frame_of = |id: &NodeId| doc.node(id).map(|n| &n.frame);
        let same = frame_of(&e.from).filter(|f| Some(*f) == frame_of(&e.to));
        let target = same
            .and_then(|f| doc.frame(f))
            .filter(|f| !f.implicit)
            .and_then(|f| file.frames.iter_mut().find(|fd| fd.label == f.label && frame_matches(fd, f)));
        match target {
            Some(fd) => fd.edges.push(def),
            None => file.edges.push(def),
        }
    }
    file
}

fn frame_matches(def: &FrameDef, frame: &Frame) -> bool {
    match &def.id {
        Some(id) => *id == frame.id.0,
        None => def.label.as_deref().map(slugify).as_deref() == Some(frame.id.as_str()),
    }
}

fn raise_node(doc: &Document, id: &NodeId) -> NodeDef {
    let n = &doc.nodes[id];
    NodeDef {
        label: n.label.clone(),
        id: Some(n.id.0.clone()).filter(|i| *i != slugify(&n.label)),
        class: n.class.clone(),
        style: n.style.clone(),
        x: n.at.map(|p| p.x),
        y: n.at.map(|p| p.y),
        note: n.note.clone(),
        link: n.link.clone(),
        tags: n.tags.clone(),
        children: n.children.iter().map(|c| raise_node(doc, c)).collect(),
        span: None,
    }
}
