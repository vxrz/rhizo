//! Learning the viewer: the tutorial's progress card, the sample gallery,
//! the empty-canvas hint's links and the status bar's hints switch.

use std::path::{Path, PathBuf};

use rhizo_app::chrome::{self, EmptyLink};
use rhizo_app::gallery::{self, Drawn};
use rhizo_app::learn::{self, Seen};
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::{Affine, Rect};
use rhizo_app::{Built, Engine, files};
use rhizo_core::{Color, Document};
use rhizo_interop::Format;
use rhizo_project::samples::{self, Sample};

use super::App;

/// The tutorial's progress, for this session: what was seen, and the
/// steps ticked (a step stays ticked once done, even if undone later).
#[derive(Default)]
pub struct Learning {
    pub seen: Seen,
    pub done: Vec<bool>,
    /// The tutorial as it starts, parsed once.
    start: Option<Document>,
    /// The status bar's hints are off (this session; the viewer keeps no settings file).
    pub hints_off: bool,
    pub gallery: Option<GalleryDialog>,
    /// Thumbnails, kept between openings: built for this background.
    thumbs: Option<(Color, Vec<Option<Built>>)>,
    /// The empty-canvas hint's links, in window pixels, and the one under the pointer.
    pub empty_links: Vec<(Rect, EmptyLink)>,
    pub empty_hover: Option<EmptyLink>,
}

#[derive(Default)]
pub struct GalleryDialog {
    pub drawn: Drawn,
    pub focus: Option<usize>,
    pub hover: Option<usize>,
}

impl App {
    /// The open document is the tutorial.
    pub fn learning(&self) -> bool {
        self.editor.as_ref().is_some_and(|e| learn::is_tutorial(e.document()))
    }

    /// The window saw a step happen; it counts only in the tutorial.
    pub fn saw(&mut self, f: impl FnOnce(&mut Seen)) {
        if self.learning() {
            f(&mut self.learn.seen);
        }
    }

    /// The steps done, updated from the document.
    fn learn_done(&mut self) -> Option<Vec<bool>> {
        let doc = self.editor.as_ref().map(|e| e.document()).filter(|d| learn::is_tutorial(d))?;
        let start = self.learn.start.get_or_insert_with(learn::start);
        let now = learn::progress(start, doc, self.learn.seen);
        let done = &mut self.learn.done;
        done.resize(now.len(), false);
        for (d, n) in done.iter_mut().zip(now) {
            *d |= n;
        }
        Some(done.clone())
    }

    /// The tutorial's card over the canvas, in window pixels.
    pub fn learn_scene(&mut self) -> Option<Scene> {
        if self.on_welcome() || self.learn.gallery.is_some() {
            return None;
        }
        let done = self.learn_done()?;
        let (canvas, dpr, bg) = (self.canvas(), self.scale_factor(), self.background());
        self.fonts();
        let fonts = self.fonts.as_ref().expect("loaded above");
        let mut s = Scene::new();
        learn::card(&mut s, fonts, &mut self.glyphs, canvas, dpr, bg, &done);
        Some(s)
    }

    /// Help ▸ Learn rhizo: a copy of the tutorial, saved where the person
    /// says, like a template: rhizo writes every edit to the file, so a
    /// temporary copy would lose the work.
    pub fn learn(&mut self) {
        self.new_named("learn", "Learn rhizo.rhizo");
    }

    pub fn toggle_hints(&mut self) {
        self.learn.hints_off = !self.learn.hints_off;
        let what = if self.learn.hints_off { "Hints hidden" } else { "Hints shown" };
        self.toast(format!("{what} in the status bar"));
    }

    /// The empty-canvas hint (an empty document), centred over the canvas.
    pub fn empty_scene(&mut self) -> Option<Scene> {
        self.learn.empty_links.clear();
        let empty = self.built.as_ref().is_some_and(|b| b.nodes == 0 && b.document.frames.iter().all(|f| f.implicit));
        if !empty || self.on_welcome() {
            return None;
        }
        let (canvas, dpr, bg) = (self.canvas(), self.scale_factor(), self.background());
        let size = canvas.size().to_vec2() - rhizo_app::vello::kurbo::Vec2::new(0.0, chrome::BAR_HEIGHT * dpr);
        let hover = self.learn.empty_hover;
        self.fonts();
        let fonts = self.fonts.as_ref().expect("loaded above");
        let mut over = Scene::new();
        let links = chrome::empty_hint(&mut over, fonts, &mut self.glyphs, size, dpr, bg, hover);
        self.learn.empty_links = links.into_iter().map(|(r, l)| (r + canvas.origin().to_vec2(), l)).collect();
        let mut s = Scene::new();
        s.append(&over, Some(Affine::translate(canvas.origin().to_vec2())));
        Some(s)
    }

    /// The empty-canvas link under the pointer.
    pub fn empty_link_at_cursor(&self) -> Option<EmptyLink> {
        let p = self.cursor.to_point();
        self.learn.empty_links.iter().find(|(r, _)| r.contains(p)).map(|(_, l)| *l)
    }

    pub fn follow_empty_link(&mut self, link: EmptyLink) {
        match link {
            EmptyLink::Learn => self.learn(),
            EmptyLink::Samples => self.open_gallery(),
        }
    }

    // ------------------------------------------------------------ gallery

    /// File ▸ Open Sample…: the gallery, its thumbnails drawn from the
    /// samples (once per background).
    pub fn open_gallery(&mut self) {
        self.menu = None;
        self.palette.open = None;
        let bg = self.background();
        if self.learn.thumbs.as_ref().is_none_or(|(c, _)| *c != bg) {
            let theme = self.session.theme.clone().unwrap_or_else(|| {
                let dark = chrome::ink(bg).r > 128;
                (if dark { "dark" } else { "light" }).to_string()
            });
            let thumbs = gallery::thumbnails(&mut Engine::default(), Some(&theme));
            self.learn.thumbs = Some((bg, thumbs));
        }
        self.learn.gallery = Some(GalleryDialog::default());
        self.redraw();
    }

    pub fn gallery_scene(&mut self) -> Option<Scene> {
        let g = self.learn.gallery.as_ref()?;
        let (focus, hover) = (g.focus, g.hover);
        let (size, dpr, bg) = (self.size(), self.scale_factor(), self.background());
        self.fonts();
        let fonts = self.fonts.as_ref().expect("loaded above");
        let thumbs = self.learn.thumbs.as_ref().map_or(&[][..], |(_, t)| t.as_slice());
        let mut s = Scene::new();
        let drawn = gallery::draw(&mut s, fonts, &mut self.glyphs, size, dpr, bg, thumbs, focus, hover);
        if let Some(g) = &mut self.learn.gallery {
            g.drawn = drawn;
        }
        Some(s)
    }

    /// A press while the gallery is open: a tile opens its sample, Close closes.
    pub fn gallery_press(&mut self) {
        let p = self.cursor.to_point();
        let Some(g) = &self.learn.gallery else { return };
        if g.drawn.close.contains(p) {
            self.learn.gallery = None;
        } else if let Some(i) = g.drawn.tiles.iter().position(|r| r.contains(p)) {
            self.open_sample(i);
        }
        self.redraw();
    }

    pub fn gallery_hover(&mut self) {
        let p = self.cursor.to_point();
        let Some(g) = &mut self.learn.gallery else { return };
        let hover = g.drawn.tiles.iter().position(|r| r.contains(p));
        if hover != g.hover {
            g.hover = hover;
            self.redraw();
        }
    }

    /// Arrows move between samples, Enter opens one, Esc closes.
    pub fn gallery_key(&mut self, key: &winit::keyboard::Key<&str>) {
        use winit::keyboard::{Key, NamedKey};
        let Some(g) = &mut self.learn.gallery else { return };
        let n = g.drawn.tiles.len().max(1);
        match key {
            Key::Named(NamedKey::Escape) => self.learn.gallery = None,
            Key::Named(NamedKey::Enter | NamedKey::Space) => {
                if let Some(i) = g.focus {
                    self.open_sample(i);
                }
            }
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown | NamedKey::Tab) => {
                g.focus = Some(g.focus.map_or(0, |i| (i + 1).min(n - 1)));
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => {
                g.focus = Some(g.focus.map_or(0, |i| i.saturating_sub(1)))
            }
            _ => {}
        }
        self.redraw();
    }

    /// Opens a copy of the `i`-th sample of the gallery: a file saved where
    /// a dialog says, a project into a new folder; Mermaid and Markdown go
    /// through the import, which asks where to save the diagram.
    fn open_sample(&mut self, i: usize) {
        let Some(sample) = gallery::ordered().get(i).copied() else { return };
        let to = match kind(sample) {
            Kind::Import => {
                let dir = std::env::temp_dir().join(format!("rhizo-samples-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&dir);
                // create_dir, not _all: a folder of that name another user made in /tmp is refused.
                std::fs::create_dir(&dir).map(|()| dir).ok()
            }
            Kind::Project => self
                .ask(|d| d.set_title("Copy the sample project into a new folder").set_file_name(sample.id).save_file()),
            Kind::File => self.ask_rhizo("Save a copy of the sample", None, sample.files[0].0),
        };
        let Some(to) = to else { return };
        match samples::copy(sample, &to) {
            Ok(path) => {
                self.learn.gallery = None;
                self.open(path);
            }
            Err(e) => self.fail(e),
        }
    }
}

enum Kind {
    File,
    Project,
    /// Another format: opening it imports it.
    Import,
}

fn kind(s: &Sample) -> Kind {
    let first = PathBuf::from(s.files[0].0);
    match () {
        _ if s.is_project() => Kind::Project,
        _ if files::opening(&first) != files::Opening::Direct && Format::from_path(Path::new(&first)).is_some() => {
            Kind::Import
        }
        _ => Kind::File,
    }
}
