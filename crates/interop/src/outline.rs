//! Outline formats: OPML, FreeMind/Freeplane `.mm`, and Markdown.
//!
//! These are trees, so each frame's roots become top-level items. Explicit
//! edges survive in FreeMind (as arrow links) and Markdown (as a list of
//! connections); OPML has no place for them.

use std::fmt::Write;

use rhizo_core::{Color, Document, LayoutKind, NodeId};
use rhizo_lang::Diagnostic;
use rhizo_lang::ast::{EdgeDef, File, NodeDef};

use crate::util::xml;

/// Frames become top-level items when there is more than one, or when the
/// only one has a label.
fn tops(doc: &Document) -> Vec<(Option<String>, Vec<&NodeId>)> {
    let single = doc.frames.len() == 1 && doc.frames[0].label.is_none();
    doc.frames
        .iter()
        .map(|f| {
            let label = if single { None } else { Some(f.label.clone().unwrap_or_else(|| f.id.to_string())) };
            (label, doc.roots(&f.id).map(|n| &n.id).collect::<Vec<_>>())
        })
        .filter(|(_, roots)| !roots.is_empty())
        .collect()
}

// --------------------------------------------------------------- OPML ----

pub fn opml_export(doc: &Document) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n  <head>\n");
    let _ = writeln!(s, "    <title>{}</title>", xml(doc.title.as_deref().unwrap_or("Untitled")));
    s.push_str("  </head>\n  <body>\n");
    fn node(doc: &Document, id: &NodeId, depth: usize, s: &mut String) {
        let n = &doc.nodes[id];
        let pad = "  ".repeat(depth);
        let mut attrs = format!("text=\"{}\"", xml(&n.label));
        if let Some(note) = &n.note {
            let _ = write!(attrs, " _note=\"{}\"", xml(note));
        }
        if let Some(link) = &n.link {
            let _ = write!(attrs, " type=\"link\" url=\"{}\"", xml(link));
        }
        if n.children.is_empty() {
            let _ = writeln!(s, "{pad}<outline {attrs}/>");
        } else {
            let _ = writeln!(s, "{pad}<outline {attrs}>");
            for c in &n.children {
                node(doc, c, depth + 1, s);
            }
            let _ = writeln!(s, "{pad}</outline>");
        }
    }
    for (label, roots) in tops(doc) {
        match label {
            Some(l) => {
                let _ = writeln!(s, "    <outline text=\"{}\">", xml(&l));
                for r in roots {
                    node(doc, r, 3, &mut s);
                }
                s.push_str("    </outline>\n");
            }
            None => {
                for r in roots {
                    node(doc, r, 2, &mut s);
                }
            }
        }
    }
    s.push_str("  </body>\n</opml>\n");
    s
}

fn xml_error(e: roxmltree::Error) -> Vec<Diagnostic> {
    let pos = e.pos();
    vec![Diagnostic::error(format!("invalid XML at line {}, column {}: {e}", pos.row, pos.col), None)]
}

pub fn opml_import(src: &str) -> Result<File, Vec<Diagnostic>> {
    let doc = roxmltree::Document::parse(src).map_err(xml_error)?;
    let root = doc.root_element();
    if root.tag_name().name() != "opml" {
        return Err(vec![Diagnostic::error("not an OPML document (no <opml> root)", None)]);
    }
    let title = root
        .children()
        .find(|n| n.has_tag_name("head"))
        .and_then(|h| h.children().find(|n| n.has_tag_name("title")))
        .and_then(|t| t.text())
        .map(str::to_string);
    let body = root
        .children()
        .find(|n| n.has_tag_name("body"))
        .ok_or_else(|| vec![Diagnostic::error("OPML document has no <body>", None)])?;
    fn outline(n: roxmltree::Node) -> NodeDef {
        let mut def = NodeDef::new(n.attribute("text").or(n.attribute("title")).unwrap_or(""));
        def.note = n.attribute("_note").map(str::to_string);
        def.link = n.attribute("url").or(n.attribute("htmlUrl")).or(n.attribute("xmlUrl")).map(str::to_string);
        def.children = n.children().filter(|c| c.has_tag_name("outline")).map(outline).collect();
        def
    }
    let nodes: Vec<NodeDef> = body.children().filter(|c| c.has_tag_name("outline")).map(outline).collect();
    Ok(File {
        title,
        layout: Some(if nodes.len() == 1 { LayoutKind::Mindmap } else { LayoutKind::Tree }),
        nodes,
        ..Default::default()
    })
}

// ----------------------------------------------------------- FreeMind ----

pub fn freemind_export(doc: &Document) -> String {
    let mut s = String::from("<map version=\"1.0.1\">\n");
    let tops = tops(doc);
    let mut links: std::collections::HashMap<&NodeId, Vec<&rhizo_core::Edge>> = std::collections::HashMap::new();
    for e in &doc.edges {
        links.entry(&e.from).or_default().push(e);
    }
    fn node(
        doc: &Document,
        id: &NodeId,
        depth: usize,
        side: Option<&str>,
        links: &std::collections::HashMap<&NodeId, Vec<&rhizo_core::Edge>>,
        s: &mut String,
    ) {
        let n = &doc.nodes[id];
        let pad = "  ".repeat(depth);
        let mut attrs = format!("ID=\"{}\" TEXT=\"{}\"", xml(&mm_id(id)), xml(&n.label));
        if let Some(side) = side {
            let _ = write!(attrs, " POSITION=\"{side}\"");
        }
        let style = n
            .class
            .iter()
            .filter_map(|c| doc.styles.get(c))
            .fold(rhizo_core::Style::default(), |a, b| a.overlay(b))
            .overlay(&n.style);
        if let Some(c) = style.fill {
            let _ = write!(attrs, " BACKGROUND_COLOR=\"{}\"", c.to_hex_rgb());
        }
        if let Some(c) = style.text {
            let _ = write!(attrs, " COLOR=\"{}\"", c.to_hex_rgb());
        }
        if let Some(l) = &n.link {
            let _ = write!(attrs, " LINK=\"{}\"", xml(l));
        }
        let _ = writeln!(s, "{pad}<node {attrs}>");
        if let Some(note) = &n.note {
            let html: String = note.lines().map(|l| format!("<p>{}</p>", xml(l))).collect();
            let _ =
                writeln!(s, "{pad}  <richcontent TYPE=\"NOTE\"><html><head/><body>{html}</body></html></richcontent>");
        }
        for e in links.get(id).into_iter().flatten() {
            let _ = writeln!(
                s,
                "{pad}  <arrowlink DESTINATION=\"{}\" ENDARROW=\"Default\" STARTARROW=\"None\"/>",
                xml(&mm_id(&e.to))
            );
        }
        let half = n.children.len().div_ceil(2);
        for (i, c) in n.children.iter().enumerate() {
            let side = (depth == 0).then_some(if i < half { "right" } else { "left" });
            node(doc, c, depth + 1, side, links, s);
        }
        let _ = writeln!(s, "{pad}</node>");
    }
    let single_root = tops.len() == 1 && tops[0].0.is_none() && tops[0].1.len() == 1;
    if single_root {
        node(doc, tops[0].1[0], 0, None, &links, &mut s);
    } else {
        // FreeMind needs exactly one root.
        let _ = writeln!(s, "<node ID=\"ID_root\" TEXT=\"{}\">", xml(doc.title.as_deref().unwrap_or("Map")));
        for (label, roots) in &tops {
            match label {
                Some(l) => {
                    let _ = writeln!(s, "  <node TEXT=\"{}\" POSITION=\"right\">", xml(l));
                    for r in roots {
                        node(doc, r, 2, None, &links, &mut s);
                    }
                    s.push_str("  </node>\n");
                }
                None => {
                    for r in roots {
                        node(doc, r, 1, Some("right"), &links, &mut s);
                    }
                }
            }
        }
        s.push_str("</node>\n");
    }
    s.push_str("</map>\n");
    s
}

fn mm_id(id: &NodeId) -> String {
    format!("ID_{}", id.as_str().replace(|c: char| !c.is_ascii_alphanumeric() && c != '_', "_"))
}

pub fn freemind_import(src: &str) -> Result<File, Vec<Diagnostic>> {
    let doc = roxmltree::Document::parse(src).map_err(xml_error)?;
    let map = doc.root_element();
    if map.tag_name().name() != "map" {
        return Err(vec![Diagnostic::error("not a FreeMind/Freeplane map (no <map> root)", None)]);
    }
    let mut edges = Vec::new();
    let mut ids = std::collections::HashMap::new();
    fn text_of(n: roxmltree::Node) -> String {
        if let Some(t) = n.attribute("TEXT") {
            return t.to_string();
        }
        // Rich text node content.
        n.children()
            .find(|c| c.has_tag_name("richcontent") && c.attribute("TYPE") == Some("NODE"))
            .map(|rc| html_text(rc))
            .unwrap_or_default()
    }
    fn walk(
        n: roxmltree::Node,
        edges: &mut Vec<(String, String)>,
        ids: &mut std::collections::HashMap<String, usize>,
        counter: &mut usize,
    ) -> NodeDef {
        let mut def = NodeDef::new(text_of(n));
        *counter += 1;
        let own = n.attribute("ID").map(str::to_string).unwrap_or_else(|| format!("auto_{counter}"));
        def.id = Some(format!("mm-{}", *counter));
        ids.insert(own.clone(), *counter);
        def.link = n.attribute("LINK").map(str::to_string);
        def.style.fill = n.attribute("BACKGROUND_COLOR").and_then(|c| c.parse::<Color>().ok());
        def.style.text = n.attribute("COLOR").and_then(|c| c.parse::<Color>().ok());
        for c in n.children() {
            if c.has_tag_name("node") {
                def.children.push(walk(c, edges, ids, counter));
            } else if c.has_tag_name("arrowlink") {
                if let Some(dest) = c.attribute("DESTINATION") {
                    edges.push((own.clone(), dest.to_string()));
                }
            } else if c.has_tag_name("richcontent") && c.attribute("TYPE") == Some("NOTE") {
                let t = html_text(c);
                if !t.is_empty() {
                    def.note = Some(t);
                }
            }
        }
        def
    }
    let mut counter = 0;
    let nodes: Vec<NodeDef> = map
        .children()
        .filter(|c| c.has_tag_name("node"))
        .map(|n| walk(n, &mut edges, &mut ids, &mut counter))
        .collect();
    // Ids are only needed for arrow links; drop the rest so labels drive ids.
    let linked: std::collections::HashSet<usize> =
        edges.iter().flat_map(|(a, b)| [ids.get(a), ids.get(b)]).flatten().copied().collect();
    fn strip(def: &mut NodeDef, linked: &std::collections::HashSet<usize>) {
        if let Some(n) = def.id.as_deref().and_then(|i| i.strip_prefix("mm-")).and_then(|n| n.parse::<usize>().ok()) {
            if !linked.contains(&n) {
                def.id = None;
            }
        }
        for c in &mut def.children {
            strip(c, linked);
        }
    }
    let mut nodes = nodes;
    for n in &mut nodes {
        strip(n, &linked);
    }
    let edges = edges
        .into_iter()
        .filter_map(|(a, b)| Some(EdgeDef::new(format!("mm-{}", ids.get(&a)?), format!("mm-{}", ids.get(&b)?))))
        .collect();
    Ok(File { layout: Some(LayoutKind::Mindmap), nodes, edges, ..Default::default() })
}

/// Plain text of an HTML fragment: paragraphs and line breaks become newlines.
fn html_text(n: roxmltree::Node) -> String {
    let mut out = String::new();
    for d in n.descendants() {
        if d.is_text() {
            let t = d.text().unwrap_or("").trim();
            if !t.is_empty() {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(t);
            }
        } else if (d.has_tag_name("p") || d.has_tag_name("br") || d.has_tag_name("li"))
            && !out.is_empty()
            && !out.ends_with('\n')
        {
            out.push('\n');
        }
    }
    out.trim().to_string()
}

// ----------------------------------------------------------- Markdown ----

pub fn markdown_export(doc: &Document) -> String {
    let mut s = String::new();
    if let Some(t) = &doc.title {
        // Front matter, as markmap reads it; a heading would become a node.
        let _ = writeln!(s, "---\ntitle: {}\n---\n", t.replace('\n', " "));
    }
    fn item(doc: &Document, id: &NodeId, depth: usize, s: &mut String) {
        let n = &doc.nodes[id];
        let label = n.label.replace('\n', " ");
        let text = match &n.link {
            Some(l) => format!("[{label}]({l})"),
            None => label,
        };
        let _ = writeln!(s, "{}- {text}", "  ".repeat(depth));
        if let Some(note) = &n.note {
            for l in note.lines() {
                let _ = writeln!(s, "{}  > {l}", "  ".repeat(depth));
            }
        }
        for c in &n.children {
            item(doc, c, depth + 1, s);
        }
    }
    for (label, roots) in tops(doc) {
        if let Some(l) = label {
            let _ = writeln!(s, "## {l}\n");
        }
        for r in roots {
            item(doc, r, 0, &mut s);
        }
        s.push('\n');
    }
    if !doc.edges.is_empty() {
        s.push_str("## Connections\n\n");
        for e in &doc.edges {
            let (a, b) = (&doc.nodes[&e.from].label, &doc.nodes[&e.to].label);
            match &e.label {
                Some(l) => {
                    let _ = writeln!(s, "- {a} → {b} ({l})");
                }
                None => {
                    let _ = writeln!(s, "- {a} → {b}");
                }
            }
        }
    }
    s
}

/// Headings and list items form one tree (the markmap convention): a heading
/// nests under the nearest shallower heading, list items nest under the last
/// heading and under each other by indentation.
pub fn markdown_import(src: &str) -> Result<File, Vec<Diagnostic>> {
    // (level, def) where headings use levels 1..=6 and list items 10+indent.
    let mut items: Vec<(usize, NodeDef, Option<usize>)> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut in_code = false;
    let mut title = None;
    let mut body = src;
    if let Some(rest) = src.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            title = rest[..end]
                .lines()
                .find_map(|l| l.strip_prefix("title:"))
                .map(|t| t.trim().trim_matches('"').to_string());
            body = rest[end + 4..].trim_start_matches(['-', '\n']);
        }
    }
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || line.trim().is_empty() {
            continue;
        }
        let (level, text) = if let Some(h) = line.strip_prefix('#') {
            let extra = h.chars().take_while(|&c| c == '#').count();
            let rest = h[extra..].trim();
            if !h[extra..].starts_with(' ') && !rest.is_empty() {
                continue;
            }
            (1 + extra, rest.to_string())
        } else {
            let indent: usize =
                line.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum();
            let t = line.trim_start();
            let body =
                t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")).or_else(|| {
                    let digits = t.chars().take_while(char::is_ascii_digit).count();
                    (digits > 0)
                        .then(|| t[digits..].strip_prefix(". ").or_else(|| t[digits..].strip_prefix(") ")))
                        .flatten()
                });
            match body {
                Some(b) => (10 + indent, b.trim().to_string()),
                None => {
                    // A quote under an item is its note; other prose is skipped.
                    if let Some(q) = t.strip_prefix('>') {
                        if let Some((_, last, _)) = items.last_mut() {
                            let q = q.trim();
                            last.note = Some(match last.note.take() {
                                Some(n) => format!("{n}\n{q}"),
                                None => q.to_string(),
                            });
                        }
                    }
                    continue;
                }
            }
        };
        let text = text.trim_start_matches("[ ] ").trim_start_matches("[x] ").to_string();
        let (label, link) = split_link(&text);
        let mut def = NodeDef::new(strip_inline(&label));
        def.link = link;
        while stack.last().is_some_and(|&(l, _)| l >= level) {
            stack.pop();
        }
        let parent = stack.last().map(|&(_, k)| k);
        items.push((level, def, parent));
        stack.push((level, items.len() - 1));
    }
    if items.is_empty() {
        return Err(vec![Diagnostic::error("no headings or list items found", None)]);
    }
    let mut defs: Vec<Option<NodeDef>> = items.iter().map(|(_, d, _)| Some(d.clone())).collect();
    let mut roots = Vec::new();
    for k in (0..items.len()).rev() {
        let def = defs[k].take().unwrap();
        match items[k].2 {
            Some(p) => defs[p].as_mut().unwrap().children.insert(0, def),
            None => roots.insert(0, def),
        }
    }
    let layout = if roots.len() == 1 { LayoutKind::Mindmap } else { LayoutKind::Tree };
    Ok(File { title, layout: Some(layout), nodes: roots, ..Default::default() })
}

/// `[text](url)` as the whole item becomes a linked node.
fn split_link(s: &str) -> (String, Option<String>) {
    if let (Some(rest), true) = (s.strip_prefix('['), s.ends_with(')')) {
        if let Some((text, url)) = rest.split_once("](") {
            return (text.to_string(), Some(url[..url.len() - 1].to_string()));
        }
    }
    (s.to_string(), None)
}

fn strip_inline(s: &str) -> String {
    s.replace("**", "").replace("__", "").replace('`', "")
}
