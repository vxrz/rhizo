//! The project sidebar, left of the text pane and the canvas: the files
//! under the project root, each opening to the typed things it defines,
//! grouped by kind; problems counted per file and in total along the
//! bottom. Drawn like the rest of the chrome, from the theme background.

use std::collections::HashSet;

use rhizo_core::Color;
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, Stroke};
use vello::peniko::Fill;

use crate::chrome::{self, ACCENT, ERROR, Text, WARNING, ink, with_alpha};
use crate::draw::FontCache;
use crate::peniko_color;
use crate::project::{Kind, Scan, Thing};

/// Width in logical pixels: at first, and the narrowest and widest it gets.
pub const WIDTH: f64 = 248.0;
pub const MIN_WIDTH: f64 = 180.0;
pub const MAX_WIDTH: f64 = 480.0;

/// Row heights, in logical pixels.
const HEADER: f64 = 44.0;
const FILE_ROW: f64 = 28.0;
const GROUP_ROW: f64 = 24.0;
const THING_ROW: f64 = 26.0;
const FOOTER: f64 = 36.0;

/// The sidebar's state; the window keeps one for the session.
pub struct Sidebar {
    /// Shown or hidden by the person; `None` until they choose. Then it
    /// shows for projects of two files or more, and for opened folders.
    pub open: Option<bool>,
    /// Width in logical pixels.
    pub width: f64,
    /// Files opened to list their things, by path relative to the root.
    pub expanded: HashSet<String>,
    /// How far the rows are scrolled, in pixels.
    pub scroll: f64,
}

impl Default for Sidebar {
    fn default() -> Self {
        Sidebar { open: None, width: WIDTH, expanded: HashSet::new(), scroll: 0.0 }
    }
}

/// A small picture before a name, in the sidebar and in palette lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Kind(Kind),
    File,
    Folder,
    Error,
    Warning,
    /// A reference to a node of another file.
    Reference,
    Edge,
}

/// What the sidebar lists, made from a project scan.
#[derive(Default)]
pub struct Model {
    /// The root folder's name.
    pub name: String,
    pub files: Vec<FileEntry>,
    pub things: Vec<Thing>,
    pub errors: usize,
    pub warnings: usize,
}

pub struct FileEntry {
    /// Relative to the root, forward slashes.
    pub rel: String,
    pub errors: usize,
    pub warnings: usize,
    /// Its top-level things by kind, in [`Kind`] order: indices into [`Model::things`].
    pub groups: Vec<(Kind, Vec<usize>)>,
}

/// One line of the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// An index into [`Model::files`].
    File(usize),
    /// A kind's heading under a file.
    Group(usize, Kind),
    /// An index into [`Model::things`].
    Thing(usize),
}

impl Model {
    pub fn new(scan: &Scan) -> Model {
        let things = scan.things();
        let files = scan
            .files
            .iter()
            .map(|rel| {
                let (errors, warnings) = scan.counts(Some(rel));
                let mut groups: Vec<(Kind, Vec<usize>)> = Vec::new();
                for (i, t) in things.iter().enumerate().filter(|(_, t)| &t.rel == rel && t.top) {
                    match groups.iter_mut().find(|(k, _)| *k == t.kind) {
                        Some((_, list)) => list.push(i),
                        None => groups.push((t.kind, vec![i])),
                    }
                }
                groups.sort_by_key(|(k, _)| *k);
                FileEntry { rel: rel.clone(), errors, warnings, groups }
            })
            .collect();
        let (errors, warnings) = scan.counts(None);
        let name =
            scan.root.file_name().map_or_else(|| scan.root.display().to_string(), |n| n.to_string_lossy().into());
        Model { name, files, things, errors, warnings }
    }

    /// The rows shown with `expanded` files open.
    pub fn rows(&self, expanded: &HashSet<String>) -> Vec<Row> {
        let mut rows = Vec::new();
        for (f, file) in self.files.iter().enumerate() {
            rows.push(Row::File(f));
            if !expanded.contains(&file.rel) {
                continue;
            }
            for (kind, things) in &file.groups {
                rows.push(Row::Group(f, *kind));
                rows.extend(things.iter().map(|&t| Row::Thing(t)));
            }
        }
        rows
    }

    /// The file a row belongs to.
    pub fn file_of(&self, row: Row) -> Option<&FileEntry> {
        match row {
            Row::File(f) | Row::Group(f, _) => self.files.get(f),
            Row::Thing(t) => {
                let rel = &self.things.get(t)?.rel;
                self.files.iter().find(|f| &f.rel == rel)
            }
        }
    }
}

/// Where everything goes in a sidebar drawn at `rect`.
pub struct Layout {
    pub rect: Rect,
    /// The shown rows and where each is.
    pub rows: Vec<(Rect, Row)>,
    /// The problems summary along the bottom; a click lists them.
    pub footer: Rect,
    /// Which files show their things.
    open: Vec<bool>,
    dpr: f64,
}

impl Layout {
    pub fn new(model: &Model, state: &mut Sidebar, rect: Rect, dpr: f64) -> Layout {
        let footer = Rect::new(rect.x0, rect.y1 - FOOTER * dpr, rect.x1, rect.y1);
        let list = Rect::new(rect.x0, rect.y0 + HEADER * dpr, rect.x1, footer.y0);
        let rows = model.rows(&state.expanded);
        let height = |r: &Row| {
            dpr * match r {
                Row::File(_) => FILE_ROW,
                Row::Group(..) => GROUP_ROW,
                Row::Thing(_) => THING_ROW,
            }
        };
        let total: f64 = rows.iter().map(height).sum::<f64>() + 8.0 * dpr;
        state.scroll = state.scroll.clamp(0.0, (total - list.height()).max(0.0));
        let mut y = list.y0 - state.scroll;
        let mut placed = Vec::new();
        for r in rows {
            let h = height(&r);
            if y + h > list.y0 && y < list.y1 {
                placed.push((Rect::new(list.x0, y, list.x1, y + h), r));
            }
            y += h;
        }
        let open = model.files.iter().map(|f| state.expanded.contains(&f.rel)).collect();
        Layout { rect, rows: placed, footer, open, dpr }
    }

    /// The row under window point `p`, if it shows there.
    pub fn row_at(&self, p: Point) -> Option<Row> {
        let list = Rect::new(self.rect.x0, self.rect.y0 + HEADER * self.dpr, self.rect.x1, self.footer.y0);
        if !list.contains(p) {
            return None;
        }
        self.rows.iter().find(|(r, _)| r.contains(p)).map(|(_, row)| *row)
    }

    /// True on the sidebar's right edge, which resizes it.
    pub fn on_divider(&self, p: Point) -> bool {
        (p.x - self.rect.x1).abs() <= 4.0 * self.dpr && p.y >= self.rect.y0 && p.y <= self.rect.y1
    }
}

/// What to highlight besides hover.
pub struct Marks<'a> {
    /// The open file, relative to the root.
    pub current: Option<&'a str>,
    /// The thing selected on the canvas (an id in the open file).
    pub selected: Option<&'a str>,
    pub hover: Option<Row>,
    /// A thing an edge is being dragged to.
    pub drop: Option<Row>,
    pub hover_footer: bool,
}

/// Draws the sidebar laid out by `layout`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    model: &Model,
    layout: &Layout,
    dpr: f64,
    theme_bg: Color,
    marks: &Marks,
) {
    let ink = ink(theme_bg);
    let dark = theme_bg.luminance() < 0.4;
    let bg = theme_bg.mix(ink, if dark { 0.06 } else { 0.04 });
    let rect = layout.rect;
    let px = |v: f64| v * dpr;
    let fill = |s: &mut Scene, r: Rect, c: Color| s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(c), None, &r);
    let pill = |s: &mut Scene, r: Rect, c: Color| {
        s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(c), None, &RoundedRect::from_rect(r, px(6.0)))
    };
    fill(s, rect, bg);
    let edge = Line::new((rect.x1 - 0.5 * dpr, rect.y0), (rect.x1 - 0.5 * dpr, rect.y1));
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink.mix(theme_bg, 0.86)), None, &edge);

    let mut t = Text { s, fonts, cache, size: px(13.0) as f32, baseline: 0.0 };
    let pad = px(12.0);
    let width = rect.width() - px(1.0);

    // Header: the folder, and how many files.
    let mid = rect.y0 + px(HEADER / 2.0) + px(2.0);
    glyph(t.s, Glyph::Folder, Point::new(rect.x0 + pad + px(7.0), mid), dpr, theme_bg);
    t.baseline = (mid + px(13.0) * 0.35) as f32;
    t.size = px(11.5) as f32;
    let count = chrome::plural(model.files.len(), "file");
    let cw = t.width(&count, 400);
    t.draw(&count, rect.x0 + width - pad - cw, 400, ink.mix(bg, 0.5));
    t.size = px(13.0) as f32;
    let name = t.clip(&model.name, 600, width - 2.0 * pad - px(22.0) - cw - px(8.0));
    t.draw(&name, rect.x0 + pad + px(22.0), 600, ink);

    // Rows, clipped to the list between header and footer.
    let list = Rect::new(rect.x0, rect.y0 + px(HEADER), rect.x1, layout.footer.y0);
    t.s.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &list);
    let tint = |a: f32| with_alpha(ACCENT, a);
    for &(r, row) in &layout.rows {
        let inner = Rect::new(r.x0 + px(6.0), r.y0 + px(1.0), r.x0 + width - px(6.0), r.y1 - px(1.0));
        let mid = r.center().y;
        t.baseline = (mid + px(13.0) * 0.35) as f32;
        let hovered = marks.hover == Some(row);
        match row {
            Row::File(f) => {
                let file = &model.files[f];
                let current = marks.current == Some(file.rel.as_str());
                if current {
                    pill(t.s, inner, tint(if dark { 0.22 } else { 0.12 }));
                } else if hovered {
                    pill(t.s, inner, with_alpha(ink, 0.05));
                }
                let x = rect.x0 + pad;
                if !file.groups.is_empty() {
                    chevron(t.s, Point::new(x + px(4.0), mid), layout.open[f], dpr, ink.mix(bg, 0.45));
                }
                glyph(t.s, Glyph::File, Point::new(x + px(19.0), mid), dpr, theme_bg);
                let nx = x + px(32.0);
                // Counts on the right: errors, then warnings.
                let mut right = rect.x0 + width - pad;
                t.size = px(11.0) as f32;
                for (n, tone) in [(file.warnings, WARNING), (file.errors, ERROR)] {
                    if n == 0 {
                        continue;
                    }
                    let label = n.to_string();
                    let w = t.width(&label, 600) + px(10.0);
                    let b = Rect::new(right - w, mid - px(8.0), right, mid + px(8.0));
                    t.s.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        peniko_color(with_alpha(tone, 0.18)),
                        None,
                        &RoundedRect::from_rect(b, px(8.0)),
                    );
                    t.baseline = (mid + px(11.0) * 0.35) as f32;
                    t.draw(&label, b.x0 + px(5.0), 600, tone.mix(ink, 0.3));
                    right = b.x0 - px(4.0);
                }
                t.size = px(13.0) as f32;
                t.baseline = (mid + px(13.0) * 0.35) as f32;
                let weight = if current { 600 } else { 500 };
                let (dir, base) = match file.rel.rfind('/') {
                    Some(i) => file.rel.split_at(i + 1),
                    None => ("", file.rel.as_str()),
                };
                let room = right - nx - px(6.0);
                let base = t.clip(base, weight, room);
                let bw = t.width(&base, weight);
                let dir = t.clip(dir, 400, room - bw);
                let dw = t.draw(&dir, nx, 400, ink.mix(bg, 0.5));
                t.draw(&base, nx + dw, weight, ink);
            }
            Row::Group(_, kind) => {
                t.size = px(10.5) as f32;
                t.baseline = (mid + px(3.0) + px(10.5) * 0.35) as f32;
                t.draw(&kind.title().to_uppercase(), rect.x0 + pad + px(32.0), 600, ink.mix(bg, 0.5));
                t.size = px(13.0) as f32;
            }
            Row::Thing(i) => {
                let thing = &model.things[i];
                let selected = marks.current == Some(thing.rel.as_str()) && marks.selected == Some(thing.id.as_str());
                if marks.drop == Some(row) {
                    pill(t.s, inner, tint(if dark { 0.3 } else { 0.18 }));
                    let rr = RoundedRect::from_rect(inner, px(6.0));
                    t.s.stroke(&Stroke::new(px(1.5)), Affine::IDENTITY, peniko_color(ACCENT), None, &rr);
                } else if selected {
                    pill(t.s, inner, tint(if dark { 0.22 } else { 0.12 }));
                } else if hovered {
                    pill(t.s, inner, with_alpha(ink, 0.05));
                }
                let x = rect.x0 + pad + px(32.0);
                glyph(t.s, Glyph::Kind(thing.kind), Point::new(x + px(6.0), mid), dpr, theme_bg);
                let label = if thing.label.is_empty() { &thing.id } else { &thing.label };
                let label = t.clip(label, 400, rect.x0 + width - pad - x - px(18.0));
                t.draw(&label, x + px(18.0), 400, if thing.frame { ink.mix(bg, 0.25) } else { ink });
            }
        }
    }
    t.s.pop_layer();

    // Footer: the whole project's problems.
    let f = layout.footer;
    let hairline = Line::new((f.x0, f.y0 + 0.5 * dpr), (f.x0 + width, f.y0 + 0.5 * dpr));
    t.s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink.mix(theme_bg, 0.86)), None, &hairline);
    if marks.hover_footer {
        let r = Rect::new(f.x0 + px(6.0), f.y0 + px(4.0), f.x0 + width - px(6.0), f.y1 - px(4.0));
        pill(t.s, r, with_alpha(ink, 0.05));
    }
    let mid = f.center().y;
    let (text, tone) = summary(model.errors, model.warnings);
    let dot = Circle::new((f.x0 + pad + px(5.0), mid), px(3.5));
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(tone), None, &dot);
    t.size = px(12.0) as f32;
    t.baseline = (mid + px(12.0) * 0.35) as f32;
    let keys = chrome::keys(t.fonts, "⇧⌘M");
    let kw = t.width(&keys, 400);
    let text = t.clip(&text, 500, width - 2.0 * pad - px(16.0) - kw - px(8.0));
    t.draw(&text, f.x0 + pad + px(16.0), 500, ink.mix(bg, 0.15));
    t.draw(&keys, f.x0 + width - pad - kw, 400, ink.mix(bg, 0.5));
}

/// What hovering a row says: its full name, cut short in the row, and
/// what else there is to know.
pub fn tip(model: &Model, row: Row) -> String {
    match row {
        Row::File(f) => model.files.get(f).map_or_else(String::new, |file| match (file.errors, file.warnings) {
            (0, 0) => file.rel.clone(),
            (e, w) => format!("{} · {}", file.rel, summary(e, w).0),
        }),
        Row::Group(_, kind) => kind.title().to_string(),
        Row::Thing(i) => model.things.get(i).map_or_else(String::new, |t| {
            let name = if t.label.is_empty() { &t.id } else { &t.label };
            let drag = if t.frame { "" } else { " · drag onto the canvas to refer to it" };
            format!("{name} · {} in {}{drag}", t.ty, t.rel)
        }),
    }
}

/// "2 errors · 1 warning", or "No problems", and its dot's color.
pub fn summary(errors: usize, warnings: usize) -> (String, Color) {
    match (errors, warnings) {
        (0, 0) => ("No problems".into(), Color::hex(0x10b981)),
        (e, 0) => (chrome::plural(e, "error"), ERROR),
        (0, w) => (chrome::plural(w, "warning"), WARNING),
        (e, w) => (format!("{} · {}", chrome::plural(e, "error"), chrome::plural(w, "warning")), ERROR),
    }
}

pub(crate) fn chevron(s: &mut Scene, at: Point, open: bool, dpr: f64, color: Color) {
    let k = 3.2 * dpr;
    let mut p = BezPath::new();
    if open {
        p.move_to((at.x - k, at.y - k / 2.0));
        p.line_to((at.x, at.y + k / 2.0));
        p.line_to((at.x + k, at.y - k / 2.0));
    } else {
        p.move_to((at.x - k / 2.0, at.y - k));
        p.line_to((at.x + k / 2.0, at.y));
        p.line_to((at.x - k / 2.0, at.y + k));
    }
    let pen = Stroke::new(1.4 * dpr).with_caps(vello::kurbo::Cap::Round).with_join(vello::kurbo::Join::Round);
    s.stroke(&pen, Affine::IDENTITY, peniko_color(color), None, &p);
}

/// The color each glyph is drawn in, on `bg`.
fn tone(g: Glyph, bg: Color) -> Color {
    let dark = bg.luminance() < 0.4;
    let pick = |light: u32, night: u32| Color::hex(if dark { night } else { light });
    match g {
        Glyph::Kind(Kind::Entity) => pick(0x6366f1, 0x818cf8),
        Glyph::Kind(Kind::Enum) => pick(0x0ea5e9, 0x38bdf8),
        Glyph::Kind(Kind::Operation) => pick(0x10b981, 0x34d399),
        Glyph::Kind(Kind::Flow) => pick(0x8b5cf6, 0xa78bfa),
        Glyph::Kind(Kind::Environment) => pick(0x0891b2, 0x22d3ee),
        Glyph::Kind(Kind::Goal) => pick(0xf59e0b, 0xfbbf24),
        Glyph::Kind(Kind::Decision) => pick(0xec4899, 0xf472b6),
        Glyph::Error => ERROR,
        Glyph::Warning => WARNING,
        Glyph::Reference | Glyph::Edge => ACCENT,
        Glyph::Kind(Kind::Other) | Glyph::File | Glyph::Folder => ink(bg).mix(bg, 0.45),
    }
}

/// Draws glyph `g` about 14 logical pixels across, centred on `at`.
pub fn glyph(s: &mut Scene, g: Glyph, at: Point, dpr: f64, bg: Color) {
    let c = peniko_color(tone(g, bg));
    let t = Affine::translate(at.to_vec2()) * Affine::scale(dpr);
    let line = Stroke::new(1.4).with_join(vello::kurbo::Join::Round).with_caps(vello::kurbo::Cap::Round);
    match g {
        Glyph::Kind(Kind::Entity) => {
            let card = RoundedRect::new(-6.0, -5.0, 6.0, 5.0, 2.0);
            s.fill(Fill::NonZero, t, c, None, &RoundedRect::new(-6.0, -5.0, 6.0, -1.5, (2.0, 2.0, 0.0, 0.0)));
            s.stroke(&line, t, c, None, &card);
        }
        Glyph::Kind(Kind::Enum) => {
            for y in [-4.0, 0.0, 4.0] {
                s.fill(Fill::NonZero, t, c, None, &Circle::new((-4.5, y), 1.3));
                s.stroke(&line, t, c, None, &Line::new((-1.5, y), (5.5, y)));
            }
        }
        Glyph::Kind(Kind::Operation) => {
            s.stroke(&line, t, c, None, &RoundedRect::new(-6.5, -4.0, 6.5, 4.0, 4.0));
            s.fill(Fill::NonZero, t, c, None, &Circle::new((-2.5, 0.0), 1.5));
        }
        Glyph::Kind(Kind::Flow) => {
            s.stroke(&line, t, c, None, &Line::new((-3.0, 0.0), (2.0, 0.0)));
            s.fill(Fill::NonZero, t, c, None, &Circle::new((-4.5, 0.0), 2.2));
            s.stroke(&line, t, c, None, &RoundedRect::new(2.0, -3.0, 7.0, 3.0, 1.0));
        }
        Glyph::Kind(Kind::Environment) => {
            let dashed = Stroke::new(1.3).with_dashes(0.0, [2.2, 1.6]);
            s.stroke(&dashed, t, c, None, &RoundedRect::new(-6.5, -5.0, 6.5, 5.0, 2.0));
            s.fill(Fill::NonZero, t, c, None, &RoundedRect::new(-3.0, -1.5, 3.0, 1.5, 0.8));
        }
        Glyph::Kind(Kind::Goal) => {
            s.stroke(&line, t, c, None, &Circle::new((0.0, 0.0), 5.5));
            s.fill(Fill::NonZero, t, c, None, &Circle::new((0.0, 0.0), 2.2));
        }
        Glyph::Kind(Kind::Decision) => {
            let mut d = BezPath::new();
            d.move_to((0.0, -6.0));
            d.line_to((6.0, 0.0));
            d.line_to((0.0, 6.0));
            d.line_to((-6.0, 0.0));
            d.close_path();
            s.stroke(&line, t, c, None, &d);
        }
        Glyph::Kind(Kind::Other) => {
            s.stroke(&line, t, c, None, &Circle::new((0.0, 0.0), 4.0));
        }
        Glyph::File => {
            let mut d = BezPath::new();
            d.move_to((-4.5, -6.5));
            d.line_to((1.5, -6.5));
            d.line_to((4.5, -3.5));
            d.line_to((4.5, 6.5));
            d.line_to((-4.5, 6.5));
            d.close_path();
            s.stroke(&Stroke::new(1.3).with_join(vello::kurbo::Join::Round), t, c, None, &d);
        }
        Glyph::Folder => {
            let mut f = BezPath::new();
            f.move_to((-6.5, -4.5));
            f.line_to((-2.0, -4.5));
            f.line_to((-0.5, -2.5));
            f.line_to((6.5, -2.5));
            f.line_to((6.5, 5.0));
            f.line_to((-6.5, 5.0));
            f.close_path();
            s.stroke(&Stroke::new(1.3).with_join(vello::kurbo::Join::Round), t, c, None, &f);
        }
        Glyph::Error => {
            s.fill(Fill::NonZero, t, c, None, &Circle::new((0.0, 0.0), 5.5));
            let white = peniko_color(Color::rgb(255, 255, 255));
            s.stroke(
                &Stroke::new(1.6).with_caps(vello::kurbo::Cap::Round),
                t,
                white,
                None,
                &Line::new((0.0, -2.8), (0.0, 0.8)),
            );
            s.fill(Fill::NonZero, t, white, None, &Circle::new((0.0, 3.0), 0.9));
        }
        Glyph::Warning => {
            let mut d = BezPath::new();
            d.move_to((0.0, -6.0));
            d.line_to((6.5, 5.5));
            d.line_to((-6.5, 5.5));
            d.close_path();
            s.fill(Fill::NonZero, t, c, None, &d);
            let white = peniko_color(Color::rgb(255, 255, 255));
            s.stroke(
                &Stroke::new(1.5).with_caps(vello::kurbo::Cap::Round),
                t,
                white,
                None,
                &Line::new((0.0, -2.0), (0.0, 1.5)),
            );
            s.fill(Fill::NonZero, t, white, None, &Circle::new((0.0, 3.6), 0.85));
        }
        Glyph::Reference => {
            let dashed = Stroke::new(1.3).with_dashes(0.0, [2.4, 1.6]);
            s.stroke(&dashed, t, c, None, &RoundedRect::new(-6.5, -4.5, 6.5, 4.5, 2.5));
        }
        Glyph::Edge => {
            s.stroke(&line, t, c, None, &Line::new((-6.0, 0.0), (5.0, 0.0)));
            let mut head = BezPath::new();
            head.move_to((2.0, -3.0));
            head.line_to((5.5, 0.0));
            head.line_to((2.0, 3.0));
            s.stroke(&line, t, c, None, &head);
        }
    }
}

/// A thing being dragged out of the sidebar: a chip under the pointer.
#[allow(clippy::too_many_arguments)]
pub fn chip(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    at: Point,
    dpr: f64,
    bg: Color,
    g: Glyph,
    label: &str,
) {
    let ink = ink(bg);
    let mut t = Text { s, fonts, cache, size: (13.0 * dpr) as f32, baseline: 0.0 };
    let label = t.clip(label, 500, 260.0 * dpr);
    let (h, pad) = (30.0 * dpr, 10.0 * dpr);
    let w = pad + 16.0 * dpr + t.width(&label, 500) + pad;
    let r = Rect::new(at.x + 10.0 * dpr, at.y + 6.0 * dpr, at.x + 10.0 * dpr + w, at.y + 6.0 * dpr + h);
    chrome::shadow(t.s, r, h / 2.0, dpr);
    let rr = RoundedRect::from_rect(r, h / 2.0);
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg), None, &rr);
    t.s.stroke(&Stroke::new(1.5 * dpr), Affine::IDENTITY, peniko_color(ACCENT), None, &rr);
    glyph(t.s, g, Point::new(r.x0 + pad + 6.0 * dpr, r.center().y), dpr, bg);
    t.baseline = (r.center().y + 13.0 * dpr * 0.35) as f32;
    t.draw(&label, r.x0 + pad + 16.0 * dpr, 500, ink);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::project::Opened;

    fn shop() -> Model {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop");
        Model::new(&Scan::read(1, &Opened::Folder(root), &rhizo_kit::Kits::builtin()))
    }

    #[test]
    fn files_open_to_their_things_grouped_by_kind() {
        let m = shop();
        assert_eq!(m.name, "shop");
        let rels: Vec<&str> = m.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(
            rels,
            [
                "api.rhizo",
                "flows/checkout.rhizo",
                "flows/order-intake.rhizo",
                "infra.rhizo",
                "intent.rhizo",
                "model.rhizo"
            ]
        );
        let mut open = HashSet::new();
        assert_eq!(m.rows(&open).len(), 6, "closed: files only");
        open.insert("model.rhizo".to_string());
        let rows = m.rows(&open);
        let labels: Vec<String> = rows
            .iter()
            .map(|r| match *r {
                Row::File(f) => m.files[f].rel.clone(),
                Row::Group(_, k) => k.title().to_string(),
                Row::Thing(t) => format!("  {}", m.things[t].label),
            })
            .collect();
        assert_eq!(labels[5..], ["model.rhizo", "Entities", "  Customer", "  Order", "Enums", "  Status"]);
        assert_eq!(m.file_of(rows[7]).map(|f| f.rel.as_str()), Some("model.rhizo"));
        open.insert("intent.rhizo".to_string());
        let intent: Vec<Kind> =
            m.rows(&open).iter().filter_map(|r| if let Row::Group(4, k) = r { Some(*k) } else { None }).collect();
        assert_eq!(intent, [Kind::Goal, Kind::Decision]);
        assert_eq!((m.errors, m.warnings), (0, 0));
    }

    #[test]
    fn layout_scrolls_and_hits() {
        let m = shop();
        let expanded = m.files.iter().map(|f| f.rel.clone()).collect();
        let mut state = Sidebar { expanded, scroll: 1e9, ..Sidebar::default() };
        let rect = Rect::new(0.0, 0.0, 240.0, 300.0);
        let l = Layout::new(&m, &mut state, rect, 1.0);
        assert!(state.scroll > 0.0 && state.scroll < 1e9, "clamped to the content");
        let last = l.rows.last().unwrap();
        assert!(last.0.y1 <= l.footer.y0 + 8.0, "scrolled to the end");
        assert_eq!(l.row_at(last.0.center()), Some(last.1));
        assert_eq!(l.row_at(Point::new(10.0, 5.0)), None, "the header is not a row");
        assert_eq!(l.row_at(l.footer.center()), None);
        assert!(l.on_divider(Point::new(241.0, 100.0)) && !l.on_divider(Point::new(200.0, 100.0)));
        assert_eq!(summary(0, 0).0, "No problems");
        assert_eq!(summary(2, 1).0, "2 errors · 1 warning");
        assert_eq!(summary(0, 1).0, "1 warning");
    }
}
