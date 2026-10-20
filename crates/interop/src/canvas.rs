//! Formats that carry geometry: JSON Canvas (Obsidian and others), draw.io
//! (diagrams.net, Confluence), and GraphML (yEd, Gephi, Cytoscape, networkx).
//! Exports take a laid-out [`Diagram`] so positions match rhizo's own render.

use std::collections::HashMap;
use std::fmt::Write;

use rhizo_core::{Arrow, Color, Document, Edge, LayoutKind, LineStyle, Point, Rect, Shape};
use rhizo_lang::Diagnostic;
use rhizo_lang::ast::{EdgeDef, File, FrameDef, NodeDef};
use rhizo_layout::{Diagram, EdgeKind};
use serde::{Deserialize, Serialize};

use crate::util::xml;

// -------------------------------------------------------- JSON Canvas ----

#[derive(Serialize, Deserialize, Default)]
struct Canvas {
    #[serde(default)]
    nodes: Vec<CanvasNode>,
    #[serde(default)]
    edges: Vec<CanvasEdge>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CanvasNode {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CanvasEdge {
    id: String,
    from_node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from_side: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from_end: Option<String>,
    to_node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to_side: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to_end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
}

fn side(r: &Rect, toward: Point) -> &'static str {
    let c = r.center();
    let (dx, dy) = ((toward.x - c.x) / r.width.max(1.0), (toward.y - c.y) / r.height.max(1.0));
    if dx.abs() >= dy.abs() {
        if dx >= 0.0 { "right" } else { "left" }
    } else if dy >= 0.0 {
        "bottom"
    } else {
        "top"
    }
}

pub fn canvas_export(doc: &Document, d: &Diagram) -> String {
    let mut c = Canvas::default();
    for f in d.frames.iter().filter(|f| f.visible) {
        c.nodes.push(CanvasNode {
            id: format!("frame-{}", f.id),
            kind: "group".into(),
            x: f.rect.x.round() as i64,
            y: f.rect.y.round() as i64,
            width: f.rect.width.round() as i64,
            height: f.rect.height.round() as i64,
            label: f.label.clone(),
            ..Default::default()
        });
    }
    let rects: HashMap<&str, Rect> = d.nodes.iter().map(|n| (n.id.as_str(), n.rect)).collect();
    for n in &d.nodes {
        let node = &doc.nodes[&n.id];
        let styled =
            node.style.fill.is_some() || node.class.iter().any(|c| doc.styles.get(c).is_some_and(|s| s.fill.is_some()));
        let mut text = n.label.clone();
        if let Some(note) = &node.note {
            text = format!("{text}\n\n{note}");
        }
        c.nodes.push(CanvasNode {
            id: n.id.to_string(),
            kind: if node.link.is_some() { "link".into() } else { "text".into() },
            x: n.rect.x.round() as i64,
            y: n.rect.y.round() as i64,
            width: n.rect.width.round() as i64,
            height: n.rect.height.round() as i64,
            text: node.link.is_none().then_some(text),
            url: node.link.clone(),
            color: styled.then(|| n.stroke.to_hex_rgb()),
            ..Default::default()
        });
    }
    for (i, e) in d.edges.iter().enumerate() {
        let (Some(ra), Some(rb)) = (rects.get(e.from.as_str()), rects.get(e.to.as_str())) else { continue };
        c.edges.push(CanvasEdge {
            id: format!("edge-{}", i + 1),
            from_node: e.from.to_string(),
            from_side: Some(side(ra, rb.center()).into()),
            from_end: e.arrow.at_start().then(|| "arrow".into()),
            to_node: e.to.to_string(),
            to_side: Some(side(rb, ra.center()).into()),
            to_end: (!e.arrow.at_end()).then(|| "none".into()),
            label: e.label.as_ref().map(|l| l.lines.join(" ")),
            color: (e.kind == EdgeKind::Branch).then(|| e.color.to_hex_rgb()),
        });
    }
    serde_json::to_string_pretty(&c).expect("canvas serializes") + "\n"
}

pub fn canvas_import(src: &str) -> Result<File, Vec<Diagnostic>> {
    let c: Canvas =
        serde_json::from_str(src).map_err(|e| vec![Diagnostic::error(format!("invalid JSON Canvas: {e}"), None)])?;
    let groups: Vec<&CanvasNode> = c.nodes.iter().filter(|n| n.kind == "group").collect();
    let mut file = File { layout: Some(LayoutKind::Free), ..Default::default() };
    let mut frames: Vec<FrameDef> = groups
        .iter()
        .map(|g| FrameDef {
            id: Some(sanitize(&g.id)),
            label: g.label.clone(),
            layout: Some(LayoutKind::Free),
            ..Default::default()
        })
        .collect();
    let inside = |n: &CanvasNode, g: &CanvasNode| {
        n.x >= g.x && n.y >= g.y && n.x + n.width <= g.x + g.width && n.y + n.height <= g.y + g.height
    };
    for n in c.nodes.iter().filter(|n| n.kind != "group") {
        let label = match n.kind.as_str() {
            "text" => n.text.clone().unwrap_or_default(),
            "link" => n.url.clone().unwrap_or_default(),
            "file" => n.file.as_deref().map(|f| f.rsplit('/').next().unwrap_or(f).to_string()).unwrap_or_default(),
            other => other.to_string(),
        };
        // First line is the label, the rest is the note.
        let (label, note) = match label.split_once("\n\n") {
            Some((l, rest)) => (l.trim().to_string(), Some(rest.trim().to_string())),
            None => (label.trim().to_string(), None),
        };
        let mut def = NodeDef::new(label.trim_start_matches('#').trim());
        def.id = Some(sanitize(&n.id));
        def.note = note.filter(|n| !n.is_empty());
        def.link = n.url.clone();
        def.x = Some(n.x as f32);
        def.y = Some(n.y as f32);
        if let Some(c) = n.color.as_deref().and_then(canvas_color) {
            def.style.stroke = Some(c);
            def.style.fill = Some(c.mix(Color::rgb(255, 255, 255), 0.85));
        }
        // Smallest containing group wins.
        let owner = groups
            .iter()
            .enumerate()
            .filter(|(_, g)| inside(n, g))
            .min_by_key(|(_, g)| g.width * g.height)
            .map(|(i, _)| i);
        match owner {
            Some(i) => {
                // Free layout positions are frame-relative.
                def.x = Some((n.x - groups[i].x) as f32);
                def.y = Some((n.y - groups[i].y) as f32);
                frames[i].nodes.push(def);
            }
            None => file.nodes.push(def),
        }
    }
    for (i, g) in groups.iter().enumerate() {
        frames[i].x = Some(g.x as f32);
        frames[i].y = Some(g.y as f32);
    }
    for e in &c.edges {
        let mut def = EdgeDef::new(sanitize(&e.from_node), sanitize(&e.to_node));
        def.label = e.label.clone();
        let start = e.from_end.as_deref() == Some("arrow");
        let end = e.to_end.as_deref() != Some("none");
        def.arrow = match (start, end) {
            (false, true) => None,
            (true, true) => Some(Arrow::Both),
            (true, false) => Some(Arrow::Start),
            (false, false) => Some(Arrow::None),
        };
        def.style.stroke = e.color.as_deref().and_then(canvas_color);
        file.edges.push(def);
    }
    file.frames = frames;
    Ok(file)
}

fn sanitize(id: &str) -> String {
    id.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

/// Canvas preset colors "1"–"6" or hex.
fn canvas_color(c: &str) -> Option<Color> {
    Some(match c {
        "1" => Color::hex(0xfb464c),
        "2" => Color::hex(0xe9973f),
        "3" => Color::hex(0xe0de71),
        "4" => Color::hex(0x44cf6e),
        "5" => Color::hex(0x53dfdd),
        "6" => Color::hex(0xa882ff),
        hex => return hex.parse().ok(),
    })
}

/// The document edge behind each drawn link, matched per (from, to) pair in
/// document order. Every layout pass keeps that order within a pair.
fn edge_sources<'a>(doc: &'a Document, d: &Diagram) -> Vec<Option<&'a Edge>> {
    let mut queues: HashMap<(&str, &str), std::collections::VecDeque<&Edge>> = HashMap::new();
    for e in &doc.edges {
        queues.entry((e.from.as_str(), e.to.as_str())).or_default().push_back(e);
    }
    d.edges
        .iter()
        .map(|eb| match eb.kind {
            EdgeKind::Link => queues.get_mut(&(eb.from.as_str(), eb.to.as_str())).and_then(|q| q.pop_front()),
            EdgeKind::Branch => None,
        })
        .collect()
}

/// draw.io attributes carrying rhizo's typed data, empty for plain items.
fn typed_attrs(ty: Option<&str>, kind: Option<&str>, id: Option<&str>, props: &rhizo_core::Props) -> String {
    let mut out = String::new();
    for (k, v) in [("rhizo-id", id), ("rhizo-type", ty), ("rhizo-kind", kind)] {
        if let Some(v) = v {
            let _ = write!(out, " {k}=\"{}\"", xml(v));
        }
    }
    if !props.is_empty() {
        let json = serde_json::to_string(props).expect("props serialize");
        let _ = write!(out, " rhizo-props=\"{}\"", xml(&json));
    }
    out
}

// ------------------------------------------------------------- draw.io ----

pub fn drawio_export(doc: &Document, d: &Diagram) -> String {
    let mut s = String::new();
    let title = doc.title.as_deref().unwrap_or("Page-1");
    let _ = writeln!(s, "<mxfile host=\"rhizo\" type=\"device\">");
    let _ = writeln!(s, "  <diagram id=\"rhizo\" name=\"{}\">", xml(title));
    let b = d.bounds;
    let _ = writeln!(
        s,
        "    <mxGraphModel dx=\"{}\" dy=\"{}\" grid=\"1\" gridSize=\"10\" guides=\"1\" tooltips=\"1\" connect=\"1\" arrows=\"1\" fold=\"1\" page=\"0\" pageScale=\"1\" math=\"0\" shadow=\"0\" background=\"{}\">",
        b.width.round(),
        b.height.round(),
        d.background.to_hex_rgb()
    );
    s.push_str("      <root>\n        <mxCell id=\"0\"/>\n        <mxCell id=\"1\" parent=\"0\"/>\n");
    let html = |t: &str| xml(&t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>"));
    let mut frame_rect: HashMap<&str, Rect> = HashMap::new();
    for f in d.frames.iter().filter(|f| f.visible) {
        frame_rect.insert(f.id.as_str(), f.rect);
        let _ = writeln!(
            s,
            "        <mxCell id=\"f-{}\" value=\"{}\" style=\"rounded=1;arcSize=4;whiteSpace=wrap;html=1;container=1;collapsible=0;verticalAlign=top;align=left;spacingLeft=16;spacingTop=8;fontStyle=1;fillColor={};strokeColor={};fontColor={};\" vertex=\"1\" parent=\"1\">",
            xml(f.id.as_str()),
            html(f.label.as_deref().unwrap_or("")),
            f.fill.to_hex_rgb(),
            f.stroke.to_hex_rgb(),
            f.label_color.to_hex_rgb()
        );
        let _ = writeln!(
            s,
            "          <mxGeometry x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" as=\"geometry\"/>",
            f.rect.x.round(),
            f.rect.y.round(),
            f.rect.width.round(),
            f.rect.height.round()
        );
        s.push_str("        </mxCell>\n");
    }
    for n in &d.nodes {
        let shape = match n.shape {
            Shape::Rect => "rounded=0;",
            Shape::Rounded => "rounded=1;arcSize=20;",
            Shape::Pill => "rounded=1;arcSize=50;",
            Shape::Ellipse => "ellipse;",
            Shape::Circle => "ellipse;aspect=fixed;",
            Shape::Diamond => "rhombus;",
            Shape::Hexagon => "shape=hexagon;perimeter=hexagonPerimeter2;size=0.2;fixedSize=0;",
            Shape::Text => "text;strokeColor=none;fillColor=none;",
        };
        let dashed = if n.line == LineStyle::Solid { "" } else { "dashed=1;" };
        let bold = if n.font_weight >= 600 { "fontStyle=1;" } else { "" };
        let link = doc.nodes[&n.id].link.as_deref().map(|l| format!(" link=\"{}\"", xml(l))).unwrap_or_default();
        let tooltip = n.note.as_deref().map(|t| format!(" tooltip=\"{}\"", xml(t))).unwrap_or_default();
        let (parent, ox, oy) = match frame_rect.get(n.frame.as_str()) {
            Some(r) => (format!("f-{}", n.frame), r.x, r.y),
            None => ("1".to_string(), 0.0, 0.0),
        };
        let node = &doc.nodes[&n.id];
        let typed = typed_attrs(node.ty.as_ref().map(|t| t.as_str()), None, None, &node.props);
        let has_extra = !link.is_empty() || !tooltip.is_empty() || !typed.is_empty();
        let style = format!(
            "{shape}whiteSpace=wrap;html=1;fillColor={};strokeColor={};fontColor={};fontSize={};strokeWidth={};{dashed}{bold}",
            if n.shape == Shape::Text { "none".into() } else { n.fill.to_hex_rgb() },
            if n.shape == Shape::Text { "none".into() } else { n.stroke.to_hex_rgb() },
            n.text.to_hex_rgb(),
            n.font_size.round(),
            n.stroke_width
        );
        let geometry = format!(
            "<mxGeometry x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" as=\"geometry\"/>",
            (n.rect.x - ox).round(),
            (n.rect.y - oy).round(),
            n.rect.width.round(),
            n.rect.height.round()
        );
        if has_extra {
            // Links, tooltips and typed data live on a wrapping <UserObject>.
            let _ = writeln!(
                s,
                "        <UserObject id=\"n-{}\" label=\"{}\"{link}{tooltip}{typed}>",
                xml(n.id.as_str()),
                html(&n.label)
            );
            let _ = writeln!(
                s,
                "          <mxCell style=\"{style}\" vertex=\"1\" parent=\"{}\">{geometry}</mxCell>",
                xml(&parent)
            );
            s.push_str("        </UserObject>\n");
        } else {
            let _ = writeln!(
                s,
                "        <mxCell id=\"n-{}\" value=\"{}\" style=\"{style}\" vertex=\"1\" parent=\"{}\">{geometry}</mxCell>",
                xml(n.id.as_str()),
                html(&n.label),
                xml(&parent)
            );
        }
    }
    let sources = edge_sources(doc, d);
    for (i, e) in d.edges.iter().enumerate() {
        let start = if e.arrow.at_start() { "startArrow=classic;" } else { "startArrow=none;" };
        let end = if e.arrow.at_end() { "endArrow=classic;" } else { "endArrow=none;" };
        let dashed = match e.line {
            LineStyle::Solid => "",
            LineStyle::Dashed => "dashed=1;",
            LineStyle::Dotted => "dashed=1;dashPattern=1 4;",
        };
        let routing =
            match d.frames.iter().find(|f| d.nodes.iter().any(|n| n.id == e.from && n.frame == f.id)).map(|f| f.layout)
            {
                Some(LayoutKind::Tree) => "edgeStyle=orthogonalEdgeStyle;rounded=1;",
                Some(LayoutKind::Layered) => "edgeStyle=orthogonalEdgeStyle;rounded=1;curved=1;",
                _ => "curved=1;",
            };
        let label = html(&e.label.as_ref().map(|l| l.lines.join("\n")).unwrap_or_default());
        let style = format!(
            "{routing}html=1;strokeColor={};strokeWidth={};fontColor={};{start}{end}{dashed}",
            e.color.to_hex_rgb(),
            e.width,
            e.label.as_ref().map_or(e.color, |l| l.color).to_hex_rgb()
        );
        let ends = format!("source=\"n-{}\" target=\"n-{}\"", xml(e.from.as_str()), xml(e.to.as_str()));
        let geometry = "<mxGeometry relative=\"1\" as=\"geometry\"/>";
        let typed = sources[i]
            .map(|src| {
                typed_attrs(
                    src.ty.as_ref().map(|t| t.as_str()),
                    src.kind.map(|k| k.name()),
                    src.id.as_ref().map(|i| i.as_str()),
                    &src.props,
                )
            })
            .unwrap_or_default();
        if typed.is_empty() {
            let _ = writeln!(
                s,
                "        <mxCell id=\"e-{}\" value=\"{label}\" style=\"{style}\" edge=\"1\" parent=\"1\" {ends}>{geometry}</mxCell>",
                i + 1
            );
        } else {
            let _ = writeln!(s, "        <UserObject id=\"e-{}\" label=\"{label}\"{typed}>", i + 1);
            let _ =
                writeln!(s, "          <mxCell style=\"{style}\" edge=\"1\" parent=\"1\" {ends}>{geometry}</mxCell>");
            s.push_str("        </UserObject>\n");
        }
    }
    s.push_str("      </root>\n    </mxGraphModel>\n  </diagram>\n</mxfile>\n");
    s
}

// ------------------------------------------------------------ GraphML ----

pub fn graphml_export(doc: &Document, d: &Diagram) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd\">\n");
    for (id, for_, name, ty) in [
        ("label", "node", "label", "string"),
        ("note", "node", "note", "string"),
        ("link", "node", "link", "string"),
        ("frame", "node", "frame", "string"),
        ("x", "node", "x", "double"),
        ("y", "node", "y", "double"),
        ("width", "node", "width", "double"),
        ("height", "node", "height", "double"),
        ("fill", "node", "fill", "string"),
        ("shape", "node", "shape", "string"),
        ("elabel", "edge", "label", "string"),
        ("kind", "edge", "kind", "string"),
        ("color", "edge", "color", "string"),
    ] {
        let _ = writeln!(s, "  <key id=\"{id}\" for=\"{for_}\" attr.name=\"{name}\" attr.type=\"{ty}\"/>");
    }
    // Typed keys only when used, so plain exports stay byte-identical.
    let typed = doc.is_typed();
    if typed {
        for (id, for_, name) in [
            ("type", "node", "type"),
            ("props", "node", "props"),
            ("eid", "edge", "id"),
            ("etype", "edge", "type"),
            ("ekind", "edge", "semantic-kind"),
            ("eprops", "edge", "props"),
        ] {
            let _ = writeln!(s, "  <key id=\"{id}\" for=\"{for_}\" attr.name=\"{name}\" attr.type=\"string\"/>");
        }
    }
    let _ = writeln!(s, "  <graph id=\"{}\" edgedefault=\"directed\">", xml(doc.title.as_deref().unwrap_or("rhizo")));
    for n in &d.nodes {
        let node = &doc.nodes[&n.id];
        let _ = writeln!(s, "    <node id=\"{}\">", xml(n.id.as_str()));
        let mut data = |k: &str, v: &str| {
            let _ = writeln!(s, "      <data key=\"{k}\">{}</data>", xml(v));
        };
        data("label", &n.label);
        if let Some(v) = &node.note {
            data("note", v);
        }
        if let Some(v) = &node.link {
            data("link", v);
        }
        data("frame", n.frame.as_str());
        data("x", &n.rect.x.round().to_string());
        data("y", &n.rect.y.round().to_string());
        data("width", &n.rect.width.round().to_string());
        data("height", &n.rect.height.round().to_string());
        data("fill", &n.fill.to_hex_rgb());
        data("shape", n.shape.name());
        if let Some(t) = &node.ty {
            data("type", t.as_str());
        }
        if !node.props.is_empty() {
            data("props", &serde_json::to_string(&node.props).expect("props serialize"));
        }
        s.push_str("    </node>\n");
    }
    let sources = edge_sources(doc, d);
    for (i, e) in d.edges.iter().enumerate() {
        let _ = writeln!(
            s,
            "    <edge id=\"e{}\" source=\"{}\" target=\"{}\">",
            i + 1,
            xml(e.from.as_str()),
            xml(e.to.as_str())
        );
        if let Some(l) = &e.label {
            let _ = writeln!(s, "      <data key=\"elabel\">{}</data>", xml(&l.lines.join(" ")));
        }
        let kind = if e.kind == EdgeKind::Branch { "child" } else { "link" };
        let _ = writeln!(s, "      <data key=\"kind\">{kind}</data>");
        let _ = writeln!(s, "      <data key=\"color\">{}</data>", e.color.to_hex_rgb());
        if let Some(src) = sources[i].filter(|_| typed) {
            let fields = [
                ("eid", src.id.as_ref().map(|v| v.to_string())),
                ("etype", src.ty.as_ref().map(|v| v.to_string())),
                ("ekind", src.kind.map(|k| k.name().to_string())),
                (
                    "eprops",
                    (!src.props.is_empty()).then(|| serde_json::to_string(&src.props).expect("props serialize")),
                ),
            ];
            for (k, v) in fields {
                if let Some(v) = v {
                    let _ = writeln!(s, "      <data key=\"{k}\">{}</data>", xml(&v));
                }
            }
        }
        s.push_str("    </edge>\n");
    }
    s.push_str("  </graph>\n</graphml>\n");
    s
}
