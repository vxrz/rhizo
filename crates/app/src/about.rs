//! The footer (made in Erlangen, with the Franconian rake) that the
//! welcome screen shows, and the About dialog around it.

use rhizo_core::Color;
use rhizo_core::about::{BUILT_BY, COPYRIGHT, LICENSE, MADE_IN, TAGLINE, VERSION};
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, BezPath, Line, Rect, RoundedRect, Stroke, Vec2};
use vello::peniko::Fill;

use crate::chrome::{self, Text, ink};
use crate::draw::FontCache;
use crate::peniko_color;

/// What a click on the footer opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// The code's home, in the browser.
    GitHub,
    /// The About dialog.
    About,
}

/// Franconian red.
const GULES: Color = Color::hex(0xe30613);

/// The shield's outline, in a 20×24 box.
const SHIELD: &str = "M0 0 H20 V13 C20 19 15 22.4 10 24 C5 22.4 0 19 0 13 Z";
/// The silver base: per fess, dancetty of three points rising into the red.
const BASE: &str = "M0 13 L3.333 4 L6.667 13 L10 4 L13.333 13 L16.667 4 L20 13 V24 H0 Z";

fn path(d: &str) -> BezPath {
    BezPath::from_svg(d).expect("valid path data")
}

/// The Franconian rake ("Fränkischer Rechen", from the arms of Middle
/// Franconia) as a small shield `height` pixels tall, its top left at
/// `at`, outlined in `outline` so the white base reads on any background.
/// Drawn from the blazon, as `assets/brand/franconian-rake.svg`.
pub fn rake(s: &mut Scene, at: Vec2, height: f64, outline: Color) {
    let t = Affine::translate(at) * Affine::scale(height / 24.0);
    let shield = path(SHIELD);
    s.push_clip_layer(Fill::NonZero, t, &shield);
    s.fill(Fill::NonZero, t, peniko_color(GULES), None, &Rect::new(0.0, 0.0, 20.0, 24.0));
    s.fill(Fill::NonZero, t, peniko_color(Color::hex(0xffffff)), None, &path(BASE));
    s.pop_layer();
    // About 1/30 of the height on screen (at least a hairline), in the shield's own units.
    let width = (height / 30.0).max(0.8) * 24.0 / height;
    s.stroke(&Stroke::new(width).with_join(vello::kurbo::Join::Round), t, peniko_color(outline), None, &shield);
}

/// A heart for "Made with ❤️", drawn: UI fonts often have no emoji.
fn heart(s: &mut Scene, at: Vec2, size: f64) {
    let t = Affine::translate(at) * Affine::scale(size / 20.0);
    let d = "M10 18 C4 13.5 0.5 10.5 0.5 6.2 C0.5 3.1 2.9 0.8 5.8 0.8 C7.6 0.8 9.1 1.8 10 3.2 \
             C10.9 1.8 12.4 0.8 14.2 0.8 C17.1 0.8 19.5 3.1 19.5 6.2 C19.5 10.5 16 13.5 10 18 Z";
    s.fill(Fill::NonZero, t, peniko_color(GULES), None, &path(d));
}

/// A piece of a footer row.
enum Piece {
    Text(String),
    Heart,
    Rake,
    Link(&'static str, Link),
}

/// The footer's two rows, as the README and `rhizo about` have them.
fn rows() -> [Vec<Piece>; 2] {
    let (made, rest) = MADE_IN.split_once("❤️").unwrap_or((MADE_IN, ""));
    [
        vec![
            Piece::Text(made.trim_end().to_string() + " "),
            Piece::Heart,
            Piece::Text(format!(" {} ", rest.trim())),
            Piece::Rake,
            Piece::Text(format!(" · {BUILT_BY}")),
        ],
        vec![
            Piece::Text(format!("{COPYRIGHT} · Open source ({LICENSE}) · ")),
            Piece::Link("GitHub", Link::GitHub),
            Piece::Text(format!(" · rhizo {VERSION} · ")),
            Piece::Link("About rhizo", Link::About),
        ],
    ]
}

/// How the footer is drawn: text size and colours, the links under the
/// pointer, and whether "About rhizo" opens anything (not inside the dialog).
pub struct FooterLook {
    pub size: f32,
    pub ink: Color,
    pub link: Color,
    pub hover: Option<Link>,
    pub about_link: bool,
    pub px: f64,
}

/// Draws the footer's two rows centred on `center_x`, the first row's
/// baseline at `y`, shrunk to fit `max_w`. Returns the links' rects.
pub fn footer(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    center_x: f64,
    y: f64,
    max_w: f64,
    look: &FooterLook,
) -> Vec<(Rect, Link)> {
    let mut t = Text { s, fonts, cache, size: look.size, baseline: 0.0 };
    let rows = rows();
    let width = |t: &Text, row: &[Piece]| -> f64 {
        let em = t.size as f64;
        row.iter()
            .map(|p| match p {
                Piece::Text(s) => t.width(s, 400),
                Piece::Link(s, _) => t.width(s, 500),
                Piece::Heart => em * 0.9,
                Piece::Rake => em * 1.1 * 20.0 / 24.0,
            })
            .sum()
    };
    let widest = rows.iter().map(|r| width(&t, r)).fold(0.0, f64::max);
    if widest > max_w && widest > 0.0 {
        t.size *= (max_w / widest) as f32;
    }
    let em = t.size as f64;
    let mut hits = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let baseline = y + i as f64 * em * 1.75;
        t.baseline = baseline as f32;
        let mut x = center_x - width(&t, row) / 2.0;
        for p in row {
            match p {
                Piece::Text(s) => x += t.draw(s, x, 400, look.ink),
                Piece::Heart => {
                    heart(t.s, Vec2::new(x + em * 0.05, baseline - em * 0.78), em * 0.8);
                    x += em * 0.9;
                }
                Piece::Rake => {
                    let h = em * 1.1;
                    rake(t.s, Vec2::new(x, baseline - em * 0.92), h, look.ink);
                    x += h * 20.0 / 24.0;
                }
                Piece::Link(s, link) => {
                    let active = *link != Link::About || look.about_link;
                    let color = if active { look.link } else { look.ink };
                    let w = t.draw(s, x, 500, color);
                    if active {
                        if look.hover == Some(*link) {
                            let under = Line::new((x, baseline + 2.0 * look.px), (x + w, baseline + 2.0 * look.px));
                            t.s.stroke(&Stroke::new(look.px), Affine::IDENTITY, peniko_color(color), None, &under);
                        }
                        hits.push((Rect::new(x - 2.0, baseline - em, x + w + 2.0, baseline + em * 0.4), *link));
                    }
                    x += w;
                }
            }
        }
    }
    hits
}

/// Height of the footer drawn at `size`, from the first baseline to below the second row.
pub fn footer_height(size: f32) -> f64 {
    size as f64 * (1.75 + 0.5)
}

/// Draws the About dialog over a `size` (pixels) window. Returns the
/// panel (a click outside it closes the dialog) and the links in it.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    hover: Option<Link>,
) -> (Rect, Vec<(Rect, Link)>) {
    let px = |v: f64| v * dpr;
    let fg = ink(bg);
    let dark = bg.luminance() < 0.4;
    // The window dims behind the dialog.
    let mut dim = Color::rgb(0, 0, 0);
    dim.a = if dark { 110 } else { 60 };
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(dim), None, &Rect::new(0.0, 0.0, size.x, size.y));
    let (w, h) = (px(520.0).min(size.x - px(32.0)), px(256.0));
    let panel = Rect::from_center_size((size.x / 2.0, size.y / 2.0), (w, h));
    let card = RoundedRect::from_rect(panel, px(14.0));
    chrome::shadow(s, panel, px(14.0), dpr);
    let fill = if dark { bg.mix(fg, 0.06) } else { bg };
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &card);
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(fg.mix(bg, 0.8)), None, &card);

    let cx = panel.center().x;
    let mut y = panel.y0 + px(30.0);
    let logo_h = px(44.0);
    let k = logo_h / 142.0;
    crate::welcome::logo(s, Affine::translate((cx - 446.0 * k / 2.0, y)) * Affine::scale(k), dark);
    y += logo_h + px(30.0);
    let mut t = Text { s, fonts, cache, size: px(13.0) as f32, baseline: 0.0 };
    let centred = |t: &mut Text, text: &str, y: f64, weight: u16, color: Color| {
        let text = t.clip(text, weight, w - px(48.0));
        t.baseline = y as f32;
        let x = cx - t.width(&text, weight) / 2.0;
        t.draw(&text, x, weight, color);
    };
    centred(&mut t, &format!("Version {VERSION}"), y, 600, fg);
    y += px(24.0);
    t.size = px(13.5) as f32;
    centred(&mut t, TAGLINE, y, 400, fg.mix(fill, 0.25));
    y += px(28.0);
    let rule = Line::new((panel.x0 + px(32.0), y), (panel.x1 - px(32.0), y));
    t.s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(fg.mix(fill, 0.85)), None, &rule);
    y += px(30.0);
    let look = FooterLook {
        size: px(11.5) as f32,
        ink: fg.mix(fill, 0.4),
        link: chrome::ACCENT.mix(fg, 0.25),
        hover,
        about_link: false,
        px: dpr,
    };
    let hits = footer(t.s, t.fonts, t.cache, cx, y, w - px(40.0), &look);
    y += footer_height(look.size) + px(14.0);
    t.size = px(11.5) as f32;
    let licence = "Licensed under the MIT License or the Apache License 2.0, at your option.";
    centred(&mut t, licence, y, 400, fg.mix(fill, 0.5));
    (panel, hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_follow_the_shared_footer_text() {
        let text = |row: &[Piece]| -> String {
            row.iter()
                .map(|p| match p {
                    Piece::Text(s) => s.clone(),
                    Piece::Heart => "❤️".into(),
                    Piece::Rake => String::new(),
                    Piece::Link(s, _) => s.to_string(),
                })
                .collect()
        };
        let [one, two] = rows();
        let [want_one, want_two] = rhizo_core::about::footer_lines("GitHub");
        assert_eq!(text(&one).replace("  ", " "), want_one, "the rake sits after Metropolregion Nürnberg");
        assert_eq!(text(&two), want_two);
        assert!(matches!(one[3], Piece::Rake) && matches!(&one[2], Piece::Text(s) if s.ends_with("Nürnberg ")));
    }
}
