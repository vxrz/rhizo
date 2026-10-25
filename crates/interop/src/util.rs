use std::collections::{HashMap, HashSet};

use rhizo_core::{Document, Frame, LayoutKind};
use rhizo_lang::ast::Span;

pub fn effective(doc: &Document, frame: &Frame) -> LayoutKind {
    rhizo_layout::effective_layout(doc, frame)
}

/// Entering or leaving a frame, in nesting order.
pub enum Visit<'a> {
    Open(&'a Frame, usize),
    Close(&'a Frame, usize),
}

/// Walks the frames as they nest: each frame opens, then the frames inside
/// it open and close, then it closes. Depth 0 is the top level. Flat
/// documents give open, close, open, close… in document order.
pub fn frame_tree(doc: &Document) -> Vec<Visit<'_>> {
    let parent = |f: &Frame| rhizo_layout::layout_parent(doc, f).cloned();
    let mut out = Vec::new();
    let mut stack: Vec<(&Frame, usize, bool)> =
        doc.frames.iter().rev().filter(|f| parent(f).is_none()).map(|f| (f, 0, false)).collect();
    while let Some((f, depth, closing)) = stack.pop() {
        if closing {
            out.push(Visit::Close(f, depth));
            continue;
        }
        out.push(Visit::Open(f, depth));
        stack.push((f, depth, true));
        let kids = doc.frames.iter().rev().filter(|c| parent(c).as_ref() == Some(&f.id));
        stack.extend(kids.map(|c| (c, depth + 1, false)));
    }
    out
}

/// Byte span of line `idx` of `src[base..]`, in `src` coordinates.
pub fn line_span(src: &str, base: usize, idx: usize) -> Span {
    let mut off = base;
    for (i, line) in src[base..].split_inclusive('\n').enumerate() {
        if i == idx {
            return Some((off, line.trim_end_matches(['\n', '\r']).len()));
        }
        off += line.len();
    }
    None
}

/// Maps rhizo ids to identifiers another language accepts: ASCII letters,
/// digits and `_`, starting with a letter, avoiding keywords, unique.
pub struct Ids {
    map: HashMap<String, String>,
    used: HashSet<String>,
    reserved: &'static [&'static str],
}

impl Ids {
    pub fn new(reserved: &'static [&'static str]) -> Self {
        Ids { map: HashMap::new(), used: HashSet::new(), reserved }
    }

    pub fn mermaid() -> Self {
        Ids::new(&[
            "end",
            "graph",
            "flowchart",
            "subgraph",
            "class",
            "classDef",
            "style",
            "linkStyle",
            "click",
            "direction",
            "default",
            "call",
            "href",
            "o",
            "x",
        ])
    }

    pub fn get(&mut self, id: &str) -> String {
        if let Some(v) = self.map.get(id) {
            return v.clone();
        }
        // Keys like "frame:x" and "class:x" keep separate namespaces but
        // print as plain "x".
        let shown = id.strip_prefix("frame:").or_else(|| id.strip_prefix("class:")).unwrap_or(id);
        let mut base: String =
            shown.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
        if base.is_empty() || !base.starts_with(|c: char| c.is_ascii_alphabetic()) {
            base = format!("n_{base}");
        }
        if self.reserved.iter().any(|r| r.eq_ignore_ascii_case(&base)) {
            base = format!("n_{base}");
        }
        let mut out = base.clone();
        let mut i = 2;
        while self.used.contains(&out) {
            out = format!("{base}_{i}");
            i += 1;
        }
        self.used.insert(out.clone());
        self.map.insert(id.to_string(), out.clone());
        out
    }
}

pub fn xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// A quoted string for C-like languages (DOT, D2).
pub fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
