//! Canonical KDL output. `rhizo fmt` is `write(&raise(&lower(&parse(src))))`.

use std::fmt::Write;

use rhizo_core::{Confidence, LayoutKind, Props, Prov, Style};
use serde_json::Value;

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
        self.line(0, &format!("rhizo version={}", f.rhizo.unwrap_or(1)));
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
        for k in &f.kits {
            let version = k.version.as_deref().map(|v| format!(" version={}", quoted(v))).unwrap_or_default();
            self.line(0, &format!("use {}{version}", kdl_string(&k.kit)));
        }
        if let Some(b) = &f.base {
            self.line(0, &format!("base {}", quoted(b)));
        }
        for (name, iri) in &f.prefixes {
            self.line(0, &format!("prefix {} {}", kdl_string(name), quoted(iri)));
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
        if let Some(t) = &f.ty {
            let _ = write!(s, " type={}", kdl_string(t));
        }
        style_props(&f.style, &mut s);
        if f.nodes.is_empty() && f.edges.is_empty() && f.props.is_empty() {
            self.line(0, &s);
            return;
        }
        s.push_str(" {");
        self.line(0, &s);
        self.props(&f.props, 1);
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
        if let Some(t) = &n.ty {
            let _ = write!(s, " type={}", kdl_string(t));
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
        let typed_block = !n.props.is_empty() || !n.ports.is_empty() || !n.accept.is_empty() || n.prov.is_some();
        if n.children.is_empty() && !multiline_note && !typed_block {
            self.line(depth, &s);
            return;
        }
        s.push_str(" {");
        self.line(depth, &s);
        self.props(&n.props, depth + 1);
        for p in &n.ports {
            let mut line = format!("port {} {}", p.dir.name(), quoted(p.name.as_str()));
            if let Some(k) = p.kind {
                let _ = write!(line, " kind={}", k.name());
            }
            if let Some(sc) = &p.schema {
                let _ = write!(line, " schema={}", kdl_string(sc));
            }
            if let Some(l) = &p.label {
                let _ = write!(line, " label={}", quoted(l));
            }
            self.line(depth + 1, &line);
        }
        for a in &n.accept {
            self.line(depth + 1, &format!("accept {}", quoted(a)));
        }
        if let Some(p) = &n.prov {
            self.prov(p, depth + 1);
        }
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
        if let Some(id) = &e.id {
            let _ = write!(s, " id={}", kdl_string(id));
        }
        if let Some(t) = &e.ty {
            let _ = write!(s, " type={}", kdl_string(t));
        }
        if let Some(k) = e.kind {
            let _ = write!(s, " kind={}", k.name());
        }
        if let Some(p) = &e.from_port {
            let _ = write!(s, " from-port={}", kdl_string(p));
        }
        if let Some(p) = &e.to_port {
            let _ = write!(s, " to-port={}", kdl_string(p));
        }
        style_props(&e.style, &mut s);
        if e.props.is_empty() && e.prov.is_none() {
            self.line(depth, &s);
            return;
        }
        s.push_str(" {");
        self.line(depth, &s);
        self.props(&e.props, depth + 1);
        if let Some(p) = &e.prov {
            self.prov(p, depth + 1);
        }
        self.line(depth, "}");
    }

    /// Canonical JiK: scalars as properties, arrays and objects as children.
    fn props(&mut self, props: &Props, depth: usize) {
        if !props.is_empty() {
            self.object("props", props.iter(), depth);
        }
    }

    fn object<'a>(&mut self, name: &str, entries: impl Iterator<Item = (&'a String, &'a Value)>, depth: usize) {
        let mut line = name.to_string();
        let mut complex = Vec::new();
        for (k, v) in entries {
            match jik_scalar(v) {
                Some(sv) => {
                    let _ = write!(line, " {}={sv}", kdl_string(k));
                }
                None => complex.push((k, v)),
            }
        }
        if complex.is_empty() {
            self.line(depth, &line);
            return;
        }
        line.push_str(" {");
        self.line(depth, &line);
        for (k, v) in complex {
            self.value(&kdl_string(k), v, depth + 1);
        }
        self.line(depth, "}");
    }

    fn value(&mut self, name: &str, v: &Value, depth: usize) {
        match v {
            Value::Array(items) if items.is_empty() => self.line(depth, &format!("(array){name}")),
            Value::Array(items) if items.iter().all(|i| jik_scalar(i).is_some()) => {
                let vals: Vec<String> = items.iter().filter_map(jik_scalar).collect();
                let prefix = if items.len() == 1 { "(array)" } else { "" };
                self.line(depth, &format!("{prefix}{name} {}", vals.join(" ")));
            }
            Value::Array(items) => {
                self.line(depth, &format!("{name} {{"));
                for item in items {
                    self.value("-", item, depth + 1);
                }
                self.line(depth, "}");
            }
            Value::Object(map) if map.is_empty() => self.line(depth, &format!("(object){name}")),
            Value::Object(map) => self.object(name, map.iter(), depth),
            scalar => self.line(depth, &format!("{name} {}", jik_scalar(scalar).unwrap_or_default())),
        }
    }

    fn prov(&mut self, p: &Prov, depth: usize) {
        let mut line = format!("prov method={}", p.method.name());
        if let Some(by) = &p.by {
            let _ = write!(line, " by={}", quoted(by));
        }
        if let Some(at) = &p.at {
            let _ = write!(line, " at={}", quoted(at));
        }
        match &p.confidence {
            Some(Confidence::Number(n)) => {
                let _ = write!(line, " confidence={n}");
            }
            Some(Confidence::Scheme(s)) => {
                let _ = write!(line, " confidence={}", quoted(s));
            }
            None => {}
        }
        if p.sources.is_empty() && p.derived_from.is_empty() && p.review.is_none() {
            self.line(depth, &line);
            return;
        }
        line.push_str(" {");
        self.line(depth, &line);
        for c in &p.sources {
            let mut l = format!("source {}", kdl_string(&c.source));
            if let Some(loc) = &c.locator {
                let _ = write!(l, " locator={}", quoted(loc));
            }
            if let Some(q) = &c.quote {
                let _ = write!(l, " quote={}", quoted(q));
            }
            self.line(depth + 1, &l);
        }
        if !p.derived_from.is_empty() {
            let ids: Vec<String> = p.derived_from.iter().map(|i| kdl_string(i)).collect();
            self.line(depth + 1, &format!("derived-from {}", ids.join(" ")));
        }
        if let Some(r) = &p.review {
            let mut l = format!("review status={}", r.status.name());
            for (k, v) in [("by", &r.by), ("at", &r.at), ("note", &r.note)] {
                if let Some(v) = v {
                    let _ = write!(l, " {k}={}", quoted(v));
                }
            }
            self.line(depth + 1, &l);
        }
        self.line(depth, "}");
    }
}

fn jik_scalar(v: &Value) -> Option<String> {
    Some(match v {
        Value::Null => "#null".into(),
        Value::Bool(b) => if *b { "#true" } else { "#false" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quoted(s),
        Value::Array(_) | Value::Object(_) => return None,
    })
}

/// `-` reads like an outline, which is what trees and mindmaps are.
fn node_keyword(layout: Option<LayoutKind>) -> &'static str {
    match layout {
        Some(LayoutKind::Layered | LayoutKind::Grid | LayoutKind::Free) => "node",
        _ => "-",
    }
}
