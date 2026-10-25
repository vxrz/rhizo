//! Semantic, accessible SVG.
//!
//! Output uses presentation attributes (so every SVG consumer, including
//! resvg and krilla, renders it identically) plus stable classes and ids
//! (`rz-node`, `node-<id>`, `rz-depth-1`, user classes) for styling and
//! scripting in the browser.

use std::collections::BTreeSet;
use std::fmt::Write;

use rhizo_core::{Color, LineStyle, Shape};
use rhizo_layout::{Diagram, EdgeBox, EdgeKind, FrameBox, Ink, LabelBox, NodeBox, Path, Seg, Typed};

#[derive(Clone, Debug, Default)]
pub struct SvgOptions {
    /// Omit the background rectangle.
    pub transparent: bool,
    /// Font family list written into the SVG. Defaults to the diagram's.
    pub font_family: Option<String>,
}

pub fn to_svg(d: &Diagram, opts: &SvgOptions) -> String {
    let b = d.bounds;
    let mut s = String::with_capacity(4096 + d.nodes.len() * 400);
    let family = opts.font_family.as_deref().unwrap_or(&d.font_family);
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="{} {} {} {}" width="{}" height="{}" role="img" aria-labelledby="rz-title" font-family="{}" class="rhizo">"#,
        n(b.x),
        n(b.y),
        n(b.width),
        n(b.height),
        n(b.width.ceil()),
        n(b.height.ceil()),
        attr(family)
    );
    s.push('\n');
    let title = d.title.as_deref().unwrap_or("Diagram");
    let _ = writeln!(s, r#"<title id="rz-title">{}</title>"#, text(title));
    let connections = d.edges.iter().filter(|e| e.kind != EdgeKind::Lifeline).count();
    let _ = writeln!(s, "<desc>{} nodes, {connections} connections. Made with rhizo.</desc>", d.nodes.len());

    let arrow_colors: BTreeSet<String> =
        d.edges.iter().filter(|e| e.arrow.at_end() || e.arrow.at_start()).map(|e| e.color.to_hex_rgb()).collect();
    if !arrow_colors.is_empty() {
        s.push_str("<defs>\n");
        for c in &arrow_colors {
            let _ = writeln!(
                s,
                r#"<marker id="rz-arrow-{}" viewBox="0 0 10 10" refX="9" refY="5" markerUnits="userSpaceOnUse" markerWidth="10" markerHeight="10" orient="auto-start-reverse"><path d="M1,1.5 L9,5 L1,8.5 Z" fill="{}"/></marker>"#,
                &c[1..],
                c
            );
        }
        s.push_str("</defs>\n");
    }

    if !opts.transparent {
        let _ = writeln!(
            s,
            r#"<rect class="rz-background" x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
            n(b.x),
            n(b.y),
            n(b.width),
            n(b.height),
            d.background.to_hex_rgb()
        );
    }

    s.push_str("<g class=\"rz-frames\">\n");
    for f in d.frames.iter().filter(|f| f.visible) {
        frame(&mut s, f);
    }
    s.push_str("</g>\n<g class=\"rz-edges\">\n");
    for e in &d.edges {
        edge(&mut s, e);
    }
    s.push_str("</g>\n<g class=\"rz-nodes\">\n");
    for nb in &d.nodes {
        node(&mut s, nb);
    }
    s.push_str("</g>\n</svg>\n");
    s
}

fn frame(s: &mut String, f: &FrameBox) {
    let r = f.rect;
    // Top-level frames keep the bare class, so plain documents render byte-identically.
    let depth = if f.depth > 0 { format!(" rz-frame-nested rz-depth-{}", f.depth) } else { String::new() };
    let _ = write!(s, r#"<g class="rz-frame{depth}" id="frame-{}">"#, attr(f.id.as_str()));
    let _ = write!(
        s,
        r#"<rect x="{}" y="{}" width="{}" height="{}" rx="14" fill="{}"{} stroke="{}" stroke-width="1"/>"#,
        n(r.x),
        n(r.y),
        n(r.width),
        n(r.height),
        f.fill.to_hex_rgb(),
        opacity("fill-opacity", f.fill),
        f.stroke.to_hex_rgb()
    );
    if let Some(label) = &f.label {
        let _ = write!(
            s,
            r#"<text x="{}" y="{}" font-size="{}" font-weight="600" fill="{}" letter-spacing="0.02em">{}</text>"#,
            n(r.x + 20.0),
            n(r.y + 16.0 + f.label_size * 0.8),
            n(f.label_size),
            f.label_color.to_hex_rgb(),
            text(label)
        );
    }
    s.push_str("</g>\n");
}

fn dash(line: LineStyle, width: f32) -> String {
    match line {
        LineStyle::Solid => String::new(),
        LineStyle::Dashed => format!(r#" stroke-dasharray="{} {}""#, n(width * 4.0 + 2.0), n(width * 3.0 + 2.0)),
        LineStyle::Dotted => format!(r#" stroke-dasharray="0.1 {}" stroke-linecap="round""#, n(width * 2.5 + 1.5)),
    }
}

fn path_d(p: &Path) -> String {
    let mut d = format!("M{},{}", n(p.start.x), n(p.start.y));
    for seg in &p.segs {
        match *seg {
            Seg::Line(q) => {
                let _ = write!(d, " L{},{}", n(q.x), n(q.y));
            }
            Seg::Cubic(a, b, q) => {
                let _ = write!(d, " C{},{} {},{} {},{}", n(a.x), n(a.y), n(b.x), n(b.y), n(q.x), n(q.y));
            }
        }
    }
    d
}

fn edge(s: &mut String, e: &EdgeBox) {
    let kind = match e.kind {
        EdgeKind::Branch => "rz-branch",
        EdgeKind::Link => "rz-link",
        EdgeKind::Lifeline => "rz-lifeline",
    };
    let _ = write!(
        s,
        r#"<g class="rz-edge {kind}{}" data-from="{}" data-to="{}">"#,
        user_classes(&e.class),
        attr(e.from.as_str()),
        attr(e.to.as_str())
    );
    let marker = |at: bool, which: &str| {
        if at { format!(r#" {which}="url(#rz-arrow-{})""#, &e.color.to_hex_rgb()[1..]) } else { String::new() }
    };
    let cap = if e.line == LineStyle::Dotted { "" } else { r#" stroke-linecap="round""# };
    let _ = write!(
        s,
        r#"<path d="{}" fill="none" stroke="{}"{} stroke-width="{}"{}{}{}{}/>"#,
        path_d(&e.path),
        e.color.to_hex_rgb(),
        opacity("stroke-opacity", e.color),
        n(e.width),
        cap,
        dash(e.line, e.width),
        marker(e.arrow.at_end(), "marker-end"),
        marker(e.arrow.at_start(), "marker-start"),
    );
    if let Some(l) = &e.label {
        label(s, l);
    }
    s.push_str("</g>\n");
}

fn label(s: &mut String, l: &LabelBox) {
    let r = l.rect();
    let _ = write!(
        s,
        r#"<g class="rz-edge-label"><rect x="{}" y="{}" width="{}" height="{}" rx="4" fill="{}" fill-opacity="0.92"/>"#,
        n(r.x),
        n(r.y),
        n(r.width),
        n(r.height),
        l.background.to_hex_rgb()
    );
    text_block(s, &l.lines, l.center.x, l.center.y, l.font_size, 400, l.line_height, l.color);
    s.push_str("</g>");
}

fn node(s: &mut String, nb: &NodeBox) {
    if let Some(link) = &nb.link {
        let _ = write!(s, r#"<a href="{0}" xlink:href="{0}">"#, attr(link));
    }
    let _ = write!(
        s,
        r#"<g class="rz-node rz-shape-{} rz-depth-{}{}{}{}" id="node-{}">"#,
        nb.shape.name(),
        nb.depth,
        nb.branch.map(|b| format!(" rz-branch-{b}")).unwrap_or_default(),
        nb.typed.as_deref().map(typed_classes).unwrap_or_default(),
        user_classes(&nb.class),
        attr(nb.id.as_str())
    );
    if let Some(note) = &nb.note {
        let _ = write!(s, "<title>{}</title>", text(note));
    }
    let r = nb.rect;
    let paint = format!(
        r#" fill="{}"{} stroke="{}"{} stroke-width="{}"{}"#,
        nb.fill.to_hex_rgb(),
        opacity("fill-opacity", nb.fill),
        nb.stroke.to_hex_rgb(),
        opacity("stroke-opacity", nb.stroke),
        n(nb.stroke_width),
        dash(nb.line, nb.stroke_width)
    );
    match nb.shape {
        Shape::Rect | Shape::Rounded | Shape::Pill => {
            let rx = match nb.shape {
                Shape::Rect => 2.0,
                Shape::Rounded => 8.0f32.min(r.height / 2.0),
                _ => r.height / 2.0,
            };
            let _ = write!(
                s,
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}"{paint}/>"#,
                n(r.x),
                n(r.y),
                n(r.width),
                n(r.height),
                n(rx)
            );
        }
        Shape::Ellipse | Shape::Circle => {
            let c = r.center();
            let _ = write!(
                s,
                r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}"{paint}/>"#,
                n(c.x),
                n(c.y),
                n(r.width / 2.0),
                n(r.height / 2.0)
            );
        }
        Shape::Diamond => {
            let c = r.center();
            let _ = write!(
                s,
                r#"<path d="M{},{} L{},{} L{},{} L{},{} Z" stroke-linejoin="round"{paint}/>"#,
                n(c.x),
                n(r.y),
                n(r.right()),
                n(c.y),
                n(c.x),
                n(r.bottom()),
                n(r.x),
                n(c.y)
            );
        }
        Shape::Hexagon => {
            let inset = (r.height * 0.35).min(r.width / 2.0);
            let c = r.center();
            let _ = write!(
                s,
                r#"<path d="M{},{} L{},{} L{},{} L{},{} L{},{} L{},{} Z" stroke-linejoin="round"{paint}/>"#,
                n(r.x + inset),
                n(r.y),
                n(r.right() - inset),
                n(r.y),
                n(r.right()),
                n(c.y),
                n(r.right() - inset),
                n(r.bottom()),
                n(r.x + inset),
                n(r.bottom()),
                n(r.x),
                n(c.y)
            );
        }
        Shape::Text => {}
    }
    if nb.ink().is_empty() {
        let c = r.center();
        text_block(s, &nb.lines, c.x, c.y, nb.font_size, nb.font_weight, nb.line_height, nb.text);
    }
    for i in nb.ink() {
        ink(s, &i.clone().offset(rhizo_core::Point::new(r.x, r.y)));
    }
    s.push_str("</g>");
    if nb.link.is_some() {
        s.push_str("</a>");
    }
    s.push('\n');
}

/// `rz-type-data-entity rz-card rz-state-draft`, with a leading space.
fn typed_classes(t: &Typed) -> String {
    let mut c = String::new();
    // Kit type names are `[a-z0-9-]` segments joined by dots; CURIE types may hold anything.
    if let Some(ty) =
        t.ty.as_deref().filter(|ty| ty.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '-'))
    {
        let _ = write!(c, " rz-type-{}", ty.replace('.', "-"));
    }
    for (on, name) in [(t.card, "rz-card"), (t.row, "rz-row"), (t.container, "rz-container")] {
        if on {
            let _ = write!(c, " {name}");
        }
    }
    if t.state != rhizo_core::DecorState::Normal {
        let _ = write!(c, " rz-state-{}", t.state.name());
    }
    c
}

/// One mark of a typed node's drawing, in diagram coordinates.
fn ink(s: &mut String, i: &Ink) {
    match i {
        Ink::Text { text: t, x, baseline, size, weight, color, end, class } => {
            let weight = if *weight != 400 { format!(r#" font-weight="{weight}""#) } else { String::new() };
            let anchor = if *end { r#" text-anchor="end""# } else { "" };
            let _ = write!(
                s,
                r#"<text class="{class}" x="{}" y="{}" font-size="{}"{weight} fill="{}"{anchor}>{}</text>"#,
                n(*x),
                n(*baseline),
                n(*size),
                color.to_hex_rgb(),
                text(t)
            );
        }
        Ink::Tag { rect, fill, text: t, size, color, class } => {
            let c = rect.center();
            let _ = write!(
                s,
                r#"<g class="{class}"><rect x="{}" y="{}" width="{}" height="{}" rx="3" fill="{}"/><text x="{}" y="{}" font-size="{}" font-weight="700" fill="{}" text-anchor="middle">{}</text></g>"#,
                n(rect.x),
                n(rect.y),
                n(rect.width),
                n(rect.height),
                fill.to_hex_rgb(),
                n(c.x),
                n(c.y + size * 0.35),
                n(*size),
                color.to_hex_rgb(),
                text(t)
            );
        }
        Ink::Rule { from, to, color, width } => {
            let _ = write!(
                s,
                r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="{}"/>"#,
                n(from.x),
                n(from.y),
                n(to.x),
                n(to.y),
                color.to_hex_rgb(),
                n(*width)
            );
        }
        Ink::Band { rect: r, radius, fill } => {
            let rr = radius.min(r.height).min(r.width / 2.0).max(0.0);
            let _ = write!(
                s,
                r#"<path class="rz-card-header" d="M{},{} L{},{} A{},{} 0 0 1 {},{} L{},{} A{},{} 0 0 1 {},{} L{},{} Z" fill="{}"/>"#,
                n(r.x),
                n(r.bottom()),
                n(r.x),
                n(r.y + rr),
                n(rr),
                n(rr),
                n(r.x + rr),
                n(r.y),
                n(r.right() - rr),
                n(r.y),
                n(rr),
                n(rr),
                n(r.right()),
                n(r.y + rr),
                n(r.right()),
                n(r.bottom()),
                fill.to_hex_rgb()
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn text_block(s: &mut String, lines: &[String], cx: f32, cy: f32, size: f32, weight: u16, lh: f32, color: Color) {
    let top = cy - lh * lines.len() as f32 / 2.0;
    // Center each line box on its x-height: baseline ≈ line middle + 0.35em.
    let baseline = |i: usize| top + lh * (i as f32 + 0.5) + size * 0.35;
    let weight_attr = if weight != 400 { format!(r#" font-weight="{weight}""#) } else { String::new() };
    let _ = write!(
        s,
        r#"<text x="{}" y="{}" font-size="{}"{weight_attr} fill="{}" text-anchor="middle">"#,
        n(cx),
        n(baseline(0)),
        n(size),
        color.to_hex_rgb()
    );
    if lines.len() == 1 {
        s.push_str(&text(&lines[0]));
    } else {
        for (i, l) in lines.iter().enumerate() {
            let _ = write!(s, r#"<tspan x="{}" y="{}">{}</tspan>"#, n(cx), n(baseline(i)), text(l));
        }
    }
    s.push_str("</text>");
}

fn user_classes(classes: &[String]) -> String {
    classes
        .iter()
        .filter(|c| c.chars().all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '_'))
        .map(|c| format!(" {c}"))
        .collect()
}

fn opacity(name: &str, c: Color) -> String {
    if c.is_opaque() { String::new() } else { format!(r#" {name}="{}""#, n(c.opacity())) }
}

/// Compact number formatting: at most two decimals, no trailing zeros.
fn n(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').to_string()
    }
}

fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

fn attr(s: &str) -> String {
    text(s).replace('"', "&quot;")
}
