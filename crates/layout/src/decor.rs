//! Typed nodes drawn the way their kit's look says ([`Document::decor`]):
//! a badge and subtitle beside the label, an entity as a card with one row
//! per field, a VPC as a container its subnets sit in.
//!
//! Everything here is sized and placed relative to the node's own top-left
//! corner; renderers draw the [`Ink`] on top of the node's shape instead of
//! its centered label.

use rhizo_core::{Color, Decor, DecorState, Document, LineStyle, Node, NodeId, Point, Rect, Shape, Size, Theme};

use crate::resolve::{self, NodeCtx, NodeLook};
use crate::text::{self, TextMeasure};
use crate::{Ink, Options};

/// What a node looks like and how big it is, plus the boxes drawn inside it.
pub(crate) struct Built<'a> {
    pub look: NodeLook,
    pub lines: Vec<String>,
    pub line_height: f32,
    pub size: Size,
    pub ink: Vec<Ink>,
    /// Rows and contained nodes, placed relative to this node's top-left.
    pub inner: Vec<Inner<'a>>,
}

/// A node drawn inside another (a card's row, a container's child) rather
/// than placed by the frame's layout.
pub(crate) struct Inner<'a> {
    pub node: &'a Node,
    /// The node it is drawn inside.
    pub host: NodeId,
    pub rect: Rect,
    pub look: NodeLook,
    pub lines: Vec<String>,
    pub line_height: f32,
    pub ink: Vec<Ink>,
    pub row: bool,
    pub depth: usize,
}

pub(crate) struct Env<'a> {
    pub doc: &'a Document,
    pub theme: &'a Theme,
    pub measure: &'a dyn TextMeasure,
    pub opts: &'a Options,
}

/// True for a node drawn inside its parent: a row of a card, or anything
/// in a container.
pub(crate) fn hosted(doc: &Document, n: &Node) -> bool {
    let Some(parent) = n.parent.as_ref() else { return false };
    let Some(p) = doc.decor.get(parent) else { return false };
    p.container || (p.card && doc.decor.get(&n.id).is_some_and(|d| d.row.is_some()))
}

/// Color for a tone name (`blue`) or a color (`#2563eb`), in a shade that
/// reads on the theme's background.
pub(crate) fn tone(theme: &Theme, name: &str) -> Color {
    let dark = theme.background.luminance() < 0.4;
    let pick = |light: u32, dark_v: u32| Color::hex(if dark { dark_v } else { light });
    match name {
        "blue" => pick(0x2563eb, 0x60a5fa),
        "green" => pick(0x16a34a, 0x4ade80),
        "amber" => pick(0xb45309, 0xfbbf24),
        "red" => pick(0xdc2626, 0xf87171),
        "violet" => pick(0x7c3aed, 0xa78bfa),
        "pink" => pick(0xdb2777, 0xf472b6),
        "teal" => pick(0x0f766e, 0x2dd4bf),
        "orange" => pick(0xea580c, 0xfb923c),
        "indigo" => pick(0x4f46e5, 0x818cf8),
        _ => name.parse().unwrap_or(theme.muted_text),
    }
}

const PAD: f32 = 12.0;
const GAP: f32 = 8.0;

impl<'e> Env<'e> {
    fn width(&self, s: &str, size: f32, weight: u16) -> f32 {
        self.measure.width(s, size, weight)
    }

    fn badge_size(&self) -> f32 {
        (self.theme.font_size * 0.7).round().max(8.0)
    }

    /// A tag with `text`, its top-left at `at`.
    fn tag(&self, text: &str, tone_name: &str, at: Point, class: &'static str) -> Ink {
        let size = self.badge_size();
        let c = tone(self.theme, tone_name);
        let w = (self.width(text, size, 700) + 10.0).ceil();
        Ink::Tag {
            rect: Rect::new(at.x, at.y, w, size + 6.0),
            fill: self.theme.tint(c, 0.85),
            text: text.to_string(),
            size,
            color: c.mix(self.theme.text, 0.15),
            class,
        }
    }

    fn tag_width(&self, text: &str) -> f32 {
        (self.width(text, self.badge_size(), 700) + 10.0).ceil()
    }

    /// The node and everything drawn inside it.
    pub fn build(&self, n: &'e Node, cx: &NodeCtx) -> Built<'e> {
        let mut look = resolve::node(self.doc, self.theme, n, cx);
        match self.doc.decor.get(&n.id) {
            Some(d) if d.container => self.container(n, d, look, cx),
            Some(d) if d.card => self.card(n, d, look, cx),
            Some(d) if d.badge.is_some() || d.subtitle.is_some() || d.state != DecorState::Normal => {
                state(&mut look, d.state, self.theme);
                let (ink, w, h, lines) = self.media(n, d, &look, Point::new(0.0, 0.0));
                let size = crate::node_size(look.shape, w, h, look.font_size);
                let off = Point::new(((size.width - w) / 2.0).round(), ((size.height - h) / 2.0).round());
                let ink = ink.into_iter().map(|i| i.offset(off)).collect();
                let line_height = self.measure.line_height(look.font_size);
                Built { look, lines, line_height, size, ink, inner: Vec::new() }
            }
            _ => self.plain(n, look, cx),
        }
    }

    fn plain(&self, n: &'e Node, look: NodeLook, cx: &NodeCtx) -> Built<'e> {
        let max_w =
            if cx.depth == 0 && cx.has_children { self.opts.max_text_width * 1.2 } else { self.opts.max_text_width };
        let lines = text::wrap(&n.label, max_w, look.font_size, look.font_weight, self.measure);
        let line_height = self.measure.line_height(look.font_size);
        let tw = lines.iter().map(|l| self.width(l, look.font_size, look.font_weight)).fold(0.0, f32::max);
        let th = line_height * lines.len() as f32;
        let size = crate::node_size(look.shape, tw, th, look.font_size);
        Built { look, lines, line_height, size, ink: Vec::new(), inner: Vec::new() }
    }

    /// Badge on the left, the label beside it, the subtitle under the
    /// label. Returns the ink, the block's width and height, and the lines.
    fn media(&self, n: &Node, d: &Decor, look: &NodeLook, at: Point) -> (Vec<Ink>, f32, f32, Vec<String>) {
        let fs = look.font_size;
        let weight = look.font_weight.max(600);
        let max_w = self.opts.max_text_width;
        let lines = text::wrap(&n.label, max_w, fs, weight, self.measure);
        let lh = self.measure.line_height(fs);
        let sfs = (fs - 2.0).max(8.0);
        let slh = self.measure.line_height(sfs);
        let subs = d.subtitle.as_deref().map(|s| text::wrap(s, max_w, sfs, 400, self.measure)).unwrap_or_default();

        let mut ink = Vec::new();
        let mut tx = at.x;
        if let Some(b) = &d.badge {
            let bh = self.badge_size() + 6.0;
            ink.push(self.tag(&b.text, &b.tone, Point::new(at.x, at.y + (lh - bh) / 2.0), "rz-badge"));
            tx += self.tag_width(&b.text) + GAP;
        }
        let muted = d.state == DecorState::Void;
        let title_color = if muted { self.theme.muted_text } else { look.text };
        let mut w: f32 = 0.0;
        for (i, l) in lines.iter().enumerate() {
            let baseline = at.y + lh * (i as f32 + 0.5) + fs * 0.35;
            let lw = self.width(l, fs, weight);
            w = w.max(lw);
            ink.push(Ink::Text {
                text: l.clone(),
                x: tx,
                baseline,
                size: fs,
                weight,
                color: title_color,
                end: false,
                class: "rz-title",
            });
            if muted {
                let y = baseline - fs * 0.3;
                ink.push(Ink::Rule {
                    from: Point::new(tx, y),
                    to: Point::new(tx + lw, y),
                    color: self.theme.muted_text,
                    width: 1.25,
                });
            }
        }
        let top = at.y + lh * lines.len() as f32;
        for (j, l) in subs.iter().enumerate() {
            w = w.max(self.width(l, sfs, 400));
            ink.push(Ink::Text {
                text: l.clone(),
                x: tx,
                baseline: top + slh * (j as f32 + 0.5) + sfs * 0.35,
                size: sfs,
                weight: 400,
                color: self.theme.muted_text,
                end: false,
                class: "rz-subtitle",
            });
        }
        let h = lh * lines.len() as f32 + slh * subs.len() as f32;
        (ink, (tx - at.x + w).ceil(), h.ceil(), lines)
    }

    /// A title band, then one row per child with a row look.
    fn card(&self, n: &'e Node, d: &Decor, mut look: NodeLook, cx: &NodeCtx) -> Built<'e> {
        state(&mut look, d.state, self.theme);
        // shortcut: cards are always rectangles; an inline `shape` other than
        // rect/rounded is ignored. Upgrade if anyone wants elliptic cards.
        if look.shape != Shape::Rect {
            look.shape = Shape::Rounded;
        }
        let th = self.theme;
        let fs = look.font_size;
        let lh = self.measure.line_height(fs);
        let bh = self.badge_size() + 6.0;
        let lines = text::wrap(&n.label, self.opts.max_text_width, fs, 600, self.measure);
        let header = ((lh * lines.len() as f32).max(bh) + 2.0 * 9.0).ceil();
        let (rfs, dfs) = ((fs - 1.0).max(8.0), (fs - 2.0).max(8.0));
        let rh = (self.measure.line_height(rfs) + 10.0).ceil();

        let rows: Vec<(&Node, &rhizo_core::DecorRow)> = n
            .children
            .iter()
            .filter_map(|c| self.doc.node(c))
            .filter(|c| hosted(self.doc, c))
            .filter_map(|c| Some((c, self.doc.decor.get(&c.id)?.row.as_ref()?)))
            .collect();

        // Width: the widest of the title and the rows.
        let title_w = lines.iter().map(|l| self.width(l, fs, 600)).fold(0.0, f32::max)
            + d.badge.as_ref().map_or(0.0, |b| GAP * 1.5 + self.tag_width(&b.text));
        let row_w = |c: &Node, r: &rhizo_core::DecorRow| {
            let weight = if r.strong { 600 } else { 400 };
            self.width(&c.label, rfs, weight)
                + if r.strong { self.width("*", rfs, 600) + 2.0 } else { 0.0 }
                + r.marker.as_ref().map_or(0.0, |m| 6.0 + self.tag_width(&m.text))
                + r.detail.as_ref().map_or(0.0, |s| 24.0 + self.width(s, dfs, 400))
        };
        let content = rows.iter().map(|(c, r)| row_w(c, r)).fold(title_w, f32::max);
        let w = (content + 2.0 * PAD).max(150.0).ceil();
        let h = header + if rows.is_empty() { 0.0 } else { rows.len() as f32 * rh + 6.0 };

        let accent = tone(th, &d.tone);
        let band = th.tint(accent, 0.88);
        let mut ink = Vec::new();
        if rows.is_empty() {
            look.fill = band;
        } else {
            let sw = look.stroke_width / 2.0;
            let radius = if look.shape == Shape::Rect { 2.0 } else { 8.0 };
            ink.push(Ink::Band { rect: Rect::new(sw, sw, w - 2.0 * sw, header - sw), radius: radius - sw, fill: band });
            ink.push(Ink::Rule {
                from: Point::new(0.0, header),
                to: Point::new(w, header),
                color: look.stroke,
                width: look.stroke_width,
            });
        }
        let title_color = if d.state == DecorState::Void { th.muted_text } else { look.text };
        let top = (header - lh * lines.len() as f32) / 2.0;
        for (i, l) in lines.iter().enumerate() {
            ink.push(Ink::Text {
                text: l.clone(),
                x: PAD,
                baseline: top + lh * (i as f32 + 0.5) + fs * 0.35,
                size: fs,
                weight: 600,
                color: title_color,
                end: false,
                class: "rz-title",
            });
        }
        if let Some(b) = &d.badge {
            let x = w - PAD - self.tag_width(&b.text);
            ink.push(self.tag(&b.text, &b.tone, Point::new(x, (header - bh) / 2.0), "rz-badge"));
        }

        let divider = look.stroke.mix(look.fill, 0.5);
        let inner = rows
            .iter()
            .enumerate()
            .map(|(i, (c, r))| {
                let mut ink = Vec::new();
                if i > 0 {
                    ink.push(Ink::Rule {
                        from: Point::new(PAD, 0.0),
                        to: Point::new(w - PAD, 0.0),
                        color: divider,
                        width: 1.0,
                    });
                }
                let baseline = rh / 2.0 + rfs * 0.35;
                let weight = if r.strong { 600 } else { 400 };
                let mut x = PAD;
                ink.push(Ink::Text {
                    text: c.label.clone(),
                    x,
                    baseline,
                    size: rfs,
                    weight,
                    color: th.text,
                    end: false,
                    class: "rz-field",
                });
                x += self.width(&c.label, rfs, weight);
                if r.strong {
                    ink.push(Ink::Text {
                        text: "*".into(),
                        x: x + 1.0,
                        baseline,
                        size: rfs,
                        weight: 600,
                        color: tone(th, "red"),
                        end: false,
                        class: "rz-required",
                    });
                    x += self.width("*", rfs, 600) + 2.0;
                }
                if let Some(m) = &r.marker {
                    ink.push(self.tag(&m.text, &m.tone, Point::new(x + 6.0, (rh - bh) / 2.0), "rz-marker"));
                }
                if let Some(detail) = &r.detail {
                    ink.push(Ink::Text {
                        text: detail.clone(),
                        x: w - PAD,
                        baseline,
                        size: dfs,
                        weight: 400,
                        color: th.muted_text,
                        end: true,
                        class: "rz-detail",
                    });
                }
                let rlook = NodeLook {
                    shape: Shape::Text,
                    fill: Color { a: 0, ..look.fill },
                    stroke_width: 0.0,
                    font_size: rfs,
                    font_weight: weight,
                    ..look.clone()
                };
                Inner {
                    node: c,
                    host: n.id.clone(),
                    rect: Rect::new(0.0, header + i as f32 * rh, w, rh),
                    look: rlook,
                    lines: vec![c.label.clone()],
                    line_height: self.measure.line_height(rfs),
                    ink,
                    row: true,
                    depth: cx.depth + 1,
                }
            })
            .collect();
        Built { look, lines, line_height: lh, size: Size::new(w, h), ink, inner }
    }

    /// The header (badge, label, subtitle) top-left, the children stacked
    /// under it, each built the way it would be on its own.
    fn container(&self, n: &'e Node, d: &Decor, mut look: NodeLook, cx: &NodeCtx) -> Built<'e> {
        let th = self.theme;
        if n.style.fill.is_none() && n.class.is_empty() {
            let accent = tone(th, &d.tone);
            // No fill: edges to the nodes inside stay visible.
            look.fill = Color { a: 0, ..th.background };
            look.stroke = th.tint(accent, 0.35);
            look.text = th.text;
        }
        look.shape = Shape::Rounded;
        state(&mut look, d.state, th);
        let (ink, hw, hh, lines) = self.media(n, d, &look, Point::new(PAD, 10.0));
        let mut y = 10.0 + hh + 12.0;
        let mut w = hw;
        let mut inner = Vec::new();
        let child_cx = NodeCtx { layout: cx.layout, depth: cx.depth + 1, branch: None, has_children: false };
        for c in n.children.iter().filter_map(|c| self.doc.node(c)) {
            let b = self.build(c, &child_cx);
            let rect = Rect::new(PAD, y, b.size.width, b.size.height);
            for mut i in b.inner {
                i.rect = i.rect.translate(rect.x, rect.y);
                inner.push(i);
            }
            inner.push(Inner {
                node: c,
                host: n.id.clone(),
                rect,
                look: b.look,
                lines: b.lines,
                line_height: b.line_height,
                ink: b.ink,
                row: false,
                depth: child_cx.depth,
            });
            w = w.max(b.size.width);
            y += b.size.height + 10.0;
        }
        // Nested containers line up at full width.
        let is_container = |i: &Inner| self.doc.decor.get(&i.node.id).is_some_and(|d| d.container);
        for i in inner.iter_mut().filter(|i| i.host == n.id && is_container(i)) {
            i.rect.width = w;
        }
        // Outer boxes first, so the deepest are drawn (and hit) last.
        inner.sort_by_key(|i| i.depth);
        let size = Size::new((w + 2.0 * PAD).ceil(), (y - 10.0 + PAD).max(hh + 20.0).ceil());
        let line_height = self.measure.line_height(look.font_size);
        Built { look, lines, line_height, size, ink, inner }
    }
}

/// A proposal draws dashed; a rejected or superseded node greyed out (its
/// label struck through, see `media`).
fn state(look: &mut NodeLook, s: DecorState, theme: &Theme) {
    match s {
        DecorState::Normal => {}
        DecorState::Draft => {
            look.line = LineStyle::Dashed;
            look.stroke = theme.muted_text;
        }
        DecorState::Void => {
            look.fill = theme.tint(look.fill, 0.5);
            look.stroke = theme.node_stroke;
            look.text = theme.muted_text;
        }
    }
}
