//! Draws a laid-out [`Diagram`] straight into a vello scene. Geometry,
//! colors, dashes, arrowheads and text placement mirror the SVG writer
//! (crates/render/src/svg.rs) so the window matches the exports.

use std::collections::HashMap;
use std::sync::Arc;

use rhizo_core::{Color, LineStyle, Shape};
use rhizo_layout::{Diagram, EdgeBox, FrameBox, Ink, LabelBox, NodeBox, Path as LPath, Seg};
use rhizo_render::Fonts;
use vello::kurbo::{
    Affine, BezPath, Cap, Ellipse, Join, Line, Point, Rect, RoundedRect, RoundedRectRadii, Shape as _, Stroke,
};
use vello::peniko::{Blob, Fill, FontData};
use vello::{Glyph, Scene};

use crate::peniko_color;

/// Font files wrapped for vello once, so glyph outlines are cached per font.
#[derive(Default)]
pub struct FontCache(HashMap<usize, FontData>);

impl FontCache {
    pub(crate) fn get(&mut self, data: &Arc<Vec<u8>>, index: u32) -> FontData {
        self.0
            .entry(Arc::as_ptr(data) as usize)
            .or_insert_with(|| FontData::new(Blob::new(data.clone() as Arc<dyn AsRef<[u8]> + Send + Sync>), index))
            .clone()
    }
}

pub fn scene(d: &Diagram, fonts: &Fonts, cache: &mut FontCache) -> Scene {
    let mut s = Scene::new();
    // Diagram coordinates start at bounds.x/y; the scene starts at 0,0.
    let base = Affine::translate((-d.bounds.x as f64, -d.bounds.y as f64));
    let mut cx = Cx { s: &mut s, base, fonts, cache };
    for f in d.frames.iter().filter(|f| f.visible) {
        cx.frame(f);
    }
    for e in &d.edges {
        cx.edge(e);
    }
    for n in &d.nodes {
        cx.node(n);
    }
    s
}

/// One edge (path, arrowheads, label), for overlays that restyle it.
pub fn edge(s: &mut Scene, fonts: &Fonts, cache: &mut FontCache, transform: Affine, e: &EdgeBox) {
    Cx { s, base: transform, fonts, cache }.edge(e);
}

/// One line of text for overlays (labels being edited, the palette).
/// Returns the drawn width.
#[allow(clippy::too_many_arguments)]
pub fn text(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    transform: Affine,
    text: &str,
    x: f32,
    baseline: f32,
    size: f32,
    weight: u16,
    color: Color,
) -> f32 {
    let mut cx = Cx { s, base: transform, fonts, cache };
    cx.text_line(text, x, baseline, size, weight, color, 0.0)
}

struct Cx<'a> {
    s: &'a mut Scene,
    base: Affine,
    fonts: &'a Fonts,
    cache: &'a mut FontCache,
}

fn rect(r: &rhizo_core::Rect) -> Rect {
    Rect::new(r.x as f64, r.y as f64, r.right() as f64, r.bottom() as f64)
}

fn pt(p: rhizo_core::Point) -> Point {
    Point::new(p.x as f64, p.y as f64)
}

pub fn bez(p: &LPath) -> BezPath {
    let mut b = BezPath::new();
    b.move_to(pt(p.start));
    for seg in &p.segs {
        match *seg {
            Seg::Line(q) => b.line_to(pt(q)),
            Seg::Cubic(c1, c2, q) => b.curve_to(pt(c1), pt(c2), pt(q)),
        }
    }
    b
}

/// The same dash patterns as the SVG writer.
fn stroke(width: f32, line: LineStyle) -> Stroke {
    let w = width as f64;
    let s = Stroke::new(w).with_join(Join::Round);
    match line {
        LineStyle::Solid => s.with_caps(Cap::Round),
        LineStyle::Dashed => s.with_caps(Cap::Round).with_dashes(0.0, [w * 4.0 + 2.0, w * 3.0 + 2.0]),
        LineStyle::Dotted => s.with_caps(Cap::Round).with_dashes(0.0, [0.1, w * 2.5 + 1.5]),
    }
}

impl Cx<'_> {
    fn fill(&mut self, shape: &impl vello::kurbo::Shape, c: Color) {
        self.s.fill(Fill::NonZero, self.base, peniko_color(c), None, shape);
    }

    fn stroke(&mut self, shape: &impl vello::kurbo::Shape, c: Color, st: &Stroke) {
        self.s.stroke(st, self.base, peniko_color(c), None, shape);
    }

    fn frame(&mut self, f: &FrameBox) {
        let r = RoundedRect::from_rect(rect(&f.rect), 14.0);
        self.fill(&r, f.fill);
        self.stroke(&r, f.stroke, &Stroke::new(1.0));
        if let Some(label) = &f.label {
            let baseline = f.rect.y + 16.0 + f.label_size * 0.8;
            self.text_line(label, f.rect.x + 20.0, baseline, f.label_size, 600, f.label_color, 0.0);
        }
    }

    fn edge(&mut self, e: &EdgeBox) {
        let path = bez(&e.path);
        self.stroke(&path, e.color, &stroke(e.width, e.line));
        if e.arrow.at_end() {
            self.arrow(pt(e.path.end()), e.path.end_tangent(), e.color);
        }
        if e.arrow.at_start() {
            let t = e.path.start_tangent();
            self.arrow(pt(e.path.start), rhizo_core::Point::new(-t.x, -t.y), e.color);
        }
        if let Some(l) = &e.label {
            self.label(l);
        }
    }

    /// The SVG marker: a 10×10 triangle whose tip (9, 5) sits on the end point.
    fn arrow(&mut self, tip: Point, dir: rhizo_core::Point, c: Color) {
        let (dx, dy) = (dir.x as f64, dir.y as f64);
        let at = |back: f64, side: f64| Point::new(tip.x - dx * back - dy * side, tip.y - dy * back + dx * side);
        let mut b = BezPath::new();
        b.move_to(at(0.0, 0.0));
        b.line_to(at(8.0, 3.5));
        b.line_to(at(8.0, -3.5));
        b.close_path();
        self.fill(&b, c);
    }

    fn label(&mut self, l: &LabelBox) {
        let r = RoundedRect::from_rect(rect(&l.rect()), 4.0);
        let mut bg = l.background;
        // 92% like the SVG writer, of whatever alpha a fade left it.
        bg.a = (bg.a as f32 * 0.92) as u8;
        self.fill(&r, bg);
        self.text_block(&l.lines, l.center.x, l.center.y, l.font_size, 400, l.line_height, l.color);
    }

    fn node(&mut self, n: &NodeBox) {
        let r = rect(&n.rect);
        let st = stroke(n.stroke_width, n.line);
        let shape: Option<BezPath> = match n.shape {
            Shape::Rect => Some(RoundedRect::from_rect(r, 2.0).to_path(0.1)),
            Shape::Rounded => Some(RoundedRect::from_rect(r, 8.0f64.min(r.height() / 2.0)).to_path(0.1)),
            Shape::Pill => Some(RoundedRect::from_rect(r, r.height() / 2.0).to_path(0.1)),
            Shape::Ellipse | Shape::Circle => Some(Ellipse::from_rect(r).to_path(0.1)),
            Shape::Diamond => {
                let c = r.center();
                Some(poly(&[(c.x, r.y0), (r.x1, c.y), (c.x, r.y1), (r.x0, c.y)]))
            }
            Shape::Hexagon => {
                let inset = (r.height() * 0.35).min(r.width() / 2.0);
                let c = r.center();
                Some(poly(&[
                    (r.x0 + inset, r.y0),
                    (r.x1 - inset, r.y0),
                    (r.x1, c.y),
                    (r.x1 - inset, r.y1),
                    (r.x0 + inset, r.y1),
                    (r.x0, c.y),
                ]))
            }
            Shape::Text => None,
        };
        if let Some(p) = shape {
            self.fill(&p, n.fill);
            if n.stroke_width > 0.0 {
                self.stroke(&p, n.stroke, &st);
            }
        }
        if n.ink().is_empty() {
            let c = n.rect.center();
            self.text_block(&n.lines, c.x, c.y, n.font_size, n.font_weight, n.line_height, n.text);
        }
        for i in n.ink() {
            self.ink(&i.clone().offset(rhizo_core::Point::new(n.rect.x, n.rect.y)));
        }
    }

    /// A typed node's badge, subtitle, card rows: the SVG writer's `ink`.
    fn ink(&mut self, i: &Ink) {
        match i {
            Ink::Text { text, x, baseline, size, weight, color, end, .. } => {
                self.text_line(text, *x, *baseline, *size, *weight, *color, if *end { 1.0 } else { 0.0 });
            }
            Ink::Tag { rect: r, fill, text, size, color, .. } => {
                self.fill(&RoundedRect::from_rect(rect(r), 3.0), *fill);
                let c = r.center();
                self.text_line(text, c.x, c.y + size * 0.35, *size, 700, *color, 0.5);
            }
            Ink::Rule { from, to, color, width } => {
                self.stroke(&Line::new(pt(*from), pt(*to)), *color, &Stroke::new(*width as f64));
            }
            Ink::Band { rect: r, radius, fill } => {
                let rr = radius.min(r.height).min(r.width / 2.0).max(0.0) as f64;
                self.fill(&RoundedRect::from_rect(rect(r), RoundedRectRadii::new(rr, rr, 0.0, 0.0)), *fill);
            }
        }
    }

    /// Centered lines; the same baseline math as the SVG writer.
    #[allow(clippy::too_many_arguments)]
    fn text_block(&mut self, lines: &[String], cx: f32, cy: f32, size: f32, weight: u16, lh: f32, color: Color) {
        let top = cy - lh * lines.len() as f32 / 2.0;
        for (i, line) in lines.iter().enumerate() {
            let baseline = top + lh * (i as f32 + 0.5) + size * 0.35;
            self.text_line(line, cx, baseline, size, weight, color, 0.5);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text_line(
        &mut self,
        text: &str,
        x: f32,
        baseline: f32,
        size: f32,
        weight: u16,
        color: Color,
        // 0 starts the text at `x`, 0.5 centers it there, 1 ends it there.
        align: f32,
    ) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let Some(shaped) = self.fonts.shape(text, size, weight) else { return 0.0 };
        let x0 = x - shaped.width * align;
        let font = self.cache.get(&shaped.font.0, shaped.font.1);
        self.s
            .draw_glyphs(&font)
            .font_size(size)
            .transform(self.base * Affine::translate((x0 as f64, baseline as f64)))
            .brush(peniko_color(color))
            .draw(Fill::NonZero, shaped.glyphs.into_iter().map(|(id, gx)| Glyph { id: id as u32, x: gx, y: 0.0 }));
        shaped.width
    }
}

fn poly(points: &[(f64, f64)]) -> BezPath {
    let mut b = BezPath::new();
    b.move_to(points[0]);
    for &p in &points[1..] {
        b.line_to(p);
    }
    b.close_path();
    b
}
