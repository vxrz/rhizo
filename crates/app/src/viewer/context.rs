//! Context menus: a right-click (or ctrl-click on macOS) on a node, an
//! edge, the canvas, a sidebar row or the text pane lists what can be done
//! there, from the command registry, with each command's keys.
//!
//! Drawn in the window like the palette rather than as native menus: a
//! native one runs its own modal loop inside a winit event (so the window
//! stops drawing and its events queue up behind it), cannot be rendered by
//! the offscreen tests, and would need GTK on Linux. A drawn one also takes
//! the same keys as the palette.

use rhizo_app::context::{self, Item, Place, items};
use rhizo_app::files;
use rhizo_app::palette::{self, MenuRow};
use rhizo_app::project::Spot;
use rhizo_app::sidebar::Row;
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::{Rect, Vec2};
use rhizo_core::NodeId;
use winit::keyboard::{Key, NamedKey};

use super::App;
use super::focus::Focus;
use super::interaction::Target;

pub struct ContextMenu {
    pub at: Vec2,
    pub items: Vec<Item>,
    pub hover: Option<usize>,
    /// Where the enabled rows were drawn, with their index.
    pub rects: Vec<(Rect, usize)>,
}

impl App {
    /// What the pointer is on, for a right-click; selects the node or edge
    /// there unless it is selected already.
    fn place_here(&mut self) -> Option<Place> {
        let p = self.cursor.to_point();
        if let Some(l) = self.side_layout().filter(|l| l.rect.contains(p)) {
            let path = |rel: &str| self.workspace.scan.as_ref().map(|s| s.path(rel));
            return match l.row_at(p)? {
                Row::File(f) => Some(Place::File(path(&self.workspace.model.files.get(f)?.rel.clone())?)),
                Row::Thing(i) => Some(Place::Thing(i)),
                Row::Group(..) => None,
            };
        }
        if self.pane_rect().contains(p) {
            self.focus_on(Focus::Pane);
            return Some(Place::Pane);
        }
        if self.inspector_rect().contains(p) || !self.canvas().contains(p) || self.built.is_none() {
            return None;
        }
        self.focus_on(Focus::Canvas);
        self.interaction.menu_point = self.to_diagram(self.cursor);
        let target = self
            .node_at_cursor()
            .map(Target::Node)
            .or_else(|| self.edge_at_cursor().map(Target::Edge))
            .or_else(|| self.frame_at_cursor().map(Target::Frame));
        match target {
            Some(Target::Node(id)) => {
                if !self.interaction.nodes().contains(&id) {
                    self.interaction.select_nodes(vec![id.clone()]);
                }
                let reference = self.built.as_ref().is_some_and(|b| b.references.contains_key(&id));
                let code = self
                    .built
                    .as_ref()
                    .and_then(|b| b.document.nodes.get(&id))
                    .is_some_and(|n| n.props.contains_key("origin") && !rhizo_kit::infra::adopt::adopted(&n.props));
                Some(Place::Node { reference, many: self.interaction.nodes().len() > 1, code })
            }
            Some(Target::Frame(f)) => {
                self.interaction.select_nodes(Vec::new());
                self.interaction.selected = Some(Target::Frame(f));
                Some(Place::Frame)
            }
            Some(t) => {
                self.interaction.select_nodes(Vec::new());
                self.interaction.selected = Some(t);
                Some(Place::Edge)
            }
            None => {
                self.interaction.select_nodes(Vec::new());
                Some(Place::Canvas)
            }
        }
    }

    /// A right-click: the menu for what is under the pointer.
    pub fn context_menu(&mut self) {
        self.finish_edit(true);
        self.palette.open = None;
        self.hide_tip();
        self.menu = self.place_here().map(|p| ContextMenu {
            at: self.cursor,
            items: items(&p),
            hover: None,
            rects: Vec::new(),
        });
        self.redraw();
    }

    /// How an item reads and whether it can run now.
    fn menu_row(&self, item: &Item) -> MenuRow {
        let row = |name: &str, enabled| MenuRow { name: name.into(), keys: String::new(), enabled };
        let thing = |i: &usize| self.workspace.model.things.get(*i);
        match item {
            Item::Separator => row("", false),
            // In the text pane, the text commands work on its text.
            Item::Run(a) => {
                let mut r = context::row(*a, self.selection());
                r.enabled |= self.focus() == Focus::Pane;
                r
            }
            Item::Open(p) => row(&format!("Open {}", files::name(p)), Some(p) != self.session.path.as_ref()),
            Item::Reveal(_) => row(palette::REVEAL, true),
            Item::GoTo(i) => row("Go to it", thing(i).is_some()),
            Item::Refer(i) => {
                let here = self.current_rel();
                let ok = self.editor.is_some() && thing(i).is_some_and(|t| !t.frame && Some(&t.rel) != here.as_ref());
                row("Insert a reference to it", ok)
            }
        }
    }

    fn run_item(&mut self, item: Item) {
        self.menu = None;
        match item {
            Item::Run(a) => {
                self.command(a);
                self.interaction.menu_point = None;
            }
            Item::Open(p) => self.go(p, Spot::File),
            Item::Reveal(p) => {
                if let Err(e) = files::reveal(&p) {
                    self.fail(format!("cannot show {}: {e}", p.display()));
                }
            }
            Item::GoTo(i) => {
                if let (Some(t), Some(scan)) = (self.workspace.model.things.get(i).cloned(), &self.workspace.scan) {
                    let spot = if t.frame { Spot::File } else { Spot::Node(NodeId(t.id.clone())) };
                    let path = scan.path(&t.rel);
                    self.go(path, spot);
                }
            }
            Item::Refer(i) => {
                if let Some(t) = self.workspace.model.things.get(i).cloned() {
                    self.refer(&t, None, None);
                }
            }
            Item::Separator => {}
        }
        self.redraw();
    }

    /// Keys while a context menu is open: up and down, Enter runs, Esc closes.
    pub fn menu_key(&mut self, key: &Key<&str>) {
        let rows: Vec<MenuRow> = self.menu.iter().flat_map(|m| m.items.iter()).map(|i| self.menu_row(i)).collect();
        let Some(m) = &mut self.menu else { return };
        let enabled: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].enabled).collect();
        let at = m.hover.and_then(|h| enabled.iter().position(|&i| i == h));
        match key {
            Key::Named(NamedKey::Escape) => self.menu = None,
            Key::Named(NamedKey::ArrowDown) => {
                m.hover = enabled.get(at.map_or(0, |a| (a + 1).min(enabled.len().saturating_sub(1)))).copied()
            }
            Key::Named(NamedKey::ArrowUp) => m.hover = enabled.get(at.map_or(0, |a| a.saturating_sub(1))).copied(),
            Key::Named(NamedKey::Enter | NamedKey::Space) => {
                if let Some(item) = m.hover.and_then(|h| m.items.get(h).cloned()) {
                    self.run_item(item);
                }
            }
            _ => {}
        }
        self.redraw();
    }

    /// A press while a menu is open: a row runs, anywhere else closes it.
    pub fn menu_press(&mut self) {
        let Some(m) = &self.menu else { return };
        let hit = m.rects.iter().find(|(r, _)| r.contains(self.cursor.to_point())).map(|&(_, i)| m.items[i].clone());
        match hit {
            Some(item) => self.run_item(item),
            None => self.menu = None,
        }
        self.redraw();
    }

    /// The row under the pointer is highlighted.
    pub fn menu_hover(&mut self) {
        let Some(m) = &mut self.menu else { return };
        let hover = m.rects.iter().find(|(r, _)| r.contains(self.cursor.to_point())).map(|&(_, i)| i);
        if hover.is_some() && hover != m.hover {
            m.hover = hover;
            self.redraw();
        }
    }

    pub fn menu_scene(&mut self) -> Option<Scene> {
        let m = self.menu.as_ref()?;
        let rows: Vec<MenuRow> = m.items.iter().map(|i| self.menu_row(i)).collect();
        let (at, hover) = (m.at, m.hover);
        let (size, dpr, bg) = (self.size(), self.scale_factor(), self.background());
        self.fonts();
        let fonts = self.fonts.as_ref().expect("loaded above");
        let mut s = Scene::new();
        let rects = palette::draw_menu(&mut s, fonts, &mut self.glyphs, at, size, dpr, bg, &rows, hover);
        if let Some(m) = &mut self.menu {
            m.rects = rects;
        }
        Some(s)
    }
}
