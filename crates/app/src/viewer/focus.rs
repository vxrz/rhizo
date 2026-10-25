//! Which part of the window has the keyboard, the ring that shows it, and
//! the tooltips that say what a control does.
//!
//! Exactly one part has the keys at a time. It is worked out from what is
//! open, the first of: a dialog (the adopt report), a context menu, the
//! palette, an inspector field being typed in, the text pane, the sidebar,
//! and last the canvas (where a label may be typed). [`App::key`] routes
//! by it, Esc steps back towards the canvas.

use std::time::{Duration, Instant};

use rhizo_app::chrome;
use rhizo_app::inspector::{self, Input};
use rhizo_app::palette::{self, Action};
use rhizo_app::sidebar;
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::Vec2;

use super::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Canvas,
    Pane,
    /// An inspector field being typed in.
    Inspector,
    Sidebar,
    Palette,
    /// A context menu.
    Menu,
    /// The adopt report, or the About dialog.
    Dialog,
}

/// What is open, for [`focus_of`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Open {
    pub dialog: bool,
    pub menu: bool,
    pub palette: bool,
    pub field: bool,
    pub pane: bool,
    pub sidebar: bool,
}

/// The part with the keys: the first that is open, modal ones first.
pub fn focus_of(o: Open) -> Focus {
    match () {
        _ if o.dialog => Focus::Dialog,
        _ if o.menu => Focus::Menu,
        _ if o.palette => Focus::Palette,
        _ if o.field => Focus::Inspector,
        _ if o.pane => Focus::Pane,
        _ if o.sidebar => Focus::Sidebar,
        _ => Focus::Canvas,
    }
}

/// A text that takes typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTarget {
    Pane,
    /// The inspector field being typed in.
    Field,
    /// The palette's query.
    Query,
    /// A label typed on the canvas.
    Label,
}

/// The field Tab (or, `back`, shift-Tab) goes to from field `from`: the
/// next one that takes typing, round to the first.
pub fn next_field(inputs: &[Input], from: usize, back: bool) -> Option<usize> {
    let typed = |i: &usize| matches!(inputs[*i], Input::Text | Input::Number { .. });
    let n = inputs.len();
    let order: Vec<usize> =
        if back { (1..=n).map(|k| (from + n - k) % n).collect() } else { (1..=n).map(|k| (from + k) % n).collect() };
    order.into_iter().find(typed).filter(|&i| i != from || typed(&from))
}

/// How long the pointer rests on a control before its tooltip shows.
pub const TIP_DELAY: Duration = Duration::from_millis(600);

/// A tooltip waiting to show, or showing: its text and keys, and since
/// when the pointer rests on what it is about.
pub struct Tip {
    pub text: String,
    pub keys: String,
    pub since: Instant,
    pub at: Vec2,
}

impl App {
    pub fn focus(&self) -> Focus {
        focus_of(Open {
            dialog: self.adopting.job.review().is_some() || self.chrome.about.is_some() || self.learn.gallery.is_some(),
            menu: self.menu.is_some(),
            palette: self.palette.open.is_some(),
            field: self.inspector.typing.is_some(),
            pane: self.pane.focused && self.pane_shown(),
            sidebar: self.workspace.focused && self.sidebar_shown(),
        })
    }

    /// Gives the keys to the canvas, the pane or the sidebar (an inspector
    /// field takes them by being typed in; the palette, menus and dialogs
    /// by opening).
    pub fn focus_on(&mut self, f: Focus) {
        if f != Focus::Inspector {
            self.finish_field(true);
        }
        self.focus_pane(f == Focus::Pane);
        if self.workspace.focused != (f == Focus::Sidebar) {
            self.workspace.focused = f == Focus::Sidebar;
            self.redraw();
        }
    }

    /// Input methods (CJK, dead keys) work while a text has the keys.
    pub fn allow_ime(&mut self) {
        let on = self.focused_text().is_some();
        if on != self.ime_on {
            self.ime_on = on;
            if let Some(w) = self.window() {
                w.set_ime_allowed(on);
            }
        }
    }

    /// The ring round the part with the keys. The canvas gets one only
    /// when something beside it could have them instead.
    pub fn focus_scene(&self) -> Option<Scene> {
        let dpr = self.scale_factor();
        let rect = match self.focus() {
            Focus::Pane => self.pane_rect(),
            Focus::Inspector => self.inspector_rect(),
            Focus::Sidebar => self.side_layout_rect(),
            Focus::Canvas if self.pane_shown() || self.sidebar_shown() || self.inspector_shown() => self.canvas(),
            _ => return None,
        };
        if self.on_welcome() || rect.width() <= 0.0 {
            return None;
        }
        let mut s = Scene::new();
        chrome::focus_ring(&mut s, rect, dpr);
        Some(s)
    }

    /// What the pointer rests on says, with its keys: inspector controls,
    /// sidebar rows, the status bar's parts.
    fn tip_here(&mut self) -> Option<(String, String)> {
        if self.palette.open.is_some()
            || self.menu.is_some()
            || self.adopting.job.review().is_some()
            || self.chrome.about.is_some()
            || self.learn.gallery.is_some()
        {
            return None;
        }
        let p = self.cursor.to_point();
        if let (Some(hit), true) = (self.inspector.hover, self.inspector_rect().contains(p)) {
            let text = inspector::tip(&self.inspector_model()?, hit);
            return Some(split_keys(text));
        }
        if let Some(row) = self.workspace.hover {
            return Some((sidebar::tip(&self.workspace.model, row), String::new()));
        }
        if self.workspace.hover_footer {
            return Some(split_keys(palette::tip(Action::ProblemsMenu)));
        }
        let bar = self.chrome.bar;
        if bar.zoom.contains(p) {
            let zoom = |a| palette::command(a).map_or("", |c| c.keys);
            return Some((
                "Zoom: 100%, fit, in and out".into(),
                format!(
                    "{}  {}  {}  {}",
                    zoom(Action::ZoomActual),
                    zoom(Action::Fit),
                    zoom(Action::ZoomIn),
                    zoom(Action::ZoomOut)
                ),
            ));
        }
        if bar.problems.is_some_and(|r| r.contains(p)) {
            return Some(split_keys(palette::tip(Action::ProblemsMenu)));
        }
        if bar.hint.contains(p) {
            return Some(split_keys(palette::tip(Action::Palette)));
        }
        None
    }

    /// Follows the pointer: a new control starts the wait for its tooltip.
    pub fn update_tip(&mut self) {
        let here = self.tip_here().filter(|(t, _)| !t.is_empty());
        let same = matches!((&here, &self.tip), (Some((t, _)), Some(tip)) if *t == tip.text);
        if same {
            return;
        }
        let shown = self.tip.as_ref().is_some_and(|t| t.since.elapsed() >= TIP_DELAY);
        self.tip = here.map(|(text, keys)| Tip { text, keys, since: Instant::now(), at: self.cursor });
        if shown {
            self.redraw();
        }
    }

    /// A press or a key hides the tooltip until the pointer moves on.
    pub fn hide_tip(&mut self) {
        if let Some(t) = &mut self.tip {
            t.since = Instant::now() + Duration::from_secs(3600);
            self.redraw();
        }
    }

    /// When the waiting tooltip is due.
    pub fn tip_due(&self) -> Option<Instant> {
        self.tip.as_ref().map(|t| t.since + TIP_DELAY).filter(|t| *t > Instant::now())
    }

    /// The tooltip, once the pointer has rested long enough.
    pub fn tip_scene(&mut self) -> Option<Scene> {
        let tip = self.tip.as_ref().filter(|t| t.since.elapsed() >= TIP_DELAY)?;
        let (text, keys, at) = (tip.text.clone(), tip.keys.clone(), tip.at);
        let (size, dpr, bg) = (self.size(), self.scale_factor(), self.background());
        self.fonts();
        let fonts = self.fonts.as_ref().expect("loaded above");
        let mut s = Scene::new();
        chrome::tooltip(&mut s, fonts, &mut self.glyphs, at, size, dpr, bg, &text, &keys);
        Some(s)
    }
}

/// "Toggle inspector   ⌘I" into the name and the keys.
fn split_keys(tip: String) -> (String, String) {
    match tip.split_once("   ") {
        Some((t, k)) => (t.to_string(), k.to_string()),
        None => (tip, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_part_has_the_keys_modal_ones_first() {
        let all = Open { dialog: true, menu: true, palette: true, field: true, pane: true, sidebar: true };
        assert_eq!(focus_of(all), Focus::Dialog);
        assert_eq!(focus_of(Open { dialog: false, ..all }), Focus::Menu);
        assert_eq!(focus_of(Open { dialog: false, menu: false, ..all }), Focus::Palette);
        let typing = Open { field: true, pane: true, sidebar: true, ..Open::default() };
        assert_eq!(focus_of(typing), Focus::Inspector, "a field being typed in wins over the pane");
        assert_eq!(focus_of(Open { field: false, ..typing }), Focus::Pane);
        assert_eq!(focus_of(Open { sidebar: true, ..Open::default() }), Focus::Sidebar);
        assert_eq!(focus_of(Open::default()), Focus::Canvas);
    }

    #[test]
    fn tab_walks_the_fields_that_take_typing() {
        let inputs = [Input::Text, Input::Toggle, Input::Number { integer: true }, Input::Choice(vec![]), Input::Text];
        assert_eq!(next_field(&inputs, 0, false), Some(2), "skips the switch");
        assert_eq!(next_field(&inputs, 2, false), Some(4), "and the list");
        assert_eq!(next_field(&inputs, 4, false), Some(0), "round to the first");
        assert_eq!(next_field(&inputs, 0, true), Some(4), "shift-Tab goes back");
        assert_eq!(next_field(&inputs, 2, true), Some(0));
        assert_eq!(next_field(&[Input::Text], 0, false), Some(0), "one field: stays");
        assert_eq!(next_field(&[Input::Toggle, Input::Text], 1, false), Some(1));
        assert_eq!(next_field(&[Input::Toggle], 0, false), None);
    }

    #[test]
    fn tips_split_off_their_keys() {
        assert_eq!(split_keys("Toggle inspector   ⌘I".into()), ("Toggle inspector".into(), "⌘I".into()));
        assert_eq!(split_keys("Compile specs…".into()), ("Compile specs…".into(), String::new()));
    }
}
