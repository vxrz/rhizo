//! `rhizo-view FILE [--theme light|dark|paper]`: a live window onto a rhizo
//! document. Edit the file anywhere and the window follows; edit in the
//! window and the file follows (KDL files only).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use rhizo_app::draw::{self, FontCache};
use rhizo_app::interact::{self, Dir, DragMode};
use rhizo_app::palette::{self, Action};
use rhizo_app::vello::kurbo::{Affine, Line, Rect, RoundedRect, Stroke, Vec2};
use rhizo_app::vello::peniko::Fill;
use rhizo_app::vello::util::{RenderContext, RenderSurface};
use rhizo_app::vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene, wgpu};
use rhizo_app::{Built, Engine, peniko_color};
use rhizo_core::{Color, NodeId, Point};
use rhizo_lang::{EditError, Editor, Placement, Syntax};
use rhizo_render::Fonts;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const MARGIN: f64 = 24.0;
/// Selection, drop and edit indicators: the brand indigo.
const ACCENT: Color = Color::hex(0x6366f1);
/// Label given to nodes made with Tab or Enter until they are typed over.
const NEW_LABEL: &str = "New idea";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gesture {
    /// Move, re-parent or select, per the frame (D19).
    Drag(DragMode),
    /// ⌥-drag, or any drag in a layered/grid frame: draw an edge.
    Connect,
}

struct NodeDrag {
    id: NodeId,
    gesture: Gesture,
    start: Point,
    now: Point,
    /// Drop placement (outline drags) or the node to connect to.
    target: Option<Placement>,
    connect_to: Option<NodeId>,
}

/// A label being typed in the canvas.
struct TextEdit {
    id: NodeId,
    text: String,
    /// Made by Tab/Enter just now: Esc or an empty label removes it again.
    created: bool,
}

struct PaletteState {
    query: String,
    index: usize,
}

enum Reload {
    Built(Box<Built>, Duration),
    Failed(String),
}

/// Diagram units → physical pixels: `pixel = offset + diagram * zoom`.
#[derive(Clone, Copy)]
struct Camera {
    zoom: f64,
    offset: Vec2,
}

struct Gpu {
    window: Arc<Window>,
    surface: RenderSurface<'static>,
    renderer: Renderer,
}

struct App {
    path: PathBuf,
    cx: RenderContext,
    gpu: Option<Gpu>,
    built: Option<Built>,
    camera: Camera,
    cursor: Vec2,
    drag_from: Option<Vec2>,
    last_click: Option<(Instant, Vec2)>,
    modifiers: ModifiersState,
    /// Present for KDL files; edits go through it and are written back.
    editor: Option<Editor>,
    selected: Option<NodeId>,
    node_drag: Option<NodeDrag>,
    editing: Option<TextEdit>,
    palette: Option<PaletteState>,
    /// "Connect to…" from the palette: the next node clicked gets the edge.
    connecting: Option<NodeId>,
    /// Text for overlays, loaded on first use.
    fonts: Option<Fonts>,
    glyphs: FontCache,
}

impl App {
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

    /// Fits the whole diagram, never magnifying past 100%.
    fn fit(&mut self) {
        let Some(b) = &self.built else { return };
        let size = self.size();
        let m = MARGIN * self.scale_factor();
        let zoom = ((size.x - 2.0 * m) / b.width).min((size.y - 2.0 * m) / b.height).min(self.scale_factor()).max(0.01);
        let offset = (size - Vec2::new(b.width, b.height) * zoom) / 2.0;
        self.camera = Camera { zoom, offset };
        self.redraw();
    }

    /// Zooms by `factor`, keeping the diagram point under `at` (pixels) fixed.
    fn zoom_at(&mut self, factor: f64, at: Vec2) {
        let dpr = self.scale_factor();
        let zoom = (self.camera.zoom * factor).clamp(0.02 * dpr, 16.0 * dpr);
        let k = zoom / self.camera.zoom;
        self.camera.offset = at - (at - self.camera.offset) * k;
        self.camera.zoom = zoom;
        self.redraw();
    }

    /// Window pixels → diagram coordinates.
    fn to_diagram(&self, p: Vec2) -> Option<Point> {
        let b = &self.built.as_ref()?.diagram.bounds;
        let s = (p - self.camera.offset) / self.camera.zoom;
        Some(Point::new(s.x as f32 + b.x, s.y as f32 + b.y))
    }

    fn node_at_cursor(&self) -> Option<NodeId> {
        let p = self.to_diagram(self.cursor)?;
        interact::hit(&self.built.as_ref()?.diagram, p).map(|n| n.id.clone())
    }

    // ---------------------------------------------------------------- edits

    /// Runs an edit and writes the result back to the file.
    fn edit<T>(&mut self, f: impl FnOnce(&mut Editor) -> Result<T, EditError>) -> Option<T> {
        let Some(editor) = &mut self.editor else {
            eprintln!("editing works on .rhizo/.kdl files; this one is read-only here");
            return None;
        };
        match f(editor) {
            Ok(v) => {
                self.save();
                self.redraw();
                Some(v)
            }
            Err(e) => {
                eprintln!("{e}");
                None
            }
        }
    }

    /// Atomic write: the watcher never sees a half-written file.
    fn save(&self) {
        let Some(editor) = &self.editor else { return };
        let name = self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let tmp = self.path.with_file_name(format!(".{name}.rhizo-tmp"));
        let result = std::fs::write(&tmp, editor.source()).and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(e) = result {
            eprintln!("cannot save {}: {e}", self.path.display());
        }
    }

    fn undo(&mut self, redo: bool) {
        let changed = self.editor.as_mut().is_some_and(|e| if redo { e.redo() } else { e.undo() });
        if changed {
            self.save();
        }
    }

    /// Adds a node next to (Enter) or under (Tab) the selection and starts
    /// typing its label.
    fn add(&mut self, child: bool) {
        let Some(sel) = self.selected.clone() else { return };
        let new = self.edit(|e| if child { e.add_child(&sel, NEW_LABEL) } else { e.add_after(&sel, NEW_LABEL) });
        if let Some(id) = new {
            self.selected = Some(id.clone());
            self.editing = Some(TextEdit { id, text: String::new(), created: true });
        }
    }

    fn start_rename(&mut self) {
        let Some(id) = self.selected.clone() else { return };
        let label = self.editor.as_ref().and_then(|e| e.document().node(&id)).map(|n| n.label.clone());
        if let Some(text) = label {
            self.editing = Some(TextEdit { id, text, created: false });
            self.redraw();
        }
    }

    /// Ends label editing. `keep` commits the text; otherwise a node made for
    /// this edit is removed again.
    fn finish_edit(&mut self, keep: bool) {
        let Some(te) = self.editing.take() else { return };
        let text = te.text.trim().to_string();
        if !keep || (te.created && text.is_empty()) {
            if te.created {
                self.undo(false);
                self.selected = None;
            }
        } else if !text.is_empty() {
            let unchanged =
                self.editor.as_ref().and_then(|e| e.document().node(&te.id)).is_some_and(|n| n.label == text);
            if !unchanged {
                self.edit(|e| e.rename(&te.id, &text));
            }
        }
        self.redraw();
    }

    fn delete_selected(&mut self) {
        let Some(id) = self.selected.clone() else { return };
        let parent = self.editor.as_ref().and_then(|e| e.document().node(&id)).and_then(|n| n.parent.clone());
        if self.edit(|e| e.delete(&id)).is_some() {
            self.selected = parent;
        }
    }

    fn navigate(&mut self, dir: Dir) {
        let (Some(sel), Some(b)) = (&self.selected, &self.built) else {
            // Nothing selected yet: start at the first root.
            self.selected = self.built.as_ref().and_then(|b| b.diagram.nodes.first()).map(|n| n.id.clone());
            self.redraw();
            return;
        };
        if let Some(next) = interact::nearest(&b.diagram, sel, dir) {
            self.selected = Some(next);
            self.redraw();
        }
    }

    fn run(&mut self, action: Action) {
        let center = self.size() / 2.0;
        match action {
            Action::AddChild => self.add(true),
            Action::AddSibling => self.add(false),
            Action::Rename => self.start_rename(),
            Action::Delete => self.delete_selected(),
            Action::Connect => self.connecting = self.selected.clone(),
            Action::Undo => self.undo(false),
            Action::Redo => self.undo(true),
            Action::Fit => self.fit(),
            Action::ZoomActual => {
                let dpr = self.scale_factor();
                self.zoom_at(dpr / self.camera.zoom, center);
            }
            Action::ZoomIn => self.zoom_at(1.25, center),
            Action::ZoomOut => self.zoom_at(0.8, center),
        }
        self.redraw();
    }

    // -------------------------------------------------------------- pointer

    fn press(&mut self) {
        if self.editing.is_some() {
            self.finish_edit(true);
        }
        if let Some(from) = self.connecting.take() {
            if let Some(to) = self.node_at_cursor().filter(|t| *t != from) {
                self.edit(|e| e.connect(&from, &to));
            }
            self.redraw();
            return;
        }
        let hit = self.to_diagram(self.cursor).and_then(|p| {
            let built = self.built.as_ref()?;
            let n = interact::hit(&built.diagram, p)?;
            Some((n.id.clone(), interact::drag_mode(&built.diagram, n), p))
        });
        match hit {
            Some((id, mode, p)) => {
                let gesture = if self.modifiers.alt_key() || mode == DragMode::Fixed {
                    Gesture::Connect
                } else {
                    Gesture::Drag(mode)
                };
                self.selected = Some(id.clone());
                self.node_drag = Some(NodeDrag { id, gesture, start: p, now: p, target: None, connect_to: None });
            }
            None => {
                self.selected = None;
                self.drag_from = Some(self.cursor);
            }
        }
        self.redraw();
    }

    fn drag_to(&mut self, p: Point) {
        let (Some(drag), Some(built)) = (&mut self.node_drag, &self.built) else { return };
        drag.now = p;
        match drag.gesture {
            Gesture::Drag(DragMode::Outline) => {
                drag.target = self
                    .editor
                    .as_ref()
                    .and_then(|ed| interact::drop_target(&built.diagram, ed.document(), &drag.id, p));
            }
            Gesture::Connect => {
                drag.connect_to = interact::hit(&built.diagram, p).map(|n| n.id.clone()).filter(|t| *t != drag.id);
            }
            Gesture::Drag(_) => {}
        }
        self.redraw();
    }

    fn release(&mut self) {
        self.drag_from = None;
        let Some(drag) = self.node_drag.take() else { return };
        let delta = Point::new(drag.now.x - drag.start.x, drag.now.y - drag.start.y);
        // A click, not a drag: selecting was all it did.
        if delta.x.hypot(delta.y) * (self.camera.zoom as f32) < 4.0 {
            self.redraw();
            return;
        }
        match drag.gesture {
            Gesture::Drag(DragMode::Move) => {
                let pos = match (&self.built, &self.editor) {
                    (Some(b), Some(ed)) => interact::free_position(&b.diagram, ed.document(), &drag.id, delta),
                    _ => None,
                };
                if let Some(pos) = pos {
                    self.edit(|e| e.move_node(&drag.id, pos.x, pos.y));
                }
            }
            Gesture::Drag(DragMode::Outline) => {
                if let Some(target) = drag.target {
                    self.edit(|e| e.reparent(&drag.id, &target));
                }
            }
            Gesture::Connect => {
                if let Some(to) = drag.connect_to {
                    self.edit(|e| e.connect(&drag.id, &to));
                }
            }
            Gesture::Drag(DragMode::Fixed) => {}
        }
        self.redraw();
    }

    // ------------------------------------------------------------- keyboard

    fn key(&mut self, event: &KeyEvent, event_loop: &ActiveEventLoop) {
        let cmd = self.modifiers.super_key() || self.modifiers.control_key();
        let key = event.logical_key.as_ref();
        // shortcut: plain key text only, no IME composition (CJK input, some
        // dead keys); add winit Ime events when labels need it.
        let typed = event.text.as_deref().filter(|t| t.chars().all(|c| !c.is_control())).map(str::to_string);

        // The palette takes every key while open.
        if let Some(p) = &mut self.palette {
            let matches = palette::filter(&p.query, self.selected.is_some());
            match key {
                Key::Named(NamedKey::Escape) => self.palette = None,
                Key::Named(NamedKey::ArrowDown) => p.index = (p.index + 1).min(matches.len().saturating_sub(1)),
                Key::Named(NamedKey::ArrowUp) => p.index = p.index.saturating_sub(1),
                Key::Named(NamedKey::Backspace) => {
                    p.query.pop();
                    p.index = 0;
                }
                Key::Named(NamedKey::Enter) => {
                    let action = matches.get(p.index).map(|c| c.action);
                    self.palette = None;
                    if let Some(a) = action {
                        self.run(a);
                    }
                }
                _ if !cmd => {
                    if let Some(t) = typed {
                        p.query.push_str(&t);
                        p.index = 0;
                    }
                }
                _ => {}
            }
            self.redraw();
            return;
        }

        // Typing a label.
        if let Some(te) = &mut self.editing {
            match key {
                Key::Named(NamedKey::Escape) => self.finish_edit(false),
                Key::Named(NamedKey::Enter) => self.finish_edit(true),
                Key::Named(NamedKey::Tab) => {
                    // Commit and keep outlining: Tab nests, like in the outline itself.
                    self.finish_edit(true);
                    self.add(true);
                }
                Key::Named(NamedKey::Backspace) => {
                    te.text.pop();
                }
                _ if !cmd => {
                    if let Some(t) = typed {
                        te.text.push_str(&t);
                    }
                }
                _ => {}
            }
            self.redraw();
            return;
        }

        let center = self.size() / 2.0;
        match key {
            Key::Character("k" | "K") if cmd => self.palette = Some(PaletteState { query: String::new(), index: 0 }),
            Key::Character("z" | "Z") if cmd => self.undo(self.modifiers.shift_key()),
            Key::Named(NamedKey::Escape) if cmd => event_loop.exit(),
            Key::Named(NamedKey::Escape) => {
                self.selected = None;
                self.node_drag = None;
                self.connecting = None;
            }
            Key::Named(NamedKey::Tab) => self.add(true),
            Key::Named(NamedKey::Enter) => self.add(false),
            Key::Named(NamedKey::F2) => self.start_rename(),
            Key::Named(NamedKey::Backspace | NamedKey::Delete) => self.delete_selected(),
            Key::Named(NamedKey::ArrowLeft) => self.navigate(Dir::Left),
            Key::Named(NamedKey::ArrowRight) => self.navigate(Dir::Right),
            Key::Named(NamedKey::ArrowUp) => self.navigate(Dir::Up),
            Key::Named(NamedKey::ArrowDown) => self.navigate(Dir::Down),
            Key::Character("f" | "F") => self.fit(),
            Key::Character("0") => {
                let dpr = self.scale_factor();
                self.zoom_at(dpr / self.camera.zoom, center);
            }
            Key::Character("=" | "+") => self.zoom_at(1.25, center),
            Key::Character("-") => self.zoom_at(0.8, center),
            _ => {}
        }
        self.redraw();
    }

    // -------------------------------------------------------------- drawing

    fn fonts(&mut self) -> &Fonts {
        self.fonts.get_or_insert_with(|| Fonts::system(rhizo_core::theme::FONT_STACK))
    }

    /// Selection, drag feedback and the label editor, in scene coordinates.
    fn overlay(&mut self) -> Option<Scene> {
        self.fonts();
        let built = self.built.as_ref()?;
        let b = built.diagram.bounds;
        let to_scene = |r: rhizo_core::Rect| {
            Rect::new((r.x - b.x) as f64, (r.y - b.y) as f64, (r.right() - b.x) as f64, (r.bottom() - b.y) as f64)
        };
        let node = |id: &NodeId| built.diagram.nodes.iter().find(|n| &n.id == id);
        let center = |id: &NodeId| node(id).map(|n| to_scene(n.rect).center());
        let accent = peniko_color(ACCENT);
        let mut s = Scene::new();
        let width = 2.0 / self.camera.zoom * self.scale_factor();
        let thick = Stroke::new(width * 2.0);
        if let Some(n) = self.selected.as_ref().and_then(node) {
            let r = RoundedRect::from_rect(to_scene(n.rect).inflate(4.0, 4.0), 10.0);
            s.stroke(&Stroke::new(width), Affine::IDENTITY, accent, None, &r);
        }
        // "Connect to…": a line from the source to the pointer.
        if let (Some(from), Some(p)) = (self.connecting.as_ref().and_then(center), self.to_diagram(self.cursor)) {
            let to = (p.x as f64 - b.x as f64, p.y as f64 - b.y as f64);
            s.stroke(
                &Stroke::new(width).with_dashes(0.0, [6.0 * width, 4.0 * width]),
                Affine::IDENTITY,
                accent,
                None,
                &Line::new(from, to),
            );
        }
        if let Some(drag) = &self.node_drag {
            let moved = (drag.now.x - drag.start.x).hypot(drag.now.y - drag.start.y) * (self.camera.zoom as f32) >= 4.0;
            match (drag.gesture, moved) {
                (Gesture::Connect, true) => {
                    if let Some(from) = center(&drag.id) {
                        let to = match drag.connect_to.as_ref().and_then(center) {
                            Some(c) => c,
                            None => ((drag.now.x - b.x) as f64, (drag.now.y - b.y) as f64).into(),
                        };
                        s.stroke(&thick, Affine::IDENTITY, accent, None, &Line::new(from, to));
                    }
                    if let Some(n) = drag.connect_to.as_ref().and_then(node) {
                        let rr = RoundedRect::from_rect(to_scene(n.rect).inflate(6.0, 6.0), 12.0);
                        s.stroke(&thick, Affine::IDENTITY, accent, None, &rr);
                    }
                }
                (Gesture::Drag(mode), true) if mode != DragMode::Fixed => {
                    if let Some(n) = node(&drag.id) {
                        let ghost = to_scene(n.rect.translate(drag.now.x - drag.start.x, drag.now.y - drag.start.y));
                        let r = RoundedRect::from_rect(ghost, 8.0);
                        let mut tint = ACCENT;
                        tint.a = 24;
                        s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(tint), None, &r);
                        s.stroke(
                            &Stroke::new(width).with_dashes(0.0, [6.0 * width, 4.0 * width]),
                            Affine::IDENTITY,
                            accent,
                            None,
                            &r,
                        );
                    }
                    if let Some(target) = &drag.target {
                        let (Placement::Child(t) | Placement::Before(t) | Placement::After(t)) = target;
                        if let Some(n) = node(t) {
                            let r = to_scene(n.rect);
                            let layout = built.diagram.frames.iter().find(|f| f.id == n.frame).map(|f| f.layout);
                            let vertical = self
                                .editor
                                .as_ref()
                                .zip(layout)
                                .is_some_and(|(ed, l)| interact::siblings_stack_vertically(ed.document(), n, l));
                            let gap = 5.0;
                            let line = match (target, vertical) {
                                (Placement::Child(_), _) => None,
                                (Placement::Before(_), true) => Some(Line::new((r.x0, r.y0 - gap), (r.x1, r.y0 - gap))),
                                (Placement::After(_), true) => Some(Line::new((r.x0, r.y1 + gap), (r.x1, r.y1 + gap))),
                                (Placement::Before(_), false) => {
                                    Some(Line::new((r.x0 - gap, r.y0), (r.x0 - gap, r.y1)))
                                }
                                (Placement::After(_), false) => Some(Line::new((r.x1 + gap, r.y0), (r.x1 + gap, r.y1))),
                            };
                            match line {
                                Some(l) => s.stroke(&thick, Affine::IDENTITY, accent, None, &l),
                                None => {
                                    let rr = RoundedRect::from_rect(r.inflate(6.0, 6.0), 12.0);
                                    s.stroke(&thick, Affine::IDENTITY, accent, None, &rr);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // The label editor sits on top of its node.
        if let Some(te) = &self.editing {
            if let Some(n) = node(&te.id) {
                let fonts = self.fonts.as_ref().expect("loaded above");
                let (shown, color) = if te.text.is_empty() && te.created {
                    (NEW_LABEL, n.text.mix(n.fill, 0.55))
                } else {
                    (te.text.as_str(), n.text)
                };
                let size = n.font_size;
                let text_w = fonts.shape(shown, size, n.font_weight).map_or(0.0, |s| s.width) as f64;
                let r = to_scene(n.rect);
                let w = r.width().max(text_w + 2.0 * size as f64);
                let rect = Rect::from_center_size(r.center(), (w, r.height().max(size as f64 * 2.0)));
                let rr = RoundedRect::from_rect(rect, 8.0);
                s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(n.fill), None, &rr);
                s.stroke(&thick, Affine::IDENTITY, accent, None, &rr);
                let c = rect.center();
                let x = c.x as f32 - text_w as f32 / 2.0;
                let baseline = c.y as f32 + size * 0.35;
                let drawn = draw::text(
                    &mut s,
                    fonts,
                    &mut self.glyphs,
                    Affine::IDENTITY,
                    shown,
                    x,
                    baseline,
                    size,
                    n.font_weight,
                    color,
                );
                // Caret after the typed text (before the placeholder).
                let cx = if te.text.is_empty() { x as f64 } else { (x + drawn) as f64 + 1.0 };
                let caret =
                    Line::new((cx, baseline as f64 - size as f64 * 0.95), (cx, baseline as f64 + size as f64 * 0.2));
                s.stroke(&Stroke::new(width.max(1.2)), Affine::IDENTITY, accent, None, &caret);
            }
        }
        Some(s)
    }

    /// The ⌘K palette, in window pixels.
    fn palette_scene(&mut self) -> Option<Scene> {
        let query = self.palette.as_ref()?.query.clone();
        let index = self.palette.as_ref()?.index;
        let bg = self.built.as_ref().map_or(Color::rgb(255, 255, 255), |b| b.background);
        let fg = bg.readable_on(Color::hex(0xf4f4f5), Color::hex(0x18181b));
        let dpr = self.scale_factor();
        let size = self.size();
        let matches = palette::filter(&query, self.selected.is_some());
        let fonts = self.fonts.get_or_insert_with(|| Fonts::system(rhizo_core::theme::FONT_STACK));
        let mut s = Scene::new();
        let (w, row) = ((520.0 * dpr).min(size.x - 32.0 * dpr), 34.0 * dpr);
        let rows = matches.len().min(8);
        let x0 = (size.x - w) / 2.0;
        let y0 = 80.0 * dpr;
        let panel = Rect::new(x0, y0, x0 + w, y0 + row * (rows as f64 + 1.0) + 12.0 * dpr);
        let rr = RoundedRect::from_rect(panel, 12.0 * dpr);
        let mut shadow = Color::rgb(0, 0, 0);
        shadow.a = 40;
        s.fill(Fill::NonZero, Affine::translate((0.0, 6.0 * dpr)), peniko_color(shadow), None, &rr);
        s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg), None, &rr);
        s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(fg.mix(bg, 0.8)), None, &rr);
        let size_pt = 15.0 * dpr as f32;
        let pad = 16.0 * dpr;
        let base = |i: f64| (y0 + 6.0 * dpr + row * (i + 0.5)) as f32 + size_pt * 0.35;
        let (shown, color) =
            if query.is_empty() { ("Type a command…", fg.mix(bg, 0.5)) } else { (query.as_str(), fg) };
        let drawn = draw::text(
            &mut s,
            fonts,
            &mut self.glyphs,
            Affine::IDENTITY,
            shown,
            (x0 + pad) as f32,
            base(0.0),
            size_pt,
            500,
            color,
        );
        let cx = x0 + pad + if query.is_empty() { 0.0 } else { drawn as f64 + 1.0 };
        s.stroke(
            &Stroke::new(1.5 * dpr),
            Affine::IDENTITY,
            peniko_color(ACCENT),
            None,
            &Line::new((cx, base(0.0) as f64 - size_pt as f64), (cx, base(0.0) as f64 + 3.0 * dpr)),
        );
        for (i, c) in matches.iter().take(rows).enumerate() {
            let y = i as f64 + 1.0;
            if i == index {
                let mut hl = ACCENT;
                hl.a = 36;
                let r = Rect::new(
                    x0 + 6.0 * dpr,
                    y0 + 6.0 * dpr + row * y,
                    x0 + w - 6.0 * dpr,
                    y0 + 6.0 * dpr + row * (y + 1.0),
                );
                s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(hl), None, &RoundedRect::from_rect(r, 8.0 * dpr));
            }
            draw::text(
                &mut s,
                fonts,
                &mut self.glyphs,
                Affine::IDENTITY,
                c.name,
                (x0 + pad) as f32,
                base(y),
                size_pt,
                400,
                fg,
            );
            let kw = fonts.shape(c.keys, size_pt * 0.85, 400).map_or(0.0, |k| k.width) as f64;
            draw::text(
                &mut s,
                fonts,
                &mut self.glyphs,
                Affine::IDENTITY,
                c.keys,
                (x0 + w - pad - kw) as f32,
                base(y),
                size_pt * 0.85,
                400,
                fg.mix(bg, 0.45),
            );
        }
        Some(s)
    }

    fn render(&mut self) {
        let overlay = self.overlay();
        let palette = self.palette_scene();
        let (Some(gpu), Some(built)) = (&mut self.gpu, &self.built) else { return };
        let (w, h) = (gpu.surface.config.width, gpu.surface.config.height);
        let device = &self.cx.devices[gpu.surface.dev_id];
        let mut scene = Scene::new();
        let cam = self.camera;
        let view = Affine::translate(cam.offset) * Affine::scale(cam.zoom);
        scene.append(&built.scene, Some(view));
        if let Some(overlay) = &overlay {
            scene.append(overlay, Some(view));
        }
        if let Some(p) = &palette {
            scene.append(p, None);
        }
        let params = RenderParams {
            base_color: peniko_color(built.background),
            width: w,
            height: h,
            antialiasing_method: AaConfig::Area,
        };
        if let Err(e) =
            gpu.renderer.render_to_texture(&device.device, &device.queue, &scene, &gpu.surface.target_view, &params)
        {
            eprintln!("render failed: {e}");
            return;
        }
        let frame = match gpu.surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                self.cx.configure_surface(&gpu.surface);
                f
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            _ => {
                // Outdated or lost: reconfigure and try again on the next frame.
                self.cx.configure_surface(&gpu.surface);
                gpu.window.request_redraw();
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.surface.blitter.copy(&device.device, &mut encoder, &gpu.surface.target_view, &view);
        device.queue.submit([encoder.finish()]);
        device.queue.present(frame);
    }

    fn set_built(&mut self, built: Built, took: Duration) {
        let first = self.built.is_none();
        eprintln!("{} nodes in {:.0?}", built.nodes, took);
        if let Some(w) = self.window() {
            let name = self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            w.set_title(&match &built.title {
                Some(t) => format!("{t} · {name}"),
                None => name,
            });
        }
        // Our own saves come back here too; keep the editor (and its undo
        // history) when the file is what it already holds.
        if built.syntax != Syntax::Kdl {
            self.editor = None;
        } else if self.editor.as_ref().map(Editor::source) != Some(built.source.as_str()) {
            self.editor = Some(Editor::open(built.source.clone()));
        }
        if let Some(id) = &self.selected {
            let gone = self.editor.as_ref().is_some_and(|e| e.document().node(id).is_none());
            if gone {
                self.selected = None;
                self.editing = None;
            }
        }
        self.built = Some(built);
        if first { self.fit() } else { self.redraw() }
    }
}

impl ApplicationHandler<Reload> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("rhizo")
            .with_inner_size(winit::dpi::LogicalSize::new(1200.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let size = window.inner_size();
        let surface = pollster::block_on(self.cx.create_surface(
            window.clone(),
            size.width.max(1),
            size.height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .expect("GPU surface");
        let renderer =
            Renderer::new(&self.cx.devices[surface.dev_id].device, RendererOptions::default()).expect("vello renderer");
        self.gpu = Some(Gpu { window, surface, renderer });
        if self.built.is_some() {
            self.fit();
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: Reload) {
        match event {
            Reload::Built(b, took) => self.set_built(*b, took),
            Reload::Failed(msg) => eprintln!("{msg}"),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    if size.width > 0 && size.height > 0 {
                        self.cx.resize_surface(&mut gpu.surface, size.width, size.height);
                        gpu.window.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let p = Vec2::new(position.x, position.y);
                self.cursor = p;
                if self.node_drag.is_some() {
                    if let Some(d) = self.to_diagram(p) {
                        self.drag_to(d);
                    }
                } else if let Some(from) = self.drag_from {
                    self.camera.offset += p - from;
                    self.drag_from = Some(p);
                    self.redraw();
                } else if self.connecting.is_some() {
                    self.redraw();
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => {
                    let now = Instant::now();
                    let double = self.last_click.is_some_and(|(t, at)| {
                        now - t < Duration::from_millis(350) && (at - self.cursor).hypot() < 6.0
                    });
                    self.last_click = Some((now, self.cursor));
                    match (double, self.node_at_cursor()) {
                        (true, Some(_)) => {
                            self.node_drag = None;
                            self.start_rename();
                        }
                        (true, None) => self.fit(),
                        (false, _) => self.press(),
                    }
                }
                ElementState::Released => self.release(),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let zooming = self.modifiers.super_key() || self.modifiers.control_key();
                let d = match delta {
                    MouseScrollDelta::LineDelta(x, y) => Vec2::new(x as f64, y as f64) * 40.0 * self.scale_factor(),
                    MouseScrollDelta::PixelDelta(p) => Vec2::new(p.x, p.y),
                };
                if zooming {
                    self.zoom_at((d.y * 0.004).exp(), self.cursor);
                } else {
                    self.camera.offset += d;
                    self.redraw();
                }
            }
            WindowEvent::PinchGesture { delta, .. } => self.zoom_at(1.0 + delta, self.cursor),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key(&event, event_loop)
            }
            _ => {}
        }
    }
}

/// Polls the file and rebuilds off the UI thread.
fn watch(path: PathBuf, theme: Option<String>, proxy: EventLoopProxy<Reload>) {
    std::thread::spawn(move || {
        let mut engine = Engine::default();
        let mut last: Option<SystemTime> = None;
        loop {
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            if modified.is_some() && modified != last {
                last = modified;
                let started = Instant::now();
                let event = match engine.build_file(&path, theme.as_deref()) {
                    Ok(b) => Reload::Built(Box::new(b), started.elapsed()),
                    Err(msg) => Reload::Failed(msg),
                };
                if proxy.send_event(event).is_err() {
                    return; // window closed
                }
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    });
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut theme = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--theme" => theme = args.next(),
            "-h" | "--help" => {
                println!("usage: rhizo-view FILE [--theme {}]", rhizo_app::theme_names().join("|"));
                println!(
                    "view:  scroll pans · cmd/ctrl+scroll or pinch zooms · F or double-click empty space fits · 0 = 100%"
                );
                println!("edit:  click selects · arrows move the selection · Tab adds a child · Enter a sibling");
                println!("       F2 or double-click renames · Backspace deletes · drag moves or re-parents");
                println!("       alt+drag (any drag in flowcharts) connects · cmd+K commands · cmd+Z / shift+cmd+Z");
                return;
            }
            _ if path.is_none() => path = Some(PathBuf::from(a)),
            other => {
                eprintln!("unexpected argument `{other}` (see --help)");
                std::process::exit(2);
            }
        }
    }
    let Some(path) = path else {
        eprintln!("usage: rhizo-view FILE [--theme light|dark|paper]");
        std::process::exit(2);
    };
    if let Some(t) = &theme {
        if !rhizo_app::theme_names().contains(&t.as_str()) {
            eprintln!("unknown theme `{t}` ({})", rhizo_app::theme_names().join(", "));
            std::process::exit(2);
        }
    }
    let event_loop = EventLoop::<Reload>::with_user_event().build().expect("event loop");
    watch(path.clone(), theme, event_loop.create_proxy());
    let mut app = App {
        path,
        cx: RenderContext::new(),
        gpu: None,
        built: None,
        camera: Camera { zoom: 1.0, offset: Vec2::ZERO },
        cursor: Vec2::ZERO,
        drag_from: None,
        last_click: None,
        modifiers: ModifiersState::empty(),
        editor: None,
        selected: None,
        node_drag: None,
        editing: None,
        palette: None,
        connecting: None,
        fonts: None,
        glyphs: FontCache::default(),
    };
    event_loop.run_app(&mut app).expect("event loop");
}
