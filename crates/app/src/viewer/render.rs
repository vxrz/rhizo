//! The GPU surface, and each frame: the diagram under the camera, then the
//! overlay, the text pane, the sidebar, the chrome and the palette on top.

use std::sync::Arc;

use rhizo_app::draw;
use rhizo_app::peniko_color;
use rhizo_app::vello::kurbo::Affine;
use rhizo_app::vello::util::RenderSurface;
use rhizo_app::vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene, wgpu};
use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use super::App;

pub struct Gpu {
    pub window: Arc<Window>,
    pub surface: RenderSurface<'static>,
    pub renderer: Renderer,
}

impl App {
    /// Opens the window and its GPU surface.
    pub fn create_gpu(&mut self, event_loop: &ActiveEventLoop) {
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
    }

    pub fn resized(&mut self, size: PhysicalSize<u32>) {
        if let Some(gpu) = &mut self.gpu
            && size.width > 0
            && size.height > 0
        {
            self.cx.resize_surface(&mut gpu.surface, size.width, size.height);
            gpu.window.request_redraw();
        }
    }

    pub fn render(&mut self) {
        let moving = self.tick();
        if moving.is_some() || self.viewport.camera_move.is_some() || self.chrome.fading() {
            self.redraw();
        }
        let moving_scene = match (&moving, &self.built) {
            (Some(d), Some(b)) => Some(draw::scene(d, &b.fonts, &mut self.glyphs)),
            _ => None,
        };
        let overlay = self.overlay(moving.as_ref());
        let pane = self.pane_scene();
        let side = self.side_scene();
        let inspector = self.inspector_scene();
        let focus = self.focus_scene();
        let band = self.band_scene();
        let chrome = self.chrome_scene();
        let palette = self.palette_scene();
        let menu = self.menu_scene();
        let tip = self.tip_scene();
        let report = self.adopt_scene();
        let about = self.about_scene();
        let learn = self.learn_scene();
        let empty = self.empty_scene();
        let gallery = self.gallery_scene();
        let background = self.background();
        let Some(gpu) = &mut self.gpu else { return };
        let (w, h) = (gpu.surface.config.width, gpu.surface.config.height);
        let device = &self.cx.devices[gpu.surface.dev_id];
        let mut scene = Scene::new();
        let cam = self.viewport.camera;
        let view = Affine::translate(cam.offset) * Affine::scale(cam.zoom);
        if let Some(built) = &self.built {
            scene.append(moving_scene.as_ref().unwrap_or(&built.scene), Some(view));
        }
        if let Some(overlay) = &overlay {
            scene.append(overlay, Some(view));
        }
        if let Some(pane) = &pane {
            scene.append(pane, None);
        }
        if let Some(side) = &side {
            scene.append(side, None);
        }
        if let Some(inspector) = &inspector {
            scene.append(inspector, None);
        }
        for s in [&band, &focus].into_iter().flatten() {
            scene.append(s, None);
        }
        for s in [&empty, &learn].into_iter().flatten() {
            scene.append(s, None);
        }
        scene.append(&chrome, None);
        for s in [&gallery, &palette, &menu, &report, &about, &tip].into_iter().flatten() {
            scene.append(s, None);
        }
        let params = RenderParams {
            base_color: peniko_color(background),
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
}
