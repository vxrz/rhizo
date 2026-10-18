//! Canonical KDL output. `rhizo fmt` is `write(&raise(&lower(&parse(src))))`.

use std::fmt::Write;

use rhizo_core::{LayoutKind, Style};

use crate::ast::{EdgeDef, File, FrameDef, NodeDef};

const INDENT: &str = "    ";

pub fn write(file: &File) -> String {
    let mut w = Writer { out: String::new() };
    w.file(file);
    w.out
}

struct Writer {
    out: String,
}

/// A KDL v2 string literal, bare when it can be.
pub fn kdl_string(s: &str) -> String {
    if is_bare(s) { s.to_string() } else { quoted(s) }
}

fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn is_bare(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !matches!(s, "true" | "false" | "null" | "inf" | "nan")
}

fn num(v: f32) -> String {
    if v.fract() == 0.0 && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn style_props(style: &Style, out: &mut String) {
    let Style { fill, stroke, text, stroke_width, font_size, font_weight, line, shape } = style;
    if let Some(s) = shape {
        let _ = write!(out, " shape={}", s.name());
    }
    if let Some(c) = fill {
        let _ = write!(out, " fill=\"{c}\"");
    }
    if let Some(c) = stroke {
        let _ = write!(out, " stroke=\"{c}\"");
    }
    if let Some(c) = text {
        let _ = write!(out, " text=\"{c}\"");
    }
    if let Some(v) = stroke_width {
        let _ = write!(out, " stroke-width={}", num(*v));
    }
    if let Some(v) = font_size {
        let _ = write!(out, " font-size={}", num(*v));
    }
    if let Some(v) = font_weight {
        let _ = write!(out, " font-weight={v}");
    }
    if let Some(l) = line {
        let _ = write!(out, " line={}", l.name());
    }
}

fn list(items: &[String]) -> String {
    quoted(&items.join(" "))
}

impl Writer {
    fn line(&mut self, depth: usize, text: &str) {
        for _ in 0..depth {
            self.out.push_str(INDENT);
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn blank(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    fn file(&mut self, f: &File) {
        self.line(0, &format!("rhizo version={}", crate::kdl::FORMAT_VERSION));
        if let Some(t) = &f.title {
            self.line(0, &format!("title {}", quoted(t)));
        }
        if let Some(t) = &f.theme {
            self.line(0, &format!("theme {}", quoted(t)));
        }
        if let Some(l) = f.layout {
            self.line(0, &format!("layout {}", l.name()));
        }
        if let Some(d) = f.direction {
            self.line(0, &format!("direction {}", d.name()));
        }
        if !f.styles.is_empty() {
            self.blank();
            for (name, style) in &f.styles {
                let mut s = format!("style {}", quoted(name));
                style_props(style, &mut s);
                self.line(0, &s);
            }
        }
        let keyword = node_keyword(f.layout);
        if !f.nodes.is_empty() {
            self.blank();
            for n in &f.nodes {
                self.node(n, 0, keyword);
            }
        }
        for fr in &f.frames {
            self.blank();
            self.frame(fr);
        }
        if !f.edges.is_empty() {
            self.blank();
            for e in &f.edges {
                self.edge(e, 0);
            }
        }
    }

    fn frame(&mut self, f: &FrameDef) {
        let mut s = String::from("frame");
        if let Some(l) = &f.label {
            let _ = write!(s, " {}", quoted(l));
        }
        if let Some(id) = &f.id {
            let _ = write!(s, " id={}", kdl_string(id));
        }
        if let Some(l) = f.layout {
            let _ = write!(s, " layout={}", l.name());
        }
        if let Some(d) = f.direction {
            let _ = write!(s, " direction={}", d.name());
        }
        if let Some(x) = f.x {
            let _ = write!(s, " x={}", num(x));
        }
        if let Some(y) = f.y {
            let _ = write!(s, " y={}", num(y));
        }
        if let Some(c) = f.columns {
            let _ = write!(s, " columns={c}");
        }
        style_props(&f.style, &mut s);
        if f.nodes.is_empty() && f.edges.is_empty() {
            self.line(0, &s);
            return;
        }
        s.push_str(" {");
        self.line(0, &s);
        let keyword = node_keyword(f.layout);
        for n in &f.nodes {
            self.node(n, 1, keyword);
        }
        if !f.edges.is_empty() && !f.nodes.is_empty() {
            self.out.push('\n');
        }
        for e in &f.edges {
            self.edge(e, 1);
        }
        self.line(0, "}");
    }

    fn node(&mut self, n: &NodeDef, depth: usize, keyword: &str) {
        let mut s = format!("{keyword} {}", quoted(&n.label));
        if let Some(id) = &n.id {
            let _ = write!(s, " id={}", kdl_string(id));
        }
        if !n.class.is_empty() {
            let _ = write!(s, " class={}", list(&n.class));
        }
        style_props(&n.style, &mut s);
        if let Some(x) = n.x {
            let _ = write!(s, " x={}", num(x));
        }
        if let Some(y) = n.y {
            let _ = write!(s, " y={}", num(y));
        }
        if let Some(l) = &n.link {
            let _ = write!(s, " link={}", quoted(l));
        }
        if !n.tags.is_empty() {
            let _ = write!(s, " tags={}", list(&n.tags));
        }
        let multiline_note = n.note.as_deref().is_some_and(|t| t.contains('\n'));
        if let Some(note) = n.note.as_deref().filter(|_| !multiline_note) {
            let _ = write!(s, " note={}", quoted(note));
        }
        if n.children.is_empty() && !multiline_note {
            self.line(depth, &s);
            return;
        }
        s.push_str(" {");
        self.line(depth, &s);
        if multiline_note {
            for l in n.note.as_deref().unwrap_or_default().lines() {
                self.line(depth + 1, &format!("note {}", quoted(l)));
            }
        }
        for c in &n.children {
            self.node(c, depth + 1, keyword);
        }
        self.line(depth, "}");
    }

    fn edge(&mut self, e: &EdgeDef, depth: usize) {
        let mut s = format!("edge {} {}", kdl_string(&e.from), kdl_string(&e.to));
        if let Some(l) = &e.label {
            let _ = write!(s, " label={}", quoted(l));
        }
        if let Some(a) = e.arrow {
            let _ = write!(s, " arrow={}", a.name());
        }
        if !e.class.is_empty() {
            let _ = write!(s, " class={}", list(&e.class));
        }
        style_props(&e.style, &mut s);
        self.line(depth, &s);
    }
}

/// `-` reads like an outline, which is what trees and mindmaps are.
fn node_keyword(layout: Option<LayoutKind>) -> &'static str {
    match layout {
        Some(LayoutKind::Layered | LayoutKind::Grid | LayoutKind::Free) => "node",
        _ => "-",
    }
}
