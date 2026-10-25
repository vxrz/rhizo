//! The inspector (⌘I): what is selected, as a form. A node's type, look,
//! props and acceptance criteria, and where it is defined or used; a
//! frame's type, layout, direction, look and props; an edge's label,
//! kind, line, arrowheads and props; with nothing selected, the document.
//! Every change goes through the editor, so undo, the text pane, saving
//! and the problems banner follow as for any canvas edit. Also the type
//! lists of "Set type…", "Set frame type…" and "Add typed node…".

use rhizo_app::field::Field;
use rhizo_app::files;
use rhizo_app::inspector::{self, Hit, Input, Model, Place, SettingKind, Subject, Typing};
use rhizo_app::ops::{self, Owner};
use rhizo_app::palette::{Entry, List};
use rhizo_app::project::{Kind, Spot};
use rhizo_app::sidebar::Glyph;
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::Rect;
use rhizo_core::{FrameId, NodeId};
use rhizo_kit::Kits;
use serde_json::Value;

use super::App;
use super::commands::Pick;
use super::interaction::{Target, TextEdit};
use super::layout::MIN_CANVAS;

pub struct Inspector {
    pub open: bool,
    /// A text being typed in: a prop, a setting, a criterion.
    pub typing: Option<FieldEdit>,
    /// Where the panel's clickable parts were drawn.
    pub rects: Vec<(Rect, Hit)>,
    pub hover: Option<Hit>,
    /// Shared, so an edit can use them while the window is borrowed.
    pub kits: std::sync::Arc<Kits>,
    /// How far the panel is scrolled, and how tall its content was drawn.
    pub scroll: f64,
    pub height: f64,
}

impl Default for Inspector {
    fn default() -> Inspector {
        Inspector {
            open: false,
            typing: None,
            rects: Vec::new(),
            hover: None,
            kits: std::sync::Arc::new(Kits::builtin()),
            scroll: 0.0,
            height: 0.0,
        }
    }
}

/// What a text of the panel is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    Prop {
        key: String,
        within: Option<(String, String)>,
    },
    /// A setting, by its name.
    Setting(String),
    /// An acceptance criterion, by its index.
    Accept(usize),
    /// A criterion to add.
    NewAccept,
}

pub struct FieldEdit {
    /// What it writes to: the selection when typing began.
    pub owner: Owner,
    pub slot: Slot,
    pub input: Input,
    pub field: Field,
}

impl App {
    /// True when the inspector shows: it is open and there is a diagram.
    pub fn inspector_shown(&self) -> bool {
        self.inspector.open && self.built.is_some() && !self.on_welcome()
    }

    /// The inspector's width in pixels; 0 when it does not show.
    pub fn inspector_px(&self) -> f64 {
        if !self.inspector_shown() {
            return 0.0;
        }
        let dpr = self.scale_factor();
        let room = self.size().x - self.side_px() - self.pane_px() - MIN_CANVAS * dpr;
        (inspector::WIDTH * dpr).min(room).max(0.0)
    }

    /// The inspector sits right of the canvas, above the status bar.
    pub fn inspector_rect(&self) -> Rect {
        let v = self.view_size();
        Rect::new(v.x - self.inspector_px(), 0.0, v.x, v.y)
    }

    pub fn toggle_inspector(&mut self) {
        self.inspector.typing = None;
        self.change_layout(|a| a.inspector.open = !a.inspector.open);
    }

    /// Opens the inspector, if it is not.
    pub fn show_inspector(&mut self) {
        if !self.inspector.open {
            self.change_layout(|a| a.inspector.open = true);
        }
    }

    /// Scrolls the panel by `dy` pixels, within its content.
    pub fn scroll_inspector(&mut self, dy: f64) {
        let max = (self.inspector.height - self.inspector_rect().height()).max(0.0);
        let to = (self.inspector.scroll + dy).clamp(0.0, max);
        if to != self.inspector.scroll {
            self.inspector.scroll = to;
            self.redraw();
        }
    }

    /// The kit to `use` for type `ty`, with the version to ask for.
    pub fn kit_use(&self, ty: &str) -> Option<(String, String)> {
        ops::kit_use(&self.inspector.kits, ty)
    }

    /// The files using node `id` (defined here) and where in each it is first used.
    fn usage_files(&self, id: &NodeId) -> Vec<(String, Spot)> {
        let (Some(scan), Some(rel)) = (&self.workspace.scan, self.current_rel()) else { return Vec::new() };
        let mut out: Vec<(String, Spot)> = Vec::new();
        for u in scan.usages(&rel, id) {
            if !out.iter().any(|(f, _)| *f == u.file) {
                out.push((u.file.clone(), rhizo_app::project::Scan::spot(&u)));
            }
        }
        out
    }

    /// What the selection is, as something the panel writes to.
    pub fn owner(&self) -> Owner {
        match &self.interaction.selected {
            Some(Target::Node(_)) => Owner::Nodes(self.interaction.nodes()),
            Some(Target::Frame(f)) => Owner::Frame(f.clone()),
            Some(Target::Edge(k)) => Owner::Edge(*k),
            None => Owner::Document,
        }
    }

    /// The prop being typed for `owner`, with what it would become.
    fn typing_prop(&self, owner: &Owner) -> Option<(&str, Result<Option<Value>, String>)> {
        let t = self.inspector.typing.as_ref().filter(|t| t.owner == *owner)?;
        match &t.slot {
            Slot::Prop { key, .. } => Some((key.as_str(), t.input.parse(t.field.text()))),
            _ => None,
        }
    }

    /// What the panel shows for the selection: a node (or several: the
    /// props of the one selected last, "mixed" where the others differ), a
    /// frame, an edge, or the document.
    pub fn inspector_model(&self) -> Option<Model> {
        match self.interaction.selected.clone() {
            Some(Target::Frame(f)) => return self.frame_model(&f),
            Some(Target::Edge(k)) => return self.edge_model(k),
            None => return self.document_model(),
            Some(Target::Node(_)) => {}
        }
        let ids = self.interaction.nodes();
        let mut m = self.node_model(&self.selected_node()?)?;
        if ids.len() < 2 {
            return Some(m);
        }
        let built = self.built.as_ref()?;
        let doc = self.editor.as_ref().map_or(&built.document, |e| e.document());
        let nodes: Vec<&rhizo_core::Node> = ids.iter().filter_map(|id| doc.node(id)).collect();
        m.label = format!("{} nodes", ids.len());
        m.id = ids.iter().map(NodeId::as_str).collect::<Vec<_>>().join(", ");
        if nodes.iter().any(|n| n.ty != nodes[0].ty) {
            m.ty = Some(("mixed".into(), Some(("Mixed types".into(), "the selected nodes differ".into()))));
        }
        inspector::mark_mixed(&mut m.fields, nodes.iter().map(|n| &n.props));
        m.place = Place::Selection(ids.len());
        m.editable &= !ids.iter().any(|id| built.references.contains_key(id));
        m.adopt = None;
        m.accept = None;
        // The look rows that differ show empty.
        for s in &mut m.settings {
            let first = nodes.iter().map(|n| ops::node_settings(n)).map(|r| r.into_iter().find(|r| r.name == s.name));
            let all: Vec<_> = first.collect();
            if all.windows(2).any(|w| w[0] != w[1]) {
                s.value = String::new();
                s.placeholder = "mixed".into();
                if let SettingKind::Swatches { chosen, .. } = &mut s.kind {
                    *chosen = None;
                }
            }
        }
        Some(m)
    }

    fn node_model(&self, id: &NodeId) -> Option<Model> {
        let id = id.clone();
        let built = self.built.as_ref()?;
        let reference = built.references.get(&id);
        let doc = match (&self.editor, reference) {
            (Some(e), None) => e.document(),
            _ => &built.document,
        };
        let owner = Owner::Nodes(self.interaction.nodes());
        let typing = self.typing_prop(&owner);
        let place = match reference {
            Some((path, _)) => Place::DefinedIn(
                self.workspace.scan.as_ref().and_then(|s| s.rel(path)).unwrap_or_else(|| files::name(path)),
            ),
            None if self.current_rel().is_some() => {
                Place::UsedIn(self.usage_files(&id).into_iter().map(|(f, _)| f).collect())
            }
            None => Place::Alone,
        };
        let editable = self.editor.is_some() && reference.is_none();
        let mut m = ops::node_model(doc, &self.inspector.kits, &id, editable, typing)?;
        m.place = place;
        m.adopt = reference.is_none().then(|| self.adoption(&id)).flatten();
        Some(m)
    }

    fn frame_model(&self, f: &FrameId) -> Option<Model> {
        let doc = self.editor.as_ref()?.document();
        ops::frame_model(doc, &self.inspector.kits, f, self.typing_prop(&Owner::Frame(f.clone())))
    }

    fn edge_model(&self, k: usize) -> Option<Model> {
        ops::edge_model(self.editor.as_ref()?.document(), k, self.typing_prop(&Owner::Edge(k)))
    }

    fn document_model(&self) -> Option<Model> {
        let file = self.session.path.as_deref().map(files::name).unwrap_or_default();
        Some(ops::document_model(self.editor.as_ref()?.document(), &file))
    }

    /// The inspector, in window pixels.
    pub fn inspector_scene(&mut self) -> Option<Scene> {
        if !self.inspector_shown() {
            self.inspector.rects.clear();
            return None;
        }
        let rect = self.inspector_rect();
        let model = self.inspector_model();
        let typing = self.inspector.typing.as_ref().and_then(|t| {
            let m = model.as_ref()?;
            let at = match &t.slot {
                Slot::Prop { key, .. } => Hit::Field(m.fields.iter().position(|f| f.key == *key)?),
                Slot::Setting(name) => Hit::Setting(m.settings.iter().position(|s| s.name == *name)?),
                Slot::Accept(i) => Hit::Accept(*i),
                Slot::NewAccept => Hit::AcceptAdd,
            };
            Some(Typing { at, field_text: &t.field })
        });
        let (dpr, bg, hover, scroll) =
            (self.scale_factor(), self.background(), self.inspector.hover, self.inspector.scroll);
        let fonts = self.fonts.get_or_insert_with(|| rhizo_render::Fonts::system(rhizo_core::theme::FONT_STACK));
        let mut s = Scene::new();
        let (rects, height) = inspector::draw(
            &mut s,
            fonts,
            &mut self.glyphs,
            model.as_ref(),
            typing.as_ref(),
            hover,
            rect,
            dpr,
            bg,
            scroll,
        );
        self.inspector.rects = rects;
        self.inspector.height = height;
        Some(s)
    }

    /// The part of the panel under the pointer.
    pub fn inspector_hit(&self) -> Option<Hit> {
        let p = self.cursor.to_point();
        self.inspector.rects.iter().find(|(r, _)| r.contains(p)).map(|(_, h)| *h)
    }

    /// Starts typing in the panel.
    pub fn start_typing(&mut self, slot: Slot, input: Input, text: &str) {
        let owner = self.owner();
        self.inspector.typing = Some(FieldEdit { owner, slot, input, field: Field::new(text) });
    }

    /// A press on the panel.
    pub fn inspector_press(&mut self) {
        self.focus_pane(false);
        self.workspace.focused = false;
        let hit = self.inspector_hit();
        let same = match (hit, &self.inspector.typing) {
            (Some(h), Some(t)) => self.inspector_model().is_some_and(|m| match (&t.slot, h) {
                (Slot::Prop { key, .. }, Hit::Field(i)) => m.fields.get(i).is_some_and(|f| f.key == *key),
                (Slot::Setting(n), Hit::Setting(i)) => m.settings.get(i).is_some_and(|s| s.name == *n),
                (Slot::Accept(a), Hit::Accept(i)) => *a == i,
                _ => false,
            }),
            _ => false,
        };
        if !same {
            self.finish_field(true);
        }
        let Some(hit) = hit else { return self.redraw() };
        let Some(m) = self.inspector_model() else { return };
        let owner = self.owner();
        match hit {
            Hit::Type if m.subject == Subject::Frame => self.frame_types_list(),
            Hit::Type => self.types_list(false),
            Hit::Field(_) | Hit::Setting(_) | Hit::Accept(_) if same => self.click_in_field(),
            Hit::Field(i) => {
                let Some(f) = m.fields.into_iter().nth(i) else { return };
                match &f.input {
                    Input::Choice(values) => {
                        let mut entries = Vec::new();
                        let mut picks = Vec::new();
                        for v in values {
                            let detail = if f.value.as_ref() == Some(v) {
                                "current"
                            } else if f.value.is_none() && f.default.as_ref() == Some(v) {
                                "default"
                            } else {
                                ""
                            };
                            let detail = if f.mixed { "" } else { detail };
                            entries.push(Entry { name: inspector::show(v), detail: detail.into(), glyph: None });
                            picks.push(Pick::Prop(owner.clone(), f.within.clone(), f.key.clone(), Some(v.clone())));
                        }
                        if (f.value.is_some() || f.mixed) && !f.required {
                            entries.push(Entry { name: "Not set".into(), detail: String::new(), glyph: None });
                            picks.push(Pick::Prop(owner, f.within.clone(), f.key.clone(), None));
                        }
                        self.show_list(List::Values, entries, picks);
                    }
                    Input::Toggle => {
                        let on = f.value.as_ref().or(f.default.as_ref()).and_then(Value::as_bool).unwrap_or(false);
                        self.set_owner_prop(owner, f.within, f.key, Some(Value::Bool(!on || f.mixed)));
                    }
                    Input::Other => {
                        self.toast(format!("`{}` holds a list or object: edit it in the text (⌘\\)", f.key))
                    }
                    input => {
                        let text = if f.mixed {
                            String::new()
                        } else {
                            f.value.as_ref().map(inspector::show).unwrap_or_default()
                        };
                        self.start_typing(Slot::Prop { key: f.key, within: f.within }, input.clone(), &text);
                    }
                }
            }
            Hit::Setting(i) => {
                let Some(s) = m.settings.into_iter().nth(i) else { return };
                match s.kind {
                    SettingKind::Choice => self.setting_list(owner, &s.name),
                    SettingKind::Text => self.start_typing(Slot::Setting(s.name), Input::Text, &s.value),
                    _ => {}
                }
            }
            Hit::Swatch(i, j) => {
                let Some(s) = m.settings.get(i) else { return };
                if let SettingKind::Swatches { colors, .. } = &s.kind {
                    let value = colors.get(j).copied().flatten().map(ops::hex);
                    self.apply_setting(owner, &s.name, value.as_deref());
                }
            }
            Hit::Accept(i) => {
                let text = m.accept.as_ref().and_then(|a| a.get(i)).cloned().unwrap_or_default();
                self.start_typing(Slot::Accept(i), Input::Text, &text);
            }
            Hit::AcceptRemove(i) => {
                if let Some(id) = self.selected_node() {
                    self.edit("Remove criterion", |e| e.remove_accept(&id, i));
                }
            }
            Hit::AcceptAdd => self.start_typing(Slot::NewAccept, Input::Text, ""),
            Hit::AddTyped => self.types_list(true),
            Hit::Definition => self.go_to_definition(),
            Hit::Usage(i) => {
                let Some(id) = self.selected_node() else { return };
                if let (Some((file, spot)), Some(scan)) =
                    (self.usage_files(&id).into_iter().nth(i), &self.workspace.scan)
                {
                    let path = scan.path(&file);
                    self.go(path, spot);
                }
            }
            Hit::InsertReference => self.references_list(),
            Hit::Adopt => {
                if let Some(id) = self.selected_node() {
                    self.start_adopt(rhizo_app::adopt::Target::Nodes(vec![id.to_string()]))
                }
            }
        }
        self.redraw();
    }

    /// A click in the text being typed: the caret, a word, all of it.
    fn click_in_field(&mut self) {
        let (x, room) = inspector::text_span(self.inspector_rect(), self.scale_factor());
        let n = self.text.click(self.cursor);
        let (shift, dpr) = (self.modifiers.shift_key(), self.scale_factor());
        let fonts = self.fonts.get_or_insert_with(|| rhizo_render::Fonts::system(rhizo_core::theme::FONT_STACK));
        let measure = |t: &str| fonts.shape(t, (13.0 * dpr) as f32, 400).map_or(0.0, |s| s.width as f64);
        if let Some(t) = &mut self.inspector.typing {
            let scrolled = rhizo_app::field::scroll(&t.field, room.max(0.0), measure);
            t.field.click(self.cursor.x - x + scrolled, n, shift, measure);
        }
    }

    /// Ends typing in the panel: `keep` writes it (unless it is not a
    /// valid value, which keeps the field open), else it is dropped. True
    /// when the field is closed.
    pub fn finish_field(&mut self, keep: bool) -> bool {
        let Some(t) = self.inspector.typing.take() else { return true };
        if !keep {
            self.redraw();
            return true;
        }
        let text = t.field.text().to_string();
        match &t.slot {
            Slot::Prop { key, within } => match t.input.parse(&text) {
                Ok(v) => {
                    let differs = self.editor.as_ref().is_some_and(|e| {
                        let doc = e.document();
                        let get = |p: &rhizo_core::Props| match within {
                            Some((parent, child)) => p.get(parent).and_then(|o| o.get(child)).cloned(),
                            None => p.get(key).cloned(),
                        };
                        match &t.owner {
                            Owner::Nodes(ids) => ids.iter().any(|id| doc.node(id).is_some_and(|n| get(&n.props) != v)),
                            Owner::Frame(f) => doc.frame(f).is_some_and(|f| get(&f.props) != v),
                            Owner::Edge(k) => doc.edges.get(*k).is_some_and(|e| get(&e.props) != v),
                            Owner::Document => false,
                        }
                    });
                    if differs {
                        self.set_owner_prop(t.owner, within.clone(), key.clone(), v);
                    }
                }
                Err(e) => {
                    self.toast(format!("`{key}` needs {e}"));
                    self.inspector.typing = Some(t);
                    self.redraw();
                    return false;
                }
            },
            Slot::Setting(name) => {
                let now =
                    self.inspector_model_for(&t.owner).and_then(|m| m.settings.into_iter().find(|s| s.name == *name));
                if now.is_none_or(|s| s.value != text.trim()) {
                    self.apply_setting(t.owner, name, Some(text.as_str()));
                }
            }
            Slot::Accept(i) => {
                let Owner::Nodes(ids) = &t.owner else { return true };
                let (id, i, text) = (ids.last().cloned(), *i, text.trim().to_string());
                let Some(id) = id else { return true };
                let unchanged = self.editor.as_ref().and_then(|e| e.document().node(&id)?.accept.get(i).cloned());
                if unchanged.as_deref() != Some(text.as_str()) {
                    if text.is_empty() {
                        self.edit("Remove criterion", |e| e.remove_accept(&id, i));
                    } else {
                        self.edit("Edit criterion", |e| e.set_accept(&id, i, &text));
                    }
                }
            }
            Slot::NewAccept => {
                let (Owner::Nodes(ids), text) = (&t.owner, text.trim()) else { return true };
                if let (Some(id), false) = (ids.last(), text.is_empty()) {
                    let id = id.clone();
                    self.edit("Add criterion", |e| e.add_accept(&id, text));
                }
            }
        }
        self.redraw();
        true
    }

    /// The model for `owner` even when the selection moved on.
    fn inspector_model_for(&self, owner: &Owner) -> Option<Model> {
        match owner {
            Owner::Frame(f) => self.frame_model(f),
            Owner::Edge(k) => self.edge_model(*k),
            Owner::Document => self.document_model(),
            Owner::Nodes(ids) => self.node_model(ids.last()?),
        }
    }

    /// Tab and shift-Tab: writes the field and types in the next (or the
    /// previous) prop that takes typing.
    pub fn tab_field(&mut self, back: bool) {
        let (Some(t), Some(m)) = (&self.inspector.typing, self.inspector_model()) else { return };
        let Slot::Prop { key, .. } = &t.slot else {
            self.finish_field(true);
            return;
        };
        let inputs: Vec<Input> = m.fields.iter().map(|f| f.input.clone()).collect();
        let Some(at) = m.fields.iter().position(|f| f.key == *key) else { return };
        let owner = t.owner.clone();
        let next = super::focus::next_field(&inputs, at, back);
        if !self.finish_field(true) {
            return;
        }
        // The write rebuilt the form; it has the same fields in the same order.
        let Some(f) = next.and_then(|i| self.inspector_model()?.fields.into_iter().nth(i)) else { return };
        let text = if f.mixed { String::new() } else { f.value.as_ref().map(inspector::show).unwrap_or_default() };
        let slot = Slot::Prop { key: f.key, within: f.within };
        self.inspector.typing = Some(FieldEdit { owner, slot, input: f.input, field: Field::new(&text) });
        self.redraw();
    }

    /// Sets (or removes) a prop of what `owner` names, as one step.
    pub fn set_owner_prop(
        &mut self,
        owner: Owner,
        within: Option<(String, String)>,
        key: String,
        value: Option<Value>,
    ) {
        self.edit("Change prop", |e| ops::set_prop(e, &owner, within.as_ref(), &key, value));
    }

    /// Sets (or clears) setting `name` of `owner`, as one step.
    pub fn apply_setting(&mut self, owner: Owner, name: &str, value: Option<&str>) {
        let edited = self.edit("Change look", |e| ops::apply_setting(e, &owner, name, value));
        if edited.is_none() {
            self.toast(format!("{name} cannot be set to that"));
        }
    }

    /// The values of choice setting `name` (or the kits, for Kits).
    pub fn setting_list(&mut self, owner: Owner, name: &str) {
        if name == ops::KITS {
            return self.kits_list();
        }
        let current = self.inspector_model_for(&owner).and_then(|m| m.settings.into_iter().find(|s| s.name == name));
        let current = current.map(|s| s.value).unwrap_or_default();
        let mut entries = Vec::new();
        let mut picks = Vec::new();
        for (value, shown) in ops::choices(name) {
            let lit = value.unwrap_or("") == current;
            entries.push(Entry {
                name: shown.into(),
                detail: if lit { "current".into() } else { String::new() },
                glyph: None,
            });
            let (owner, name) = (owner.clone(), name.to_string());
            picks.push(Pick::Do(Box::new(move |a: &mut App| a.apply_setting(owner, &name, value))));
        }
        self.show_list(List::Values, entries, picks);
    }

    /// Sets (or removes) the type of `ids`, adding the kit's `use` line if needed.
    pub fn set_type(&mut self, ids: Vec<NodeId>, ty: Option<String>) {
        let kits = self.inspector.kits.clone();
        self.edit("Set type", |e| ops::set_type(e, &kits, &ids, ty.as_deref()));
    }

    /// Where a typed node goes without a node selected: the selected frame,
    /// else the first explicit frame, else the implicit one.
    pub fn frame_for_new(&self) -> Option<FrameId> {
        let doc = self.editor.as_ref()?.document();
        if let Some(Target::Frame(f)) = &self.interaction.selected {
            return Some(f.clone());
        }
        doc.frames.iter().find(|f| !f.implicit).or(doc.frames.first()).map(|f| f.id.clone())
    }

    /// A new node of type `ty` after the selected node (like Enter), or in
    /// [`Self::frame_for_new`]; its label is typed next.
    pub fn add_typed(&mut self, ty: String) {
        let at = match (self.selected_node(), self.frame_for_new()) {
            (Some(s), _) => ops::Spot::After(s),
            (None, Some(f)) => ops::Spot::In(f),
            (None, None) => return,
        };
        let kits = self.inspector.kits.clone();
        let new = self.edit("Add typed node", |e| ops::add_typed(e, &kits, &at, &ty));
        if let Some(id) = new {
            self.label_new(id);
        }
    }

    /// Selects a node just made and starts typing its label.
    pub fn label_new(&mut self, id: NodeId) {
        self.interaction.reveal_pending = true;
        self.interaction.selected = Some(Target::Node(id.clone()));
        self.interaction.editing = Some(TextEdit { target: Target::Node(id), field: Field::default(), created: true });
    }

    /// The type a node added next to `sel` (or under it, `child`) starts
    /// with: its sibling's, when they sit in a typed container (operations
    /// in an API, fields in an entity). Elsewhere new nodes stay plain.
    pub fn inherited_type(&self, sel: &NodeId, child: bool) -> Option<String> {
        let doc = self.editor.as_ref()?.document();
        let sibling = if child { doc.node(sel)?.children.last()? } else { sel };
        let ty = doc.node(sibling)?.ty.as_ref()?;
        rhizo_kit::container_of(doc, sibling)?;
        Some(ty.to_string())
    }

    /// Lists the node types a node can take, by kit, as the kits allow in
    /// its place: for the selection (`new` false) or a node to add (in the
    /// selected frame, or after the selected node).
    pub fn types_list(&mut self, new: bool) {
        if self.editor.is_none() {
            return self.toast("Types are set in .rhizo and .kdl files".into());
        }
        let sel = self.selected_node();
        let ids = self.interaction.nodes();
        let refs = |b: &rhizo_app::Built| ids.iter().any(|id| b.references.contains_key(id));
        if !new && self.built.as_ref().is_some_and(refs) {
            return self.toast("A reference takes its type from the file it points into".into());
        }
        let Some(doc) = self.editor.as_ref().map(|e| e.document()) else { return };
        let container = match (&sel, new) {
            (Some(id), _) => rhizo_kit::container_of(doc, id).map(str::to_string),
            (None, true) => self.frame_for_new().and_then(|f| ops::container_of_frame(doc, &f).map(str::to_string)),
            (None, false) => return,
        };
        let current = sel.as_ref().and_then(|id| doc.node(id)?.ty.as_ref().map(|t| t.to_string()));
        let mut entries = Vec::new();
        let mut picks = Vec::new();
        if let (Some(_), false, Some(_)) = (&sel, new, &current) {
            entries.push(Entry { name: "No type".into(), detail: "a plain node".into(), glyph: None });
            picks.push(Pick::SetType(ids.clone(), None));
        }
        for kit in self.inspector.kits.iter() {
            for def in kit.types.values().filter(|d| d.fits(container.as_deref())) {
                let mark = if current.as_deref() == Some(def.name.as_str()) && !new { " · current" } else { "" };
                entries.push(Entry {
                    name: def.label.clone(),
                    detail: format!("{} · {} kit{mark}", def.name, kit.name),
                    glyph: Some(Glyph::Kind(Kind::of(&def.name))),
                });
                picks.push(match (&sel, new) {
                    (Some(_), false) => Pick::SetType(ids.clone(), Some(def.name.clone())),
                    _ => Pick::AddTyped(def.name.clone()),
                });
            }
        }
        let list = if new { List::NewTyped } else { List::Types };
        self.show_list(list, entries, picks);
    }

    /// Lists the frame types (Environment, API, Workflow…) for the selected frame.
    pub fn frame_types_list(&mut self) {
        let Some(Target::Frame(f)) = self.interaction.selected.clone() else {
            return self.toast("Select a frame to type it".into());
        };
        let Some(doc) = self.editor.as_ref().map(|e| e.document()) else { return };
        let current = doc.frame(&f).and_then(|x| x.ty.as_ref()).map(|t| t.to_string());
        let mut entries = Vec::new();
        let mut picks: Vec<Pick> = Vec::new();
        let set = |ty: Option<String>, f: FrameId| -> Pick {
            Pick::Do(Box::new(move |a: &mut App| {
                let kits = a.inspector.kits.clone();
                a.edit("Set frame type", |e| ops::set_frame_type(e, &kits, &f, ty.as_deref()));
            }))
        };
        if current.is_some() {
            entries.push(Entry { name: "No type".into(), detail: "a plain frame".into(), glyph: None });
            picks.push(set(None, f.clone()));
        }
        for (name, label, kit) in ops::frame_types(&self.inspector.kits) {
            let mark = if current.as_deref() == Some(name) { " · current" } else { "" };
            entries.push(Entry {
                name: label.to_string(),
                detail: format!("{name} · {kit} kit{mark}"),
                glyph: Some(Glyph::Kind(Kind::of(name))),
            });
            picks.push(set(Some(name.to_string()), f.clone()));
        }
        self.show_list(List::Types, entries, picks);
    }
}
