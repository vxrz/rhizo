//! The window: [`App`] holds its state in parts, each in its own module
//! with the methods that work on it. Input arrives in [`events`] and is
//! handed to [`keys`] and [`mouse`]; [`render`] composes the scenes the
//! parts draw.

mod adopting;
mod commands;
mod context;
mod dialogs;
mod events;
mod focus;
mod frames;
mod hud;
mod inspect;
mod interaction;
mod keys;
mod layout;
mod learning;
#[cfg(target_os = "macos")]
mod menubar;
mod mouse;
mod overlay;
mod pane;
mod render;
mod session;
mod viewport;
mod workers;
mod workspace;

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use rhizo_app::Built;
use rhizo_app::draw::FontCache;
use rhizo_app::files;
use rhizo_app::project;
use rhizo_app::textpane::{self, Pane};
use rhizo_app::vello::kurbo::Vec2;
use rhizo_app::vello::util::RenderContext;
use rhizo_core::{Color, Theme};
use rhizo_lang::Editor;
use rhizo_render::Fonts;
use winit::event_loop::{EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window};

use adopting::Adopting;
use commands::Palette;
use hud::Chrome;
use inspect::Inspector;
use interaction::Interaction;
use pane::TextUi;
use render::Gpu;
use session::Session;
use viewport::Viewport;
pub use workers::Event;
use workspace::Workspace;

pub struct App {
    /// The open file and folder, the watcher on them, and where they lead.
    session: Session,
    /// The project around the open file, and the sidebar showing it.
    workspace: Workspace,
    viewport: Viewport,
    /// Hover, selection, drags, "Connect to…" and label typing on the canvas.
    interaction: Interaction,
    /// The ⌘K palette.
    palette: Palette,
    /// A context menu, while open.
    menu: Option<context::ContextMenu>,
    /// The tooltip waiting to show, or showing.
    tip: Option<focus::Tip>,
    /// Space is held: a drag on the canvas pans.
    space: bool,
    /// Quit was chosen: the window closes once the event is handled.
    quitting: bool,
    /// Input methods are on (a text has the keys).
    ime_on: bool,
    /// The menu bar, once the app runs (macOS).
    #[cfg(target_os = "macos")]
    menubar: Option<menubar::MenuBar>,
    /// The selected node's type and props (⌘I).
    inspector: Inspector,
    /// "Adopt node…": its background dry run and the report waiting on it.
    adopting: Adopting,
    /// For threads started later (an adopt) to answer the window.
    proxy: EventLoopProxy<Event>,
    /// Notices, the problems banner, the status bar and the welcome screen.
    chrome: Chrome,
    /// The text pane; open or not, its buffer holds the file's one undo history.
    pane: Pane,
    /// The tutorial's progress, the gallery, the hints switch.
    learn: learning::Learning,
    /// Dragging and clicking in the pane, and its link to the analyzer.
    text: TextUi,
    cx: RenderContext,
    gpu: Option<Gpu>,
    built: Option<Built>,
    /// Present for KDL files; edits go through it and are written back.
    editor: Option<Editor>,
    cursor: Vec2,
    cursor_icon: CursorIcon,
    modifiers: ModifiersState,
    /// Text for overlays, loaded on first use.
    fonts: Option<Fonts>,
    /// The text pane's monospace font, loaded on first use.
    mono: Option<Fonts>,
    glyphs: FontCache,
}

impl App {
    /// The window's state for `path` (opened once the window exists), with
    /// the file watcher, the project watcher and the text analyzer started.
    pub fn new(event_loop: &EventLoop<Event>, path: Option<PathBuf>, theme: Option<String>) -> App {
        let watched: workers::Watched = Arc::new(std::sync::Mutex::new((0, None, theme.clone())));
        let (poke, poked) = mpsc::channel();
        let tidy: Arc<std::sync::atomic::AtomicBool> = Arc::default();
        workers::watch(watched.clone(), tidy.clone(), event_loop.create_proxy(), poked);
        let project_watched: project::Watched = Arc::default();
        let (project_poke, project_poked) = mpsc::channel();
        let proxy = event_loop.create_proxy();
        project::watch(project_watched.clone(), project_poked, move |s| {
            proxy.send_event(Event::Scan(Box::new(s))).is_ok()
        });
        App {
            session: Session {
                path: None,
                folder: None,
                theme,
                watched,
                generation: 0,
                pending_open: path,
                recent: files::recent(),
                back: Vec::new(),
                pending_spot: None,
                poke,
                tidy,
            },
            workspace: Workspace::new(project_watched, project_poke),
            viewport: Viewport::default(),
            interaction: Interaction::default(),
            palette: Palette::default(),
            menu: None,
            tip: None,
            space: false,
            quitting: false,
            ime_on: false,
            #[cfg(target_os = "macos")]
            menubar: None,
            inspector: Inspector::default(),
            adopting: Adopting::default(),
            proxy: event_loop.create_proxy(),
            chrome: Chrome::default(),
            pane: Pane::default(),
            learn: learning::Learning::default(),
            text: TextUi::new(workers::analyzer(event_loop.create_proxy())),
            cx: RenderContext::new(),
            gpu: None,
            built: None,
            editor: None,
            cursor: Vec2::ZERO,
            cursor_icon: CursorIcon::Default,
            modifiers: ModifiersState::empty(),
            fonts: None,
            mono: None,
            glyphs: FontCache::default(),
        }
    }

    fn window(&self) -> Option<&Arc<Window>> {
        self.gpu.as_ref().map(|g| &g.window)
    }

    fn redraw(&self) {
        if let Some(w) = self.window() {
            w.request_redraw();
        }
    }

    fn size(&self) -> Vec2 {
        self.window().map_or(Vec2::new(800.0, 600.0), |w| {
            let s = w.inner_size();
            Vec2::new(s.width as f64, s.height as f64)
        })
    }

    fn scale_factor(&self) -> f64 {
        self.window().map_or(1.0, |w| w.scale_factor())
    }

    fn fonts(&mut self) -> &Fonts {
        self.fonts.get_or_insert_with(|| Fonts::system(rhizo_core::theme::FONT_STACK))
    }

    fn mono(&mut self) -> &Fonts {
        if self.mono.is_none() {
            let db = (*self.fonts().database()).clone();
            self.mono = Some(Fonts::with_database(db, textpane::MONO_STACK));
        }
        self.mono.as_ref().expect("just loaded")
    }

    /// The diagram's background, or the theme's on the welcome screen
    /// (`--theme`, else the system's light or dark).
    fn background(&self) -> Color {
        if let Some(b) = &self.built {
            return b.background;
        }
        let dark = self.window().and_then(|w| w.theme()) == Some(winit::window::Theme::Dark);
        let fallback = if dark { "dark" } else { "light" };
        Theme::by_name(self.session.theme.as_deref().unwrap_or(fallback)).unwrap_or_default().background
    }
}
