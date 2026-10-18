use std::fmt;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Arrow, LineStyle, Point, Style};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_string())
            }
        }
    };
}

id_type!(NodeId);
id_type!(FrameId);

/// How the nodes of a frame are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutKind {
    /// Pick from the content: `layered` when the frame has explicit edges,
    /// `mindmap` when it has nesting, `grid` otherwise.
    #[default]
    Auto,
    /// A radial-ish map that grows to both sides of the root.
    Mindmap,
    /// A tidy tree growing in one direction (org charts, outlines).
    Tree,
    /// Layered graph drawing (flowcharts, DAGs, pipelines).
    Layered,
    /// Uniform grid.
    Grid,
    /// Nodes keep their `x`/`y`; nothing is moved.
    Free,
}

impl LayoutKind {
    pub fn name(self) -> &'static str {
        match self {
            LayoutKind::Auto => "auto",
            LayoutKind::Mindmap => "mindmap",
            LayoutKind::Tree => "tree",
            LayoutKind::Layered => "layered",
            LayoutKind::Grid => "grid",
            LayoutKind::Free => "free",
        }
    }

    pub fn parse(s: &str) -> Option<LayoutKind> {
        Some(match s {
            "auto" => LayoutKind::Auto,
            "mindmap" | "map" => LayoutKind::Mindmap,
            "tree" | "org" | "outline" => LayoutKind::Tree,
            "layered" | "flow" | "flowchart" | "dag" | "graph" => LayoutKind::Layered,
            "grid" => LayoutKind::Grid,
            "free" | "freeform" | "canvas" => LayoutKind::Free,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    #[default]
    Right,
    Left,
    Down,
    Up,
}

impl Direction {
    pub fn name(self) -> &'static str {
        match self {
            Direction::Right => "right",
            Direction::Left => "left",
            Direction::Down => "down",
            Direction::Up => "up",
        }
    }

    pub fn parse(s: &str) -> Option<Direction> {
        Some(match s {
            "right" | "lr" | "LR" | "east" => Direction::Right,
            "left" | "rl" | "RL" | "west" => Direction::Left,
            "down" | "td" | "TD" | "tb" | "TB" | "south" => Direction::Down,
            "up" | "bt" | "BT" | "north" => Direction::Up,
            _ => return None,
        })
    }

    pub fn is_horizontal(self) -> bool {
        matches!(self, Direction::Right | Direction::Left)
    }
}

/// A region of the canvas with its own layout engine. A document without
/// explicit frames has one implicit frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct Frame {
    pub id: FrameId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub layout: LayoutKind,
    /// `None` means the layout's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// Pinned top-left position on the canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
    /// Column count for grid layouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    #[serde(default, skip_serializing_if = "Style::is_empty")]
    pub style: Style,
    /// True for the frame created for nodes written outside any `frame`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub implicit: bool,
}

impl Frame {
    pub fn new(id: impl Into<String>) -> Self {
        Frame {
            id: FrameId(id.into()),
            label: None,
            layout: LayoutKind::Auto,
            direction: None,
            at: None,
            columns: None,
            style: Style::default(),
            implicit: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct Node {
    pub id: NodeId,
    pub label: String,
    pub frame: FrameId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeId>,
    /// Derived from `parent`; kept in sync by [`Document`].
    #[serde(skip)]
    pub children: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub class: Vec<String>,
    #[serde(default, skip_serializing_if = "Style::is_empty")]
    pub style: Style,
    /// Top-left position. Used by free layouts, and as a pin elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Node {
    pub fn new(id: impl Into<String>, label: impl Into<String>, frame: FrameId) -> Self {
        Node {
            id: NodeId(id.into()),
            label: label.into(),
            frame,
            parent: None,
            children: Vec::new(),
            class: Vec::new(),
            style: Style::default(),
            at: None,
            note: None,
            link: None,
            tags: Vec::new(),
        }
    }
}

/// An explicit connection. Parent/child nesting is a connection too, but it
/// is implied by [`Node::parent`] and not stored here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub arrow: Arrow,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub class: Vec<String>,
    #[serde(default, skip_serializing_if = "Style::is_empty")]
    pub style: Style,
}

impl Edge {
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Edge {
            from: NodeId(from.into()),
            to: NodeId(to.into()),
            label: None,
            arrow: Arrow::End,
            class: Vec::new(),
            style: Style::default(),
        }
    }

    pub fn line(&self) -> LineStyle {
        self.style.line.unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ModelError {
    #[error("duplicate node id `{0}`")]
    DuplicateNode(NodeId),
    #[error("duplicate frame id `{0}`")]
    DuplicateFrame(FrameId),
    #[error("unknown node `{0}`")]
    UnknownNode(NodeId),
    #[error("unknown frame `{0}`")]
    UnknownFrame(FrameId),
    #[error("node `{child}` cannot be a child of `{parent}`: they are in different frames")]
    CrossFrameParent { child: NodeId, parent: NodeId },
    #[error("making `{parent}` the parent of `{child}` would create a cycle")]
    ParentCycle { child: NodeId, parent: NodeId },
}

/// A diagram: frames, nodes, edges and named styles.
///
/// Nodes are kept in document order, which is also the order children are laid
/// out in. Mutate through the methods here so `children` stays consistent with
/// `parent`; after deserializing, call [`Document::relink`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct Document {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub styles: IndexMap<String, Style>,
    pub frames: Vec<Frame>,
    pub nodes: IndexMap<NodeId, Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn frame(&self, id: &FrameId) -> Option<&Frame> {
        self.frames.iter().find(|f| &f.id == id)
    }

    pub fn frame_mut(&mut self, id: &FrameId) -> Option<&mut Frame> {
        self.frames.iter_mut().find(|f| &f.id == id)
    }

    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    pub fn add_frame(&mut self, frame: Frame) -> Result<(), ModelError> {
        if self.frame(&frame.id).is_some() {
            return Err(ModelError::DuplicateFrame(frame.id));
        }
        self.frames.push(frame);
        Ok(())
    }

    /// Adds a node. If it names a parent, it is appended to that parent's children.
    pub fn add_node(&mut self, node: Node) -> Result<(), ModelError> {
        if self.nodes.contains_key(&node.id) {
            return Err(ModelError::DuplicateNode(node.id));
        }
        if self.frame(&node.frame).is_none() {
            return Err(ModelError::UnknownFrame(node.frame));
        }
        if let Some(parent) = &node.parent {
            let p = self.nodes.get_mut(parent).ok_or_else(|| ModelError::UnknownNode(parent.clone()))?;
            if p.frame != node.frame {
                return Err(ModelError::CrossFrameParent { child: node.id, parent: parent.clone() });
            }
            p.children.push(node.id.clone());
        }
        self.nodes.insert(node.id.clone(), node);
        Ok(())
    }

    pub fn add_edge(&mut self, edge: Edge) -> Result<(), ModelError> {
        for id in [&edge.from, &edge.to] {
            if !self.nodes.contains_key(id) {
                return Err(ModelError::UnknownNode(id.clone()));
            }
        }
        self.edges.push(edge);
        Ok(())
    }

    /// Moves `child` under `parent` (or to the top level with `None`).
    pub fn set_parent(&mut self, child: &NodeId, parent: Option<&NodeId>) -> Result<(), ModelError> {
        let c = self.nodes.get(child).ok_or_else(|| ModelError::UnknownNode(child.clone()))?;
        if let Some(p) = parent {
            let pn = self.nodes.get(p).ok_or_else(|| ModelError::UnknownNode(p.clone()))?;
            if pn.frame != c.frame {
                return Err(ModelError::CrossFrameParent { child: child.clone(), parent: p.clone() });
            }
            if p == child || self.ancestors(p).any(|a| a == child) {
                return Err(ModelError::ParentCycle { child: child.clone(), parent: p.clone() });
            }
        }
        if let Some(old) = c.parent.clone() {
            if let Some(o) = self.nodes.get_mut(&old) {
                o.children.retain(|x| x != child);
            }
        }
        if let Some(p) = parent {
            self.nodes[p].children.push(child.clone());
        }
        self.nodes[child].parent = parent.cloned();
        Ok(())
    }

    /// Removes a node, its descendants, and every edge touching them.
    pub fn remove_node(&mut self, id: &NodeId) -> Option<Node> {
        let mut doomed = vec![id.clone()];
        let mut i = 0;
        while i < doomed.len() {
            if let Some(n) = self.nodes.get(&doomed[i]) {
                doomed.extend(n.children.iter().cloned());
            }
            i += 1;
        }
        if let Some(parent) = self.nodes.get(id)?.parent.clone() {
            if let Some(p) = self.nodes.get_mut(&parent) {
                p.children.retain(|c| c != id);
            }
        }
        let removed = self.nodes.shift_remove(id);
        for d in &doomed[1..] {
            self.nodes.shift_remove(d);
        }
        self.edges.retain(|e| !doomed.contains(&e.from) && !doomed.contains(&e.to));
        removed
    }

    pub fn ancestors<'a>(&'a self, id: &'a NodeId) -> impl Iterator<Item = &'a NodeId> + 'a {
        let mut cur = self.nodes.get(id).and_then(|n| n.parent.as_ref());
        let mut guard = self.nodes.len();
        std::iter::from_fn(move || {
            let c = cur?;
            if guard == 0 {
                return None;
            }
            guard -= 1;
            cur = self.nodes.get(c).and_then(|n| n.parent.as_ref());
            Some(c)
        })
    }

    pub fn depth(&self, id: &NodeId) -> usize {
        self.ancestors(id).count()
    }

    /// Top-level nodes of a frame, in document order.
    pub fn roots<'a>(&'a self, frame: &'a FrameId) -> impl Iterator<Item = &'a Node> + 'a {
        self.nodes.values().filter(move |n| &n.frame == frame && n.parent.is_none())
    }

    pub fn nodes_in<'a>(&'a self, frame: &'a FrameId) -> impl Iterator<Item = &'a Node> + 'a {
        self.nodes.values().filter(move |n| &n.frame == frame)
    }

    /// Edges whose endpoints are both in `frame`.
    pub fn edges_in<'a>(&'a self, frame: &'a FrameId) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |e| {
            let f = |id: &NodeId| self.nodes.get(id).map(|n| &n.frame);
            f(&e.from) == Some(frame) && f(&e.to) == Some(frame)
        })
    }

    /// Rebuilds `children` from `parent`. Needed after deserializing, since
    /// `children` is not serialized.
    pub fn relink(&mut self) {
        for n in self.nodes.values_mut() {
            n.children.clear();
        }
        let pairs: Vec<(NodeId, NodeId)> =
            self.nodes.values().filter_map(|n| n.parent.clone().map(|p| (p, n.id.clone()))).collect();
        for (p, c) in pairs {
            if let Some(pn) = self.nodes.get_mut(&p) {
                pn.children.push(c);
            }
        }
    }

    /// Checks every invariant the methods above maintain. Use after building
    /// a document by hand or from untrusted input.
    pub fn validate(&self) -> Vec<ModelError> {
        let mut errs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for f in &self.frames {
            if !seen.insert(&f.id) {
                errs.push(ModelError::DuplicateFrame(f.id.clone()));
            }
        }
        for n in self.nodes.values() {
            if self.frame(&n.frame).is_none() {
                errs.push(ModelError::UnknownFrame(n.frame.clone()));
            }
            if let Some(p) = &n.parent {
                match self.nodes.get(p) {
                    None => errs.push(ModelError::UnknownNode(p.clone())),
                    Some(pn) if pn.frame != n.frame => {
                        errs.push(ModelError::CrossFrameParent { child: n.id.clone(), parent: p.clone() })
                    }
                    _ => {}
                }
                if self.ancestors(&n.id).count() >= self.nodes.len() {
                    errs.push(ModelError::ParentCycle { child: n.id.clone(), parent: p.clone() });
                }
            }
        }
        for e in &self.edges {
            for id in [&e.from, &e.to] {
                if !self.nodes.contains_key(id) {
                    errs.push(ModelError::UnknownNode(id.clone()));
                }
            }
        }
        errs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        let mut d = Document::new();
        d.add_frame(Frame::new("main")).unwrap();
        let f = FrameId::from("main");
        d.add_node(Node::new("a", "A", f.clone())).unwrap();
        let mut b = Node::new("b", "B", f.clone());
        b.parent = Some("a".into());
        d.add_node(b).unwrap();
        let mut c = Node::new("c", "C", f.clone());
        c.parent = Some("b".into());
        d.add_node(c).unwrap();
        d.add_edge(Edge::new("a", "c")).unwrap();
        d
    }

    #[test]
    fn parent_links() {
        let d = doc();
        assert_eq!(d.nodes[&NodeId::from("a")].children, vec![NodeId::from("b")]);
        assert_eq!(d.depth(&"c".into()), 2);
        assert!(d.validate().is_empty());
    }

    #[test]
    fn rejects_cycles() {
        let mut d = doc();
        assert!(matches!(d.set_parent(&"a".into(), Some(&"c".into())), Err(ModelError::ParentCycle { .. })));
        d.set_parent(&"c".into(), Some(&"a".into())).unwrap();
        assert_eq!(d.nodes[&NodeId::from("a")].children.len(), 2);
        assert!(d.nodes[&NodeId::from("b")].children.is_empty());
    }

    #[test]
    fn remove_cascades() {
        let mut d = doc();
        d.remove_node(&"b".into());
        assert_eq!(d.nodes.len(), 1);
        assert!(d.edges.is_empty());
        assert!(d.nodes[&NodeId::from("a")].children.is_empty());
    }

    #[test]
    fn json_roundtrip_relinks() {
        let d = doc();
        let json = serde_json::to_string(&d).unwrap();
        let mut back: Document = serde_json::from_str(&json).unwrap();
        back.relink();
        assert_eq!(back, d);
    }
}
