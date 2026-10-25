//! The camera onto the diagram: where it is, where it is easing to, the
//! relayout in progress, and the math between window pixels and the diagram.

use std::time::Instant;

use rhizo_app::tween::{self, Camera};
use rhizo_app::vello::kurbo::{Rect, Vec2};
use rhizo_core::Point;
use rhizo_layout::Diagram;

use super::App;

const MARGIN: f64 = 24.0;

pub struct Viewport {
    pub camera: Camera,
    /// The camera easing from one view to another since the instant.
    pub camera_move: Option<(Camera, Camera, Instant)>,
    /// The previous layout (moved by the offset to match `built`) easing
    /// into the current one since the instant.
    pub relayout: Option<(Diagram, Point, Instant)>,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport { camera: Camera { zoom: 1.0, offset: Vec2::ZERO }, camera_move: None, relayout: None }
    }
}

impl Viewport {
    /// Where the camera is headed: the end of its move, or where it is.
    pub fn goal(&self) -> Camera {
        self.camera_move.map_or(self.camera, |(_, to, _)| to)
    }

    /// Puts the camera at `to`, or starts easing it there.
    pub fn set(&mut self, to: Camera, animate: bool) {
        if animate {
            self.camera_move = Some((self.camera, to, Instant::now()));
        } else {
            self.camera_move = None;
            self.camera = to;
        }
    }

    /// Pans by hand (drag or scroll): any move in progress stops.
    pub fn drag(&mut self, by: Vec2) {
        self.camera_move = None;
        self.camera.offset += by;
    }

    /// Pans the camera and both ends of a move in progress by `by` pixels.
    pub fn pan(&mut self, by: Vec2) {
        self.camera.offset += by;
        if let Some((from, to, _)) = &mut self.camera_move {
            from.offset += by;
            to.offset += by;
        }
    }

    /// The drawing's origin moved by `by` diagram units: pans so it stays
    /// where it was on screen, also for a move in progress.
    pub fn follow_origin(&mut self, by: Vec2) {
        self.camera.offset += by * self.camera.zoom;
        if let Some((from, to, _)) = &mut self.camera_move {
            from.offset += by * from.zoom;
            to.offset += by * to.zoom;
        }
    }

    /// Advances the camera move, steady around `center`.
    fn step(&mut self, center: Vec2) {
        if let Some((from, to, t0)) = self.camera_move {
            let t = progress(t0);
            self.camera = from.lerp(to, t as f64, center);
            if t >= 1.0 {
                self.camera_move = None;
            }
        }
    }

    /// Window pixels → diagram coordinates, for a diagram with `bounds`;
    /// `None` outside the `canvas` (left of it or below).
    pub fn to_diagram(&self, p: Vec2, canvas: Rect, bounds: &rhizo_core::Rect) -> Option<Point> {
        if p.y > canvas.y1 || p.x < canvas.x0 || p.x > canvas.x1 {
            return None;
        }
        let s = (p - self.camera.offset) / self.camera.zoom;
        Some(Point::new(s.x as f32 + bounds.x, s.y as f32 + bounds.y))
    }
}

/// How far along an animation that started at `t0` is, eased; 1 when done.
pub fn progress(t0: Instant) -> f32 {
    tween::ease_out(t0.elapsed().as_secs_f32() / tween::DURATION.as_secs_f32())
}

/// `cam` zoomed by `factor` (within limits), keeping the pixel `at` fixed.
pub fn zoomed(cam: Camera, factor: f64, at: Vec2, dpr: f64) -> Camera {
    let zoom = (cam.zoom * factor).clamp(0.02 * dpr, 16.0 * dpr);
    Camera { zoom, offset: at - (at - cam.offset) * (zoom / cam.zoom) }
}

/// The camera showing a `width` × `height` drawing whole and centred in
/// `canvas`, never magnifying past 100%.
pub fn fitted(canvas: Rect, width: f64, height: f64, dpr: f64) -> Camera {
    let size = canvas.size().to_vec2();
    let m = MARGIN * dpr;
    let zoom = ((size.x - 2.0 * m) / width).min((size.y - 2.0 * m) / height).min(dpr).max(0.01);
    let offset = canvas.origin().to_vec2() + (size - Vec2::new(width, height) * zoom) / 2.0;
    Camera { zoom, offset }
}

impl App {
    /// Fits the whole diagram, never magnifying past 100%.
    pub fn fit(&mut self, animate: bool) {
        let Some(b) = &self.built else { return };
        let cam = fitted(self.canvas(), b.width, b.height, self.scale_factor());
        self.move_camera(cam, animate);
    }

    pub fn move_camera(&mut self, to: Camera, animate: bool) {
        self.viewport.set(to, animate);
        self.redraw();
    }

    /// Zooms by `factor`, keeping the diagram point under `at` (pixels) fixed.
    /// Instant, for wheel and pinch; keys ease via [`App::zoom_by`].
    pub fn zoom_at(&mut self, factor: f64, at: Vec2) {
        let cam = zoomed(self.viewport.camera, factor, at, self.scale_factor());
        self.move_camera(cam, false);
    }

    /// Eases towards `factor` times the zoom the camera is heading for.
    pub fn zoom_by(&mut self, factor: impl FnOnce(f64) -> f64) {
        let goal = self.viewport.goal();
        let to = zoomed(goal, factor(goal.zoom), self.canvas().center().to_vec2(), self.scale_factor());
        self.move_camera(to, true);
    }

    /// Eases the selected node into view if it is (partly) outside.
    pub fn reveal_selection(&mut self) {
        let (Some(id), Some(b)) = (self.selected_node(), &self.built) else { return };
        let Some(n) = b.diagram.nodes.iter().find(|n| n.id == id) else { return };
        let goal = self.viewport.goal();
        let (o, r) = (b.diagram.bounds, n.rect);
        let px = |x: f32, y: f32| goal.offset + Vec2::new((x - o.x) as f64, (y - o.y) as f64) * goal.zoom;
        let canvas = self.canvas();
        let (p0, p1) =
            (px(r.x, r.y) - canvas.origin().to_vec2(), px(r.right(), r.bottom()) - canvas.origin().to_vec2());
        let size = canvas.size().to_vec2();
        let pan = tween::reveal(Rect::new(p0.x, p0.y, p1.x, p1.y), size, 48.0 * self.scale_factor());
        if pan != Vec2::ZERO {
            self.move_camera(Camera { offset: goal.offset + pan, ..goal }, true);
        }
    }

    /// Window pixels → diagram coordinates; `None` over the status bar or
    /// the text pane, so they do not click through.
    pub fn to_diagram(&self, p: Vec2) -> Option<Point> {
        let canvas = self.canvas();
        self.viewport.to_diagram(p, canvas, &self.built.as_ref()?.diagram.bounds)
    }

    /// Advances the camera move and the relayout; returns the diagram as it
    /// stands mid-relayout, if one is running.
    pub fn tick(&mut self) -> Option<Diagram> {
        let center = self.canvas().center().to_vec2();
        self.viewport.step(center);
        let (old, shift, t0) = self.viewport.relayout.as_ref()?;
        let t = progress(*t0);
        if t >= 1.0 {
            self.viewport.relayout = None;
            return None;
        }
        Some(tween::frame(old, &self.built.as_ref()?.diagram, *shift, t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).hypot() < 1e-9
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let cam = Camera { zoom: 1.5, offset: Vec2::new(-40.0, 25.0) };
        let at = Vec2::new(300.0, 200.0);
        let z = zoomed(cam, 2.0, at, 1.0);
        assert_eq!(z.zoom, 3.0);
        // The diagram point under `at` is the same before and after.
        assert!(close((at - cam.offset) / cam.zoom, (at - z.offset) / z.zoom));
    }

    #[test]
    fn zoom_stays_within_limits_per_pixel_ratio() {
        let cam = Camera { zoom: 1.0, offset: Vec2::ZERO };
        assert_eq!(zoomed(cam, 1000.0, Vec2::ZERO, 2.0).zoom, 32.0);
        assert_eq!(zoomed(cam, 0.0001, Vec2::ZERO, 2.0).zoom, 0.04);
    }

    #[test]
    fn fitting_centres_and_never_magnifies() {
        let canvas = Rect::new(100.0, 0.0, 1100.0, 800.0);
        // Small: 100% (the pixel ratio), centred.
        let cam = fitted(canvas, 200.0, 100.0, 2.0);
        assert_eq!(cam.zoom, 2.0);
        assert!(close(cam.offset, Vec2::new(100.0 + (1000.0 - 400.0) / 2.0, (800.0 - 200.0) / 2.0)));
        // Large: shrunk to fit inside the margins.
        let cam = fitted(canvas, 4000.0, 1000.0, 1.0);
        assert_eq!(cam.zoom, (1000.0 - 48.0) / 4000.0);
        assert!(cam.offset.x >= 100.0 + 24.0 - 1e-9);
    }

    #[test]
    fn window_pixels_map_to_the_diagram_inside_the_canvas_only() {
        let v = Viewport { camera: Camera { zoom: 2.0, offset: Vec2::new(10.0, 20.0) }, ..Viewport::default() };
        let canvas = Rect::new(50.0, 0.0, 500.0, 400.0);
        let bounds = rhizo_core::Rect::new(-5.0, 7.0, 100.0, 100.0);
        assert_eq!(v.to_diagram(Vec2::new(110.0, 220.0), canvas, &bounds), Some(Point::new(45.0, 107.0)));
        assert_eq!(v.to_diagram(Vec2::new(40.0, 220.0), canvas, &bounds), None, "left of the canvas");
        assert_eq!(v.to_diagram(Vec2::new(110.0, 401.0), canvas, &bounds), None, "below it");
    }

    #[test]
    fn panning_moves_a_camera_move_along() {
        let mut v = Viewport::default();
        let to = Camera { zoom: 2.0, offset: Vec2::new(5.0, 5.0) };
        v.set(to, true);
        v.pan(Vec2::new(10.0, 0.0));
        assert_eq!(v.camera.offset, Vec2::new(10.0, 0.0));
        assert_eq!(v.goal().offset, Vec2::new(15.0, 5.0));
        v.follow_origin(Vec2::new(1.0, 1.0));
        assert_eq!(v.goal().offset, Vec2::new(17.0, 7.0));
        v.drag(Vec2::new(1.0, 0.0));
        assert!(v.camera_move.is_none());
    }
}
