//! The macOS menu bar, built with `muda` from the command registry: each
//! item's name and ⌘ shortcut are the registry's, so the palette, the
//! menus and the key table agree. Items enable and disable with what is
//! open and selected; Undo and Redo say what they would take back.
//!
//! macOS hands a ⌘ key that a menu item has to that item, not to the
//! window, so those keys arrive here as menu events and go through
//! [`App::command`], which sends text commands (copy, paste, undo…) to the
//! text that has the keys. Shortcuts without ⌘ (Tab, F2, ⌫, F…) are not
//! bound in the menus, so they keep reaching text fields.

use std::sync::Mutex;

use muda::accelerator::Accelerator;
use muda::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use rhizo_app::palette::{self, Action, Arrange};
use winit::event_loop::EventLoopProxy;

use super::App;
use super::Event;
use super::focus::TextTarget;
use super::keys::{self, Open};

/// A line of a menu: a command, a command under another name, or a separator.
#[derive(Clone, Copy)]
enum Entry {
    Run(Action),
    Named(&'static str, Action),
    Sep,
}

use Entry::{Named, Run, Sep};

/// The menus after the app menu, in order.
const MENUS: &[(&str, &[Entry])] = &[
    (
        "File",
        &[
            Named("New from Template…", Action::NewMenu),
            Named("Open…", Action::Open),
            Named("Open Folder…", Action::OpenFolder),
            Named("Open Recent…", Action::RecentMenu),
            Named("Open Sample…", Action::Gallery),
            Named("Go to File…", Action::GoToFile),
            Sep,
            Named("Save", Action::SaveText),
            Named("Save As…", Action::SaveAs),
            Named("Export…", Action::Export),
            // Opening Mermaid, Markdown, OPML… imports them into a new .rhizo file.
            Named("Import…", Action::Open),
            Sep,
            Run(Action::Reveal),
            Named("Open in Text Editor", Action::OpenInEditor),
            Named("Revert Text to File on Disk", Action::RevertText),
            Sep,
            Named("Close", Action::Close),
        ],
    ),
    (
        "Edit",
        &[
            Named("Undo", Action::Undo),
            Named("Redo", Action::Redo),
            Sep,
            Run(Action::Cut),
            Run(Action::Copy),
            Run(Action::Paste),
            Run(Action::Delete),
            Named("Select All", Action::SelectAll),
            Sep,
            Named("Find Node…", Action::FindNode),
            Named("Rename", Action::Rename),
        ],
    ),
    (
        "View",
        &[
            Named("Zoom In", Action::ZoomIn),
            Named("Zoom Out", Action::ZoomOut),
            Named("Actual Size", Action::ZoomActual),
            Named("Fit to Window", Action::Fit),
            Sep,
            Named("Sidebar", Action::ToggleSidebar),
            Named("Text Pane", Action::TogglePane),
            Named("Inspector", Action::ToggleInspector),
            Named("Problems…", Action::ProblemsMenu),
            Sep,
            Named("Light Theme", Action::Theme("light")),
            Named("Dark Theme", Action::Theme("dark")),
            Named("Paper Theme", Action::Theme("paper")),
            Sep,
            Named("Tidy Layout", Action::TidyLayout),
            Named("Status-Bar Hints", Action::ToggleHints),
        ],
    ),
    (
        "Insert",
        &[
            Named("Child Node", Action::AddChild),
            Named("Sibling Node", Action::AddSibling),
            Named("Typed Node…", Action::AddTypedMenu),
            Named("Reference…", Action::InsertReferenceMenu),
            Named("Acceptance Criterion", Action::AddAccept),
            Sep,
            Named("Frame", Action::NewFrame),
            Named("Group into Frame", Action::GroupFrame),
            Sep,
            Named("Connect to…", Action::Connect),
        ],
    ),
    (
        "Format",
        &[
            Named("Shape…", Action::ShapeMenu),
            Named("Fill Colour…", Action::FillMenu),
            Named("Stroke Colour…", Action::StrokeMenu),
            Sep,
            Named("Edge Kind…", Action::EdgeKindMenu),
            Named("Line Style…", Action::LineMenu),
            Named("Arrowheads…", Action::ArrowMenu),
            Sep,
            Named("Layout…", Action::LayoutMenu),
            Named("Direction…", Action::DirectionMenu),
        ],
    ),
    (
        "Diagram",
        &[
            Named("Compile Specs…", Action::Compile),
            Named("Build Brief", Action::Brief),
            Named("Check (Problems in the Project)", Action::ProblemsMenu),
            Sep,
            Named("Adopt Node…", Action::AdoptNode),
            Named("Adopt All That Switch Without Changes", Action::AdoptAll),
            Named("Cancel Adopt", Action::CancelAdopt),
            Sep,
            Named("Change Type…", Action::ChangeTypeMenu),
            Named("Set Frame Type…", Action::FrameTypeMenu),
            Named("Ungroup Frame", Action::Ungroup),
            Sep,
            Named("Document Title…", Action::EditTitle),
            Named("Document Theme…", Action::DocThemeMenu),
            Named("Kits in Use…", Action::KitsMenu),
            Named("Remove Unused Imports", Action::RemoveUnusedImports),
            Sep,
            Named("Go to Definition", Action::GoToDefinition),
            Named("Find Usages", Action::FindUsages),
            Named("Back", Action::Back),
            Sep,
            Named("Align Left Edges", Action::Arrange(Arrange::Left)),
            Named("Align Centres", Action::Arrange(Arrange::Center)),
            Named("Align Right Edges", Action::Arrange(Arrange::Right)),
            Named("Align Top Edges", Action::Arrange(Arrange::Top)),
            Named("Align Middles", Action::Arrange(Arrange::Middle)),
            Named("Align Bottom Edges", Action::Arrange(Arrange::Bottom)),
            Named("Distribute Horizontally", Action::Arrange(Arrange::SpreadX)),
            Named("Distribute Vertically", Action::Arrange(Arrange::SpreadY)),
        ],
    ),
];

const HELP: &[Entry] = &[
    Named("Learn rhizo", Action::Learn),
    Named("Keyboard Shortcuts", Action::Shortcuts),
    Named("Command Palette", Action::Palette),
    Named("Guide to Writing rhizo", Action::Guide),
];

/// muda's form of the registry's ⌘ chord in `keys` (`⇧⌘Z` → `CmdOrCtrl+Shift+Z`).
fn accelerator(keys: &str) -> Option<Accelerator> {
    let (shift, alt, key) = palette::chord(keys)?;
    let mods = format!("CmdOrCtrl{}{}", if shift { "+Shift" } else { "" }, if alt { "+Alt" } else { "" });
    format!("{mods}+{}", key.to_uppercase()).parse().ok()
}

pub struct MenuBar {
    /// Kept alive: the app's main menu.
    _menu: Menu,
    items: Vec<(Action, &'static str, MenuItem)>,
    /// What each item was last set to: enabled, and its text.
    shown: Vec<(bool, String)>,
}

impl MenuBar {
    /// The menu bar, made the app's, with its events sent to the window.
    pub fn new(proxy: EventLoopProxy<Event>) -> MenuBar {
        let proxy = Mutex::new(proxy);
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.lock().map(|p| p.send_event(Event::Menu(e.id)));
        }));
        let mut items = Vec::new();
        let mut item = |entry: Entry| -> Box<dyn muda::IsMenuItem> {
            let (name, action) = match entry {
                Run(a) => (palette::command(a).map_or("", |c| c.name), a),
                Named(n, a) => (n, a),
                Sep => return Box::new(PredefinedMenuItem::separator()),
            };
            let keys = palette::command(action).map_or("", |c| c.keys);
            let accel = if entry_binds(name) { accelerator(keys) } else { None };
            let it = MenuItem::new(name, true, accel);
            items.push((action, name, it.clone()));
            Box::new(it)
        };
        let submenu = |title: &str, entries: Vec<Box<dyn muda::IsMenuItem>>| {
            let refs: Vec<&dyn muda::IsMenuItem> = entries.iter().map(|b| b.as_ref()).collect();
            Submenu::with_items(title, true, &refs).expect("menu")
        };
        // Drawn in the window, not muda's native panel: that cannot show the Franconian rake.
        let about = item(Named("About rhizo", Action::About));
        let quit = item(Named("Quit rhizo", Action::Quit));
        let app_menu = submenu(
            "rhizo",
            vec![
                about,
                Box::new(PredefinedMenuItem::separator()),
                Box::new(PredefinedMenuItem::services(None)),
                Box::new(PredefinedMenuItem::separator()),
                Box::new(PredefinedMenuItem::hide(None)),
                Box::new(PredefinedMenuItem::hide_others(None)),
                Box::new(PredefinedMenuItem::show_all(None)),
                Box::new(PredefinedMenuItem::separator()),
                quit,
            ],
        );
        let menus: Vec<Submenu> =
            MENUS.iter().map(|(title, entries)| submenu(title, entries.iter().map(|e| item(*e)).collect())).collect();
        let window = submenu(
            "Window",
            vec![
                Box::new(PredefinedMenuItem::minimize(None)),
                Box::new(PredefinedMenuItem::maximize(Some("Zoom"))),
                Box::new(PredefinedMenuItem::fullscreen(None)),
                Box::new(PredefinedMenuItem::separator()),
                Box::new(PredefinedMenuItem::bring_all_to_front(None)),
            ],
        );
        let help = submenu("Help", HELP.iter().map(|e| item(*e)).collect());
        let menu = Menu::new();
        menu.append(&app_menu).expect("menu");
        for m in &menus {
            menu.append(m).expect("menu");
        }
        menu.append(&window).expect("menu");
        menu.append(&help).expect("menu");
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
        help.set_as_help_menu_for_nsapp();
        let shown = vec![(true, String::new()); items.len()];
        MenuBar { _menu: menu, items, shown }
    }

    /// The action of a chosen item.
    pub fn action(&self, id: &MenuId) -> Option<Action> {
        self.items.iter().find(|(_, _, it)| it.id() == id).map(|(a, _, _)| *a)
    }
}

/// An action listed twice (Problems… and Check, Open… and Import…) binds
/// its keys once.
fn entry_binds(name: &str) -> bool {
    !matches!(name, "Import…" | "Check (Problems in the Project)")
}

impl App {
    /// Whether the menu bar offers `a` now.
    fn menu_enabled(&self, a: Action) -> bool {
        let text = self.focused_text();
        match a {
            Action::Cut | Action::Copy | Action::Paste | Action::SelectAll if text.is_some() => true,
            Action::Undo | Action::Redo => self.undo_name(a == Action::Redo).is_some(),
            Action::Close | Action::Quit | Action::Palette | Action::Theme(_) => true,
            _ => palette::COMMANDS.iter().any(|c| c.action == a && c.needs.met_by(self.selection())),
        }
    }

    /// What Undo (or Redo) would do, for its menu item: the focused
    /// field's typing, or the file's last step.
    fn undo_name(&self, redo: bool) -> Option<&'static str> {
        let field = match self.focused_text() {
            Some(TextTarget::Field) => self.inspector.typing.as_ref().map(|t| &t.field),
            Some(TextTarget::Query) => self.palette.open.as_ref().map(|p| &p.query),
            Some(TextTarget::Label) => self.interaction.editing.as_ref().map(|e| &e.field),
            _ => None,
        };
        let b = field.map_or(&self.pane.buffer, |f| &f.buffer);
        let name = if redo { b.redo_name() } else { b.undo_name() };
        name.filter(|_| field.is_some() || self.editor.is_some())
    }

    /// Brings the menu bar up to date: enabled items, Undo's name.
    pub fn refresh_menubar(&mut self) {
        let Some(bar) = &self.menubar else { return };
        let wanted: Vec<(bool, String)> = bar
            .items
            .iter()
            .map(|(a, name, _)| {
                let text = match a {
                    Action::Undo | Action::Redo => match self.undo_name(*a == Action::Redo) {
                        Some(what) => format!("{name} {what}"),
                        None => name.to_string(),
                    },
                    _ => name.to_string(),
                };
                (self.menu_enabled(*a), text)
            })
            .collect();
        let Some(bar) = &mut self.menubar else { return };
        for (i, (want, (_, _, item))) in wanted.into_iter().zip(&bar.items).enumerate() {
            if bar.shown[i] != want {
                item.set_enabled(want.0);
                item.set_text(&want.1);
                bar.shown[i] = want;
            }
        }
    }

    /// A menu item was chosen. Close does what ⌘W does: the file, else the
    /// folder, else the window.
    pub fn menu_chosen(&mut self, id: &MenuId) {
        let Some(a) = self.menubar.as_ref().and_then(|m| m.action(id)) else { return };
        if a == Action::Close && self.focused_text().is_none() {
            let open = Open { file: self.session.path.is_some(), folder: self.session.folder.is_some(), node: false };
            if let Some(s) = keys::shortcut("w", false, open) {
                self.run_shortcut(s);
            }
        } else {
            self.command(a);
        }
        self.redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_items_take_their_names_and_keys_from_the_registry() {
        for (_, entries) in MENUS.iter().chain(&[("Help", HELP)]) {
            for e in entries.iter() {
                if let Run(a) | Named(_, a) = e {
                    assert!(palette::command(*a).is_some(), "{a:?} is a command");
                }
            }
        }
        assert!(accelerator("⇧⌘Z").is_some());
        assert!(accelerator("⌘\\").is_some());
        assert!(accelerator("⌥⌘O").is_some());
        assert!(accelerator("⌘[").is_some());
        assert!(accelerator("⌘/").is_some());
        assert!(accelerator("F2").is_none(), "keys without ⌘ stay with the window");
        // Every ⌘ chord of the registry can be a menu key.
        for c in palette::COMMANDS.iter().filter(|c| palette::chord(c.keys).is_some()) {
            assert!(accelerator(c.keys).is_some(), "{}", c.keys);
        }
    }
}
