//! What the pointer and the keys do to the diagram: hover, selection,
//! dragging nodes, connecting them, and typing labels on the canvas.

use std::time::Instant;

use std::collections::HashMap;

use rhizo_app::field::Field;
use rhizo_app::interact::{self, Dir, DragMode, FrameDrop};
use rhizo_app::palette::{self, Arrange};
use rhizo_app::vello::kurbo::Vec2;
use rhizo_core::{FrameId, LayoutKind, Node, NodeId, Point};
use rhizo_lang::{EditError, Editor, Placement};

use super::App;

/// Label given to nodes made with Tab or Enter until they are typed over.
pub use rhizo_app::ops::NEW_LABEL;

/// What is selected, or whose label is being typed.
#[derive(Clone, PartialEq, Eq)]
pub enum Target {
    Node(NodeId),
    /// An index into the document's edges.
    Edge(usize),
    /// An explicit frame.
    Frame(FrameId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    /// Move, re-parent or select, per the frame (D19).
    Drag(DragMode),
    /// ⌥-drag, or any drag in a layered/grid frame: draw an edge.
    Connect,
}

pub struct NodeDrag {
    pub id: NodeId,
    pub gesture: Gesture,
    pub start: Point,
    pub now: Point,
    /// Drop placement (outline drags) or the node to connect to.
    pub target: Option<Placement>,
    pub connect_to: Option<NodeId>,
    /// Into another frame, or out to the top level, when dropped here.
    pub frame_drop: Option<FrameDrop>,
}

impl NodeDrag {
    /// True once the pointer went 4 pixels or more: a drag, not a click.
    pub fn moved(&self, zoom: f32) -> bool {
        (self.now.x - self.start.x).hypot(self.now.y - self.start.y) * zoom >= 4.0
    }
}

/// A label being typed in the canvas.
pub struct TextEdit {
    pub target: Target,
    pub field: Field,
    /// Made by Tab/Enter just now: Esc or an empty label removes it again.
    pub created: bool,
}

/// The canvas's part of the window state.
#[derive(Default)]
pub struct Interaction {
    pub hovered: Option<Target>,
    /// The selection; with several nodes selected, the one clicked last.
    pub selected: Option<Target>,
    /// Several selected nodes (shift-click, a selection box, ⌘A), the
    /// selected one among them. Ignored when it does not hold `selected`.
    pub group: Vec<NodeId>,
    /// A selection box being dragged on empty canvas: where it started and
    /// where the pointer is, in window pixels.
    pub band: Option<(Vec2, Vec2)>,
    pub node_drag: Option<NodeDrag>,
    pub editing: Option<TextEdit>,
    /// "Connect to…" from the palette: the next node clicked gets the edge.
    pub connecting: Option<NodeId>,
    /// Where a pan by dragging the background was last.
    pub drag_from: Option<Vec2>,
    pub last_click: Option<(Instant, Vec2)>,
    /// Pan the selection into view once the edit that made it is laid out.
    pub reveal_pending: bool,
    /// Where the open context menu was asked for, in diagram coordinates:
    /// "New frame" goes there.
    pub menu_point: Option<Point>,
}

impl Interaction {
    pub fn selected_node(&self) -> Option<NodeId> {
        match &self.selected {
            Some(Target::Node(id)) => Some(id.clone()),
            _ => None,
        }
    }

    /// Every selected node: the group, or the one selected node.
    pub fn nodes(&self) -> Vec<NodeId> {
        match &self.selected {
            Some(Target::Node(id)) if self.group.contains(id) => self.group.clone(),
            Some(Target::Node(id)) => vec![id.clone()],
            _ => Vec::new(),
        }
    }

    /// Selects `ids`, the last one as the selected node.
    pub fn select_nodes(&mut self, ids: Vec<NodeId>) {
        self.selected = ids.last().cloned().map(Target::Node);
        self.group = if ids.len() > 1 { ids } else { Vec::new() };
    }

    /// Shift-click: adds `id` to the selected nodes, or takes it out.
    pub fn toggle(&mut self, id: NodeId) {
        let mut ids = self.nodes();
        match ids.iter().position(|n| *n == id) {
            Some(i) => {
                ids.remove(i);
            }
            None => ids.push(id),
        }
        self.select_nodes(ids);
    }

    /// Esc: nothing selected, dragged or being connected.
    pub fn escape(&mut self) {
        self.selected = None;
        self.group.clear();
        self.band = None;
        self.node_drag = None;
        self.connecting = None;
    }

    /// Another file: nothing of the last one is hovered, selected or edited.
    pub fn clear(&mut self) {
        self.escape();
        self.hovered = None;
        self.editing = None;
    }
}

impl App {
    pub fn node_at_cursor(&self) -> Option<NodeId> {
        let p = self.to_diagram(self.cursor)?;
        interact::hit(&self.built.as_ref()?.diagram, p).map(|n| n.id.clone())
    }

    /// True when the diagram is built from the editor's current source. Edge
    /// indices only match the drawing then: between an edit and its reload
    /// they may point at another edge, so edges do not hit.
    pub fn in_sync(&self) -> bool {
        matches!((&self.built, &self.editor), (Some(b), Some(e)) if b.source == e.source())
    }

    pub fn edge_at_cursor(&self) -> Option<usize> {
        if !self.in_sync() {
            return None;
        }
        let p = self.to_diagram(self.cursor)?;
        // Six logical pixels, whatever the zoom.
        let tolerance = (6.0 * self.scale_factor() / self.viewport.camera.zoom) as f32;
        interact::hit_edge(&self.built.as_ref()?.diagram, self.editor.as_ref()?.document(), p, tolerance)
    }

    /// The child at the end of the nesting connector under the cursor.
    pub fn branch_child_at_cursor(&self) -> Option<NodeId> {
        let p = self.to_diagram(self.cursor)?;
        let tolerance = (6.0 * self.scale_factor() / self.viewport.camera.zoom) as f32;
        interact::hit_branch(&self.built.as_ref()?.diagram, p, tolerance).cloned()
    }

    pub fn selected_node(&self) -> Option<NodeId> {
        self.interaction.selected_node()
    }

    pub fn selection(&self) -> palette::Selection {
        match self.interaction.selected {
            None if self.on_welcome() => palette::Selection::NoDocument,
            None => palette::Selection::Nothing,
            Some(Target::Node(_)) if self.interaction.nodes().len() > 1 => palette::Selection::Nodes,
            Some(Target::Node(_)) => palette::Selection::Node,
            Some(Target::Edge(_)) => palette::Selection::Edge,
            Some(Target::Frame(_)) => palette::Selection::Frame,
        }
    }

    /// The frame whose label strip is under the pointer.
    pub fn frame_title_at_cursor(&self) -> Option<FrameId> {
        let p = self.to_diagram(self.cursor)?;
        let f = interact::hit_frame(&self.built.as_ref()?.diagram, p)?;
        interact::on_frame_title(f, p).then(|| f.id.clone())
    }

    /// The innermost frame under the pointer.
    pub fn frame_at_cursor(&self) -> Option<FrameId> {
        let p = self.to_diagram(self.cursor)?;
        interact::hit_frame(&self.built.as_ref()?.diagram, p).map(|f| f.id.clone())
    }

    /// Adds a node next to (Enter) or under (Tab) the selection and starts
    /// typing its label. In a typed container it takes its sibling's type.
    pub fn add(&mut self, child: bool) {
        let Some(sel) = self.selected_node() else {
            // Tab with no node selected: a node at the top of the selected
            // frame, else of the loose nodes, else of the first frame.
            if child && !matches!(self.interaction.selected, Some(Target::Edge(_))) {
                self.add_loose();
            }
            return;
        };
        let ty = self.inherited_type(&sel, child);
        let new = self.edit("Add node", |e| {
            let id = if child { e.add_child(&sel, NEW_LABEL)? } else { e.add_after(&sel, NEW_LABEL)? };
            if let Some(ty) = &ty {
                e.set_type(&id, Some(ty))?;
            }
            Ok(id)
        });
        if let Some(id) = new {
            self.label_new(id);
        }
    }

    fn add_loose(&mut self) {
        let Some(doc) = self.editor.as_ref().map(|e| e.document()) else { return };
        let frame = match &self.interaction.selected {
            Some(Target::Frame(f)) => Some(f.clone()),
            _ => doc.frames.iter().find(|f| f.implicit).map(|f| f.id.clone()).or_else(|| self.frame_for_new()),
        };
        let Some(frame) = frame else { return };
        if let Some(id) = self.edit("Add node", |e| e.add_node(&frame, NEW_LABEL)) {
            self.label_new(id);
        }
    }

    /// Starts typing the selected node's or edge's label.
    pub fn start_rename(&mut self) {
        let (Some(target), Some(ed)) = (self.interaction.selected.clone(), &self.editor) else { return };
        let label = match &target {
            Target::Node(id) => ed.document().node(id).map(|n| n.label.clone()),
            Target::Edge(k) => ed.document().edges.get(*k).map(|e| e.label.clone().unwrap_or_default()),
            Target::Frame(f) => ed.document().frame(f).map(|f| f.label.clone().unwrap_or_default()),
        };
        if let Some(text) = label {
            self.interaction.group.clear();
            self.interaction.editing = Some(TextEdit { target, field: Field::new(&text), created: false });
            self.redraw();
        }
    }

    /// Ends label editing. `keep` commits the text; otherwise a node made for
    /// this edit is removed again. An empty edge label removes the label.
    pub fn finish_edit(&mut self, keep: bool) {
        let Some(te) = self.interaction.editing.take() else { return };
        let text = te.field.text().trim().to_string();
        match te.target {
            Target::Edge(k) if keep => {
                let unchanged = self
                    .editor
                    .as_ref()
                    .and_then(|e| e.document().edges.get(k))
                    .is_some_and(|e| e.label.as_deref().unwrap_or("") == text);
                if !unchanged {
                    self.edit("Edit edge label", |e| {
                        e.set_edge_label(k, Some(text.as_str()).filter(|t| !t.is_empty()))
                    });
                }
            }
            Target::Edge(_) => {}
            Target::Node(_) if !keep || (te.created && text.is_empty()) => {
                if te.created {
                    self.undo(false);
                    self.interaction.selected = None;
                }
            }
            // A node just made: its id follows the label, as if written by hand.
            Target::Node(id) if !text.is_empty() && te.created => {
                if let Some(new) = self.edit("Rename", |e| e.rename_new(&id, &text)) {
                    self.interaction.selected = Some(Target::Node(new));
                }
            }
            Target::Node(id) if !text.is_empty() => {
                let unchanged =
                    self.editor.as_ref().and_then(|e| e.document().node(&id)).is_some_and(|n| n.label == text);
                if !unchanged {
                    self.edit("Rename", |e| e.rename(&id, &text));
                }
            }
            Target::Node(_) => {}
            Target::Frame(_) if !keep || (te.created && text.is_empty()) => {
                if te.created {
                    self.undo(false);
                    self.interaction.selected = None;
                }
            }
            Target::Frame(f) if te.created => {
                if let Some(new) = self.edit("Name frame", |e| rhizo_app::ops::name_new_frame(e, &f, &text)) {
                    self.interaction.selected = Some(Target::Frame(new));
                }
            }
            Target::Frame(f) if !text.is_empty() => {
                let unchanged = self
                    .editor
                    .as_ref()
                    .and_then(|e| e.document().frame(&f))
                    .is_some_and(|x| x.label.as_deref() == Some(text.as_str()));
                if !unchanged {
                    self.edit("Rename frame", |e| e.rename_frame(&f, &text));
                }
            }
            Target::Frame(_) => {}
        }
        self.redraw();
    }

    pub fn delete_selected(&mut self) {
        let nodes = self.interaction.nodes();
        if nodes.len() > 1 {
            return self.delete_nodes(&nodes, "Delete");
        }
        let before = self.editor.as_ref().map(|e| e.unused_imports()).unwrap_or_default();
        match self.interaction.selected.clone() {
            Some(Target::Node(id)) => {
                let parent = self.editor.as_ref().and_then(|e| e.document().node(&id)).and_then(|n| n.parent.clone());
                if self.edit("Delete", |e| e.delete(&id)).is_some() {
                    self.interaction.selected = parent.map(Target::Node);
                    self.offer_unused_imports(&before);
                }
            }
            Some(Target::Frame(f)) => self.delete_frame_menu(f),
            Some(Target::Edge(k)) => {
                self.edit("Delete edge", |e| e.disconnect(k));
                self.interaction.selected = None;
            }
            None => {}
        }
    }

    pub fn navigate(&mut self, dir: Dir) {
        self.interaction.group.clear();
        let (Some(sel), Some(b)) = (self.selected_node(), &self.built) else {
            // No node selected yet: start at the first root.
            self.interaction.selected =
                self.built.as_ref().and_then(|b| b.diagram.nodes.first()).map(|n| Target::Node(n.id.clone()));
            self.redraw();
            return;
        };
        if let Some(next) = interact::nearest(&b.diagram, &sel, dir) {
            self.interaction.selected = Some(Target::Node(next));
            self.reveal_selection();
            self.redraw();
        }
    }

    pub fn press(&mut self) {
        if self.interaction.editing.is_some() {
            self.finish_edit(true);
        }
        if let Some(from) = self.interaction.connecting.take() {
            if let Some(to) = self.node_at_cursor().filter(|t| *t != from) {
                self.edit("Connect", |e| e.connect(&from, &to));
            }
            self.redraw();
            return;
        }
        let hit = self.to_diagram(self.cursor).and_then(|p| {
            let built = self.built.as_ref()?;
            let n = interact::hit(&built.diagram, p)?;
            Some((n.id.clone(), interact::drag_mode(&built.diagram, n), p))
        });
        let shift = self.modifiers.shift_key();
        match hit {
            // Shift-click adds the node to the selection, or takes it out.
            Some((id, _, _)) if shift => self.interaction.toggle(id),
            Some((id, mode, p)) => {
                let gesture = if self.modifiers.alt_key() || mode == DragMode::Fixed {
                    Gesture::Connect
                } else {
                    Gesture::Drag(mode)
                };
                // A press inside the selection keeps it, to drag it all.
                if !self.interaction.nodes().contains(&id) {
                    self.interaction.group.clear();
                }
                self.interaction.selected = Some(Target::Node(id.clone()));
                self.interaction.node_drag =
                    Some(NodeDrag { id, gesture, start: p, now: p, target: None, connect_to: None, frame_drop: None });
            }
            None => {
                // A branch is not an edge: clicking one selects the child it leads to.
                let target = self
                    .edge_at_cursor()
                    .map(Target::Edge)
                    .or_else(|| self.branch_child_at_cursor().map(Target::Node))
                    .or_else(|| self.frame_title_at_cursor().map(Target::Frame));
                match target {
                    Some(t) if !shift => {
                        self.interaction.group.clear();
                        self.interaction.selected = Some(t);
                    }
                    // Empty canvas: a selection box (shift adds to what is selected).
                    _ => {
                        if !shift {
                            self.interaction.select_nodes(Vec::new());
                        }
                        self.interaction.band = Some((self.cursor, self.cursor));
                    }
                }
            }
        }
        self.redraw();
    }

    /// The selection box ends: the nodes it meets are selected (shift:
    /// added to the selection).
    fn finish_band(&mut self, (from, to): (Vec2, Vec2)) {
        let (Some(a), Some(b), Some(built)) = (self.to_diagram(from), self.to_diagram(to), &self.built) else { return };
        let r = rhizo_core::Rect::new(a.x.min(b.x), a.y.min(b.y), (a.x - b.x).abs(), (a.y - b.y).abs());
        if (from - to).hypot() < 4.0 {
            // A click on empty canvas: the frame there, if any, is selected.
            let frame = interact::hit_frame(&built.diagram, a).map(|f| f.id.clone());
            if !self.modifiers.shift_key() {
                self.interaction.selected = frame.map(Target::Frame);
            }
            return;
        }
        let mut ids = if self.modifiers.shift_key() { self.interaction.nodes() } else { Vec::new() };
        for id in interact::in_rect(&built.diagram, r) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        self.interaction.select_nodes(ids);
    }

    /// Every drawn node selected (⌘A on the canvas).
    pub fn select_all(&mut self) {
        let ids = self.built.as_ref().map(|b| b.diagram.nodes.iter().map(|n| n.id.clone()).collect());
        self.interaction.select_nodes(ids.unwrap_or_default());
        self.redraw();
    }

    /// Deletes `ids` (each with what it holds) as one step.
    pub fn delete_nodes(&mut self, ids: &[NodeId], name: &'static str) {
        let Some(doc) = self.editor.as_ref().map(Editor::document) else { return };
        let tops = interact::tops(doc, ids);
        let before = self.editor.as_ref().map(|e| e.unused_imports()).unwrap_or_default();
        if self.edit(name, |e| tops.iter().try_for_each(|id| e.delete(id))).is_some() {
            self.interaction.select_nodes(Vec::new());
            self.offer_unused_imports(&before);
        }
    }

    /// The selected nodes as rhizo text on the clipboard.
    pub fn copy_nodes(&mut self) -> bool {
        let ids = self.interaction.nodes();
        let Some(e) = &self.editor else { return false };
        if ids.is_empty() {
            return false;
        }
        let text = interact::copy_text(e.source(), e.document(), &ids);
        self.set_clipboard(text);
        self.toast(format!("Copied {}", rhizo_app::chrome::plural(ids.len(), "node")));
        true
    }

    pub fn cut_nodes(&mut self) {
        let ids = self.interaction.nodes();
        if self.copy_nodes() {
            self.delete_nodes(&ids, "Cut");
        }
    }

    /// Nodes from the clipboard's rhizo text, after the selected node (or
    /// in the first frame), with their children, types, props, looks and
    /// the edges between them, as one step. They get ids of their own.
    pub fn paste_nodes(&mut self) {
        if self.editor.is_none() {
            return self.toast("Pasting nodes works in .rhizo and .kdl files".into());
        }
        let text = self.clipboard_text();
        let from = Editor::open(text.as_str());
        let doc = from.document();
        let roots: Vec<&Node> = doc.nodes.values().filter(|n| n.parent.is_none()).collect();
        if roots.is_empty() || from.diagnostics().iter().any(rhizo_lang::Diagnostic::is_error) {
            return self.toast("The clipboard holds no rhizo nodes".into());
        }
        let kits: HashMap<String, (String, String)> = doc
            .nodes
            .values()
            .filter_map(|n| n.ty.as_ref())
            .filter_map(|t| Some((t.to_string(), self.kit_use(t.as_str())?)))
            .collect();
        let anchor = self.selected_node();
        let frame = self.frame_for_new();
        let made = self.edit("Paste", |e| {
            let mut ids: HashMap<NodeId, NodeId> = HashMap::new();
            let mut after = anchor.clone();
            let mut made = Vec::new();
            for r in &roots {
                let id = match (&after, &frame) {
                    (Some(a), _) => e.add_after(a, &r.label)?,
                    (None, Some(f)) => e.add_node(f, &r.label)?,
                    (None, None) => return Err(EditError::UnknownFrame("main".into())),
                };
                paste_node(e, doc, r, &id, &kits, &mut ids)?;
                let free = e.document().node(&id).and_then(|n| e.document().frame(&n.frame)).map(|f| f.layout);
                if let (Some(at), Some(LayoutKind::Free)) = (r.at, free) {
                    e.move_node(&id, at.x + 24.0, at.y + 24.0)?;
                }
                after = Some(id.clone());
                made.push(id);
            }
            for edge in &doc.edges {
                if let (Some(a), Some(b)) = (ids.get(&edge.from), ids.get(&edge.to)) {
                    e.connect(a, b)?;
                    if let Some(l) = &edge.label {
                        let k = e.document().edges.len() - 1;
                        e.set_edge_label(k, Some(l))?;
                    }
                }
            }
            Ok(made)
        });
        match made {
            Some(ids) => {
                self.interaction.select_nodes(ids);
                self.interaction.reveal_pending = true;
            }
            None => self.toast("These nodes cannot be pasted here".into()),
        }
    }

    /// Lines up or spreads out the selected nodes of free frames, as one step.
    pub fn arrange(&mut self, how: Arrange) {
        let ids = self.interaction.nodes();
        let (Some(b), Some(e)) = (&self.built, &self.editor) else { return };
        let moves = interact::arrange(&b.diagram, e.document(), &ids, how);
        if moves.is_empty() {
            return self.toast("Align and distribute place nodes of free layouts (canvas, board)".into());
        }
        let name = palette::command(palette::Action::Arrange(how)).map_or("Arrange", |c| c.name);
        self.edit(name, |e| moves.iter().try_for_each(|(id, p)| e.move_node(id, p.x, p.y)));
    }

    pub fn drag_to(&mut self, p: Point) {
        let (Some(drag), Some(built)) = (&mut self.interaction.node_drag, &self.built) else { return };
        drag.now = p;
        match drag.gesture {
            Gesture::Drag(DragMode::Outline) => {
                drag.target = self
                    .editor
                    .as_ref()
                    .and_then(|ed| interact::drop_target(&built.diagram, ed.document(), &drag.id, p));
            }
            Gesture::Connect => {
                drag.connect_to = interact::hit(&built.diagram, p).map(|n| n.id.clone()).filter(|t| *t != drag.id);
            }
            Gesture::Drag(_) => {}
        }
        // Off every node: into another frame, or out of this one.
        drag.frame_drop =
            self.editor.as_ref().and_then(|ed| interact::frame_drop(&built.diagram, ed.document(), &drag.id, p));
        self.redraw();
    }

    pub fn release(&mut self) {
        self.interaction.drag_from = None;
        if let Some(band) = self.interaction.band.take() {
            self.finish_band(band);
            return self.redraw();
        }
        let side = self.workspace.drop.take();
        let Some(drag) = self.interaction.node_drag.take() else { return };
        if let Some(thing) = side.and_then(|i| self.workspace.model.things.get(i).cloned()) {
            self.refer(&thing, None, Some(drag.id));
            return self.redraw();
        }
        let delta = Point::new(drag.now.x - drag.start.x, drag.now.y - drag.start.y);
        // A click, not a drag: selecting was all it did.
        if !drag.moved(self.viewport.camera.zoom as f32) {
            self.redraw();
            return;
        }
        if let Some(to) = &drag.frame_drop {
            let ids = if self.interaction.nodes().contains(&drag.id) {
                self.interaction.nodes()
            } else {
                vec![drag.id.clone()]
            };
            let frame = match to {
                FrameDrop::Into(f) => Some(f.clone()),
                FrameDrop::Out => None,
            };
            let Some(doc) = self.editor.as_ref().map(Editor::document) else { return };
            let tops = interact::tops(doc, &ids);
            // A position written for a free layout would pin the node in its new frame.
            let pinned: Vec<NodeId> =
                tops.iter().filter(|id| doc.node(id).is_some_and(|n| n.at.is_some())).cloned().collect();
            let name = if frame.is_some() { "Move into frame" } else { "Move out of frame" };
            self.edit(name, |e| {
                for id in &tops {
                    e.move_to_frame(id, frame.as_ref())?;
                }
                pinned.iter().try_for_each(|id| e.remove_look(id, "x").and_then(|()| e.remove_look(id, "y")))
            });
            return self.redraw();
        }
        match drag.gesture {
            Gesture::Drag(DragMode::Move) => {
                // The selected nodes go together.
                let ids = self.interaction.nodes();
                let moves = match (&self.built, &self.editor) {
                    (Some(b), Some(ed)) => interact::moved(&b.diagram, ed.document(), &ids, delta),
                    _ => Vec::new(),
                };
                if !moves.is_empty() {
                    self.edit("Move", |e| moves.iter().try_for_each(|(id, p)| e.move_node(id, p.x, p.y)));
                }
            }
            Gesture::Drag(DragMode::Outline) => {
                if let Some(target) = drag.target {
                    self.edit("Move in outline", |e| e.reparent(&drag.id, &target));
                }
            }
            Gesture::Connect => {
                if let Some(to) = drag.connect_to {
                    self.edit("Connect", |e| e.connect(&drag.id, &to));
                }
            }
            Gesture::Drag(DragMode::Fixed) => {}
        }
        self.redraw();
    }
}

/// Gives `id`, just made from `from` (a node of the pasted text `doc`),
/// its type, props and looks, then adds its children likewise. `ids` maps
/// the pasted ids to the new ones, for the edges.
fn paste_node(
    e: &mut Editor,
    doc: &rhizo_core::Document,
    from: &Node,
    id: &NodeId,
    kits: &HashMap<String, (String, String)>,
    ids: &mut HashMap<NodeId, NodeId>,
) -> Result<(), EditError> {
    ids.insert(from.id.clone(), id.clone());
    if let Some(ty) = &from.ty {
        if let Some((k, v)) = kits.get(ty.as_str()) {
            e.use_kit(k, v)?;
        }
        e.set_type(id, Some(ty.as_str()))?;
    }
    // Looks as the file writes them: `stroke-width`, not `stroke_width`.
    let looks = serde_json::to_value(&from.style).ok();
    for (key, value) in looks.as_ref().and_then(|v| v.as_object()).into_iter().flatten() {
        e.set_look(id, &key.replace('_', "-"), value)?;
    }
    for (key, value) in &from.props {
        e.set_prop(id, key, value)?;
    }
    for child in from.children.iter().filter_map(|c| doc.node(c)) {
        let made = e.add_child(id, &child.label)?;
        paste_node(e, doc, child, &made, kits, ids)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_app::textpane::Buffer;

    fn ids(names: &[&str]) -> Vec<NodeId> {
        names.iter().map(|n| NodeId::from(*n)).collect()
    }

    #[test]
    fn shift_click_adds_and_removes_nodes() {
        let mut i = Interaction::default();
        i.select_nodes(ids(&["a"]));
        assert_eq!((i.nodes(), i.group.len()), (ids(&["a"]), 0), "one node: no group");
        i.toggle("b".into());
        i.toggle("c".into());
        assert_eq!(i.nodes(), ids(&["a", "b", "c"]));
        assert_eq!(i.selected_node(), Some("c".into()), "the one clicked last is the selected one");
        i.toggle("b".into());
        assert_eq!(i.nodes(), ids(&["a", "c"]));
        i.toggle("c".into());
        assert_eq!((i.nodes(), i.selected_node()), (ids(&["a"]), Some("a".into())));
        i.toggle("a".into());
        assert!(i.nodes().is_empty() && i.selected.is_none());
        // Selecting something else (arrows, the text's caret) leaves the group.
        i.select_nodes(ids(&["a", "b"]));
        i.selected = Some(Target::Node("z".into()));
        assert_eq!(i.nodes(), ids(&["z"]));
        i.select_nodes(ids(&["a", "b"]));
        i.escape();
        assert!(i.nodes().is_empty() && i.group.is_empty());
    }

    const SRC: &str = "rhizo version=1\n- \"Root\" {\n    - \"A\" {\n        - \"A1\"\n    }\n    - \"B\" shape=diamond fill=\"#e11d48\"\n}\nframe \"Board\" layout=free {\n    node \"X\" x=0 y=0\n    node \"Y\" x=200 y=0\n    edge x y label=\"ships\"\n}\n";

    #[test]
    fn deleting_several_nodes_is_one_undo_step() {
        let mut e = Editor::open(SRC);
        let mut history = Buffer::new(SRC.to_string());
        // A1 goes with A: only the tops are deleted.
        let doomed = interact::tops(e.document(), &ids(&["a1", "a", "x"]));
        assert_eq!(doomed, ids(&["a", "x"]));
        doomed.iter().try_for_each(|id| e.delete(id)).unwrap();
        history.replace_all_as(e.source(), "Delete");
        let doc = e.document();
        assert!(doc.node(&"a1".into()).is_none() && doc.node(&"x".into()).is_none() && doc.edges.is_empty());
        assert_eq!(history.undo_name(), Some("Delete"));
        assert!(history.undo());
        assert_eq!(history.text(), SRC, "one undo brings all of them back");
        assert!(!history.undo());
    }

    #[test]
    fn pasted_nodes_keep_children_looks_and_edges_with_ids_of_their_own() {
        let e = Editor::open(SRC);
        let text = interact::copy_text(SRC, e.document(), &ids(&["a", "b", "x", "y"]));
        let from = Editor::open(text.as_str());
        let doc = from.document();
        let mut to = Editor::open(SRC);
        let mut map = HashMap::new();
        let mut after: NodeId = "b".into();
        for root in ["a", "b"].map(|r| &doc.nodes[&NodeId::from(r)]) {
            let id = to.add_after(&after, &root.label).unwrap();
            paste_node(&mut to, doc, root, &id, &HashMap::new(), &mut map).unwrap();
            after = id;
        }
        let board =
            to.document().frames.iter().find(|f| !f.implicit && f.layout == LayoutKind::Free).unwrap().id.clone();
        for root in ["x", "y"].map(|r| &doc.nodes[&NodeId::from(r)]) {
            let id = to.add_node(&board, &root.label).unwrap();
            paste_node(&mut to, doc, root, &id, &HashMap::new(), &mut map).unwrap();
        }
        for edge in &doc.edges {
            to.connect(&map[&edge.from], &map[&edge.to]).unwrap();
        }
        let pasted = to.document();
        assert_eq!(pasted.nodes.len(), e.document().nodes.len() + 5, "A, A1, B, X and Y again");
        let new = |old: &str| &pasted.nodes[&map[&NodeId::from(old)]];
        assert_ne!(map[&NodeId::from("a")], NodeId::from("a"), "a new id");
        assert_eq!(new("a1").parent.as_ref(), Some(&map[&NodeId::from("a")]), "children come along");
        assert_eq!(new("b").style.shape, Some(rhizo_core::Shape::Diamond), "looks too");
        assert_eq!(new("b").style.fill, e.document().nodes[&NodeId::from("b")].style.fill);
        assert!(new("b").props.is_empty(), "looks are not props");
        let edge = pasted.edges.last().unwrap();
        assert_eq!((&edge.from, &edge.to), (&map[&NodeId::from("x")], &map[&NodeId::from("y")]));
    }
}
