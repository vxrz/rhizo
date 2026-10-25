//! The keyboard. A key goes to the part that has the keys ([`Focus`]): the
//! adopt report, a context menu, the palette, an inspector field, the text
//! pane, the sidebar or the canvas (where a label may be typed). What it
//! leaves goes to the ⌘ shortcuts, then the welcome screen or the canvas.
//! What a key means is worked out by the plain functions here
//! ([`shortcut`], [`canvas_key`], [`welcome_key`], [`motion`],
//! [`text_key`]); the `App` methods carry it out.

use rhizo_app::field::Edit;
use rhizo_app::interact::Dir;
use rhizo_app::palette::{self, Action, List};
use rhizo_app::sidebar::Row;
use rhizo_app::textpane::Motion;
use winit::event::KeyEvent;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

use super::App;
use super::focus::{Focus, TextTarget};

/// What a ⌘ (Ctrl) shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Palette,
    Undo {
        redo: bool,
    },
    Run(Action),
    /// ⌘W in an opened folder with no file open: back to the welcome screen.
    CloseFolder,
    /// ⌘W with nothing open.
    Quit,
}

/// What a key does while the adopt report waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKey {
    /// ⌘Enter: the Adopt button.
    Adopt,
    Cancel,
    /// Scroll by this many lines.
    Scroll(isize),
}

/// The report takes every key; only these do something. Plain ↩ does not
/// adopt: writing needs the button or ⌘Enter.
pub fn report_key(key: &Key<&str>, cmd: bool) -> Option<ReportKey> {
    Some(match key {
        Key::Named(NamedKey::Escape) => ReportKey::Cancel,
        Key::Named(NamedKey::Enter) if cmd => ReportKey::Adopt,
        Key::Named(NamedKey::ArrowDown) => ReportKey::Scroll(1),
        Key::Named(NamedKey::ArrowUp) => ReportKey::Scroll(-1),
        Key::Named(NamedKey::PageDown | NamedKey::Space) => ReportKey::Scroll(20),
        Key::Named(NamedKey::PageUp) => ReportKey::Scroll(-20),
        _ => return None,
    })
}

/// What is open, for the shortcuts that need it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Open {
    pub file: bool,
    pub folder: bool,
    /// A node is selected.
    pub node: bool,
}

/// Whether a ⌘ shortcut works with what is `open`; the palette and menus
/// say the same with the registry's [`palette::Needs`].
fn takes(action: Action, open: Open) -> bool {
    match action {
        Action::TogglePane | Action::InsertReferenceMenu | Action::ToggleInspector => open.file,
        Action::Paste | Action::SelectAll | Action::FindNode => open.file,
        Action::GoToFile | Action::ToggleSidebar | Action::ProblemsMenu => open.file || open.folder,
        Action::FindUsages | Action::Cut | Action::Copy | Action::GroupFrame => open.node,
        _ => true,
    }
}

/// The shortcut ⌘ (Ctrl) plus the character `c` (lowercased) is, if any:
/// the registry's chord for it ([`palette::chord`]).
pub fn shortcut(c: &str, shift: bool, open: Open) -> Option<Shortcut> {
    match (c, shift) {
        ("w", false) if !open.file && open.folder => return Some(Shortcut::CloseFolder),
        ("w", false) if !open.file => return Some(Shortcut::Quit),
        _ => {}
    }
    // ⇧⌘K opens the palette too.
    let shift = shift && c != "k";
    let command = palette::COMMANDS.iter().find(|k| palette::chord(k.keys) == Some((shift, false, c.to_string())))?;
    Some(match command.action {
        _ if !takes(command.action, open) => return None,
        Action::Palette => Shortcut::Palette,
        Action::Undo => Shortcut::Undo { redo: false },
        Action::Redo => Shortcut::Undo { redo: true },
        a => Shortcut::Run(a),
    })
}

/// Modifier keys as texts read them: ⌘ (Ctrl), shift, and what moves by
/// line (⌘ on macOS) and by word (⌥ on macOS, Ctrl elsewhere).
#[derive(Clone, Copy, Debug, Default)]
pub struct Mods {
    pub cmd: bool,
    pub shift: bool,
    pub line: bool,
    pub word: bool,
}

/// What a key does in a text, the pane's or a field's: `typed` is the
/// text it types, `page` the lines PageUp and PageDown move. Esc, and in
/// fields Enter and Tab, are the owner's.
pub fn text_key(key: &Key<&str>, typed: Option<&str>, m: Mods, page: usize) -> Option<Edit> {
    if let Some(motion) = motion(key, m.line, m.word, m.cmd, page) {
        return Some(Edit::Move(motion, m.shift));
    }
    let (back, ahead) = match (m.line, m.word) {
        (true, _) => (Motion::LineStart, Motion::LineEnd),
        (_, true) => (Motion::WordLeft, Motion::WordRight),
        _ => (Motion::Left, Motion::Right),
    };
    Some(match key {
        Key::Named(NamedKey::Backspace) => Edit::Delete(back),
        Key::Named(NamedKey::Delete) => Edit::Delete(ahead),
        Key::Named(NamedKey::Enter) => Edit::Newline,
        Key::Named(NamedKey::Tab) => Edit::Indent { outdent: m.shift },
        Key::Character(c) if m.cmd => match c.to_lowercase().as_str() {
            "a" => Edit::SelectAll,
            "c" => Edit::Copy,
            "x" => Edit::Cut,
            "v" => Edit::Paste,
            "z" if m.shift => Edit::Redo,
            "z" => Edit::Undo,
            _ => return None,
        },
        _ if !m.cmd => Edit::Type(typed?.to_string()),
        _ => return None,
    })
}

/// What a key does in the sidebar, which has `rows` (the one at `at`
/// marked): the row to mark, or to open (Enter) or fold (left and right).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideKey {
    Mark(Row),
    Activate(Row),
    Fold(Row, bool),
    Leave,
}

pub fn side_key(key: &Key<&str>, rows: &[Row], at: Option<usize>) -> Option<SideKey> {
    let row = |i: usize| rows.get(i).copied();
    Some(match key {
        Key::Named(NamedKey::Escape) => SideKey::Leave,
        Key::Named(NamedKey::ArrowDown) => {
            SideKey::Mark(row(at.map_or(0, |i| (i + 1).min(rows.len().saturating_sub(1))))?)
        }
        Key::Named(NamedKey::ArrowUp) => SideKey::Mark(row(at.map_or(0, |i| i.saturating_sub(1)))?),
        Key::Named(NamedKey::Enter | NamedKey::Space) => SideKey::Activate(row(at?)?),
        Key::Named(NamedKey::ArrowRight) => SideKey::Fold(row(at?)?, true),
        Key::Named(NamedKey::ArrowLeft) => SideKey::Fold(row(at?)?, false),
        _ => return None,
    })
}

/// What a key does on the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasKey {
    /// ⌘Esc.
    Quit,
    /// Esc: nothing selected, dragged or being connected.
    Escape,
    Add {
        child: bool,
    },
    Rename,
    Delete,
    Navigate(Dir),
    Run(Action),
}

pub fn canvas_key(key: &Key<&str>, cmd: bool) -> Option<CanvasKey> {
    Some(match key {
        Key::Named(NamedKey::Escape) if cmd => CanvasKey::Quit,
        Key::Named(NamedKey::Escape) => CanvasKey::Escape,
        Key::Named(NamedKey::Tab) => CanvasKey::Add { child: true },
        Key::Named(NamedKey::Enter) => CanvasKey::Add { child: false },
        Key::Named(NamedKey::F2) => CanvasKey::Rename,
        Key::Named(NamedKey::Backspace | NamedKey::Delete) => CanvasKey::Delete,
        Key::Named(NamedKey::ArrowLeft) => CanvasKey::Navigate(Dir::Left),
        Key::Named(NamedKey::ArrowRight) => CanvasKey::Navigate(Dir::Right),
        Key::Named(NamedKey::ArrowUp) => CanvasKey::Navigate(Dir::Up),
        Key::Named(NamedKey::ArrowDown) => CanvasKey::Navigate(Dir::Down),
        Key::Character("f" | "F") => CanvasKey::Run(Action::Fit),
        Key::Character("0") => CanvasKey::Run(Action::ZoomActual),
        Key::Character("=" | "+") => CanvasKey::Run(Action::ZoomIn),
        Key::Character("-") => CanvasKey::Run(Action::ZoomOut),
        _ => return None,
    })
}

/// What a key does on the welcome screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WelcomeKey {
    Focus(usize),
    /// Enter or Space: runs the focused item (or focuses the first).
    Choose,
    Unfocus,
}

/// Arrow keys, Tab and Enter on the welcome screen: tiles across, rows
/// down. There are `n` items, the first `tiles` of them tiles.
pub fn welcome_key(key: &Key<&str>, focus: Option<usize>, n: usize, tiles: usize) -> Option<WelcomeKey> {
    let i = focus;
    let next = match key {
        Key::Named(NamedKey::ArrowRight | NamedKey::Tab) => i.map_or(0, |i| (i + 1).min(n - 1)),
        Key::Named(NamedKey::ArrowLeft) => i.map_or(0, |i| i.saturating_sub(1)),
        Key::Named(NamedKey::ArrowDown) => i.map_or(0, |i| if i < tiles { tiles } else { (i + 1).min(n - 1) }),
        Key::Named(NamedKey::ArrowUp) => i.map_or(0, |i| match i {
            _ if i < tiles => i,
            _ if i == tiles => 0,
            _ => i - 1,
        }),
        Key::Named(NamedKey::Enter | NamedKey::Space) => return Some(WelcomeKey::Choose),
        Key::Named(NamedKey::Escape) => return Some(WelcomeKey::Unfocus),
        _ => return None,
    };
    Some(WelcomeKey::Focus(next))
}

/// Where a key moves the pane's caret: by character, by word (`word`), to
/// the ends of the line or the file (`line`, ⌘ on macOS; `cmd` with Home
/// and End), or a `page` of lines.
pub fn motion(key: &Key<&str>, line: bool, word: bool, cmd: bool, page: usize) -> Option<Motion> {
    let horizontal = |back: bool| match (line, word, back) {
        (true, _, true) => Motion::Home,
        (true, _, false) => Motion::LineEnd,
        (_, true, true) => Motion::WordLeft,
        (_, true, false) => Motion::WordRight,
        (_, _, true) => Motion::Left,
        (_, _, false) => Motion::Right,
    };
    match key {
        Key::Named(NamedKey::ArrowLeft) => Some(horizontal(true)),
        Key::Named(NamedKey::ArrowRight) => Some(horizontal(false)),
        Key::Named(NamedKey::ArrowUp) => Some(if line { Motion::DocStart } else { Motion::Up(1) }),
        Key::Named(NamedKey::ArrowDown) => Some(if line { Motion::DocEnd } else { Motion::Down(1) }),
        Key::Named(NamedKey::Home) => Some(if cmd { Motion::DocStart } else { Motion::Home }),
        Key::Named(NamedKey::End) => Some(if cmd { Motion::DocEnd } else { Motion::LineEnd }),
        Key::Named(NamedKey::PageUp) => Some(Motion::Up(page)),
        Key::Named(NamedKey::PageDown) => Some(Motion::Down(page)),
        _ => None,
    }
}

impl App {
    fn text_mods(&self) -> Mods {
        let m = self.modifiers;
        let mac = cfg!(target_os = "macos");
        Mods {
            cmd: m.super_key() || m.control_key(),
            shift: m.shift_key(),
            line: mac && m.super_key(),
            word: if mac { m.alt_key() } else { m.control_key() },
        }
    }

    pub fn key(&mut self, event: &KeyEvent, event_loop: &ActiveEventLoop) {
        let cmd = self.modifiers.super_key() || self.modifiers.control_key();
        let key = event.logical_key.as_ref();
        // Composed text (CJK input, some dead keys) comes as winit Ime events.
        let typed = event.text.as_deref().filter(|t| t.chars().all(|c| !c.is_control())).map(str::to_string);
        let typed = typed.as_deref();
        self.hide_tip();

        // An inspector field whose nodes are no longer the ones shown is written first.
        if self.inspector.typing.as_ref().is_some_and(|t| t.owner != self.owner()) {
            self.finish_field(true);
        }
        let focus = self.focus();
        let taken = match focus {
            // Esc or Enter closes the About dialog; it takes every key.
            Focus::Dialog if self.learn.gallery.is_some() && self.chrome.about.is_none() => {
                self.gallery_key(&key);
                true
            }
            Focus::Dialog if self.chrome.about.is_some() => {
                if matches!(key, Key::Named(NamedKey::Escape | NamedKey::Enter)) {
                    self.chrome.about = None;
                }
                true
            }
            Focus::Dialog => {
                match report_key(&key, cmd) {
                    Some(ReportKey::Adopt) => self.confirm_adopt(),
                    Some(ReportKey::Cancel) => self.cancel_adopt(),
                    Some(ReportKey::Scroll(n)) => self.scroll_report(n),
                    None => {}
                }
                true
            }
            Focus::Menu => {
                self.menu_key(&key);
                true
            }
            // The palette takes every key while open.
            Focus::Palette => {
                self.palette_key(&key, typed);
                true
            }
            Focus::Inspector => self.field_key(&key, typed),
            Focus::Pane => self.pane_key(&key, typed),
            Focus::Sidebar => self.sidebar_key(&key),
            Focus::Canvas if self.interaction.editing.is_some() => self.label_key(&key, typed),
            Focus::Canvas => false,
        };
        if taken {
            return self.redraw();
        }
        // ⌥⌘O by the key's place: ⌥ changes the character it types on macOS.
        if cmd && self.modifiers.alt_key() && event.physical_key == PhysicalKey::Code(KeyCode::KeyO) {
            self.run(Action::OpenFolder);
            return;
        }
        if let (Key::Character(c), true) = (&key, cmd) {
            let open = Open {
                file: self.session.path.is_some(),
                folder: self.session.folder.is_some(),
                node: !self.interaction.nodes().is_empty(),
            };
            if let Some(s) = shortcut(c.to_lowercase().as_str(), self.modifiers.shift_key(), open) {
                self.run_shortcut(s);
                self.redraw();
                return;
            }
        }
        if focus != Focus::Canvas {
            return; // the canvas's own keys wait until Esc
        }
        if self.on_welcome() {
            let (n, tiles) = (self.welcome().items().len(), rhizo_lang::starters::STARTERS.len());
            if let Some(k) = welcome_key(&key, self.chrome.welcome_focus, n, tiles) {
                self.run_welcome_key(k);
                self.redraw();
            }
            return;
        }
        // Space held: dragging pans.
        if key == Key::Named(NamedKey::Space) {
            self.space = true;
            return;
        }
        if let Some(k) = canvas_key(&key, cmd) {
            self.run_canvas_key(k, event_loop);
        }
        self.redraw();
    }

    pub fn run_shortcut(&mut self, s: Shortcut) {
        match s {
            Shortcut::Palette => self.show_palette(List::Commands),
            Shortcut::Undo { redo } => self.command(if redo { Action::Redo } else { Action::Undo }),
            Shortcut::Run(a) => self.command(a),
            Shortcut::CloseFolder => {
                self.session.folder = None;
                self.switch_to(None);
            }
            Shortcut::Quit => self.quitting = true,
        }
    }

    fn run_canvas_key(&mut self, k: CanvasKey, event_loop: &ActiveEventLoop) {
        match k {
            CanvasKey::Quit => {
                self.flush();
                event_loop.exit();
            }
            CanvasKey::Escape => self.interaction.escape(),
            CanvasKey::Add { child } => self.add(child),
            CanvasKey::Rename => self.start_rename(),
            CanvasKey::Delete => self.delete_selected(),
            CanvasKey::Navigate(dir) => self.navigate(dir),
            CanvasKey::Run(a) => self.run(a),
        }
    }

    fn run_welcome_key(&mut self, k: WelcomeKey) {
        match k {
            WelcomeKey::Focus(i) => self.chrome.welcome_focus = Some(i),
            WelcomeKey::Choose => {
                match self.chrome.welcome_focus.and_then(|i| self.welcome().items().get(i).cloned()) {
                    Some(item) => self.activate(item),
                    None => self.chrome.welcome_focus = Some(0),
                }
            }
            WelcomeKey::Unfocus => self.chrome.welcome_focus = None,
        }
    }

    /// Keys while the palette is open: move, filter, choose or close.
    fn palette_key(&mut self, key: &Key<&str>, typed: Option<&str>) {
        let selection = self.selection();
        let mods = self.text_mods();
        let Some(p) = &mut self.palette.open else { return };
        let matches = palette::rows(p.list, p.query.text(), selection, &p.entries).len();
        match key {
            Key::Named(NamedKey::Escape) => self.palette.open = None,
            Key::Named(NamedKey::ArrowDown) => p.index = (p.index + 1).min(matches.saturating_sub(1)),
            Key::Named(NamedKey::ArrowUp) => p.index = p.index.saturating_sub(1),
            Key::Named(NamedKey::Enter) => {
                let i = p.index;
                self.choose(i);
            }
            _ => {
                if let Some(e) = text_key(key, typed, mods, 1) {
                    self.text_edit(TextTarget::Query, e);
                }
            }
        }
    }

    /// Keys while an inspector field is typed in: Enter writes it, Tab
    /// writes it and goes to the next field, Esc drops it. False for keys
    /// it leaves to the window (⌘ shortcuts).
    fn field_key(&mut self, key: &Key<&str>, typed: Option<&str>) -> bool {
        match key {
            Key::Named(NamedKey::Escape) => {
                self.finish_field(false);
            }
            Key::Named(NamedKey::Enter) => {
                self.finish_field(true);
            }
            Key::Named(NamedKey::Tab) => self.tab_field(self.modifiers.shift_key()),
            _ => match text_key(key, typed, self.text_mods(), 1) {
                Some(e) => self.text_edit(TextTarget::Field, e),
                None => return false,
            },
        }
        true
    }

    /// Keys while a label is typed on the canvas.
    fn label_key(&mut self, key: &Key<&str>, typed: Option<&str>) -> bool {
        match key {
            Key::Named(NamedKey::Escape) => self.finish_edit(false),
            Key::Named(NamedKey::Enter) => self.finish_edit(true),
            Key::Named(NamedKey::Tab) => {
                // Commit and keep outlining: Tab nests, like in the outline itself.
                self.finish_edit(true);
                self.add(true);
            }
            _ => match text_key(key, typed, self.text_mods(), 1) {
                Some(e) => self.text_edit(TextTarget::Label, e),
                None => return false,
            },
        }
        true
    }

    /// Keys while the pane has focus; false for those it leaves to the window.
    fn pane_key(&mut self, key: &Key<&str>, typed: Option<&str>) -> bool {
        if key == &Key::Named(NamedKey::Escape) {
            let b = &mut self.pane.buffer;
            if b.selection().is_empty() {
                self.focus_on(Focus::Canvas);
            } else {
                b.set_caret(b.head, false);
            }
            return true;
        }
        let page = (self.pane_rect().height() / self.metrics().line).max(2.0) as usize - 1;
        match text_key(key, typed, self.text_mods(), page) {
            Some(e) => self.text_edit(TextTarget::Pane, e),
            None => return false,
        }
        true
    }

    /// Keys while the sidebar has focus: arrows mark a row, Enter opens
    /// it, left and right fold a file, Esc hands the keys to the canvas.
    fn sidebar_key(&mut self, key: &Key<&str>) -> bool {
        let rows = self.workspace.model.rows(&self.workspace.side.expanded);
        let at = self.workspace.hover.and_then(|h| rows.iter().position(|r| *r == h));
        match side_key(key, &rows, at) {
            Some(SideKey::Mark(r)) => self.workspace.hover = Some(r),
            Some(SideKey::Activate(r)) => self.side_activate(r),
            Some(SideKey::Fold(Row::File(f), open)) => {
                if let Some(rel) = self.workspace.model.files.get(f).map(|f| f.rel.clone()) {
                    if open {
                        self.workspace.side.expanded.insert(rel);
                    } else {
                        self.workspace.side.expanded.remove(&rel);
                    }
                }
            }
            Some(SideKey::Fold(..)) => {}
            Some(SideKey::Leave) => self.focus_on(Focus::Canvas),
            None => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTHING: Open = Open { file: false, folder: false, node: false };
    const FILE: Open = Open { file: true, folder: false, node: false };
    const FOLDER: Open = Open { file: false, folder: true, node: false };

    #[test]
    fn the_adopt_report_takes_the_keys() {
        let named = |k| Key::Named(k);
        assert_eq!(report_key(&named(NamedKey::Escape), false), Some(ReportKey::Cancel));
        assert_eq!(report_key(&named(NamedKey::Enter), true), Some(ReportKey::Adopt));
        assert_eq!(report_key(&named(NamedKey::Enter), false), None, "plain Enter does not write");
        assert_eq!(report_key(&named(NamedKey::ArrowDown), false), Some(ReportKey::Scroll(1)));
        assert_eq!(report_key(&named(NamedKey::PageUp), false), Some(ReportKey::Scroll(-20)));
        assert_eq!(report_key(&Key::Character("a"), true), None);
    }

    #[test]
    fn command_shortcuts() {
        let run = |a| Some(Shortcut::Run(a));
        assert_eq!(shortcut("k", true, NOTHING), Some(Shortcut::Palette));
        assert_eq!(shortcut("z", false, FILE), Some(Shortcut::Undo { redo: false }));
        assert_eq!(shortcut("z", true, FILE), Some(Shortcut::Undo { redo: true }));
        assert_eq!(shortcut("n", false, NOTHING), run(Action::NewMenu));
        assert_eq!(shortcut("n", true, NOTHING), None);
        assert_eq!(shortcut("o", false, NOTHING), run(Action::Open));
        assert_eq!(shortcut("o", true, NOTHING), run(Action::RecentMenu));
        assert_eq!(shortcut("s", true, FILE), run(Action::SaveAs));
        assert_eq!(shortcut("s", false, FILE), run(Action::SaveText));
        assert_eq!(shortcut("e", false, NOTHING), run(Action::Export));
        assert_eq!(shortcut("[", false, NOTHING), run(Action::Back));
        assert_eq!(shortcut("x", false, FILE), None);
    }

    #[test]
    fn shortcuts_that_need_something_open() {
        let run = |a| Some(Shortcut::Run(a));
        assert_eq!(shortcut("\\", false, NOTHING), None);
        assert_eq!(shortcut("\\", false, FOLDER), None);
        assert_eq!(shortcut("\\", false, FILE), run(Action::TogglePane));
        assert_eq!(shortcut("r", false, FOLDER), None);
        assert_eq!(shortcut("r", false, FILE), run(Action::InsertReferenceMenu));
        assert_eq!(shortcut("i", false, FILE), run(Action::ToggleInspector));
        assert_eq!(shortcut("i", false, FOLDER), None);
        assert_eq!(shortcut("i", true, FILE), None);
        for open in [FILE, FOLDER] {
            assert_eq!(shortcut("p", false, open), run(Action::GoToFile));
            assert_eq!(shortcut("1", false, open), run(Action::ToggleSidebar));
            assert_eq!(shortcut("m", true, open), run(Action::ProblemsMenu));
        }
        assert_eq!(shortcut("p", false, NOTHING), None);
        assert_eq!(shortcut("m", false, FILE), None);
        assert_eq!(shortcut("u", true, FILE), None);
        assert_eq!(shortcut("u", true, Open { node: true, ..FILE }), run(Action::FindUsages));
        assert_eq!(shortcut("g", false, FILE), None, "grouping needs nodes");
        assert_eq!(shortcut("g", false, Open { node: true, ..FILE }), run(Action::GroupFrame));
    }

    #[test]
    fn cmd_w_closes_the_file_then_the_folder_then_the_window() {
        assert_eq!(shortcut("w", false, Open { folder: true, ..FILE }), Some(Shortcut::Run(Action::Close)));
        assert_eq!(shortcut("w", false, FOLDER), Some(Shortcut::CloseFolder));
        assert_eq!(shortcut("w", false, NOTHING), Some(Shortcut::Quit));
        assert_eq!(shortcut("w", true, FILE), None);
    }

    #[test]
    fn canvas_keys() {
        let named = |k| Key::Named(k);
        assert_eq!(canvas_key(&named(NamedKey::Escape), true), Some(CanvasKey::Quit));
        assert_eq!(canvas_key(&named(NamedKey::Escape), false), Some(CanvasKey::Escape));
        assert_eq!(canvas_key(&named(NamedKey::Tab), false), Some(CanvasKey::Add { child: true }));
        assert_eq!(canvas_key(&named(NamedKey::Enter), false), Some(CanvasKey::Add { child: false }));
        assert_eq!(canvas_key(&named(NamedKey::F2), false), Some(CanvasKey::Rename));
        assert_eq!(canvas_key(&named(NamedKey::Delete), false), Some(CanvasKey::Delete));
        assert_eq!(canvas_key(&named(NamedKey::Backspace), false), Some(CanvasKey::Delete));
        assert_eq!(canvas_key(&named(NamedKey::ArrowUp), false), Some(CanvasKey::Navigate(Dir::Up)));
        assert_eq!(canvas_key(&Key::Character("F"), false), Some(CanvasKey::Run(Action::Fit)));
        assert_eq!(canvas_key(&Key::Character("0"), false), Some(CanvasKey::Run(Action::ZoomActual)));
        assert_eq!(canvas_key(&Key::Character("+"), false), Some(CanvasKey::Run(Action::ZoomIn)));
        assert_eq!(canvas_key(&Key::Character("-"), false), Some(CanvasKey::Run(Action::ZoomOut)));
        assert_eq!(canvas_key(&Key::Character("q"), false), None);
    }

    #[test]
    fn welcome_keys_move_across_tiles_and_down_rows() {
        let key = |k| Key::Named(k);
        // Three tiles, then two rows.
        let step = |k, focus| welcome_key(&key(k), focus, 5, 3);
        assert_eq!(step(NamedKey::ArrowRight, None), Some(WelcomeKey::Focus(0)));
        assert_eq!(step(NamedKey::Tab, Some(1)), Some(WelcomeKey::Focus(2)));
        assert_eq!(step(NamedKey::ArrowRight, Some(4)), Some(WelcomeKey::Focus(4)));
        assert_eq!(step(NamedKey::ArrowLeft, Some(0)), Some(WelcomeKey::Focus(0)));
        assert_eq!(step(NamedKey::ArrowDown, Some(1)), Some(WelcomeKey::Focus(3)), "tiles go down to the rows");
        assert_eq!(step(NamedKey::ArrowDown, Some(3)), Some(WelcomeKey::Focus(4)));
        assert_eq!(step(NamedKey::ArrowUp, Some(3)), Some(WelcomeKey::Focus(0)), "the first row goes up to the tiles");
        assert_eq!(step(NamedKey::ArrowUp, Some(4)), Some(WelcomeKey::Focus(3)));
        assert_eq!(step(NamedKey::ArrowUp, Some(2)), Some(WelcomeKey::Focus(2)));
        assert_eq!(step(NamedKey::Space, None), Some(WelcomeKey::Choose));
        assert_eq!(step(NamedKey::Escape, Some(2)), Some(WelcomeKey::Unfocus));
        assert_eq!(welcome_key(&Key::Character("a"), None, 5, 3), None);
    }

    #[test]
    fn every_command_chord_reaches_its_command() {
        let all = Open { file: true, folder: true, node: true };
        for c in palette::COMMANDS {
            let Some((shift, alt, key)) = palette::chord(c.keys) else { continue };
            if alt {
                continue; // ⌥⌘O is matched by the key's place
            }
            let want = match c.action {
                Action::Palette => Shortcut::Palette,
                Action::Undo => Shortcut::Undo { redo: false },
                Action::Redo => Shortcut::Undo { redo: true },
                a => Shortcut::Run(a),
            };
            assert_eq!(shortcut(&key, shift, all), Some(want), "{}", c.keys);
        }
        assert_eq!(shortcut("c", false, Open { node: true, ..FILE }), Some(Shortcut::Run(Action::Copy)));
        assert_eq!(shortcut("c", false, FILE), None, "copy needs a node (texts copy before shortcuts)");
        assert_eq!(shortcut("v", false, FILE), Some(Shortcut::Run(Action::Paste)));
        assert_eq!(shortcut("a", false, FILE), Some(Shortcut::Run(Action::SelectAll)));
        assert_eq!(shortcut("q", false, NOTHING), Some(Shortcut::Run(Action::Quit)));
        assert_eq!(shortcut("/", false, NOTHING), Some(Shortcut::Run(Action::Shortcuts)));
    }

    #[test]
    fn text_keys_are_the_same_in_every_text() {
        let mac = Mods { cmd: false, shift: false, line: false, word: false };
        let named = |k| Key::Named(k);
        assert_eq!(text_key(&named(NamedKey::ArrowLeft), None, mac, 1), Some(Edit::Move(Motion::Left, false)));
        let shift_word = Mods { shift: true, word: true, ..mac };
        assert_eq!(
            text_key(&named(NamedKey::ArrowRight), None, shift_word, 1),
            Some(Edit::Move(Motion::WordRight, true))
        );
        let cmd = Mods { cmd: true, line: true, ..mac };
        assert_eq!(text_key(&named(NamedKey::ArrowLeft), None, cmd, 1), Some(Edit::Move(Motion::Home, false)));
        assert_eq!(text_key(&named(NamedKey::Backspace), None, cmd, 1), Some(Edit::Delete(Motion::LineStart)));
        assert_eq!(
            text_key(&named(NamedKey::Backspace), None, Mods { word: true, ..mac }, 1),
            Some(Edit::Delete(Motion::WordLeft))
        );
        assert_eq!(text_key(&named(NamedKey::Delete), None, mac, 1), Some(Edit::Delete(Motion::Right)));
        for (c, e) in
            [("a", Edit::SelectAll), ("c", Edit::Copy), ("x", Edit::Cut), ("v", Edit::Paste), ("z", Edit::Undo)]
        {
            assert_eq!(text_key(&Key::Character(c), Some(c), cmd, 1), Some(e));
        }
        assert_eq!(text_key(&Key::Character("Z"), None, Mods { shift: true, ..cmd }, 1), Some(Edit::Redo));
        assert_eq!(text_key(&Key::Character("k"), None, cmd, 1), None, "left to the shortcuts");
        assert_eq!(text_key(&Key::Character("é"), Some("é"), mac, 1), Some(Edit::Type("é".into())));
        assert_eq!(
            text_key(&named(NamedKey::Tab), None, Mods { shift: true, ..mac }, 1),
            Some(Edit::Indent { outdent: true })
        );
        assert_eq!(text_key(&named(NamedKey::F5), None, mac, 1), None);
    }

    #[test]
    fn the_sidebar_walks_its_rows() {
        let rows = [Row::File(0), Row::Group(0, rhizo_app::project::Kind::Entity), Row::Thing(4), Row::File(1)];
        let key = |k| Key::Named(k);
        assert_eq!(side_key(&key(NamedKey::ArrowDown), &rows, None), Some(SideKey::Mark(Row::File(0))));
        assert_eq!(side_key(&key(NamedKey::ArrowDown), &rows, Some(1)), Some(SideKey::Mark(Row::Thing(4))));
        assert_eq!(side_key(&key(NamedKey::ArrowDown), &rows, Some(3)), Some(SideKey::Mark(Row::File(1))));
        assert_eq!(side_key(&key(NamedKey::ArrowUp), &rows, Some(0)), Some(SideKey::Mark(Row::File(0))));
        assert_eq!(side_key(&key(NamedKey::Enter), &rows, Some(2)), Some(SideKey::Activate(Row::Thing(4))));
        assert_eq!(side_key(&key(NamedKey::Enter), &rows, None), None);
        assert_eq!(side_key(&key(NamedKey::ArrowLeft), &rows, Some(0)), Some(SideKey::Fold(Row::File(0), false)));
        assert_eq!(side_key(&key(NamedKey::Escape), &rows, None), Some(SideKey::Leave));
        assert_eq!(side_key(&key(NamedKey::ArrowDown), &[], None), None);
    }

    #[test]
    fn caret_motions() {
        let key = |k| Key::Named(k);
        assert_eq!(motion(&key(NamedKey::ArrowLeft), false, false, false, 9), Some(Motion::Left));
        assert_eq!(motion(&key(NamedKey::ArrowLeft), false, true, false, 9), Some(Motion::WordLeft));
        assert_eq!(motion(&key(NamedKey::ArrowLeft), true, true, true, 9), Some(Motion::Home));
        assert_eq!(motion(&key(NamedKey::ArrowRight), true, false, true, 9), Some(Motion::LineEnd));
        assert_eq!(motion(&key(NamedKey::ArrowUp), true, false, true, 9), Some(Motion::DocStart));
        assert_eq!(motion(&key(NamedKey::ArrowDown), false, false, false, 9), Some(Motion::Down(1)));
        assert_eq!(motion(&key(NamedKey::End), false, false, true, 9), Some(Motion::DocEnd));
        assert_eq!(motion(&key(NamedKey::Home), false, false, false, 9), Some(Motion::Home));
        assert_eq!(motion(&key(NamedKey::PageUp), false, false, false, 9), Some(Motion::Up(9)));
        assert_eq!(motion(&key(NamedKey::Backspace), false, false, false, 9), None);
    }
}
