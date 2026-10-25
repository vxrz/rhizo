//! The ⌘K palette and the actions it (and the keys) run.

use std::path::{Path, PathBuf};

use rhizo_app::field::{Edit, Field};
use rhizo_app::files;
use rhizo_app::ops::Owner;
use rhizo_app::palette::{self, Action, Entry, List};
use rhizo_app::project::{Spot, Thing};
use rhizo_app::textpane;
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::Rect;
use rhizo_core::NodeId;
use serde_json::Value;

use super::App;
use super::focus::Focus;

/// The palette, while open, and where its rows were drawn.
#[derive(Default)]
pub struct Palette {
    pub open: Option<PaletteState>,
    /// Where the palette's rows were drawn, with their index, for clicks.
    pub rects: Vec<(Rect, usize)>,
}

pub struct PaletteState {
    pub list: List,
    pub query: Field,
    pub index: usize,
    /// The entries of a list the window fills, and what choosing each does.
    pub entries: Vec<Entry>,
    pub picks: Vec<Pick>,
}

/// What choosing an entry of a palette list does.
pub enum Pick {
    /// A file or folder, as Open… would.
    Open(PathBuf),
    /// A file, and a place in it; ⌘[ comes back.
    Go(PathBuf, Spot),
    /// Puts a reference to this thing into the open file.
    Refer(Thing),
    /// Sets or (`None`) removes the type of nodes.
    SetType(Vec<NodeId>, Option<String>),
    /// Adds a node of this type.
    AddTyped(String),
    /// Sets or (`None`) removes a prop of what the owner names (inside
    /// an object prop, when one is named).
    Prop(Owner, Option<(String, String)>, String, Option<Value>),
    /// Anything else the window does with the choice.
    Do(Box<dyn FnOnce(&mut App)>),
}

impl App {
    /// A command from the menu bar, a context menu, the palette or a key:
    /// the text ones (cut, copy, paste, select all, undo) go to the text
    /// that has the keys, if one does; the rest to [`App::run`].
    pub fn command(&mut self, action: Action) {
        let edit = match action {
            Action::Cut => Some(Edit::Cut),
            Action::Copy => Some(Edit::Copy),
            Action::Paste => Some(Edit::Paste),
            Action::SelectAll => Some(Edit::SelectAll),
            Action::Undo => Some(Edit::Undo),
            Action::Redo => Some(Edit::Redo),
            _ => None,
        };
        if let (Some(t), Some(e)) = (self.focused_text(), edit) {
            return self.text_edit(t, e);
        }
        // The adopt report waits for Adopt or Cancel.
        if self.focus() == Focus::Dialog && action != Action::Quit {
            return;
        }
        self.run(action);
    }

    pub fn run(&mut self, action: Action) {
        match action {
            Action::AddChild => self.add(true),
            Action::AddSibling => self.add(false),
            Action::Rename => self.start_rename(),
            Action::Delete => self.delete_selected(),
            Action::Connect => self.interaction.connecting = self.selected_node(),
            Action::Undo => self.undo(false),
            Action::Redo => self.undo(true),
            Action::Fit => self.fit(true),
            Action::TidyLayout => self.tidy_layout(),
            Action::ZoomActual => {
                let dpr = self.scale_factor();
                self.zoom_by(|z| dpr / z);
            }
            Action::ZoomIn => self.zoom_by(|_| 1.25),
            Action::ZoomOut => self.zoom_by(|_| 0.8),
            Action::NewMenu => self.show_palette(List::Templates),
            Action::New(kind) => self.new_file(kind),
            Action::Open => self.open_dialog(),
            Action::OpenFolder => self.open_folder_dialog(),
            Action::RecentMenu => {
                self.session.recent = files::recent();
                let picks = self.session.recent.iter().cloned().map(Pick::Open).collect();
                self.show_list(List::Recent, palette::path_entries(&self.session.recent), picks);
            }
            Action::GoToFile => {
                let paths: Vec<PathBuf> = match &self.workspace.scan {
                    Some(s) if !s.files.is_empty() => s.files.iter().map(|rel| s.path(rel)).collect(),
                    _ => {
                        let dir = self.session.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
                        dir.map(|d| rhizo_project::list(&d).into_iter().map(|rel| d.join(rel)).collect())
                            .unwrap_or_default()
                    }
                };
                let picks = paths.iter().map(|p| Pick::Go(p.clone(), Spot::File)).collect();
                self.show_list(List::Project, palette::path_entries(&paths), picks);
            }
            Action::Pick(_) => {}
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::ProblemsMenu => self.problems_list(),
            Action::InsertReferenceMenu => self.references_list(),
            Action::FindUsages => self.usages_list(),
            Action::GoToDefinition => self.go_to_definition(),
            Action::ToggleInspector => self.toggle_inspector(),
            Action::AddTypedMenu => self.types_list(true),
            Action::ChangeTypeMenu => self.types_list(false),
            Action::Back => match self.session.back.pop() {
                Some(p) => self.open(p),
                None => self.toast("Nothing to go back to".into()),
            },
            Action::SaveAs => self.save_as(),
            Action::Export => self.export(),
            Action::Close => self.switch_to(None),
            Action::Reveal => self.hand_to_system(files::reveal),
            Action::OpenInEditor => self.hand_to_system(files::open_in_editor),
            Action::TogglePane => self.toggle_pane(),
            Action::SaveText => self.save_text(),
            Action::AdoptNode => match self.selected_node() {
                Some(id) => self.start_adopt(rhizo_app::adopt::Target::Nodes(vec![id.to_string()])),
                None => self.toast("Select a node of imported code to adopt".into()),
            },
            Action::AdoptAll => self.start_adopt(rhizo_app::adopt::Target::All),
            Action::CancelAdopt => self.cancel_adopt(),
            Action::Cut => self.cut_nodes(),
            Action::Copy => {
                self.copy_nodes();
            }
            Action::Paste => self.paste_nodes(),
            Action::SelectAll => self.select_all(),
            Action::FindNode => self.nodes_list(),
            Action::Arrange(how) => self.arrange(how),
            Action::Theme(t) => self.set_theme(t),
            Action::Compile => self.compile_dialog(),
            Action::Brief => self.brief(),
            Action::Shortcuts => self.show_palette(List::Shortcuts),
            Action::Guide => self.guide(),
            Action::Palette => self.show_palette(List::Commands),
            Action::Quit => self.quitting = true,
            Action::About => self.open_link(rhizo_app::about::Link::About),
            Action::NewFrame => self.new_frame(),
            Action::GroupFrame => self.group_selected(),
            Action::Ungroup => self.ungroup(),
            Action::FrameTypeMenu => self.frame_types_list(),
            Action::LayoutMenu => self.layout_list(rhizo_app::ops::LAYOUT),
            Action::DirectionMenu => self.layout_list(rhizo_app::ops::DIRECTION),
            Action::ShapeMenu => self.setting_list(self.owner(), rhizo_app::ops::SHAPE),
            Action::FillMenu => self.colour_list(true),
            Action::StrokeMenu => self.colour_list(false),
            Action::AddAccept => self.add_accept(),
            Action::EdgeKindMenu => self.setting_list(self.owner(), rhizo_app::ops::KIND),
            Action::LineMenu => self.setting_list(self.owner(), rhizo_app::ops::LINE),
            Action::ArrowMenu => self.setting_list(self.owner(), rhizo_app::ops::ARROW),
            Action::EditTitle => self.edit_title(),
            Action::KitsMenu => self.kits_list(),
            Action::DocThemeMenu => self.setting_list(Owner::Document, rhizo_app::ops::THEME),
            Action::RemoveUnusedImports => self.remove_unused_imports(),
            Action::Learn => self.learn(),
            Action::Gallery => self.open_gallery(),
            Action::ToggleHints => self.toggle_hints(),
            Action::RevertText => {
                if let Some(e) = &self.editor {
                    let disk = e.source().to_string();
                    self.pane.sync = textpane::Sync::default();
                    self.pane.buffer.replace_all_as(&disk, "Revert text");
                    self.after_text_change();
                    self.analyze();
                }
            }
        }
        self.redraw();
    }

    pub fn show_palette(&mut self, list: List) {
        if list == List::Commands {
            self.saw(|s| s.palette = true);
            let (entries, picks) = self.palette_extras();
            return self.show_list(list, entries, picks);
        }
        self.show_list(list, Vec::new(), Vec::new());
    }

    /// What ⌘K finds besides commands: the kits' types, to add a node of
    /// one, and the open file's nodes, to go to one (see [`palette::rows`]).
    fn palette_extras(&self) -> (Vec<Entry>, Vec<Pick>) {
        let (mut entries, mut picks) = (Vec::new(), Vec::new());
        let Some(doc) = self.editor.as_ref().map(|e| e.document()) else { return (entries, picks) };
        for kit in self.inspector.kits.iter() {
            // Frame types (an environment, an API) are not nodes: "New frame" and "Set frame type…" find them.
            for def in kit.types.values().filter(|d| !d.frame) {
                let glyph = Some(rhizo_app::sidebar::Glyph::Kind(rhizo_app::project::Kind::of(&def.name)));
                entries.push(Entry {
                    name: def.label.clone(),
                    detail: format!("{} · {} kit", def.name, kit.name),
                    glyph,
                });
                picks.push(Pick::AddTyped(def.name.clone()));
            }
        }
        if let Some(path) = &self.session.path {
            for n in doc.nodes.values().filter(|n| !n.label.is_empty()) {
                entries.push(Entry { name: n.label.clone(), detail: format!("node · {}", n.id), glyph: None });
                picks.push(Pick::Go(path.clone(), Spot::Node(n.id.clone())));
            }
        }
        (entries, picks)
    }

    pub fn show_list(&mut self, list: List, entries: Vec<Entry>, picks: Vec<Pick>) {
        self.menu = None;
        self.palette.open = Some(PaletteState { list, query: Field::default(), index: 0, entries, picks });
    }

    /// The open file's nodes, to go to one (⌘F).
    fn nodes_list(&mut self) {
        let (Some(b), Some(path)) = (&self.built, &self.session.path) else { return };
        let doc = self.editor.as_ref().map_or(&b.document, |e| e.document());
        let nodes: Vec<&rhizo_core::Node> = b.diagram.nodes.iter().filter_map(|n| doc.node(&n.id)).collect();
        let entries = nodes
            .iter()
            .map(|n| Entry {
                name: if n.label.is_empty() { n.id.to_string() } else { n.label.clone() },
                detail: n.ty.as_ref().map_or_else(|| n.id.to_string(), |t| format!("{} · {t}", n.id)),
                glyph: None,
            })
            .collect();
        let picks = nodes.iter().map(|n| Pick::Go(path.clone(), Spot::Node(n.id.clone()))).collect();
        self.show_list(List::Nodes, entries, picks);
    }

    pub fn pick(&mut self, pick: Pick) {
        match pick {
            Pick::Open(p) => self.open(p),
            Pick::Go(p, spot) => self.go(p, spot),
            Pick::Refer(thing) => {
                self.refer(&thing, None, None);
            }
            Pick::SetType(id, ty) => self.set_type(id, ty),
            Pick::AddTyped(ty) => self.add_typed(ty),
            Pick::Prop(owner, within, key, value) => self.set_owner_prop(owner, within, key, value),
            Pick::Do(f) => f(self),
        }
    }

    /// Runs the palette's chosen row and closes it.
    pub fn choose(&mut self, index: usize) {
        let Some(p) = self.palette.open.take() else { return };
        let rows = palette::rows(p.list, p.query.text(), self.selection(), &p.entries);
        match rows.get(index).map(|r| r.action) {
            Some(Action::Pick(i)) => {
                if let Some(pick) = p.picks.into_iter().nth(i) {
                    self.pick(pick);
                }
            }
            Some(a) => self.command(a),
            None => {}
        }
        self.redraw();
    }

    /// The ⌘K palette, in window pixels.
    pub fn palette_scene(&mut self) -> Option<Scene> {
        let p = self.palette.open.as_ref()?;
        let rows = palette::rows(p.list, p.query.text(), self.selection(), &p.entries);
        let (bg, dpr, size) = (self.background(), self.scale_factor(), self.size());
        let fonts = self.fonts.get_or_insert_with(|| rhizo_render::Fonts::system(rhizo_core::theme::FONT_STACK));
        let mut s = Scene::new();
        let q = (p.list, &p.query, p.index);
        self.palette.rects = palette::draw_query(&mut s, fonts, &mut self.glyphs, size, dpr, bg, q, &rows);
        Some(s)
    }
}
