//! Mermaid: flowchart/graph, mindmap, sequenceDiagram and stateDiagram
//! import; flowchart, mindmap and sequenceDiagram export.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use rhizo_core::{Arrow, Color, Direction, Document, LayoutKind, LineStyle, Shape, Style};
use rhizo_lang::Diagnostic;
use rhizo_lang::ast::{EdgeDef, File, FrameDef, NodeDef};

use crate::util::{Ids, Visit, effective, frame_tree, line_span};

// ---------------------------------------------------------------- export --

pub fn export(doc: &Document) -> String {
    let single_tree = doc.frames.len() == 1
        && doc.edges.is_empty()
        && matches!(effective(doc, &doc.frames[0]), LayoutKind::Mindmap | LayoutKind::Tree)
        && doc.roots(&doc.frames[0].id).count() == 1;
    let mut out = String::new();
    if let Some(t) = &doc.title {
        let _ = write!(out, "---\ntitle: {}\n---\n", yaml_scalar(t));
    }
    let single_sequence = doc.frames.len() == 1 && effective(doc, &doc.frames[0]) == LayoutKind::Sequence;
    if single_tree {
        mindmap_out(doc, &mut out)
    } else if single_sequence {
        sequence_out(doc, &mut out)
    } else {
        flowchart_out(doc, &mut out)
    }
    out
}

fn yaml_scalar(s: &str) -> String {
    if s.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_') {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Mermaid label text inside `"..."`.
fn label(s: &str) -> String {
    s.replace('"', "#quot;").replace('\n', "<br>")
}

fn mindmap_out(doc: &Document, out: &mut String) {
    out.push_str("mindmap\n");
    let mut ids = Ids::mermaid();
    let root = doc.roots(&doc.frames[0].id).next().unwrap();
    let mut stack = vec![(&root.id, 1usize)];
    while let Some((id, depth)) = stack.pop() {
        let n = &doc.nodes[id];
        let shape = n.style.shape.or_else(|| n.class.iter().find_map(|c| doc.styles.get(c)?.shape));
        let mid = ids.get(n.id.as_str());
        let text = label(&n.label);
        let body = match (depth, shape) {
            (1, None) | (_, Some(Shape::Circle)) => format!("{mid}((\"{text}\"))"),
            (_, Some(Shape::Rect)) => format!("{mid}[\"{text}\"]"),
            (_, Some(Shape::Rounded | Shape::Pill | Shape::Ellipse)) => format!("{mid}(\"{text}\")"),
            (_, Some(Shape::Hexagon)) => format!("{mid}{{{{\"{text}\"}}}}"),
            _ if needs_quotes(&n.label) => format!("{mid}[\"{text}\"]"),
            _ => n.label.clone(),
        };
        let _ = writeln!(out, "{}{}", "  ".repeat(depth), body);
        for c in n.children.iter().rev() {
            stack.push((c, depth + 1));
        }
    }
}

/// `participant` boxes, `actor` for pills (and back on import).
fn sequence_out(doc: &Document, out: &mut String) {
    out.push_str("sequenceDiagram\n");
    let mut ids = Ids::mermaid();
    let frame = &doc.frames[0].id;
    for n in doc.nodes_in(frame) {
        let shape = n.style.shape.or_else(|| n.class.iter().rev().find_map(|c| doc.styles.get(c)?.shape));
        let kind = if shape == Some(Shape::Pill) { "actor" } else { "participant" };
        let id = ids.get(n.id.as_str());
        if id == n.label {
            let _ = writeln!(out, "    {kind} {id}");
        } else {
            let _ = writeln!(out, "    {kind} {id} as {}", message_text(&n.label));
        }
    }
    for e in doc.edges_in(frame) {
        let dashed = e.line() != LineStyle::Solid || style_line(doc, &e.class) != LineStyle::Solid;
        let dash = if dashed { "--" } else { "-" };
        // Mermaid has no arrow at the start only: the message is flipped.
        let (from, to) = if e.arrow == Arrow::Start { (&e.to, &e.from) } else { (&e.from, &e.to) };
        let op = match e.arrow {
            Arrow::End | Arrow::Start => format!("{dash}>>"),
            Arrow::Both => format!("<<{dash}>>"),
            Arrow::None => format!("{dash}>"),
        };
        let text = e.label.as_deref().map(message_text).unwrap_or_default();
        let _ = writeln!(out, "    {}{op}{}: {text}", ids.get(from.as_str()), ids.get(to.as_str()));
    }
}

/// Text after `:` or `as`, which runs to the end of the line.
fn message_text(s: &str) -> String {
    s.replace('#', "#35;").replace(';', "#59;").replace('\n', "<br>")
}

fn needs_quotes(s: &str) -> bool {
    s.is_empty() || s.contains(['(', ')', '[', ']', '{', '}', '\n', '"', ':']) || s.trim() != s
}

fn mermaid_dir(d: Direction) -> &'static str {
    match d {
        Direction::Down => "TD",
        Direction::Up => "BT",
        Direction::Right => "LR",
        Direction::Left => "RL",
    }
}

fn default_dir(kind: LayoutKind) -> Direction {
    match kind {
        LayoutKind::Mindmap => Direction::Right,
        _ => Direction::Down,
    }
}

fn flowchart_out(doc: &Document, out: &mut String) {
    let main = doc.frames.first();
    let dir = main.map_or(Direction::Down, |f| f.direction.unwrap_or(default_dir(effective(doc, f))));
    let _ = writeln!(out, "flowchart {}", mermaid_dir(dir));
    let mut ids = Ids::mermaid();
    let shape_of = |n: &rhizo_core::Node| {
        n.style.shape.or_else(|| n.class.iter().rev().find_map(|c| doc.styles.get(c)?.shape)).unwrap_or(Shape::Rounded)
    };
    let decl = |ids: &mut Ids, n: &rhizo_core::Node| {
        let id = ids.get(n.id.as_str());
        let t = label(&n.label);
        match shape_of(n) {
            Shape::Rect | Shape::Text => format!("{id}[\"{t}\"]"),
            Shape::Rounded => format!("{id}(\"{t}\")"),
            Shape::Pill => format!("{id}([\"{t}\"])"),
            Shape::Ellipse | Shape::Circle => format!("{id}((\"{t}\"))"),
            Shape::Diamond => format!("{id}{{\"{t}\"}}"),
            Shape::Hexagon => format!("{id}{{{{\"{t}\"}}}}"),
        }
    };
    let single = doc.frames.len() == 1 && doc.frames[0].label.is_none();
    for visit in frame_tree(doc) {
        match visit {
            Visit::Open(f, depth) => {
                let nested = !single && !f.implicit;
                let outer = "    ".repeat(depth + 1);
                let indent = if nested { format!("{outer}    ") } else { outer.clone() };
                if nested {
                    let fid = ids.get(&format!("frame:{}", f.id));
                    let name = label(f.label.as_deref().unwrap_or(f.id.as_str()));
                    let _ = writeln!(out, "{outer}subgraph {fid}[\"{name}\"]");
                    let fdir = f.direction.unwrap_or(default_dir(effective(doc, f)));
                    if fdir != dir {
                        let _ = writeln!(out, "{indent}direction {}", mermaid_dir(fdir));
                    }
                }
                for n in doc.nodes_in(&f.id) {
                    let _ = writeln!(out, "{indent}{}", decl(&mut ids, n));
                }
            }
            Visit::Close(f, depth) => {
                if !single && !f.implicit {
                    let _ = writeln!(out, "{}end", "    ".repeat(depth + 1));
                }
            }
        }
    }

    let mut link_styles = Vec::new();
    let mut k = 0usize;
    for n in doc.nodes.values() {
        for c in &n.children {
            let _ = writeln!(out, "    {} --> {}", ids.get(n.id.as_str()), ids.get(c.as_str()));
            k += 1;
        }
    }
    for e in &doc.edges {
        let dashed = e.line() != LineStyle::Solid || style_line(doc, &e.class) != LineStyle::Solid;
        let op = match (e.arrow, dashed) {
            (Arrow::End, false) => "-->",
            (Arrow::None, false) => "---",
            (Arrow::Both, false) => "<-->",
            (Arrow::Start, false) => "<--",
            (Arrow::End, true) => "-.->",
            (Arrow::None, true) => "-.-",
            (Arrow::Both, true) => "<-.->",
            (Arrow::Start, true) => "<-.-",
        };
        let (a, b) = (ids.get(e.from.as_str()), ids.get(e.to.as_str()));
        match &e.label {
            Some(l) => {
                let _ = writeln!(out, "    {a} {op}|\"{}\"| {b}", label(l));
            }
            None => {
                let _ = writeln!(out, "    {a} {op} {b}");
            }
        }
        let css = css(&e.style, true);
        if !css.is_empty() {
            link_styles.push(format!("    linkStyle {k} {css}"));
        }
        k += 1;
    }

    for (name, style) in &doc.styles {
        let css = css(style, false);
        if !css.is_empty() {
            let _ = writeln!(out, "    classDef {} {css}", ids.get(&format!("class:{name}")));
        }
    }
    let mut by_class: Vec<(&String, Vec<String>)> = Vec::new();
    for n in doc.nodes.values() {
        for c in &n.class {
            if !doc.styles.contains_key(c) {
                continue;
            }
            let mid = ids.get(n.id.as_str());
            match by_class.iter_mut().find(|(k, _)| *k == c) {
                Some((_, v)) => v.push(mid),
                None => by_class.push((c, vec![mid])),
            }
        }
    }
    for (c, nodes) in by_class {
        let _ = writeln!(out, "    class {} {}", nodes.join(","), ids.get(&format!("class:{c}")));
    }
    for n in doc.nodes.values() {
        let css = css(&n.style, false);
        if !css.is_empty() {
            let _ = writeln!(out, "    style {} {css}", ids.get(n.id.as_str()));
        }
        if let Some(link) = &n.link {
            let _ = writeln!(out, "    click {} href \"{}\"", ids.get(n.id.as_str()), link.replace('"', "%22"));
        }
    }
    for l in link_styles {
        out.push_str(&l);
        out.push('\n');
    }
}

fn style_line(doc: &Document, classes: &[String]) -> LineStyle {
    classes.iter().rev().find_map(|c| doc.styles.get(c)?.line).unwrap_or_default()
}

fn css(s: &Style, edge: bool) -> String {
    let mut parts = Vec::new();
    if let Some(c) = s.fill.filter(|_| !edge) {
        parts.push(format!("fill:{}", c.to_hex_rgb()));
    }
    if let Some(c) = s.stroke {
        parts.push(format!("stroke:{}", c.to_hex_rgb()));
    }
    if let Some(c) = s.text {
        parts.push(format!("color:{}", c.to_hex_rgb()));
    }
    if let Some(w) = s.stroke_width {
        parts.push(format!("stroke-width:{w}px"));
    }
    if let Some(sz) = s.font_size.filter(|_| !edge) {
        parts.push(format!("font-size:{sz}px"));
    }
    if let Some(w) = s.font_weight.filter(|_| !edge) {
        parts.push(format!("font-weight:{w}"));
    }
    if matches!(s.line, Some(LineStyle::Dashed | LineStyle::Dotted)) && !edge {
        parts.push("stroke-dasharray:5 5".into());
    }
    parts.join(",")
}

// ---------------------------------------------------------------- import --

/// Warnings (constructs that were dropped) come back with the file.
pub fn import(src: &str) -> Result<(File, Vec<Diagnostic>), Vec<Diagnostic>> {
    let (title, body_start) = front_matter(src);
    let mut lines = src[body_start..].lines().enumerate().peekable();
    let offset = |line_idx: usize| line_span(src, body_start, line_idx);
    // Find the header.
    while let Some((_, l)) = lines.peek() {
        let t = strip_comment(l).trim();
        if t.is_empty() {
            lines.next();
        } else {
            break;
        }
    }
    let Some((hi, header)) = lines.next() else {
        return Err(vec![Diagnostic::error("empty Mermaid document", None)]);
    };
    let header = strip_comment(header).trim();
    let kind = header.split_whitespace().next().unwrap_or("");
    let rest: Vec<(usize, &str)> = lines.collect();
    let mut warnings = Vec::new();
    let mut file = match kind {
        "flowchart" | "graph" => Flowchart::new(src, body_start).run(header, &rest)?,
        "mindmap" => mindmap(&rest, &offset)?,
        "sequenceDiagram" => sequence(&rest, &offset, &mut warnings),
        "stateDiagram" | "stateDiagram-v2" => state_diagram(&rest, &offset)?,
        other => {
            return Err(vec![
                Diagnostic::error(format!("Mermaid `{other}` diagrams are not supported yet"), offset(hi))
                    .with_help("supported: flowchart, graph, mindmap, sequenceDiagram, stateDiagram"),
            ]);
        }
    };
    if file.title.is_none() {
        file.title = title;
    }
    Ok((file, warnings))
}

/// Message arrows, longest first so `-->>` is not read as `-->`.
const MESSAGE_ARROWS: &[(&str, Arrow, LineStyle)] = &[
    ("<<-->>", Arrow::Both, LineStyle::Dashed),
    ("<<->>", Arrow::Both, LineStyle::Solid),
    ("-->>", Arrow::End, LineStyle::Dashed),
    ("--x", Arrow::End, LineStyle::Dashed),
    ("--)", Arrow::End, LineStyle::Dashed),
    ("->>", Arrow::End, LineStyle::Solid),
    ("-->", Arrow::None, LineStyle::Dashed),
    ("-x", Arrow::End, LineStyle::Solid),
    ("-)", Arrow::End, LineStyle::Solid),
    ("->", Arrow::None, LineStyle::Solid),
];

/// Participants, actors, messages and notes. Blocks (`alt`, `loop`, ...),
/// activations and numbering are dropped with one warning each; the
/// messages inside blocks are kept.
// shortcut: blocks are flattened, upgrade once rhizo has fragments
// (e.g. frames inside a sequence) to keep `alt`/`loop` structure.
fn sequence(
    lines: &[(usize, &str)],
    offset: &dyn Fn(usize) -> rhizo_lang::ast::Span,
    warnings: &mut Vec<Diagnostic>,
) -> File {
    let mut file = File { layout: Some(LayoutKind::Sequence), ..Default::default() };
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut touch = |id: &str, nodes: &mut Vec<NodeDef>| -> usize {
        *seen.entry(id.to_string()).or_insert_with(|| {
            let mut d = NodeDef::new(id);
            d.id = Some(id.to_string());
            nodes.push(d);
            nodes.len() - 1
        })
    };
    let mut dropped: Vec<&str> = Vec::new();
    for &(i, raw) in lines {
        let t = strip_comment(raw).trim();
        let t = t.strip_prefix("create ").unwrap_or(t).trim();
        let word = t.split_whitespace().next().unwrap_or("");
        let rest = t[word.len()..].trim();
        match word {
            "" => {}
            "participant" | "actor" => {
                let (id, label) = match rest.split_once(" as ") {
                    Some((id, label)) => (id.trim(), Some(decode(label))),
                    None => (rest, None),
                };
                let k = touch(id, &mut file.nodes);
                let def = &mut file.nodes[k];
                def.span = offset(i);
                if let Some(label) = label {
                    def.label = label;
                }
                if word == "actor" {
                    def.style.shape = Some(Shape::Pill);
                }
            }
            "title" => file.title = Some(decode(rest.trim_start_matches(':'))),
            w if w.eq_ignore_ascii_case("note") => {
                // `Note right of A: text`, `Note over A,B: text`: kept on the first participant.
                let Some((place, text)) = rest.split_once(':') else { continue };
                let who = place.split_whitespace().last().unwrap_or("").split(',').next().unwrap_or("").trim();
                if who.is_empty() {
                    continue;
                }
                let k = touch(who, &mut file.nodes);
                let note = &mut file.nodes[k].note;
                let text = decode(text);
                *note = Some(match note.take() {
                    Some(old) => format!("{old}\n{text}"),
                    None => text,
                });
            }
            // Block continuations; the opening keyword already warned.
            "accTitle:" | "accDescr:" | "accDescr" | "end" | "else" | "and" | "option" => {}
            _ => {
                let Some((a, arrow, line, b, text)) = message(t) else {
                    if !dropped.contains(&word) {
                        dropped.push(word);
                        warnings.push(Diagnostic::warning(
                            format!("Mermaid `{word}` has no rhizo equivalent yet; the line was skipped"),
                            offset(i),
                        ));
                    }
                    continue;
                };
                let (a, b) = (touch(a, &mut file.nodes), touch(b, &mut file.nodes));
                let id = |k: usize| file.nodes[k].id.clone().unwrap();
                let mut e = EdgeDef::new(id(a), id(b));
                e.label = text.map(decode).filter(|l| !l.is_empty());
                e.arrow = Some(arrow).filter(|a| *a != Arrow::End);
                e.style.line = Some(line).filter(|l| *l != LineStyle::Solid);
                e.span = offset(i);
                file.edges.push(e);
            }
        }
    }
    file
}

/// `A->>+B: text` as (from, arrow, line, to, text).
fn message(t: &str) -> Option<(&str, Arrow, LineStyle, &str, Option<&str>)> {
    let (head, text) = match t.split_once(':') {
        Some((h, x)) => (h, Some(x)),
        None => (t, None),
    };
    let (at, &(op, arrow, line)) = (1..head.len())
        .filter(|&i| head.is_char_boundary(i))
        .find_map(|i| MESSAGE_ARROWS.iter().find(|(op, ..)| head[i..].starts_with(op)).map(|m| (i, m)))?;
    let from = head[..at].trim();
    // `+`/`-` after the arrow (de)activates the receiver.
    let to = head[at + op.len()..].trim().trim_start_matches(['+', '-']).trim();
    (!from.is_empty() && !to.is_empty() && !to.contains(char::is_whitespace)).then_some((from, arrow, line, to, text))
}

fn front_matter(src: &str) -> (Option<String>, usize) {
    let trimmed = src.trim_start();
    let lead = src.len() - trimmed.len();
    if !trimmed.starts_with("---") {
        return (None, 0);
    }
    let after = &trimmed[3..];
    let Some(end) = after.find("\n---") else { return (None, 0) };
    let fm = &after[..end];
    let title =
        fm.lines().find_map(|l| l.trim().strip_prefix("title:")).map(|t| t.trim().trim_matches('"').to_string());
    let close = lead + 3 + end + 4;
    let next_line = src[close..].find('\n').map_or(src.len(), |i| close + i + 1);
    (title, next_line)
}

fn strip_comment(l: &str) -> &str {
    match l.find("%%") {
        Some(i) => &l[..i],
        None => l,
    }
}

fn decode(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    let s = s.strip_prefix('`').and_then(|x| x.strip_suffix('`')).unwrap_or(s);
    let mut out = s
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n")
        .replace("#quot;", "\"")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    // #35; style numeric entities
    while let Some(i) = out.find('#') {
        let Some(j) = out[i..].find(';') else { break };
        let Ok(code) = out[i + 1..i + j].parse::<u32>() else { break };
        let Some(ch) = char::from_u32(code) else { break };
        out.replace_range(i..i + j + 1, &ch.to_string());
    }
    out
}

const SHAPES: &[(&str, &str, Shape)] = &[
    ("(((", ")))", Shape::Circle),
    ("((", "))", Shape::Circle),
    ("([", "])", Shape::Pill),
    ("[[", "]]", Shape::Rect),
    ("[(", ")]", Shape::Rect),
    ("[/", "/]", Shape::Rect),
    ("[/", "\\]", Shape::Rect),
    ("[\\", "\\]", Shape::Rect),
    ("[\\", "/]", Shape::Rect),
    ("{{", "}}", Shape::Hexagon),
    ("[", "]", Shape::Rect),
    ("(", ")", Shape::Rounded),
    ("{", "}", Shape::Diamond),
    (">", "]", Shape::Rect),
];

/// Parses a bracketed shape at the start of `s`: returns (text, shape, bytes consumed).
fn shape_at(s: &str) -> Option<(String, Shape, usize)> {
    for &(open, close, shape) in SHAPES {
        let Some(rest) = s.strip_prefix(open) else { continue };
        let (text, used) = if rest.trim_start().starts_with('"') {
            let lead = rest.len() - rest.trim_start().len();
            let q = &rest[lead + 1..];
            let end = q.find('"')?;
            let after = &q[end + 1..];
            let after_trim = after.trim_start();
            if !after_trim.starts_with(close) {
                continue;
            }
            (q[..end].to_string(), lead + 1 + end + 1 + (after.len() - after_trim.len()) + close.len())
        } else {
            let end = rest.find(close)?;
            (rest[..end].to_string(), end + close.len())
        };
        return Some((decode(&text), shape, open.len() + used));
    }
    None
}

#[derive(Default)]
struct Link {
    arrow: Arrow,
    line: LineStyle,
    thick: bool,
    label: Option<String>,
}

struct Flowchart<'a> {
    src: &'a str,
    base: usize,
    file: File,
    /// Node id → (frame index or None, NodeDef)
    nodes: Vec<(Option<usize>, NodeDef)>,
    index: HashMap<String, usize>,
    frames: Vec<FrameDef>,
    /// The subgraph each frame was declared in, by index.
    parents: Vec<Option<usize>>,
    stack: Vec<usize>,
    classes: HashMap<String, Style>,
    edge_styles: Vec<(String, Style)>,
    diags: Vec<Diagnostic>,
}

impl<'a> Flowchart<'a> {
    fn new(src: &'a str, base: usize) -> Self {
        Flowchart {
            src,
            base,
            file: File::default(),
            nodes: Vec::new(),
            index: HashMap::new(),
            frames: Vec::new(),
            parents: Vec::new(),
            stack: Vec::new(),
            classes: HashMap::new(),
            edge_styles: Vec::new(),
            diags: Vec::new(),
        }
    }

    fn run(mut self, header: &str, lines: &[(usize, &str)]) -> Result<File, Vec<Diagnostic>> {
        let dir = header.split_whitespace().nth(1).and_then(parse_dir).unwrap_or(Direction::Down);
        self.file.layout = Some(LayoutKind::Layered);
        self.file.direction = Some(dir);
        for &(i, raw) in lines {
            let span = line_span(self.src, self.base, i);
            for stmt in split_statements(strip_comment(raw)) {
                let stmt = stmt.trim();
                if !stmt.is_empty() {
                    self.statement(stmt, span);
                }
            }
        }
        if !self.stack.is_empty() {
            self.diags.push(Diagnostic::error("`subgraph` without matching `end`", None));
        }
        if self.diags.iter().any(Diagnostic::is_error) {
            return Err(self.diags);
        }
        self.finish()
    }

    fn statement(&mut self, stmt: &str, span: rhizo_lang::ast::Span) {
        let word = stmt.split_whitespace().next().unwrap_or("");
        let rest = stmt[word.len()..].trim();
        match word {
            "subgraph" => {
                let (id, label) = match shape_at_after_id(rest) {
                    Some((id, Some((text, _)))) => (id, text),
                    Some((id, None)) if !rest.contains(' ') => (id.clone(), id),
                    _ => {
                        let l = decode(rest);
                        (rhizo_core::slugify(&l), l)
                    }
                };
                self.frames.push(FrameDef {
                    id: Some(id),
                    label: Some(label),
                    layout: Some(LayoutKind::Layered),
                    ..Default::default()
                });
                self.parents.push(self.stack.last().copied());
                self.stack.push(self.frames.len() - 1);
            }
            "end" if rest.is_empty() => {
                if self.stack.pop().is_none() {
                    self.diags.push(Diagnostic::error("`end` without a `subgraph`", span));
                }
            }
            "direction" => {
                let d = parse_dir(rest);
                match self.stack.last() {
                    Some(&f) => self.frames[f].direction = d,
                    None => self.file.direction = d.or(self.file.direction),
                }
            }
            "classDef" => {
                let mut parts = rest.splitn(2, char::is_whitespace);
                let names = parts.next().unwrap_or("");
                let style = parse_css(parts.next().unwrap_or(""));
                for n in names.split(',') {
                    self.classes.insert(n.trim().to_string(), style.clone());
                }
            }
            "class" => {
                let mut parts = rest.rsplitn(2, char::is_whitespace);
                let class = parts.next().unwrap_or("").to_string();
                for id in parts.next().unwrap_or("").split(',') {
                    let k = self.touch(id.trim(), None);
                    self.nodes[k].1.class.push(class.clone());
                }
            }
            "style" => {
                let mut parts = rest.splitn(2, char::is_whitespace);
                let id = parts.next().unwrap_or("");
                let style = parse_css(parts.next().unwrap_or(""));
                let k = self.touch(id, None);
                self.nodes[k].1.style = self.nodes[k].1.style.overlay(&style);
            }
            "linkStyle" => {
                let mut parts = rest.splitn(2, char::is_whitespace);
                let which = parts.next().unwrap_or("").to_string();
                let style = parse_css(parts.next().unwrap_or(""));
                self.edge_styles.push((which, style));
            }
            "click" => {
                let mut parts = rest.split_whitespace();
                let id = parts.next().unwrap_or("");
                let url = parts.find(|p| p.starts_with('"')).map(|u| u.trim_matches('"').to_string());
                if let Some(url) = url {
                    let k = self.touch(id, None);
                    self.nodes[k].1.link = Some(url);
                }
            }
            "accTitle:" | "accDescr:" | "accDescr" | "title" => {}
            _ => self.chain(stmt, span),
        }
    }

    fn current_frame(&self) -> Option<usize> {
        self.stack.last().copied()
    }

    /// Records a node mention; the first mention decides its frame.
    fn touch(&mut self, id: &str, shape: Option<(String, Shape)>) -> usize {
        let k = match self.index.get(id) {
            Some(&k) => k,
            None => {
                let mut def = NodeDef::new(id);
                def.id = Some(id.to_string());
                self.nodes.push((self.current_frame(), def));
                self.index.insert(id.to_string(), self.nodes.len() - 1);
                self.nodes.len() - 1
            }
        };
        if let Some((text, shape)) = shape {
            let def = &mut self.nodes[k].1;
            def.label = text;
            def.style.shape = Some(shape);
        }
        k
    }

    /// `a`, `a[Label]:::cls`, or `a & b(Other)`.
    fn group(&mut self, p: &mut Cursor, stmt: &str, span: rhizo_lang::ast::Span) -> Option<Vec<usize>> {
        let mut group = Vec::new();
        loop {
            p.skip_ws();
            let Some(id) = p.ident() else {
                self.diags.push(Diagnostic::error(format!("expected a node id in `{stmt}`"), span));
                return None;
            };
            let shape = if p.rest().starts_with("@{") {
                p.new_shape_syntax()
            } else {
                shape_at(p.rest()).map(|(text, shape, used)| {
                    p.i += used;
                    (text, shape)
                })
            };
            let k = self.touch(&id, shape);
            if let Some(after) = p.rest().strip_prefix(":::") {
                let name: String =
                    after.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
                p.i += 3 + name.len();
                self.nodes[k].1.class.push(name);
            }
            group.push(k);
            p.skip_ws();
            if p.rest().starts_with('&') {
                p.i += 1;
                continue;
            }
            return Some(group);
        }
    }

    /// `A --> B & C -- text --> D`
    fn chain(&mut self, stmt: &str, span: rhizo_lang::ast::Span) {
        let mut p = Cursor { s: stmt, i: 0 };
        let Some(mut left) = self.group(&mut p, stmt, span) else { return };
        loop {
            p.skip_ws();
            if p.done() {
                return;
            }
            let Some(link) = p.link() else {
                self.diags.push(Diagnostic::error(format!("could not read `{}`", p.rest().trim()), span));
                return;
            };
            let Some(right) = self.group(&mut p, stmt, span) else { return };
            for &a in &left {
                for &b in &right {
                    let mut e = EdgeDef::new(self.nodes[a].1.id.clone().unwrap(), self.nodes[b].1.id.clone().unwrap());
                    e.label = link.label.clone();
                    e.arrow = Some(link.arrow).filter(|a| *a != Arrow::End);
                    if link.line != LineStyle::Solid {
                        e.style.line = Some(link.line);
                    }
                    if link.thick {
                        e.style.stroke_width = Some(3.0);
                    }
                    e.span = span;
                    self.file.edges.push(e);
                }
            }
            left = right;
        }
    }

    fn finish(mut self) -> Result<File, Vec<Diagnostic>> {
        // Edge styles from linkStyle.
        let n = self.file.edges.len();
        for (which, style) in std::mem::take(&mut self.edge_styles) {
            let targets: Vec<usize> = if which == "default" {
                (0..n).collect()
            } else {
                which.split(',').filter_map(|i| i.trim().parse().ok()).filter(|&i| i < n).collect()
            };
            for i in targets {
                let e = &mut self.file.edges[i];
                e.style = e.style.overlay(&Style { fill: None, ..style.clone() });
            }
        }
        for (name, style) in std::mem::take(&mut self.classes) {
            self.file.styles.insert(name, style);
        }
        // Drop classes that have no definition (Mermaid built-ins).
        let known: HashSet<String> = self.file.styles.keys().cloned().collect();
        for (_, def) in &mut self.nodes {
            def.class.retain(|c| known.contains(c));
        }
        for (frame, def) in std::mem::take(&mut self.nodes) {
            match frame {
                Some(f) => self.frames[f].nodes.push(def),
                None => self.file.nodes.push(def),
            }
        }
        // Nested subgraphs become nested frames; a child always follows its parent.
        let mut frames: Vec<Option<FrameDef>> = self.frames.into_iter().map(Some).collect();
        for i in (0..frames.len()).rev() {
            if let Some(p) = self.parents[i] {
                let child = frames[i].take().expect("children are moved once");
                frames[p].as_mut().expect("parents come first").frames.insert(0, child);
            }
        }
        self.file.frames = frames.into_iter().flatten().collect();
        Ok(self.file)
    }
}

fn shape_at_after_id(s: &str) -> Option<(String, Option<(String, Shape)>)> {
    let mut c = Cursor { s, i: 0 };
    let id = c.ident()?;
    c.skip_ws();
    let shape = shape_at(c.rest()).map(|(t, sh, _)| (t, sh));
    Some((id, shape))
}

fn parse_dir(s: &str) -> Option<Direction> {
    match s.trim() {
        "TB" | "TD" => Some(Direction::Down),
        "BT" => Some(Direction::Up),
        "LR" => Some(Direction::Right),
        "RL" => Some(Direction::Left),
        _ => None,
    }
}

fn parse_css(s: &str) -> Style {
    let mut style = Style::default();
    for part in s.trim().trim_end_matches(';').split([',', ';']) {
        let Some((k, v)) = part.split_once(':') else { continue };
        let v = v.trim();
        match k.trim() {
            "fill" => style.fill = v.parse::<Color>().ok(),
            "stroke" => style.stroke = v.parse::<Color>().ok(),
            "color" => style.text = v.parse::<Color>().ok(),
            "stroke-width" => style.stroke_width = v.trim_end_matches("px").parse().ok(),
            "font-size" => style.font_size = v.trim_end_matches("px").parse().ok(),
            "font-weight" => style.font_weight = if v == "bold" { Some(700) } else { v.parse().ok() },
            "stroke-dasharray" => style.line = Some(LineStyle::Dashed),
            _ => {}
        }
    }
    style
}

/// Splits on `;` outside of brackets and quotes.
fn split_statements(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quoted, mut start) = (0i32, false, 0);
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth -= 1,
            ';' if !quoted && depth <= 0 => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out
}

struct Cursor<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }

    fn done(&self) -> bool {
        self.rest().trim().is_empty()
    }

    fn skip_ws(&mut self) {
        let r = self.rest();
        self.i += r.len() - r.trim_start().len();
    }

    /// A node id: stops at whitespace, shape brackets, `&`, `:::`, or a link.
    fn ident(&mut self) -> Option<String> {
        let r = self.rest();
        let mut end = 0;
        let bytes = r.as_bytes();
        while end < r.len() {
            let c = r[end..].chars().next().unwrap();
            if c.is_whitespace() || "[({>&;\"|".contains(c) || r[end..].starts_with(":::") || r[end..].starts_with("@{")
            {
                break;
            }
            if (c == '-' || c == '=' || c == '.' || c == '<' || c == '~')
                && end + 1 < r.len()
                && matches!(bytes[end + 1], b'-' | b'=' | b'.' | b'>')
            {
                break;
            }
            end += c.len_utf8();
        }
        if end == 0 {
            return None;
        }
        self.i += end;
        Some(r[..end].to_string())
    }

    /// The new `A@{ shape: diamond, label: "x" }` syntax.
    fn new_shape_syntax(&mut self) -> Option<(String, Shape)> {
        let r = self.rest();
        let end = r.find('}')?;
        let body = &r[2..end];
        self.i += end + 1;
        let mut label = None;
        let mut shape = Shape::Rounded;
        for part in body.split(',') {
            let Some((k, v)) = part.split_once(':') else { continue };
            let v = v.trim().trim_matches('"');
            match k.trim() {
                "label" => label = Some(decode(v)),
                "shape" => {
                    shape = match v {
                        "diam" | "diamond" | "decision" | "question" => Shape::Diamond,
                        "circle" | "circ" | "dbl-circ" | "sm-circ" | "f-circ" => Shape::Circle,
                        "stadium" | "pill" | "terminal" => Shape::Pill,
                        "hex" | "hexagon" | "prepare" => Shape::Hexagon,
                        "rounded" | "event" => Shape::Rounded,
                        "text" => Shape::Text,
                        _ => Shape::Rect,
                    }
                }
                _ => {}
            }
        }
        Some((label.unwrap_or_default(), shape))
    }

    fn link(&mut self) -> Option<Link> {
        let save = self.i;
        let r = self.rest();
        let mut link = Link::default();
        let mut k = 0;
        let b = r.as_bytes();
        let start_head = matches!(b.first(), Some(b'<'))
            || (matches!(b.first(), Some(b'x' | b'o')) && matches!(b.get(1), Some(b'-' | b'=')));
        if start_head {
            k += 1;
        }
        let body_start = k;
        if r[k..].starts_with("-.") {
            link.line = LineStyle::Dashed;
            k += 1;
            while b.get(k) == Some(&b'.') {
                k += 1;
            }
            if b.get(k) == Some(&b'-') {
                k += 1;
            }
        } else if r[k..].starts_with("--") {
            while b.get(k) == Some(&b'-') {
                k += 1;
            }
        } else if r[k..].starts_with("==") {
            link.thick = true;
            while b.get(k) == Some(&b'=') {
                k += 1;
            }
        } else if r[k..].starts_with("~~~") {
            k += 3;
            while b.get(k) == Some(&b'~') {
                k += 1;
            }
            link.line = LineStyle::Dotted;
        } else {
            self.i = save;
            return None;
        }
        let body = &r[body_start..k];
        let mut end_head = false;
        if matches!(b.get(k), Some(b'>'))
            || (matches!(b.get(k), Some(b'x' | b'o')) && b.get(k + 1).is_none_or(|c| c.is_ascii_whitespace()))
        {
            end_head = true;
            k += 1;
        }
        // Inline text form: `A -- text --> B`.
        if !end_head && matches!(body, "--" | "==" | "-.") && b.get(k).is_some_and(|c| c.is_ascii_whitespace()) {
            let closer = match body {
                "--" => "--",
                "==" => "==",
                _ => ".-",
            };
            if let Some(pos) = r[k..].find(closer) {
                let text = r[k..k + pos].trim();
                let mut j = k + pos + closer.len();
                while matches!(b.get(j), Some(b'-' | b'=' | b'.')) {
                    j += 1;
                }
                if matches!(b.get(j), Some(b'>' | b'x' | b'o')) {
                    end_head = true;
                    j += 1;
                }
                link.label = Some(decode(text));
                k = j;
            }
        }
        link.arrow = match (start_head, end_head) {
            (false, true) => Arrow::End,
            (true, true) => Arrow::Both,
            (true, false) => Arrow::Start,
            (false, false) => Arrow::None,
        };
        self.i += k;
        self.skip_ws();
        if self.rest().starts_with('|') {
            let r = self.rest();
            if let Some(end) = r[1..].find('|') {
                link.label = Some(decode(&r[1..1 + end]));
                self.i += end + 2;
            }
        }
        Some(link)
    }
}

fn mindmap(lines: &[(usize, &str)], offset: &dyn Fn(usize) -> rhizo_lang::ast::Span) -> Result<File, Vec<Diagnostic>> {
    // (indent, def, parent index)
    let mut items: Vec<(usize, NodeDef, Option<usize>)> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut diags = Vec::new();
    for &(i, raw) in lines {
        let line = strip_comment(raw);
        if line.trim().is_empty() {
            continue;
        }
        let indent: usize = line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum();
        let t = line.trim();
        if t.starts_with("::icon(") || t.starts_with(":::") {
            continue;
        }
        let mut def = match shape_at_after_id(t) {
            Some((id, Some((text, shape)))) => {
                let mut d = NodeDef::new(text);
                d.style.shape = Some(match shape {
                    Shape::Rect => Shape::Rect,
                    Shape::Rounded => Shape::Rounded,
                    other => other,
                });
                if id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') && id != d.label {
                    d.id = Some(id);
                }
                d
            }
            _ => match shape_at(t) {
                Some((text, shape, _)) => {
                    let mut d = NodeDef::new(text);
                    d.style.shape = Some(shape);
                    d
                }
                None => NodeDef::new(decode(t)),
            },
        };
        def.span = offset(i);
        while stack.last().is_some_and(|&(ind, _)| ind >= indent) {
            stack.pop();
        }
        let parent = stack.last().map(|&(_, k)| k);
        if parent.is_none() && !items.is_empty() {
            diags.push(Diagnostic::error(
                "a mindmap has exactly one root; this line is not indented under it",
                offset(i),
            ));
            continue;
        }
        items.push((indent, def, parent));
        stack.push((indent, items.len() - 1));
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    // Root's shape is the theme's job unless it was set explicitly as a circle.
    if let Some(root) = items.first_mut()
        && root.1.style.shape == Some(Shape::Circle)
    {
        root.1.style.shape = None;
    }
    let mut defs: Vec<Option<NodeDef>> = items.iter().map(|(_, d, _)| Some(d.clone())).collect();
    for k in (1..items.len()).rev() {
        let child = defs[k].take().unwrap();
        let p = items[k].2.unwrap();
        defs[p].as_mut().unwrap().children.insert(0, child);
    }
    let mut file = File { layout: Some(LayoutKind::Mindmap), ..Default::default() };
    if let Some(Some(root)) = defs.into_iter().next() {
        file.nodes.push(root);
    }
    Ok(file)
}

fn state_diagram(
    lines: &[(usize, &str)],
    offset: &dyn Fn(usize) -> rhizo_lang::ast::Span,
) -> Result<File, Vec<Diagnostic>> {
    let mut file = File { layout: Some(LayoutKind::Layered), direction: Some(Direction::Down), ..Default::default() };
    let mut nodes: Vec<NodeDef> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut touch = |id: &str, nodes: &mut Vec<NodeDef>| -> String {
        let (key, label, shape) = match id {
            "[*]" => ("start".to_string(), "●".to_string(), Some(Shape::Circle)),
            _ => (id.to_string(), id.to_string(), None),
        };
        if !seen.contains_key(&key) {
            let mut d = NodeDef::new(label);
            d.id = Some(key.clone());
            d.style.shape = shape;
            nodes.push(d);
            seen.insert(key.clone(), nodes.len() - 1);
        }
        key
    };
    let mut ends = 0;
    for &(i, raw) in lines {
        let t = strip_comment(raw).trim();
        if t.is_empty() || t == "}" || t.starts_with("direction") || t.starts_with("note") || t.starts_with("classDef")
        {
            if let Some(d) = t.strip_prefix("direction") {
                file.direction = parse_dir(d);
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("state ") {
            // state "Long name" as id   |   state id { (composites are flattened)
            if let Some((name, id)) = rest.split_once(" as ") {
                let id = id.trim().trim_end_matches('{').trim();
                let key = touch(id, &mut nodes);
                let k = seen_index(&nodes, &key);
                nodes[k].label = decode(name);
            }
            continue;
        }
        if let Some((lhs, rhs)) = t.split_once("-->") {
            let (rhs, label) = match rhs.split_once(':') {
                Some((r, l)) => (r, Some(l.trim().to_string())),
                None => (rhs, None),
            };
            let a = touch(lhs.trim(), &mut nodes);
            let b_raw = rhs.trim();
            let b = if b_raw == "[*]" {
                ends += 1;
                let key = format!("end-{ends}");
                let mut d = NodeDef::new("◉");
                d.id = Some(key.clone());
                d.style.shape = Some(Shape::Circle);
                nodes.push(d);
                key
            } else {
                touch(b_raw, &mut nodes)
            };
            let mut e = EdgeDef::new(a, b);
            e.label = label;
            e.span = offset(i);
            file.edges.push(e);
            continue;
        }
        if let Some((id, desc)) = t.split_once(':') {
            let key = touch(id.trim(), &mut nodes);
            let k = seen_index(&nodes, &key);
            nodes[k].label = format!("{}\n{}", nodes[k].label, desc.trim());
            continue;
        }
        if t.chars().all(|c| c.is_alphanumeric() || c == '_') {
            touch(t, &mut nodes);
            continue;
        }
        // Anything else (notes, forks, concurrency) has no rhizo equivalent yet.
    }
    file.nodes = nodes;
    Ok(file)
}

fn seen_index(nodes: &[NodeDef], key: &str) -> usize {
    nodes.iter().position(|n| n.id.as_deref() == Some(key)).unwrap()
}
