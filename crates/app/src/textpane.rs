//! The text pane: the document's KDL beside the canvas, highlighted, with
//! line numbers, and editable. This module holds the parts that need no
//! window: highlighting, the buffer with its caret, selection and undo
//! history, where nodes and edges sit in the text, the rules for saving and
//! for outside changes, and drawing into a scene.
//!
//! The editor is hand-rolled on a monospace grid rather than built on
//! parley's `PlainEditor`: parley 0.12 (the release that matches vello 0.11)
//! needs Rust 1.88 against the workspace's 1.85, brings a second font system,
//! and lays the whole buffer out again on every change, where a grid only
//! touches the lines on screen.

use std::ops::Range;
use std::time::{Duration, Instant};

use rhizo_core::{Color, NodeId};
use rhizo_lang::Diagnostic;
use rhizo_render::Fonts;
use vello::kurbo::{Affine, BezPath, Line, Rect, Stroke, Vec2};
use vello::peniko::Fill;
use vello::{Glyph, Scene};

use crate::chrome::{self, ACCENT, ERROR, WARNING};
use crate::draw::FontCache;
use crate::peniko_color;

/// How long typing pauses before the text is checked, and saved if it parses.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// Monospace families, first installed wins.
pub const MONO_STACK: &str =
    "SF Mono, Menlo, JetBrains Mono, Cascadia Mono, Consolas, DejaVu Sans Mono, Liberation Mono, monospace";
const TAB: usize = 4;
/// Undo steps kept; each is a copy of the text.
const HISTORY: usize = 200;
/// Typing within this long of the last key joins its undo step.
const RUN: Duration = Duration::from_secs(1);

// ------------------------------------------------------------ highlighting

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A statement's name: `-`, `node`, `edge`, `frame`, `style`…
    Keyword,
    String,
    /// A property name, before `=`.
    Property,
    /// A bare value or argument: an id, `diamond`, a type annotation.
    Ident,
    Number,
    /// `#true`, `#false`, `#null`, `#inf`…
    Literal,
    Comment,
    Punct,
}

/// A highlighted byte range. Whitespace between tokens has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

fn delimiter(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\\' | b'/' | b'(' | b')' | b'{' | b'}' | b';' | b'[' | b']')
        || matches!(c, b'=' | b'"' | b'#')
}

/// Where a string opened at `i` (its first quote) ends. `hashes` > 0 for
/// raw strings. An unclosed one-line string stops at the line end, so a
/// quote being typed does not paint the rest of the file.
fn string_end(b: &[u8], i: usize, hashes: usize) -> usize {
    let multi = b[i..].starts_with(b"\"\"\"");
    let close: &[u8] = if multi { b"\"\"\"" } else { b"\"" };
    let mut j = i + close.len();
    while j < b.len() {
        if hashes == 0 && b[j] == b'\\' {
            j += 2;
            continue;
        }
        if !multi && b[j] == b'\n' {
            return j;
        }
        let k = j + close.len();
        if b[j..].starts_with(close) && b.len() >= k + hashes && b[k..k + hashes].iter().all(|&h| h == b'#') {
            return k + hashes;
        }
        j += 1;
    }
    b.len()
}

/// A raw string opening at `i`: where its quote is, and how many `#` it has.
fn raw_open(b: &[u8], i: usize) -> Option<(usize, usize)> {
    let skip = usize::from(b[i] == b'r');
    let hashes = b[i + skip..].iter().take_while(|&&h| h == b'#').count();
    (b.get(i + skip + hashes) == Some(&b'"')).then_some((i + skip + hashes, hashes))
}

/// Splits KDL into tokens for coloring. Forgiving: any text gives tokens,
/// so half-typed statements still color sensibly.
pub fn highlight(src: &str) -> Vec<Token> {
    let b = src.as_bytes();
    let n = b.len();
    let at = |j: usize| b.get(j).copied().unwrap_or(0);
    let mut out = Vec::new();
    let mut i = 0;
    // The next name starts a statement.
    let mut statement = true;
    // Inside `(type)`.
    let mut annotation = false;
    // After `\`: the next newline continues the statement.
    let mut continued = false;
    while i < n {
        let start = i;
        let c = b[i];
        let kind = match c {
            b' ' | b'\t' | b'\r' => {
                i += 1;
                continue;
            }
            b'\n' => {
                i += 1;
                statement |= !std::mem::take(&mut continued);
                continue;
            }
            b';' | b'{' | b'}' => {
                i += 1;
                statement = true;
                Kind::Punct
            }
            b'/' if at(i + 1) == b'/' => {
                i = src[i..].find('\n').map_or(n, |k| i + k);
                Kind::Comment
            }
            b'/' if at(i + 1) == b'*' => {
                let mut depth = 0;
                while i < n {
                    if b[i..].starts_with(b"/*") {
                        depth += 1;
                        i += 2;
                    } else if b[i..].starts_with(b"*/") {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                Kind::Comment
            }
            b'/' if at(i + 1) == b'-' => {
                i += 2;
                Kind::Comment
            }
            b'(' | b')' => {
                annotation = c == b'(';
                i += 1;
                Kind::Punct
            }
            b'=' | b'\\' | b'/' | b'[' | b']' => {
                continued |= c == b'\\';
                i += 1;
                Kind::Punct
            }
            b'"' => {
                i = string_end(b, i, 0).min(n);
                statement = false;
                if at(i) == b'=' { Kind::Property } else { Kind::String }
            }
            b'#' | b'r' if raw_open(b, i).is_some() => {
                // `#"raw"#`, `##"raw"##`, and KDL 1's `r"raw"` / `r#"raw"#`.
                let (quote, hashes) = raw_open(b, i).expect("checked");
                i = string_end(b, quote, hashes).min(n);
                statement = false;
                Kind::String
            }
            b'#' => {
                i += 1;
                while i < n && !delimiter(b[i]) {
                    i += 1;
                }
                statement = false;
                Kind::Literal
            }
            _ => {
                while i < n && !delimiter(b[i]) {
                    i += 1;
                }
                let word = &b[start..i];
                let digit = |k: usize| word.get(k).is_some_and(u8::is_ascii_digit);
                let number = digit(0) || (matches!(word[0], b'+' | b'-' | b'.') && digit(1));
                let kind = if at(i) == b'=' {
                    Kind::Property
                } else if number {
                    Kind::Number
                } else if annotation {
                    Kind::Ident
                } else if statement {
                    Kind::Keyword
                } else {
                    Kind::Ident
                };
                if !annotation {
                    statement = false;
                }
                kind
            }
        };
        out.push(Token { start, end: i, kind });
    }
    out
}

// ------------------------------------------------------------------ buffer

/// Where a caret motion goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    /// The first non-blank of the line, or its start when already there.
    Home,
    /// The very start of the line.
    LineStart,
    LineEnd,
    Up(usize),
    Down(usize),
    DocStart,
    DocEnd,
}

/// What kind of typing an edit is; runs of the same kind share an undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typing {
    Insert,
    Delete,
}

/// One change to the text: at `.0`, `.1` bytes went and `.2` bytes came.
pub type Change = (usize, usize, usize);

#[derive(Clone)]
struct Snapshot {
    text: String,
    anchor: usize,
    head: usize,
    /// What the step after it did, for the menu's "Undo Rename".
    name: &'static str,
}

struct Run {
    kind: Typing,
    at: Instant,
    caret: usize,
}

/// The pane's text, its caret and selection, and the one undo history of
/// the open file: typing, canvas edits and outside changes all land here.
pub struct Buffer {
    text: String,
    /// Byte offset where each line starts.
    lines: Vec<usize>,
    /// Bumped on every change, to tell fresh analyses from stale ones.
    pub version: u64,
    /// The selection runs from `anchor` to `head`, the caret.
    pub anchor: usize,
    pub head: usize,
    /// The column up and down aim for.
    goal: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    run: Option<Run>,
    /// Changes not yet passed on by [`Buffer::take_changes`].
    changes: Vec<Change>,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer::new(String::new())
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect()
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The smallest change that turns `old` into `new`, on char boundaries.
pub fn diff(old: &str, new: &str) -> Change {
    let mut pre = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(pre) || !new.is_char_boundary(pre) {
        pre -= 1;
    }
    let room = old.len().min(new.len()) - pre;
    let mut suf = old.bytes().rev().zip(new.bytes().rev()).take(room).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(old.len() - suf) || !new.is_char_boundary(new.len() - suf) {
        suf -= 1;
    }
    (pre, old.len() - pre - suf, new.len() - pre - suf)
}

/// Where `offset` lands after `change`: before it stays, after it moves
/// along, inside what was replaced goes to the end of the replacement.
pub fn map_offset(offset: usize, (at, gone, came): Change) -> usize {
    if offset <= at {
        offset
    } else if offset >= at + gone {
        offset - gone + came
    } else {
        at + came
    }
}

impl Buffer {
    pub fn new(text: String) -> Buffer {
        let lines = line_starts(&text);
        Buffer {
            text,
            lines,
            version: 0,
            anchor: 0,
            head: 0,
            goal: None,
            undo: Vec::new(),
            redo: Vec::new(),
            run: None,
            changes: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.lines[line.min(self.lines.len() - 1)]
    }

    /// The end of `line`, before its newline.
    pub fn line_end(&self, line: usize) -> usize {
        self.lines.get(line + 1).map_or(self.text.len(), |&s| s - 1)
    }

    pub fn line(&self, line: usize) -> &str {
        &self.text[self.line_start(line)..self.line_end(line)]
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.lines.partition_point(|&s| s <= offset) - 1
    }

    /// The column `offset` is in, with tabs to the next multiple of four.
    pub fn col_of(&self, offset: usize) -> usize {
        let l = self.line_of(offset);
        columns(&self.text[self.line_start(l)..offset])
    }

    /// The offset nearest column `col` of `line`.
    pub fn offset_at(&self, line: usize, col: usize) -> usize {
        let start = self.line_start(line);
        let mut c = 0;
        for (i, ch) in self.line(line).char_indices() {
            let w = if ch == '\t' { TAB - c % TAB } else { 1 };
            // The boundary nearest `col`: before this character unless past its middle.
            if col * 2 <= c * 2 + w {
                return start + i;
            }
            c += w;
        }
        self.line_end(line)
    }

    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn selected(&self) -> &str {
        &self.text[self.selection()]
    }

    /// Moves the caret; `extend` keeps the anchor (shift).
    pub fn set_caret(&mut self, offset: usize, extend: bool) {
        let mut o = offset.min(self.text.len());
        while !self.text.is_char_boundary(o) {
            o -= 1;
        }
        self.head = o;
        if !extend {
            self.anchor = o;
        }
        self.run = None;
    }

    fn prev(&self, o: usize) -> usize {
        self.text[..o].chars().next_back().map_or(0, |c| o - c.len_utf8())
    }

    fn next(&self, o: usize) -> usize {
        self.text[o..].chars().next().map_or(o, |c| o + c.len_utf8())
    }

    fn target(&self, m: Motion) -> usize {
        let h = self.head;
        let line = self.line_of(h);
        match m {
            Motion::Left => self.prev(h),
            Motion::Right => self.next(h),
            Motion::WordLeft => {
                let before = &self.text[..h];
                let t = before.trim_end_matches(|c: char| !word_char(c));
                t.trim_end_matches(word_char).len()
            }
            Motion::WordRight => {
                let after = &self.text[h..];
                let t = after.trim_start_matches(|c: char| !word_char(c));
                self.text.len() - t.trim_start_matches(word_char).len()
            }
            Motion::Home => {
                let start = self.line_start(line);
                let first = start + self.line(line).len() - self.line(line).trim_start().len();
                if h == first { start } else { first }
            }
            Motion::LineStart => self.line_start(line),
            Motion::LineEnd => self.line_end(line),
            Motion::Up(n) | Motion::Down(n) => {
                let to = if let Motion::Up(_) = m {
                    if line == 0 {
                        return 0;
                    }
                    line.saturating_sub(n)
                } else {
                    if line + 1 == self.line_count() {
                        return self.text.len();
                    }
                    (line + n).min(self.line_count() - 1)
                };
                self.offset_at(to, self.goal.unwrap_or_else(|| self.col_of(h)))
            }
            Motion::DocStart => 0,
            Motion::DocEnd => self.text.len(),
        }
    }

    pub fn motion(&mut self, m: Motion, extend: bool) {
        let vertical = matches!(m, Motion::Up(_) | Motion::Down(_));
        let goal = if vertical { Some(self.goal.unwrap_or_else(|| self.col_of(self.head))) } else { None };
        let sel = self.selection();
        let to = match m {
            // Collapsing a selection goes to its edge.
            Motion::Left if !extend && !sel.is_empty() => sel.start,
            Motion::Right if !extend && !sel.is_empty() => sel.end,
            _ => self.target(m),
        };
        self.set_caret(to, extend);
        self.goal = goal;
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.head = self.text.len();
    }

    /// Selects the word (or run of other characters) at `offset`.
    pub fn select_word(&mut self, offset: usize) {
        self.set_caret(offset, false);
        let line = self.line_of(offset);
        let (ls, le) = (self.line_start(line), self.line_end(line));
        let kind = |c: char| {
            if word_char(c) {
                0
            } else if c.is_whitespace() {
                1
            } else {
                2
            }
        };
        let Some(k) =
            self.text[offset..le].chars().next().or_else(|| self.text[ls..offset].chars().next_back()).map(kind)
        else {
            return;
        };
        let start = offset
            - self.text[ls..offset].chars().rev().take_while(|&c| kind(c) == k).map(char::len_utf8).sum::<usize>();
        let end =
            offset + self.text[offset..le].chars().take_while(|&c| kind(c) == k).map(char::len_utf8).sum::<usize>();
        self.anchor = start;
        self.head = end;
    }

    /// Selects whole lines from `from` to `to`, newlines included.
    pub fn select_lines(&mut self, from: usize, to: usize) {
        let (a, b) = (from.min(to), from.max(to));
        self.anchor = self.line_start(a);
        self.head = if b + 1 < self.line_count() { self.line_start(b + 1) } else { self.text.len() };
        self.run = None;
    }

    fn checkpoint(&mut self, name: &'static str) {
        self.undo.push(Snapshot { text: self.text.clone(), anchor: self.anchor, head: self.head, name });
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn changed(&mut self, change: Change) {
        self.version += 1;
        self.lines = line_starts(&self.text);
        self.changes.push(change);
    }

    /// Replaces `r` with `s` and puts the caret after it. Typing of one kind
    /// at the caret within [`RUN`] joins the previous undo step.
    fn replace(&mut self, r: Range<usize>, s: &str, typing: Option<Typing>) {
        let joins = matches!((&self.run, typing), (Some(run), Some(t))
            if run.kind == t && run.caret == self.head && run.at.elapsed() < RUN);
        if !joins {
            self.checkpoint("Typing");
        }
        self.text.replace_range(r.clone(), s);
        let caret = r.start + s.len();
        self.anchor = caret;
        self.head = caret;
        self.goal = None;
        self.run = typing.map(|kind| Run { kind, at: Instant::now(), caret });
        self.changed((r.start, r.len(), s.len()));
    }

    /// Typed text, over the selection.
    pub fn type_text(&mut self, s: &str) {
        let typing = (!s.contains('\n')).then_some(Typing::Insert);
        self.replace(self.selection(), s, typing);
    }

    /// Pasted text: its own undo step.
    pub fn paste(&mut self, s: &str) {
        self.replace(self.selection(), &s.replace("\r\n", "\n"), None);
    }

    /// Enter: a new line indented like this one, one level deeper after `{`.
    /// Between `{` and `}` the brace moves to a line of its own.
    pub fn newline(&mut self) {
        let sel = self.selection();
        let line = self.line(self.line_of(sel.start));
        let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let before = self.text[..sel.start].trim_end_matches([' ', '\t']);
        let after = self.text[sel.end..].trim_start_matches([' ', '\t']);
        if !before.ends_with('{') {
            return self.replace(sel, &format!("\n{indent}"), None);
        }
        let inner = format!("\n{indent}{}", " ".repeat(TAB));
        if after.starts_with('}') {
            self.replace(sel, &format!("{inner}\n{indent}"), None);
            let caret = self.head - 1 - indent.len();
            self.set_caret(caret, false);
        } else {
            self.replace(sel, &inner, None);
        }
    }

    /// Backspace, Delete and their word and line forms: removes the
    /// selection, or from the caret to where `m` goes. Backspace in leading
    /// spaces goes back to the previous tab stop.
    pub fn delete(&mut self, m: Motion) {
        let mut r = self.selection();
        if r.is_empty() {
            let h = self.head;
            let ls = self.line_start(self.line_of(h));
            let lead = &self.text[ls..h];
            let to = if m == Motion::Left && !lead.is_empty() && lead.bytes().all(|b| b == b' ') {
                h - ((lead.len() - 1) % TAB + 1)
            } else {
                self.target(m)
            };
            r = to.min(h)..to.max(h);
        }
        if !r.is_empty() {
            self.replace(r, "", Some(Typing::Delete));
        }
    }

    /// Tab and Shift+Tab: indents or outdents the selected lines, or inserts
    /// spaces to the next tab stop.
    pub fn indent(&mut self, outdent: bool) {
        let sel = self.selection();
        let (a, b) =
            (self.line_of(sel.start), self.line_of(sel.end.max(sel.start + 1) - 1).max(self.line_of(sel.start)));
        if !outdent && (sel.is_empty() || a == b && !self.selected().contains('\n') && sel.len() < self.line(a).len()) {
            let col = self.col_of(self.head);
            return self.replace(sel, &" ".repeat(TAB - col % TAB), None);
        }
        let (start, end) = (self.line_start(a), self.line_end(b));
        let block: Vec<String> = self.text[start..end]
            .split('\n')
            .map(|l| {
                if outdent {
                    let n = l.len() - l.trim_start_matches(' ').len();
                    let cut = if l.starts_with('\t') { 1 } else { n.min(TAB) };
                    l[cut..].to_string()
                } else if l.is_empty() {
                    String::new()
                } else {
                    format!("{}{l}", " ".repeat(TAB))
                }
            })
            .collect();
        let new = block.join("\n");
        if new == self.text[start..end] {
            return;
        }
        let keep = !sel.is_empty();
        self.replace(start..end, &new, None);
        if keep {
            self.anchor = start;
            self.head = start + new.len();
        }
    }

    /// The text changed outside the pane (a canvas edit, another program):
    /// one undo step, and the caret stays by the text around it.
    pub fn replace_all(&mut self, new: &str) {
        self.replace_all_as(new, "Change");
    }

    /// [`Buffer::replace_all`], with the step named for the Edit menu.
    pub fn replace_all_as(&mut self, new: &str, name: &'static str) {
        if new == self.text {
            return;
        }
        self.checkpoint(name);
        self.swap(new.to_string(), None);
    }

    /// What Undo would take back, and Redo do again.
    pub fn undo_name(&self) -> Option<&'static str> {
        self.undo.last().map(|s| s.name)
    }

    pub fn redo_name(&self) -> Option<&'static str> {
        self.redo.last().map(|s| s.name)
    }

    fn swap(&mut self, new: String, caret: Option<(usize, usize)>) {
        let change = diff(&self.text, &new);
        let (anchor, head) = caret.unwrap_or((map_offset(self.anchor, change), map_offset(self.head, change)));
        self.text = new;
        self.changed(change);
        self.run = None;
        self.goal = None;
        self.anchor = anchor.min(self.text.len());
        self.set_caret(head, true);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.redo.push(Snapshot { text: self.text.clone(), anchor: self.anchor, head: self.head, name: s.name });
        self.swap(s.text, Some((s.anchor, s.head)));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        self.undo.push(Snapshot { text: self.text.clone(), anchor: self.anchor, head: self.head, name: s.name });
        self.swap(s.text, Some((s.anchor, s.head)));
        true
    }

    /// The changes since the last call, oldest first.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }
}

/// Columns `s` takes, tabs to the next multiple of four.
fn columns(s: &str) -> usize {
    s.chars().fold(0, |c, ch| if ch == '\t' { c + TAB - c % TAB } else { c + 1 })
}

// ------------------------------------------------------------------- spans

/// A byte range of the text.
pub type Span = Range<usize>;

/// A statement the caret can be in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Statement {
    Node(NodeId),
    /// An index into the document's edges.
    Edge(usize),
}

/// Where nodes, edges and problems sit in the text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spans {
    pub nodes: Vec<(NodeId, Span)>,
    /// Parallel to the document's edges.
    pub edges: Vec<Option<Span>>,
    /// Problem ranges; `true` for errors.
    pub problems: Vec<(Span, bool)>,
}

fn shift(r: &mut Span, change: Change) {
    *r = map_offset(r.start, change)..map_offset(r.end, change);
}

impl Spans {
    /// Follows an edit, so spans stay close until the next analysis.
    pub fn shift(&mut self, change: Change) {
        for r in self.nodes.iter_mut().map(|(_, r)| r).chain(self.edges.iter_mut().flatten()) {
            shift(r, change);
        }
        for (r, _) in &mut self.problems {
            shift(r, change);
        }
    }

    /// The innermost node or edge statement around `offset`.
    pub fn at(&self, offset: usize) -> Option<Statement> {
        let nodes = self.nodes.iter().map(|(id, r)| (Statement::Node(id.clone()), r));
        let edges = self.edges.iter().enumerate().filter_map(|(k, r)| Some((Statement::Edge(k), r.as_ref()?)));
        nodes
            .chain(edges)
            .filter(|(_, r)| r.start <= offset && offset <= r.end)
            .min_by_key(|(_, r)| r.len())
            .map(|(s, _)| s)
    }

    pub fn of(&self, s: &Statement) -> Option<Span> {
        match s {
            Statement::Node(id) => self.nodes.iter().find(|(n, _)| n == id).map(|(_, r)| r.clone()),
            Statement::Edge(k) => self.edges.get(*k).cloned().flatten(),
        }
    }
}

/// Reads `src` like the viewer does (without kits or imports): its
/// problems, and where each node and edge came from.
pub fn analyze(src: &str) -> (Vec<Diagnostic>, Spans) {
    let (file, mut diagnostics) = rhizo_lang::kdl::parse(src);
    let lowered = rhizo_lang::lower::lower_with_sources(&file);
    diagnostics.extend(lowered.diagnostics);
    let range = |s: rhizo_lang::ast::Span| s.map(|(o, n)| o..o + n);
    let mut nodes: Vec<(NodeId, Span)> =
        lowered.node_spans.into_iter().filter_map(|(id, s)| Some((id, range(s)?))).collect();
    nodes.sort_by_key(|(_, r)| r.start);
    let problems = diagnostics
        .iter()
        .filter_map(|d| {
            let r = range(d.span)?;
            Some((r.start..r.end.max(r.start + 1).min(src.len()), d.is_error()))
        })
        .collect();
    let edges = lowered.edge_spans.into_iter().map(range).collect();
    (diagnostics, Spans { nodes, edges, problems })
}

// ------------------------------------------------------------- saving rules

/// What a change of the file on disk means for the pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outside {
    /// Our own save coming back, or nothing new.
    Ours,
    /// Show it: there was no unsaved typing.
    Adopt,
    /// There is unsaved typing; keep it and say so.
    Conflict,
}

/// When typing is checked and saved, and what happens when the file
/// changes underneath it.
#[derive(Default)]
pub struct Sync {
    /// The last keystroke not yet checked.
    pub typed: Option<Instant>,
    /// The file changed on disk while there was unsaved typing; until the
    /// user picks a side, nothing is saved.
    pub conflict: bool,
}

impl Sync {
    pub fn typed(&mut self, now: Instant) {
        self.typed = Some(now);
    }

    /// When the typing so far is due to be checked.
    pub fn due(&self) -> Option<Instant> {
        self.typed.map(|t| t + DEBOUNCE)
    }

    /// True (once) when typing has paused long enough.
    pub fn take_due(&mut self, now: Instant) -> bool {
        let due = self.due().is_some_and(|d| d <= now);
        if due {
            self.typed = None;
        }
        due
    }

    /// The file now holds `disk`. `base` is the text the window last read
    /// or wrote, `text` what the pane shows.
    pub fn outside(&mut self, text: &str, base: Option<&str>, disk: &str) -> Outside {
        if base == Some(disk) {
            return Outside::Ours;
        }
        if disk == text || base.is_none_or(|b| b == text) {
            self.conflict = false;
            return Outside::Adopt;
        }
        self.conflict = true;
        Outside::Conflict
    }
}

// ------------------------------------------------------------------- pane

/// Sizes in window pixels, from the monospace font.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub size: f32,
    pub line: f64,
    pub advance: f64,
    /// Width of the line-number gutter.
    pub gutter: f64,
    /// Space above the first line.
    pub top: f64,
}

impl Metrics {
    pub fn new(mono: &Fonts, dpr: f64, lines: usize) -> Metrics {
        let size = (13.0 * dpr) as f32;
        let advance = mono.shape("0", size, 400).map_or(size as f64 * 0.6, |s| s.width as f64);
        let digits = lines.max(1).to_string().len().max(2) as f64;
        Metrics { size, line: (size as f64 * 1.62).round(), advance, gutter: (digits + 3.0) * advance, top: 10.0 * dpr }
    }

    /// Where column 0 of the text starts, from the pane's left edge.
    pub fn text_x(&self) -> f64 {
        self.gutter + self.advance
    }
}

/// The pane's state; the window keeps one for the session.
pub struct Pane {
    pub open: bool,
    /// Width in logical pixels.
    pub width: f64,
    /// Keys go to the text rather than the canvas.
    pub focused: bool,
    pub buffer: Buffer,
    tokens: Vec<Token>,
    tokens_version: u64,
    pub spans: Spans,
    /// The version `spans` was analyzed at; edits since only shifted them.
    pub analyzed: u64,
    pub scroll: Vec2,
    /// IME composition in progress, drawn at the caret.
    pub preedit: String,
    pub sync: Sync,
}

impl Default for Pane {
    fn default() -> Self {
        Pane {
            open: false,
            width: 460.0,
            focused: false,
            buffer: Buffer::default(),
            tokens: Vec::new(),
            tokens_version: u64::MAX,
            spans: Spans::default(),
            analyzed: 0,
            scroll: Vec2::ZERO,
            preedit: String::new(),
            sync: Sync::default(),
        }
    }
}

impl Pane {
    /// A new file: its text, no history.
    pub fn reset(&mut self, text: &str) {
        self.buffer = Buffer::new(text.to_string());
        self.spans = Spans::default();
        self.analyzed = 0;
        self.scroll = Vec2::ZERO;
        self.preedit.clear();
        self.sync = Sync::default();
        self.tokens_version = u64::MAX;
    }

    /// Brings tokens and spans up to date with the buffer's edits.
    pub fn refresh(&mut self) {
        for c in self.buffer.take_changes() {
            self.spans.shift(c);
        }
        if self.tokens_version != self.buffer.version {
            self.tokens = highlight(self.buffer.text());
            self.tokens_version = self.buffer.version;
        }
    }

    /// The text offset under window point `p`, in a pane drawn at `rect`.
    pub fn offset_at(&self, m: &Metrics, rect: Rect, p: Vec2) -> usize {
        let b = &self.buffer;
        let y = p.y - rect.y0 - m.top + self.scroll.y;
        let line = (y / m.line).floor().clamp(0.0, (b.line_count() - 1) as f64) as usize;
        let col = ((p.x - rect.x0 - m.text_x() + self.scroll.x) / m.advance).round().max(0.0) as usize;
        if y < 0.0 {
            return 0;
        }
        b.offset_at(line, col)
    }

    fn max_scroll(&self, m: &Metrics, rect: Rect) -> f64 {
        (self.buffer.line_count() as f64 * m.line + 2.0 * m.top - rect.height() + 3.0 * m.line).max(0.0)
    }

    /// Scrolls by `d` pixels, within the text.
    pub fn scroll_by(&mut self, m: &Metrics, rect: Rect, d: Vec2) {
        self.scroll.y = (self.scroll.y - d.y).clamp(0.0, self.max_scroll(m, rect));
        self.scroll.x = (self.scroll.x - d.x).max(0.0);
    }

    /// Scrolls just enough to show the caret.
    pub fn reveal_caret(&mut self, m: &Metrics, rect: Rect) {
        let b = &self.buffer;
        let y = b.line_of(b.head) as f64 * m.line;
        let room = rect.height() - 2.0 * m.top - m.line;
        self.scroll.y = self.scroll.y.clamp((y - room).max(0.0), y);
        let x = b.col_of(b.head) as f64 * m.advance;
        let wide = (rect.width() - m.text_x() - 2.0 * m.advance).max(m.advance);
        self.scroll.x = self.scroll.x.clamp((x - wide).max(0.0), x);
    }

    /// Scrolls `line` to a third of the way down, unless it shows already.
    pub fn reveal_line(&mut self, m: &Metrics, rect: Rect, line: usize) {
        let y = line as f64 * m.line;
        let room = rect.height() - 2.0 * m.top - 2.0 * m.line;
        if y < self.scroll.y || y > self.scroll.y + room {
            self.scroll.y = (y - room / 3.0).clamp(0.0, self.max_scroll(m, rect));
        }
        self.scroll.x = 0.0;
    }

    /// Where the caret is drawn, in window pixels (for the IME's popup).
    pub fn caret_rect(&self, m: &Metrics, rect: Rect) -> Rect {
        let b = &self.buffer;
        let x = rect.x0 + m.text_x() + b.col_of(b.head) as f64 * m.advance - self.scroll.x;
        let y = rect.y0 + m.top + b.line_of(b.head) as f64 * m.line - self.scroll.y;
        Rect::new(x, y, x + 2.0, y + m.line)
    }
}

// ------------------------------------------------------------------ drawing

/// Token colors for a light or dark background.
fn color(kind: Kind, dark: bool, ink: Color, bg: Color) -> Color {
    let pick = |light: u32, night: u32| Color::hex(if dark { night } else { light });
    match kind {
        Kind::Keyword => pick(0x4f46e5, 0xa5b4fc),
        Kind::String => pick(0x047857, 0x86efac),
        Kind::Property => pick(0xa16207, 0xfcd34d),
        Kind::Ident => pick(0x0e7490, 0x67e8f9),
        Kind::Number => pick(0xc2410c, 0xfdba74),
        Kind::Literal => pick(0xbe185d, 0xf9a8d4),
        Kind::Comment => ink.mix(bg, 0.55),
        Kind::Punct => ink.mix(bg, 0.42),
    }
}

/// What else the pane shows besides the buffer.
pub struct Extras<'a> {
    /// The statement of the node or edge selected on the canvas.
    pub statement: Option<Span>,
    /// A note along the bottom: unsaved errors, or a conflict.
    pub note: Option<&'a str>,
}

/// Draws the pane into `rect` (window pixels). Only lines on screen are
/// shaped, so the cost does not grow with the file.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    pane: &Pane,
    mono: &Fonts,
    ui: &Fonts,
    cache: &mut FontCache,
    rect: Rect,
    dpr: f64,
    theme_bg: Color,
    extras: &Extras,
) {
    let ink = chrome::ink(theme_bg);
    let dark = theme_bg.luminance() < 0.4;
    let bg = theme_bg.mix(ink, if dark { 0.045 } else { 0.028 });
    let m = Metrics::new(mono, dpr, pane.buffer.line_count());
    let b = &pane.buffer;
    let fill = |s: &mut Scene, r: Rect, c: Color| s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(c), None, &r);
    fill(s, rect, bg);
    s.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    let (x0, text_x) = (rect.x0, rect.x0 + m.text_x() - pane.scroll.x);
    let line_top = |l: usize| rect.y0 + m.top + l as f64 * m.line - pane.scroll.y;
    let first = ((pane.scroll.y - m.top) / m.line).floor().max(0.0) as usize;
    let last = (((pane.scroll.y + rect.height()) / m.line).ceil() as usize).min(b.line_count() - 1);
    let caret_line = b.line_of(b.head);
    let sel = b.selection();
    let col_x = |o: usize| text_x + b.col_of(o) as f64 * m.advance;

    // The caret's line, faintly.
    if pane.focused && sel.is_empty() && (first..=last).contains(&caret_line) {
        let y = line_top(caret_line);
        fill(s, Rect::new(x0, y, rect.x1, y + m.line), chrome::with_alpha(ink, if dark { 0.05 } else { 0.035 }));
    }
    // The canvas selection's statement: its first line tinted, a bar along all of it.
    let statement = extras.statement.clone().filter(|r| r.end <= b.text().len());
    if let Some(r) = &statement {
        let (l0, l1) = (b.line_of(r.start), b.line_of(r.end.saturating_sub(1).max(r.start)));
        if l1 >= first && l0 <= last {
            let y = line_top(l0);
            fill(s, Rect::new(x0 + m.gutter, y, rect.x1, y + m.line), chrome::with_alpha(ACCENT, 0.10));
            let bar =
                Rect::new(x0 + m.gutter - 3.0 * dpr, y.max(rect.y0), x0 + m.gutter - 1.0 * dpr, line_top(l1) + m.line);
            fill(s, bar, chrome::with_alpha(ACCENT, 0.75));
        }
    }
    // The selection.
    if !sel.is_empty() {
        let tone = if pane.focused { chrome::with_alpha(ACCENT, 0.28) } else { chrome::with_alpha(ink, 0.12) };
        let (l0, l1) = (b.line_of(sel.start).max(first), b.line_of(sel.end).min(last));
        for l in l0..=l1 {
            let (ls, le) = (b.line_start(l), b.line_end(l));
            let (a, z) = (sel.start.max(ls), sel.end.min(le));
            let newline = if sel.end > le { m.advance * 0.6 } else { 0.0 };
            if a > z || (a == z && newline == 0.0) {
                continue;
            }
            let y = line_top(l);
            fill(s, Rect::new(col_x(a).max(x0 + m.gutter), y, col_x(z) + newline, y + m.line), tone);
        }
    }

    let baseline = |l: usize| (line_top(l) + m.line / 2.0) as f32 + m.size * 0.36;
    let errors: Vec<&(Span, bool)> =
        pane.spans.problems.iter().filter(|(r, _)| r.end <= b.text().len() && r.start <= b.text().len()).collect();
    let mut glyphs = Glyphs {
        s: &mut *s,
        mono,
        ui,
        cache: &mut *cache,
        size: m.size,
        advance: m.advance,
        left: x0 + m.gutter,
        right: rect.x1,
    };
    for l in first..=last {
        let (ls, le) = (b.line_start(l), b.line_end(l));
        let y = baseline(l);
        // Line number, right-aligned in the gutter.
        let num = (l + 1).to_string();
        let tone = if errors.iter().any(|(r, e)| *e && b.line_of(r.start) == l) {
            ERROR
        } else if l == caret_line && pane.focused {
            ink.mix(bg, 0.25)
        } else {
            ink.mix(bg, 0.62)
        };
        let nx = x0 + m.gutter - (num.len() as f64 + 1.5) * m.advance;
        glyphs.run(&num, nx, y, tone, false);
        // The text, token by token.
        let tokens = &pane.tokens;
        let mut k = tokens.partition_point(|t| t.end <= ls);
        while k < tokens.len() && tokens[k].start < le {
            let t = tokens[k];
            let (a, z) = (t.start.max(ls), t.end.min(le));
            if a < z {
                let x = text_x + b.col_of(a) as f64 * m.advance;
                glyphs.run(&b.text()[a..z], x, y, color(t.kind, dark, ink, bg), true);
            }
            k += 1;
        }
    }
    // Problems: a wavy underline.
    for (r, error) in &errors {
        let (l0, l1) = (b.line_of(r.start), b.line_of(r.end.saturating_sub(1).max(r.start)));
        for l in l0.max(first)..=l1.min(last) {
            let (a, z) = (r.start.max(b.line_start(l)), r.end.min(b.line_end(l)));
            let (xa, xz) = (col_x(a), col_x(z).max(col_x(a) + m.advance));
            let y = baseline(l) as f64 + 3.0 * dpr;
            glyphs.s.stroke(
                &Stroke::new(1.1 * dpr),
                Affine::IDENTITY,
                peniko_color(if *error { ERROR } else { WARNING }),
                None,
                &squiggle(xa, xz, y, dpr),
            );
        }
    }
    // Composition in progress, then the caret.
    if pane.focused && (first..=last).contains(&caret_line) {
        let y = line_top(caret_line);
        let mut x = col_x(b.head);
        if !pane.preedit.is_empty() {
            let w = columns(&pane.preedit) as f64 * m.advance;
            fill(s, Rect::new(x, y + 2.0 * dpr, x + w, y + m.line - 2.0 * dpr), bg);
            let mut g = Glyphs { s, mono, ui, cache, size: m.size, advance: m.advance, left: x0, right: rect.x1 };
            g.run(&pane.preedit, x, baseline(caret_line), ink, true);
            let under = Line::new((x, y + m.line - 3.0 * dpr), (x + w, y + m.line - 3.0 * dpr));
            s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink), None, &under);
            x += w;
        }
        fill(s, Rect::new(x - 0.75 * dpr, y + 2.0 * dpr, x + 0.75 * dpr, y + m.line - 2.0 * dpr), ACCENT);
    }
    // The note along the bottom.
    if let Some(note) = extras.note {
        let h = 30.0 * dpr;
        let strip = Rect::new(x0, rect.y1 - h, rect.x1, rect.y1);
        fill(s, strip, bg.mix(WARNING, if dark { 0.16 } else { 0.12 }));
        let hair = Line::new((x0, strip.y0 + 0.5 * dpr), (rect.x1, strip.y0 + 0.5 * dpr));
        s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(WARNING.mix(bg, 0.5)), None, &hair);
        let mut t = chrome::Text { s, fonts: ui, cache, size: 12.0 * dpr as f32, baseline: 0.0 };
        t.baseline = (strip.y0 + h / 2.0) as f32 + t.size * 0.35;
        let text = t.clip(note, 500, strip.width() - 24.0 * dpr);
        t.draw(&text, x0 + 12.0 * dpr, 500, ink.mix(WARNING, 0.25));
    }
    s.pop_layer();
    // The divider.
    let line = Line::new((rect.x1 - 0.5 * dpr, rect.y0), (rect.x1 - 0.5 * dpr, rect.y1));
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink.mix(theme_bg, 0.86)), None, &line);
}

/// A wave from `x0` to `x1` around `y`.
fn squiggle(x0: f64, x1: f64, y: f64, dpr: f64) -> BezPath {
    let (step, amp) = (2.5 * dpr, 1.4 * dpr);
    let mut p = BezPath::new();
    p.move_to((x0, y));
    let mut x = x0;
    let mut up = true;
    while x < x1 {
        x = (x + step).min(x1);
        p.line_to((x, if up { y - amp } else { y + amp }));
        up = !up;
    }
    p
}

/// Draws text on the monospace grid; characters the font lacks come from
/// the UI font.
struct Glyphs<'a> {
    s: &'a mut Scene,
    mono: &'a Fonts,
    ui: &'a Fonts,
    cache: &'a mut FontCache,
    size: f32,
    advance: f64,
    /// Glyphs outside this band are skipped.
    left: f64,
    right: f64,
}

impl Glyphs<'_> {
    /// `text` from `x` on baseline `y`; `clip` skips what falls left of the band.
    fn run(&mut self, text: &str, x: f64, y: f32, color: Color, clip: bool) {
        let Some(shaped) = self.mono.shape(text, self.size, 400) else { return };
        let mut col = 0;
        let mut out = Vec::new();
        let mut missing = Vec::new();
        for (ch, (id, _)) in text.chars().zip(&shaped.glyphs) {
            let gx = x + col as f64 * self.advance;
            col += if ch == '\t' { TAB - col % TAB } else { 1 };
            if gx > self.right {
                break;
            }
            if ch.is_whitespace() || (clip && gx + self.advance < self.left) {
                continue;
            }
            if *id == 0 {
                missing.push((ch, gx));
            } else {
                out.push(Glyph { id: *id as u32, x: (gx - x) as f32, y: 0.0 });
            }
        }
        let brush = peniko_color(color);
        if !out.is_empty() {
            let font = self.cache.get(&shaped.font.0, shaped.font.1);
            self.s
                .draw_glyphs(&font)
                .font_size(self.size)
                .transform(Affine::translate((x, y as f64)))
                .brush(brush)
                .draw(Fill::NonZero, out.into_iter());
        }
        for (ch, gx) in missing {
            let Some(g) = self.ui.shape(ch.encode_utf8(&mut [0; 4]), self.size, 400) else { continue };
            let font = self.cache.get(&g.font.0, g.font.1);
            let glyphs = g.glyphs.iter().map(|&(id, gx)| Glyph { id: id as u32, x: gx, y: 0.0 });
            self.s
                .draw_glyphs(&font)
                .font_size(self.size)
                .transform(Affine::translate((gx, y as f64)))
                .brush(brush)
                .draw(Fill::NonZero, glyphs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(&str, Kind)> {
        highlight(src).into_iter().map(|t| (&src[t.start..t.end], t.kind)).collect()
    }

    #[test]
    fn highlights_kdl() {
        use Kind::*;
        let src = "rhizo version=1\n- \"Root\" shape=diamond { - \"A\" id=a; - #\"raw \"q\"\"# x=1.5 }\n// note\nedge a b /* c /* d */ */ ok=#true\n/-node \"x\"";
        assert_eq!(
            kinds(src),
            vec![
                ("rhizo", Keyword),
                ("version", Property),
                ("=", Punct),
                ("1", Number),
                ("-", Keyword),
                ("\"Root\"", String),
                ("shape", Property),
                ("=", Punct),
                ("diamond", Ident),
                ("{", Punct),
                ("-", Keyword),
                ("\"A\"", String),
                ("id", Property),
                ("=", Punct),
                ("a", Ident),
                (";", Punct),
                ("-", Keyword),
                ("#\"raw \"q\"\"#", String),
                ("x", Property),
                ("=", Punct),
                ("1.5", Number),
                ("}", Punct),
                ("// note", Comment),
                ("edge", Keyword),
                ("a", Ident),
                ("b", Ident),
                ("/* c /* d */ */", Comment),
                ("ok", Property),
                ("=", Punct),
                ("#true", Literal),
                ("/-", Comment),
                ("node", Keyword),
                ("\"x\"", String),
            ]
        );
    }

    #[test]
    fn highlighting_forgives_half_typed_text() {
        use Kind::*;
        // An unclosed string ends at the line end; the next line still starts a statement.
        assert_eq!(kinds("- \"Ro\n- \"B\""), vec![("-", Keyword), ("\"Ro", String), ("-", Keyword), ("\"B\"", String)]);
        assert_eq!(kinds("\"\"\"\nmulti\n\"\"\" x"), vec![("\"\"\"\nmulti\n\"\"\"", String), ("x", Ident)]);
        assert_eq!(
            kinds("(t)node -2 r#\"v1\"#"),
            vec![("(", Punct), ("t", Ident), (")", Punct), ("node", Keyword), ("-2", Number), ("r#\"v1\"#", String)]
        );
        assert_eq!(kinds("a \\\n  b"), vec![("a", Keyword), ("\\", Punct), ("b", Ident)]);
        assert_eq!(kinds("é \"ü\" #"), vec![("é", Keyword), ("\"ü\"", String), ("#", Literal)]);
        assert!(highlight("").is_empty());
        assert_eq!(kinds("/* open"), vec![("/* open", Comment)]);
    }

    fn buf(text: &str, caret: usize) -> Buffer {
        let mut b = Buffer::new(text.into());
        b.set_caret(caret, false);
        b
    }

    #[test]
    fn caret_moves_by_char_word_line_and_column() {
        let mut b = buf("- \"Root\" {\n\tid=é_x\n}", 0);
        b.motion(Motion::WordRight, false);
        assert_eq!(b.head, 7, "past `Root`");
        b.motion(Motion::WordRight, false);
        assert_eq!(&b.text()[..b.head], "- \"Root\" {\n\tid");
        b.motion(Motion::WordLeft, false);
        assert_eq!(b.col_of(b.head), 4, "the tab is four columns");
        b.motion(Motion::Up(1), false);
        assert_eq!(b.head, 4, "same column on the line above");
        b.motion(Motion::Down(1), false);
        assert_eq!(b.col_of(b.head), 4, "and back");
        b.motion(Motion::LineEnd, false);
        b.motion(Motion::Left, false);
        assert_eq!(&b.text()[b.head..b.line_end(1)], "x");
        b.motion(Motion::Left, true);
        b.motion(Motion::Left, true);
        assert_eq!(b.selected(), "é_", "shift+left steps over whole characters");
        b.motion(Motion::Right, false);
        assert!(b.selection().is_empty() && &b.text()[b.head..b.head + 1] == "x", "collapses to the end");
        b.motion(Motion::Home, false);
        assert_eq!(b.col_of(b.head), 4, "home: first non-blank");
        b.motion(Motion::Home, false);
        assert_eq!(b.col_of(b.head), 0, "home again: line start");
        b.motion(Motion::Down(5), false);
        assert_eq!(b.head, b.text().len() - 1);
        b.motion(Motion::Down(1), false);
        assert_eq!(b.head, b.text().len(), "down on the last line: its end");
        b.motion(Motion::DocStart, true);
        assert_eq!(b.selection(), 0..b.text().len());
        // Columns past a short line clamp to its end, and remember the goal.
        let mut b = buf("abcdef\nab\nabcdef", 5);
        b.motion(Motion::Down(1), false);
        assert_eq!(b.head, 9);
        b.motion(Motion::Down(1), false);
        assert_eq!(b.col_of(b.head), 5);
        assert_eq!(b.offset_at(0, 99), 6);
        assert_eq!(b.offset_at(1, 1), 8);
    }

    #[test]
    fn editing_and_indentation() {
        let mut b = buf("- \"A\" {}", 7);
        b.newline();
        assert_eq!(b.text(), "- \"A\" {\n    \n}");
        assert_eq!(b.head, 12, "caret on the indented line");
        b.type_text("- \"B\"");
        b.newline();
        assert_eq!(b.text(), "- \"A\" {\n    - \"B\"\n    \n}");
        b.delete(Motion::Left);
        assert_eq!(b.text(), "- \"A\" {\n    - \"B\"\n\n}", "backspace in indentation: a tab stop");
        b.delete(Motion::Left);
        b.delete(Motion::WordLeft);
        assert_eq!(b.text(), "- \"A\" {\n    - \"\n}");
        b.motion(Motion::DocStart, false);
        b.delete(Motion::Right);
        assert_eq!(&b.text()[..2], " \"");
        b.select_all();
        b.type_text("x");
        assert_eq!(b.text(), "x");
        // Tab: spaces to the next stop; with lines selected, indents them all.
        let mut b = buf("a\nb\n", 1);
        b.indent(false);
        assert_eq!(b.text(), "a   \nb\n");
        b.select_lines(0, 1);
        b.indent(false);
        assert_eq!(b.text(), "    a   \n    b\n");
        assert_eq!(b.selected(), "    a   \n    b", "the lines stay selected");
        b.indent(true);
        b.indent(true);
        assert_eq!(b.text(), "a   \nb\n");
        b.select_word(0);
        assert_eq!(b.selected(), "a");
        b.select_word(2);
        assert_eq!(b.selected(), "   ");
    }

    #[test]
    fn one_history_for_typing_and_outside_edits() {
        let mut b = buf("- \"A\"\n", 5);
        for c in ["!", "!", "!"] {
            b.type_text(c);
        }
        b.delete(Motion::Left);
        assert_eq!(b.text(), "- \"A\"!!\n");
        // Canvas edits land as steps of their own; the caret keeps its place.
        b.replace_all("- \"A\"!!\n- \"B\"\n");
        assert_eq!(b.head, 7);
        b.replace_all("rhizo version=1\n- \"A\"!!\n- \"B\"\n");
        assert_eq!(&b.text()[..b.head], "rhizo version=1\n- \"A\"!!");
        assert!(b.undo() && b.undo());
        assert_eq!(b.text(), "- \"A\"!!\n");
        assert!(b.undo(), "the deletion");
        assert_eq!(b.text(), "- \"A\"!!!\n");
        assert!(b.undo(), "the typing run");
        assert_eq!((b.text(), b.head), ("- \"A\"\n", 5));
        assert!(!b.undo());
        assert!(b.redo() && b.redo() && b.redo() && b.redo());
        assert_eq!(b.text(), "rhizo version=1\n- \"A\"!!\n- \"B\"\n");
        assert!(!b.redo());
        // A caret move ends the run.
        let mut b = buf("ab", 1);
        b.type_text("x");
        b.motion(Motion::Right, false);
        b.type_text("y");
        b.undo();
        assert_eq!(b.text(), "axb");
        // Changes are reported for spans to follow.
        let mut b = buf("abc", 0);
        b.take_changes();
        b.type_text("zz");
        b.replace_all("zzabcd");
        assert_eq!(b.take_changes(), vec![(0, 0, 2), (5, 0, 1)]);
    }

    #[test]
    fn undo_steps_carry_their_names_for_the_menu() {
        let mut b = buf("a", 1);
        assert_eq!(b.undo_name(), None);
        b.type_text("b");
        assert_eq!(b.undo_name(), Some("Typing"));
        b.replace_all_as("ab\nc", "Add child");
        assert_eq!(b.undo_name(), Some("Add child"));
        b.undo();
        assert_eq!((b.undo_name(), b.redo_name()), (Some("Typing"), Some("Add child")));
        b.redo();
        assert_eq!((b.undo_name(), b.redo_name()), (Some("Add child"), None));
    }

    #[test]
    fn diffs_and_offsets() {
        assert_eq!(diff("abc", "abXc"), (2, 0, 1));
        assert_eq!(diff("aaa", "aa"), (2, 1, 0));
        assert_eq!(diff("héllo", "hállo"), (1, 2, 2), "char boundaries");
        assert_eq!(diff("same", "same"), (4, 0, 0));
        let c = (4, 2, 5);
        assert_eq!([map_offset(3, c), map_offset(4, c), map_offset(5, c), map_offset(6, c)], [3, 4, 9, 9]);
        let mut s = Spans { nodes: vec![("a".into(), 2..10)], edges: vec![None, Some(12..14)], problems: vec![] };
        s.shift((5, 0, 3));
        assert_eq!((s.nodes[0].1.clone(), s.edges[1].clone()), (2..13, Some(15..17)));
    }

    #[test]
    fn statements_under_the_caret() {
        let src = "rhizo version=1\n- \"Root\" {\n    - \"Child\" id=kid\n}\nedge kid root\n";
        let (diags, spans) = analyze(src);
        assert!(diags.is_empty(), "{diags:?}");
        let at = |needle: &str| spans.at(src.find(needle).unwrap());
        assert_eq!(at("Child"), Some(Statement::Node("kid".into())), "innermost");
        assert_eq!(at("\"Root"), Some(Statement::Node("root".into())));
        assert_eq!(at("edge"), Some(Statement::Edge(0)));
        assert_eq!(at("version"), None);
        let r = spans.of(&Statement::Node("kid".into())).unwrap();
        assert!(src[r].starts_with("- \"Child\" id=kid"));
        // Problems carry their place and severity.
        let (_, spans) = analyze("rhizo version=1\n- \"A\" shap=x\n");
        assert_eq!(spans.problems.len(), 1);
        assert!(spans.problems[0].1);
        assert_eq!(&"rhizo version=1\n- \"A\" shap=x\n"[spans.problems[0].0.clone()], "shap=x");
    }

    #[test]
    fn saving_waits_for_a_pause_and_conflicts_hold() {
        let t0 = Instant::now();
        let mut s = Sync::default();
        assert!(!s.take_due(t0));
        s.typed(t0);
        s.typed(t0 + Duration::from_millis(100));
        assert_eq!(s.due(), Some(t0 + Duration::from_millis(100) + DEBOUNCE));
        assert!(!s.take_due(t0 + Duration::from_millis(250)), "still typing");
        assert!(s.take_due(t0 + Duration::from_millis(300)));
        assert!(!s.take_due(t0 + Duration::from_millis(400)), "once");

        // Our own save coming back changes nothing.
        assert_eq!(s.outside("typed more", Some("saved"), "saved"), Outside::Ours);
        // No unsaved typing: show what the other program wrote.
        assert_eq!(s.outside("saved", Some("saved"), "theirs"), Outside::Adopt);
        assert_eq!(s.outside("", None, "first"), Outside::Adopt);
        // Unsaved typing: keep it, and hold saving until resolved.
        assert_eq!(s.outside("mine", Some("saved"), "theirs"), Outside::Conflict);
        assert!(s.conflict);
        // They wrote what we typed: no conflict.
        assert_eq!(s.outside("same", Some("saved"), "same"), Outside::Adopt);
        assert!(!s.conflict);
    }
}
