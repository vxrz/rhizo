//! Frames from the canvas and the registry: new, group (⌘G), ungroup,
//! delete with or without what they hold, their type, layout and
//! direction; and the lists behind the look, edge and document commands.

use rhizo_app::field::Field;
use rhizo_app::inspector::{COLOUR_NAMES, FILLS, Input, STROKES};
use rhizo_app::ops::{self, Owner};
use rhizo_app::palette::{Entry, List};
use rhizo_core::FrameId;

use super::App;
use super::commands::Pick;
use super::inspect::Slot;
use super::interaction::{Target, TextEdit};

/// Label of a frame just made, until it is typed over.
pub const NEW_FRAME: &str = "New frame";

impl App {
    pub fn selected_frame(&self) -> Option<FrameId> {
        match &self.interaction.selected {
            Some(Target::Frame(f)) => Some(f.clone()),
            _ => None,
        }
    }

    /// Selects a frame just made and starts typing its label.
    pub fn label_new_frame(&mut self, f: FrameId) {
        self.interaction.select_nodes(Vec::new());
        self.interaction.selected = Some(Target::Frame(f.clone()));
        self.interaction.editing = Some(TextEdit { target: Target::Frame(f), field: Field::default(), created: true });
    }

    /// An empty frame: inside the selected frame, else at the point
    /// right-clicked (pinned there), else at the end of the file.
    pub fn new_frame(&mut self) {
        let parent = self.selected_frame();
        let point = self.interaction.menu_point.take();
        let at = point.filter(|_| parent.is_none()).map(|p| (p.x.round(), p.y.round()));
        if let Some(f) = self.edit("New frame", |e| e.add_frame(parent.as_ref(), NEW_FRAME, at)) {
            self.label_new_frame(f);
        }
    }

    /// ⌘G: the selected nodes into a new frame, with the edges among them.
    pub fn group_selected(&mut self) {
        let ids = self.interaction.nodes();
        if ids.is_empty() {
            return;
        }
        match self.edit("Group into frame", |e| e.group(&ids, NEW_FRAME)) {
            Some(f) => self.label_new_frame(f),
            None => self.toast("Group nodes at the top of one frame (not nested in another node)".into()),
        }
    }

    /// The selected frame goes; what it held stays where it was.
    pub fn ungroup(&mut self) {
        let Some(f) = self.selected_frame() else { return };
        if self.edit("Ungroup frame", |e| e.delete_frame(&f, true)).is_some() {
            self.interaction.selected = None;
        }
    }

    /// Delete on a frame asks: keep what it holds, or delete it all.
    pub fn delete_frame_menu(&mut self, f: FrameId) {
        let n = self.editor.as_ref().map_or(0, |e| e.document().nodes.values().filter(|n| n.frame == f).count());
        let entries = vec![
            Entry { name: "Delete the frame, keep what it holds".into(), detail: "ungroup".into(), glyph: None },
            Entry {
                name: "Delete the frame and everything in it".into(),
                detail: rhizo_app::chrome::plural(n, "node"),
                glyph: None,
            },
        ];
        let del = |keep: bool, f: FrameId| -> Pick {
            Pick::Do(Box::new(move |a: &mut App| {
                let before = a.editor.as_ref().map(|e| e.unused_imports()).unwrap_or_default();
                if a.edit("Delete frame", |e| e.delete_frame(&f, keep)).is_some() {
                    a.interaction.selected = None;
                    a.offer_unused_imports(&before);
                }
            }))
        };
        self.show_list(List::Values, entries, vec![del(true, f.clone()), del(false, f)]);
    }

    /// The frame a layout command is about: the selected one, the one the
    /// selected node or edge sits in; `Document` for the nodes outside frames.
    fn layout_owner(&self) -> Owner {
        let Some(doc) = self.editor.as_ref().map(|e| e.document()) else { return Owner::Document };
        let frame = match &self.interaction.selected {
            Some(Target::Frame(f)) => Some(f.clone()),
            Some(Target::Node(id)) => doc.node(id).map(|n| n.frame.clone()),
            Some(Target::Edge(k)) => doc.edges.get(*k).and_then(|e| doc.node(&e.from)).map(|n| n.frame.clone()),
            None => None,
        };
        match frame.filter(|f| doc.frame(f).is_some_and(|f| !f.implicit)) {
            Some(f) => Owner::Frame(f),
            None => Owner::Document,
        }
    }

    /// "Set layout…" and "Set direction…".
    pub fn layout_list(&mut self, name: &str) {
        let owner = self.layout_owner();
        self.setting_list(owner, name);
    }

    /// "Set fill colour…" and "Set stroke colour…", for the selected nodes or frame.
    pub fn colour_list(&mut self, fill: bool) {
        let owner = self.owner();
        if !matches!(owner, Owner::Nodes(_) | Owner::Frame(_)) {
            return self.toast("Select nodes or a frame to colour them".into());
        }
        let (colors, name) = if fill { (FILLS, ops::FILL) } else { (STROKES, ops::STROKE) };
        let mut entries = Vec::new();
        let mut picks = Vec::new();
        for (c, label) in colors.iter().zip(COLOUR_NAMES) {
            let hex = c.map(ops::hex);
            entries.push(Entry { name: label.into(), detail: hex.clone().unwrap_or_default(), glyph: None });
            let owner = owner.clone();
            picks.push(Pick::Do(Box::new(move |a: &mut App| a.apply_setting(owner, name, hex.as_deref()))));
        }
        self.show_list(List::Values, entries, picks);
    }

    /// Types a new acceptance criterion for the selected node, in the inspector.
    pub fn add_accept(&mut self) {
        if self.interaction.nodes().len() != 1 {
            return self.toast("Select one node to add a criterion to it".into());
        }
        self.show_inspector();
        self.start_typing(Slot::NewAccept, Input::Text, "");
    }

    /// Types the document's title, in the inspector.
    pub fn edit_title(&mut self) {
        self.interaction.escape();
        self.show_inspector();
        let title = self.editor.as_ref().and_then(|e| e.document().title.clone()).unwrap_or_default();
        self.start_typing(Slot::Setting(ops::TITLE.into()), Input::Text, &title);
    }

    /// The kits, to use one (adding its `use` line) or stop using it.
    pub fn kits_list(&mut self) {
        let Some(doc) = self.editor.as_ref().map(|e| e.document()) else { return };
        let mut entries = Vec::new();
        let mut picks = Vec::new();
        for kit in self.inspector.kits.iter().filter(|k| k.name != rhizo_kit::IMPLICIT) {
            let used = doc.kits.iter().any(|k| k.kit == kit.name);
            let detail = if used {
                "in use · choose to stop using it".to_string()
            } else {
                kit.description.clone().unwrap_or_default()
            };
            entries.push(Entry { name: kit.name.clone(), detail, glyph: None });
            let (name, version) = (kit.name.clone(), format!("{}.{}", kit.version.major, kit.version.minor));
            picks.push(Pick::Do(Box::new(move |a: &mut App| {
                if used {
                    a.edit("Stop using kit", |e| e.unuse_kit(&name));
                } else {
                    a.edit("Use kit", |e| e.use_kit(&name, &version));
                }
            })));
        }
        self.show_list(List::Values, entries, picks);
    }

    /// Removes every `import` no node refers to any more.
    pub fn remove_unused_imports(&mut self) {
        let unused = self.editor.as_ref().map(|e| e.unused_imports()).unwrap_or_default();
        if unused.is_empty() {
            return self.toast("Every import is in use".into());
        }
        if self.edit("Remove unused imports", |e| unused.iter().try_for_each(|a| e.remove_import(a))).is_some() {
            self.toast(format!("Removed {}", rhizo_app::chrome::plural(unused.len(), "unused import")));
        }
    }

    /// After a delete: offers to remove the imports it left unused (those
    /// not in `before`).
    pub fn offer_unused_imports(&mut self, before: &[String]) {
        let Some(e) = &self.editor else { return };
        let doc = e.document();
        let new: Vec<String> = e.unused_imports().into_iter().filter(|a| !before.contains(a)).collect();
        if new.is_empty() {
            return;
        }
        let mut entries = Vec::new();
        let mut picks = Vec::new();
        for alias in new {
            let path = doc.imports.get(&alias).cloned().unwrap_or_default();
            entries.push(Entry { name: format!("Remove the unused import `{alias}`"), detail: path, glyph: None });
            picks.push(Pick::Do(Box::new(move |a: &mut App| {
                a.edit("Remove import", |e| e.remove_import(&alias));
            })));
        }
        entries.push(Entry { name: "Keep it".into(), detail: "nothing refers to it any more".into(), glyph: None });
        picks.push(Pick::Do(Box::new(|_| {})));
        self.show_list(List::Values, entries, picks);
    }
}

#[cfg(test)]
mod tests {
    use rhizo_app::inspector::{COLOUR_NAMES, FILLS, STROKES};

    #[test]
    fn every_swatch_has_a_name() {
        assert_eq!(COLOUR_NAMES.len(), FILLS.len());
        assert_eq!(COLOUR_NAMES.len(), STROKES.len());
    }
}
