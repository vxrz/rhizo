//! The text pane beside the canvas: its typing checked off the UI thread,
//! the selection followed both ways, clicks, drags and the clipboard.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use rhizo_app::chrome::Problems;
use rhizo_app::textpane::{self, Extras, Metrics, Outside, Statement};
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::Vec2;
use rhizo_lang::{Diagnostic, Editor};
use winit::event::Ime;

use rhizo_app::field::{self, Edit, Field};

use super::App;
use super::focus::{Focus, TextTarget};
use super::interaction::Target;
use super::layout::{MIN_CANVAS, MIN_PANE};
use super::workers::{Analyzed, Request};

/// What a drag that started in the text pane does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PaneDrag {
    /// Resizes the pane.
    Divider,
    /// Selects by characters (1), words (2) or lines (3), from this line.
    Select(u8, usize),
}

/// The window's side of the pane (the pane itself is [`textpane::Pane`]).
pub struct TextUi {
    pub drag: Option<PaneDrag>,
    /// Clicks in the pane: when, where, and how many in a row.
    clicks: (Instant, Vec2, u8),
    /// The selection the pane last showed (or set), so each side follows the other once.
    pub synced: Option<Target>,
    analyzer: Sender<Request>,
    clipboard: Option<arboard::Clipboard>,
}

impl TextUi {
    pub fn new(analyzer: Sender<Request>) -> TextUi {
        TextUi { drag: None, clicks: (Instant::now(), Vec2::ZERO, 0), synced: None, analyzer, clipboard: None }
    }

    /// Counts a click at `at`: 1, 2 or 3 when quick and close to the last
    /// one, then 1 again.
    pub fn click(&mut self, at: Vec2) -> u8 {
        let (t, last, n) = self.clicks;
        let n = if t.elapsed() < Duration::from_millis(400) && (last - at).hypot() < 6.0 { n % 3 + 1 } else { 1 };
        self.clicks = (Instant::now(), at, n);
        n
    }

    /// The system clipboard, connected on first use.
    pub fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.clipboard = Some(c),
                Err(e) => eprintln!("no clipboard: {e}"),
            }
        }
        self.clipboard.as_mut()
    }
}

impl App {
    /// The pane holds typing the file does not have yet.
    pub fn pane_dirty(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| e.source() != self.pane.buffer.text())
    }

    /// Shows the editor's text after a canvas edit, `name`d for the Edit menu.
    pub fn text_from_editor(&mut self, name: &'static str) {
        let Some(e) = &self.editor else { return };
        self.pane.buffer.replace_all_as(e.source(), name);
        self.pane.refresh();
        self.analyze();
    }

    /// Has the pane's text checked off the UI thread; if it differs from the
    /// file and parses, the answer brings an editor on it, which is saved.
    pub fn analyze(&mut self) {
        if self.editor.is_none() {
            return;
        }
        let save = self.pane_dirty();
        let b = &self.pane.buffer;
        let r = Request { generation: self.session.generation, version: b.version, text: b.text().to_string(), save };
        let _ = self.text.analyzer.send(r);
    }

    pub fn analyzed(&mut self, a: Analyzed) {
        if a.generation != self.session.generation || a.version != self.pane.buffer.version {
            return; // the text has changed since; its own check is on the way
        }
        let errors = a.diagnostics.iter().any(Diagnostic::is_error);
        // Text that is not KDL any more lowers to nothing: keep the spans
        // that followed the typing, just take the problems.
        if errors && a.spans.nodes.is_empty() && !self.pane.spans.nodes.is_empty() {
            self.pane.spans.problems = a.spans.problems;
        } else {
            self.pane.spans = a.spans;
        }
        self.pane.analyzed = a.version;
        let (dirty, in_sync) = (self.pane_dirty(), self.in_sync());
        if errors && (dirty || !in_sync) {
            self.chrome.problems = Problems::new(self.pane.buffer.text(), &a.diagnostics);
        } else if !dirty && in_sync {
            self.chrome.problems = self.built.as_ref().and_then(|b| b.warnings.clone());
        } else if let (Some(ed), true, false) = (a.editor, dirty, self.pane.sync.conflict) {
            self.editor = Some(ed);
            self.save();
        }
        self.selection_to_text();
        self.redraw();
    }

    /// Writes the pane's text as it is, parsed or not: ⌘S, and before the
    /// file is closed or the window quits, so typing is never lost.
    pub fn flush(&mut self) {
        if !self.pane_dirty() {
            return;
        }
        self.pane.sync = textpane::Sync::default();
        let text = self.pane.buffer.text().to_string();
        self.write(&text);
    }

    /// The file on disk now holds `disk`, from outside the window (or our
    /// own save). Shows it unless there is unsaved typing; then the typing
    /// stays and a note says so.
    pub fn outside_change(&mut self, disk: &str) {
        let base = self.editor.as_ref().map(Editor::source);
        match self.pane.sync.outside(self.pane.buffer.text(), base, disk) {
            Outside::Ours => return,
            Outside::Adopt if base.is_none() => self.pane.reset(disk),
            Outside::Adopt => self.pane.buffer.replace_all_as(disk, "Change on disk"),
            Outside::Conflict => {}
        }
        self.editor = Some(Editor::open(disk));
        // Edges are selected by index, which an outside edit may shift.
        if matches!(self.interaction.selected, Some(Target::Edge(_))) {
            self.interaction.selected = None;
            self.interaction.editing = None;
        }
        self.after_text_change();
        self.analyze();
    }

    /// Brings tokens and spans up to date and keeps the caret in view.
    pub fn after_text_change(&mut self) {
        self.pane.refresh();
        if self.pane_shown() {
            let (m, r) = (self.metrics(), self.pane_rect());
            self.pane.reveal_caret(&m, r);
        }
        self.redraw();
    }

    /// After typing: wait for a pause, follow the caret.
    pub fn typed(&mut self) {
        self.pane.sync.typed(Instant::now());
        self.after_text_change();
        self.caret_to_canvas();
    }

    pub fn metrics(&mut self) -> Metrics {
        let (dpr, lines) = (self.scale_factor(), self.pane.buffer.line_count());
        Metrics::new(self.mono(), dpr, lines)
    }

    pub fn focus_pane(&mut self, on: bool) {
        let on = on && self.pane_shown();
        if self.pane.focused != on {
            self.pane.focused = on;
            self.pane.preedit.clear();
            self.redraw();
        }
    }

    pub fn toggle_pane(&mut self) {
        if self.editor.is_none() {
            return self.toast("The text pane edits .rhizo and .kdl files".into());
        }
        let open = !self.pane.open;
        self.change_layout(|a| a.pane.open = open);
        self.focus_pane(open);
        if open {
            self.saw(|s| s.pane = true);
            self.text.synced = None;
            self.selection_to_text();
            let (m, r) = (self.metrics(), self.pane_rect());
            self.pane.reveal_caret(&m, r);
        }
    }

    /// Selects on the canvas the node or edge whose statement holds the caret.
    pub fn caret_to_canvas(&mut self) {
        if self.pane.spans.nodes.is_empty() {
            return; // nothing known about the text yet
        }
        let target = match self.pane.spans.at(self.pane.buffer.head) {
            Some(Statement::Node(id)) => {
                let drawn = self.built.as_ref().is_some_and(|b| b.diagram.nodes.iter().any(|n| n.id == id));
                drawn.then_some(Target::Node(id))
            }
            Some(Statement::Edge(k)) => self.in_sync().then_some(Target::Edge(k)),
            None => None,
        };
        if target != self.interaction.selected {
            self.interaction.selected = target.clone();
            self.reveal_selection();
        }
        self.text.synced = target;
        self.redraw();
    }

    /// Puts the caret on the statement of what was selected on the canvas.
    pub fn selection_to_text(&mut self) {
        if self.interaction.selected == self.text.synced {
            return;
        }
        let statement = match &self.interaction.selected {
            Some(Target::Node(id)) => Statement::Node(id.clone()),
            Some(Target::Edge(k)) => Statement::Edge(*k),
            Some(Target::Frame(_)) | None => {
                self.text.synced = None;
                return;
            }
        };
        // Not known yet (a node just added): tried again once the analysis is in.
        let Some(r) = self.pane.spans.of(&statement).filter(|r| r.end <= self.pane.buffer.text().len()) else {
            return;
        };
        self.text.synced = self.interaction.selected.clone();
        self.pane.buffer.set_caret(r.start, false);
        if self.pane_shown() {
            let (m, rect) = (self.metrics(), self.pane_rect());
            let line = self.pane.buffer.line_of(r.start);
            self.pane.reveal_line(&m, rect, line);
        }
        self.redraw();
    }

    /// The clipboard's text; empty when it holds none.
    pub fn clipboard_text(&mut self) -> String {
        self.text.clipboard().and_then(|c| c.get_text().ok()).unwrap_or_default()
    }

    pub fn set_clipboard(&mut self, text: String) {
        if let Some(c) = self.text.clipboard()
            && let Err(e) = c.set_text(text)
        {
            eprintln!("cannot copy: {e}");
        }
    }

    /// The text that has the keyboard, if any: the pane, an inspector
    /// field, the palette's query, or a label typed on the canvas.
    pub fn focused_text(&self) -> Option<TextTarget> {
        match self.focus() {
            Focus::Pane => Some(TextTarget::Pane),
            Focus::Inspector => Some(TextTarget::Field),
            Focus::Palette => Some(TextTarget::Query),
            Focus::Canvas if self.interaction.editing.is_some() => Some(TextTarget::Label),
            _ => None,
        }
    }

    fn field_of(&mut self, t: TextTarget) -> Option<&mut Field> {
        match t {
            TextTarget::Pane => None,
            TextTarget::Field => self.inspector.typing.as_mut().map(|t| &mut t.field),
            TextTarget::Query => self.palette.open.as_mut().map(|p| &mut p.query),
            TextTarget::Label => self.interaction.editing.as_mut().map(|e| &mut e.field),
        }
    }

    /// Applies an edit to text `t`, with the system clipboard; the pane's
    /// undo is the file's.
    pub fn text_edit(&mut self, t: TextTarget, e: Edit) {
        if t == TextTarget::Pane && matches!(e, Edit::Undo | Edit::Redo) {
            return self.undo(e == Edit::Redo);
        }
        let mut clip = if e == Edit::Paste { self.clipboard_text() } else { String::new() };
        let b = match self.field_of(t) {
            Some(f) => &mut f.buffer,
            None if t == TextTarget::Pane => &mut self.pane.buffer,
            None => return,
        };
        let changed = field::apply(b, &e, &mut clip, t != TextTarget::Pane);
        if matches!(e, Edit::Copy | Edit::Cut) && !clip.is_empty() {
            self.set_clipboard(clip);
        }
        match t {
            TextTarget::Pane if changed => self.typed(),
            TextTarget::Pane => {
                self.after_text_change();
                if matches!(e, Edit::Move(..)) {
                    self.caret_to_canvas();
                }
            }
            TextTarget::Query if changed => {
                if let Some(p) = &mut self.palette.open {
                    p.index = 0;
                }
            }
            _ => {}
        }
        self.redraw();
    }

    /// A click in the pane: places the caret (shift extends), selects a
    /// word on double and a line on triple click; or grabs the divider.
    pub fn pane_press(&mut self) {
        let dpr = self.scale_factor();
        if (self.cursor.x - self.pane_rect().x1).abs() <= 4.0 * dpr {
            self.text.drag = Some(PaneDrag::Divider);
            return;
        }
        if self.interaction.editing.is_some() {
            self.finish_edit(true);
        }
        self.focus_pane(true);
        let (m, r) = (self.metrics(), self.pane_rect());
        let o = self.pane.offset_at(&m, r, self.cursor);
        let n = self.text.click(self.cursor);
        let b = &mut self.pane.buffer;
        let line = b.line_of(o);
        match n {
            1 => b.set_caret(o, self.modifiers.shift_key()),
            2 => b.select_word(o),
            _ => b.select_lines(line, line),
        }
        self.text.drag = Some(PaneDrag::Select(n, line));
        self.caret_to_canvas();
        self.redraw();
    }

    /// Dragging from a press in the pane.
    pub fn pane_drag_to(&mut self, drag: PaneDrag) {
        let dpr = self.scale_factor();
        match drag {
            PaneDrag::Divider => {
                let side = self.side_px();
                let max = ((self.size().x - side) / dpr - MIN_CANVAS).max(MIN_PANE);
                let width = ((self.cursor.x - side) / dpr).clamp(MIN_PANE, max);
                self.change_layout(|a| a.pane.width = width);
            }
            PaneDrag::Select(n, from) => {
                let (m, r) = (self.metrics(), self.pane_rect());
                let o = self.pane.offset_at(&m, r, self.cursor);
                let b = &mut self.pane.buffer;
                match n {
                    3 => b.select_lines(from, b.line_of(o)),
                    _ => b.set_caret(o, true),
                }
                self.pane.reveal_caret(&m, r);
                self.redraw();
            }
        }
    }

    /// ⌘S: the text as typed goes to the file now, errors and all.
    pub fn save_text(&mut self) {
        if self.pane_dirty() {
            self.flush();
        } else if self.session.path.is_some() {
            self.toast("Every edit is saved as you make it".into());
        }
    }

    /// The text pane, in window pixels.
    pub fn pane_scene(&mut self) -> Option<Scene> {
        if !self.pane_shown() {
            return None;
        }
        self.pane.refresh();
        self.mono();
        let (rect, dpr, bg) = (self.pane_rect(), self.scale_factor(), self.background());
        let statement = match &self.interaction.selected {
            Some(Target::Node(id)) => self.pane.spans.of(&Statement::Node(id.clone())),
            Some(Target::Edge(k)) if self.in_sync() => self.pane.spans.of(&Statement::Edge(*k)),
            _ => None,
        };
        let errors = self.pane.spans.problems.iter().any(|p| p.1);
        let note = if self.pane.sync.conflict {
            Some(if cfg!(target_os = "macos") {
                "Changed on disk while you typed · ⌘S keeps yours · ⌘K Revert takes theirs"
            } else {
                "Changed on disk while you typed · Ctrl+S keeps yours · Ctrl+K Revert takes theirs"
            })
        } else if errors && self.pane_dirty() && self.pane.sync.typed.is_none() {
            Some("Not saved: the text has errors · saves once it parses")
        } else {
            None
        };
        let (Some(mono), Some(ui)) = (&self.mono, &self.fonts) else { return None };
        let mut s = Scene::new();
        textpane::draw(&mut s, &self.pane, mono, ui, &mut self.glyphs, rect, dpr, bg, &Extras { statement, note });
        if self.pane.focused {
            let m = Metrics::new(mono, dpr, self.pane.buffer.line_count());
            let c = self.pane.caret_rect(&m, rect);
            if let Some(w) = self.window() {
                w.set_ime_cursor_area(
                    winit::dpi::PhysicalPosition::new(c.x0, c.y0),
                    winit::dpi::PhysicalSize::new(c.width(), c.height()),
                );
            }
        }
        Some(s)
    }

    /// Composed text from an input method, into the focused text.
    pub fn ime(&mut self, ime: Ime) {
        let Some(t) = self.focused_text() else { return };
        match (ime, self.field_of(t)) {
            (Ime::Preedit(text, _), Some(f)) => f.compose(text),
            (Ime::Commit(text), Some(f)) => f.commit(&text),
            (Ime::Preedit(text, _), None) => self.pane.preedit = text,
            (Ime::Commit(text), None) => {
                self.pane.preedit.clear();
                self.pane.buffer.type_text(&text);
                self.typed();
            }
            _ => {}
        }
        if t == TextTarget::Query
            && let Some(p) = &mut self.palette.open
        {
            p.index = 0;
        }
        self.redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_clicks_in_one_place_count_up_to_three() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut t = TextUi::new(tx);
        let at = Vec2::new(10.0, 10.0);
        let counts: Vec<u8> = (0..4).map(|_| t.click(at)).collect();
        assert_eq!(counts, [1, 2, 3, 1]);
        assert_eq!(t.click(at + Vec2::new(20.0, 0.0)), 1, "a click elsewhere starts again");
    }
}
