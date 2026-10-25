//! Graphviz DOT and D2 export.

use std::fmt::Write;

use rhizo_core::{Arrow, Direction, Document, LayoutKind, LineStyle, Node, Shape, Style, Theme};

use crate::util::{Ids, Visit, effective, frame_tree, quoted};

fn node_style(doc: &Document, n: &Node) -> Style {
    n.class.iter().filter_map(|c| doc.styles.get(c)).fold(Style::default(), |a, s| a.overlay(s)).overlay(&n.style)
}

fn main_direction(doc: &Document) -> Direction {
    doc.frames
        .first()
        .map(|f| {
            f.direction.unwrap_or(match effective(doc, f) {
                LayoutKind::Mindmap => Direction::Right,
                _ => Direction::Down,
            })
        })
        .unwrap_or(Direction::Down)
}

pub fn dot(doc: &Document) -> String {
    let theme = doc.theme.as_deref().and_then(Theme::by_name).unwrap_or_default();
    let mut ids = Ids::new(&["node", "edge", "graph", "digraph", "subgraph", "strict"]);
    let mut s = String::new();
    let rankdir = match main_direction(doc) {
        Direction::Down => "TB",
        Direction::Up => "BT",
        Direction::Right => "LR",
        Direction::Left => "RL",
    };
    let _ = writeln!(s, "digraph {} {{", quoted(doc.title.as_deref().unwrap_or("rhizo")));
    let _ = writeln!(
        s,
        "    graph [rankdir={rankdir}, fontname=\"Helvetica\", nodesep=0.4, ranksep=0.5, bgcolor=\"{}\"];",
        theme.background.to_hex_rgb()
    );
    if let Some(t) = &doc.title {
        let _ = writeln!(s, "    label={}; labelloc=t; fontsize=18;", quoted(t));
    }
    let _ = writeln!(
        s,
        "    node [shape=box, style=\"rounded,filled\", fillcolor=\"{}\", color=\"{}\", fontcolor=\"{}\", fontname=\"Helvetica\", fontsize=12, margin=\"0.2,0.1\"];",
        theme.node_fill.to_hex_rgb(),
        theme.node_stroke.to_hex_rgb(),
        theme.text.to_hex_rgb()
    );
    let _ = writeln!(
        s,
        "    edge [color=\"{}\", fontname=\"Helvetica\", fontsize=10, fontcolor=\"{}\"];",
        theme.edge.to_hex_rgb(),
        theme.muted_text.to_hex_rgb()
    );

    for visit in frame_tree(doc) {
        let (f, depth) = match visit {
            Visit::Open(f, depth) => (f, depth),
            Visit::Close(f, depth) => {
                if !f.implicit {
                    let _ = writeln!(s, "{}}}", "    ".repeat(depth + 1));
                }
                continue;
            }
        };
        let cluster = !f.implicit;
        let outer = "    ".repeat(depth + 1);
        let indent = if cluster { format!("{outer}    ") } else { outer.clone() };
        if cluster {
            let _ = writeln!(s, "{outer}subgraph cluster_{} {{", ids.get(&format!("frame:{}", f.id)));
            // Nested clusters alternate fills, like the drawing.
            let fill = if depth % 2 == 1 { theme.background } else { theme.frame_fill };
            let _ = writeln!(
                s,
                "{indent}label={}; style=\"rounded,filled\"; fillcolor=\"{}\"; color=\"{}\";",
                quoted(f.label.as_deref().unwrap_or("")),
                fill.to_hex_rgb(),
                theme.frame_stroke.to_hex_rgb()
            );
        }
        for n in doc.nodes_in(&f.id) {
            let st = node_style(doc, n);
            let mut attrs = vec![format!("label={}", quoted(&n.label))];
            let shape = st.shape.unwrap_or(Shape::Rounded);
            attrs.push(match shape {
                Shape::Rect => "style=filled".into(),
                Shape::Rounded => "style=\"rounded,filled\"".into(),
                Shape::Pill => "style=\"rounded,filled\", shape=box, peripheries=1".into(),
                Shape::Ellipse => "shape=ellipse, style=filled".into(),
                Shape::Circle => "shape=circle, style=filled".into(),
                Shape::Diamond => "shape=diamond, style=filled".into(),
                Shape::Hexagon => "shape=hexagon, style=filled".into(),
                Shape::Text => "shape=plaintext".into(),
            });
            if let Some(c) = st.fill {
                attrs.push(format!("fillcolor=\"{c}\""));
            }
            if let Some(c) = st.stroke {
                attrs.push(format!("color=\"{c}\""));
            }
            if let Some(c) = st.text {
                attrs.push(format!("fontcolor=\"{c}\""));
            }
            if let Some(w) = st.stroke_width {
                attrs.push(format!("penwidth={w}"));
            }
            if let Some(l) = &n.link {
                attrs.push(format!("URL={}", quoted(l)));
            }
            if let Some(t) = &n.note {
                attrs.push(format!("tooltip={}", quoted(t)));
            }
            let _ = writeln!(s, "{indent}{} [{}];", ids.get(n.id.as_str()), attrs.join(", "));
        }
    }
    for n in doc.nodes.values() {
        for c in &n.children {
            let _ = writeln!(s, "    {} -> {} [arrowhead=none];", ids.get(n.id.as_str()), ids.get(c.as_str()));
        }
    }
    for e in &doc.edges {
        let mut attrs = Vec::new();
        if let Some(l) = &e.label {
            attrs.push(format!("label={}", quoted(l)));
        }
        match e.arrow {
            Arrow::End => {}
            Arrow::None => attrs.push("dir=none".into()),
            Arrow::Both => attrs.push("dir=both".into()),
            Arrow::Start => attrs.push("dir=back".into()),
        }
        match e.line() {
            LineStyle::Solid => {}
            LineStyle::Dashed => attrs.push("style=dashed".into()),
            LineStyle::Dotted => attrs.push("style=dotted".into()),
        }
        if let Some(c) = e.style.stroke {
            attrs.push(format!("color=\"{c}\""));
        }
        if let Some(w) = e.style.stroke_width {
            attrs.push(format!("penwidth={w}"));
        }
        let a = if attrs.is_empty() { String::new() } else { format!(" [{}]", attrs.join(", ")) };
        let _ = writeln!(s, "    {} -> {}{a};", ids.get(e.from.as_str()), ids.get(e.to.as_str()));
    }
    s.push_str("}\n");
    s
}

pub fn d2(doc: &Document) -> String {
    let mut ids = Ids::new(&[
        "direction",
        "classes",
        "vars",
        "style",
        "shape",
        "label",
        "near",
        "width",
        "height",
        "layers",
        "scenarios",
        "steps",
    ]);
    let mut s = String::new();
    if let Some(t) = &doc.title {
        let _ = writeln!(s, "# {t}");
    }
    let dir = match main_direction(doc) {
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::Right => "right",
        Direction::Left => "left",
    };
    let _ = writeln!(s, "direction: {dir}");
    if !doc.styles.is_empty() {
        s.push_str("\nclasses: {\n");
        for (name, st) in &doc.styles {
            let _ = writeln!(s, "  {}: {{", ids.get(&format!("class:{name}")));
            d2_style(&mut s, st, "    ");
            s.push_str("  }\n");
        }
        s.push_str("}\n");
    }
    let mut path_of = std::collections::HashMap::new();
    // Messages of sequence frames go inside their container: D2 orders them
    // by declaration there.
    let mut written = vec![false; doc.edges.len()];
    // Each open container's path, so nested ones become `outer.inner.node`.
    let mut prefixes: Vec<String> = Vec::new();
    for visit in frame_tree(doc) {
        let (f, depth) = match visit {
            Visit::Open(f, depth) => (f, depth),
            Visit::Close(f, depth) => {
                if !f.implicit {
                    prefixes.pop();
                    let _ = writeln!(s, "{}}}", "  ".repeat(depth));
                }
                continue;
            }
        };
        s.push('\n');
        let container = !f.implicit;
        let outer = "  ".repeat(depth);
        let fid = ids.get(&format!("frame:{}", f.id));
        let prefix = match container {
            true => format!("{}{fid}.", prefixes.last().map_or("", String::as_str)),
            false => String::new(),
        };
        let indent = if container { format!("{outer}  ") } else { outer.clone() };
        let indent = indent.as_str();
        if container {
            let _ = writeln!(s, "{outer}{fid}: {} {{", quoted(f.label.as_deref().unwrap_or(f.id.as_str())));
            prefixes.push(prefix.clone());
        }
        let sequence = effective(doc, f) == LayoutKind::Sequence;
        if sequence {
            // shortcut: an implicit sequence frame makes the whole D2 root a
            // sequence diagram, other frames included; fine while documents
            // mixing it with frames are rare.
            let _ = writeln!(s, "{indent}shape: sequence_diagram");
        }
        for n in doc.nodes_in(&f.id) {
            let id = ids.get(n.id.as_str());
            path_of.insert(n.id.clone(), format!("{prefix}{id}"));
            let mut body = String::new();
            let shape = n.style.shape.or_else(|| n.class.iter().rev().find_map(|c| doc.styles.get(c)?.shape));
            match shape {
                Some(Shape::Ellipse) => body.push_str(&format!("{indent}  shape: oval\n")),
                Some(Shape::Circle) => body.push_str(&format!("{indent}  shape: circle\n")),
                Some(Shape::Diamond) => body.push_str(&format!("{indent}  shape: diamond\n")),
                Some(Shape::Hexagon) => body.push_str(&format!("{indent}  shape: hexagon\n")),
                Some(Shape::Text) => body.push_str(&format!("{indent}  shape: text\n")),
                Some(Shape::Pill) => body.push_str(&format!("{indent}  style.border-radius: 20\n")),
                Some(Shape::Rounded) => body.push_str(&format!("{indent}  style.border-radius: 8\n")),
                Some(Shape::Rect) | None => {}
            }
            let defined: Vec<&String> = n.class.iter().filter(|c| doc.styles.contains_key(*c)).collect();
            if let Some(c) = defined.last() {
                body.push_str(&format!("{indent}  class: {}\n", ids.get(&format!("class:{c}"))));
            }
            let mut st = String::new();
            d2_style(&mut st, &Style { shape: None, ..n.style.clone() }, &format!("{indent}  "));
            body.push_str(&st);
            if let Some(l) = &n.link {
                body.push_str(&format!("{indent}  link: {}\n", quoted(l)));
            }
            if let Some(t) = &n.note {
                body.push_str(&format!("{indent}  tooltip: {}\n", quoted(t)));
            }
            if body.is_empty() {
                let _ = writeln!(s, "{indent}{id}: {}", quoted(&n.label));
            } else {
                let _ = write!(s, "{indent}{id}: {} {{\n{body}{indent}}}\n", quoted(&n.label));
            }
        }
        if sequence {
            for (k, e) in doc.edges.iter().enumerate() {
                let inside = |id| doc.node(id).is_some_and(|n| n.frame == f.id);
                if inside(&e.from) && inside(&e.to) {
                    written[k] = true;
                    d2_edge(&mut s, e, &ids.get(e.from.as_str()), &ids.get(e.to.as_str()), indent);
                }
            }
        }
    }
    s.push('\n');
    for n in doc.nodes.values() {
        for c in &n.children {
            let _ = writeln!(s, "{} -> {}", path_of[&n.id], path_of[c]);
        }
    }
    for (e, _) in doc.edges.iter().zip(&written).filter(|(_, w)| !**w) {
        d2_edge(&mut s, e, &path_of[&e.from], &path_of[&e.to], "");
    }
    s
}

fn d2_edge(s: &mut String, e: &rhizo_core::Edge, from: &str, to: &str, indent: &str) {
    let op = match e.arrow {
        Arrow::End => "->",
        Arrow::Start => "<-",
        Arrow::Both => "<->",
        Arrow::None => "--",
    };
    let mut line = format!("{indent}{from} {op} {to}");
    if let Some(l) = &e.label {
        line.push_str(&format!(": {}", quoted(l)));
    }
    let mut st = String::new();
    if e.line() != LineStyle::Solid {
        st.push_str(&format!("{indent}  style.stroke-dash: 4\n"));
    }
    if let Some(c) = e.style.stroke {
        st.push_str(&format!("{indent}  style.stroke: \"{c}\"\n"));
    }
    if let Some(w) = e.style.stroke_width {
        st.push_str(&format!("{indent}  style.stroke-width: {}\n", w.round().max(1.0)));
    }
    if st.is_empty() {
        let _ = writeln!(s, "{line}");
    } else {
        let _ = write!(s, "{line} {{\n{st}{indent}}}\n");
    }
}

fn d2_style(s: &mut String, st: &Style, indent: &str) {
    if let Some(c) = st.fill {
        let _ = writeln!(s, "{indent}style.fill: \"{c}\"");
    }
    if let Some(c) = st.stroke {
        let _ = writeln!(s, "{indent}style.stroke: \"{c}\"");
    }
    if let Some(c) = st.text {
        let _ = writeln!(s, "{indent}style.font-color: \"{c}\"");
    }
    if let Some(w) = st.stroke_width {
        let _ = writeln!(s, "{indent}style.stroke-width: {}", w.round().clamp(1.0, 15.0));
    }
    if let Some(sz) = st.font_size {
        let _ = writeln!(s, "{indent}style.font-size: {}", sz.round().clamp(8.0, 100.0));
    }
    if st.font_weight.is_some_and(|w| w >= 600) {
        let _ = writeln!(s, "{indent}style.bold: true");
    }
    if matches!(st.line, Some(LineStyle::Dashed | LineStyle::Dotted)) {
        let _ = writeln!(s, "{indent}style.stroke-dash: 4");
    }
}
