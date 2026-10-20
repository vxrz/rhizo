//! The rhizo viewer core: document → vello scene, plus offscreen rendering.
//!
//! The scene is built from the same SVG `rhizo render` writes (layout →
//! semantic SVG → usvg → vello), so the window shows exactly what the
//! exports show, text included.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use rhizo_core::{Color, Theme};
use rhizo_lang::{Report, Syntax};
use rhizo_layout::Diagram;
use rhizo_render::Fonts;
use vello::wgpu;
use vello_svg::usvg;
pub use vello_svg::vello;

/// A laid-out document, ready to draw.
pub struct Built {
    pub scene: vello::Scene,
    /// The laid-out diagram, moved so its bounds start at the origin.
    pub diagram: Diagram,
    /// Size of the drawing in diagram units (CSS px).
    pub width: f64,
    pub height: f64,
    pub background: Color,
    pub title: Option<String>,
    pub nodes: usize,
}

/// Loads documents and builds scenes. Keeps font databases between reloads.
pub struct Engine {
    /// Layout metrics, keyed by the theme's font stack.
    measure: HashMap<&'static str, Fonts>,
    // shortcut: vello_svg needs usvg 0.48 while rhizo-render uses 0.47, so
    // system fonts load twice; drop when rhizo-render moves to 0.48.
    svg_fonts: Arc<usvg::fontdb::Database>,
}

impl Default for Engine {
    fn default() -> Self {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Engine { measure: HashMap::new(), svg_fonts: Arc::new(db) }
    }
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
        let fonts = self.measure.entry(theme.font_family).or_insert_with(|| Fonts::system(theme.font_family));
        let mut diagram = rhizo_render::layout(&doc, &theme, fonts);
        move_to_origin(&mut diagram);
        let svg = rhizo_render::to_svg(&diagram, &Default::default());
        // shortcut: usvg shapes every label again (~350 ms of a 5,000-node reload);
        // draw the Diagram straight into vello with a glyph cache to get under 100 ms.
        let opt = usvg::Options { fontdb: self.svg_fonts.clone(), ..Default::default() };
        let tree = usvg::Tree::from_str(&svg, &opt).map_err(|e| format!("internal SVG error: {e}"))?;
        let mut scene = vello::Scene::new();
        vello_svg::append_tree(&mut scene, &tree);
        Ok(Built {
            scene,
            width: tree.size().width() as f64,
            height: tree.size().height() as f64,
            background: theme.background,
            title: doc.title.clone(),
            nodes: doc.nodes.len(),
            diagram,
        })
    }
}

/// shortcut: vello_svg 0.11 applies text transforms twice, so the SVG must be
/// transform-free: a viewBox at 0,0 and no transformed groups. Drop when
/// vello_svg fixes nested absolute transforms for text.
fn move_to_origin(d: &mut Diagram) {
    let (dx, dy) = (-d.bounds.x, -d.bounds.y);
    d.bounds = d.bounds.translate(dx, dy);
    for f in &mut d.frames {
        f.rect = f.rect.translate(dx, dy);
    }
    for n in &mut d.nodes {
        n.rect = n.rect.translate(dx, dy);
    }
    for e in &mut d.edges {
        e.path = e.path.map(|p| p.offset(dx, dy));
        if let Some(l) = &mut e.label {
            l.center = l.center.offset(dx, dy);
        }
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
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let start = y * row as usize;
        pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    Some((w, h, pixels))
}
