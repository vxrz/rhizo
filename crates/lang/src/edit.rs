//! Editing a KDL document in place.
//!
//! Every operation becomes a small text splice at the spans kdl-rs reports,
//! so comments, blank lines and the author's formatting survive and the diff
//! stays minimal. After each splice the source is parsed and lowered again;
//! an edit that would add errors is refused. Undo and redo keep whole source
//! snapshots, which is cheap at diagram sizes and can never drift.

use std::collections::HashMap;

use kdl::{KdlDocument, KdlNode};
use rhizo_core::{Direction, Document, FrameId, LayoutKind, NodeId, slugify};

use crate::diag::Diagnostic;
use crate::lower::{Lowered, lower_with_sources};
use crate::write::{jik_lines, jik_scalar, kdl_string, num, quoted};

/// Where [`Editor::reparent`] puts a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Last child of this node.
    Child(NodeId),
    /// Sibling right before this node.
    Before(NodeId),
    /// Sibling right after this node.
    After(NodeId),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EditError {
    #[error("no node `{0}`")]
    UnknownNode(NodeId),
    #[error("no frame `{0}`")]
    UnknownFrame(FrameId),
    #[error("node `{0}` has no statement of its own to edit")]
    NoStatement(NodeId),
    #[error("the edit would make the document invalid: {0}")]
    Invalid(String),
    #[error("`{0}` cannot move under itself or its own descendants")]
    Cycle(NodeId),
    #[error("`{0}` is not a valid prop key")]
    InvalidKey(String),
    #[error("no edge #{0}")]
    UnknownEdge(usize),
    /// Its statement is gone, or lowering dropped part of its path.
    #[error("edge #{0} has no statement of its own to edit")]
    NoEdgeStatement(usize),
}

/// One replacement: `start..end` of the old source becomes `text`.
struct Splice {
    start: usize,
    end: usize,
    text: String,
}

struct State {
    kdl: Option<KdlDocument>,
    lowered: Lowered,
}

fn analyze(src: &str) -> State {
    let (file, mut diagnostics) = crate::kdl::parse(src);
    if diagnostics.iter().any(Diagnostic::is_error) {
        let lowered =
            Lowered { document: Document::default(), diagnostics, node_spans: HashMap::new(), edge_spans: Vec::new() };
        return State { kdl: None, lowered };
    }
    let mut lowered = lower_with_sources(&file);
    diagnostics.append(&mut lowered.diagnostics);
    lowered.diagnostics = diagnostics;
    State { kdl: KdlDocument::parse(src).ok(), lowered }
}

fn error_count(s: &State) -> usize {
    s.lowered.diagnostics.iter().filter(|d| d.is_error()).count()
}

pub struct Editor {
    src: String,
    state: State,
    undo: Vec<String>,
    redo: Vec<String>,
}

impl Editor {
    /// Opens KDL source. The document may have errors; editing works on
    /// whatever lowered.
    pub fn open(src: impl Into<String>) -> Editor {
        let src = src.into();
        let state = analyze(&src);
        Editor { src, state, undo: Vec::new(), redo: Vec::new() }
    }

    pub fn source(&self) -> &str {
        &self.src
    }

    pub fn document(&self) -> &Document {
        &self.state.lowered.document
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.state.lowered.diagnostics
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.src, prev));
        self.state = analyze(&self.src);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.src, next));
        self.state = analyze(&self.src);
        true
    }

    /// Sets a node's position (`x`/`y`), as free layouts use it.
    pub fn move_node(&mut self, id: &NodeId, x: f32, y: f32) -> Result<(), EditError> {
        self.node_or_err(id)?;
        let Some(kn) = self.statement(id) else {
            let label = self.document().nodes[id].label.clone();
            let splice = self.declare(id, &label, &format!(" x={} y={}", num(x), num(y)))?;
            return self.apply(vec![splice]);
        };
        let mut splices = Vec::new();
        let mut missing = String::new();
        for (key, v) in [("x", x), ("y", y)] {
            match kn.entries().iter().find(|e| e.name().is_some_and(|n| n.value() == key)) {
                Some(e) => splices.push(Splice {
                    start: e.span().offset(),
                    end: e.span().offset() + e.span().len(),
                    text: format!("{key}={}", num(v)),
                }),
                None => missing.push_str(&format!(" {key}={}", num(v))),
            }
        }
        if !missing.is_empty() {
            let at = after_entries(kn);
            splices.push(Splice { start: at, end: at, text: missing });
        }
        self.apply(splices)
    }

    /// Changes a node's label. The id stays the same: it is written out when
    /// it would otherwise be derived from the new label.
    pub fn rename(&mut self, id: &NodeId, label: &str) -> Result<(), EditError> {
        self.node_or_err(id)?;
        let pin =
            if slugify(label) != id.as_str() { format!(" id={}", kdl_string(id.as_str())) } else { String::new() };
        let Some(kn) = self.statement(id) else {
            let splice = self.declare(id, label, "")?;
            return self.apply(vec![splice]);
        };
        let has_id = kn.entries().iter().any(|e| e.name().is_some_and(|n| n.value() == "id"));
        let pin = if has_id { String::new() } else { pin };
        let (start, end, text) = self.label_place(kn, label).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        self.apply(vec![Splice { start, end, text: format!("{text}{pin}") }])
    }

    /// Adds a child under `parent` and returns its id.
    pub fn add_child(&mut self, parent: &NodeId, label: &str) -> Result<NodeId, EditError> {
        self.node_or_err(parent)?;
        let kn = self.statement(parent).ok_or_else(|| EditError::NoStatement(parent.clone()))?;
        let keyword = if string_named(kn) { "-" } else { kn.name().value() };
        let line = format!("{keyword} {}", quoted(label));
        let splice = insert_child(&self.src, kn, &line);
        self.apply_and_find(splice, &line)
    }

    /// Adds a top-level node to a frame and returns its id.
    pub fn add_node(&mut self, frame: &FrameId, label: &str) -> Result<NodeId, EditError> {
        let f = self.document().frame(frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))?;
        let keyword = match f.layout {
            // A typed frame (an environment, an API) holds things, not an outline.
            _ if f.ty.is_some() => "node",
            LayoutKind::Layered | LayoutKind::Grid | LayoutKind::Free | LayoutKind::Sequence => "node",
            _ => "-",
        };
        let line = format!("{keyword} {}", quoted(label));
        let splice = self.append_to_frame(frame, &line)?;
        self.apply_and_find(splice, &line)
    }

    /// Adds an edge `from → to`.
    pub fn connect(&mut self, from: &NodeId, to: &NodeId) -> Result<(), EditError> {
        let (a, b) = (self.node_or_err(from)?.frame.clone(), self.node_or_err(to)?.frame.clone());
        let line = format!("edge {} {}", kdl_string(from.as_str()), kdl_string(to.as_str()));
        let frame = if a == b { a } else { self.implicit_frame_id() };
        let splice = self.append_to_frame(&frame, &line)?;
        self.apply(vec![splice])
    }

    /// Deletes a node with its descendants and every edge touching them.
    /// Edge paths through a deleted node are split, not reconnected.
    pub fn delete(&mut self, id: &NodeId) -> Result<(), EditError> {
        self.node_or_err(id)?;
        let doc = self.document();
        let mut doomed = vec![id.clone()];
        let mut i = 0;
        while i < doomed.len() {
            doomed.extend(doc.nodes[&doomed[i]].children.iter().cloned());
            i += 1;
        }
        let mut splices = Vec::new();
        // The statement of the top-most deleted node holds its descendants.
        if let Some(kn) = self.statement(id) {
            splices.push(self.removal(id, kn));
        }
        splices.extend(self.edge_cuts(&doomed, |_| false));
        self.apply(splices)
    }

    /// The splices that remove the edges touching `doomed`: paths through
    /// them are split, not reconnected. Statements at an offset `skip`
    /// accepts are left alone (they go with something else).
    fn edge_cuts(&self, doomed: &[NodeId], skip: impl Fn(usize) -> bool) -> Vec<Splice> {
        let doc = self.document();
        let mut splices = Vec::new();
        for (off, ks) in self.edge_statements() {
            if skip(off) {
                continue;
            }
            let edges: Vec<_> = ks.iter().map(|&k| &doc.edges[k]).collect();
            if !edges.iter().any(|e| doomed.contains(&e.from) || doomed.contains(&e.to)) {
                continue;
            }
            let Some(kn) = self.kdl_node_at(off) else { continue };
            let args = kn.entries().iter().filter(|e| e.name().is_none()).count();
            let mut ends: Vec<&NodeId> = edges.iter().map(|e| &e.from).collect();
            ends.extend(edges.last().map(|e| &e.to));
            // If lowering dropped part of the path, keeping pieces could be wrong: drop it all.
            let keep: Vec<Vec<usize>> = if ends.len() == args {
                let mut segs = vec![Vec::new()];
                for (i, e) in ends.iter().enumerate() {
                    if doomed.contains(e) {
                        segs.push(Vec::new());
                    } else {
                        segs.last_mut().unwrap().push(i);
                    }
                }
                segs.into_iter().filter(|s| s.len() >= 2).collect()
            } else {
                Vec::new()
            };
            let props = self.edge_props(kn, None);
            splices.push(self.split_path(kn, keep.into_iter().map(|s| (s, props.clone())).collect()));
        }
        splices
    }

    /// Edge statements by their offset, with the edges each made.
    fn edge_statements(&self) -> Vec<(usize, Vec<usize>)> {
        let mut statements: Vec<(usize, Vec<usize>)> = Vec::new();
        for (k, span) in self.state.lowered.edge_spans.iter().enumerate() {
            let Some((off, _)) = span else { continue };
            match statements.iter_mut().find(|(o, _)| o == off) {
                Some((_, ks)) => ks.push(k),
                None => statements.push((*off, vec![k])),
            }
        }
        statements
    }

    /// Removes `document().edges[k]`. A path (`edge a b c`) is split around
    /// that hop; the other hops keep their properties.
    pub fn disconnect(&mut self, k: usize) -> Result<(), EditError> {
        let (kn, hops, hop) = self.edge_statement(k)?;
        let props = self.edge_props(kn, None);
        let segs = [(0..=hop).collect::<Vec<_>>(), (hop + 1..=hops).collect()];
        let segs = segs.into_iter().filter(|s| s.len() >= 2).map(|s| (s, props.clone())).collect();
        let splice = self.split_path(kn, segs);
        self.apply(vec![splice])
    }

    /// Adds, replaces or (with `None`) removes the label of
    /// `document().edges[k]`. A hop of a longer path is split out first, so
    /// only it gets the label.
    pub fn set_edge_label(&mut self, k: usize, label: Option<&str>) -> Result<(), EditError> {
        self.edge_statement(k)?;
        if self.document().edges[k].label.as_deref() == label {
            return Ok(());
        }
        self.set_edge_entry(k, "label", label.map(|l| format!("label={}", quoted(l))), false)
    }

    /// Sets (or, `None`, removes) an entry of the statement of
    /// `document().edges[k]`: `kind`, `arrow`, `line`, `type`, `stroke`…
    /// The value is written bare where KDL allows. A hop of a longer path is
    /// split out first, as for [`Editor::set_edge_label`].
    pub fn set_edge_attr(&mut self, k: usize, key: &str, value: Option<&str>) -> Result<(), EditError> {
        if !prop_key_ok(key) {
            return Err(EditError::InvalidKey(key.to_string()));
        }
        let text = value.map(|v| format!("{}={}", kdl_string(key), kdl_string(v)));
        self.set_edge_entry(k, key, text, matches!(key, "kind" | "type" | "id" | "from-port" | "to-port"))
    }

    /// Writes the `key=…` entry `text` (or removes it) on edge `k`'s
    /// statement; `v2` raises the header for version 2 features.
    fn set_edge_entry(&mut self, k: usize, key: &str, text: Option<String>, v2: bool) -> Result<(), EditError> {
        let (kn, hops, hop) = self.edge_statement(k)?;
        let mut splices = if v2 && text.is_some() { self.v2_header("") } else { Vec::new() };
        if hops == 1 {
            splices.extend(self.set_entry(kn, key, text));
        } else {
            let (old, new) = (self.edge_props(kn, None), self.edge_props(kn, Some((key, text.as_deref()))));
            let segs =
                [((0..=hop).collect(), old.clone()), (vec![hop, hop + 1], new), ((hop + 1..=hops).collect(), old)];
            splices.push(self.split_path(kn, segs.into_iter().filter(|(s, _)| s.len() >= 2).collect()));
        }
        self.apply(merged(splices))
    }

    /// Runs `f` once edge `k` has a statement of its own: a hop of a path is
    /// split out first (the edge keeps its index), and joined back if `f` fails.
    fn on_own_edge(&mut self, k: usize, f: impl FnOnce(&mut Self) -> Result<(), EditError>) -> Result<(), EditError> {
        let (kn, hops, hop) = self.edge_statement(k)?;
        let split = hops > 1;
        if split {
            let props = self.edge_props(kn, None);
            let segs = [(0..=hop).collect(), vec![hop, hop + 1], (hop + 1..=hops).collect::<Vec<_>>()];
            let segs = segs.into_iter().filter(|s| s.len() >= 2).map(|s| (s, props.clone())).collect();
            let splice = self.split_path(kn, segs);
            self.apply(vec![splice])?;
        }
        let done = f(self);
        if done.is_err() && split {
            self.undo();
            self.redo.pop();
        }
        done
    }

    /// Moves a node (with its subtree) to a new place in the outline.
    pub fn reparent(&mut self, id: &NodeId, to: &Placement) -> Result<(), EditError> {
        let node = self.node_or_err(id)?;
        let label = node.label.clone();
        let target = match to {
            Placement::Child(t) | Placement::Before(t) | Placement::After(t) => t,
        };
        self.node_or_err(target)?;
        let doc = self.document();
        if target == id || doc.ancestors(target).any(|a| a == id) {
            return Err(EditError::Cycle(id.clone()));
        }
        // Dropping a node where it already is changes nothing.
        let siblings = |of: &NodeId| -> Vec<NodeId> {
            match &doc.nodes[of].parent {
                Some(p) => doc.nodes[p].children.clone(),
                None => doc.roots(&doc.nodes[of].frame).map(|n| n.id.clone()).collect(),
            }
        };
        let same_parent =
            doc.nodes[id].parent == doc.nodes[target].parent && doc.nodes[id].frame == doc.nodes[target].frame;
        let pos = |list: &[NodeId], n: &NodeId| list.iter().position(|x| x == n);
        let already = match to {
            Placement::Child(t) => doc.nodes[t].children.last() == Some(id),
            Placement::After(t) => {
                same_parent && {
                    let l = siblings(t);
                    pos(&l, t).map(|i| i + 1) == pos(&l, id)
                }
            }
            Placement::Before(t) => {
                same_parent && {
                    let l = siblings(t);
                    pos(&l, id).map(|i| i + 1) == pos(&l, t)
                }
            }
        };
        if already {
            return Ok(());
        }
        let tkn = self.statement(target).ok_or_else(|| EditError::NoStatement(target.clone()))?;
        let mut splices = Vec::new();
        // The node's own text, or a fresh statement for an edge-created node.
        let (text, old_indent) = match self.statement(id) {
            Some(kn) => {
                splices.push(self.removal(id, kn));
                self.statement_text(kn)
            }
            None => (format!("- {} id={}", quoted(&label), kdl_string(id.as_str())), String::new()),
        };
        splices.push(placement_splice(&self.src, to, tkn, |new| reindent(&text, &old_indent, new)));
        self.apply(splices)
    }

    /// A statement's text, with a `// comment` trailing it on its line
    /// (it moves with it), and the indentation of its line.
    fn statement_text(&self, kn: &KdlNode) -> (String, String) {
        let (start, end) = (kn.span().offset(), kn.span().offset() + kn.span().len());
        let (_, le) = whole_lines(&self.src, start, end);
        let tail = self.src[end..le.max(end)].trim_end_matches('\n');
        let text = if tail.trim_start().starts_with("//") {
            // Span plus tail is the original line, spacing included.
            format!("{}{}", &self.src[start..end], tail.trim_end())
        } else {
            self.src[start..end].trim_end().to_string()
        };
        (text, line_indent(&self.src, start))
    }

    /// Adds a sibling right after `after` and returns its id.
    pub fn add_after(&mut self, after: &NodeId, label: &str) -> Result<NodeId, EditError> {
        let node = self.node_or_err(after)?;
        let Some(kn) = self.statement(after) else {
            // An edge-created node has no statement to follow; add to its frame.
            let frame = node.frame.clone();
            return self.add_node(&frame, label);
        };
        let keyword = if string_named(kn) { "-" } else { kn.name().value() };
        let line = format!("{keyword} {}", quoted(label));
        let splice = placement_splice(&self.src, &Placement::After(after.clone()), kn, |_| line.clone());
        self.apply_and_find(splice, &line)
    }

    /// Places a reference to node `target_id` of another file: adds
    /// `import "<import_path>" as=<alias>` unless the file imports it already
    /// (the alias is the file's stem, numbered if another import has it),
    /// raises `rhizo version=1` to 2, and adds `node "<label>" id=<alias>:<target_id>`
    /// to `frame`, or at the top level. One edit, so one undo step. Returns the
    /// reference's id; when the file has it already, nothing changes.
    pub fn insert_reference(
        &mut self,
        import_path: &str,
        target_id: &str,
        label: &str,
        frame: Option<&FrameId>,
    ) -> Result<NodeId, EditError> {
        let doc = self.document();
        let same = |p: &str| p.trim_start_matches("./") == import_path.trim_start_matches("./");
        let existing = doc.imports.iter().find(|(_, p)| same(p)).map(|(a, _)| a.clone());
        let alias = existing.clone().unwrap_or_else(|| {
            let base = alias_for(import_path);
            let taken = |a: &str| doc.imports.contains_key(a) || doc.prefixes.contains_key(a);
            (1..)
                .map(|n| if n == 1 { base.clone() } else { format!("{base}{n}") })
                .find(|a| !taken(a))
                .expect("endless")
        });
        let id = NodeId(format!("{alias}:{target_id}"));
        if existing.is_some() && doc.nodes.contains_key(&id) {
            return Ok(id);
        }
        let frame = match frame {
            Some(f) => Some(doc.frame(f).ok_or_else(|| EditError::UnknownFrame(f.clone()))?),
            None => doc.frames.iter().find(|f| f.implicit),
        };
        let keyword = match frame.map(|f| f.layout) {
            Some(LayoutKind::Mindmap | LayoutKind::Tree) => "-",
            _ => "node",
        };
        // `model:order` is a fine bare KDL value when both halves are.
        let bare = kdl_string(&alias) == alias && kdl_string(target_id) == target_id;
        let written = if bare { id.to_string() } else { kdl_string(id.as_str()) };
        let line = format!("{keyword} {} id={written}", quoted(label));
        let import = if existing.is_some() {
            String::new()
        } else {
            format!("import {} as={}\n", quoted(import_path), kdl_string(&alias))
        };
        // The header and import first: an insertion at the same place keeps that order.
        let mut splices = self.v2_header(&import);
        splices.push(match frame.filter(|f| !f.implicit) {
            Some(f) => self.append_to_frame(&f.id, &line)?,
            None => {
                let at = self.src.len();
                let lead = if self.src.is_empty() || self.src.ends_with('\n') { "" } else { "\n" };
                Splice { start: at, end: at, text: format!("{lead}{line}\n") }
            }
        });
        self.apply(merged(splices))?;
        if self.document().nodes.contains_key(&id) {
            Ok(id)
        } else {
            self.undo();
            self.redo.pop();
            Err(EditError::Invalid(format!("`{id}` did not come out as a node")))
        }
    }

    /// Sets a node's `type`, or removes it with `None`. Types need format
    /// version 2: the header is added or raised when it says less.
    pub fn set_type(&mut self, id: &NodeId, ty: Option<&str>) -> Result<(), EditError> {
        let node = self.node_or_err(id)?;
        if node.ty.as_ref().map(|t| t.as_str()) == ty {
            return Ok(());
        }
        let text = ty.map(|t| format!("type={}", kdl_string(t)));
        let mut splices = if ty.is_some() { self.v2_header("") } else { Vec::new() };
        match self.statement(id) {
            Some(kn) => splices.extend(self.set_entry(kn, "type", text)),
            None => {
                // Only a typed node can need removing, and edges cannot type one.
                let label = node.label.clone();
                splices.push(self.declare(id, &label, &format!(" {}", text.unwrap_or_default()))?);
            }
        }
        self.apply(merged(splices))
    }

    /// Adds `use <kit> version="<version>"` unless the document uses the kit.
    pub fn use_kit(&mut self, kit: &str, version: &str) -> Result<(), EditError> {
        if self.document().kits.iter().any(|k| k.kit == kit) {
            return Ok(());
        }
        let line = format!("use {} version={}\n", kdl_string(kit), quoted(version));
        let splices = self.v2_header(&line);
        self.apply(merged(splices))
    }

    /// Sets prop `key` of a node to `value`, in its `props` block, which is
    /// made when missing: a scalar (a string, number, boolean or null) as
    /// `key=value`, a list or object as a child block (JiK), each replacing
    /// what was there.
    pub fn set_prop(&mut self, id: &NodeId, key: &str, value: &serde_json::Value) -> Result<(), EditError> {
        let node = self.node_or_err(id)?;
        if !prop_key_ok(key) {
            return Err(EditError::InvalidKey(key.to_string()));
        }
        if node.props.get(key) == Some(value) {
            return Ok(());
        }
        let mut splices = self.v2_header("");
        match self.statement(id) {
            None => {
                let text = jik_scalar(value).ok_or_else(|| {
                    EditError::Invalid(format!("`{key}` takes a list or object: edit it in the text"))
                })?;
                let label = node.label.clone();
                let entry = format!("{}={text}", kdl_string(key));
                splices.push(self.declare(id, &label, &format!(" {{ props {entry} }}"))?);
            }
            Some(kn) => splices.extend(self.prop_splices(kn, key, value)),
        }
        self.apply(merged(splices))
    }

    /// Sets a look of a node on its own statement (`shape=diamond`,
    /// `fill="#e11d48"`), where the file writes looks, unlike
    /// [`Editor::set_prop`]. `value` is a string, number or boolean.
    pub fn set_look(&mut self, id: &NodeId, key: &str, value: &serde_json::Value) -> Result<(), EditError> {
        self.node_or_err(id)?;
        if !prop_key_ok(key) {
            return Err(EditError::InvalidKey(key.to_string()));
        }
        let text = look_text(key, value)?;
        let kn = self.statement(id).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        let splice = self.set_entry(kn, key, Some(format!("{}={text}", kdl_string(key)))).expect("text given");
        self.apply(vec![splice])
    }

    /// Sets prop `key` of an explicit frame to `value`, as [`Editor::set_prop`] does for nodes.
    pub fn set_frame_prop(&mut self, frame: &FrameId, key: &str, value: &serde_json::Value) -> Result<(), EditError> {
        let f = self.document().frame(frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))?;
        if !prop_key_ok(key) {
            return Err(EditError::InvalidKey(key.to_string()));
        }
        if f.props.get(key) == Some(value) {
            return Ok(());
        }
        let kn = self.frame_statement(frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))?;
        let mut splices = self.v2_header("");
        splices.extend(self.prop_splices(kn, key, value));
        self.apply(merged(splices))
    }

    /// Removes a look (`shape`, `fill`…) from a node's statement.
    pub fn remove_look(&mut self, id: &NodeId, key: &str) -> Result<(), EditError> {
        self.node_or_err(id)?;
        let Some(kn) = self.statement(id) else { return Ok(()) };
        let splices = self.set_entry(kn, key, None).into_iter().collect();
        self.apply(splices)
    }

    /// Lets a node just made (by Tab, Enter or "Add typed node…") take
    /// `label`, its id following the label: the label's slug, numbered when
    /// another node or edge has it. A typed node writes it as `id=` right
    /// after the label (typed nodes keep explicit ids); an untyped one only
    /// when lowering would not derive it. Returns the new id.
    pub fn rename_new(&mut self, id: &NodeId, label: &str) -> Result<NodeId, EditError> {
        let node = self.node_or_err(id)?;
        let typed = node.ty.is_some();
        let doc = self.document();
        let taken = |c: &str| {
            c != id.as_str()
                && (doc.nodes.contains_key(&NodeId::from(c))
                    || doc.edges.iter().any(|e| e.id.as_ref().is_some_and(|i| i.as_str() == c)))
        };
        let slug = slugify(label);
        let new = unique(if slug.is_empty() { "node" } else { &slug }, taken);
        let kn = self.statement(id).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        let (start, end, mut text) = self.label_place(kn, label).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        let entry = kn.entries().iter().find(|e| e.name().is_some_and(|n| n.value() == "id"));
        let id_text = format!("id={}", kdl_string(&new));
        let mut splices = Vec::new();
        match (entry, typed || new != slug) {
            (Some(e), true) => {
                let s = e.span().offset();
                splices.push(Splice { start: s, end: s + self.piece(e.span()).trim_end().len(), text: id_text });
            }
            (Some(_), false) => splices.extend(self.set_entry(kn, "id", None)),
            (None, true) => text = format!("{text} {id_text}"),
            (None, false) => {}
        }
        splices.push(Splice { start, end, text });
        self.apply(splices)?;
        let new = NodeId(new);
        if self.document().nodes.contains_key(&new) {
            Ok(new)
        } else {
            self.undo();
            self.redo.pop();
            Err(EditError::Invalid(format!("`{new}` did not come out as a node")))
        }
    }

    /// Adds an acceptance criterion (`accept "…"`) as the node's last child.
    pub fn add_accept(&mut self, id: &NodeId, text: &str) -> Result<(), EditError> {
        self.node_or_err(id)?;
        let kn = self.statement(id).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        let mut splices = self.v2_header("");
        splices.push(insert_child(&self.src, kn, &format!("accept {}", quoted(text))));
        self.apply(merged(splices))
    }

    /// Rewrites the node's `i`-th acceptance criterion in place.
    pub fn set_accept(&mut self, id: &NodeId, i: usize, text: &str) -> Result<(), EditError> {
        let (_, a) = self.accept_statement(id, i)?;
        let e = a.entries().iter().find(|e| e.name().is_none()).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        let start = e.span().offset();
        let end = start + self.piece(e.span()).trim_end().len();
        self.apply(vec![Splice { start, end, text: quoted(text) }])
    }

    /// Removes the node's `i`-th acceptance criterion; the `{ }` goes with the last child.
    pub fn remove_accept(&mut self, id: &NodeId, i: usize) -> Result<(), EditError> {
        let (kn, a) = self.accept_statement(id, i)?;
        let splice = self.child_removal(kn, a);
        self.apply(vec![splice])
    }

    /// Adds an empty `frame "<label>"` as the last statement of `parent`'s
    /// block, or at the end of the file; `at` pins its position (`x`/`y`).
    /// Returns its id.
    pub fn add_frame(
        &mut self,
        parent: Option<&FrameId>,
        label: &str,
        at: Option<(f32, f32)>,
    ) -> Result<FrameId, EditError> {
        let pos = at.map_or_else(String::new, |(x, y)| format!(" x={} y={}", num(x), num(y)));
        let line = format!("{}{pos}", self.new_frame_line(label));
        let splice = match parent {
            Some(p) => {
                let f = self.frame_or_err(p)?;
                if !f.implicit && f.layout == LayoutKind::Sequence {
                    return Err(EditError::Invalid("a sequence frame cannot hold frames".into()));
                }
                self.append_to_frame(p, &line)?
            }
            None => self.at_end(&line),
        };
        self.apply_and_find_frame(vec![splice])
    }

    /// Changes a frame's label. Its id stays: it is written out when it
    /// would otherwise follow the new label (compiled output is named by it).
    pub fn rename_frame(&mut self, frame: &FrameId, label: &str) -> Result<(), EditError> {
        let kn = self.frame_kn(frame)?;
        let has_id = kn.entries().iter().any(|e| e.name().is_some_and(|n| n.value() == "id"));
        let pin = if !has_id && slugify(label) != frame.as_str() {
            format!(" id={}", kdl_string(frame.as_str()))
        } else {
            String::new()
        };
        let (start, end, text) = match self.label_place(kn, label) {
            Some(place) => place,
            None => {
                let at = kn.name().span().offset() + kn.name().span().len();
                (at, at, format!(" {}", quoted(label)))
            }
        };
        self.apply(vec![Splice { start, end, text: format!("{text}{pin}") }])
    }

    /// Sets a frame's `type` (an environment, an API…), or removes it with
    /// `None`. Types need format version 2: the header says so after.
    pub fn set_frame_type(&mut self, frame: &FrameId, ty: Option<&str>) -> Result<(), EditError> {
        self.set_frame_entry(frame, "type", ty.map(kdl_string), true)
    }

    /// Sets a frame's `layout`, or removes it (`auto`) with `None`.
    pub fn set_frame_layout(&mut self, frame: &FrameId, layout: Option<LayoutKind>) -> Result<(), EditError> {
        self.set_frame_entry(frame, "layout", layout.map(|l| l.name().to_string()), false)
    }

    /// Sets a frame's `direction`, or removes it (the layout's own) with `None`.
    pub fn set_frame_direction(&mut self, frame: &FrameId, direction: Option<Direction>) -> Result<(), EditError> {
        self.set_frame_entry(frame, "direction", direction.map(|d| d.name().to_string()), false)
    }

    /// Sets a look of a frame (`fill`, `stroke`…), as [`Editor::set_look`]
    /// does for nodes, or removes it with `None`.
    pub fn set_frame_look(
        &mut self,
        frame: &FrameId,
        key: &str,
        value: Option<&serde_json::Value>,
    ) -> Result<(), EditError> {
        if !prop_key_ok(key) {
            return Err(EditError::InvalidKey(key.to_string()));
        }
        let text = value.map(|v| look_text(key, v)).transpose()?;
        self.set_frame_entry(frame, key, text, false)
    }

    /// Deletes a frame. With `keep_contents` ("Ungroup") what it holds
    /// (nodes, edges, frames; not its `props`) takes its place, in its
    /// parent frame or at the top level. Without, it all goes, with every
    /// edge elsewhere that touches a node of it (paths are split).
    pub fn delete_frame(&mut self, frame: &FrameId, keep_contents: bool) -> Result<(), EditError> {
        let kn = self.frame_kn(frame)?;
        let (start, end) = (kn.span().offset(), kn.span().offset() + self.piece(kn.span()).trim_end().len());
        let (ls, le) = whole_lines(&self.src, start, end);
        let contents = if keep_contents { self.contents_of(kn) } else { String::new() };
        if !contents.is_empty() {
            let nl = if self.src[ls..le].ends_with('\n') { "\n" } else { "" };
            return self.apply(vec![Splice { start: ls, end: le, text: format!("{contents}{nl}") }]);
        }
        let doc = self.document();
        let doomed: Vec<NodeId> =
            doc.nodes.values().filter(|n| doc.frame_within(&n.frame, frame)).map(|n| n.id.clone()).collect();
        // The only statement of its parent's block: the block goes too.
        let parent = doc.frame(frame).and_then(|f| f.parent.as_ref()).and_then(|p| self.frame_statement(p));
        let mut splices = match parent.filter(|pk| pk.children().is_some_and(|c| c.nodes().len() == 1)) {
            Some(pk) => vec![self.block_removal(pk)],
            None => vec![Splice { start: ls, end: le, text: String::new() }],
        };
        if !keep_contents {
            splices.extend(self.edge_cuts(&doomed, |off| (start..end).contains(&off)));
        }
        self.apply(splices)
    }

    /// Moves nodes into a new frame `label`, made where the first of them
    /// is. They must sit at the top of one frame (children go with their
    /// parent); the new frame nests in it. Edges among them move along.
    /// Returns the new frame's id.
    pub fn group(&mut self, ids: &[NodeId], label: &str) -> Result<FrameId, EditError> {
        for id in ids {
            self.node_or_err(id)?;
        }
        let doc = self.document();
        let tops: Vec<&NodeId> = ids.iter().filter(|id| !doc.ancestors(id).any(|a| ids.contains(a))).collect();
        let first = tops.first().ok_or_else(|| EditError::Invalid("nothing to group".into()))?;
        let frame = doc.nodes[*first].frame.clone();
        if tops.iter().any(|id| doc.nodes[*id].parent.is_some() || doc.nodes[*id].frame != frame) {
            return Err(EditError::Invalid("group nodes at the top of one frame".into()));
        }
        let f = self.frame_or_err(&frame)?;
        if f.layout == LayoutKind::Sequence {
            return Err(EditError::Invalid("a sequence frame cannot hold frames".into()));
        }
        let container = if f.implicit { None } else { Some(self.frame_kn(&frame)?) };
        // The grouped nodes and what they hold.
        let mut grouped: Vec<NodeId> = tops.iter().map(|id| (*id).clone()).collect();
        let mut i = 0;
        while i < grouped.len() {
            grouped.extend(doc.nodes[&grouped[i]].children.iter().cloned());
            i += 1;
        }
        let mut splices = Vec::new();
        let mut nodes: Vec<(usize, String, String)> = Vec::new();
        let mut declared = Vec::new();
        for id in &tops {
            match self.statement(id) {
                Some(kn) => {
                    splices.push(self.removal(id, kn));
                    let (text, indent) = self.statement_text(kn);
                    nodes.push((kn.span().offset(), text, indent));
                }
                None => declared.push(format!("node {} id={}", quoted(&doc.nodes[*id].label), kdl_string(id.as_str()))),
            }
        }
        nodes.sort_by_key(|n| n.0);
        // Edge statements of the same frame among the grouped nodes, whole.
        let mut edges: Vec<(usize, String, String)> = Vec::new();
        for (off, ks) in self.edge_statements() {
            let Some(kn) = self.kdl_node_at(off) else { continue };
            let args = kn.entries().iter().filter(|e| e.name().is_none()).count();
            let inside = |k: &usize| grouped.contains(&doc.edges[*k].from) && grouped.contains(&doc.edges[*k].to);
            if self.parent_offset(off) == container.map(|c| c.span().offset())
                && args == ks.len() + 1
                && ks.iter().all(inside)
            {
                let (start, end) = (kn.span().offset(), kn.span().offset() + kn.span().len());
                let (ls, le) = whole_lines(&self.src, start, end);
                splices.push(Splice { start: ls, end: le, text: String::new() });
                let (text, indent) = self.statement_text(kn);
                edges.push((off, text, indent));
            }
        }
        edges.sort_by_key(|e| e.0);
        let indent = match (nodes.first(), container) {
            (Some((off, ..)), _) => line_indent(&self.src, *off),
            (None, Some(c)) => format!("{}    ", line_indent(&self.src, c.span().offset())),
            (None, None) => String::new(),
        };
        let inner = format!("{indent}    ");
        let mut body: Vec<String> =
            nodes.iter().map(|(_, t, old)| format!("{inner}{}", reindent(t, old, &inner))).collect();
        body.extend(declared.iter().map(|l| format!("{inner}{l}")));
        body.extend(edges.iter().map(|(_, t, old)| format!("{inner}{}", reindent(t, old, &inner))));
        let text = format!("{} {{\n{}\n{indent}}}", self.new_frame_line(label), body.join("\n"));
        splices.push(match nodes.first() {
            Some((off, ..)) => {
                let at = self.src[..*off].rfind('\n').map_or(0, |i| i + 1);
                Splice { start: at, end: at, text: format!("{indent}{text}\n") }
            }
            None => self.append_to_frame(&frame, &text)?,
        });
        self.apply_and_find_frame(splices)
    }

    /// Moves a node (with what it holds) to the end of `frame`'s block, or
    /// of the file (`None`: the top level). It becomes a root there.
    pub fn move_to_frame(&mut self, id: &NodeId, frame: Option<&FrameId>) -> Result<(), EditError> {
        let node = self.node_or_err(id)?;
        let target = match frame {
            Some(f) => Some(self.frame_or_err(f)?),
            None => self.document().frames.iter().find(|f| f.implicit),
        };
        if node.parent.is_none() && target.is_some_and(|t| t.id == node.frame) {
            return Ok(());
        }
        let outline = target.is_some_and(|t| matches!(t.layout, LayoutKind::Mindmap | LayoutKind::Tree));
        let keyword = if outline { "-" } else { "node" };
        let label = node.label.clone();
        let mut splices = Vec::new();
        let (text, old) = match self.statement(id) {
            Some(kn) => {
                splices.push(self.removal(id, kn));
                self.statement_text(kn)
            }
            None => (format!("{keyword} {} id={}", quoted(&label), kdl_string(id.as_str())), String::new()),
        };
        splices.push(match frame.and_then(|f| self.frame_statement(f)) {
            Some(fk) => {
                let inner = format!("{}    ", line_indent(&self.src, fk.span().offset()));
                insert_child(&self.src, fk, &reindent(&text, &old, &inner))
            }
            None => self.at_end(&reindent(&text, &old, "")),
        });
        self.apply(splices)
    }

    /// Sets the document's `title`, or removes it with `None`.
    pub fn set_title(&mut self, title: Option<&str>) -> Result<(), EditError> {
        self.set_statement("title", title.map(quoted), &["rhizo"])
    }

    /// Sets the document's `theme`, or removes it with `None`.
    pub fn set_theme(&mut self, theme: Option<&str>) -> Result<(), EditError> {
        self.set_statement("theme", theme.map(quoted), HEAD)
    }

    /// Sets the `layout` of the nodes outside frames, or removes it with `None`.
    pub fn set_top_layout(&mut self, layout: Option<LayoutKind>) -> Result<(), EditError> {
        self.set_statement("layout", layout.map(|l| l.name().to_string()), HEAD)
    }

    /// Sets the `direction` of the nodes outside frames, or removes it with `None`.
    pub fn set_top_direction(&mut self, direction: Option<Direction>) -> Result<(), EditError> {
        self.set_statement("direction", direction.map(|d| d.name().to_string()), HEAD)
    }

    /// Removes the `use <kit>` line.
    pub fn unuse_kit(&mut self, kit: &str) -> Result<(), EditError> {
        let splices = self.top_removals("use", |n| first_string(n) == Some(kit));
        self.apply(splices)
    }

    /// Removes the `import` whose alias is `alias`.
    pub fn remove_import(&mut self, alias: &str) -> Result<(), EditError> {
        let splices = self.top_removals("import", |n| {
            let named = n.entries().iter().find(|e| e.name().is_some_and(|k| k.value() == "as"));
            match named.and_then(|e| e.value().as_string()) {
                Some(a) => a == alias,
                None => first_string(n).is_some_and(|p| crate::kdl::import_alias(p) == alias),
            }
        });
        self.apply(splices)
    }

    /// Aliases of imports no node of this file refers to any more.
    pub fn unused_imports(&self) -> Vec<String> {
        let doc = self.document();
        let used = |a: &str| doc.nodes.keys().any(|id| id.as_str().strip_prefix(a).is_some_and(|r| r.starts_with(':')));
        doc.imports.keys().filter(|a| !used(a)).cloned().collect()
    }

    /// Sets prop `key` of `document().edges[k]` to `value`, as
    /// [`Editor::set_prop`] does for nodes. A hop of a path (`edge a b c`)
    /// is split out first, so only it gets the prop.
    pub fn set_edge_prop(&mut self, k: usize, key: &str, value: &serde_json::Value) -> Result<(), EditError> {
        if !prop_key_ok(key) {
            return Err(EditError::InvalidKey(key.to_string()));
        }
        self.edge_statement(k)?;
        if self.document().edges[k].props.get(key) == Some(value) {
            return Ok(());
        }
        self.on_own_edge(k, |e| {
            let (kn, _, _) = e.edge_statement(k)?;
            let mut splices = e.v2_header("");
            splices.extend(e.prop_splices(kn, key, value));
            e.apply(merged(splices))
        })
    }

    /// The splices that set `key` to `value` in the `props` block of `kn`.
    fn prop_splices(&self, kn: &KdlNode, key: &str, value: &serde_json::Value) -> Vec<Splice> {
        let mut splices = Vec::new();
        let child_of = |pn: &KdlNode| {
            pn.children().and_then(|c| c.nodes().iter().rfind(|c| c.name().value() == key)).map(|c| c.span())
        };
        let Some(text) = jik_scalar(value) else {
            // A list or object: a child of `props`.
            let lines = jik_lines(key, value);
            let Some(pn) = props_statement(kn) else {
                let inner = format!("{}    ", line_indent(&self.src, kn.span().offset()));
                let body: Vec<String> = lines.iter().map(|l| format!("{inner}    {l}")).collect();
                splices.push(insert_child(&self.src, kn, &format!("props {{\n{}\n{inner}}}", body.join("\n"))));
                return splices;
            };
            match child_of(pn) {
                Some(span) => {
                    let indent = line_indent(&self.src, span.offset());
                    let start = span.offset();
                    let end = start + self.piece(span).trim_end().len();
                    splices.push(Splice { start, end, text: lines.join(&format!("\n{indent}")) });
                }
                None => {
                    splices.extend(self.set_entry(pn, key, None));
                    let indent = format!("{}    ", line_indent(&self.src, pn.span().offset()));
                    splices.push(insert_child(&self.src, pn, &lines.join(&format!("\n{indent}"))));
                }
            }
            return splices;
        };
        let entry = format!("{}={text}", kdl_string(key));
        match props_statement(kn) {
            None => splices.push(insert_child(&self.src, kn, &format!("props {entry}"))),
            // `servers "a"` (or a block) written as a child: it becomes `key=value` in place.
            Some(pn) => match child_of(pn) {
                Some(span) => {
                    let start = span.offset();
                    let end = start + self.piece(span).trim_end().len();
                    splices.push(Splice { start, end, text: format!("{} {text}", kdl_string(key)) });
                }
                None => splices.extend(self.set_entry(pn, key, Some(entry))),
            },
        }
        splices
    }

    /// Removes prop `key` of a node; the `props` block goes with its last prop.
    pub fn remove_prop(&mut self, id: &NodeId, key: &str) -> Result<(), EditError> {
        if !self.node_or_err(id)?.props.contains_key(key) {
            return Ok(());
        }
        let kn = self.statement(id).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        let splice = self.prop_removal(kn, key).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        self.apply(vec![splice])
    }

    /// Removes prop `key` of an explicit frame, as [`Editor::remove_prop`] does for nodes.
    pub fn remove_frame_prop(&mut self, frame: &FrameId, key: &str) -> Result<(), EditError> {
        if !self.frame_or_err(frame)?.props.contains_key(key) {
            return Ok(());
        }
        let kn = self.frame_kn(frame)?;
        let splice = self.prop_removal(kn, key).ok_or_else(|| EditError::UnknownFrame(frame.clone()))?;
        self.apply(vec![splice])
    }

    /// Removes prop `key` of `document().edges[k]`; a hop of a path is split out first.
    pub fn remove_edge_prop(&mut self, k: usize, key: &str) -> Result<(), EditError> {
        if !self.document().edges.get(k).ok_or(EditError::UnknownEdge(k))?.props.contains_key(key) {
            return Ok(());
        }
        self.on_own_edge(k, |e| {
            let (kn, _, _) = e.edge_statement(k)?;
            let splice = e.prop_removal(kn, key).ok_or(EditError::NoEdgeStatement(k))?;
            e.apply(vec![splice])
        })
    }

    /// The splice that removes prop `key` from the `props` block of `kn`.
    fn prop_removal(&self, kn: &KdlNode, key: &str) -> Option<Splice> {
        let pn = props_statement(kn)?;
        let children = pn.children().map(|c| c.nodes()).unwrap_or_default();
        let named = pn.entries().iter().filter(|e| e.name().is_some()).count();
        if named + children.len() == 1 {
            // The last prop: the whole `props`, and the node's `{ }` if nothing else is in it.
            return Some(self.child_removal(kn, pn));
        }
        match children.iter().rfind(|c| c.name().value() == key) {
            // The last child: its block goes too, so no empty `{ }` remains.
            Some(_) if children.len() == 1 => Some(self.block_removal(pn)),
            Some(child) => Some(self.child_removal(pn, child)),
            None => self.set_entry(pn, key, None),
        }
    }

    /// Removes `child` from `kn`'s block; the block goes with its last child.
    fn child_removal(&self, kn: &KdlNode, child: &KdlNode) -> Splice {
        if kn.children().is_some_and(|c| c.nodes().len() == 1) {
            return self.block_removal(kn);
        }
        let (start, end) = whole_lines(&self.src, child.span().offset(), child.span().offset() + child.span().len());
        Splice { start, end, text: String::new() }
    }

    /// Removes the `{ … }` of `kn`.
    fn block_removal(&self, kn: &KdlNode) -> Splice {
        let end = kn.span().offset() + self.piece(kn.span()).trim_end().len();
        Splice { start: after_entries(kn), end, text: String::new() }
    }

    // ------------------------------------------------------------------

    /// Replaces, adds or (with `None`) removes the `key=…` entry of `kn`.
    fn set_entry(&self, kn: &KdlNode, key: &str, text: Option<String>) -> Option<Splice> {
        let entry = kn.entries().iter().rfind(|e| e.name().is_some_and(|n| n.value() == key));
        match (entry, text) {
            (Some(e), Some(text)) => {
                let start = e.span().offset();
                Some(Splice { start, end: start + self.piece(e.span()).trim_end().len(), text })
            }
            (Some(e), None) => {
                // With the spaces before it, so no double space is left.
                let end = e.span().offset() + self.piece(e.span()).trim_end().len();
                let start = self.src[..e.span().offset()].trim_end_matches([' ', '\t']).len();
                Some(Splice { start, end, text: String::new() })
            }
            (None, Some(text)) => {
                let at = after_entries(kn);
                let at = self.src[..at].trim_end_matches([' ', '\t']).len();
                Some(Splice { start: at, end: at, text: format!(" {text}") })
            }
            (None, None) => None,
        }
    }

    /// What makes the header ready for version 2 features: `rhizo version=2`
    /// (added, or raised from 1), then `line` (an `import` or a `use`, with
    /// its newline; may be empty) after the last header statement.
    fn v2_header(&self, line: &str) -> Vec<Splice> {
        let mut splices = Vec::new();
        let top = self.state.kdl.as_ref().map(|k| k.nodes()).unwrap_or_default();
        match top.iter().find(|n| n.name().value() == "rhizo") {
            None => splices.push(Splice { start: 0, end: 0, text: format!("rhizo version=2\n{line}") }),
            Some(h) => {
                let version = h.entries().iter().find(|e| e.name().is_some_and(|n| n.value() == "version"));
                if let Some(e) = version.filter(|e| e.value().as_integer() == Some(1)) {
                    let start = e.span().offset();
                    splices.push(Splice { start, end: start + e.span().len(), text: "version=2".into() });
                }
                if !line.is_empty() {
                    let heads = ["rhizo", "title", "use", "import"];
                    let after = top.iter().rfind(|n| heads.contains(&n.name().value())).unwrap_or(h);
                    let end = after.span().offset() + after.span().len();
                    let (_, le) = whole_lines(&self.src, after.span().offset(), end);
                    let lead = if self.src[..le].ends_with('\n') { "" } else { "\n" };
                    splices.push(Splice { start: le, end: le, text: format!("{lead}{line}") });
                }
            }
        }
        splices
    }

    /// Removes a node's statement. When it is the only statement in its
    /// parent's block, the whole `{ … }` goes, so no empty braces remain.
    fn removal(&self, id: &NodeId, kn: &KdlNode) -> Splice {
        let parent = self.document().nodes[id].parent.as_ref().and_then(|p| self.statement(p));
        if let Some(pk) = parent.filter(|pk| pk.children().is_some_and(|c| c.nodes().len() == 1)) {
            let start = after_entries(pk);
            let end = pk.span().offset() + pk.span().len();
            return Splice { start, end, text: String::new() };
        }
        let (start, end) = whole_lines(&self.src, kn.span().offset(), kn.span().offset() + kn.span().len());
        Splice { start, end, text: String::new() }
    }

    /// The statement edge `k` came from, its number of hops, and which hop
    /// `k` is.
    fn edge_statement(&self, k: usize) -> Result<(&KdlNode, usize, usize), EditError> {
        let spans = &self.state.lowered.edge_spans;
        let (off, _) = spans.get(k).ok_or(EditError::UnknownEdge(k))?.ok_or(EditError::NoEdgeStatement(k))?;
        let kn = self.kdl_node_at(off).ok_or(EditError::NoEdgeStatement(k))?;
        let same: Vec<usize> = (0..spans.len()).filter(|&i| spans[i].is_some_and(|(o, _)| o == off)).collect();
        // A path that lowered only in part has no reliable hop numbering.
        if kn.entries().iter().filter(|e| e.name().is_none()).count() != same.len() + 1 {
            return Err(EditError::NoEdgeStatement(k));
        }
        Ok((kn, same.len(), same.iter().position(|&i| i == k).expect("k shares its own span")))
    }

    /// An edge statement's properties as written, each after a space.
    /// `Some((key, entry))` replaces, adds or (`entry` `None`) drops the
    /// `key=…` entry; `entry` is the whole `key=value` text.
    fn edge_props(&self, kn: &KdlNode, set: Option<(&str, Option<&str>)>) -> String {
        let mut out = String::new();
        let mut written = false;
        for e in kn.entries().iter().filter(|e| e.name().is_some()) {
            match set {
                Some((key, entry)) if e.name().is_some_and(|n| n.value() == key) => {
                    written = true;
                    out.extend(entry.map(|t| format!(" {t}")));
                }
                _ => out.push_str(&format!(" {}", self.piece(e.span()))),
            }
        }
        if let (Some((_, Some(t))), false) = (set, written) {
            out.push_str(&format!(" {t}"));
        }
        out
    }

    /// Rewrites an edge statement as one `edge` per segment (argument indices
    /// plus properties text), each with the original block, or removes it
    /// when there are no segments. Comments around it stay.
    fn split_path(&self, kn: &KdlNode, segs: Vec<(Vec<usize>, String)>) -> Splice {
        let start = kn.span().offset();
        let end = start + self.piece(kn.span()).trim_end().len();
        if segs.is_empty() {
            let (start, end) = whole_lines(&self.src, start, end);
            return Splice { start, end, text: String::new() };
        }
        let args: Vec<_> = kn.entries().iter().filter(|e| e.name().is_none()).collect();
        let block = if kn.children().is_some() { &self.src[after_entries(kn)..end] } else { "" };
        let lines: Vec<String> = segs
            .iter()
            .map(|(seg, props)| {
                let a: Vec<&str> = seg.iter().map(|&i| self.piece(args[i].span())).collect();
                format!("edge {}{props}{block}", a.join(" "))
            })
            .collect();
        Splice { start, end, text: lines.join(&format!("\n{}", line_indent(&self.src, start))) }
    }

    fn frame_or_err(&self, frame: &FrameId) -> Result<&rhizo_core::Frame, EditError> {
        self.document().frame(frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))
    }

    /// The statement of an explicit frame; the implicit one has none.
    fn frame_kn(&self, frame: &FrameId) -> Result<&KdlNode, EditError> {
        self.frame_statement(frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))
    }

    /// Replaces, adds or (`None`) removes the `key=value` entry of a frame.
    fn set_frame_entry(
        &mut self,
        frame: &FrameId,
        key: &str,
        value: Option<String>,
        v2: bool,
    ) -> Result<(), EditError> {
        let kn = self.frame_kn(frame)?;
        let mut splices = if v2 && value.is_some() { self.v2_header("") } else { Vec::new() };
        splices.extend(self.set_entry(kn, key, value.map(|v| format!("{}={v}", kdl_string(key)))));
        self.apply(merged(splices))
    }

    /// `frame "<label>"`, with an `id` when another frame has the label's
    /// slug already, so no frame's id changes.
    fn new_frame_line(&self, label: &str) -> String {
        let taken = |c: &str| self.document().frames.iter().any(|f| f.id.as_str() == c);
        let slug = slugify(label);
        let id = unique(if slug.is_empty() { "frame" } else { &slug }, taken);
        if id == slug {
            format!("frame {}", quoted(label))
        } else {
            format!("frame {} id={}", quoted(label), kdl_string(&id))
        }
    }

    /// Applies `splices` and returns the explicit frame they made.
    fn apply_and_find_frame(&mut self, splices: Vec<Splice>) -> Result<FrameId, EditError> {
        let before: Vec<FrameId> = self.document().frames.iter().map(|f| f.id.clone()).collect();
        self.apply(splices)?;
        let made = self.document().frames.iter().find(|f| !f.implicit && !before.contains(&f.id));
        Ok(made.expect("the new frame lowered").id.clone())
    }

    /// Where a statement's label is, and `label` as it replaces it there:
    /// the name of a string-named node, a `label=` entry, or the first argument.
    fn label_place(&self, kn: &KdlNode, label: &str) -> Option<(usize, usize, String)> {
        let label_prop = kn.entries().iter().find(|e| e.name().is_some_and(|n| n.value() == "label"));
        let first_arg = kn.entries().iter().find(|e| e.name().is_none());
        let end = |s: miette::SourceSpan| s.offset() + self.piece(s).trim_end().len();
        if string_named(kn) {
            let s = kn.name().span();
            Some((s.offset(), end(s), quoted(label)))
        } else if let Some(e) = label_prop {
            Some((e.span().offset(), end(e.span()), format!("label={}", quoted(label))))
        } else {
            first_arg.map(|e| (e.span().offset(), end(e.span()), quoted(label)))
        }
    }

    /// What a frame's block holds but its `props`, as text indented like the frame.
    fn contents_of(&self, kn: &KdlNode) -> String {
        let Some(block) = kn.children() else { return String::new() };
        let indent = line_indent(&self.src, kn.span().offset());
        let end = kn.span().offset() + self.piece(kn.span()).trim_end().len();
        let open = after_entries(kn) + self.src[after_entries(kn)..].find('{').expect("a block starts with `{`");
        let close = self.src[..end].rfind('}').expect("a block ends with `}`");
        let mut inner = String::new();
        let mut at = open + 1;
        for p in block.nodes().iter().filter(|c| c.name().value() == "props") {
            let (s, e) = whole_lines(&self.src, p.span().offset(), p.span().offset() + p.span().len());
            inner.push_str(&self.src[at..s.max(at)]);
            at = e.min(close).max(at);
        }
        inner.push_str(&self.src[at..close]);
        // The rest of the `{` line, then lines indented one level deeper.
        let mut lines = inner.lines();
        let first = lines.next().unwrap_or_default().trim();
        let rest: Vec<&str> = lines.map(str::trim_end).collect();
        let cut = rest.iter().filter(|l| !l.is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
        let mut out: Vec<String> = Vec::new();
        if !first.is_empty() {
            out.push(format!("{indent}{first}"));
        }
        out.extend(rest.iter().map(|l| if l.is_empty() { String::new() } else { format!("{indent}{}", &l[cut..]) }));
        while out.first().is_some_and(String::is_empty) {
            out.remove(0);
        }
        while out.last().is_some_and(String::is_empty) {
            out.pop();
        }
        out.join("\n")
    }

    /// The offset of the statement whose block holds the one at `offset`;
    /// `None` at the top level.
    fn parent_offset(&self, offset: usize) -> Option<usize> {
        fn find(doc: &KdlDocument, parent: Option<usize>, offset: usize) -> Option<Option<usize>> {
            doc.nodes().iter().find_map(|n| {
                if n.span().offset() == offset {
                    return Some(parent);
                }
                n.children().and_then(|c| find(c, Some(n.span().offset()), offset))
            })
        }
        find(self.state.kdl.as_ref()?, None, offset).flatten()
    }

    /// The node's statement and its `i`-th `accept` child.
    fn accept_statement(&self, id: &NodeId, i: usize) -> Result<(&KdlNode, &KdlNode), EditError> {
        self.node_or_err(id)?;
        let kn = self.statement(id).ok_or_else(|| EditError::NoStatement(id.clone()))?;
        let a = kn.children().into_iter().flat_map(|c| c.nodes()).filter(|c| c.name().value() == "accept").nth(i);
        let a = a.ok_or_else(|| EditError::Invalid(format!("`{id}` has no acceptance criterion {}", i + 1)))?;
        Ok((kn, a))
    }

    /// Replaces the value of the top-level statement `name`, adds it after
    /// the last of the statements `after` (or at the top), or (`None`) removes it.
    fn set_statement(&mut self, name: &str, value: Option<String>, after: &[&str]) -> Result<(), EditError> {
        let top = self.state.kdl.as_ref().map(|k| k.nodes()).unwrap_or_default();
        let splice = match (top.iter().rfind(|n| n.name().value() == name), value) {
            (Some(n), Some(v)) => match n.entries().iter().find(|e| e.name().is_none()) {
                Some(e) => {
                    let start = e.span().offset();
                    Splice { start, end: start + self.piece(e.span()).trim_end().len(), text: v }
                }
                None => {
                    let at = after_entries(n);
                    Splice { start: at, end: at, text: format!(" {v}") }
                }
            },
            (Some(_), None) => return self.apply(self.top_removals(name, |_| true)),
            (None, Some(v)) => match top.iter().rfind(|n| after.contains(&n.name().value())) {
                Some(a) => {
                    let (_, le) = whole_lines(&self.src, a.span().offset(), a.span().offset() + a.span().len());
                    let lead = if self.src[..le].ends_with('\n') { "" } else { "\n" };
                    Splice { start: le, end: le, text: format!("{lead}{name} {v}\n") }
                }
                None => Splice { start: 0, end: 0, text: format!("{name} {v}\n") },
            },
            (None, None) => return Ok(()),
        };
        self.apply(vec![splice])
    }

    /// Removals of the top-level statements `name` that `pick` accepts.
    fn top_removals(&self, name: &str, pick: impl Fn(&KdlNode) -> bool) -> Vec<Splice> {
        let top = self.state.kdl.as_ref().map(|k| k.nodes()).unwrap_or_default();
        top.iter()
            .filter(|n| n.name().value() == name && pick(n))
            .map(|n| {
                let (start, end) = whole_lines(&self.src, n.span().offset(), n.span().offset() + n.span().len());
                Splice { start, end, text: String::new() }
            })
            .collect()
    }

    /// `line` as a new last line of the file.
    fn at_end(&self, line: &str) -> Splice {
        let at = self.src.len();
        let lead = if self.src.is_empty() || self.src.ends_with('\n') { "" } else { "\n" };
        Splice { start: at, end: at, text: format!("{lead}{line}\n") }
    }

    fn piece(&self, s: miette::SourceSpan) -> &str {
        &self.src[s.offset()..s.offset() + s.len()]
    }

    fn node_or_err(&self, id: &NodeId) -> Result<&rhizo_core::Node, EditError> {
        self.document().node(id).ok_or_else(|| EditError::UnknownNode(id.clone()))
    }

    fn statement(&self, id: &NodeId) -> Option<&KdlNode> {
        let (off, _) = self.state.lowered.node_spans.get(id).copied().flatten()?;
        self.kdl_node_at(off)
    }

    fn kdl_node_at(&self, offset: usize) -> Option<&KdlNode> {
        fn find(doc: &KdlDocument, offset: usize) -> Option<&KdlNode> {
            for n in doc.nodes() {
                if n.span().offset() == offset {
                    return Some(n);
                }
                if let Some(found) = n.children().and_then(|c| find(c, offset)) {
                    return Some(found);
                }
            }
            None
        }
        find(self.state.kdl.as_ref()?, offset)
    }

    fn implicit_frame_id(&self) -> FrameId {
        self.document()
            .frames
            .iter()
            .find(|f| f.implicit)
            .map_or_else(|| FrameId::from(crate::lower::IMPLICIT_FRAME), |f| f.id.clone())
    }

    /// The `frame` statement of an explicit frame. Explicit frames lower in
    /// source order, parents before the frames inside them, so the n-th
    /// explicit frame is the n-th `frame` node in that order.
    fn frame_statement(&self, frame: &FrameId) -> Option<&KdlNode> {
        let n = self.document().frames.iter().filter(|f| !f.implicit).position(|f| &f.id == frame)?;
        let is_frame = |k: &&KdlNode| k.name().value() == "frame";
        let mut stack: Vec<&KdlNode> = self.state.kdl.as_ref()?.nodes().iter().filter(is_frame).rev().collect();
        let mut seen = 0;
        while let Some(k) = stack.pop() {
            if seen == n {
                return Some(k);
            }
            seen += 1;
            stack.extend(k.children().map(|c| c.nodes()).unwrap_or_default().iter().filter(is_frame).rev());
        }
        None
    }

    fn append_to_frame(&self, frame: &FrameId, line: &str) -> Result<Splice, EditError> {
        let f = self.document().frame(frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))?;
        if f.implicit || self.document().frames.is_empty() {
            return Ok(self.at_end(line));
        }
        let kn = self.frame_statement(frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))?;
        Ok(insert_child(&self.src, kn, line))
    }

    /// A statement for a node that so far exists only through an edge.
    fn declare(&self, id: &NodeId, label: &str, extra: &str) -> Result<Splice, EditError> {
        let frame = self.document().nodes[id].frame.clone();
        let keyword = if self
            .document()
            .frame(&frame)
            .is_some_and(|f| matches!(f.layout, LayoutKind::Mindmap | LayoutKind::Tree))
        {
            "-"
        } else {
            "node"
        };
        self.append_to_frame(&frame, &format!("{keyword} {} id={}{extra}", quoted(label), kdl_string(id.as_str())))
    }

    fn apply(&mut self, mut splices: Vec<Splice>) -> Result<(), EditError> {
        if splices.is_empty() {
            return Ok(());
        }
        // Ties: the removal (wider) first, then the insertion at its start.
        splices.sort_by_key(|s| std::cmp::Reverse((s.start, s.end)));
        let mut src = self.src.clone();
        let mut limit = usize::MAX;
        for s in &splices {
            if s.end > limit {
                return Err(EditError::Invalid("the edit overlaps itself; make it in the text".into()));
            }
            src.replace_range(s.start..s.end, &s.text);
            limit = s.start;
        }
        if src == self.src {
            return Ok(()); // nothing changed: nothing to undo
        }
        let state = analyze(&src);
        if error_count(&state) > error_count(&self.state) {
            let first =
                state.lowered.diagnostics.iter().find(|d| d.is_error()).map(|d| d.message.clone()).unwrap_or_default();
            return Err(EditError::Invalid(first));
        }
        self.undo.push(std::mem::replace(&mut self.src, src));
        self.redo.clear();
        self.state = state;
        Ok(())
    }

    /// Applies an insertion and returns the id of the node it created.
    fn apply_and_find(&mut self, splice: Splice, line: &str) -> Result<NodeId, EditError> {
        let at = splice.start + splice.text.find(line).expect("the line is part of the splice");
        self.apply(vec![splice])?;
        let found = self.state.lowered.node_spans.iter().find(|(_, s)| s.is_some_and(|(o, _)| o == at));
        Ok(found.map(|(id, _)| id.clone()).expect("the inserted node lowered"))
    }
}

/// Joins insertions at the same place in the order they were made (apply
/// would reverse them), without doubling the newline between them.
fn merged(splices: Vec<Splice>) -> Vec<Splice> {
    let mut out: Vec<Splice> = Vec::new();
    for s in splices {
        match out.iter_mut().find(|m| m.start == s.start && m.end == m.start && s.end == s.start) {
            Some(m) => m.text.push_str(if m.text.ends_with('\n') { s.text.trim_start_matches('\n') } else { &s.text }),
            None => out.push(s),
        }
    }
    out
}

/// A look's value as written: a string bare where it can be, a number or a boolean.
fn look_text(key: &str, value: &serde_json::Value) -> Result<String, EditError> {
    match value {
        serde_json::Value::String(s) => Ok(kdl_string(s)),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => Ok(jik_scalar(value).unwrap_or_default()),
        _ => Err(EditError::Invalid(format!("`{key}` takes a string, number or boolean"))),
    }
}

/// `base`, or `base-2`, `base-3`… the first that is not `taken`, as lowering derives ids.
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base}-{i}")).find(|c| !taken(c)).expect("endless")
}

/// `text`, a statement from a line indented `old`, for a place indented
/// `new`: the lines after the first move with it.
fn reindent(text: &str, old: &str, new: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(i, l)| match l.strip_prefix(old) {
            Some(rest) if i > 0 => format!("{new}{rest}"),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Statements of the header, after which `layout`, `theme`… go.
const HEAD: &[&str] = &["rhizo", "title", "theme", "use", "base", "prefix", "import", "layout", "direction"];

/// A node's `props` block; the last one counts, as in lowering.
fn props_statement(kn: &KdlNode) -> Option<&KdlNode> {
    kn.children()?.nodes().iter().rfind(|c| c.name().value() == "props")
}

/// A prop key the panel and agents may write: not empty, no control
/// characters, no spaces around it.
fn prop_key_ok(key: &str) -> bool {
    !key.is_empty() && key.trim() == key && !key.chars().any(char::is_control)
}

/// An import alias for `path`: its file stem, made a valid prefix.
fn alias_for(path: &str) -> String {
    let stem: String = crate::kdl::import_alias(path)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect();
    if stem.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') { stem } else { format!("f{stem}") }
}

/// The first argument of a statement, as a string.
fn first_string(n: &KdlNode) -> Option<&str> {
    n.entries().iter().find(|e| e.name().is_none()).and_then(|e| e.value().as_string())
}

fn string_named(n: &KdlNode) -> bool {
    n.name().repr().is_some_and(|r| r.starts_with('"') || r.starts_with('#'))
}

/// Where new entries go: right after the last entry, or after the name.
fn after_entries(n: &KdlNode) -> usize {
    let s = n.entries().last().map_or(n.name().span(), |e| e.span());
    s.offset() + s.len()
}

/// Leading whitespace of the line `offset` is on.
fn line_indent(src: &str, offset: usize) -> String {
    let start = src[..offset].rfind('\n').map_or(0, |i| i + 1);
    src[start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// Widens `start..end` to whole lines when nothing else is on them, so a
/// removal leaves no blank line or stray `;` behind.
fn whole_lines(src: &str, start: usize, end: usize) -> (usize, usize) {
    let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
    let rest = &src[end..];
    let line_end = rest.find('\n').map_or(src.len(), |i| end + i + 1);
    let before_blank = src[line_start..start].trim().is_empty();
    let after = src[end..line_end].trim();
    // A trailing `// comment` belongs to the statement it follows.
    if before_blank && (after.is_empty() || after == ";" || after.starts_with("//")) {
        (line_start, line_end)
    } else {
        // Shares the line (`- "A"; - "B"`): take the node with the `;` after
        // it and the spaces after that, or else the `;` before it.
        let end = start + src[start..end].trim_end().len();
        let rest = &src[end..];
        if let Some(i) = rest.find(|c: char| !c.is_whitespace()).filter(|&i| rest[i..].starts_with(';')) {
            let after = &rest[i + 1..];
            let spaces = after.len() - after.trim_start_matches([' ', '\t']).len();
            return (start, end + i + 1 + spaces);
        }
        let before = src[..start].trim_end_matches([' ', '\t']);
        if before.ends_with(';') {
            return (before.len() - 1, end);
        }
        (start, end)
    }
}

/// Where text goes for a placement relative to `target`'s statement.
/// `text_for` gets the indentation the text should use for nested lines.
fn placement_splice(src: &str, to: &Placement, target: &KdlNode, text_for: impl Fn(&str) -> String) -> Splice {
    let (t_start, t_end) = (target.span().offset(), target.span().offset() + target.span().len());
    let t_indent = line_indent(src, t_start);
    match to {
        Placement::Child(_) => insert_child(src, target, &text_for(&format!("{t_indent}    "))),
        Placement::Before(_) => {
            let line_start = src[..t_start].rfind('\n').map_or(0, |i| i + 1);
            if src[line_start..t_start].trim().is_empty() {
                Splice { start: line_start, end: line_start, text: format!("{t_indent}{}\n", text_for(&t_indent)) }
            } else {
                Splice { start: t_start, end: t_start, text: format!("{}; ", text_for(&t_indent)) }
            }
        }
        Placement::After(_) => {
            let (_, le) = whole_lines(src, t_start, t_end);
            let rest = src[t_end..le.max(t_end)].trim();
            // The node ends its line (a trailing comment still counts).
            if le > t_end && (rest.is_empty() || rest.starts_with("//")) {
                Splice { start: le, end: le, text: format!("{t_indent}{}\n", text_for(&t_indent)) }
            } else {
                let end = src[t_start..t_end].trim_end().len() + t_start;
                Splice { start: end, end, text: format!("; {}", text_for(&t_indent)) }
            }
        }
    }
}

/// Inserts `line` as the last child of `parent`, matching its indentation.
fn insert_child(src: &str, parent: &KdlNode, line: &str) -> Splice {
    let indent = line_indent(src, parent.span().offset());
    let child = format!("{indent}    ");
    let span_end = parent.span().offset() + parent.span().len();
    match parent.children() {
        None => {
            let at = after_entries(parent);
            Splice { start: at, end: at, text: format!(" {{\n{child}{line}\n{indent}}}") }
        }
        Some(block) => {
            let close = src[..span_end].rfind('}').expect("a block ends with `}`");
            let close_line = src[..close].rfind('\n').map_or(0, |i| i + 1);
            if src[close_line..close].trim().is_empty() && close_line > parent.span().offset() {
                Splice { start: close_line, end: close_line, text: format!("{child}{line}\n") }
            } else if block.nodes().is_empty() {
                let open = src[..close].rfind('{').expect("a block starts with `{`");
                Splice { start: open + 1, end: close, text: format!(" {line} ") }
            } else {
                Splice { start: close, end: close, text: format!("; {line} ") }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"rhizo version=1
// A comment that must survive every edit.
title "Plan"

- "Root" {
    - "Users" { - "Personas"; - "Jobs" }
    - "Pricing" class=risk   // trailing comment
}

frame "Flow" layout=layered {
    node "Build" shape=pill
    edge build "Test" "Ship" label="go"
}

edge pricing build
"#;

    fn ed() -> Editor {
        let e = Editor::open(SRC);
        assert!(!e.diagnostics().iter().any(Diagnostic::is_error), "{:#?}", e.diagnostics());
        e
    }

    fn id(s: &str) -> NodeId {
        NodeId::from(s)
    }

    /// Lines removed and added between two sources, ignoring order.
    fn diff(a: &str, b: &str) -> (Vec<String>, Vec<String>) {
        let al: Vec<&str> = a.lines().collect();
        let bl: Vec<&str> = b.lines().collect();
        let removed = al.iter().filter(|l| !bl.contains(l)).map(|l| l.to_string()).collect();
        let added = bl.iter().filter(|l| !al.contains(l)).map(|l| l.to_string()).collect();
        (removed, added)
    }

    #[test]
    fn rename_touches_one_line_and_keeps_the_id() {
        let mut e = ed();
        e.rename(&id("pricing"), "Pricing model").unwrap();
        let (removed, added) = diff(SRC, e.source());
        assert_eq!(removed, vec!["    - \"Pricing\" class=risk   // trailing comment"]);
        assert_eq!(added, vec!["    - \"Pricing model\" id=pricing class=risk   // trailing comment"]);
        assert_eq!(e.document().nodes[&id("pricing")].label, "Pricing model");
        assert_eq!(e.document().edges.len(), 3, "edges still resolve");
    }

    #[test]
    fn rename_inside_a_one_liner() {
        let mut e = ed();
        e.rename(&id("jobs"), "Jobs to be done").unwrap();
        assert!(e.source().contains("- \"Users\" { - \"Personas\"; - \"Jobs to be done\" id=jobs }"));
    }

    #[test]
    fn move_adds_then_updates_position() {
        let mut e = ed();
        e.move_node(&id("build"), 10.0, 20.5).unwrap();
        assert!(e.source().contains("node \"Build\" shape=pill x=10 y=20.5\n"));
        e.move_node(&id("build"), 30.0, 40.0).unwrap();
        assert!(e.source().contains("node \"Build\" shape=pill x=30 y=40\n"));
        let at = e.document().nodes[&id("build")].at.unwrap();
        assert_eq!((at.x, at.y), (30.0, 40.0));
    }

    #[test]
    fn moving_an_edge_created_node_declares_it() {
        let mut e = ed();
        e.move_node(&id("ship"), 5.0, 5.0).unwrap();
        assert!(e.source().contains("    node \"Ship\" id=ship x=5 y=5\n}"));
        assert_eq!(e.document().nodes.len(), 8);
    }

    #[test]
    fn add_child_and_sibling_shapes() {
        let mut e = ed();
        let a = e.add_child(&id("pricing"), "Tiers").unwrap();
        assert_eq!(e.document().nodes[&a].parent.as_ref(), Some(&id("pricing")));
        assert!(
            e.source().contains("    - \"Pricing\" class=risk {\n        - \"Tiers\"\n    }   // trailing comment")
        );
        let b = e.add_child(&id("root"), "Metrics").unwrap();
        assert!(e.source().contains("    - \"Metrics\"\n}"), "{}", e.source());
        assert_eq!(e.document().nodes[&b].parent.as_ref(), Some(&id("root")));
        let c = e.add_child(&id("users"), "Admins").unwrap();
        assert!(e.source().contains("{ - \"Personas\"; - \"Jobs\" ; - \"Admins\" }"), "{}", e.source());
        assert_eq!(e.document().nodes[&c].parent.as_ref(), Some(&id("users")));
        assert!(e.source().contains("// A comment that must survive every edit."));
    }

    #[test]
    fn add_node_to_frames() {
        let mut e = ed();
        let n = e.add_node(&FrameId::from("flow"), "Deploy").unwrap();
        assert_eq!(e.document().nodes[&n].frame.as_str(), "flow");
        assert!(e.source().contains("    node \"Deploy\"\n}"));
        let m = e.add_node(&FrameId::from("main"), "Loose idea").unwrap();
        assert!(e.source().ends_with("edge pricing build\n- \"Loose idea\"\n"));
        assert_eq!(e.document().nodes[&m].frame.as_str(), "main");
    }

    #[test]
    fn add_node_to_nested_frames() {
        let src = "frame \"Outer\" {\n    node \"A\"\n    frame \"Inner\" {\n        node \"B\"\n    }\n}\nframe \"Next\" {\n    node \"C\"\n}\n";
        let mut e = Editor::open(src);
        let n = e.add_node(&FrameId::from("inner"), "D").unwrap();
        assert_eq!(e.document().nodes[&n].frame.as_str(), "inner");
        assert!(e.source().contains("        node \"B\"\n        - \"D\"\n    }"), "{}", e.source());
        let m = e.add_node(&FrameId::from("next"), "E").unwrap();
        assert_eq!(e.document().nodes[&m].frame.as_str(), "next");
        let o = e.add_node(&FrameId::from("outer"), "F").unwrap();
        assert_eq!(e.document().nodes[&o].frame.as_str(), "outer");
    }

    #[test]
    fn connect_appends_an_edge() {
        let mut e = ed();
        e.connect(&id("build"), &id("ship")).unwrap();
        assert!(e.source().contains("    edge build ship\n}"));
        e.connect(&id("personas"), &id("build")).unwrap();
        assert!(e.source().ends_with("edge personas build\n"));
        assert_eq!(e.document().edges.len(), 5);
    }

    #[test]
    fn delete_removes_subtree_and_splits_paths() {
        let mut e = ed();
        e.delete(&id("users")).unwrap();
        assert!(!e.source().contains("Users"));
        assert!(!e.source().contains("Personas"));
        assert_eq!(e.document().nodes.len(), 5);

        e.delete(&id("test")).unwrap();
        // `edge build "Test" "Ship"` loses its middle: no path survives.
        assert!(!e.source().contains("edge build"));
        assert!(e.source().contains("edge pricing build"));

        let mut e = ed();
        e.delete(&id("build")).unwrap();
        assert!(e.source().contains("    edge \"Test\" \"Ship\" label=\"go\"\n}"), "{}", e.source());
        assert!(!e.source().contains("edge pricing"));
        assert!(e.source().contains("// A comment that must survive every edit."));
    }

    #[test]
    fn looks_go_on_the_statement_not_into_props() {
        let mut e = Editor::open("rhizo version=1\n- \"A\"\n");
        e.set_look(&id("a"), "shape", &serde_json::json!("diamond")).unwrap();
        e.set_look(&id("a"), "stroke-width", &serde_json::json!(2.5)).unwrap();
        e.set_look(&id("a"), "fill", &serde_json::json!("#e11d48")).unwrap();
        let n = e.document().node(&id("a")).unwrap();
        assert_eq!(n.style.shape, Some(rhizo_core::Shape::Diamond));
        assert_eq!(n.style.stroke_width, Some(2.5));
        assert!(n.props.is_empty(), "{}", e.source());
        e.set_look(&id("a"), "shape", &serde_json::json!("pill")).unwrap();
        assert_eq!(e.source().matches("shape=").count(), 1, "replaced, not added: {}", e.source());
        assert!(e.set_look(&id("a"), "fill", &serde_json::json!([1])).is_err());
    }

    #[test]
    fn undo_and_redo() {
        let mut e = ed();
        e.rename(&id("root"), "Strategy").unwrap();
        e.add_child(&id("root"), "Goals").unwrap();
        assert!(e.undo() && e.undo());
        assert_eq!(e.source(), SRC);
        assert!(!e.undo());
        assert!(e.redo());
        assert!(e.source().contains("Strategy"));
        e.move_node(&id("build"), 1.0, 1.0).unwrap();
        assert!(!e.can_redo(), "a new edit clears redo");
    }

    #[test]
    fn errors_are_refused_and_unknown_ids_reported() {
        let mut e = ed();
        assert_eq!(e.rename(&id("nope"), "x"), Err(EditError::UnknownNode(id("nope"))));
        // Renaming to an empty-quoted label is fine, but a typed feature in a v1 file is not.
        let before = e.source().to_string();
        assert!(e.add_node(&FrameId::from("ghost"), "x").is_err());
        assert_eq!(e.source(), before);
    }

    #[test]
    fn reparent_moves_subtrees_and_reindents() {
        let mut e = ed();
        e.reparent(&id("users"), &Placement::Child(id("pricing"))).unwrap();
        assert!(
            e.source()
                .contains("    - \"Pricing\" class=risk {\n        - \"Users\" { - \"Personas\"; - \"Jobs\" }\n    }"),
            "{}",
            e.source()
        );
        assert_eq!(e.document().nodes[&id("users")].parent.as_ref(), Some(&id("pricing")));
        assert_eq!(e.document().nodes[&id("personas")].parent.as_ref(), Some(&id("users")));
        assert!(e.source().contains("// A comment that must survive every edit."));

        let mut e = ed();
        e.reparent(&id("pricing"), &Placement::Before(id("users"))).unwrap();
        let order: Vec<&str> = e.document().nodes[&id("root")].children.iter().map(|c| c.as_str()).collect();
        assert_eq!(order, vec!["pricing", "users"]);
        assert!(
            e.source().contains("    - \"Pricing\" class=risk   // trailing comment\n    - \"Users\""),
            "{}",
            e.source()
        );

        let mut e = ed();
        e.reparent(&id("personas"), &Placement::After(id("jobs"))).unwrap();
        let order: Vec<&str> = e.document().nodes[&id("users")].children.iter().map(|c| c.as_str()).collect();
        assert_eq!(order, vec!["jobs", "personas"]);
    }

    #[test]
    fn reparent_multiline_reindents_every_line() {
        let src = "- \"Root\" {\n    - \"A\" {\n        - \"A1\"\n    }\n    - \"B\"\n}\n";
        let mut e = Editor::open(src);
        e.reparent(&id("a"), &Placement::Child(id("b"))).unwrap();
        assert_eq!(
            e.source(),
            "- \"Root\" {\n    - \"B\" {\n        - \"A\" {\n            - \"A1\"\n        }\n    }\n}\n"
        );
        assert_eq!(e.document().nodes[&id("a1")].parent.as_ref(), Some(&id("a")));
        assert!(e.undo());
        assert_eq!(e.source(), src);
    }

    #[test]
    fn reparent_refuses_cycles_and_noops() {
        let mut e = ed();
        assert_eq!(e.reparent(&id("users"), &Placement::Child(id("personas"))), Err(EditError::Cycle(id("users"))));
        assert_eq!(e.reparent(&id("root"), &Placement::Child(id("root"))), Err(EditError::Cycle(id("root"))));
        e.reparent(&id("pricing"), &Placement::After(id("users"))).unwrap();
        assert_eq!(e.source(), SRC, "already there");
        assert!(!e.can_undo());
    }

    #[test]
    fn removals_leave_no_empty_blocks_or_double_spaces() {
        let src = "- \"Root\" {\n    - \"Solo\" {\n        - \"Only\"\n    }\n    - \"Row\" { - \"A\"; - \"B\"; - \"C\" }\n    - \"Other\"\n}\n";
        let mut e = Editor::open(src);
        e.reparent(&id("only"), &Placement::Child(id("other"))).unwrap();
        assert!(e.source().contains("    - \"Solo\"\n"), "{}", e.source());
        assert!(e.source().contains("- \"Other\" {\n        - \"Only\"\n    }"), "{}", e.source());

        let mut e = Editor::open(src);
        e.delete(&id("b")).unwrap();
        assert!(e.source().contains("{ - \"A\"; - \"C\" }"), "{}", e.source());
        e.delete(&id("c")).unwrap();
        assert!(e.source().contains("{ - \"A\" }"), "{}", e.source());
        e.delete(&id("a")).unwrap();
        assert!(e.source().contains("    - \"Row\"\n"), "{}", e.source());

        let mut e = Editor::open(src);
        e.reparent(&id("a"), &Placement::After(id("other"))).unwrap();
        assert!(e.source().contains("{ - \"B\"; - \"C\" }"), "{}", e.source());
        assert!(e.source().contains("    - \"Other\"\n    - \"A\"\n"), "{}", e.source());
    }

    #[test]
    fn add_after_inserts_right_after_the_sibling() {
        let mut e = ed();
        let a = e.add_after(&id("users"), "Partners").unwrap();
        let order: Vec<&str> = e.document().nodes[&id("root")].children.iter().map(|c| c.as_str()).collect();
        assert_eq!(order, vec!["users", a.as_str(), "pricing"]);
        assert!(e.source().contains("    - \"Users\" { - \"Personas\"; - \"Jobs\" }\n    - \"Partners\"\n"));

        let b = e.add_after(&id("personas"), "Admins").unwrap();
        assert!(e.source().contains("{ - \"Personas\"; - \"Admins\"; - \"Jobs\" }"), "{}", e.source());
        assert_eq!(e.document().nodes[&b].parent.as_ref(), Some(&id("users")));

        let c = e.add_after(&id("pricing"), "Packaging").unwrap();
        assert!(
            e.source().contains("    - \"Pricing\" class=risk   // trailing comment\n    - \"Packaging\"\n"),
            "{}",
            e.source()
        );
        assert_eq!(e.document().nodes[&c].parent.as_ref(), Some(&id("root")));

        // An edge-created node: the new node joins its frame.
        let d = e.add_after(&id("ship"), "Monitor").unwrap();
        assert_eq!(e.document().nodes[&d].frame.as_str(), "flow");
    }

    /// Index of the edge `from → to`.
    fn edge(e: &Editor, from: &str, to: &str) -> usize {
        e.document().edges.iter().position(|x| x.from.as_str() == from && x.to.as_str() == to).unwrap()
    }

    #[test]
    fn disconnect_removes_one_edge() {
        let mut e = ed();
        e.disconnect(edge(&e, "pricing", "build")).unwrap();
        let (removed, added) = diff(SRC, e.source());
        assert_eq!(removed, vec!["edge pricing build"]);
        assert!(added.is_empty());
        assert_eq!(e.document().edges.len(), 2);

        // First hop of a path: the rest keeps its label.
        e.disconnect(edge(&e, "build", "test")).unwrap();
        assert!(e.source().contains("    edge \"Test\" \"Ship\" label=\"go\"\n}"), "{}", e.source());
        e.disconnect(edge(&e, "test", "ship")).unwrap();
        assert!(!e.source().contains("edge"));
        assert!(e.source().contains("// A comment that must survive every edit."));
        assert!(e.undo() && e.undo() && e.undo());
        assert_eq!(e.source(), SRC);
        assert_eq!(e.disconnect(9), Err(EditError::UnknownEdge(9)));
    }

    #[test]
    fn disconnect_splits_a_path_in_the_middle() {
        let src = "rhizo version=2\nframe \"F\" {\n    edge a b c d class=hot { prov method=inferred; }   // keep me\n    node \"E\"\n}\n";
        let mut e = Editor::open(src);
        assert!(!e.diagnostics().iter().any(Diagnostic::is_error), "{:#?}", e.diagnostics());
        e.disconnect(edge(&e, "b", "c")).unwrap();
        assert_eq!(
            e.source(),
            "rhizo version=2\nframe \"F\" {\n    edge a b class=hot { prov method=inferred; }\n    \
             edge c d class=hot { prov method=inferred; }   // keep me\n    node \"E\"\n}\n"
        );
        let pairs: Vec<_> = e.document().edges.iter().map(|x| (x.from.as_str(), x.to.as_str())).collect();
        assert_eq!(pairs, vec![("a", "b"), ("c", "d")]);
        assert_eq!(e.document().edges[1].class, vec!["hot"]);
        assert!(e.document().edges.iter().all(|x| x.prov.is_some()));
    }

    #[test]
    fn edge_labels_are_set_replaced_and_removed() {
        let mut e = ed();
        let k = edge(&e, "pricing", "build");
        e.set_edge_label(k, Some("feeds")).unwrap();
        let (removed, added) = diff(SRC, e.source());
        assert_eq!(
            (removed, added),
            (vec!["edge pricing build".into()], vec!["edge pricing build label=\"feeds\"".into()])
        );
        e.set_edge_label(k, Some("drives")).unwrap();
        assert!(e.source().contains("edge pricing build label=\"drives\"\n"));
        e.set_edge_label(k, None).unwrap();
        assert_eq!(e.source(), SRC);
        assert_eq!(e.document().edges[k].label, None);
        assert!(e.undo());
        assert!(e.source().contains("label=\"drives\""));
    }

    #[test]
    fn edge_label_on_a_path_hop_splits_it_out() {
        let mut e = ed();
        e.set_edge_label(edge(&e, "test", "ship"), Some("ship it")).unwrap();
        assert!(
            e.source()
                .contains("    edge build \"Test\" label=\"go\"\n    edge \"Test\" \"Ship\" label=\"ship it\"\n}"),
            "{}",
            e.source()
        );
        let mut e = ed();
        e.set_edge_label(edge(&e, "build", "test"), None).unwrap();
        assert!(
            e.source().contains("    edge build \"Test\"\n    edge \"Test\" \"Ship\" label=\"go\"\n}"),
            "{}",
            e.source()
        );
        assert_eq!(e.document().edges[edge(&e, "test", "ship")].label.as_deref(), Some("go"));
        // Unchanged: no edit, nothing to undo.
        let mut e = ed();
        e.set_edge_label(edge(&e, "build", "test"), Some("go")).unwrap();
        assert!(!e.can_undo());
    }

    #[test]
    fn non_ascii_offsets() {
        let src = "- \"Größe\" {\n    - \"Maße\"\n}\n";
        let mut e = Editor::open(src);
        e.rename(&id("mase"), "Gewicht").unwrap();
        assert_eq!(e.source(), "- \"Größe\" {\n    - \"Gewicht\" id=mase\n}\n");
        let n = e.add_child(&id("grose"), "Höhe").unwrap();
        assert_eq!(n.as_str(), "hohe");
    }

    #[test]
    fn insert_reference_adds_the_import_and_raises_the_version() {
        let src = "rhizo version=1\n// keep me\ntitle \"Plan\"\n\n- \"Root\" {\n    - \"A\"\n}\n";
        let mut e = Editor::open(src);
        let r = e.insert_reference("../model.rhizo", "order", "Order", None).unwrap();
        assert_eq!(r, id("model:order"));
        assert_eq!(
            e.source(),
            "rhizo version=2\n// keep me\ntitle \"Plan\"\nimport \"../model.rhizo\" as=model\n\n- \"Root\" {\n    - \"A\"\n}\nnode \"Order\" id=model:order\n"
        );
        assert!(!e.diagnostics().iter().any(Diagnostic::is_error), "{:#?}", e.diagnostics());
        assert_eq!(e.document().nodes[&r].label, "Order");
        // Once more: already there, nothing changes.
        let before = e.source().to_string();
        assert_eq!(e.insert_reference("../model.rhizo", "order", "Order", None).unwrap(), r);
        assert_eq!(e.source(), before);
        // One step back undoes all of it.
        assert!(e.undo());
        assert_eq!(e.source(), src);
    }

    #[test]
    fn insert_reference_reuses_an_import_and_avoids_alias_clashes() {
        let src = "rhizo version=2\nuse data version=\"0.1\"\nimport \"./model.rhizo\" as=m\nimport \"other/api.rhizo\" as=api\nnode \"Pay\" id=pay\n";
        let mut e = Editor::open(src);
        assert_eq!(e.insert_reference("model.rhizo", "order", "The order", None).unwrap(), id("m:order"));
        let (removed, added) = diff(src, e.source());
        assert!(removed.is_empty());
        assert_eq!(added, ["node \"The order\" id=m:order"]);
        let r = e.insert_reference("api.rhizo", "place-order", "Place order", None).unwrap();
        assert_eq!(r, id("api2:place-order"), "`api` is another file already");
        assert!(e.source().contains("import \"other/api.rhizo\" as=api\nimport \"api.rhizo\" as=api2\nnode \"Pay\""));
        assert!(e.connect(&id("pay"), &r).is_ok(), "the reference takes edges");
    }

    #[test]
    fn insert_reference_into_a_frame_and_into_a_bare_file() {
        let src = "rhizo version=2\nframe \"Checkout\" id=checkout layout=layered {\n    node \"Cart\" id=cart\n}\n";
        let mut e = Editor::open(src);
        let r =
            e.insert_reference("../api.rhizo", "place-order", "Place order", Some(&FrameId::from("checkout"))).unwrap();
        assert_eq!(e.document().nodes[&r].frame, FrameId::from("checkout"));
        assert!(e.source().contains("    node \"Cart\" id=cart\n    node \"Place order\" id=api:place-order\n}"));
        assert!(e.source().starts_with("rhizo version=2\nimport \"../api.rhizo\" as=api\nframe"));
        assert_eq!(
            e.insert_reference("a.rhizo", "x", "X", Some(&FrameId::from("nope"))),
            Err(EditError::UnknownFrame(FrameId::from("nope")))
        );

        let mut bare = Editor::open("rhizo version=2");
        bare.insert_reference("2024 plan.rhizo", "x", "X", None).unwrap();
        assert_eq!(
            bare.source(),
            "rhizo version=2\nimport \"2024 plan.rhizo\" as=f2024-plan\nnode \"X\" id=f2024-plan:x\n"
        );
        let mut none = Editor::open("");
        none.insert_reference("m.rhizo", "x", "X", None).unwrap();
        assert!(!none.diagnostics().iter().any(Diagnostic::is_error), "{}", none.source());
    }

    const TYPED: &str = r#"rhizo version=2
use data version="0.1"
// keep me
frame "Shop API" id=shop-api type=data.api layout=layered {
    node "List orders" id=list-orders type=data.operation { props method="get" path="/orders" }   // trailing
    frame "Admin" id=admin {
        node "Ban" id=ban type=data.operation {
            props method="post" path="/ban" {
                tags "admin"
            }
            accept "Banned users cannot log in"
        }
    }
}
"#;

    use serde_json::json;

    #[test]
    fn set_prop_replaces_adds_and_round_trips() {
        let mut e = Editor::open(TYPED);
        assert!(!e.diagnostics().iter().any(Diagnostic::is_error), "{:#?}", e.diagnostics());
        e.set_prop(&id("list-orders"), "method", &json!("post")).unwrap();
        let (removed, added) = diff(TYPED, e.source());
        assert_eq!(removed.len(), 1);
        assert_eq!(
            added,
            [
                "    node \"List orders\" id=list-orders type=data.operation { props method=\"post\" path=\"/orders\" }   // trailing"
            ]
        );
        e.set_prop(&id("list-orders"), "deprecated", &json!(true)).unwrap();
        e.set_prop(&id("list-orders"), "limit", &json!(20)).unwrap();
        e.set_prop(&id("list-orders"), "summary", &json!("All \"open\" orders")).unwrap();
        assert!(
            e.source().contains(
                "{ props method=\"post\" path=\"/orders\" deprecated=#true limit=20 summary=\"All \\\"open\\\" orders\" }   // trailing"
            ),
            "{}",
            e.source()
        );
        let props = &e.document().nodes[&id("list-orders")].props;
        assert_eq!(props["deprecated"], json!(true));
        assert_eq!(props["limit"], json!(20));
        assert_eq!(props["summary"], json!("All \"open\" orders"));
        // Unchanged: no edit.
        let before = e.source().to_string();
        e.set_prop(&id("list-orders"), "limit", &json!(20)).unwrap();
        assert_eq!(e.source(), before);
        // Removing them all brings back the original text.
        for k in ["summary", "limit", "deprecated"] {
            e.remove_prop(&id("list-orders"), k).unwrap();
        }
        e.set_prop(&id("list-orders"), "method", &json!("get")).unwrap();
        assert_eq!(e.source(), TYPED);
    }

    #[test]
    fn lists_and_objects_are_child_blocks() {
        let mut e = Editor::open(TYPED);
        let n = id("list-orders");
        let v = json!({ "address": "module.x", "also": ["a.b"], "keep": { "name": "\"x\"", "tags": "{\n  a = 1\n}" } });
        e.set_prop(&n, "origin", &v).unwrap();
        assert_eq!(e.document().nodes[&n].props["origin"], v);
        assert!(e.source().contains("            (array)also \"a.b\"\n"), "{}", e.source());
        // Replaced in place.
        let w = json!({ "address": "module.y", "also": ["a.b", "c.d"] });
        e.set_prop(&n, "origin", &w).unwrap();
        assert_eq!(e.document().nodes[&n].props["origin"], w);
        e.remove_prop(&n, "origin").unwrap();
        assert_eq!(e.source(), TYPED);
        // A list can replace a scalar, and a scalar the list (as a child, in place).
        e.set_prop(&n, "method", &json!(["get", "head"])).unwrap();
        assert_eq!(e.document().nodes[&n].props["method"], json!(["get", "head"]));
        e.set_prop(&n, "method", &json!("get")).unwrap();
        assert_eq!(e.document().nodes[&n].props["method"], json!("get"));
        // Into a nested props block that has children already.
        let mut e = Editor::open(TYPED);
        e.set_prop(&id("ban"), "limits", &json!({ "rate": 5 })).unwrap();
        assert!(
            e.source().contains("                tags \"admin\"\n                limits rate=5\n"),
            "{}",
            e.source()
        );
        // A node without props.
        let mut e = Editor::open("rhizo version=2\nnode \"A\" id=a\n");
        e.set_prop(&id("a"), "list", &json!([1, 2])).unwrap();
        assert_eq!(e.source(), "rhizo version=2\nnode \"A\" id=a {\n    props {\n        list 1 2\n    }\n}\n");
    }

    #[test]
    fn frame_and_edge_props() {
        let src = "rhizo version=2\nframe \"F\" id=f {\n    props {\n        dir \"x\"\n    }\n    node \"A\" id=a\n    node \"B\" id=b\n    edge a b\n}\n";
        let mut e = Editor::open(src);
        let blocks = json!([{ "type": "output", "body": { "value": "module.x.y" } }]);
        e.set_frame_prop(&FrameId::from("f"), "blocks", &blocks).unwrap();
        e.set_frame_prop(&FrameId::from("f"), "cloud", &json!("aws")).unwrap();
        let f = &e.document().frames.iter().find(|f| f.id.as_str() == "f").unwrap().props;
        assert_eq!((&f["blocks"], &f["cloud"], &f["dir"]), (&blocks, &json!("aws"), &json!("x")));
        e.set_edge_prop(0, "route", &json!(["GET /a", "POST /a"])).unwrap();
        assert_eq!(e.document().edges[0].props["route"], json!(["GET /a", "POST /a"]));
        e.set_edge_prop(0, "route", &json!("GET /a")).unwrap();
        assert_eq!(e.document().edges[0].props["route"], json!("GET /a"));
        assert!(
            e.source().contains("    edge a b {\n        props {\n            route \"GET /a\"\n"),
            "{}",
            e.source()
        );
        assert!(e.set_frame_prop(&FrameId::from("nope"), "x", &json!(1)).is_err());
        assert!(e.set_edge_prop(5, "x", &json!(1)).is_err());
    }

    #[test]
    fn props_in_a_nested_frame_and_child_props() {
        let mut e = Editor::open(TYPED);
        e.set_prop(&id("ban"), "path", &json!("/bans")).unwrap();
        assert!(e.source().contains("            props method=\"post\" path=\"/bans\" {\n"), "{}", e.source());
        // A prop written as a child becomes `key=value` in place.
        e.set_prop(&id("ban"), "tags", &json!("staff")).unwrap();
        assert!(e.source().contains("                tags \"staff\"\n"), "{}", e.source());
        assert_eq!(e.document().nodes[&id("ban")].props["tags"], json!("staff"));
        e.remove_prop(&id("ban"), "tags").unwrap();
        assert!(
            e.source().contains("            props method=\"post\" path=\"/bans\"\n            accept"),
            "{}",
            e.source()
        );
        assert_eq!(e.document().nodes[&id("ban")].frame.as_str(), "admin");
        assert!(e.source().contains("// keep me"));
    }

    #[test]
    fn the_last_prop_takes_its_block_along() {
        let src = "rhizo version=2\nnode \"A\" { props x=1 }\nnode \"B\" {\n    props y=2\n    note \"keep\"\n}\n";
        let mut e = Editor::open(src);
        e.remove_prop(&id("a"), "x").unwrap();
        e.remove_prop(&id("b"), "y").unwrap();
        assert_eq!(e.source(), "rhizo version=2\nnode \"A\"\nnode \"B\" {\n    note \"keep\"\n}\n");
        assert!(e.document().nodes[&id("a")].props.is_empty());
        // Unknown props are no edit at all.
        e.remove_prop(&id("a"), "nope").unwrap();
        assert!(e.undo() && e.undo());
        assert_eq!(e.source(), src);
    }

    #[test]
    fn typing_a_v1_mindmap_node_raises_the_version_and_adds_the_kit() {
        let mut e = ed();
        e.set_type(&id("pricing"), Some("rhizo.decision")).unwrap();
        assert!(e.source().starts_with("rhizo version=2\n"));
        assert!(e.source().contains("    - \"Pricing\" class=risk type=rhizo.decision   // trailing comment\n"));
        assert_eq!(e.document().nodes[&id("pricing")].ty.as_ref().map(|t| t.as_str()), Some("rhizo.decision"));
        e.set_prop(&id("pricing"), "status", &json!("proposed")).unwrap();
        assert!(
            e.source().contains("    - \"Pricing\" class=risk type=rhizo.decision {\n        props status=\"proposed\"\n    }   // trailing comment\n"),
            "{}",
            e.source()
        );
        e.use_kit("data", "0.1").unwrap();
        assert!(e.source().contains("title \"Plan\"\nuse data version=\"0.1\"\n\n"), "{}", e.source());
        e.use_kit("data", "0.1").unwrap();
        assert_eq!(e.source().matches("use data").count(), 1, "once");
        e.set_type(&id("pricing"), Some("rhizo.goal")).unwrap();
        assert!(e.source().contains("class=risk type=rhizo.goal {"));
        e.remove_prop(&id("pricing"), "status").unwrap();
        e.set_type(&id("pricing"), None).unwrap();
        assert!(e.source().contains("    - \"Pricing\" class=risk   // trailing comment\n"), "{}", e.source());
        // Undo walks back one edit at a time, to the very start.
        for _ in 0..6 {
            assert!(e.undo());
        }
        assert_eq!(e.source(), SRC);
        assert!(e.redo());
        assert!(e.source().contains("type=rhizo.decision"));
    }

    #[test]
    fn typing_a_headerless_file_and_an_edge_created_node() {
        let mut e = Editor::open("- \"Idea\"\nedge idea \"Later\"\n");
        e.set_type(&id("idea"), Some("rhizo.goal")).unwrap();
        assert_eq!(e.source(), "rhizo version=2\n- \"Idea\" type=rhizo.goal\nedge idea \"Later\"\n");
        e.set_prop(&id("later"), "due", &json!("2027-01")).unwrap();
        assert!(e.source().ends_with("node \"Later\" id=later { props due=\"2027-01\" }\n"), "{}", e.source());
        e.set_type(&id("later"), Some("rhizo.goal")).unwrap();
        assert!(e.source().contains("node \"Later\" id=later type=rhizo.goal { props"), "{}", e.source());
        assert!(!e.diagnostics().iter().any(Diagnostic::is_error), "{:#?}", e.diagnostics());
    }

    #[test]
    fn invalid_keys_values_and_types_are_refused() {
        let mut e = Editor::open(TYPED);
        let n = id("list-orders");
        assert_eq!(e.set_prop(&n, "", &json!(1)), Err(EditError::InvalidKey(String::new())));
        assert_eq!(e.set_prop(&n, " padded", &json!(1)), Err(EditError::InvalidKey(" padded".into())));
        assert_eq!(e.set_prop(&n, "a\nb", &json!(1)), Err(EditError::InvalidKey("a\nb".into())));
        assert!(e.set_prop(&id("nope"), "x", &json!(1)).is_err());
        assert!(matches!(e.set_type(&n, Some("")), Err(EditError::Invalid(_))));
        assert_eq!(e.source(), TYPED);
        assert!(!e.can_undo());
        // Odd but valid keys are quoted.
        e.set_prop(&n, "x-rate limit", &json!(1.5)).unwrap();
        assert!(e.source().contains("path=\"/orders\" \"x-rate limit\"=1.5 }"), "{}", e.source());
        assert_eq!(e.document().nodes[&n].props["x-rate limit"], json!(1.5));
    }

    fn clean(e: &Editor) {
        assert!(!e.diagnostics().iter().any(Diagnostic::is_error), "{:#?}\n{}", e.diagnostics(), e.source());
    }

    fn fid(s: &str) -> FrameId {
        FrameId::from(s)
    }

    /// Undoes every step, back to `src`.
    fn undo_all(e: &mut Editor, src: &str) {
        while e.undo() {}
        assert_eq!(e.source(), src);
    }

    #[test]
    fn owner_scenario_builds_a_production_environment() {
        let mut e = Editor::open("");
        let prod = e.add_frame(None, "Production", None).unwrap();
        assert_eq!(e.source(), "frame \"Production\"\n");
        e.use_kit("infra", "0.1").unwrap();
        e.set_frame_type(&prod, Some("infra.environment")).unwrap();
        e.set_frame_prop(&prod, "cloud", &json!("aws")).unwrap();
        e.set_frame_prop(&prod, "region", &json!("eu-central-1")).unwrap();
        let typed = |e: &mut Editor, ty: &str, label: &str| {
            let n = e.add_node(&prod, "New idea").unwrap();
            e.set_type(&n, Some(ty)).unwrap();
            e.rename_new(&n, label).unwrap()
        };
        let bucket = typed(&mut e, "infra.bucket", "Bucket");
        let function = typed(&mut e, "infra.function", "Function");
        e.connect(&function, &bucket).unwrap();
        let k = edge(&e, "function", "bucket");
        e.set_edge_prop(k, "access", &json!("write")).unwrap();
        assert_eq!(
            e.source(),
            r#"rhizo version=2
use infra version="0.1"
frame "Production" type=infra.environment {
    props cloud="aws" region="eu-central-1"
    node "Bucket" id=bucket type=infra.bucket
    node "Function" id=function type=infra.function
    edge function bucket {
        props access="write"
    }
}
"#
        );
        assert!(e.diagnostics().is_empty(), "{:#?}", e.diagnostics());
        undo_all(&mut e, "");
    }

    const FRAMES: &str = r#"rhizo version=2
// keep me
frame "Outer" {
    node "A"
    frame "Inner" type=infra.environment {
        props cloud="aws"
        node "B"   // b
        edge b c
        node "C"
    }
}
edge a b
"#;

    #[test]
    fn frames_are_added_renamed_typed_and_laid_out() {
        let mut e = Editor::open(FRAMES);
        clean(&e);
        let n = e.add_frame(Some(&fid("inner")), "Deep", Some((10.0, 20.5))).unwrap();
        assert!(e.source().contains("        node \"C\"\n        frame \"Deep\" x=10 y=20.5\n    }"), "{}", e.source());
        assert_eq!(e.document().frame(&n).unwrap().parent, Some(fid("inner")));
        // A label another frame's id has gets an id of its own, so no id moves.
        let twin = e.add_frame(None, "Inner", None).unwrap();
        assert_eq!(twin, fid("inner-2"));
        assert!(e.source().ends_with("edge a b\nframe \"Inner\" id=inner-2\n"), "{}", e.source());
        e.rename_frame(&fid("inner"), "Production EU").unwrap();
        assert!(e.source().contains("    frame \"Production EU\" id=inner type=infra.environment {\n"));
        e.set_frame_layout(&fid("inner"), Some(LayoutKind::Layered)).unwrap();
        e.set_frame_direction(&fid("inner"), Some(Direction::Down)).unwrap();
        e.set_frame_look(&fid("inner"), "fill", Some(&json!("#fef3c7"))).unwrap();
        assert!(
            e.source().contains(
                "frame \"Production EU\" id=inner type=infra.environment layout=layered direction=down fill=\"#fef3c7\" {"
            ),
            "{}",
            e.source()
        );
        let f = e.document().frame(&fid("inner")).unwrap();
        assert_eq!(
            (f.layout, f.direction, f.label.as_deref()),
            (LayoutKind::Layered, Some(Direction::Down), Some("Production EU"))
        );
        e.set_frame_type(&fid("inner"), None).unwrap();
        e.set_frame_layout(&fid("inner"), None).unwrap();
        e.set_frame_look(&fid("inner"), "fill", None).unwrap();
        assert!(e.source().contains("    frame \"Production EU\" id=inner direction=down {\n"), "{}", e.source());
        assert!(e.source().contains("// keep me"));
        undo_all(&mut e, FRAMES);
        // A typed frame takes the version 2 header; a sequence takes no frames.
        let mut v1 = Editor::open("frame \"F\"\n");
        v1.set_frame_type(&fid("f"), Some("flow.workflow")).unwrap();
        assert_eq!(v1.source(), "rhizo version=2\nframe \"F\" type=flow.workflow\n");
        let mut seq = Editor::open("frame \"S\" layout=sequence {\n    node \"A\"\n}\n");
        assert!(seq.add_frame(Some(&fid("s")), "X", None).is_err());
        assert!(e.rename_frame(&fid("main"), "X").is_err(), "the implicit frame has no statement");
    }

    #[test]
    fn frame_props_come_and_go() {
        let mut e = Editor::open(FRAMES);
        e.set_frame_prop(&fid("inner"), "region", &json!("eu-central-1")).unwrap();
        assert!(e.source().contains("        props cloud=\"aws\" region=\"eu-central-1\"\n"), "{}", e.source());
        e.remove_frame_prop(&fid("inner"), "region").unwrap();
        assert_eq!(e.source(), FRAMES);
        e.remove_frame_prop(&fid("inner"), "cloud").unwrap();
        assert!(e.source().contains("type=infra.environment {\n        node \"B\""), "{}", e.source());
        assert!(e.document().frame(&fid("inner")).unwrap().props.is_empty());
        undo_all(&mut e, FRAMES);
    }

    #[test]
    fn deleting_a_frame_takes_its_edges_or_keeps_its_contents() {
        let mut e = Editor::open(FRAMES);
        e.delete_frame(&fid("inner"), false).unwrap();
        assert_eq!(e.source(), "rhizo version=2\n// keep me\nframe \"Outer\" {\n    node \"A\"\n}\n");
        clean(&e);
        undo_all(&mut e, FRAMES);

        e.delete_frame(&fid("inner"), true).unwrap();
        assert_eq!(
            e.source(),
            "rhizo version=2\n// keep me\nframe \"Outer\" {\n    node \"A\"\n    node \"B\"   // b\n    edge b c\n    node \"C\"\n}\nedge a b\n"
        );
        assert_eq!(e.document().nodes[&id("b")].frame, fid("outer"));
        assert_eq!(e.document().edges.len(), 2, "its edges still resolve");
        // Ungrouping the outer frame puts it all at the top level.
        e.delete_frame(&fid("outer"), true).unwrap();
        assert!(e.source().starts_with("rhizo version=2\n// keep me\nnode \"A\"\nnode \"B\"   // b\nedge b c\n"));
        clean(&e);
        undo_all(&mut e, FRAMES);

        // The only frame in its parent: the parent's block goes too.
        let src = "frame \"P\" {\n    frame \"Q\" { node \"X\" }\n}\nedge x y\n";
        let mut e = Editor::open(src);
        e.delete_frame(&fid("q"), false).unwrap();
        assert_eq!(e.source(), "frame \"P\"\n");
        let mut e = Editor::open(src);
        e.delete_frame(&fid("q"), true).unwrap();
        assert_eq!(e.source(), "frame \"P\" {\n    node \"X\"\n}\nedge x y\n");
        assert!(e.delete_frame(&fid("nope"), true).is_err());
    }

    const FLOW: &str = r#"rhizo version=2
frame "Flow" layout=layered {
    node "Build" shape=pill   // first
    node "Test"
    // between
    node "Ship"
    edge build test
    edge test ship
}
edge ship build
"#;

    #[test]
    fn group_moves_nodes_and_the_edges_among_them_into_a_new_frame() {
        let mut e = Editor::open(FLOW);
        let ci = e.group(&[id("test"), id("build")], "CI").unwrap();
        assert_eq!(ci, fid("ci"));
        assert_eq!(
            e.source(),
            r#"rhizo version=2
frame "Flow" layout=layered {
    frame "CI" {
        node "Build" shape=pill   // first
        node "Test"
        edge build test
    }
    // between
    node "Ship"
    edge test ship
}
edge ship build
"#
        );
        clean(&e);
        let doc = e.document();
        assert_eq!(
            (doc.nodes[&id("build")].frame.as_str(), doc.frame(&ci).unwrap().parent.clone()),
            ("ci", Some(fid("flow")))
        );
        assert_eq!(doc.edges.len(), 3, "every edge kept");
        assert!(e.undo());
        assert_eq!(e.source(), FLOW, "one step");
        // Nodes of two frames, or a child without its parent, do not group.
        let mut e = Editor::open(FRAMES);
        assert!(matches!(e.group(&[id("a"), id("b")], "X"), Err(EditError::Invalid(_))));
        let mut e = ed();
        assert!(matches!(e.group(&[id("users")], "X"), Err(EditError::Invalid(_))));
        // The implicit frame: a new top-level frame, the outline reindented.
        let src = "- \"Root\" {\n    - \"A\"\n}\n- \"Other\"\n";
        let mut e = Editor::open(src);
        e.group(&[id("root"), id("a")], "Map").unwrap();
        assert_eq!(e.source(), "frame \"Map\" {\n    - \"Root\" {\n        - \"A\"\n    }\n}\n- \"Other\"\n");
        assert_eq!(e.document().nodes[&id("a")].parent, Some(id("root")));
        // An edge-created node is declared in the frame.
        let mut e = ed();
        e.group(&[id("ship"), id("build")], "Release").unwrap();
        assert!(
            e.source().contains(
                "    frame \"Release\" {\n        node \"Build\" shape=pill\n        node \"Ship\" id=ship\n    }\n"
            ),
            "{}",
            e.source()
        );
        assert_eq!(e.document().nodes[&id("ship")].frame.as_str(), "release");
        clean(&e);
    }

    #[test]
    fn nodes_move_into_and_out_of_frames() {
        let mut e = Editor::open(FLOW);
        let ci = e.group(&[id("build"), id("test")], "CI").unwrap();
        let grouped = e.source().to_string();
        e.move_to_frame(&id("ship"), Some(&ci)).unwrap();
        assert!(
            e.source().contains(
                "        edge build test\n        node \"Ship\"\n    }\n    // between\n    edge test ship\n"
            ),
            "{}",
            e.source()
        );
        assert_eq!(e.document().nodes[&id("ship")].frame, ci);
        e.move_to_frame(&id("ship"), Some(&ci)).unwrap();
        e.move_to_frame(&id("build"), None).unwrap();
        assert!(e.source().ends_with("edge ship build\nnode \"Build\" shape=pill   // first\n"), "{}", e.source());
        assert!(e.document().frames.iter().any(|f| f.implicit), "it is at the top level");
        clean(&e);
        assert!(e.undo() && e.undo());
        assert_eq!(e.source(), grouped);
        // A child becomes a root of the frame; an edge-created node is declared there.
        let mut e = ed();
        e.move_to_frame(&id("personas"), Some(&fid("flow"))).unwrap();
        assert!(e.source().contains("- \"Users\" { - \"Jobs\" }"), "{}", e.source());
        assert!(e.source().contains("    - \"Personas\"\n}"), "{}", e.source());
        e.move_to_frame(&id("ship"), None).unwrap();
        assert!(e.source().ends_with("node \"Ship\" id=ship\n"), "{}", e.source());
        assert_eq!(e.document().nodes[&id("ship")].frame.as_str(), "main");
        clean(&e);
    }

    #[test]
    fn a_new_nodes_id_follows_its_label() {
        let src = "rhizo version=2\nframe \"F\" type=infra.environment {\n    node \"New idea\" type=infra.bucket   // b\n    node \"Files\" id=files\n    - \"New idea 2\" id=new-idea-2\n}\n";
        let mut e = Editor::open(src);
        assert_eq!(e.rename_new(&id("new-idea"), "Bucket").unwrap(), id("bucket"));
        assert!(e.source().contains("    node \"Bucket\" id=bucket type=infra.bucket   // b\n"), "{}", e.source());
        // Taken: numbered. An `id=` written already is replaced in place.
        assert_eq!(e.rename_new(&id("new-idea-2"), "Files").unwrap(), id("files-2"));
        assert!(e.source().contains("    - \"Files\" id=files-2\n"), "{}", e.source());
        // Untyped and free: no id at all.
        assert_eq!(e.rename_new(&id("files-2"), "Logs").unwrap(), id("logs"));
        assert!(e.source().contains("    - \"Logs\"\n"), "{}", e.source());
        clean(&e);
        undo_all(&mut e, src);
    }

    #[test]
    fn acceptance_criteria_are_added_edited_and_removed() {
        let src = "rhizo version=1\n- \"Pay\"   // keep\n";
        let mut e = Editor::open(src);
        e.add_accept(&id("pay"), "Refunds within a day").unwrap();
        e.add_accept(&id("pay"), "Receipts by mail").unwrap();
        assert_eq!(
            e.source(),
            "rhizo version=2\n- \"Pay\" {\n    accept \"Refunds within a day\"\n    accept \"Receipts by mail\"\n}   // keep\n"
        );
        e.set_accept(&id("pay"), 0, "Refunds within an hour").unwrap();
        assert_eq!(e.document().nodes[&id("pay")].accept, ["Refunds within an hour", "Receipts by mail"]);
        e.remove_accept(&id("pay"), 0).unwrap();
        e.remove_accept(&id("pay"), 0).unwrap();
        assert_eq!(e.source(), "rhizo version=2\n- \"Pay\"   // keep\n");
        assert!(e.remove_accept(&id("pay"), 0).is_err());
        undo_all(&mut e, src);
    }

    #[test]
    fn looks_are_removed() {
        let mut e = ed();
        e.remove_look(&id("build"), "shape").unwrap();
        assert!(e.source().contains("    node \"Build\"\n"));
        e.remove_look(&id("build"), "shape").unwrap();
        assert_eq!(e.document().nodes[&id("build")].style.shape, None);
        assert!(e.undo() && !e.undo());
        assert_eq!(e.source(), SRC);
    }

    #[test]
    fn edge_attrs_and_props_on_single_edges_and_path_hops() {
        let mut e = ed();
        let k = edge(&e, "pricing", "build");
        e.set_edge_attr(k, "kind", Some("data")).unwrap();
        e.set_edge_attr(k, "line", Some("dashed")).unwrap();
        e.set_edge_attr(k, "arrow", Some("both")).unwrap();
        assert!(e.source().starts_with("rhizo version=2\n"));
        assert!(e.source().ends_with("edge pricing build kind=data line=dashed arrow=both\n"), "{}", e.source());
        e.set_edge_attr(k, "line", None).unwrap();
        assert!(e.source().ends_with("edge pricing build kind=data arrow=both\n"));
        // A hop of a path is split out; the edge keeps its index.
        let k = edge(&e, "test", "ship");
        e.set_edge_attr(k, "line", Some("dotted")).unwrap();
        assert!(
            e.source().contains(
                "    edge build \"Test\" label=\"go\"\n    edge \"Test\" \"Ship\" label=\"go\" line=dotted\n}"
            ),
            "{}",
            e.source()
        );
        assert_eq!(edge(&e, "test", "ship"), k);
        let k = edge(&e, "build", "test");
        e.set_edge_prop(k, "route", &json!("GET /a")).unwrap();
        assert_eq!(edge(&e, "build", "test"), k);
        assert_eq!(e.document().edges[k].props["route"], json!("GET /a"));
        assert!(e.document().edges[edge(&e, "test", "ship")].props.is_empty(), "only this hop");
        e.remove_edge_prop(k, "route").unwrap();
        assert!(e.document().edges[k].props.is_empty());
        clean(&e);
        assert!(e.set_edge_attr(k, "arrow", Some("sideways")).is_err());
        assert!(e.set_edge_attr(99, "arrow", None).is_err());
        undo_all(&mut e, SRC);
        // A failing prop on a hop leaves the path whole.
        let mut e = ed();
        assert!(e.set_edge_prop(edge(&e, "test", "ship"), "", &json!(1)).is_err());
        assert_eq!(e.source(), SRC);
    }

    #[test]
    fn header_statements_and_kits_and_imports() {
        let src = "rhizo version=2\n// keep me\nuse data version=\"0.1\"\nuse infra version=\"0.1\"\nimport \"model.rhizo\" as=m\nimport \"lib/api.rhizo\"\nnode \"Order\" id=m:order\n";
        let mut e = Editor::open(src);
        clean(&e);
        e.set_title(Some("Shop")).unwrap();
        assert!(e.source().starts_with("rhizo version=2\ntitle \"Shop\"\n// keep me\n"), "{}", e.source());
        e.set_title(Some("Shop \"v2\"")).unwrap();
        assert_eq!(e.document().title.as_deref(), Some("Shop \"v2\""));
        e.set_theme(Some("dark")).unwrap();
        e.set_top_layout(Some(LayoutKind::Grid)).unwrap();
        e.set_top_direction(Some(Direction::Down)).unwrap();
        assert!(
            e.source().contains("import \"lib/api.rhizo\"\ntheme \"dark\"\nlayout grid\ndirection down\nnode"),
            "{}",
            e.source()
        );
        e.set_top_layout(Some(LayoutKind::Tree)).unwrap();
        assert!(e.source().contains("\nlayout tree\n"));
        assert_eq!(e.unused_imports(), ["api"]);
        e.remove_import("api").unwrap();
        e.unuse_kit("data").unwrap();
        assert!(!e.source().contains("api.rhizo") && !e.source().contains("use data"));
        e.set_title(None).unwrap();
        e.set_top_layout(None).unwrap();
        assert!(!e.source().contains("title") && !e.source().contains("layout"));
        clean(&e);
        undo_all(&mut e, src);
        let mut bare = Editor::open("- \"A\"\n");
        bare.set_title(Some("Plan")).unwrap();
        assert_eq!(bare.source(), "title \"Plan\"\n- \"A\"\n");
    }
}
