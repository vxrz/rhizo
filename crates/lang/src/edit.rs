//! Editing a KDL document in place.
//!
//! Every operation becomes a small text splice at the spans kdl-rs reports,
//! so comments, blank lines and the author's formatting survive and the diff
//! stays minimal. After each splice the source is parsed and lowered again;
//! an edit that would add errors is refused. Undo and redo keep whole source
//! snapshots, which is cheap at diagram sizes and can never drift.

use std::collections::HashMap;

use kdl::{KdlDocument, KdlNode};
use rhizo_core::{Document, FrameId, LayoutKind, NodeId, slugify};

use crate::diag::Diagnostic;
use crate::lower::{Lowered, lower_with_sources};
use crate::write::{kdl_string, num, quoted};

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
        let label_prop = kn.entries().iter().find(|e| e.name().is_some_and(|n| n.value() == "label"));
        let first_arg = kn.entries().iter().find(|e| e.name().is_none());
        let (start, end, text) = if string_named(kn) {
            let s = kn.name().span();
            (s.offset(), s.offset() + s.len(), quoted(label))
        } else if let Some(e) = label_prop {
            (e.span().offset(), e.span().offset() + e.span().len(), format!("label={}", quoted(label)))
        } else if let Some(e) = first_arg {
            (e.span().offset(), e.span().offset() + e.span().len(), quoted(label))
        } else {
            return Err(EditError::NoStatement(id.clone()));
        };
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
            LayoutKind::Layered | LayoutKind::Grid | LayoutKind::Free => "node",
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
            let (start, end) = whole_lines(&self.src, kn.span().offset(), kn.span().offset() + kn.span().len());
            splices.push(Splice { start, end, text: String::new() });
        }
        // Edge statements, grouped by their source span.
        let mut statements: Vec<(usize, Vec<usize>)> = Vec::new();
        for (k, span) in self.state.lowered.edge_spans.iter().enumerate() {
            let Some((off, _)) = span else { continue };
            match statements.iter_mut().find(|(o, _)| o == off) {
                Some((_, ks)) => ks.push(k),
                None => statements.push((*off, vec![k])),
            }
        }
        for (off, ks) in statements {
            let edges: Vec<_> = ks.iter().map(|&k| &doc.edges[k]).collect();
            if !edges.iter().any(|e| doomed.contains(&e.from) || doomed.contains(&e.to)) {
                continue;
            }
            let Some(kn) = self.kdl_node_at(off) else { continue };
            let args: Vec<_> = kn.entries().iter().filter(|e| e.name().is_none()).collect();
            let mut ends: Vec<&NodeId> = edges.iter().map(|e| &e.from).collect();
            ends.extend(edges.last().map(|e| &e.to));
            let (start, end) = whole_lines(&self.src, kn.span().offset(), kn.span().offset() + kn.span().len());
            // If lowering dropped part of the path, keeping pieces could be wrong: drop it all.
            let keep: Vec<Vec<usize>> = if ends.len() == args.len() {
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
            let text = if keep.is_empty() {
                String::new()
            } else {
                let indent = line_indent(&self.src, kn.span().offset());
                let piece = |e: &kdl::KdlEntry| {
                    let s = e.span();
                    self.src[s.offset()..s.offset() + s.len()].to_string()
                };
                let props: String =
                    kn.entries().iter().filter(|e| e.name().is_some()).map(|e| format!(" {}", piece(e))).collect();
                let block = kn.children().map(|c| {
                    let s = c.span();
                    format!(" {{ {} }}", self.src[s.offset()..s.offset() + s.len()].trim())
                });
                keep.iter()
                    .map(|seg| {
                        let a: Vec<String> = seg.iter().map(|&i| piece(args[i])).collect();
                        format!("{indent}edge {}{props}{}\n", a.join(" "), block.clone().unwrap_or_default())
                    })
                    .collect()
            };
            splices.push(Splice { start, end, text });
        }
        self.apply(splices)
    }

    // ------------------------------------------------------------------

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
    /// source order, so the n-th explicit frame is the n-th `frame` node.
    fn frame_statement(&self, frame: &FrameId) -> Option<&KdlNode> {
        let n = self.document().frames.iter().filter(|f| !f.implicit).position(|f| &f.id == frame)?;
        self.state.kdl.as_ref()?.nodes().iter().filter(|k| k.name().value() == "frame").nth(n)
    }

    fn append_to_frame(&self, frame: &FrameId, line: &str) -> Result<Splice, EditError> {
        let f = self.document().frame(frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))?;
        if f.implicit || self.document().frames.is_empty() {
            let at = self.src.len();
            let lead = if self.src.is_empty() || self.src.ends_with('\n') { "" } else { "\n" };
            return Ok(Splice { start: at, end: at, text: format!("{lead}{line}\n") });
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
        splices.sort_by_key(|s| std::cmp::Reverse(s.start));
        let mut src = self.src.clone();
        let mut limit = usize::MAX;
        for s in &splices {
            assert!(s.end <= limit, "overlapping edits");
            src.replace_range(s.start..s.end, &s.text);
            limit = s.start;
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
    if before_blank && (after.is_empty() || after == ";") {
        (line_start, line_end)
    } else {
        // Shares the line (`- "A"; - "B"`): take the node and its `;`.
        let semi = rest
            .find(|c: char| !c.is_whitespace())
            .filter(|&i| rest[i..].starts_with(';'))
            .map_or(end, |i| end + i + 1);
        (start, semi)
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
    fn non_ascii_offsets() {
        let src = "- \"Größe\" {\n    - \"Maße\"\n}\n";
        let mut e = Editor::open(src);
        e.rename(&id("mase"), "Gewicht").unwrap();
        assert_eq!(e.source(), "- \"Größe\" {\n    - \"Gewicht\" id=mase\n}\n");
        let n = e.add_child(&id("grose"), "Höhe").unwrap();
        assert_eq!(n.as_str(), "hohe");
    }
}
