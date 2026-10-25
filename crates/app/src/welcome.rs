//! The welcome screen, shown when no file is open (or the file to open does
//! not exist yet): new from a template, open a file or a folder, and recent
//! files and folders.

use std::path::{Path, PathBuf};

use rhizo_core::Color;
use rhizo_lang::starters::STARTERS;
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, BezPath, Circle, Line, Point, Rect, RoundedRect, Stroke, Vec2};
use vello::peniko::Fill;

use crate::chrome::{self, ACCENT, Text, ink, with_alpha};
use crate::draw::FontCache;
use crate::sidebar::{self, Glyph};
use crate::{about, files, peniko_color};

/// Recent files listed on the screen; ⇧⌘O lists them all.
const SHOWN_RECENT: usize = 6;

/// Something on the welcome screen that can be clicked or chosen with the keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A new file from this starter, saved where a dialog says.
    New(&'static str),
    /// The missing file, created from this starter.
    Create(&'static str),
    /// A copy of the tutorial (`rhizo new learn`).
    Learn,
    /// The gallery of samples.
    Samples,
    Open,
    /// A folder, as a project.
    OpenFolder,
    /// A recent file or folder.
    Recent(PathBuf),
    /// The footer's links: the code's home, and the About dialog.
    GitHub,
    About,
}

pub struct Welcome<'a> {
    /// The file the window is waiting for, when it does not exist yet.
    pub missing: Option<&'a Path>,
    pub recent: &'a [PathBuf],
}

impl Welcome<'_> {
    /// Everything selectable, in keyboard order.
    pub fn items(&self) -> Vec<Item> {
        let make = if self.missing.is_some() { Item::Create } else { Item::New };
        let mut items: Vec<Item> = STARTERS.iter().map(|&(kind, _)| make(kind)).collect();
        if self.missing.is_none() {
            items.extend([Item::Learn, Item::Samples]);
        }
        items.push(Item::Open);
        items.push(Item::OpenFolder);
        items.extend(self.recent.iter().take(SHOWN_RECENT).cloned().map(Item::Recent));
        items.extend([Item::GitHub, Item::About]);
        items
    }
}

/// What each starter is for, under its name.
fn blurb(kind: &str) -> &'static str {
    match kind {
        "mindmap" => "Ideas around a root",
        "flow" => "Steps and decisions",
        "canvas" => "Place things freely",
        _ => "Entities and an API",
    }
}

/// The brand colors: seed, core, three branches, two sprouts, the cross-link.
struct Brand {
    seed: Color,
    core: Color,
    branches: [Color; 3],
    sprouts: [Color; 2],
    link: Color,
}

fn brand(dark: bool) -> Brand {
    if dark {
        Brand {
            seed: Color::hex(0xf8f8fc),
            core: Color::hex(0x6366f1),
            branches: [Color::hex(0x818cf8), Color::hex(0x38bdf8), Color::hex(0x34d399)],
            sprouts: [Color::hex(0xfbbf24), Color::hex(0xf472b6)],
            link: with_alpha(Color::hex(0xf5f3ff), 0.45),
        }
    } else {
        Brand {
            seed: Color::hex(0x14131f),
            core: Color::hex(0x6366f1),
            branches: [Color::hex(0x6366f1), Color::hex(0x0ea5e9), Color::hex(0x10b981)],
            sprouts: [Color::hex(0xf59e0b), Color::hex(0xec4899)],
            link: with_alpha(Color::hex(0x14131f), 0.3),
        }
    }
}

fn path(d: &str) -> BezPath {
    BezPath::from_svg(d).expect("valid path data")
}

/// The logo lockup (mark and wordmark, from `assets/brand/logo-*.svg`), 446×142 units.
pub(crate) fn logo(s: &mut Scene, at: Affine, dark: bool) {
    let b = brand(dark);
    let c = |c: Color| peniko_color(c);
    // The mark, from its 600×712 viewBox at (236, 152) into 120×142.
    let m = at * Affine::scale(142.0 / 712.0) * Affine::translate((-236.0, -152.0));
    let round = |w: f64| Stroke::new(w).with_caps(vello::kurbo::Cap::Round);
    let branches =
        ["M380 512 C 500 512, 500 292, 610 292", "M380 512 L 660 512", "M380 512 C 490 512, 490 736, 590 736"];
    for (d, col) in branches.iter().zip(b.branches) {
        s.stroke(&round(38.0), m, c(col), None, &path(d));
    }
    let sprouts = ["M650 292 C 720 292, 718 206, 780 206", "M630 736 C 700 736, 698 812, 760 812"];
    for (d, col) in sprouts.iter().zip(b.sprouts) {
        s.stroke(&round(22.0), m, c(col), None, &path(d));
    }
    let link = round(12.0).with_dashes(0.0, [2.0, 26.0]);
    s.stroke(&link, m, c(b.link), None, &path("M736 548 C 790 610, 800 700, 790 770"));
    for ((x, y, r), col) in
        [(646.0, 292.0, 46.0), (702.0, 512.0, 46.0), (626.0, 736.0, 46.0)].into_iter().zip(b.branches)
    {
        s.fill(Fill::NonZero, m, c(col), None, &Circle::new((x, y), r));
    }
    for ((x, y), col) in [(804.0, 206.0), (784.0, 812.0)].into_iter().zip(b.sprouts) {
        s.fill(Fill::NonZero, m, c(col), None, &Circle::new((x, y), 30.0));
    }
    s.fill(Fill::NonZero, m, c(b.seed), None, &Circle::new((356.0, 512.0), 88.0));
    s.fill(Fill::NonZero, m, c(b.core), None, &Circle::new((356.0, 512.0), 30.0));

    // The wordmark, a monoline drawing (never set in a font), 312×110 at (150, 22).
    let w = at * Affine::translate((150.0, 22.0)) * Affine::scale(0.95);
    let pen = round(14.0).with_join(vello::kurbo::Join::Round);
    let ink = c(b.seed);
    for d in [
        "M7 40 L7 100 M7 64 C7 48 17 40 33 40",
        "M63 10 L63 100 M63 64 C63 48 73 40 89 40 C105 40 113 48 113 64 L113 100",
        "M143 44 L143 100",
        "M173 40 L213 40 L173 100 L213 100",
    ] {
        s.stroke(&pen, w, ink, None, &path(d));
    }
    s.stroke(&pen, w, ink, None, &Circle::new((273.0, 70.0), 30.0));
    s.fill(Fill::NonZero, w, c(b.branches[0]), None, &Circle::new((143.0, 17.0), 11.0));
}

/// A small picture of a starter, centred on `at`, `u` pixels per unit.
fn icon(s: &mut Scene, kind: &str, at: Point, u: f64, dark: bool, bg: Color) {
    let b = brand(dark);
    let t = Affine::translate(at.to_vec2()) * Affine::scale(u);
    let line = Stroke::new(2.0);
    let soft = peniko_color(ink(bg).mix(bg, 0.6));
    let [indigo, sky, emerald] = b.branches.map(peniko_color);
    let boxed = |s: &mut Scene, r: Rect, radius: f64, col| {
        let rr = RoundedRect::from_rect(r, radius);
        s.fill(Fill::NonZero, t, peniko_color(bg), None, &rr);
        s.stroke(&line, t, col, None, &rr);
    };
    match kind {
        "mindmap" => {
            let ends = [(14.0, -11.0), (18.0, 0.0), (14.0, 11.0)];
            for ((x, y), col) in ends.into_iter().zip([indigo, sky, emerald]) {
                let mut p = BezPath::new();
                p.move_to((-18.0, 0.0));
                p.curve_to((-2.0, 0.0), (-2.0, y), (x, y));
                s.stroke(&Stroke::new(2.5), t, col, None, &p);
                s.fill(Fill::NonZero, t, col, None, &Circle::new((x, y), 4.0));
            }
            s.fill(Fill::NonZero, t, peniko_color(b.seed), None, &Circle::new((-19.0, 0.0), 6.5));
            s.fill(Fill::NonZero, t, peniko_color(b.core), None, &Circle::new((-19.0, 0.0), 2.4));
        }
        "flow" => {
            s.stroke(&line, t, soft, None, &Line::new((-14.0, 0.0), (14.0, 0.0)));
            boxed(s, Rect::new(-29.0, -6.0, -14.0, 6.0), 6.0, indigo);
            let mut d = BezPath::new();
            d.move_to((0.0, -9.0));
            d.line_to((9.0, 0.0));
            d.line_to((0.0, 9.0));
            d.line_to((-9.0, 0.0));
            d.close_path();
            s.fill(Fill::NonZero, t, peniko_color(bg), None, &d);
            s.stroke(&line, t, sky, None, &d);
            boxed(s, Rect::new(14.0, -6.0, 29.0, 6.0), 2.0, emerald);
        }
        "canvas" => {
            let dots = Stroke::new(1.5).with_dashes(0.0, [2.0, 3.0]);
            s.stroke(&dots, t, soft, None, &Line::new((-17.0, -8.0), (15.0, -5.0)));
            s.stroke(&dots, t, soft, None, &Line::new((-17.0, -8.0), (-1.0, 10.0)));
            boxed(s, Rect::new(-27.0, -14.0, -9.0, -3.0), 3.0, indigo);
            boxed(s, Rect::new(6.0, -11.0, 26.0, 0.0), 3.0, sky);
            boxed(s, Rect::new(-10.0, 5.0, 8.0, 16.0), 5.5, emerald);
        }
        _ => {
            let card = RoundedRect::new(-20.0, -15.0, 20.0, 15.0, 4.0);
            s.fill(Fill::NonZero, t, peniko_color(bg), None, &card);
            let mut band = b.branches[0];
            band.a = 60;
            s.fill(
                Fill::NonZero,
                t,
                peniko_color(band),
                None,
                &RoundedRect::new(-20.0, -15.0, 20.0, -6.0, (4.0, 4.0, 0.0, 0.0)),
            );
            s.stroke(&line, t, indigo, None, &card);
            for y in [0.0, 8.0] {
                s.fill(Fill::NonZero, t, emerald, None, &Circle::new((-13.0, y), 2.0));
                s.stroke(
                    &Stroke::new(2.0),
                    t,
                    soft,
                    None,
                    &Line::new((-8.0, y), (if y == 0.0 { 12.0 } else { 6.0 }, y)),
                );
            }
        }
    }
}

/// Draws the welcome screen over a `size` (pixels) window and returns where
/// each of [`Welcome::items`] is, for clicks. `focus` is the item chosen
/// with the keys, `hover` the one under the pointer.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    w: &Welcome,
    focus: Option<usize>,
    hover: Option<usize>,
) -> Vec<Rect> {
    let ink = ink(bg);
    let dark = ink.r > 128;
    let items = w.items();
    let recent = items.iter().filter(|i| matches!(i, Item::Recent(_))).count();
    let rows = 2 + recent;
    let cards = items.iter().filter(|i| matches!(i, Item::Learn | Item::Samples)).count();
    let px = |v: f64| v * dpr;
    let card_h = if cards > 0 { px(52.0) + px(12.0) } else { 0.0 };

    let col_w = px(600.0).min(size.x - px(64.0));
    let tile_h = px(112.0);
    let row_h = px(38.0);
    let intro_h = if w.missing.is_some() { px(96.0) } else { px(40.0) };
    let total =
        px(46.0) + intro_h + px(30.0) + px(24.0) + tile_h + card_h + px(36.0) + px(24.0) + row_h * (2 + recent) as f64;
    let x0 = (size.x - col_w) / 2.0;
    let mut y = ((size.y - total) / 2.0 - px(16.0)).max(px(32.0));

    let mut t = Text { s, fonts, cache, size: 0.0, baseline: 0.0 };
    // Logo, centred.
    let logo_h = px(46.0);
    let k = logo_h / 142.0;
    logo(t.s, Affine::translate(((size.x - 446.0 * k) / 2.0, y)) * Affine::scale(k), dark);
    y += logo_h;

    let centred = |t: &mut Text, text: &str, y: f64, weight: u16, color: Color| {
        let text = t.clip(text, weight, col_w);
        let x = (size.x - t.width(&text, weight)) / 2.0;
        t.baseline = y as f32;
        t.draw(&text, x, weight, color);
    };
    match w.missing {
        Some(p) => {
            t.size = px(17.0) as f32;
            centred(&mut t, &format!("{} does not exist yet", files::name(p)), y + px(40.0), 600, ink);
            t.size = px(13.5) as f32;
            let dir = p.parent().map(files::display_path).unwrap_or_default();
            let line = format!("Pick a template to create it in {dir}.");
            centred(&mut t, &line, y + px(64.0), 400, ink.mix(bg, 0.42));
            let wait = "Or keep this window open: it shows the file as soon as it appears.";
            centred(&mut t, wait, y + px(84.0), 400, ink.mix(bg, 0.42));
        }
        None => {
            t.size = px(15.0) as f32;
            centred(&mut t, "Think in graphs. Ship in any format.", y + px(34.0), 400, ink.mix(bg, 0.42));
        }
    }
    y += intro_h + px(30.0);

    let header = |t: &mut Text, text: &str, y: f64| {
        t.size = px(11.5) as f32;
        t.baseline = y as f32;
        t.draw(text, x0 + px(2.0), 600, ink.mix(bg, 0.5));
    };
    header(&mut t, if w.missing.is_some() { "CREATE FROM A TEMPLATE" } else { "NEW" }, y + px(12.0));
    y += px(24.0);

    let lit = |i: usize| (focus == Some(i), hover == Some(i));
    let mut rects = Vec::with_capacity(items.len());
    let gap = px(12.0);
    let tile_w = (col_w - gap * 3.0) / 4.0;
    for (i, &(kind, name)) in STARTERS.iter().enumerate() {
        let r = Rect::new(x0 + i as f64 * (tile_w + gap), y, x0 + i as f64 * (tile_w + gap) + tile_w, y + tile_h);
        let (focused, hovered) = lit(i);
        let card = RoundedRect::from_rect(r, px(12.0));
        let fill = if hovered || focused { bg.mix(ACCENT, if dark { 0.12 } else { 0.06 }) } else { bg.mix(ink, 0.03) };
        t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &card);
        let (edge, width) = match (focused, hovered) {
            (true, _) => (ACCENT, 2.0),
            (_, true) => (ACCENT.mix(bg, 0.45), 1.0),
            _ => (ink.mix(bg, 0.88), 1.0),
        };
        t.s.stroke(&Stroke::new(px(width)), Affine::IDENTITY, peniko_color(edge), None, &card);
        icon(t.s, kind, Point::new(r.center().x, y + px(38.0)), dpr, dark, fill);
        t.size = px(13.5) as f32;
        t.baseline = (y + px(78.0)) as f32;
        let nw = t.width(name, 600);
        t.draw(name, r.center().x - nw / 2.0, 600, ink);
        t.size = px(11.5) as f32;
        t.baseline = (y + px(96.0)) as f32;
        let blurb = t.clip(blurb(kind), 400, tile_w - px(12.0));
        let bw = t.width(&blurb, 400);
        t.draw(&blurb, r.center().x - bw / 2.0, 400, ink.mix(bg, 0.5));
        rects.push(r);
    }
    y += tile_h;

    // Two wide cards: the tutorial and the samples.
    let card_w = (col_w - gap) / 2.0;
    for (k, item) in items.iter().enumerate().skip(STARTERS.len()).take(cards) {
        let x = x0 + (k - STARTERS.len()) as f64 * (card_w + gap);
        let r = Rect::new(x, y + px(12.0), x + card_w, y + px(12.0) + px(52.0));
        let (focused, hovered) = lit(k);
        let rr = RoundedRect::from_rect(r, px(12.0));
        let fill = if hovered || focused { bg.mix(ACCENT, if dark { 0.12 } else { 0.06 }) } else { bg.mix(ink, 0.03) };
        t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &rr);
        let (edge, width) = match (focused, hovered) {
            (true, _) => (ACCENT, 2.0),
            (_, true) => (ACCENT.mix(bg, 0.45), 1.0),
            _ => (ink.mix(bg, 0.88), 1.0),
        };
        t.s.stroke(&Stroke::new(px(width)), Affine::IDENTITY, peniko_color(edge), None, &rr);
        let (title, line) = match item {
            Item::Learn => ("Learn rhizo", "A hands-on tutorial in ten steps"),
            _ => ("Samples", "Mindmaps, flows, APIs, infrastructure…"),
        };
        let mark = Point::new(r.x0 + px(24.0), r.center().y);
        t.s.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            peniko_color(ACCENT.mix(bg, 0.8)),
            None,
            &Circle::new(mark, px(11.0)),
        );
        if *item == Item::Learn {
            t.size = px(13.0) as f32;
            t.baseline = (mark.y + px(13.0) * 0.36) as f32;
            let gw = t.width("?", 700);
            t.draw("?", mark.x - gw / 2.0, 700, ACCENT);
        } else {
            // Four small tiles: a gallery.
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let c = Point::new(mark.x + dx * px(3.2), mark.y + dy * px(3.2));
                let sq = Rect::from_center_size(c, (px(5.0), px(5.0)));
                t.s.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    peniko_color(ACCENT),
                    None,
                    &RoundedRect::from_rect(sq, px(1.2)),
                );
            }
        }
        t.size = px(13.5) as f32;
        t.baseline = (r.center().y - px(3.0)) as f32;
        t.draw(title, r.x0 + px(46.0), 600, ink);
        t.size = px(11.5) as f32;
        t.baseline = (r.center().y + px(13.0)) as f32;
        let line = t.clip(line, 400, card_w - px(56.0));
        t.draw(&line, r.x0 + px(46.0), 400, ink.mix(bg, 0.5));
        rects.push(r);
    }
    y += card_h + px(36.0);

    header(&mut t, if recent > 0 { "OPEN" } else { "OPEN A FILE" }, y + px(12.0));
    y += px(24.0);
    let size_pt = px(14.0) as f32;
    let keys_open = chrome::keys(t.fonts, "⌘O");
    let keys_folder = chrome::keys(t.fonts, "⌥⌘O");
    for (i, item) in items.iter().enumerate().skip(STARTERS.len() + cards).take(rows) {
        let r = Rect::new(x0, y, x0 + col_w, y + row_h);
        let (focused, hovered) = lit(i);
        let rr = RoundedRect::from_rect(r, px(8.0));
        if hovered || focused {
            let fill = bg.mix(ACCENT, if dark { 0.12 } else { 0.06 });
            t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &rr);
        }
        if focused {
            t.s.stroke(&Stroke::new(px(2.0)), Affine::IDENTITY, peniko_color(ACCENT), None, &rr);
        }
        t.size = size_pt;
        t.baseline = (r.center().y + px(14.0) * 0.35) as f32;
        let x = x0 + px(14.0);
        let glyph_at = Point::new(x + px(6.5), r.center().y);
        match item {
            Item::Recent(p) => {
                sidebar::glyph(t.s, if p.is_dir() { Glyph::Folder } else { Glyph::File }, glyph_at, dpr, bg);
                let name = files::name(p);
                let nx = x + px(22.0);
                let used = t.draw(&name, nx, 500, ink);
                t.size = px(12.5) as f32;
                let dir = p.parent().map(files::display_path).unwrap_or_default();
                let dir = t.clip(&dir, 400, (x0 + col_w - nx - used - px(26.0)).max(0.0));
                t.draw(&dir, nx + used + px(12.0), 400, ink.mix(bg, 0.5));
            }
            _ => {
                // "Open…" or "Open folder…", and the shortcut.
                let (glyph, label, keys) = match item {
                    Item::OpenFolder => (Glyph::Folder, "Open folder…", &keys_folder),
                    _ => (Glyph::File, "Open…", &keys_open),
                };
                sidebar::glyph(t.s, glyph, glyph_at, dpr, bg);
                t.draw(label, x + px(22.0), 500, ink);
                t.size = px(12.5) as f32;
                let kw = t.width(keys, 400);
                t.draw(keys, x0 + col_w - px(14.0) - kw, 400, ink.mix(bg, 0.5));
            }
        }
        rects.push(r);
        y += row_h;
    }

    // What else works, then the footer, along the bottom.
    let footer_size = px(11.5) as f32;
    let footer_y = (size.y - px(18.0) - footer_size as f64 * 1.75).max(y + px(64.0));
    t.size = px(12.0) as f32;
    let hint = format!("Drop a file here to open it   ·   {}", chrome::keys(t.fonts, chrome::HINT));
    centred(&mut t, &hint, footer_y - px(30.0), 400, ink.mix(bg, 0.55));
    let first_link = STARTERS.len() + cards + rows;
    let lit_link = |link: about::Link| {
        let i = first_link + usize::from(link == about::Link::About);
        focus == Some(i) || hover == Some(i)
    };
    let look = about::FooterLook {
        size: footer_size,
        ink: ink.mix(bg, 0.5),
        link: ink.mix(bg, 0.25),
        hover: [about::Link::GitHub, about::Link::About].into_iter().find(|l| lit_link(*l)),
        about_link: true,
        px: dpr,
    };
    let links = about::footer(t.s, t.fonts, t.cache, size.x / 2.0, footer_y, size.x - px(32.0), &look);
    for (r, link) in links {
        if focus == Some(first_link + usize::from(link == about::Link::About)) {
            let ring = RoundedRect::from_rect(r.inflate(px(3.0), px(2.0)), px(4.0));
            t.s.stroke(&Stroke::new(px(2.0)), Affine::IDENTITY, peniko_color(ACCENT), None, &ring);
        }
        rects.push(r);
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_in_keyboard_order() {
        let recent: Vec<PathBuf> = (0..9).map(|i| PathBuf::from(format!("/r{i}.rhizo"))).collect();
        let w = Welcome { missing: None, recent: &recent };
        let items = w.items();
        assert_eq!(items[0], Item::New("mindmap"));
        assert_eq!(items[4..6], [Item::Learn, Item::Samples], "the tutorial and the samples after the templates");
        assert_eq!(items[6], Item::Open);
        assert_eq!(items[7], Item::OpenFolder);
        assert_eq!(items.len(), 4 + 2 + 2 + SHOWN_RECENT + 2);
        assert_eq!(items[items.len() - 2..], [Item::GitHub, Item::About], "the footer's links come last");
        let missing = Welcome { missing: Some(Path::new("/x/a.rhizo")), recent: &[] };
        assert_eq!(missing.items()[3], Item::Create("data"));
        assert_eq!(missing.items().len(), 8, "waiting for a file: no tutorial or samples");
    }
}
