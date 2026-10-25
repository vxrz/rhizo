//! The event loop's entry points: each event goes to the part it is for.

use std::time::Instant;

use rhizo_app::vello::kurbo::Vec2;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::WindowId;

use super::App;
use super::workers::Event;

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        self.create_gpu(event_loop);
        #[cfg(target_os = "macos")]
        {
            self.menubar = Some(super::menubar::MenuBar::new(self.proxy.clone()));
        }
        if self.built.is_some() {
            self.fit(false);
        }
        self.update_title();
        if let Some(p) = self.session.pending_open.take() {
            self.open(p);
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: Event) {
        match event {
            Event::Reload(r) => self.reloaded(r),
            Event::Analyzed(a) => self.analyzed(*a),
            Event::Scan(s) => self.scanned(*s),
            Event::Adopt(u) => self.adopt_update(u),
            #[cfg(target_os = "macos")]
            Event::Menu(id) => self.menu_chosen(&id),
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.flush();
    }

    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quitting {
            self.flush();
            return event_loop.exit();
        }
        if self.pane.sync.take_due(Instant::now()) {
            self.analyze();
        }
        #[cfg(target_os = "macos")]
        self.refresh_menubar();
        self.allow_ime();
        event_loop.set_control_flow(match self.next_wake() {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.flush();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => self.resized(size),
            WindowEvent::RedrawRequested => {
                self.render();
                return;
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                return;
            }
            WindowEvent::CursorMoved { position, .. } => self.cursor_moved(Vec2::new(position.x, position.y)),
            WindowEvent::DroppedFile(path) => self.open(path),
            WindowEvent::ThemeChanged(_) => self.redraw(),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                if !self.left_button(state) {
                    return;
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                self.context_menu();
                return;
            }
            WindowEvent::MouseInput { state, button: MouseButton::Middle, .. } => self.middle_button(state),
            WindowEvent::Ime(ime) => self.ime(ime),
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            WindowEvent::PinchGesture { .. } if !(self.canvas().x0..=self.canvas().x1).contains(&self.cursor.x) => {}
            WindowEvent::PinchGesture { delta, .. } => self.zoom_at(1.0 + delta, self.cursor),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key(&event, event_loop)
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) && self.space {
                    self.space = false;
                    self.interaction.drag_from = None;
                    self.update_pointer();
                }
                return;
            }
            WindowEvent::Focused(false) => {
                self.space = false;
                return;
            }
            WindowEvent::CursorLeft { .. } => {
                if self.interaction.hovered.take().is_some() {
                    self.redraw();
                }
                return;
            }
            _ => return,
        }
        self.update_pointer();
        self.selection_to_text();
    }
}
