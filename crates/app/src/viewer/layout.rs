//! Where the parts of the window go: the sidebar, the text pane, the
//! canvas and the status bar.

use rhizo_app::chrome;
use rhizo_app::sidebar::{self, Layout};
use rhizo_app::vello::kurbo::{Rect, Vec2};

use super::App;

/// The narrowest the pane and the canvas get, in logical pixels.
pub const MIN_PANE: f64 = 240.0;
pub const MIN_CANVAS: f64 = 240.0;

impl App {
    /// The window without the status bar (shown with a diagram), in pixels.
    pub fn view_size(&self) -> Vec2 {
        let bar = if self.built.is_some() { chrome::BAR_HEIGHT } else { 0.0 };
        self.size() - Vec2::new(0.0, bar * self.scale_factor())
    }

    /// True when the project sidebar shows: as the person chose, else for
    /// an opened folder or a project of two files or more.
    pub fn sidebar_shown(&self) -> bool {
        let Some(scan) = &self.workspace.scan else { return false };
        let auto = self.session.folder.is_some() || scan.files.len() >= 2;
        !self.on_welcome() && !scan.root.as_os_str().is_empty() && self.workspace.side.open.unwrap_or(auto)
    }

    /// The sidebar's width in pixels; 0 when it does not show.
    pub fn side_px(&self) -> f64 {
        if !self.sidebar_shown() {
            return 0.0;
        }
        let dpr = self.scale_factor();
        (self.workspace.side.width * dpr).min(self.size().x - MIN_CANVAS * dpr).max(sidebar::MIN_WIDTH * dpr)
    }

    /// Where the sidebar is; empty when it does not show.
    pub fn side_layout_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.side_px(), self.view_size().y)
    }

    pub fn side_layout(&mut self) -> Option<Layout> {
        let rect = Rect::new(0.0, 0.0, self.side_px(), self.view_size().y);
        let dpr = self.scale_factor();
        self.sidebar_shown().then(|| Layout::new(&self.workspace.model, &mut self.workspace.side, rect, dpr))
    }

    /// True when the text pane shows: it is open and the file is KDL.
    pub fn pane_shown(&self) -> bool {
        self.pane.open && self.editor.is_some() && !self.on_welcome()
    }

    /// The text pane's width in pixels; 0 when it does not show.
    pub fn pane_px(&self) -> f64 {
        if !self.pane_shown() {
            return 0.0;
        }
        let dpr = self.scale_factor();
        (self.pane.width * dpr).min(self.size().x - self.side_px() - MIN_CANVAS * dpr).max(MIN_PANE * dpr)
    }

    /// The text pane sits between the sidebar and the canvas.
    pub fn pane_rect(&self) -> Rect {
        let x0 = self.side_px();
        Rect::new(x0, 0.0, x0 + self.pane_px(), self.view_size().y)
    }

    /// Where the diagram shows: right of the sidebar and the pane, left of
    /// the inspector, above the status bar.
    pub fn canvas(&self) -> Rect {
        let v = self.view_size();
        Rect::new(self.side_px() + self.pane_px(), 0.0, v.x - self.inspector_px(), v.y)
    }

    /// Changes what is beside the canvas (the sidebar, the pane, the
    /// inspector) keeping the diagram centred in what is left of the window.
    pub fn change_layout(&mut self, f: impl FnOnce(&mut Self)) {
        let before = self.canvas().center().x;
        f(self);
        let pan = Vec2::new(self.canvas().center().x - before, 0.0);
        self.viewport.pan(pan);
        self.redraw();
    }
}
