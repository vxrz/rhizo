//! The rhizo viewer core: document → vello scene, plus offscreen rendering.
//!
//! Layout runs with the same font metrics the scene draws with, and
//! [`draw`] mirrors the SVG writer, so the window matches the exports.

pub mod draw;
pub mod interact;

use std::collections::HashMap;
use std::path::Path;

use rhizo_core::{Color, Theme};
use rhizo_lang::{Report, Syntax};
use rhizo_layout::Diagram;
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
    pub title: Option<String>,
    pub nodes: usize,
    /// The source the diagram was built from, and its syntax.
    pub source: String,
    pub syntax: Syntax,
}

/// Loads documents and builds scenes. Keeps fonts between reloads.
#[derive(Default)]
pub struct Engine {
    /// Metrics and glyphs, keyed by the theme's font stack.
    fonts: HashMap<&'static str, Fonts>,
    glyphs: draw::FontCache,
}

impl Engine {
    /// Reads, lays out and draws `path`. On problems returns the rendered
    /// diagnostics instead.
    pub fn build_file(&mut self, path: &Path, theme: Option<&str>) -> Result<Built, String> {
        let src = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let syntax = match path.extension().and_then(|e| e.to_str()) {
            Some("json") => Syntax::Json,
            Some("yaml" | "yml") => Syntax::Yaml,
            _ => Syntax::Kdl,
        };
        self.build(&path.display().to_string(), &src, syntax, theme)
    }

    pub fn build(&mut self, name: &str, src: &str, syntax: Syntax, theme: Option<&str>) -> Result<Built, String> {
        let loaded = rhizo_lang::load(src, syntax);
        let (doc, warnings) =
            loaded.into_result().map_err(|errs| format!("{:?}", miette::Report::new(Report::new(name, src, errs))))?;
        if !warnings.is_empty() {
            eprintln!("{:?}", miette::Report::new(Report::new(name, src, warnings)));
        }
        let theme = rhizo_render::theme_for(&doc, theme);
        let fonts = self.fonts.entry(theme.font_family).or_insert_with(|| Fonts::system(theme.font_family));
        let diagram = rhizo_render::layout(&doc, &theme, fonts);
        let scene = draw::scene(&diagram, fonts, &mut self.glyphs);
        Ok(Built {
            scene,
            width: diagram.bounds.width as f64,
            height: diagram.bounds.height as f64,
            background: theme.background,
            title: doc.title.clone(),
            nodes: doc.nodes.len(),
            diagram,
            source: src.to_string(),
            syntax,
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
    let mut cx = vello::util::RenderContext::new();
    let dev = pollster::block_on(cx.device(None))?;
    let (device, queue) = (&cx.devices[dev].device, &cx.devices[dev].queue);
    let (w, h) = ((built.width * scale).ceil() as u32, (built.height * scale).ceil() as u32);
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
    let mut scene = vello::Scene::new();
    scene.append(&built.scene, Some(vello::kurbo::Affine::scale(scale)));
    let params = vello::RenderParams {
        base_color: peniko_color(built.background),
        width: w,
        height: h,
        antialiasing_method: vello::AaConfig::Area,
    };
    renderer.render_to_texture(device, queue, &scene, &view, &params).ok()?;

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
    Some((w, h, pixels))
}
