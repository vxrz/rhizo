//! The viewer's own UI over the diagram: the status bar, the problems
//! banner, and hover and selection emphasis. UI colors derive from the
//! theme background, so light, dark and paper all read well.

use rhizo_core::{Color, Shape};
use rhizo_lang::Diagnostic;
use rhizo_layout::{Diagram, EdgeBox, FrameBox, NodeBox};
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, Circle, Line, Rect, RoundedRect, Stroke, Vec2};
use vello::peniko::Fill;

use crate::draw::{self, FontCache};
use crate::peniko_color;

/// Selection, hover, drop and edit indicators: the brand indigo.
pub const ACCENT: Color = Color::hex(0x6366f1);
pub(crate) const ERROR: Color = Color::hex(0xef4444);
pub(crate) const WARNING: Color = Color::hex(0xf59e0b);
const SUCCESS: Color = Color::hex(0x10b981);
/// Status bar height in logical pixels.
pub const BAR_HEIGHT: f64 = 28.0;
/// Spelled out by [`keys`] where the font lacks the symbol.
pub const HINT: &str = if cfg!(target_os = "macos") { "⌘K commands" } else { "Ctrl+K commands" };

/// `text` with key symbols the font lacks spelled out ("⇧⌘Z" → "Shift+Cmd+Z").
pub fn keys(fonts: &Fonts, text: &str) -> String {
    let missing = fonts.shape(text, 12.0, 400).is_none_or(|s| s.glyphs.iter().any(|g| g.0 == 0));
    if missing { spell_keys(text) } else { text.to_string() }
}

fn spell_keys(text: &str) -> String {
    text.replace("⌥ ", "⌥").replace('⌥', "Alt+").replace('⇧', "Shift+").replace('⌘', "Cmd+").replace('⌫', "Backspace")
}

/// A soft drop shadow under a card.
pub(crate) fn shadow(s: &mut Scene, card: Rect, radius: f64, dpr: f64) {
    let c = with_alpha(Color::rgb(0, 0, 0), 0.16);
    s.draw_blurred_rounded_rect(Affine::translate((0.0, 3.0 * dpr)), card, peniko_color(c), radius, 6.0 * dpr);
}

/// Text color for UI drawn on `bg`.
pub fn ink(bg: Color) -> Color {
    bg.readable_on(Color::hex(0xf4f4f5), Color::hex(0x18181b))
}

pub(crate) fn with_alpha(c: Color, a: f32) -> Color {
    Color { a: (a * 255.0) as u8, ..c }
}

// ------------------------------------------------------------- problems

/// What is wrong with the document, condensed for the banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problems {
    pub errors: usize,
    pub warnings: usize,
    /// The first problem: errors before warnings, then by position.
    pub message: String,
    pub help: Option<String>,
    /// 1-based line and column of the first problem.
    pub at: Option<(usize, usize)>,
}

impl Problems {
    /// `None` when there is nothing to report.
    pub fn new(src: &str, diagnostics: &[Diagnostic]) -> Option<Problems> {
        let first = diagnostics.iter().min_by_key(|d| (!d.is_error(), d.span.map_or(usize::MAX, |s| s.0)))?;
        let errors = diagnostics.iter().filter(|d| d.is_error()).count();
        Some(Problems {
            errors,
            warnings: diagnostics.len() - errors,
            message: first.message.clone(),
            help: first.help.clone(),
            at: first.line_col(src),
        })
    }

    /// One error without a position, such as an unreadable file.
    pub fn single(message: &str) -> Problems {
        Problems { errors: 1, warnings: 0, message: message.to_string(), help: None, at: None }
    }

    pub fn is_error(&self) -> bool {
        self.errors > 0
    }

    /// The problems besides the first, e.g. "+2 errors, 1 warning".
    pub fn more(&self) -> Option<String> {
        let (e, w) =
            if self.is_error() { (self.errors - 1, self.warnings) } else { (0, self.warnings.saturating_sub(1)) };
        match (e, w) {
            (0, 0) => None,
            (e, 0) => Some(format!("+{}", plural(e, "error"))),
            (0, w) => Some(format!("+{}", plural(w, "warning"))),
            (e, w) => Some(format!("+{}, {}", plural(e, "error"), plural(w, "warning"))),
        }
    }
}

pub fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

// ----------------------------------------------------------- status bar

/// What the status bar shows.
pub struct Status {
    pub file: String,
    pub nodes: usize,
    pub edges: usize,
    /// 1.0 is 100%.
    pub zoom: f64,
    pub selection: Option<String>,
    /// How visible "Saved" is (0 hidden, 1 shown), after an edit is written.
    pub saved: f32,
    /// Errors and warnings in the whole project, when there are any.
    pub project: (usize, usize),
    /// A background job's progress ("Adopting `x`… planning the original code").
    pub job: Option<String>,
    /// What the keys do for the selection ([`crate::palette::hints`]);
    /// empty shows the ⌘K hint alone.
    pub hints: Vec<String>,
}

impl Status {
    pub fn counts(&self) -> String {
        format!("{} · {}", plural(self.nodes, "node"), plural(self.edges, "edge"))
    }

    pub fn zoom_text(&self) -> String {
        format!("{:.0}%", self.zoom * 100.0)
    }
}

pub fn describe_node(id: &str, label: &str) -> String {
    if label.is_empty() { id.to_string() } else { format!("{id} “{label}”") }
}

pub fn describe_edge(from: &str, to: &str, label: Option<&str>) -> String {
    match label.filter(|l| !l.is_empty()) {
        Some(l) => format!("edge {from} → {to} “{l}”"),
        None => format!("edge {from} → {to}"),
    }
}

/// Where the status bar's parts were drawn, for hovering them.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bar {
    /// Where the file name ends.
    pub file_end: f64,
    pub zoom: Rect,
    /// The project's problems, when it has any.
    pub problems: Option<Rect>,
    /// The ⌘K hint.
    pub hint: Rect,
}

/// Draws the status bar along the bottom of a `size` (pixels) window.
/// Returns where its parts are, for hovering them.
pub fn status_bar(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    st: &Status,
) -> Bar {
    let ink = ink(bg);
    let h = BAR_HEIGHT * dpr;
    let top = size.y - h;
    let bar = Rect::new(0.0, top, size.x, size.y);
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg.mix(ink, 0.035)), None, &bar);
    let hairline = Line::new((0.0, top + 0.5 * dpr), (size.x, top + 0.5 * dpr));
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink.mix(bg, 0.86)), None, &hairline);

    let mut t = Text { s, fonts, cache, size: 12.0 * dpr as f32, baseline: (top + h / 2.0) as f32 };
    t.baseline += t.size * 0.35;
    let pad = 12.0 * dpr;
    let mut x = pad;
    x += t.draw(&st.file, x, 600, ink.mix(bg, 0.08));
    let file_end = x;
    x += t.draw(&format!("   {}", st.counts()), x, 400, ink.mix(bg, 0.5));
    if st.saved > 0.0 {
        x += t.draw("   Saved", x, 400, with_alpha(ink.mix(bg, 0.5), st.saved));
    }
    if let Some(job) = &st.job {
        let dot = Circle::new((x + 18.0 * dpr, top + h / 2.0), 3.5 * dpr);
        t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(WARNING), None, &dot);
        let job = t.clip(job, 500, (size.x * 0.45 - x).max(80.0 * dpr));
        x += 26.0 * dpr + t.draw(&job, x + 26.0 * dpr, 500, ink.mix(bg, 0.15));
    }

    // The selection's hints, quiet, where they fit: on a narrow window, or
    // beside a long selection, fewer of them, down to the ⌘K hint alone.
    let room = if size.x < 900.0 * dpr { 0.0 } else { size.x * 0.4 };
    let mut shown = st.hints.len().min(4);
    let hint = loop {
        if shown == 0 {
            break keys(t.fonts, HINT);
        }
        let text = keys(t.fonts, &st.hints[..shown].join("  ·  "));
        if t.width(&text, 400) <= room {
            break text;
        }
        shown -= 1;
    };
    let hint_x = size.x - pad - t.width(&hint, 400);
    t.draw(&hint, hint_x, 400, ink.mix(bg, 0.55));
    let zoom = st.zoom_text();
    let mut zoom_x = hint_x - 16.0 * dpr - t.width(&zoom, 500);
    t.draw(&zoom, zoom_x, 500, ink.mix(bg, 0.3));
    let part = |x0: f64, x1: f64| Rect::new(x0 - 6.0 * dpr, top, x1 + 6.0 * dpr, size.y);
    let mut bar = Bar {
        file_end: 0.0,
        zoom: part(zoom_x, hint_x - 16.0 * dpr),
        problems: None,
        hint: part(hint_x, size.x - pad),
    };
    // The project's problems, before the zoom; the sidebar lists them.
    if st.project != (0, 0) {
        let (text, tone) = crate::sidebar::summary(st.project.0, st.project.1);
        let text = format!("{text} in project");
        let w = t.width(&text, 500);
        let x0 = zoom_x - 20.0 * dpr - w;
        t.s.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            peniko_color(tone),
            None,
            &Circle::new((x0 - 9.0 * dpr, top + h / 2.0), 3.5 * dpr),
        );
        t.draw(&text, x0, 500, tone.mix(ink, 0.35));
        bar.problems = Some(part(x0 - 14.0 * dpr, x0 + w));
        zoom_x = x0 - 12.0 * dpr;
    }

    // The selection sits in the middle, or as near it as the sides allow.
    if let Some(sel) = &st.selection {
        let (lo, hi) = (x + 24.0 * dpr, zoom_x - 24.0 * dpr);
        let dot = 12.0 * dpr;
        let text = t.clip(sel, 500, hi - lo - dot);
        let w = dot + t.width(&text, 500);
        let x0 = ((size.x - w) / 2.0).min(hi - w).max(lo);
        if hi - lo > dot * 3.0 {
            let c = Circle::new((x0 + 3.5 * dpr, top + h / 2.0), 3.5 * dpr);
            t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(ACCENT), None, &c);
            t.draw(&text, x0 + dot, 500, ink.mix(bg, 0.12));
        }
    }
    Bar { file_end, ..bar }
}

/// A link of the empty-canvas hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyLink {
    Learn,
    Samples,
}

/// What an empty document shows, centred in a `size` canvas: how to start,
/// and links to the tutorial and the samples (`hover` underlined). Returns
/// where the links are, in the canvas's pixels.
#[allow(clippy::too_many_arguments)]
pub fn empty_hint(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    hover: Option<EmptyLink>,
) -> Vec<(Rect, EmptyLink)> {
    let ink = ink(bg);
    let mut t = Text { s, fonts, cache, size: 0.0, baseline: 0.0 };
    let mid = size.y / 2.0;
    let line = |t: &mut Text, text: &str, dy: f64, pt: f64, color: Color| {
        t.size = (pt * dpr) as f32;
        t.baseline = (mid + dy * dpr) as f32;
        let text = t.clip(text, 400, size.x - 32.0 * dpr);
        t.draw(&text, (size.x - t.width(&text, 400)) / 2.0, 400, color);
    };
    line(&mut t, "Double-click or press Tab to add a node", -34.0, 16.0, ink.mix(bg, 0.3));
    line(&mut t, "Drag a file here to open it", -8.0, 13.0, ink.mix(bg, 0.5));
    let all = keys(t.fonts, "⌘K for everything");
    line(&mut t, &all, 12.0, 13.0, ink.mix(bg, 0.5));
    let links = [(EmptyLink::Learn, "Learn rhizo"), (EmptyLink::Samples, "Open a sample…")];
    t.size = (13.0 * dpr) as f32;
    t.baseline = (mid + 44.0 * dpr) as f32;
    let gap = "     ·     ";
    let total = links.iter().map(|(_, l)| t.width(l, 500)).sum::<f64>() + t.width(gap, 400);
    let mut x = (size.x - total) / 2.0;
    let mut rects = Vec::new();
    for (i, (link, label)) in links.into_iter().enumerate() {
        if i > 0 {
            x += t.draw(gap, x, 400, ink.mix(bg, 0.6));
        }
        let w = t.draw(label, x, 500, ACCENT.mix(ink, if hover == Some(link) { 0.25 } else { 0.0 }));
        let base = t.baseline as f64;
        if hover == Some(link) {
            let u = Line::new((x, base + 3.0 * dpr), (x + w, base + 3.0 * dpr));
            t.s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ACCENT), None, &u);
        }
        rects.push((Rect::new(x - 4.0 * dpr, base - 16.0 * dpr, x + w + 4.0 * dpr, base + 6.0 * dpr), link));
        x += w;
    }
    rects
}

/// The visible frames of `d` with nothing in them: no node, no frame.
pub fn empty_frames(d: &Diagram) -> Vec<&FrameBox> {
    d.frames
        .iter()
        .filter(|f| f.visible)
        .filter(|f| {
            !d.nodes.iter().any(|n| n.frame == f.id) && !d.frames.iter().any(|g| g.parent.as_ref() == Some(&f.id))
        })
        .collect()
}

/// A short hint inside each empty frame of `d`, in scene coordinates
/// (like [`emphasis`]), so it zooms with the frame.
pub fn empty_frame_hints(s: &mut Scene, d: &Diagram, fonts: &Fonts, cache: &mut FontCache) {
    let base = Affine::translate((-d.bounds.x as f64, -d.bounds.y as f64));
    for f in empty_frames(d) {
        let r = &f.rect;
        let size = 11.5;
        let mid = r.y + r.height / 2.0 + f.label_size / 2.0;
        // Two short lines: an empty frame is small.
        for (k, line) in ["Add typed node…", "or drag nodes here"].into_iter().enumerate() {
            let text = clip(fonts, line, size, 400, r.width as f64 - 20.0);
            let w = fonts.shape(&text, size, 400).map_or(0.0, |s| s.width);
            let y = mid + (k as f32 - 0.5) * size * 1.45 + size * 0.35;
            draw::text(
                s,
                fonts,
                cache,
                base,
                &text,
                r.x + (r.width - w) / 2.0,
                y,
                size,
                400,
                f.label_color.mix(f.fill, 0.4),
            );
        }
    }
}

/// The box a drag on empty canvas draws to select the nodes it meets
/// (window pixels).
pub fn selection_box(s: &mut Scene, r: Rect, dpr: f64) {
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(with_alpha(ACCENT, 0.1)), None, &r);
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(with_alpha(ACCENT, 0.85)), None, &r);
}

/// The part of the window that has the keyboard: an accent ring just
/// inside its edge (window pixels).
pub fn focus_ring(s: &mut Scene, r: Rect, dpr: f64) {
    let r = r.inset(-1.5 * dpr);
    let ring = RoundedRect::from_rect(r, 4.0 * dpr);
    s.stroke(&Stroke::new(3.0 * dpr), Affine::IDENTITY, peniko_color(with_alpha(ACCENT, 0.25)), None, &ring);
    s.stroke(&Stroke::new(1.5 * dpr), Affine::IDENTITY, peniko_color(with_alpha(ACCENT, 0.9)), None, &ring);
}

/// A tooltip under the pointer at `at`: `text`, then `keys` in grey, kept
/// inside a `size` window. Dark on light themes, light on dark ones.
#[allow(clippy::too_many_arguments)]
pub fn tooltip(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    at: Vec2,
    size: Vec2,
    dpr: f64,
    bg: Color,
    text: &str,
    keys_text: &str,
) {
    let ink = ink(bg);
    let mut t = Text { s, fonts, cache, size: 12.0 * dpr as f32, baseline: 0.0 };
    let pad = 8.0 * dpr;
    let keys_text = keys(t.fonts, keys_text);
    let text = t.clip(text, 400, size.x - 48.0 * dpr);
    let gap = if keys_text.is_empty() { 0.0 } else { 12.0 * dpr };
    let w = pad + t.width(&text, 400) + gap + t.width(&keys_text, 400) + pad;
    let h = 24.0 * dpr;
    // Below and right of the pointer, flipped to stay in the window.
    let mut x0 = at.x + 4.0 * dpr;
    let mut y0 = at.y + 20.0 * dpr;
    if x0 + w > size.x - 4.0 * dpr {
        x0 = (size.x - 4.0 * dpr - w).max(4.0 * dpr);
    }
    if y0 + h > size.y - 4.0 * dpr {
        y0 = at.y - 8.0 * dpr - h;
    }
    let card = Rect::new(x0, y0, x0 + w, y0 + h);
    shadow(t.s, card, 6.0 * dpr, dpr);
    let fill = ink.mix(bg, 0.08);
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &RoundedRect::from_rect(card, 6.0 * dpr));
    t.baseline = (card.center().y + 12.0 * dpr * 0.35) as f32;
    let tw = t.draw(&text, x0 + pad, 400, bg);
    t.draw(&keys_text, x0 + pad + tw + gap, 400, bg.mix(fill, 0.45));
}

// ---------------------------------------------------------------- toast

/// A short confirmation centred above the status bar, `alpha` for fading.
#[allow(clippy::too_many_arguments)]
pub fn toast(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    text: &str,
    alpha: f32,
) {
    let ink = ink(bg);
    let mut t = Text { s, fonts, cache, size: 13.0 * dpr as f32, baseline: 0.0 };
    let (pad, dot) = (14.0 * dpr, 7.0 * dpr);
    let text = t.clip(text, 500, size.x - 64.0 * dpr);
    let (w, h) = (pad + dot + 8.0 * dpr + t.width(&text, 500) + pad, 34.0 * dpr);
    let x0 = (size.x - w) / 2.0;
    let y0 = size.y - (BAR_HEIGHT + 16.0) * dpr - h;
    let card = RoundedRect::from_rect(Rect::new(x0, y0, x0 + w, y0 + h), h / 2.0);
    let mid = y0 + h / 2.0;
    let sh = with_alpha(Color::rgb(0, 0, 0), 0.16 * alpha);
    t.s.draw_blurred_rounded_rect(
        Affine::translate((0.0, 3.0 * dpr)),
        card.rect(),
        peniko_color(sh),
        h / 2.0,
        6.0 * dpr,
    );
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(with_alpha(ink, alpha)), None, &card);
    let c = Circle::new((x0 + pad + dot / 2.0, mid), dot / 2.0);
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(with_alpha(SUCCESS, alpha)), None, &c);
    t.baseline = mid as f32 + t.size * 0.35;
    t.draw(&text, x0 + pad + dot + 8.0 * dpr, 500, with_alpha(bg, alpha));
}

// --------------------------------------------------------------- banner

/// Draws the problems banner at the top of a `size` (pixels) window.
pub fn banner(s: &mut Scene, fonts: &Fonts, cache: &mut FontCache, size: Vec2, dpr: f64, bg: Color, p: &Problems) {
    let tone = if p.is_error() { ERROR } else { WARNING };
    let ink = ink(bg);
    let mut t = Text { s, fonts, cache, size: 13.0 * dpr as f32, baseline: 0.0 };
    let (pad, gap, icon) = (12.0 * dpr, 10.0 * dpr, 18.0 * dpr);
    let loc = p.at.map(|(l, c)| format!("{l}:{c}"));
    let more = p.more();
    let loc_w = loc.as_ref().map_or(0.0, |l| t.width(l, 600) + gap);
    let more_w = more.as_ref().map_or(0.0, |m| t.width(m, 500) + 2.0 * gap + gap);
    let max_w = (size.x - 32.0 * dpr).min(820.0 * dpr);
    let fixed = pad + icon + gap + loc_w + more_w + pad;
    let message = t.clip(&p.message, 400, max_w - fixed);
    let mut used = t.width(&message, 400);
    let help = p.help.as_ref().map(|h| t.clip(&format!("  ·  {h}"), 400, max_w - fixed - used)).unwrap_or_default();
    used += t.width(&help, 400);

    let (w, h) = (fixed + used, 38.0 * dpr);
    let x0 = ((size.x - w) / 2.0).max(16.0 * dpr);
    let card = RoundedRect::from_rect(Rect::new(x0, 12.0 * dpr, x0 + w, 12.0 * dpr + h), 10.0 * dpr);
    let mid = card.rect().center().y;
    shadow(t.s, card.rect(), 10.0 * dpr, dpr);
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg.mix(tone, 0.09)), None, &card);
    t.s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(tone.mix(bg, 0.45)), None, &card);

    let mut x = x0 + pad;
    let c = Circle::new((x + icon / 2.0, mid), icon / 2.0);
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(tone), None, &c);
    t.baseline = mid as f32 + t.size * 0.35;
    let mark_w = t.width("!", 700);
    t.draw("!", x + (icon - mark_w) / 2.0, 700, Color::rgb(255, 255, 255));
    x += icon + gap;
    if let Some(l) = &loc {
        t.draw(l, x, 600, tone.mix(ink, 0.35));
        x += loc_w;
    }
    x += t.draw(&message, x, 400, ink);
    t.draw(&help, x, 400, ink.mix(bg, 0.45));
    if let Some(m) = &more {
        let mw = t.width(m, 500);
        let pill = Rect::new(x0 + w - pad - mw - 2.0 * gap, mid - 11.0 * dpr, x0 + w - pad, mid + 11.0 * dpr);
        let rr = RoundedRect::from_rect(pill, 11.0 * dpr);
        t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(with_alpha(tone, 0.16)), None, &rr);
        t.draw(m, pill.x0 + gap, 500, tone.mix(ink, 0.35));
    }
}

// ------------------------------------------------------------- emphasis

/// Something drawn in the diagram that can be hovered or selected.
#[derive(Clone, Copy)]
pub enum Mark<'a> {
    Node(&'a NodeBox),
    Edge(&'a EdgeBox),
    Frame(&'a FrameBox),
}

impl Mark<'_> {
    fn same(self, other: Mark) -> bool {
        match (self, other) {
            (Mark::Node(a), Mark::Node(b)) => a.id == b.id,
            (Mark::Frame(a), Mark::Frame(b)) => a.id == b.id,
            (Mark::Edge(a), Mark::Edge(b)) => std::ptr::eq(a, b),
            _ => false,
        }
    }
}

/// Hover and selection over `d`, in scene coordinates (diagram minus the
/// bounds origin). `px` is diagram units per logical pixel, so strokes keep
/// their on-screen width at any zoom. A selected node also lights up its
/// edges, so its connections show.
#[allow(clippy::too_many_arguments)]
pub fn emphasis(
    s: &mut Scene,
    d: &Diagram,
    fonts: &Fonts,
    cache: &mut FontCache,
    px: f64,
    selected: Option<Mark>,
    hovered: Option<Mark>,
) {
    let base = Affine::translate((-d.bounds.x as f64, -d.bounds.y as f64));
    let ring = |n: &NodeBox, grow: f64| {
        let r = Rect::new(n.rect.x as f64, n.rect.y as f64, n.rect.right() as f64, n.rect.bottom() as f64);
        let radius = match n.shape {
            Shape::Pill | Shape::Ellipse | Shape::Circle => r.height() / 2.0,
            _ => 8.0,
        };
        RoundedRect::from_rect(r.inflate(grow * px, grow * px), radius + grow * px)
    };
    let accent = |a: f32| peniko_color(with_alpha(ACCENT, a));
    // Butt caps: a halo stops where its edge meets the node.
    let halo = |e: &EdgeBox, extra: f64| Stroke::new(e.width as f64 + extra * px);
    let restyled = |e: &EdgeBox| EdgeBox { color: ACCENT, width: e.width.max(1.5) + 0.75 * px as f32, ..e.clone() };

    let frame_ring = |f: &FrameBox, grow: f64| {
        let r = Rect::new(f.rect.x as f64, f.rect.y as f64, f.rect.right() as f64, f.rect.bottom() as f64);
        RoundedRect::from_rect(r.inflate(grow * px, grow * px), 12.0 + grow * px)
    };
    match hovered.filter(|h| !selected.is_some_and(|s| s.same(*h))) {
        Some(Mark::Frame(f)) => s.stroke(&Stroke::new(1.5 * px), base, accent(0.45), None, &frame_ring(f, 3.0)),
        Some(Mark::Node(n)) => {
            s.stroke(&Stroke::new(5.0 * px), base, accent(0.12), None, &ring(n, 3.5));
            s.stroke(&Stroke::new(1.5 * px), base, accent(0.55), None, &ring(n, 3.5));
        }
        Some(Mark::Edge(e)) => s.stroke(&halo(e, 7.0), base, accent(0.2), None, &draw::bez(&e.path)),
        None => {}
    }
    match selected {
        Some(Mark::Node(n)) => {
            for e in d.edges.iter().filter(|e| e.from == n.id || e.to == n.id) {
                draw::edge(s, fonts, cache, base, &restyled(e));
            }
            s.stroke(&Stroke::new(7.0 * px), base, accent(0.16), None, &ring(n, 4.0));
            s.stroke(&Stroke::new(2.0 * px), base, accent(1.0), None, &ring(n, 4.0));
        }
        Some(Mark::Edge(e)) => {
            s.stroke(&halo(e, 9.0), base, accent(0.22), None, &draw::bez(&e.path));
            draw::edge(s, fonts, cache, base, &restyled(e));
        }
        Some(Mark::Frame(f)) => {
            s.stroke(&Stroke::new(7.0 * px), base, accent(0.14), None, &frame_ring(f, 3.5));
            s.stroke(&Stroke::new(2.0 * px), base, accent(1.0), None, &frame_ring(f, 3.5));
        }
        None => {}
    }
}

// ----------------------------------------------------------------- text

/// `text`, cut short with an ellipsis to fit `max` pixels at `size`.
pub fn clip(fonts: &Fonts, text: &str, size: f32, weight: u16, max: f64) -> String {
    let width = |t: &str| fonts.shape(t, size, weight).map_or(0.0, |s| s.width as f64);
    if width(text) <= max {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cut = format!("{}…", chars.iter().collect::<String>().trim_end());
        if width(&cut) <= max {
            return cut;
        }
    }
    String::new()
}

/// One-line UI text at a shared size and baseline.
pub(crate) struct Text<'a> {
    pub s: &'a mut Scene,
    pub fonts: &'a Fonts,
    pub cache: &'a mut FontCache,
    pub size: f32,
    pub baseline: f32,
}

impl Text<'_> {
    pub fn width(&self, text: &str, weight: u16) -> f64 {
        self.fonts.shape(text, self.size, weight).map_or(0.0, |s| s.width as f64)
    }

    /// Draws `text` at `x`; returns its width.
    pub fn draw(&mut self, text: &str, x: f64, weight: u16, color: Color) -> f64 {
        draw::text(
            self.s,
            self.fonts,
            self.cache,
            Affine::IDENTITY,
            text,
            x as f32,
            self.baseline,
            self.size,
            weight,
            color,
        ) as f64
    }

    /// `text`, cut short with an ellipsis to fit `max` pixels.
    pub fn clip(&self, text: &str, weight: u16, max: f64) -> String {
        clip(self.fonts, text, self.size, weight, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_lang::{Syntax, load};

    fn problems(src: &str) -> Option<Problems> {
        Problems::new(src, &load(src, Syntax::Kdl).diagnostics)
    }

    #[test]
    fn banner_shows_the_first_problem() {
        let src = "rhizo version=1\n- \"Root\" shap=diamond {\n    - \"Child\" fill=\"#zzz\"\n}\n";
        let p = problems(src).unwrap();
        assert_eq!((p.errors, p.at), (2, Some((2, 10))));
        assert!(p.message.contains("shap"), "{}", p.message);
        assert_eq!(p.help.as_deref(), Some("did you mean `shape`?"));
        assert_eq!(p.more().as_deref(), Some("+1 error"));
        assert!(problems("rhizo version=1\n- \"Fine\"\n").is_none());
    }

    #[test]
    fn counting_the_rest() {
        let p = |errors, warnings| Problems { errors, warnings, ..Problems::single("x") };
        assert_eq!(p(1, 0).more(), None);
        assert_eq!(p(3, 0).more().as_deref(), Some("+2 errors"));
        assert_eq!(p(1, 1).more().as_deref(), Some("+1 warning"));
        assert_eq!(p(2, 3).more().as_deref(), Some("+1 error, 3 warnings"));
        assert_eq!(p(0, 1).more(), None);
        assert_eq!(p(0, 2).more().as_deref(), Some("+1 warning"));
        assert!(!p(0, 2).is_error());
    }

    /// An empty frame gets its hint; one with a node or a frame in it does not.
    #[test]
    fn empty_frames_are_found() {
        let src = "rhizo version=2\nframe \"Empty\" id=empty\nframe \"Full\" id=full {\n    node \"A\"\n}\nframe \"Outer\" id=outer {\n    frame \"Inner\" id=inner\n}\n";
        let doc = load(src, Syntax::Kdl).document;
        assert!(doc.frames.len() >= 4, "{:?}", doc.frames);
        let d = rhizo_layout::layout(&doc, &rhizo_core::Theme::default(), &rhizo_layout::text::Heuristic);
        let empty: Vec<&str> = empty_frames(&d).iter().map(|f| f.id.as_str()).collect();
        assert_eq!(empty, ["empty", "inner"]);
    }

    #[test]
    fn status_text() {
        let st = Status {
            file: "a.rhizo".into(),
            nodes: 1,
            edges: 12,
            zoom: 1.254,
            selection: None,
            saved: 0.0,
            project: (0, 0),
            job: None,
            hints: Vec::new(),
        };
        assert_eq!(st.counts(), "1 node · 12 edges");
        assert_eq!(st.zoom_text(), "125%");
        assert_eq!(describe_node("tokyo", "Tokyo"), "tokyo “Tokyo”");
        assert_eq!(describe_node("x", ""), "x");
        assert_eq!(describe_edge("a", "b", None), "edge a → b");
        assert_eq!(describe_edge("a", "b", Some("")), "edge a → b");
        assert_eq!(describe_edge("a", "b", Some("ships")), "edge a → b “ships”");
        assert_eq!(spell_keys("⇧⌘Z"), "Shift+Cmd+Z");
        assert_eq!(spell_keys("⌥ drag"), "Alt+drag");
        assert_eq!(spell_keys("⌫"), "Backspace");
    }
}
