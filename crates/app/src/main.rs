//! `rhizo-view FILE [--theme light|dark|paper]`: a live window onto a rhizo
//! document. Edit the file anywhere and the window follows; select and drag
//! nodes in the window and the file follows (KDL files only).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use rhizo_app::interact::{self, DragMode};
use rhizo_app::vello::kurbo::{Affine, Line, Rect, RoundedRect, Stroke, Vec2};
use rhizo_app::vello::peniko::Fill;
use rhizo_app::vello::util::{RenderContext, RenderSurface};
use rhizo_app::vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene, wgpu};
use rhizo_app::{Built, Engine, peniko_color};
use rhizo_core::{Color, NodeId, Point};
use rhizo_lang::{EditError, Editor, Placement, Syntax};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const MARGIN: f64 = 24.0;
/// Selection and drop indicators: the brand indigo.
const ACCENT: Color = Color::hex(0x6366f1);

struct NodeDrag {
    id: NodeId,
    mode: DragMode,
    start: Point,
    now: Point,
    target: Option<Placement>,
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

    /// Runs an edit and writes the result back to the file.
    fn edit(&mut self, f: impl FnOnce(&mut Editor) -> Result<(), EditError>) {
        let Some(editor) = &mut self.editor else {
            eprintln!("editing works on .rhizo/.kdl files; this one is read-only here");
            return;
        };
        match f(editor) {
            Ok(()) => self.save(),
            Err(e) => eprintln!("{e}"),
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

    fn press(&mut self) {
        let hit = self.to_diagram(self.cursor).and_then(|p| {
            let built = self.built.as_ref()?;
            let n = interact::hit(&built.diagram, p)?;
            Some((n.id.clone(), interact::drag_mode(&built.diagram, n), p))
        });
        match hit {
            Some((id, mode, p)) => {
                self.selected = Some(id.clone());
                self.node_drag = Some(NodeDrag { id, mode, start: p, now: p, target: None });
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
        drag.target = match (drag.mode, &self.editor) {
            (DragMode::Outline, Some(ed)) => interact::drop_target(&built.diagram, ed.document(), &drag.id, p),
            _ => None,
        };
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
        match drag.mode {
            DragMode::Move => {
                let pos = match (&self.built, &self.editor) {
                    (Some(b), Some(ed)) => interact::free_position(&b.diagram, ed.document(), &drag.id, delta),
                    _ => None,
                };
                if let Some(pos) = pos {
                    self.edit(|e| e.move_node(&drag.id, pos.x, pos.y));
                }
            }
            DragMode::Outline => {
                if let Some(target) = drag.target {
                    self.edit(|e| e.reparent(&drag.id, &target));
                }
            }
            DragMode::Fixed => {}
        }
        self.redraw();
    }

    /// Selection, drag ghost and drop indicator, in scene coordinates.
    fn overlay(&self) -> Option<Scene> {
        let built = self.built.as_ref()?;
        let b = built.diagram.bounds;
        let to_scene = |r: rhizo_core::Rect| {
            Rect::new((r.x - b.x) as f64, (r.y - b.y) as f64, (r.right() - b.x) as f64, (r.bottom() - b.y) as f64)
        };
        let node = |id: &NodeId| built.diagram.nodes.iter().find(|n| &n.id == id);
        let accent = peniko_color(ACCENT);
        let mut s = Scene::new();
        let width = 2.0 / self.camera.zoom * self.scale_factor();
        if let Some(n) = self.selected.as_ref().and_then(node) {
            let r = RoundedRect::from_rect(to_scene(n.rect).inflate(4.0, 4.0), 10.0);
            s.stroke(&Stroke::new(width), Affine::IDENTITY, accent, None, &r);
        }
        if let Some(drag) = &self.node_drag {
            let moved = (drag.now.x - drag.start.x).hypot(drag.now.y - drag.start.y) * (self.camera.zoom as f32) >= 4.0;
            if let (Some(n), true, false) = (node(&drag.id), moved, drag.mode == DragMode::Fixed) {
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
                    let vertical = self.editor.as_ref().is_some_and(|ed| {
                        interact::siblings_stack_vertically(
                            ed.document(),
                            n,
                            built
                                .diagram
                                .frames
                                .iter()
                                .find(|f| f.id == n.frame)
                                .map_or(rhizo_core::LayoutKind::Mindmap, |f| f.layout),
                        )
                    });
                    let gap = 5.0;
                    let thick = Stroke::new(width * 2.0);
                    match target {
                        Placement::Child(_) => {
                            let rr = RoundedRect::from_rect(r.inflate(6.0, 6.0), 12.0);
                            s.stroke(&thick, Affine::IDENTITY, accent, None, &rr);
                        }
                        Placement::Before(_) if vertical => {
                            s.stroke(
                                &thick,
                                Affine::IDENTITY,
                                accent,
                                None,
                                &Line::new((r.x0, r.y0 - gap), (r.x1, r.y0 - gap)),
                            );
                        }
                        Placement::After(_) if vertical => {
                            s.stroke(
                                &thick,
                                Affine::IDENTITY,
                                accent,
                                None,
                                &Line::new((r.x0, r.y1 + gap), (r.x1, r.y1 + gap)),
                            );
                        }
                        Placement::Before(_) => {
                            s.stroke(
                                &thick,
                                Affine::IDENTITY,
                                accent,
                                None,
                                &Line::new((r.x0 - gap, r.y0), (r.x0 - gap, r.y1)),
                            );
                        }
                        Placement::After(_) => {
                            s.stroke(
                                &thick,
                                Affine::IDENTITY,
                                accent,
                                None,
                                &Line::new((r.x1 + gap, r.y0), (r.x1 + gap, r.y1)),
                            );
                        }
                    }
                }
            }
        }
        Some(s)
    }

    fn render(&mut self) {
        let overlay = self.overlay();
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
            if !built.diagram.nodes.iter().any(|n| &n.id == id) {
                self.selected = None;
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
                if self.node_drag.is_some() {
                    self.cursor = p;
                    if let Some(d) = self.to_diagram(p) {
                        self.drag_to(d);
                    }
                    return;
                }
                if let Some(from) = self.drag_from {
                    self.camera.offset += p - from;
                    self.drag_from = Some(p);
                    self.redraw();
                }
                self.cursor = p;
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => {
                    let now = Instant::now();
                    let double = self.last_click.is_some_and(|(t, at)| {
                        now - t < Duration::from_millis(350) && (at - self.cursor).hypot() < 6.0
                    });
                    self.last_click = Some((now, self.cursor));
                    let on_node = self
                        .to_diagram(self.cursor)
                        .zip(self.built.as_ref())
                        .is_some_and(|(p, b)| interact::hit(&b.diagram, p).is_some());
                    if double && !on_node {
                        self.fit();
                    } else {
                        self.press();
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
                let center = self.size() / 2.0;
                match event.logical_key.as_ref() {
                    Key::Character("f" | "F") => self.fit(),
                    Key::Character("0") => {
                        let dpr = self.scale_factor();
                        self.zoom_at(dpr / self.camera.zoom, center);
                    }
                    Key::Character("=" | "+") => self.zoom_at(1.25, center),
                    Key::Character("-") => self.zoom_at(0.8, center),
                    Key::Named(NamedKey::Escape) if self.modifiers.super_key() => event_loop.exit(),
                    Key::Named(NamedKey::Escape) => {
                        self.selected = None;
                        self.node_drag = None;
                        self.redraw();
                    }
                    Key::Character("z" | "Z") if self.modifiers.super_key() || self.modifiers.control_key() => {
                        let redo = self.modifiers.shift_key();
                        let changed = self.editor.as_mut().is_some_and(|e| if redo { e.redo() } else { e.undo() });
                        if changed {
                            self.save();
                        }
                    }
                    _ => {}
                }
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
                println!("scroll to pan · cmd/ctrl+scroll or pinch to zoom · F or double-click to fit · 0 for 100%");
                println!(
                    "click selects · drag moves (free frames) or re-parents (mindmaps, trees) · cmd+Z / shift+cmd+Z"
                );
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
    };
    event_loop.run_app(&mut app).expect("event loop");
}
