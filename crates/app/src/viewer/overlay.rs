//! Feedback drawn over the diagram, in scene coordinates: hover and
//! selection, "Connect to…", drags and the label being typed.

use rhizo_app::chrome::{self, ACCENT, Mark};
use rhizo_app::draw::FontCache;
use rhizo_app::field::{self, Look};
use rhizo_app::interact::{self, DragMode, FrameDrop};
use rhizo_app::peniko_color;
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::{self, Affine, Line, Rect, RoundedRect, Stroke};
use rhizo_app::vello::peniko::Fill;
use rhizo_core::{Color, Document, FrameId, NodeId, Point};
use rhizo_lang::Placement;
use rhizo_layout::{Diagram, EdgeBox, FrameBox, NodeBox};
use rhizo_render::Fonts;

use super::App;
use super::interaction::{Gesture, NEW_LABEL, NodeDrag, Target, TextEdit};

/// Font size for typing a label onto an edge that has none yet.
const EDGE_LABEL_SIZE: f32 = 12.0;

/// The diagram the overlay goes over, and the strokes it draws with.
struct Over<'a> {
    /// The diagram as drawn: the built one, or a relayout in progress.
    d: &'a Diagram,
    /// The diagram as built, whose frames say how siblings stack.
    built: &'a Diagram,
    doc: Option<&'a Document>,
    /// Edge indices only match the drawing while it is built from the editor's source.
    in_sync: bool,
    /// A thin line, in scene units.
    width: f64,
    background: Color,
}

impl<'a> Over<'a> {
    fn to_scene(&self, r: rhizo_core::Rect) -> Rect {
        let b = self.d.bounds;
        Rect::new((r.x - b.x) as f64, (r.y - b.y) as f64, (r.right() - b.x) as f64, (r.bottom() - b.y) as f64)
    }

    /// A diagram point in the scene (rounded as `f32` first).
    fn point(&self, p: Point) -> kurbo::Point {
        let b = self.d.bounds;
        ((p.x - b.x) as f64, (p.y - b.y) as f64).into()
    }

    fn node(&self, id: &NodeId) -> Option<&'a NodeBox> {
        self.d.nodes.iter().find(|n| &n.id == id)
    }

    fn center(&self, id: &NodeId) -> Option<kurbo::Point> {
        self.node(id).map(|n| self.to_scene(n.rect).center())
    }

    fn edge(&self, k: usize) -> Option<&'a EdgeBox> {
        interact::drawn_edge(self.d, self.doc?, k).filter(|_| self.in_sync)
    }

    fn mark(&self, t: &Option<Target>) -> Option<Mark<'a>> {
        match t.as_ref()? {
            Target::Node(id) => self.node(id).map(Mark::Node),
            Target::Edge(k) => self.edge(*k).map(Mark::Edge),
            Target::Frame(f) => self.frame(f).map(Mark::Frame),
        }
    }

    fn frame(&self, id: &FrameId) -> Option<&'a FrameBox> {
        self.d.frames.iter().find(|f| &f.id == id && f.visible)
    }

    /// Where a drag between frames would land: the frame lit up, or the
    /// frame being left dashed, with what the drop does by the pointer.
    fn frame_drop(&self, s: &mut Scene, drag: &NodeDrag, fonts: &Fonts, glyphs: &mut FontCache) {
        let Some(to) = &drag.frame_drop else { return };
        let accent = peniko_color(ACCENT);
        let leaving = self.doc.and_then(|d| d.node(&drag.id)).and_then(|n| self.frame(&n.frame));
        let text = match to {
            FrameDrop::Into(f) => {
                let Some(fb) = self.frame(f) else { return };
                let r =
                    RoundedRect::from_rect(self.to_scene(fb.rect).inflate(4.0 * self.width, 4.0 * self.width), 14.0);
                let mut tint = ACCENT;
                tint.a = 28;
                s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(tint), None, &r);
                s.stroke(&Stroke::new(self.width * 2.5), Affine::IDENTITY, accent, None, &r);
                format!("Move into {}", fb.label.as_deref().unwrap_or(f.as_str()))
            }
            FrameDrop::Out => {
                if let Some(fb) = leaving {
                    let r = RoundedRect::from_rect(
                        self.to_scene(fb.rect).inflate(4.0 * self.width, 4.0 * self.width),
                        14.0,
                    );
                    s.stroke(&self.dashed(), Affine::IDENTITY, accent, None, &r);
                }
                "Move out of the frame".to_string()
            }
        };
        // The label by the pointer, on a chip so it reads over anything.
        let size = (6.0 * self.width) as f32;
        let p = self.point(drag.now);
        let w = fonts.shape(&text, size, 600).map_or(0.0, |sh| sh.width) as f64;
        let (x, y) = (p.x + 14.0 * self.width / 2.0, p.y + 18.0 * self.width / 2.0);
        let chip = RoundedRect::new(
            x - 6.0 * self.width / 2.0,
            y - size as f64 * 1.1,
            x + w + 6.0 * self.width / 2.0,
            y + size as f64 * 0.5,
            6.0,
        );
        s.fill(Fill::NonZero, Affine::IDENTITY, accent, None, &chip);
        rhizo_app::draw::text(
            s,
            fonts,
            glyphs,
            Affine::IDENTITY,
            &text,
            x as f32,
            y as f32,
            size,
            600,
            Color::rgb(255, 255, 255),
        );
    }

    fn thick(&self) -> Stroke {
        Stroke::new(self.width * 2.0)
    }

    fn dashed(&self) -> Stroke {
        Stroke::new(self.width).with_dashes(0.0, [6.0 * self.width, 4.0 * self.width])
    }

    /// "Connect to…": a line from the source to the pointer at `to`.
    fn connect_line(&self, s: &mut Scene, from: &NodeId, to: Point) {
        if let Some(from) = self.center(from) {
            let b = self.d.bounds;
            let to = (to.x as f64 - b.x as f64, to.y as f64 - b.y as f64);
            s.stroke(&self.dashed(), Affine::IDENTITY, peniko_color(ACCENT), None, &Line::new(from, to));
        }
    }

    /// A node being dragged (with the others selected, in free frames),
    /// once it has moved: the edge it would make, or the ghosts and where
    /// they would land.
    fn drag(&self, s: &mut Scene, drag: &NodeDrag, zoom: f32, group: &[NodeId]) {
        if !drag.moved(zoom) {
            return;
        }
        let (accent, thick) = (peniko_color(ACCENT), self.thick());
        match drag.gesture {
            Gesture::Connect => {
                if let Some(from) = self.center(&drag.id) {
                    let to = match drag.connect_to.as_ref().and_then(|t| self.center(t)) {
                        Some(c) => c,
                        None => self.point(drag.now),
                    };
                    s.stroke(&thick, Affine::IDENTITY, accent, None, &Line::new(from, to));
                }
                if let Some(n) = drag.connect_to.as_ref().and_then(|t| self.node(t)) {
                    let rr = RoundedRect::from_rect(self.to_scene(n.rect).inflate(6.0, 6.0), 12.0);
                    s.stroke(&thick, Affine::IDENTITY, accent, None, &rr);
                }
            }
            Gesture::Drag(mode) if mode != DragMode::Fixed => {
                let others = group.iter().filter(|id| mode == DragMode::Move && **id != drag.id);
                let free = |n: &&NodeBox| n.id == drag.id || interact::drag_mode(self.built, n) == DragMode::Move;
                for n in std::iter::once(&drag.id).chain(others).filter_map(|id| self.node(id)).filter(free) {
                    let ghost = self.to_scene(n.rect.translate(drag.now.x - drag.start.x, drag.now.y - drag.start.y));
                    let r = RoundedRect::from_rect(ghost, 8.0);
                    let mut tint = ACCENT;
                    tint.a = 24;
                    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(tint), None, &r);
                    s.stroke(&self.dashed(), Affine::IDENTITY, accent, None, &r);
                }
                if let Some(target) = &drag.target {
                    self.drop_target(s, target);
                }
            }
            Gesture::Drag(_) => {}
        }
    }

    /// Where an outline drag would put the node: a line before or after a
    /// sibling (across the way siblings stack), or a ring round its new parent.
    fn drop_target(&self, s: &mut Scene, target: &Placement) {
        let (Placement::Child(t) | Placement::Before(t) | Placement::After(t)) = target;
        let Some(n) = self.node(t) else { return };
        let r = self.to_scene(n.rect);
        let layout = self.built.frames.iter().find(|f| f.id == n.frame).map(|f| f.layout);
        let vertical = self.doc.zip(layout).is_some_and(|(doc, l)| interact::siblings_stack_vertically(doc, n, l));
        let gap = 5.0;
        let line = match (target, vertical) {
            (Placement::Child(_), _) => None,
            (Placement::Before(_), true) => Some(Line::new((r.x0, r.y0 - gap), (r.x1, r.y0 - gap))),
            (Placement::After(_), true) => Some(Line::new((r.x0, r.y1 + gap), (r.x1, r.y1 + gap))),
            (Placement::Before(_), false) => Some(Line::new((r.x0 - gap, r.y0), (r.x0 - gap, r.y1))),
            (Placement::After(_), false) => Some(Line::new((r.x1 + gap, r.y0), (r.x1 + gap, r.y1))),
        };
        let (accent, thick) = (peniko_color(ACCENT), self.thick());
        match line {
            Some(l) => s.stroke(&thick, Affine::IDENTITY, accent, None, &l),
            None => {
                let rr = RoundedRect::from_rect(r.inflate(6.0, 6.0), 12.0);
                s.stroke(&thick, Affine::IDENTITY, accent, None, &rr);
            }
        }
    }

    /// The label editor, on top of its node or of its edge's label.
    fn label_editor(&self, s: &mut Scene, te: &TextEdit, fonts: &Fonts, glyphs: &mut FontCache) {
        // Where it goes, font size and weight, fill, ink.
        let look = match &te.target {
            Target::Node(id) => {
                self.node(id).map(|n| (self.to_scene(n.rect), n.font_size, n.font_weight, n.fill, n.text))
            }
            Target::Edge(k) => self.edge(*k).map(|e| match &e.label {
                Some(l) => (self.to_scene(l.rect()), l.font_size, 400, l.background, l.color),
                None => {
                    let m = e.path.midpoint();
                    let at = self.to_scene(rhizo_core::Rect::new(m.x, m.y, 0.0, 0.0));
                    (at, EDGE_LABEL_SIZE, 400, self.background, e.color)
                }
            }),
            // Over the frame's label, top left.
            Target::Frame(f) => self.frame(f).map(|fb| {
                let size = fb.label_size;
                let r = rhizo_core::Rect::new(
                    fb.rect.x + 12.0,
                    fb.rect.y + 10.0,
                    (fb.rect.width - 24.0).min(size * 16.0),
                    size * 2.0,
                );
                (self.to_scene(r), size, 600, fb.fill, fb.label_color)
            }),
        };
        let Some((r, size, weight, fill, ink)) = look else { return };
        let accent = peniko_color(ACCENT);
        let f = &te.field;
        let empty = f.text().is_empty() && f.preedit.is_empty();
        let typed = format!("{}{}", f.text(), f.preedit);
        let placeholder = if matches!(te.target, Target::Frame(_)) { super::frames::NEW_FRAME } else { NEW_LABEL };
        let shown = if empty && te.created { placeholder } else { typed.as_str() };
        let text_w = fonts.shape(shown, size, weight).map_or(0.0, |s| s.width) as f64;
        let w = r.width().max(text_w + 2.0 * size as f64);
        let rect = Rect::from_center_size(r.center(), (w, r.height().max(size as f64 * 2.0)));
        let rr = RoundedRect::from_rect(rect, 8.0);
        s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &rr);
        s.stroke(&self.thick(), Affine::IDENTITY, accent, None, &rr);
        let c = rect.center();
        let x = c.x - text_w / 2.0;
        let baseline = c.y + size as f64 * 0.35;
        if empty && te.created {
            // The placeholder, with the caret before it.
            rhizo_app::draw::text(
                s,
                fonts,
                glyphs,
                Affine::IDENTITY,
                placeholder,
                x as f32,
                baseline as f32,
                size,
                weight,
                ink.mix(fill, 0.55),
            );
            let caret = Line::new((x, baseline - size as f64 * 0.95), (x, baseline + size as f64 * 0.2));
            s.stroke(&Stroke::new(self.width.max(1.2)), Affine::IDENTITY, accent, None, &caret);
        } else {
            let look = Look { size, weight, ink, focused: true, px: self.width.max(1.2) / 1.5 };
            field::draw(s, fonts, glyphs, f, x, baseline, text_w + 2.0, &look);
        }
    }
}

impl App {
    /// Hover, selection, drag feedback and the label editor, in scene
    /// coordinates, over `moving` (the relayout in progress) if given.
    pub fn overlay(&mut self, moving: Option<&Diagram>) -> Option<Scene> {
        self.fonts();
        let built = self.built.as_ref()?;
        let zoom = self.viewport.camera.zoom;
        let over = Over {
            d: moving.unwrap_or(&built.diagram),
            built: &built.diagram,
            doc: self.editor.as_ref().map(|e| e.document()),
            in_sync: self.in_sync(),
            width: 2.0 / zoom * self.scale_factor(),
            background: built.background,
        };
        let mut s = Scene::new();
        let (selected, hovered) = (over.mark(&self.interaction.selected), over.mark(&self.interaction.hovered));
        chrome::empty_frame_hints(&mut s, over.d, &built.fonts, &mut self.glyphs);
        chrome::emphasis(&mut s, over.d, &built.fonts, &mut self.glyphs, over.width / 2.0, selected, hovered);
        // The other selected nodes.
        let group = self.interaction.nodes();
        for id in group.iter().filter(|id| Some(*id) != self.interaction.selected_node().as_ref()) {
            let mark = over.node(id).map(Mark::Node);
            chrome::emphasis(&mut s, over.d, &built.fonts, &mut self.glyphs, over.width / 2.0, mark, None);
        }
        if let (Some(from), Some(p)) = (&self.interaction.connecting, self.to_diagram(self.cursor)) {
            over.connect_line(&mut s, from, p);
        }
        if let Some(drag) = &self.interaction.node_drag {
            over.drag(&mut s, drag, zoom as f32, &group);
            if drag.moved(zoom as f32) {
                let fonts = self.fonts.as_ref().expect("loaded above");
                over.frame_drop(&mut s, drag, fonts, &mut self.glyphs);
            }
        }
        if let Some(te) = &self.interaction.editing {
            let fonts = self.fonts.as_ref().expect("loaded above");
            over.label_editor(&mut s, te, fonts, &mut self.glyphs);
        }
        Some(s)
    }

    /// The selection box being dragged, in window pixels.
    pub fn band_scene(&self) -> Option<Scene> {
        let (a, b) = self.interaction.band?;
        let mut s = Scene::new();
        chrome::selection_box(&mut s, Rect::from_points(a.to_point(), b.to_point()), self.scale_factor());
        Some(s)
    }
}
