//! AST ⇄ document model.

use std::collections::{HashMap, HashSet};

use rhizo_core::{
    Document, Edge, EdgeId, Frame, FrameId, LayoutKind, Node, NodeId, Point, PortName, Theme, TypeName, slugify,
};

use crate::ast::{EdgeDef, File, FrameDef, NodeDef, Span};
use crate::diag::{Diagnostic, suggest_name};

pub const IMPLICIT_FRAME: &str = "main";

/// Resolves ids, label references and styles. The document is complete even
/// when there are errors (bad edges are dropped), so tools can still show
/// something while the user is typing.
pub fn lower(file: &File) -> (Document, Vec<Diagnostic>) {
    let l = lower_with_sources(file);
    (l.document, l.diagnostics)
}

/// [`lower`], plus where each node and edge came from in the source.
pub struct Lowered {
    pub document: Document,
    pub diagnostics: Vec<Diagnostic>,
    /// Source span of each node's statement. Nodes created by an edge
    /// reference have none.
    pub node_spans: HashMap<NodeId, Span>,
    /// Source span of the statement each edge came from, parallel to
    /// `document.edges`. `edge a b c` gives two edges with the same span.
    pub edge_spans: Vec<Span>,
}

pub fn lower_with_sources(file: &File) -> Lowered {
    // JSON, YAML and the outline importers build ASTs without a depth limit;
    // lowering recurses, so refuse what could overflow the stack.
    let deepest =
        file.nodes.iter().chain(file.all_frames().into_iter().flat_map(|f| &f.nodes)).map(depth_of).max().unwrap_or(0);
    if deepest > crate::MAX_DEPTH {
        let msg = format!("nested {deepest} levels deep; rhizo reads at most {}", crate::MAX_DEPTH);
        return Lowered {
            document: Document::default(),
            diagnostics: vec![Diagnostic::error(msg, None)],
            node_spans: HashMap::new(),
            edge_spans: Vec::new(),
        };
    }
    let mut cx = Lower::default();
    cx.run(file);
    let mut seen = HashSet::new();
    cx.diags.retain(|d| seen.insert((d.message.clone(), d.span)));
    Lowered { document: cx.doc, diagnostics: cx.diags, node_spans: cx.node_spans, edge_spans: cx.edge_spans }
}

#[derive(Default)]
struct Lower {
    doc: Document,
    diags: Vec<Diagnostic>,
    /// Every explicit id, collected up front so derived ids never take one.
    reserved: HashSet<String>,
    taken: HashSet<String>,
    by_label: HashMap<String, Vec<NodeId>>,
    node_spans: HashMap<NodeId, Span>,
    edge_spans: Vec<Span>,
}

impl Lower {
    fn run(&mut self, file: &File) {
        self.doc.title = file.title.clone();
        if let Some(t) = &file.theme
            && Theme::by_name(t).is_none()
        {
            let mut d = Diagnostic::warning(format!("unknown theme `{t}`, using `light`"), None);
            d.help = Some(format!("available: {}", Theme::NAMES.join(", ")));
            self.diags.push(d);
        }
        self.doc.theme = file.theme.clone();
        self.doc.styles = file.styles.clone();
        self.doc.kits = file.kits.clone();
        self.doc.base = file.base.clone();
        self.doc.prefixes = file.prefixes.clone();
        self.doc.imports = file.imports.clone();

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
        for f in file.all_frames() {
            reserve(&f.nodes, &mut explicit);
        }
        // Ids end up in file names, comments and code; a line break in one
        // could smuggle lines into generated files.
        let frame_ids = file.all_frames().into_iter().filter_map(|f| f.id.clone().map(|id| (id, f.span)));
        for (id, span) in explicit.iter().cloned().chain(frame_ids).collect::<Vec<_>>().iter() {
            if id.chars().any(char::is_control) {
                self.diags.push(
                    Diagnostic::error("an id cannot contain line breaks, tabs or other control characters", *span)
                        .with_help("use letters, digits, `-`, `_`, `.` and `:`"),
                );
            }
        }
        // Edge ids share the namespace, so derived node ids must avoid them too.
        let edge_ids = file.edges.iter().chain(file.all_frames().into_iter().flat_map(|f| &f.edges));
        explicit.extend(edge_ids.filter_map(|e| e.id.clone().map(|id| (id, e.span))));
        for (id, span) in explicit {
            if !self.reserved.insert(id.clone()) {
                self.diags.push(Diagnostic::error(format!("id `{id}` is used more than once"), span));
            }
        }

        let mut frame_ids = HashSet::new();
        let has_implicit = !file.nodes.is_empty() || file.frames.is_empty();
        // Every frame with its parent, parents first.
        let mut frames: Vec<(FrameId, Option<FrameId>, &FrameDef)> = Vec::new();
        let mut stack: Vec<(&FrameDef, Option<FrameId>, usize)> =
            file.frames.iter().rev().map(|f| (f, None, 0)).collect();
        while let Some((f, parent, depth)) = stack.pop() {
            let i = frames.len();
            let base = f.id.clone().or_else(|| f.label.as_deref().map(slugify)).unwrap_or(format!("frame-{}", i + 1));
            let id = unique(&base, |c| frame_ids.contains(c) || (has_implicit && c == IMPLICIT_FRAME));
            if f.id.as_deref().is_some_and(|given| given != id) {
                self.diags.push(Diagnostic::error(format!("frame id `{base}` is used more than once"), f.span));
            }
            frame_ids.insert(id.clone());
            let fid = FrameId(id);
            if depth == rhizo_core::MAX_FRAME_DEPTH && !f.frames.is_empty() {
                let msg = format!("frames nest at most {} deep", rhizo_core::MAX_FRAME_DEPTH);
                self.diags.push(Diagnostic::error(msg, f.frames[0].span));
            } else if f.layout == Some(LayoutKind::Sequence) && !f.frames.is_empty() {
                self.diags.push(
                    Diagnostic::error("a sequence frame cannot hold frames", f.frames[0].span)
                        .with_help("move the frame out, next to the sequence"),
                );
            } else {
                stack.extend(f.frames.iter().rev().map(|c| (c, Some(fid.clone()), depth + 1)));
            }
            frames.push((fid, parent, f));
        }

        if has_implicit {
            let mut frame = Frame::new(IMPLICIT_FRAME);
            frame.implicit = true;
            frame.layout = file.layout.unwrap_or_default();
            frame.direction = file.direction;
            self.doc.frames.push(frame);
            let fid = FrameId::from(IMPLICIT_FRAME);
            self.nesting_in_sequence(file.layout, &file.nodes);
            for n in &file.nodes {
                self.node(n, &fid, None);
            }
        } else if file.layout.is_some() || file.direction.is_some() {
            self.diags.push(Diagnostic::warning(
                "top-level `layout`/`direction` only apply to nodes outside frames, and there are none",
                None,
            ));
        }

        for (fid, parent, f) in &frames {
            let frame = Frame {
                id: fid.clone(),
                label: f.label.clone(),
                layout: f.layout.unwrap_or_default(),
                direction: f.direction,
                at: point(f.x, f.y),
                columns: f.columns,
                parent: parent.clone(),
                style: f.style.clone(),
                implicit: false,
                ty: f.ty.clone().map(TypeName),
                props: f.props.clone(),
            };
            self.doc.frames.push(frame);
            self.nesting_in_sequence(f.layout, &f.nodes);
            for n in &f.nodes {
                self.node(n, fid, None);
            }
        }

        // Inner frames' edges first (children before parents, siblings in
        // order), so a name first used inside a nested frame lives there.
        let mut post: Vec<usize> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for (i, (_, parent, _)) in frames.iter().enumerate() {
            while let Some(&top) = open.last() {
                if parent.as_ref() == Some(&frames[top].0) {
                    break;
                }
                post.extend(open.pop());
            }
            open.push(i);
        }
        post.extend(open.into_iter().rev());
        for i in post {
            let (fid, _, f) = &frames[i];
            for e in &f.edges {
                self.edge(e, Some(fid));
            }
        }
        for e in &file.edges {
            self.edge(e, None);
        }
        self.check_classes();
        self.check_typed(file);
    }

    /// Version gate and structural checks for version-2 features.
    fn check_typed(&mut self, file: &File) {
        if self.doc.is_typed() && file.rhizo.unwrap_or(1) < 2 {
            let span = first_typed_span(file);
            self.diags.push(
                Diagnostic::error(
                    "this document uses version 2 features (type, props, ports, kits, prefixes, imports, provenance)",
                    span,
                )
                .with_help("add `rhizo version=2` at the top (JSON/YAML: `\"rhizo\": 2`)"),
            );
        }
        for err in self.doc.validate() {
            self.diags.push(Diagnostic::error(err.to_string(), None));
        }
    }

    fn nesting_in_sequence(&mut self, layout: Option<LayoutKind>, nodes: &[NodeDef]) {
        if layout != Some(LayoutKind::Sequence) {
            return;
        }
        if let Some(n) = nodes.iter().find(|n| !n.children.is_empty()) {
            self.diags.push(Diagnostic::warning(
                "nesting has no meaning in a sequence; nested nodes are drawn as more participants",
                n.span,
            ));
        }
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
        node.ty = def.ty.clone().map(TypeName);
        node.props = def.props.clone();
        node.ports = def.ports.clone();
        node.accept = def.accept.clone();
        node.prov = def.prov.clone();
        if node.ty.is_some() && def.id.is_none() {
            self.diags.push(
                Diagnostic::warning(
                    format!("typed node `{id}` has no explicit id; renaming its label would change it"),
                    def.span,
                )
                .with_help(format!("pin it with `id={id}` (`rhizo fmt` does this for you)")),
            );
        }
        let nid = NodeId(id);
        self.by_label.entry(def.label.clone()).or_default().push(nid.clone());
        if def.span.is_some() {
            self.node_spans.insert(nid.clone(), def.span);
        }
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
        if self.declared_curie(r) {
            return Err(false); // a reference into another namespace: new here, no warning
        }
        // Labels sorted: `by_label` is a HashMap, and ties must not depend on its order.
        let mut labels: Vec<&str> = self.by_label.keys().map(String::as_str).collect();
        labels.sort_unstable();
        let candidates: Vec<&str> = self.doc.nodes.keys().map(|k| k.as_str()).chain(labels).collect();
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
                // No span: the node has no statement of its own in the source.
                let mut def = NodeDef::new(r);
                if self.declared_curie(r) {
                    def.id = Some(r.to_string()); // `model:order` stays a reference, not a slug
                }
                self.node(&def, &frame, None);
                self.resolve(r, span).ok()
            }
        }
    }

    /// `prefix:local` where the prefix is declared (`prefix` or `import`).
    fn declared_curie(&self, r: &str) -> bool {
        rhizo_core::typed::curie_prefix(r)
            .is_some_and(|p| self.doc.prefixes.contains_key(p) || self.doc.imports.contains_key(p))
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
            id: def.id.clone().map(EdgeId),
            ty: def.ty.clone().map(TypeName),
            kind: def.kind,
            from_port: def.from_port.clone().map(PortName),
            to_port: def.to_port.clone().map(PortName),
            props: def.props.clone(),
            prov: def.prov.clone(),
        };
        self.doc.add_edge(edge).expect("endpoints resolved");
        self.edge_spans.push(def.span);
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

fn first_typed_span(file: &File) -> Span {
    fn in_nodes(nodes: &[NodeDef]) -> Option<Span> {
        nodes.iter().find_map(|n| if n.is_typed() { Some(n.span) } else { in_nodes(&n.children) })
    }
    let frames = file.all_frames().into_iter();
    in_nodes(&file.nodes)
        .or_else(|| {
            frames
                .clone()
                .find_map(|f| if f.ty.is_some() || !f.props.is_empty() { Some(f.span) } else { in_nodes(&f.nodes) })
        })
        .or_else(|| file.edges.iter().chain(frames.flat_map(|f| &f.edges)).find(|e| e.is_typed()).map(|e| e.span))
        .flatten()
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

/// How deep `n`'s children go, without recursion.
fn depth_of(n: &NodeDef) -> usize {
    let mut stack = vec![(n, 1usize)];
    let mut max = 0;
    while let Some((n, d)) = stack.pop() {
        max = max.max(d);
        if d > crate::MAX_DEPTH {
            break;
        }
        stack.extend(n.children.iter().map(|c| (c, d + 1)));
    }
    max
}

/// The inverse of [`lower`]: nests children, drops ids that would be derived
/// anyway, and puts each edge in the frame that holds both ends.
pub fn raise(doc: &Document) -> File {
    let mut file = File {
        rhizo: doc.is_typed().then_some(2),
        title: doc.title.clone(),
        theme: doc.theme.clone(),
        kits: doc.kits.clone(),
        base: doc.base.clone(),
        prefixes: doc.prefixes.clone(),
        imports: doc.imports.clone(),
        styles: doc.styles.clone(),
        ..Default::default()
    };
    let node_def = |id: &NodeId| -> NodeDef { raise_node(doc, id) };
    // A bare reference into an import (`model:order`, only used by edges)
    // needs no statement: the edge brings it back.
    let bare_reference = |n: &rhizo_core::Node| {
        rhizo_core::typed::curie_prefix(n.id.as_str()).is_some_and(|p| doc.imports.contains_key(p))
            && n.label == n.id.as_str()
            && n.children.is_empty()
            && n.parent.is_none()
            && n.style.is_empty()
            && n.class.is_empty()
            && !n.is_typed()
            && n.note.is_none()
            && n.link.is_none()
            && n.tags.is_empty()
            && n.at.is_none()
            && doc.edges.iter().any(|e| e.from == n.id || e.to == n.id)
    };
    // Explicit frames by their index in `doc.frames`; nested ones move into
    // their parent at the end.
    let mut defs: Vec<Option<FrameDef>> = vec![None; doc.frames.len()];
    for (i, frame) in doc.frames.iter().enumerate() {
        let roots: Vec<NodeDef> =
            doc.roots(&frame.id).filter(|n| !bare_reference(n)).map(|n| node_def(&n.id)).collect();
        if frame.implicit {
            file.layout = Some(frame.layout).filter(|l| *l != LayoutKind::Auto);
            file.direction = frame.direction;
            file.nodes = roots;
        } else {
            defs[i] = Some(FrameDef {
                id: Some(frame.id.0.clone())
                    .filter(|id| Some(id.as_str()) != frame.label.as_deref().map(slugify).as_deref()),
                label: frame.label.clone(),
                layout: Some(frame.layout).filter(|l| *l != LayoutKind::Auto),
                direction: frame.direction,
                x: frame.at.map(|p| p.x),
                y: frame.at.map(|p| p.y),
                columns: frame.columns,
                ty: frame.ty.as_ref().map(|t| t.0.clone()),
                props: frame.props.clone(),
                style: frame.style.clone(),
                nodes: roots,
                edges: Vec::new(),
                frames: Vec::new(),
                extra: Default::default(),
                span: None,
            });
        }
    }
    let index_of = |id: &FrameId| doc.frames.iter().position(|f| &f.id == id);
    for e in &doc.edges {
        let def = EdgeDef {
            from: e.from.0.clone(),
            to: e.to.0.clone(),
            label: e.label.clone(),
            arrow: Some(e.arrow).filter(|a| *a != rhizo_core::Arrow::End),
            class: e.class.clone(),
            style: e.style.clone(),
            id: e.id.as_ref().map(|i| i.0.clone()),
            ty: e.ty.as_ref().map(|t| t.0.clone()),
            kind: e.kind,
            from_port: e.from_port.as_ref().map(|p| p.0.clone()),
            to_port: e.to_port.as_ref().map(|p| p.0.clone()),
            props: e.props.clone(),
            prov: e.prov.clone(),
            extra: Default::default(),
            span: None,
        };
        // The innermost frame holding both ends.
        let frame_of = |id: &NodeId| doc.node(id).map(|n| &n.frame);
        let common = frame_of(&e.from)
            .zip(frame_of(&e.to))
            .and_then(|(a, b)| std::iter::once(a).chain(doc.frame_ancestors(a)).find(|f| doc.frame_within(b, f)));
        match common.and_then(index_of).and_then(|i| defs[i].as_mut()) {
            Some(fd) => fd.edges.push(def),
            None => file.edges.push(def),
        }
    }
    // Children come after their parents, so the last frame has no children left to collect.
    for i in (0..doc.frames.len()).rev() {
        let parent = doc.frames[i].parent.as_ref().and_then(index_of).filter(|&p| p < i && defs[p].is_some());
        match (parent, defs[i].take()) {
            (Some(p), Some(fd)) => defs[p].as_mut().unwrap().frames.insert(0, fd),
            (None, fd) => defs[i] = fd,
            (Some(_), None) => {}
        }
    }
    file.frames = defs.into_iter().flatten().collect();
    file
}

fn raise_node(doc: &Document, id: &NodeId) -> NodeDef {
    let n = &doc.nodes[id];
    NodeDef {
        label: n.label.clone(),
        // Typed nodes always keep their id: traces and generated code hang on it.
        id: Some(n.id.0.clone()).filter(|i| n.ty.is_some() || *i != slugify(&n.label)),
        class: n.class.clone(),
        style: n.style.clone(),
        x: n.at.map(|p| p.x),
        y: n.at.map(|p| p.y),
        note: n.note.clone(),
        link: n.link.clone(),
        tags: n.tags.clone(),
        ty: n.ty.as_ref().map(|t| t.0.clone()),
        props: n.props.clone(),
        ports: n.ports.clone(),
        accept: n.accept.clone(),
        prov: n.prov.clone(),
        children: n.children.iter().map(|c| raise_node(doc, c)).collect(),
        extra: Default::default(),
        span: None,
    }
}
