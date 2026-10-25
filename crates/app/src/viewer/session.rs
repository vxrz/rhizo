//! The open file (or folder) and the way between files: opening, switching,
//! going to a definition and back, the watcher's reloads, and writing edits.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use rhizo_app::files::{self, Opening};
use rhizo_app::project::Spot;
use rhizo_app::textpane::{self, Statement};
use rhizo_app::tween;
use rhizo_app::vello::kurbo::Vec2;
use rhizo_app::{Built, Failed};
use rhizo_lang::{Diagnostic, EditError, Editor, Syntax};
use rhizo_layout::stable;

use super::App;
use super::interaction::Target;
use super::viewport::progress;
use super::workers::{Reload, Watched};

pub struct Session {
    /// The open file; `None` on the welcome screen.
    pub path: Option<PathBuf>,
    /// A folder opened as a project: the sidebar's root.
    pub folder: Option<PathBuf>,
    /// `--theme`, if given.
    pub theme: Option<String>,
    pub watched: Watched,
    pub generation: u64,
    /// A file to open once the window exists (dialogs need it).
    pub pending_open: Option<PathBuf>,
    pub recent: Vec<PathBuf>,
    /// Files left by jumping to a definition, for ⌘[.
    pub back: Vec<PathBuf>,
    /// Where to go in the file just opened, once it is read.
    pub pending_spot: Option<Spot>,
    /// Wakes the watcher right after a save.
    pub poke: Sender<()>,
    /// Asks the watcher for a layout from scratch ("Tidy layout").
    pub tidy: Arc<AtomicBool>,
}

impl Session {
    /// Has the watcher follow `path`, as a new generation.
    fn follow(&mut self, path: Option<PathBuf>) {
        self.generation += 1;
        *self.watched.lock().expect("watcher alive") = (self.generation, path, self.theme.clone());
        let _ = self.poke.send(());
    }

    /// The window's title: the file's name, else the folder's.
    fn title(&self) -> String {
        match (&self.path, &self.folder) {
            (Some(p), _) => format!("{} — rhizo", files::name(p)),
            (None, Some(dir)) => format!("{} — rhizo", files::name(dir)),
            (None, None) => "rhizo".into(),
        }
    }
}

impl App {
    /// Runs an edit and writes the result back to the file: one step of
    /// the undo history, which the Edit menu calls `name`. An edit made of
    /// several steps that fails part way leaves the file as it was.
    pub fn edit<T>(&mut self, name: &'static str, f: impl FnOnce(&mut Editor) -> Result<T, EditError>) -> Option<T> {
        if self.editor.is_some() && self.pane_dirty() {
            // Typing not yet saved: the edit builds on it, if it parses.
            let ed = Editor::open(self.pane.buffer.text());
            if self.pane.sync.conflict || ed.diagnostics().iter().any(Diagnostic::is_error) {
                self.toast("Fix the text first: canvas edits wait until it parses".into());
                return None;
            }
            self.editor = Some(ed);
            self.pane.sync.typed = None;
        }
        let Some(editor) = &mut self.editor else {
            eprintln!("editing works on .rhizo/.kdl files; this one is read-only here");
            return None;
        };
        let before = editor.source().to_string();
        match f(editor) {
            Ok(v) => {
                self.save();
                self.text_from_editor(name);
                self.redraw();
                Some(v)
            }
            Err(e) => {
                if editor.source() != before {
                    *editor = Editor::open(before);
                }
                eprintln!("{e}");
                None
            }
        }
    }

    /// "Tidy layout": lays the diagram out from scratch, as exports do,
    /// instead of keeping it close to the layout on screen. The selection
    /// stays where it is on screen; the rest eases to its new place.
    pub fn tidy_layout(&mut self) {
        if self.built.is_some() {
            self.session.tidy.store(true, Ordering::Relaxed);
            let _ = self.session.poke.send(());
        }
    }

    /// Writes the edit back (atomically: the watcher never sees half a file).
    pub fn save(&mut self) {
        let Some(editor) = &self.editor else { return };
        let text = editor.source().to_string();
        self.write(&text);
    }

    pub fn write(&mut self, text: &str) {
        let Some(path) = &self.session.path else { return };
        match files::write_atomic(path, text.as_bytes()) {
            Ok(()) => {
                self.chrome.saved_at = Some(Instant::now());
                let _ = self.session.poke.send(());
            }
            Err(e) => self.fail(format!("cannot save {}: {e}", path.display())),
        }
    }

    /// One history for typing and canvas edits: it lives in the pane's
    /// buffer. The text it goes back to is checked and saved like typing.
    pub fn undo(&mut self, redo: bool) {
        let changed = if redo { self.pane.buffer.redo() } else { self.pane.buffer.undo() };
        if changed {
            self.pane.sync.typed = None;
            self.after_text_change();
            self.analyze();
        }
    }

    /// Opens `path` (and goes to `spot` in it once it is read), remembering
    /// where we came from for ⌘[.
    pub fn go(&mut self, path: PathBuf, spot: Spot) {
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        if self.session.path.as_ref() == Some(&path) {
            return self.show_spot(spot);
        }
        if let Some(cur) = self.session.path.clone() {
            self.session.back.push(cur);
        }
        self.open(path);
        self.session.pending_spot = Some(spot);
    }

    /// Selects what `spot` names in the open file; a place in the text also
    /// gets the caret.
    pub fn show_spot(&mut self, spot: Spot) {
        let Some(ed) = &self.editor else { return };
        let target = match &spot {
            Spot::File => return,
            Spot::Node(id) => ed.document().node(id).map(|_| Target::Node(id.clone())),
            Spot::Edge(k) => (*k < ed.document().edges.len()).then_some(Target::Edge(*k)),
            Spot::Offset(o) => match textpane::analyze(ed.source()).1.at(*o) {
                Some(Statement::Node(id)) => Some(Target::Node(id)),
                Some(Statement::Edge(k)) => Some(Target::Edge(k)),
                None => None,
            },
        };
        if target.is_some() || matches!(spot, Spot::Offset(_)) {
            self.interaction.selected = target;
            self.interaction.editing = None;
            self.reveal_selection();
        }
        if let Spot::Offset(o) = spot {
            let o = o.min(self.pane.buffer.text().len());
            self.pane.buffer.set_caret(o, false);
            self.text.synced = self.interaction.selected.clone();
            if self.pane_shown() {
                let (m, r) = (self.metrics(), self.pane_rect());
                let line = self.pane.buffer.line_of(o);
                self.pane.reveal_line(&m, r, line);
            }
        }
        self.redraw();
    }

    /// The selected node is a reference (`model:order`): open the file it
    /// points into, with the node it means selected.
    pub fn go_to_definition(&mut self) {
        let Some(id) = self.selected_node() else { return };
        let target = self.built.as_ref().and_then(|b| b.references.get(&id)).cloned();
        match target {
            Some((path, node)) => self.go(path, Spot::Node(node)),
            None => self.toast(format!("`{id}` is defined here, not in another file")),
        }
    }

    /// Opens a folder as the project: its first file, or an empty canvas
    /// beside the sidebar.
    pub fn open_folder(&mut self, dir: PathBuf) {
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        files::remember(&dir);
        self.session.folder = Some(dir.clone());
        self.workspace.side.open = None;
        match rhizo_project::list(&dir).first() {
            Some(rel) => self.open(dir.join(rel)),
            None => self.switch_to(None),
        }
        self.follow_project();
    }

    /// Shows `path` (or the welcome screen) and has the watcher follow it.
    pub fn switch_to(&mut self, path: Option<PathBuf>) {
        self.flush();
        // An adopt belongs to the file it read: switching away stops it, unwritten.
        if !matches!(self.adopting.job, rhizo_app::adopt::Job::Idle) && self.session.path != path {
            self.adopting.job.cancel();
        }
        self.session.follow(path.clone());
        if let Some(p) = path.as_deref().filter(|p| p.is_file()) {
            files::remember(p);
        }
        self.session.recent = files::recent();
        self.session.path = path;
        self.built = None;
        self.editor = None;
        self.chrome.problems = None;
        self.interaction.clear();
        self.viewport.relayout = None;
        self.viewport.camera_move = None;
        self.palette.open = None;
        self.chrome.saved_at = None;
        self.chrome.welcome_focus = None;
        self.pane.reset("");
        self.text.synced = None;
        self.session.pending_spot = None;
        self.focus_pane(false);
        self.follow_project();
        self.expand_current();
        self.update_title();
        self.redraw();
    }

    /// Builds the open file again as a new generation, so a change of theme
    /// redraws it ("Tidy layout" is `tidy_layout`).
    pub fn rebuild(&mut self) {
        if self.session.path.is_some() {
            let path = self.session.path.clone();
            self.session.follow(path);
        }
        self.redraw();
    }

    /// Draws the window in `theme` from now on (like `--theme`).
    pub fn set_theme(&mut self, theme: &str) {
        self.session.theme = Some(theme.to_string());
        self.rebuild();
        self.toast(format!("{} theme", theme[..1].to_uppercase() + &theme[1..]));
    }

    pub fn update_title(&self) {
        if let Some(w) = self.window() {
            w.set_title(&self.session.title());
        }
    }

    /// Opens `path`: rhizo files directly, other formats by importing them
    /// into a new `.rhizo` file first.
    pub fn open(&mut self, path: PathBuf) {
        let path = std::fs::canonicalize(&path).or_else(|_| std::path::absolute(&path)).unwrap_or(path);
        if path.is_dir() {
            return self.open_folder(path);
        }
        // A file from elsewhere leaves the folder.
        if self.session.folder.as_ref().is_some_and(|d| !path.starts_with(d)) {
            self.session.folder = None;
        }
        match files::opening(&path) {
            Opening::Direct => self.switch_to(Some(path)),
            Opening::Import(format) => self.import(&path, format),
            Opening::Unsupported => self.fail(format!(
                "rhizo cannot open {}: try .rhizo, .kdl, .json, .yaml or a format it imports",
                files::name(&path)
            )),
        }
    }

    pub fn set_built(&mut self, built: Built, took: Duration) {
        eprintln!("{} nodes in {:.0?}", built.nodes, took);
        // Typing with errors keeps its own problems in the banner.
        if !(self.pane_dirty() && self.pane.spans.problems.iter().any(|p| p.1)) {
            self.chrome.problems = built.warnings.clone();
        }
        if self.built.is_none() {
            // First build: also the moment a file the window waited for appears.
            if let Some(p) = &self.session.path {
                files::remember(p);
            }
        }
        // Our own saves come back here too; keep the editor (and the undo
        // history) when the file is what it already holds.
        if built.syntax != Syntax::Kdl {
            self.editor = None;
            self.pane.reset("");
        } else {
            self.outside_change(&built.source);
        }
        if let Some(spot) = self.session.pending_spot.take() {
            self.show_spot(spot);
        }
        if let Some(sel) = &self.interaction.selected {
            let gone = self.editor.as_ref().is_some_and(|e| match sel {
                Target::Node(id) => e.document().node(id).is_none(),
                Target::Edge(k) => *k >= e.document().edges.len(),
                Target::Frame(f) => e.document().frame(f).is_none(),
            });
            if gone {
                self.interaction.selected = None;
                self.interaction.editing = None;
            }
        }
        if let Some(e) = &self.editor {
            self.interaction.group.retain(|id| e.document().node(id).is_some());
        }
        let Some(old) = self.built.replace(built) else {
            self.fit(false);
            return;
        };
        let built = self.built.as_ref().expect("just set");
        let prefer = stable::anchors(&old.document, &built.document, self.selected_node().as_ref());
        // Start from wherever a running relayout has got to.
        let old = match self.viewport.relayout.take() {
            Some((from, shift, t0)) => tween::frame(&from, &old.diagram, shift, progress(t0)),
            None => old.diagram,
        };
        // Hold still on screen what the layout held still (else the edited
        // node), even when the drawing grew on the left or top.
        let shift = stable::anchor_shift(&old, &built.diagram, &prefer);
        let (ob, nb) = (old.bounds, built.diagram.bounds);
        let pan = Vec2::new((nb.x - ob.x - shift.x) as f64, (nb.y - ob.y - shift.y) as f64);
        self.viewport.follow_origin(pan);
        if built.nodes <= tween::MAX_ANIMATED_NODES {
            self.viewport.relayout = Some((old, shift, Instant::now()));
        }
        if std::mem::take(&mut self.interaction.reveal_pending) {
            self.reveal_selection();
        }
        self.redraw();
    }

    /// A build from the watcher: shows it, or its problems over the last
    /// good diagram.
    pub fn reloaded(&mut self, r: Reload) {
        if r.generation != self.session.generation {
            return; // a file that has since been closed
        }
        match r.result {
            Ok((b, took)) => {
                self.set_built(*b, took);
                // The drawing moved under the pointer.
                self.update_pointer();
            }
            Err(f) => self.failed(f),
        }
        self.selection_to_text();
    }

    fn failed(&mut self, f: Failed) {
        eprintln!("{}", f.report);
        self.chrome.problems = Some(f.problems);
        // Still show the text, so it can be fixed right here.
        let kdl = self.session.path.as_deref().is_some_and(|p| files::syntax_of(p) == Syntax::Kdl);
        if let Some(disk) = self.session.path.as_ref().filter(|_| kdl).and_then(|p| std::fs::read_to_string(p).ok()) {
            self.outside_change(&disk);
            // A problem jumped to: the caret goes there.
            if let Some(spot) = self.session.pending_spot.take() {
                self.show_spot(spot);
            }
        }
        self.redraw();
    }
}
