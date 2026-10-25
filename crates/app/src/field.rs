//! One-line text fields: the inspector's props, the label typed on the
//! canvas and the palette's query. They edit like the text pane, on the same
//! [`Buffer`] (caret, selection, words, undo), so every text in the window
//! answers the same keys; this module adds what a single line needs: the
//! clipboard as text, IME composition, and drawing with a caret, a
//! selection and horizontal scrolling.

use rhizo_core::Color;
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, Line, Rect, Stroke};
use vello::peniko::Fill;

use crate::chrome::{ACCENT, with_alpha};
use crate::draw::{self, FontCache};
use crate::peniko_color;
use crate::textpane::{Buffer, Motion};

/// What a key does to a text, worked out from the key by the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Moves the caret; `true` extends the selection (shift).
    Move(Motion, bool),
    /// Removes the selection, or from the caret to where the motion goes.
    Delete(Motion),
    Type(String),
    Newline,
    Indent {
        outdent: bool,
    },
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
}

/// Applies `e` to `b`. `clipboard` holds the clipboard's text: Paste reads
/// it, Copy and Cut fill it. A `one_line` text takes no newline or indent,
/// and pasted lines join with spaces. True when the text changed.
pub fn apply(b: &mut Buffer, e: &Edit, clipboard: &mut String, one_line: bool) -> bool {
    let before = b.version;
    match e {
        Edit::Move(m, extend) => b.motion(*m, *extend),
        Edit::Delete(m) => b.delete(*m),
        Edit::Type(t) => b.type_text(t),
        Edit::Newline | Edit::Indent { .. } if one_line => {}
        Edit::Newline => b.newline(),
        Edit::Indent { outdent } => b.indent(*outdent),
        Edit::SelectAll => b.select_all(),
        Edit::Copy | Edit::Cut if b.selection().is_empty() => {}
        Edit::Copy => *clipboard = b.selected().to_string(),
        Edit::Cut => {
            *clipboard = b.selected().to_string();
            b.paste("");
        }
        Edit::Paste if one_line => b.paste(&clipboard.lines().map(str::trim).collect::<Vec<_>>().join(" ")),
        Edit::Paste => b.paste(clipboard),
        Edit::Undo => {
            b.undo();
        }
        Edit::Redo => {
            b.redo();
        }
    }
    b.version != before
}

/// A one-line text field: its text with caret, selection and undo, and an
/// input method's composition not yet committed.
#[derive(Default)]
pub struct Field {
    pub buffer: Buffer,
    pub preedit: String,
}

impl Field {
    /// A field holding `text`, all of it selected, so typing replaces it
    /// and arrows go to either end.
    pub fn new(text: &str) -> Field {
        let mut buffer = Buffer::new(text.to_string());
        buffer.select_all();
        Field { buffer, preedit: String::new() }
    }

    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// An input method's composition in progress, shown at the caret.
    pub fn compose(&mut self, text: String) {
        self.preedit = text;
    }

    /// The input method's result, typed over the selection.
    pub fn commit(&mut self, text: &str) {
        self.preedit.clear();
        self.buffer.type_text(text);
    }

    /// A click `x` pixels from where the text starts: 1 places the caret
    /// (`extend` with shift), 2 selects the word, 3 the whole line.
    pub fn click(&mut self, x: f64, clicks: u8, extend: bool, measure: impl Fn(&str) -> f64) {
        let o = offset_at(self.text(), x, measure);
        match clicks {
            1 => self.buffer.set_caret(o, extend),
            2 => self.buffer.select_word(o),
            _ => self.buffer.select_all(),
        }
    }
}

/// The char boundary of `text` nearest `x` pixels from its start.
pub fn offset_at(text: &str, x: f64, measure: impl Fn(&str) -> f64) -> usize {
    let mut last = (0, 0.0);
    for (i, _) in text.char_indices().skip(1).chain(std::iter::once((text.len(), ' '))) {
        let w = measure(&text[..i]);
        if w >= x {
            return if x - last.1 < w - x { last.0 } else { i };
        }
        last = (i, w);
    }
    text.len()
}

/// How far the text scrolls left so the caret stays inside `room` pixels.
pub fn scroll(f: &Field, room: f64, measure: impl Fn(&str) -> f64) -> f64 {
    let b = &f.buffer;
    let caret = measure(&b.text()[..b.head]) + measure(&f.preedit);
    (caret - room + 2.0).max(0.0)
}

/// How a field is drawn: font size and weight, ink, and whether it has
/// the keyboard (a caret and a bright selection) or not.
pub struct Look {
    pub size: f32,
    pub weight: u16,
    pub ink: Color,
    pub focused: bool,
    /// A line's width in pixels (caret, preedit underline).
    pub px: f64,
}

/// Draws `f` starting at `x` on `baseline`, within `room` pixels (scrolled
/// so the caret shows): the selection, the text, the composition underlined,
/// and the caret.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    f: &Field,
    x: f64,
    baseline: f64,
    room: f64,
    look: &Look,
) {
    let measure = |t: &str| fonts.shape(t, look.size, look.weight).map_or(0.0, |s| s.width as f64);
    let size = look.size as f64;
    let (top, bottom) = (baseline - size * 1.05, baseline + size * 0.35);
    s.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &Rect::new(x - 1.0, top, x + room + 2.0, bottom));
    let x = x - scroll(f, room, measure);
    let b = &f.buffer;
    let text = b.text();
    let sel = b.selection();
    if !sel.is_empty() {
        let r = Rect::new(x + measure(&text[..sel.start]), top, x + measure(&text[..sel.end]), bottom);
        let tint = with_alpha(ACCENT, if look.focused { 0.3 } else { 0.14 });
        s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(tint), None, &r);
    }
    // The composition sits at the caret, between the text before and after it.
    let (head, tail) = text.split_at(b.head);
    let w = draw::text(
        s,
        fonts,
        cache,
        Affine::IDENTITY,
        head,
        x as f32,
        baseline as f32,
        look.size,
        look.weight,
        look.ink,
    ) as f64;
    let mut cx = x + w;
    if !f.preedit.is_empty() {
        let pw = draw::text(
            s,
            fonts,
            cache,
            Affine::IDENTITY,
            &f.preedit,
            cx as f32,
            baseline as f32,
            look.size,
            400,
            look.ink,
        ) as f64;
        let under = Line::new((cx, baseline + 2.0 * look.px), (cx + pw, baseline + 2.0 * look.px));
        s.stroke(&Stroke::new(look.px), Affine::IDENTITY, peniko_color(look.ink), None, &under);
        cx += pw;
    }
    draw::text(s, fonts, cache, Affine::IDENTITY, tail, cx as f32, baseline as f32, look.size, look.weight, look.ink);
    if look.focused {
        let caret = Line::new((cx + 0.5, top + size * 0.1), (cx + 0.5, bottom));
        s.stroke(&Stroke::new(1.5 * look.px), Affine::IDENTITY, peniko_color(ACCENT), None, &caret);
    }
    s.pop_layer();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 10 pixels wide.
    fn mono(t: &str) -> f64 {
        t.chars().count() as f64 * 10.0
    }

    fn field(text: &str, caret: usize) -> Field {
        let mut f = Field::new(text);
        f.buffer.set_caret(caret, false);
        f
    }

    fn edit(f: &mut Field, e: Edit) -> bool {
        apply(&mut f.buffer, &e, &mut String::new(), true)
    }

    #[test]
    fn a_new_field_has_its_text_selected() {
        let mut f = Field::new("orders");
        assert_eq!(f.buffer.selected(), "orders");
        edit(&mut f, Edit::Type("/".into()));
        assert_eq!(f.text(), "/", "typing replaces it");
    }

    #[test]
    fn the_caret_moves_by_character_word_and_line() {
        let mut f = field("ship the order", 0);
        edit(&mut f, Edit::Move(Motion::Right, false));
        assert_eq!(f.buffer.head, 1);
        edit(&mut f, Edit::Move(Motion::WordRight, false));
        assert_eq!(f.buffer.head, 4, "⌥→ to the end of the word");
        edit(&mut f, Edit::Move(Motion::WordRight, false));
        assert_eq!(f.buffer.head, 8);
        edit(&mut f, Edit::Move(Motion::WordLeft, false));
        assert_eq!(f.buffer.head, 5, "⌥← to the start of the word");
        edit(&mut f, Edit::Move(Motion::LineEnd, false));
        assert_eq!(f.buffer.head, 14, "⌘→ to the end");
        edit(&mut f, Edit::Move(Motion::LineStart, false));
        assert_eq!(f.buffer.head, 0, "⌘← to the start");
        // Up and down in one line go to its ends, as in a Mac text field.
        edit(&mut f, Edit::Move(Motion::Down(1), false));
        assert_eq!(f.buffer.head, 14);
        edit(&mut f, Edit::Move(Motion::Up(1), false));
        assert_eq!(f.buffer.head, 0);
    }

    #[test]
    fn shift_selects_and_typing_or_deleting_replaces_the_selection() {
        let mut f = field("ship the order", 5);
        edit(&mut f, Edit::Move(Motion::WordRight, true));
        assert_eq!(f.buffer.selected(), "the");
        edit(&mut f, Edit::Type("an".into()));
        assert_eq!(f.text(), "ship an order");
        edit(&mut f, Edit::Move(Motion::LineStart, true));
        assert_eq!(f.buffer.selected(), "ship an");
        assert!(edit(&mut f, Edit::Delete(Motion::Left)));
        assert_eq!(f.text(), " order");
        edit(&mut f, Edit::Move(Motion::LineEnd, false));
        edit(&mut f, Edit::Delete(Motion::WordLeft));
        assert_eq!(f.text(), " ", "⌥⌫ deletes the word");
        // Collapsing a selection with an arrow goes to its edge.
        let mut f = field("abc", 0);
        edit(&mut f, Edit::SelectAll);
        edit(&mut f, Edit::Move(Motion::Left, false));
        assert_eq!((f.buffer.head, f.buffer.selected()), (0, ""));
    }

    #[test]
    fn clicks_place_the_caret_and_select_words_and_the_line() {
        let mut f = field("ship the order", 0);
        f.click(54.0, 1, false, mono);
        assert_eq!(f.buffer.head, 5, "nearest boundary");
        f.click(118.0, 1, true, mono);
        assert_eq!(f.buffer.selected(), "the ord");
        f.click(62.0, 2, false, mono);
        assert_eq!(f.buffer.selected(), "the");
        f.click(62.0, 3, false, mono);
        assert_eq!(f.buffer.selected(), "ship the order");
        assert_eq!(offset_at("ab", -5.0, mono), 0);
        assert_eq!(offset_at("ab", 500.0, mono), 2);
        assert_eq!(offset_at("éa", 6.0, mono), 2, "char boundaries, not bytes");
    }

    #[test]
    fn the_clipboard_copies_cuts_and_pastes_one_line() {
        let mut clip = String::new();
        let mut f = field("order total", 0);
        apply(&mut f.buffer, &Edit::Move(Motion::WordRight, true), &mut clip, true);
        assert!(!apply(&mut f.buffer, &Edit::Copy, &mut clip, true), "copy changes nothing");
        assert_eq!(clip, "order");
        assert!(apply(&mut f.buffer, &Edit::Cut, &mut clip, true));
        assert_eq!((f.text(), clip.as_str()), (" total", "order"));
        apply(&mut f.buffer, &Edit::Move(Motion::LineEnd, false), &mut clip, true);
        let mut lines = String::from("two\n lines ");
        apply(&mut f.buffer, &Edit::Paste, &mut lines, true);
        assert_eq!(f.text(), " totaltwo lines", "lines join with a space");
        // Copy with nothing selected leaves the clipboard alone.
        apply(&mut f.buffer, &Edit::Copy, &mut clip, true);
        assert_eq!(clip, "order");
        assert!(!apply(&mut f.buffer, &Edit::Newline, &mut clip, true), "one line takes no newline");
    }

    #[test]
    fn undo_steps_back_through_the_typing_before_it_is_written() {
        let mut f = field("30", 2);
        edit(&mut f, Edit::Type("0".into()));
        edit(&mut f, Edit::Move(Motion::LineStart, false));
        edit(&mut f, Edit::Type("1".into()));
        assert_eq!(f.text(), "1300");
        edit(&mut f, Edit::Undo);
        assert_eq!(f.text(), "300");
        edit(&mut f, Edit::Undo);
        assert_eq!(f.text(), "30");
        edit(&mut f, Edit::Redo);
        assert_eq!(f.text(), "300");
    }

    #[test]
    fn an_input_method_composes_then_commits_at_the_caret() {
        let mut f = field("東京", 6);
        f.compose("おおさか".into());
        assert_eq!(f.text(), "東京", "the composition is not text yet");
        f.commit("大阪");
        assert_eq!((f.text(), f.preedit.as_str()), ("東京大阪", ""));
        // Committing over a selection replaces it.
        f.buffer.select_all();
        f.commit("x");
        assert_eq!(f.text(), "x");
    }

    #[test]
    fn long_text_scrolls_to_keep_the_caret_in_view() {
        let f = field("abcdefghij", 10);
        assert_eq!(scroll(&f, 50.0, mono), 52.0);
        let f = field("abcdefghij", 2);
        assert_eq!(scroll(&f, 50.0, mono), 0.0);
    }
}
