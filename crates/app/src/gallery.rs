//! The sample gallery (File ▸ Open Sample…, and the welcome screen): the
//! built-in samples ([`rhizo_project::samples`]) by group, each with a
//! thumbnail drawn from the file itself, a title and a line on what it shows.

use rhizo_core::Color;
use rhizo_interop::Format;
use rhizo_lang::Syntax;
use rhizo_project::samples::{GROUPS, SAMPLES, Sample};
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, Rect, RoundedRect, Stroke, Vec2};
use vello::peniko::Fill;

use crate::chrome::{self, ACCENT, Text};
use crate::draw::FontCache;
use crate::{Built, Engine, peniko_color};

/// The samples in the order the gallery shows them: by group.
pub fn ordered() -> Vec<&'static Sample> {
    GROUPS.iter().flat_map(|g| SAMPLES.iter().filter(move |s| s.group == *g)).collect()
}

/// A sample's first file as rhizo text: other formats (Mermaid, Markdown)
/// imported, as opening them does.
pub fn source(sample: &Sample) -> Result<String, String> {
    let (rel, src) = sample.files[0];
    match Format::from_path(std::path::Path::new(rel)) {
        Some(Format::Rhizo) | None => Ok(src.to_string()),
        Some(f) => rhizo_interop::import(src, f)
            .map(|(doc, _)| rhizo_lang::to_kdl(&doc))
            .map_err(|e| e.first().map_or_else(String::new, |d| d.message.clone())),
    }
}

/// The thumbnails of [`ordered`], drawn in `theme`; `None` where one
/// does not build.
pub fn thumbnails(engine: &mut Engine, theme: Option<&str>) -> Vec<Option<Built>> {
    ordered()
        .into_iter()
        .map(|s| {
            let src = source(s).ok()?;
            engine.prev = None;
            engine.build(s.files[0].0, &src, Syntax::Kdl, theme).ok()
        })
        .collect()
}

/// Where the gallery's parts are, for clicks.
#[derive(Clone, Debug, Default)]
pub struct Drawn {
    /// Each sample's tile, in [`ordered`] order.
    pub tiles: Vec<Rect>,
    pub close: Rect,
}

/// A row and a column of the gallery.
type Place = (usize, usize);

/// The tiles' places: groups flow along rows of `cols`, a group starting a
/// new row when it does not fit the one it would start in. Returns, per
/// sample of [`ordered`], its row and column, and where each group starts.
fn places(cols: usize) -> (Vec<Place>, Vec<Place>) {
    let (mut row, mut col) = (0, 0);
    let mut tiles = Vec::new();
    let mut starts = Vec::new();
    for g in GROUPS {
        let n = SAMPLES.iter().filter(|s| s.group == *g).count();
        if col > 0 && col + n > cols {
            (row, col) = (row + 1, 0);
        }
        starts.push((row, col));
        for _ in 0..n {
            if col == cols {
                (row, col) = (row + 1, 0);
            }
            tiles.push((row, col));
            col += 1;
        }
    }
    (tiles, starts)
}

/// Draws the gallery over a whole `size` window: `thumbs` as
/// [`thumbnails`] made them, `focus` and `hover` lit.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    thumbs: &[Option<Built>],
    focus: Option<usize>,
    hover: Option<usize>,
) -> Drawn {
    let ink = chrome::ink(bg);
    let dark = ink.r > 128;
    let px = |v: f64| v * dpr;
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg), None, &Rect::new(0.0, 0.0, size.x, size.y));
    let samples = ordered();
    let (gap, header, text_h, top) = (px(14.0), px(24.0), px(42.0), px(92.0));
    let cols = (((size.x - px(64.0) + gap) / (px(150.0) + gap)) as usize).clamp(2, 5);
    let (places, starts) = places(cols);
    let rows = places.iter().map(|p| p.0).max().unwrap_or(0) + 1;
    // As wide as the columns allow, as tall as the window allows.
    let by_width = (px(1000.0).min(size.x - px(64.0)) - gap * (cols - 1) as f64) / cols as f64;
    let by_height = ((size.y - top - px(24.0)) / rows as f64 - header - gap - text_h) / 0.6;
    let tw = by_width.min(by_height).max(px(80.0));
    let (th, row_h) = (tw * 0.6, header + tw * 0.6 + text_h + gap);
    let width = tw * cols as f64 + gap * (cols - 1) as f64;
    let x0 = (size.x - width) / 2.0;
    let y0 = top.max((size.y - rows as f64 * row_h) / 2.0 + px(20.0)).min(top + px(40.0));

    let mut t = Text { s, fonts, cache, size: px(20.0) as f32, baseline: (y0 - px(46.0)) as f32 };
    t.draw("Samples", x0, 650, ink);
    t.size = px(13.0) as f32;
    t.baseline = (y0 - px(24.0)) as f32;
    t.draw("Open one as a copy to explore and change: the original stays as it is.", x0, 400, ink.mix(bg, 0.45));
    let close_text = chrome::keys(t.fonts, "Close   Esc");
    let cw = t.width(&close_text, 500);
    t.baseline = (y0 - px(46.0)) as f32;
    t.draw(&close_text, x0 + width - cw, 500, ink.mix(bg, 0.4));
    let close = Rect::new(x0 + width - cw - px(6.0), y0 - px(64.0), x0 + width + px(6.0), y0 - px(38.0));

    let at = |(row, col): (usize, usize)| (x0 + col as f64 * (tw + gap), y0 + row as f64 * row_h);
    t.size = px(11.0) as f32;
    for (g, &start) in GROUPS.iter().zip(&starts) {
        let (x, y) = at(start);
        t.baseline = (y + px(14.0)) as f32;
        t.draw(&g.to_uppercase(), x + px(2.0), 600, ink.mix(bg, 0.5));
    }
    let mut tiles = Vec::with_capacity(samples.len());
    for (i, (sample, &place)) in samples.iter().zip(&places).enumerate() {
        let (x, y) = at(place);
        let card = Rect::new(x, y + header, x + tw, y + header + th);
        let lit = focus == Some(i) || hover == Some(i);
        let rr = RoundedRect::from_rect(card, px(10.0));
        let fill = if lit { bg.mix(ACCENT, if dark { 0.12 } else { 0.06 }) } else { bg.mix(ink, 0.03) };
        t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &rr);
        if let Some(b) = thumbs.get(i).and_then(Option::as_ref) {
            // The drawing, fitted inside with a margin, clipped to the card.
            let inner = card.inset(-px(10.0));
            let k = (inner.width() / b.width).min(inner.height() / b.height).min(dpr);
            let off = Vec2::new(inner.center().x - b.width * k / 2.0, inner.center().y - b.height * k / 2.0);
            t.s.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rr);
            t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(b.background.mix(fill, 0.3)), None, &rr);
            t.s.append(&b.scene, Some(Affine::translate(off) * Affine::scale(k)));
            t.s.pop_layer();
        }
        let (edge, w) = match (focus == Some(i), lit) {
            (true, _) => (ACCENT, 2.0),
            (_, true) => (ACCENT.mix(bg, 0.45), 1.0),
            _ => (ink.mix(bg, 0.88), 1.0),
        };
        t.s.stroke(&Stroke::new(px(w)), Affine::IDENTITY, peniko_color(edge), None, &rr);
        t.size = px(13.0) as f32;
        t.baseline = (card.y1 + px(18.0)) as f32;
        let title = t.clip(sample.title, 600, tw);
        t.draw(&title, x + px(2.0), 600, ink);
        t.size = px(11.5) as f32;
        t.baseline = (card.y1 + px(34.0)) as f32;
        let blurb = t.clip(sample.blurb, 400, tw - px(2.0));
        t.draw(&blurb, x + px(2.0), 400, ink.mix(bg, 0.5));
        tiles.push(Rect::new(card.x0, card.y0, card.x1, card.y1 + px(38.0)));
    }
    Drawn { tiles, close }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every sample shows once, by group; groups keep together and never
    /// share a column with the next one's tiles.
    #[test]
    fn gallery_lists_every_sample_by_group() {
        let o = ordered();
        assert_eq!(o.len(), SAMPLES.len());
        let groups: Vec<&str> = o.iter().map(|s| s.group).collect();
        let mut seen: Vec<&str> = groups.clone();
        seen.dedup();
        assert_eq!(seen, GROUPS, "each group once, in order");
        for cols in 2..=5 {
            let (tiles, starts) = places(cols);
            assert_eq!(tiles.len(), o.len());
            assert!(tiles.iter().all(|&(_, c)| c < cols), "{cols}");
            let mut sorted = tiles.clone();
            sorted.dedup();
            assert_eq!(sorted.len(), tiles.len(), "{cols}: no two tiles in one place");
            assert_eq!(starts.len(), GROUPS.len());
        }
        assert_eq!(places(5).1[0], (0, 0));
    }

    /// Every sample builds a thumbnail, imports included.
    #[test]
    fn every_sample_has_a_thumbnail() {
        let thumbs = thumbnails(&mut Engine::default(), Some("light"));
        for (s, t) in ordered().iter().zip(&thumbs) {
            let b = t.as_ref().unwrap_or_else(|| panic!("{}: no thumbnail", s.id));
            assert!(b.nodes > 0 && b.width > 0.0, "{}", s.id);
        }
        assert!(source(rhizo_project::samples::find("mermaid").unwrap()).unwrap().starts_with("rhizo version="));
    }
}
