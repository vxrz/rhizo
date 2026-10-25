//! The project around the open file, and the sidebar that shows it: its
//! files, their typed things and problems; references between files.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use rhizo_app::palette::{Action, List};
use rhizo_app::project::{self, Opened, Problem, Scan, Spot, Thing};
use rhizo_app::sidebar::{self, Glyph, Marks, Model, Row, Sidebar};
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::Vec2;
use rhizo_core::{FrameId, NodeId, Point};
use rhizo_lang::Editor;

use super::App;
use super::commands::Pick;
use super::interaction::Target;

/// A thing being dragged out of the sidebar, by its index in the model.
pub struct SideDrag {
    pub thing: usize,
    pub from: Vec2,
    pub moved: bool,
}

pub struct Workspace {
    /// What the project watcher follows, and its generation.
    pub watched: project::Watched,
    pub generation: u64,
    pub poke: Sender<()>,
    /// The project as last read, and the sidebar's view of it.
    pub scan: Option<Scan>,
    pub model: Model,
    pub side: Sidebar,
    /// The file whose things were opened in the sidebar on switching to it.
    pub expanded_for: Option<PathBuf>,
    pub hover: Option<Row>,
    pub hover_footer: bool,
    pub resizing: bool,
    pub drag: Option<SideDrag>,
    /// A sidebar thing an ⌥-drag from a node would connect to.
    pub drop: Option<usize>,
    /// The sidebar has the keys: arrows move `hover`, Enter opens it.
    pub focused: bool,
}

impl Workspace {
    pub fn new(watched: project::Watched, poke: Sender<()>) -> Workspace {
        Workspace {
            watched,
            generation: 0,
            poke,
            scan: None,
            model: Model::default(),
            side: Sidebar::default(),
            expanded_for: None,
            hover: None,
            hover_footer: false,
            resizing: false,
            drag: None,
            drop: None,
            focused: false,
        }
    }

    /// Has the watcher follow `opened` (a new generation), unless it does.
    fn follow(&mut self, opened: Option<Opened>) {
        let mut watched = self.watched.lock().expect("watcher alive");
        if watched.1 != opened {
            self.generation += 1;
            *watched = (self.generation, opened);
            let _ = self.poke.send(());
        }
    }

    /// The glyph and label of a thing being dragged out onto the canvas.
    pub fn drag_chip(&self) -> Option<(Glyph, String)> {
        let chip = self.drag.as_ref().filter(|d| d.moved).and_then(|d| self.model.things.get(d.thing));
        chip.map(|t| (Glyph::Kind(t.kind), if t.label.is_empty() { t.id.clone() } else { t.label.clone() }))
    }
}

impl App {
    /// Has the project watcher follow what is open: the folder, else the
    /// file, unless the file belongs to the project shown already.
    pub fn follow_project(&mut self) {
        let opened = match (&self.session.folder, &self.session.path) {
            (Some(dir), _) => Some(Opened::Folder(dir.clone())),
            (None, Some(p)) if self.workspace.scan.as_ref().is_some_and(|s| s.rel(p).is_some()) => return,
            (None, Some(p)) => Some(Opened::File(p.clone())),
            (None, None) => None,
        };
        self.workspace.follow(opened);
    }

    /// A new reading of the project: the sidebar, problems and lists follow.
    pub fn scanned(&mut self, scan: Scan) {
        if scan.generation != self.workspace.generation {
            return;
        }
        self.change_layout(|a| {
            a.workspace.model = Model::new(&scan);
            a.workspace.scan = Some(scan);
        });
        self.expand_current();
        // A file that no longer belongs here (moved, or a folder closed) is followed afresh.
        if self.session.folder.is_none()
            && self
                .session
                .path
                .as_ref()
                .is_some_and(|p| self.workspace.scan.as_ref().is_some_and(|s| s.rel(p).is_none()))
        {
            self.follow_project();
        }
        self.redraw();
    }

    /// Opens the open file's things in the sidebar, once per switch.
    pub fn expand_current(&mut self) {
        if self.workspace.expanded_for == self.session.path {
            return;
        }
        if let Some(rel) = self.current_rel() {
            self.workspace.side.expanded.insert(rel);
            self.workspace.expanded_for = self.session.path.clone();
        }
    }

    /// The open file, relative to the project root.
    pub fn current_rel(&self) -> Option<String> {
        self.workspace.scan.as_ref()?.rel(self.session.path.as_deref()?)
    }

    pub fn toggle_sidebar(&mut self) {
        if self.workspace.scan.as_ref().is_none_or(|s| s.files.is_empty() && self.session.folder.is_none()) {
            return self.toast("No project here: open a folder of .rhizo files".into());
        }
        let shown = self.sidebar_shown();
        self.change_layout(|a| a.workspace.side.open = Some(!shown));
    }

    /// Every problem in the project, as a list to jump from.
    pub fn problems_list(&mut self) {
        let Some(scan) = &self.workspace.scan else { return };
        let problems = scan.problems();
        if problems.is_empty() {
            return self.toast("No problems in the project".into());
        }
        let entries = problems.iter().map(Problem::entry).collect();
        let picks =
            problems.iter().map(|p| Pick::Go(scan.path(&p.rel), p.offset.map_or(Spot::File, Spot::Offset))).collect();
        self.show_list(List::Problems, entries, picks);
    }

    /// The typed nodes of the other files, to refer to from this one.
    pub fn references_list(&mut self) {
        if self.editor.is_none() {
            return self.toast("References go into .rhizo and .kdl files".into());
        }
        let (Some(scan), Some(cur)) = (&self.workspace.scan, self.current_rel()) else {
            return self.toast("This file is not part of a project yet".into());
        };
        let things: Vec<Thing> = scan.things().into_iter().filter(|t| !t.frame && t.rel != cur).collect();
        if things.is_empty() {
            return self.toast("The other files define nothing typed to refer to".into());
        }
        let entries = things.iter().map(Thing::entry).collect();
        self.show_list(List::References, entries, things.into_iter().map(Pick::Refer).collect());
    }

    /// The defined node a selected node stands for: itself, or what it refers to.
    pub fn defined(&self, id: &NodeId) -> Option<(String, NodeId)> {
        let scan = self.workspace.scan.as_ref()?;
        match self.built.as_ref().and_then(|b| b.references.get(id)) {
            Some((path, target)) => Some((scan.rel(path)?, target.clone())),
            None => Some((self.current_rel()?, id.clone())),
        }
    }

    /// Where the selected node is used in other files.
    pub fn usages_list(&mut self) {
        let Some(id) = self.selected_node() else { return };
        let Some((rel, local)) = self.defined(&id) else {
            return self.toast("This file is not part of a project yet".into());
        };
        let Some(scan) = &self.workspace.scan else { return };
        let usages = scan.usages(&rel, &local);
        if usages.is_empty() {
            return self.toast(format!("`{local}` is not used in other files"));
        }
        let entries = usages.iter().map(|u| scan.usage_entry(u)).collect();
        let picks = usages.iter().map(|u| Pick::Go(scan.path(&u.file), Scan::spot(u))).collect();
        self.show_list(List::Usages, entries, picks);
    }

    /// Puts a reference to `thing` into the open file, in `frame` (else the
    /// selected node's), connected from `from` if given; selects it.
    pub fn refer(&mut self, thing: &Thing, frame: Option<FrameId>, from: Option<NodeId>) -> Option<NodeId> {
        if self.editor.is_none() {
            self.toast("References go into .rhizo and .kdl files".into());
            return None;
        }
        let Some(cur) = self.current_rel() else {
            self.toast("This file is not part of a project yet".into());
            return None;
        };
        if thing.rel == cur {
            self.toast(format!("“{}” is defined in this file", thing.label));
            return None;
        }
        let import = rhizo_project::import_path(&cur, &thing.rel);
        let doc = self.editor.as_ref().map(Editor::document);
        let explicit = |f: &FrameId| doc.and_then(|d| d.frame(f)).is_some_and(|f| !f.implicit);
        let near = from.clone().or_else(|| self.selected_node());
        let frame = frame.or_else(|| doc?.node(near.as_ref()?).map(|n| n.frame.clone())).filter(explicit);
        let (id, label) = (thing.id.clone(), thing.label.clone());
        let made = self.edit("Insert reference", |e| {
            let r = e.insert_reference(&import, &id, &label, frame.as_ref())?;
            if let Some(from) = &from {
                e.connect(from, &r)?;
            }
            Ok(r)
        })?;
        self.interaction.selected = Some(Target::Node(made.clone()));
        self.interaction.reveal_pending = true;
        Some(made)
    }

    /// The innermost drawn frame under diagram point `p`.
    pub fn frame_at(&self, p: Point) -> Option<FrameId> {
        let b = self.built.as_ref()?;
        b.diagram.frames.iter().rfind(|f| f.visible && f.rect.contains(p)).map(|f| f.id.clone())
    }

    /// A press in the sidebar: resize, list problems, open a file or start
    /// dragging a thing.
    pub fn side_press(&mut self) {
        let Some(layout) = self.side_layout() else { return };
        let p = self.cursor.to_point();
        if layout.on_divider(p) {
            self.workspace.resizing = true;
        } else if layout.footer.contains(p) {
            self.run(Action::ProblemsMenu);
        } else if let Some(row) = layout.row_at(p) {
            match row {
                Row::File(f) => {
                    let on_chevron = p.x < layout.rect.x0 + 26.0 * self.scale_factor();
                    let rel = self.workspace.model.files[f].rel.clone();
                    if on_chevron && !self.workspace.side.expanded.remove(&rel) {
                        self.workspace.side.expanded.insert(rel);
                    } else if !on_chevron {
                        self.side_activate(row);
                    }
                }
                Row::Thing(i) => self.workspace.drag = Some(SideDrag { thing: i, from: self.cursor, moved: false }),
                Row::Group(..) => {}
            }
        }
        self.redraw();
    }

    /// A click on a row, or Enter on it: a file opens (the open one folds
    /// or unfolds), a thing is gone to.
    pub fn side_activate(&mut self, row: Row) {
        match row {
            Row::File(f) => {
                let Some(rel) = self.workspace.model.files.get(f).map(|f| f.rel.clone()) else { return };
                if self.current_rel().as_ref() == Some(&rel) {
                    if !self.workspace.side.expanded.remove(&rel) {
                        self.workspace.side.expanded.insert(rel);
                    }
                } else if let Some(path) = self.workspace.scan.as_ref().map(|s| s.path(&rel)) {
                    self.go(path, Spot::File);
                }
            }
            Row::Thing(i) => {
                let Some(thing) = self.workspace.model.things.get(i).cloned() else { return };
                let spot = if thing.frame { Spot::File } else { Spot::Node(NodeId(thing.id.clone())) };
                if let Some(path) = self.workspace.scan.as_ref().map(|s| s.path(&thing.rel)) {
                    self.go(path, spot);
                }
            }
            Row::Group(..) => {}
        }
        self.redraw();
    }

    /// The end of a press in the sidebar: a click on a thing goes to it; a
    /// drag onto the canvas places a reference to it there.
    pub fn side_release(&mut self) {
        self.workspace.resizing = false;
        let Some(drag) = self.workspace.drag.take() else { return };
        let Some(thing) = self.workspace.model.things.get(drag.thing).cloned() else { return };
        if !drag.moved {
            self.side_activate(Row::Thing(drag.thing));
        } else if let Some(at) = self.to_diagram(self.cursor) {
            if thing.frame {
                self.toast("Frames cannot be referenced; drag one of their nodes".into());
            } else {
                let frame = self.frame_at(at);
                self.refer(&thing, frame, None);
            }
        }
        self.redraw();
    }

    /// The thing under the pointer in the sidebar, if a node of another file.
    pub fn side_thing_at_cursor(&mut self) -> Option<usize> {
        let row = self.side_layout()?.row_at(self.cursor.to_point())?;
        let Row::Thing(i) = row else { return None };
        let t = self.workspace.model.things.get(i)?;
        (!t.frame && Some(&t.rel) != self.current_rel().as_ref()).then_some(i)
    }

    /// The project sidebar, in window pixels.
    pub fn side_scene(&mut self) -> Option<Scene> {
        let layout = self.side_layout()?;
        let current = self.current_rel();
        let selected = self.selected_node();
        let (dpr, bg) = (self.scale_factor(), self.background());
        let marks = Marks {
            current: current.as_deref(),
            selected: selected.as_ref().map(NodeId::as_str),
            hover: self.workspace.hover.filter(|_| self.workspace.drag.is_none()),
            drop: self.workspace.drop.map(Row::Thing),
            hover_footer: self.workspace.hover_footer,
        };
        self.fonts();
        let fonts = self.fonts.as_ref().expect("loaded above");
        let mut s = Scene::new();
        sidebar::draw(&mut s, fonts, &mut self.glyphs, &self.workspace.model, &layout, dpr, bg, &marks);
        Some(s)
    }
}
