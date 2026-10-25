//! The rhizo viewer core: document → vello scene, plus offscreen rendering.
//!
//! Layout runs with the same font metrics the scene draws with, and
//! [`draw`] mirrors the SVG writer, so the window matches the exports.

pub mod about;
pub mod adopt;
pub mod chrome;
pub mod context;
pub mod draw;
pub mod field;
pub mod files;
pub mod gallery;
pub mod inspector;
pub mod interact;
pub mod learn;
pub mod ops;
pub mod palette;
pub mod parity;
pub mod project;
pub mod sidebar;
pub mod textpane;
pub mod tween;
pub mod welcome;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrome::Problems;
use rhizo_core::{Color, Document, NodeId, Theme};
use rhizo_interop::Format;
use rhizo_kit::Kits;
use rhizo_lang::{Report, Syntax};
use rhizo_layout::Diagram;
use rhizo_project::Project;
use rhizo_render::Fonts;
pub use vello;
use vello::wgpu;

/// A laid-out document, ready to draw.
pub struct Built {
    pub scene: vello::Scene,
    /// The laid-out diagram, in its own coordinates (scene = diagram − bounds origin).
    pub diagram: Diagram,
    /// Size of the drawing in diagram units (CSS px).
    pub width: f64,
    pub height: f64,
    pub background: Color,
    /// The document as drawn: references resolved, kit looks applied.
    /// Exports start from it, so they match the window.
    pub document: Document,
    pub title: Option<String>,
    pub nodes: usize,
    /// The source the diagram was built from, and its syntax.
    pub source: String,
    pub syntax: Syntax,
    /// The fonts the scene was drawn with, for redrawing it (animations).
    pub fonts: Arc<Fonts>,
    /// Problems the document loaded with, if any: warnings, and errors from
    /// kits or references that do not stop it from being drawn.
    pub warnings: Option<Problems>,
    /// Other files this one imports; a change to any of them redraws it.
    pub deps: Vec<PathBuf>,
    /// References in this file (`model:order`) and what they point at:
    /// the file and the node's id there.
    pub references: HashMap<NodeId, (PathBuf, NodeId)>,
}

/// Why a document could not be built.
#[derive(Debug)]
pub struct Failed {
    /// The full miette report, for the terminal.
    pub report: String,
    pub problems: Problems,
}

/// Loads documents and builds scenes. Keeps fonts between reloads.
#[derive(Default)]
pub struct Engine {
    /// Metrics and glyphs, keyed by the theme's font stack.
    fonts: HashMap<&'static str, Arc<Fonts>>,
    glyphs: draw::FontCache,
    /// Loaded on first use.
    kits: Option<Kits>,
    /// The layout shown before an edit: builds stay close to it, so what the
    /// edit did not touch keeps its place. `None` lays out from scratch, as
    /// exports do.
    pub prev: Option<Diagram>,
}

impl Engine {
    /// Reads, lays out and draws `path`. On problems returns the rendered
    /// diagnostics instead.
    pub fn build_file(&mut self, path: &Path, theme: Option<&str>) -> Result<Built, Failed> {
        let src = std::fs::read_to_string(path).map_err(|e| {
            let report = format!("cannot read {}: {e}", path.display());
            Failed { problems: Problems::single(&report), report }
        })?;
        self.build_at(Some(path), &path.display().to_string(), &src, files::syntax_of(path), theme)
    }

    /// Builds text that is not a file: no imports are followed.
    pub fn build(&mut self, name: &str, src: &str, syntax: Syntax, theme: Option<&str>) -> Result<Built, Failed> {
        self.build_at(None, name, src, syntax, theme)
    }

    fn build_at(
        &mut self,
        path: Option<&Path>,
        name: &str,
        src: &str,
        syntax: Syntax,
        theme: Option<&str>,
    ) -> Result<Built, Failed> {
        let format = match syntax {
            Syntax::Kdl => Format::Rhizo,
            Syntax::Json => Format::Json,
            Syntax::Yaml => Format::Yaml,
        };
        let p = match path {
            Some(path) => Project::with_text(path, src.to_string(), format),
            None => Project::open_text(name, src, format),
        };
        let file = &p.files[0];
        if file.has_errors() {
            let errs = file.load.clone();
            return Err(Failed {
                problems: Problems::new(src, &errs).expect("an error"),
                report: format!("{:?}", miette::Report::new(Report::new(name, src, errs))),
            });
        }
        let kits = self.kits.get_or_insert_with(Kits::builtin);
        // Problems include broken references, and kit rules on edges that cross files.
        let warnings = p.check(kits).swap_remove(0);
        let doc = p.drawable(0, kits);
        let deps = p.files[1..].iter().map(|f| f.path.clone()).collect();
        let references = doc
            .nodes
            .keys()
            .filter_map(|id| p.target_of(0, id).map(|(f, t)| (id.clone(), (p.files[f].path.clone(), t.clone()))))
            .collect();
        let problems = Problems::new(src, &warnings);
        if !warnings.is_empty() {
            eprintln!("{:?}", miette::Report::new(Report::new(name, src, warnings)));
        }
        let theme = rhizo_render::theme_for(&doc, theme);
        let fonts = self.fonts.entry(theme.font_family).or_insert_with(|| Arc::new(Fonts::system(theme.font_family)));
        let diagram = match &self.prev {
            Some(prev) => rhizo_layout::stable::layout_from(&doc, &theme, fonts.as_ref(), prev),
            None => rhizo_render::layout(&doc, &theme, fonts.as_ref()),
        };
        let scene = draw::scene(&diagram, fonts, &mut self.glyphs);
        Ok(Built {
            scene,
            width: diagram.bounds.width as f64,
            height: diagram.bounds.height as f64,
            background: theme.background,
            title: doc.title.clone(),
            nodes: doc.nodes.len(),
            document: doc,
            diagram,
            source: src.to_string(),
            syntax,
            fonts: fonts.clone(),
            warnings: problems,
            deps,
            references,
        })
    }
}

pub fn theme_names() -> &'static [&'static str] {
    &Theme::NAMES
}

pub fn peniko_color(c: Color) -> vello::peniko::Color {
    vello::peniko::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Renders `built` at `scale` into RGBA8 pixels, without a window.
/// Returns `None` when no GPU adapter is available (headless CI).
pub fn render_offscreen(built: &Built, scale: f64) -> Option<(u32, u32, Vec<u8>)> {
    let mut scene = vello::Scene::new();
    scene.append(&built.scene, Some(vello::kurbo::Affine::scale(scale)));
    let (w, h) = ((built.width * scale).ceil() as u32, (built.height * scale).ceil() as u32);
    render_scene_offscreen(&scene, w, h, built.background).map(|px| (w, h, px))
}

/// Renders any scene into a `w`×`h` RGBA8 image on `background`.
pub fn render_scene_offscreen(scene: &vello::Scene, w: u32, h: u32, background: Color) -> Option<Vec<u8>> {
    let mut cx = vello::util::RenderContext::new();
    let dev = pollster::block_on(cx.device(None))?;
    let (device, queue) = (&cx.devices[dev].device, &cx.devices[dev].queue);
    let mut renderer = vello::Renderer::new(device, vello::RendererOptions::default()).ok()?;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rhizo offscreen"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let params = vello::RenderParams {
        base_color: peniko_color(background),
        width: w,
        height: h,
        antialiasing_method: vello::AaConfig::Area,
    };
    renderer.render_to_texture(device, queue, scene, &view, &params).ok()?;

    // Rows in a copy must be 256-byte aligned.
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rhizo readback"),
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
    rx.recv().ok()?.ok()?;
    let data = slice.get_mapped_range().ok()?;
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let start = y * row as usize;
        pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    Some(pixels)
}
