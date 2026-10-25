//! What the window says around the diagram: notices (toasts and error
//! banners), the problems banner, the status bar and the welcome screen.

use std::time::{Duration, Instant};

use rhizo_app::about;
use rhizo_app::chrome::{self, Problems};
use rhizo_app::draw::{self, FontCache};
use rhizo_app::files;
use rhizo_app::sidebar;
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::{Affine, Rect, Vec2};
use rhizo_app::welcome::{self, Item, Welcome};
use rhizo_core::Color;
use rhizo_render::Fonts;

use super::App;
use super::interaction::Target;

/// A short message: a toast when it went well, a banner when not.
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

impl Notice {
    fn lasts(&self) -> Duration {
        Duration::from_secs(if self.error { 8 } else { 3 })
    }
}

/// How long "Saved" shows in the status bar.
const SAVED_FOR: Duration = Duration::from_secs(2);
const FADE: Duration = Duration::from_millis(400);

/// 1 while a message `lasts`, fading to 0 over its last [`FADE`].
fn fade(at: Instant, lasts: Duration) -> f32 {
    let left = lasts.saturating_sub(at.elapsed());
    (left.as_secs_f32() / FADE.as_secs_f32()).min(1.0)
}

#[derive(Default)]
pub struct Chrome {
    pub notice: Option<Notice>,
    /// When an edit was last written to the file.
    pub saved_at: Option<Instant>,
    /// Errors or warnings in the file, shown in a banner.
    pub problems: Option<Problems>,
    /// Where the status bar's parts are: hovering the file name shows its
    /// full path, the others their tooltips.
    pub bar: chrome::Bar,
    pub show_path: bool,
    /// Where the welcome screen's items were drawn, for clicks.
    pub welcome_rects: Vec<Rect>,
    pub welcome_focus: Option<usize>,
    pub welcome_hover: Option<usize>,
    /// The About dialog, while open: its panel, its links and the one under the pointer.
    pub about: Option<AboutDialog>,
}

#[derive(Default)]
pub struct AboutDialog {
    pub panel: Rect,
    pub links: Vec<(Rect, about::Link)>,
    pub hover: Option<about::Link>,
}

impl Chrome {
    /// When a notice or "Saved" starts fading out, if one will, or the
    /// pane's typing is `due` to be checked.
    pub fn next_wake(&self, due: Option<Instant>) -> Option<Instant> {
        let now = Instant::now();
        let notice = self.notice.as_ref().map(|n| n.at + n.lasts() - FADE);
        let saved = self.saved_at.map(|t| t + SAVED_FOR - FADE);
        let typing = due.map(|t| t.max(now + Duration::from_millis(1)));
        [notice, saved, typing].into_iter().flatten().filter(|t| *t > now).min()
    }

    /// True while a notice or "Saved" is fading, so frames keep coming.
    pub fn fading(&mut self) -> bool {
        let live = |at: Instant, lasts: Duration| at.elapsed() < lasts && at.elapsed() + FADE >= lasts;
        if self.notice.as_ref().is_some_and(|n| n.at.elapsed() >= n.lasts()) {
            self.notice = None;
        }
        self.notice.as_ref().is_some_and(|n| live(n.at, n.lasts())) || self.saved_at.is_some_and(|t| live(t, SAVED_FOR))
    }

    /// The notice, else the file's problems, centred in `size`.
    fn draw_notice(&self, s: &mut Scene, fonts: &Fonts, glyphs: &mut FontCache, size: Vec2, dpr: f64, bg: Color) {
        match &self.notice {
            Some(n) if n.error => {
                let p = Problems::single(&n.text);
                chrome::banner(s, fonts, glyphs, size, dpr, bg, &p);
            }
            Some(n) => {
                let alpha = fade(n.at, n.lasts());
                chrome::toast(s, fonts, glyphs, size, dpr, bg, &n.text, alpha);
            }
            None => {
                if let Some(p) = &self.problems {
                    chrome::banner(s, fonts, glyphs, size, dpr, bg, p);
                }
            }
        }
    }
}

/// What an opened folder shows before a file is chosen, centred in `size`.
fn empty_folder_hint(s: &mut Scene, fonts: &Fonts, glyphs: &mut FontCache, size: Vec2, dpr: f64, bg: Color) {
    let ink = chrome::ink(bg);
    let mut line = |text: &str, y: f64, pt: f64, color: Color| {
        let pt = (pt * dpr) as f32;
        let w = fonts.shape(text, pt, 400).map_or(0.0, |s| s.width) as f64;
        let (x, y) = (((size.x - w) / 2.0) as f32, (size.y / 2.0 + y * dpr) as f32);
        draw::text(s, fonts, glyphs, Affine::IDENTITY, text, x, y, pt, 400, color);
    };
    line("Choose a file in the sidebar", -6.0, 15.0, ink.mix(bg, 0.35));
    let hint = chrome::keys(fonts, "⌘P go to file   ·   ⌘N new from template");
    line(&hint, 18.0, 12.5, ink.mix(bg, 0.55));
}

impl App {
    pub fn toast(&mut self, text: String) {
        self.chrome.notice = Some(Notice { text, error: false, at: Instant::now() });
        self.redraw();
    }

    pub fn fail(&mut self, text: String) {
        eprintln!("{text}");
        self.chrome.notice = Some(Notice { text, error: true, at: Instant::now() });
        self.redraw();
    }

    /// The welcome screen shows when no file or folder is open, or the
    /// file is not there yet.
    pub fn on_welcome(&self) -> bool {
        self.built.is_none() && self.session.path.as_ref().is_none_or(|p| !p.exists()) && self.session.folder.is_none()
    }

    pub fn welcome(&self) -> Welcome<'_> {
        Welcome { missing: self.session.path.as_deref(), recent: &self.session.recent }
    }

    pub fn activate(&mut self, item: Item) {
        match item {
            Item::New(kind) => self.new_file(kind),
            Item::Create(kind) => self.create_missing(kind),
            Item::Learn => self.learn(),
            Item::Samples => self.open_gallery(),
            Item::Open => self.open_dialog(),
            Item::OpenFolder => self.open_folder_dialog(),
            Item::Recent(p) => self.open(p),
            Item::GitHub => self.open_link(about::Link::GitHub),
            Item::About => self.open_link(about::Link::About),
        }
        self.redraw();
    }

    /// A footer link: the code's home in the browser, or the About dialog.
    pub fn open_link(&mut self, link: about::Link) {
        match link {
            about::Link::GitHub => {
                if let Err(e) = files::open_url(rhizo_core::about::GITHUB) {
                    self.fail(format!("cannot open {}: {e}", rhizo_core::about::GITHUB));
                }
            }
            about::Link::About => {
                self.menu = None;
                self.palette.open = None;
                self.chrome.about = Some(AboutDialog::default());
            }
        }
        self.redraw();
    }

    /// A press while the About dialog is open: a link opens, outside the panel closes it.
    pub fn about_press(&mut self) {
        let p = self.cursor.to_point();
        let Some(d) = &self.chrome.about else { return };
        let link = d.links.iter().find(|(r, _)| r.contains(p)).map(|(_, l)| *l);
        let outside = !d.panel.contains(p);
        match link {
            Some(l) => self.open_link(l),
            None if outside => self.chrome.about = None,
            None => {}
        }
        self.redraw();
    }

    /// The link under the pointer is underlined.
    pub fn about_hover(&mut self) {
        let p = self.cursor.to_point();
        let Some(d) = &mut self.chrome.about else { return };
        let hover = d.links.iter().find(|(r, _)| r.contains(p)).map(|(_, l)| *l);
        if hover != d.hover {
            d.hover = hover;
            self.redraw();
        }
    }

    /// The About dialog, in window pixels.
    pub fn about_scene(&mut self) -> Option<Scene> {
        let hover = self.chrome.about.as_ref()?.hover;
        let (size, dpr, bg) = (self.size(), self.scale_factor(), self.background());
        self.fonts();
        let fonts = self.fonts.as_ref().expect("loaded above");
        let mut s = Scene::new();
        let (panel, links) = about::draw(&mut s, fonts, &mut self.glyphs, size, dpr, bg, hover);
        if let Some(d) = &mut self.chrome.about {
            (d.panel, d.links) = (panel, links);
        }
        Some(s)
    }

    /// When the window next needs to wake up by itself, if at all.
    pub fn next_wake(&self) -> Option<Instant> {
        let wake = self.chrome.next_wake(self.pane.sync.due());
        [wake, self.tip_due()].into_iter().flatten().min()
    }

    /// What the status bar says about the selection.
    fn selection_text(&self) -> Option<String> {
        let many = self.interaction.nodes().len();
        if many > 1 {
            return Some(format!("{many} selected"));
        }
        self.interaction.selected.as_ref().and_then(|t| match t {
            Target::Node(id) => {
                let n = self.built.as_ref()?.diagram.nodes.iter().find(|n| &n.id == id)?;
                let text = chrome::describe_node(id.as_str(), &n.label);
                // A node defined here and referred to elsewhere says where.
                let defined = self.built.as_ref().is_some_and(|b| !b.references.contains_key(id));
                let used = match (&self.workspace.scan, self.current_rel()) {
                    (Some(scan), Some(rel)) if defined => scan.used_in(&rel, id),
                    _ => 0,
                };
                Some(if used > 0 { format!("{text} · used in {}", chrome::plural(used, "file")) } else { text })
            }
            Target::Edge(k) => {
                let e = self.editor.as_ref()?.document().edges.get(*k)?;
                Some(chrome::describe_edge(e.from.as_str(), e.to.as_str(), e.label.as_deref()))
            }
            Target::Frame(f) => {
                let doc = self.editor.as_ref()?.document();
                let frame = doc.frame(f)?;
                let what = frame.ty.as_ref().map_or_else(|| "frame".to_string(), |t| t.to_string());
                Some(format!("{} ({what})", frame.label.as_deref().unwrap_or(f.as_str())))
            }
        })
    }

    /// The status bar's contents; `None` without a diagram.
    fn status(&self, dpr: f64) -> Option<chrome::Status> {
        let file = match &self.session.path {
            Some(p) if self.chrome.show_path => files::display_path(p),
            Some(p) => files::name(p),
            None => String::new(),
        };
        let selection = self.selection_text();
        let pane = self.focus() == super::focus::Focus::Pane;
        let hints = if self.learn.hints_off { Vec::new() } else { rhizo_app::palette::hints(self.selection(), pane) };
        self.built.as_ref().map(|b| chrome::Status {
            file,
            nodes: b.nodes,
            edges: b.diagram.edges.len(),
            zoom: self.viewport.camera.zoom / dpr,
            selection,
            saved: self.chrome.saved_at.map_or(0.0, |t| fade(t, SAVED_FOR)),
            project: self.workspace.scan.as_ref().map_or((0, 0), |s| s.counts(None)),
            job: self.adopting.job.status(),
            hints,
        })
    }

    /// Status bar, problems banner, notices and the welcome screen, in window pixels.
    pub fn chrome_scene(&mut self) -> Scene {
        let (size, dpr) = (self.size(), self.scale_factor());
        let bg = self.background();
        let status = self.status(dpr);
        let on_welcome = self.on_welcome();
        let (left, right) = (self.canvas().x0, self.canvas().x1);
        let empty_folder = !on_welcome && self.session.path.is_none();
        let chip = self.workspace.drag_chip();
        self.fonts();
        let fonts = self.fonts.as_ref().expect("loaded above");
        let mut s = Scene::new();
        if on_welcome {
            let w = Welcome { missing: self.session.path.as_deref(), recent: &self.session.recent };
            let (focus, hover) = (self.chrome.welcome_focus, self.chrome.welcome_hover);
            self.chrome.welcome_rects = welcome::draw(&mut s, fonts, &mut self.glyphs, size, dpr, bg, &w, focus, hover);
        } else if let Some(st) = &status {
            self.chrome.bar = chrome::status_bar(&mut s, fonts, &mut self.glyphs, size, dpr, bg, st);
        }
        if let Some((g, label)) = &chip {
            sidebar::chip(&mut s, fonts, &mut self.glyphs, self.cursor.to_point(), dpr, bg, *g, label);
        }
        // Banners and toasts centre over the canvas, beside the sidebar and the pane.
        let mut over = Scene::new();
        let size = Vec2::new(right - left, size.y);
        if empty_folder {
            empty_folder_hint(&mut over, fonts, &mut self.glyphs, size, dpr, bg);
        }
        self.chrome.draw_notice(&mut over, fonts, &mut self.glyphs, size, dpr, bg);
        s.append(&over, Some(Affine::translate((left, 0.0))));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_fade_over_their_last_moments() {
        assert_eq!(fade(Instant::now(), SAVED_FOR), 1.0);
        let old = Instant::now() - Duration::from_secs(10);
        assert_eq!(fade(old, SAVED_FOR), 0.0);
        let notice = |error| Notice { text: String::new(), error, at: Instant::now() };
        assert!(notice(true).lasts() > notice(false).lasts(), "errors stay longer");
    }

    #[test]
    fn the_window_wakes_when_a_fade_starts() {
        let mut c = Chrome::default();
        assert_eq!(c.next_wake(None), None);
        let at = Instant::now();
        c.notice = Some(Notice { text: "Saved as x".into(), error: false, at });
        assert_eq!(c.next_wake(None), Some(at + Duration::from_secs(3) - FADE));
        // A pending check of the typing that is already due wakes at once.
        assert!(c.next_wake(Some(at)).is_some_and(|t| t < at + Duration::from_secs(1)));
        assert!(!c.fading());
    }
}
