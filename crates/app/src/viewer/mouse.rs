//! The pointer: where a press goes, dragging, scrolling, and hover
//! feedback with the pointer's shape.

use std::time::{Duration, Instant};

use rhizo_app::interact::DragMode;
use rhizo_app::sidebar::{self, Row};
use rhizo_app::vello::kurbo::Vec2;
use winit::event::{ElementState, MouseScrollDelta};
use winit::window::CursorIcon;

use super::App;
use super::focus::Focus;
use super::interaction::{Gesture, NodeDrag, Target};
use super::pane::PaneDrag;

/// The part of the window a left press or release goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Palette,
    Inspector,
    Sidebar,
    Pane,
    Welcome,
    Canvas,
}

/// Where a press goes: the open palette takes them all (a click outside
/// closes it), then the inspector, the sidebar (with its divider), the text
/// pane (with its divider), the welcome screen, and last the canvas.
pub fn press_zone(palette: bool, inspector: bool, sidebar: bool, pane: bool, welcome: bool) -> Zone {
    if palette {
        Zone::Palette
    } else if inspector {
        Zone::Inspector
    } else if sidebar {
        Zone::Sidebar
    } else if pane {
        Zone::Pane
    } else if welcome {
        Zone::Welcome
    } else {
        Zone::Canvas
    }
}

/// Where a release goes: to the sidebar or the pane while a drag that
/// started there runs, else to the canvas.
pub fn release_zone(side_drag: bool, pane_drag: bool) -> Zone {
    if side_drag {
        Zone::Sidebar
    } else if pane_drag {
        Zone::Pane
    } else {
        Zone::Canvas
    }
}

impl App {
    /// The left button went down or up. False when the event is done with,
    /// without the hover and selection updates that usually follow.
    pub fn left_button(&mut self, state: ElementState) -> bool {
        if state == ElementState::Released {
            match release_zone(self.workspace.resizing || self.workspace.drag.is_some(), self.text.drag.is_some()) {
                Zone::Sidebar => self.side_release(),
                Zone::Pane => self.text.drag = None,
                _ => self.release(),
            }
            return true;
        }
        let (dpr, bottom) = (self.scale_factor(), self.view_size().y);
        let (x, y) = (self.cursor.x, self.cursor.y);
        let sidebar = self.sidebar_shown() && x <= self.side_px() + 4.0 * dpr && y < bottom;
        let pane = self.pane_shown() && x <= self.pane_rect().x1 + 4.0 * dpr && y < bottom;
        let inspector = self.inspector_shown() && x >= self.inspector_rect().x0 && y < bottom;
        if self.chrome.about.is_some() {
            self.about_press();
            return true;
        }
        if self.learn.gallery.is_some() {
            self.gallery_press();
            return true;
        }
        // The adopt report is modal: its buttons take the press, the rest of the window waits.
        if self.adopting.job.review().is_some() {
            self.report_press();
            return true;
        }
        self.hide_tip();
        // An open context menu takes the press: a row runs, elsewhere closes it.
        if self.menu.is_some() {
            self.menu_press();
            return true;
        }
        // Ctrl-click is the right click on macOS (elsewhere Ctrl is ⌘).
        if cfg!(target_os = "macos") && self.modifiers.control_key() {
            self.context_menu();
            return true;
        }
        let zone = press_zone(self.palette.open.is_some(), inspector, sidebar, pane, self.on_welcome());
        if zone != Zone::Inspector && zone != Zone::Palette {
            self.finish_field(true);
        }
        match zone {
            Zone::Palette => {
                // A row runs; anywhere else closes the palette.
                match self.palette.rects.iter().find(|(r, _)| r.contains(self.cursor.to_point())) {
                    Some(&(_, i)) => self.choose(i),
                    None => self.palette.open = None,
                }
                self.redraw();
            }
            Zone::Inspector => self.inspector_press(),
            Zone::Sidebar => {
                self.focus_on(Focus::Sidebar);
                self.side_press();
            }
            Zone::Pane => self.pane_press(),
            Zone::Welcome => {
                let item = self.chrome.welcome_hover.and_then(|i| self.welcome().items().get(i).cloned());
                if let Some(item) = item {
                    self.chrome.welcome_focus = None;
                    self.activate(item);
                }
            }
            Zone::Canvas => return self.canvas_press(),
        }
        true
    }

    /// A press on the canvas: ⌘-click on a reference goes to its definition,
    /// a double click renames (or fits, on empty space), else select or drag.
    fn canvas_press(&mut self) -> bool {
        self.focus_on(Focus::Canvas);
        // Space held: the drag pans.
        if self.space {
            self.interaction.drag_from = Some(self.cursor);
            return true;
        }
        if let Some(link) = self.empty_link_at_cursor() {
            self.follow_empty_link(link);
            return true;
        }
        let now = Instant::now();
        let double = self
            .interaction
            .last_click
            .is_some_and(|(t, at)| now - t < Duration::from_millis(350) && (at - self.cursor).hypot() < 6.0);
        self.interaction.last_click = Some((now, self.cursor));
        let target = self
            .node_at_cursor()
            .map(Target::Node)
            .or_else(|| self.edge_at_cursor().map(Target::Edge))
            .or_else(|| self.frame_title_at_cursor().map(Target::Frame));
        let cmd = self.modifiers.super_key() || self.modifiers.control_key();
        if let (true, Some(Target::Node(id))) = (cmd, &target) {
            let is_reference = self.built.as_ref().is_some_and(|b| b.references.contains_key(id));
            if is_reference {
                self.interaction.selected = target.clone();
                self.go_to_definition();
                return false;
            }
        }
        match (double, target) {
            (true, Some(t)) => {
                self.interaction.node_drag = None;
                self.interaction.selected = Some(t);
                self.start_rename();
            }
            // An empty document has nothing to fit: a double click adds its first node.
            (true, None) if self.built.as_ref().is_some_and(|b| b.nodes == 0) => self.add(true),
            (true, None) => self.fit(true),
            (false, _) => self.press(),
        }
        true
    }

    /// The middle button pans the canvas while held.
    pub fn middle_button(&mut self, state: ElementState) {
        let on_canvas = self.canvas().contains(self.cursor.to_point()) && self.built.is_some();
        self.interaction.drag_from = (state == ElementState::Pressed && on_canvas).then_some(self.cursor);
        self.update_pointer();
    }

    /// The pointer moved to `p`: hover in the palette, and whatever drag runs.
    pub fn cursor_moved(&mut self, p: Vec2) {
        self.cursor = p;
        let dpr = self.scale_factor();
        if self.menu.is_some() {
            return self.menu_hover();
        }
        if self.chrome.about.is_some() {
            return self.about_hover();
        }
        if self.learn.gallery.is_some() {
            return self.gallery_hover();
        }
        if let Some((_, to)) = &mut self.interaction.band {
            *to = p;
            return self.redraw();
        }
        if self.adopting.job.review().is_some() {
            let hover = self.adopting.rects.iter().find(|(r, _)| r.contains(p.to_point())).map(|(_, b)| *b);
            if hover != self.adopting.hover {
                self.adopting.hover = hover;
                self.redraw();
            }
            return;
        }
        if let Some(&(_, i)) = self.palette.rects.iter().find(|(r, _)| r.contains(p.to_point()))
            && let Some(pal) = self.palette.open.as_mut().filter(|pal| pal.index != i)
        {
            pal.index = i;
            self.redraw();
        }
        if self.workspace.resizing {
            let width = (p.x / dpr).clamp(sidebar::MIN_WIDTH, sidebar::MAX_WIDTH);
            self.change_layout(|a| a.workspace.side.width = width);
        } else if let Some(d) = &mut self.workspace.drag {
            d.moved |= (p - d.from).hypot() > 4.0 * dpr;
            self.redraw();
        } else if let Some(drag) = self.text.drag {
            self.pane_drag_to(drag);
        } else if let Some(gesture) = self.interaction.node_drag.as_ref().map(|d| d.gesture) {
            // ⌥-dragging onto a thing in the sidebar connects to a reference to it.
            let over_side = p.x < self.side_px() && gesture == Gesture::Connect;
            let drop = if over_side { self.side_thing_at_cursor() } else { None };
            if drop != self.workspace.drop {
                self.workspace.drop = drop;
                self.redraw();
            }
            if let Some(d) = self.to_diagram(p) {
                self.drag_to(d);
            }
        } else if let Some(from) = self.interaction.drag_from {
            self.viewport.drag(p - from);
            self.interaction.drag_from = Some(p);
            self.redraw();
        } else if self.interaction.connecting.is_some() {
            self.redraw();
        }
    }

    /// Scrolling: the sidebar or the pane under the pointer scrolls, else
    /// the canvas pans (or zooms, with ⌘).
    pub fn wheel(&mut self, delta: MouseScrollDelta) {
        let zooming = self.modifiers.super_key() || self.modifiers.control_key();
        let d = match delta {
            MouseScrollDelta::LineDelta(x, y) => Vec2::new(x as f64, y as f64) * 40.0 * self.scale_factor(),
            MouseScrollDelta::PixelDelta(p) => Vec2::new(p.x, p.y),
        };
        if self.adopting.job.review().is_some() {
            let lines = (-d.y / (17.0 * self.scale_factor())).round() as isize;
            self.scroll_report(if lines == 0 { -d.y.signum() as isize } else { lines });
        } else if self.cursor.x < self.side_px() && self.cursor.y < self.view_size().y {
            self.workspace.side.scroll -= d.y;
            self.redraw();
        } else if self.inspector_rect().contains(self.cursor.to_point()) {
            self.scroll_inspector(-d.y);
        } else if self.pane_rect().contains(self.cursor.to_point()) {
            let (m, r) = (self.metrics(), self.pane_rect());
            self.pane.scroll_by(&m, r, d);
            self.redraw();
        } else if zooming {
            self.zoom_at((d.y * 0.004).exp(), self.cursor);
        } else {
            self.viewport.drag(d);
            self.redraw();
        }
    }

    fn set_cursor_icon(&mut self, icon: CursorIcon) {
        if icon != self.cursor_icon {
            self.cursor_icon = icon;
            if let Some(w) = self.window() {
                w.set_cursor(icon);
            }
        }
    }

    /// Hover feedback and the pointer's shape.
    pub fn update_pointer(&mut self) {
        if self.on_welcome() {
            let hover = self.chrome.welcome_rects.iter().position(|r| r.contains(self.cursor.to_point()));
            if hover != self.chrome.welcome_hover {
                self.chrome.welcome_hover = hover;
                self.redraw();
            }
            let icon = if hover.is_some() { CursorIcon::Pointer } else { CursorIcon::Default };
            self.set_cursor_icon(icon);
            return;
        }
        let link = self.empty_link_at_cursor();
        if link != self.learn.empty_hover {
            self.learn.empty_hover = link;
            self.redraw();
        }
        if link.is_some() {
            return self.set_cursor_icon(CursorIcon::Pointer);
        }
        let in_bar = self.cursor.y > self.view_size().y;
        let p = self.cursor.to_point();
        let layout = self.side_layout().filter(|l| !in_bar && p.x <= l.rect.x1 + 4.0 * self.scale_factor());
        let (side_hover, footer) = match &layout {
            Some(l) => (l.row_at(p), l.footer.contains(p)),
            None => (None, false),
        };
        if (side_hover, footer) != (self.workspace.hover, self.workspace.hover_footer) {
            (self.workspace.hover, self.workspace.hover_footer) = (side_hover, footer);
            self.redraw();
        }
        let on_side_divider = layout.as_ref().is_some_and(|l| l.on_divider(p));
        let in_side = layout.is_some() && !on_side_divider;
        let show_path = in_bar && self.cursor.x < self.chrome.bar.file_end;
        if show_path != self.chrome.show_path {
            self.chrome.show_path = show_path;
            self.redraw();
        }
        let in_inspector = !in_bar && self.inspector_rect().contains(p);
        let inspector_hover = if in_inspector && self.palette.open.is_none() { self.inspector_hit() } else { None };
        if inspector_hover != self.inspector.hover {
            self.inspector.hover = inspector_hover;
            self.redraw();
        }
        let panning = self.interaction.drag_from.is_some();
        let busy = self.interaction.band.is_some() || self.menu.is_some();
        let hovered = if panning || busy || self.interaction.node_drag.is_some() || self.palette.open.is_some() {
            None
        } else {
            self.node_at_cursor()
                .map(Target::Node)
                .or_else(|| self.edge_at_cursor().map(Target::Edge))
                .or_else(|| self.branch_child_at_cursor().map(Target::Node))
                .or_else(|| self.frame_title_at_cursor().map(Target::Frame))
        };
        if hovered != self.interaction.hovered {
            self.interaction.hovered = hovered;
            self.redraw();
        }
        let pane = self.pane_rect();
        let on_divider = pane.width() > 0.0 && (self.cursor.x - pane.x1).abs() <= 4.0 * self.scale_factor() && !in_bar;
        let icon = match &self.interaction.node_drag {
            _ if self.workspace.resizing || (on_side_divider && self.workspace.drag.is_none()) => CursorIcon::ColResize,
            _ if self.workspace.drag.as_ref().is_some_and(|d| d.moved) => CursorIcon::Grabbing,
            _ if self.text.drag == Some(PaneDrag::Divider) || (on_divider && self.text.drag.is_none()) => {
                CursorIcon::ColResize
            }
            _ if in_side && self.interaction.node_drag.is_none() => match side_hover {
                Some(Row::File(_) | Row::Thing(_)) => CursorIcon::Pointer,
                _ if footer => CursorIcon::Pointer,
                _ => CursorIcon::Default,
            },
            _ if self.text.drag.is_some() || (pane.contains(p) && !in_bar) => CursorIcon::Text,
            _ if in_inspector && self.interaction.node_drag.is_none() && !panning => {
                if inspector_hover.is_some() {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::Default
                }
            }
            _ if panning => CursorIcon::Grabbing,
            _ if self.space && self.canvas().contains(p) => CursorIcon::Grab,
            Some(d) if d.gesture == Gesture::Connect => CursorIcon::Crosshair,
            Some(NodeDrag { gesture: Gesture::Drag(DragMode::Move | DragMode::Outline), .. }) => CursorIcon::Grabbing,
            _ if self.interaction.connecting.is_some() => CursorIcon::Crosshair,
            _ if self.interaction.hovered.is_some() => CursorIcon::Pointer,
            _ => CursorIcon::Default,
        };
        self.set_cursor_icon(icon);
        self.update_tip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_go_to_the_first_part_that_takes_them() {
        assert_eq!(press_zone(true, true, true, true, true), Zone::Palette);
        assert_eq!(press_zone(false, true, false, false, false), Zone::Inspector);
        // The sidebar's divider overlaps the pane's left edge: the sidebar wins.
        assert_eq!(press_zone(false, false, true, true, false), Zone::Sidebar);
        assert_eq!(press_zone(false, false, false, true, true), Zone::Pane);
        assert_eq!(press_zone(false, false, false, false, true), Zone::Welcome);
        assert_eq!(press_zone(false, false, false, false, false), Zone::Canvas);
    }

    #[test]
    fn releases_end_the_drag_that_started() {
        assert_eq!(release_zone(true, true), Zone::Sidebar);
        assert_eq!(release_zone(false, true), Zone::Pane);
        assert_eq!(release_zone(false, false), Zone::Canvas);
    }
}
