//! Reads the KDL text format into the authoring AST.
//!
//! ```kdl
//! title "Launch plan"
//! style "risk" fill="#fee2e2" stroke="#ef4444"
//!
//! frame "Strategy" layout=mindmap {
//!     - "Launch" {
//!         - "Users" { - "Personas"; - "Jobs to be done" }
//!         - "Pricing" class=risk
//!     }
//! }
//!
//! frame "Release flow" layout=layered direction=down {
//!     node "Build" shape=pill
//!     node "Tests pass?" id=check shape=diamond
//!     edge build check
//!     edge check "Ship it" label=yes
//! }
//! ```

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use miette::SourceSpan;
use rhizo_core::{
    Arrow, Citation, Color, Confidence, Direction, EdgeKind, KitRef, LayoutKind, LineStyle, Port, PortDir, PortName,
    Props, Prov, ProvMethod, Review, ReviewStatus, Shape, Style,
};
use serde_json::Value;

use crate::ast::{EdgeDef, File, FrameDef, NodeDef, Span};
use crate::diag::{Diagnostic, suggest};

pub const FORMAT_VERSION: i128 = 2;

const TOP_LEVEL: &[&str] =
    &["rhizo", "title", "theme", "layout", "direction", "use", "base", "prefix", "style", "frame", "node", "-", "edge"];
const STYLE_PROPS: &[&str] = &["fill", "stroke", "text", "stroke-width", "font-size", "font-weight", "line", "shape"];
const NODE_PROPS: &[&str] = &["id", "label", "class", "x", "y", "note", "link", "tags", "type"];
const EDGE_PROPS: &[&str] = &["label", "arrow", "class", "id", "type", "kind", "from-port", "to-port"];
const FRAME_PROPS: &[&str] = &["id", "label", "layout", "direction", "x", "y", "columns", "type"];

/// Parses KDL source. Returns the AST and every diagnostic; the AST is only
/// meaningful when no diagnostic is an error.
pub fn parse(src: &str) -> (File, Vec<Diagnostic>) {
    let doc = match KdlDocument::parse(src) {
        Ok(doc) => doc,
        Err(err) => {
            let diags = err
                .diagnostics
                .into_iter()
                .map(|d| {
                    let mut out = Diagnostic::error(d.message.unwrap_or_else(|| "invalid KDL".into()), span(d.span));
                    out.label = d.label;
                    out.help = d.help;
                    out
                })
                .collect();
            return (File::default(), diags);
        }
    };
    let mut p = Parser { diags: Vec::new() };
    let file = p.file(&doc);
    (file, p.diags)
}

fn span(s: SourceSpan) -> Span {
    Some((s.offset(), s.len()))
}

struct Parser {
    diags: Vec<Diagnostic>,
}

/// A node's arguments and properties, split.
struct Entries<'a> {
    args: Vec<&'a KdlEntry>,
    props: Vec<(&'a str, &'a KdlEntry)>,
}

fn entries(node: &KdlNode) -> Entries<'_> {
    let mut e = Entries { args: Vec::new(), props: Vec::new() };
    for entry in node.entries() {
        match entry.name() {
            Some(name) => e.props.push((name.value(), entry)),
            None => e.args.push(entry),
        }
    }
    e
}

fn is_string_named(node: &KdlNode) -> bool {
    node.name().repr().is_some_and(|r| r.starts_with('"') || r.starts_with('#'))
}

impl Parser {
    fn err(&mut self, msg: impl Into<String>, s: SourceSpan) -> &mut Diagnostic {
        self.diags.push(Diagnostic::error(msg, span(s)));
        self.diags.last_mut().unwrap()
    }

    fn warn(&mut self, msg: impl Into<String>, s: SourceSpan) {
        self.diags.push(Diagnostic::warning(msg, span(s)));
    }

    fn unknown(&mut self, what: &str, word: &str, s: SourceSpan, known: &[&str]) {
        let d = self.err(format!("unknown {what} `{word}`"), s);
        if let Some(close) = suggest(word, known.iter().copied()) {
            d.help = Some(format!("did you mean `{close}`?"));
        } else {
            d.help = Some(format!("expected one of: {}", known.join(", ")));
        }
    }

    fn string(&mut self, e: &KdlEntry) -> Option<String> {
        match e.value() {
            KdlValue::String(s) => Some(s.clone()),
            // Numbers and booleans are accepted where a label is expected.
            KdlValue::Integer(i) => Some(i.to_string()),
            KdlValue::Float(f) => Some(f.to_string()),
            KdlValue::Bool(b) => Some(b.to_string()),
            KdlValue::Null => {
                self.err("expected a string, found #null", e.span());
                None
            }
        }
    }

    fn number(&mut self, e: &KdlEntry) -> Option<f32> {
        match e.value() {
            KdlValue::Integer(i) => Some(*i as f32),
            KdlValue::Float(f) => Some(*f as f32),
            _ => {
                self.err("expected a number", e.span());
                None
            }
        }
    }

    fn uint(&mut self, e: &KdlEntry) -> Option<u32> {
        match e.value() {
            KdlValue::Integer(i) if *i >= 0 && *i <= u32::MAX as i128 => Some(*i as u32),
            _ => {
                self.err("expected a positive whole number", e.span());
                None
            }
        }
    }

    fn keyword<T>(&mut self, e: &KdlEntry, what: &str, parse: fn(&str) -> Option<T>, known: &[&str]) -> Option<T> {
        let s = self.string(e)?;
        let v = parse(&s);
        if v.is_none() {
            self.unknown(what, &s, e.span(), known);
        }
        v
    }

    fn list(&mut self, e: &KdlEntry) -> Vec<String> {
        self.string(e)
            .map(|s| s.split([' ', ',']).filter(|w| !w.is_empty()).map(String::from).collect())
            .unwrap_or_default()
    }

    fn color(&mut self, e: &KdlEntry) -> Option<Color> {
        let s = self.string(e)?;
        match s.parse() {
            Ok(c) => Some(c),
            Err(err) => {
                self.err(err.to_string(), e.span());
                None
            }
        }
    }

    /// Applies a style property. Returns false if `key` is not a style property.
    fn style_prop(&mut self, style: &mut Style, key: &str, e: &KdlEntry) -> bool {
        match key {
            "fill" => style.fill = self.color(e),
            "stroke" => style.stroke = self.color(e),
            "text" | "color" => style.text = self.color(e),
            "stroke-width" => style.stroke_width = self.number(e),
            "font-size" => style.font_size = self.number(e),
            "font-weight" => {
                style.font_weight = match e.value() {
                    KdlValue::String(s) if s == "bold" => Some(700),
                    KdlValue::String(s) if s == "normal" => Some(400),
                    _ => self.uint(e).map(|w| w.clamp(100, 900) as u16),
                }
            }
            "line" => style.line = self.keyword(e, "line style", LineStyle::parse, &["solid", "dashed", "dotted"]),
            "shape" => {
                let names: Vec<&str> = Shape::ALL.iter().map(|s| s.name()).collect();
                style.shape = self.keyword(e, "shape", Shape::parse, &names)
            }
            _ => return false,
        }
        true
    }

    fn file(&mut self, doc: &KdlDocument) -> File {
        let mut file = File::default();
        for node in doc.nodes() {
            let name = node.name().value();
            if is_string_named(node) {
                if let Some(n) = self.node(node, Some(name.to_string())) {
                    file.nodes.push(n);
                }
                continue;
            }
            match name {
                "rhizo" => file.rhizo = self.header(node),
                "use" => {
                    let e = entries(node);
                    let kit = e.args.first().and_then(|a| self.string(a));
                    let mut version = None;
                    for (key, entry) in e.props {
                        match key {
                            "version" => version = self.string(entry),
                            other => self.unknown("`use` property", other, entry.span(), &["version"]),
                        }
                    }
                    match kit {
                        Some(kit) if e.args.len() == 1 => file.kits.push(KitRef { kit, version }),
                        _ => {
                            self.err("`use` takes one kit name", node.span()).help =
                                Some("use infra version=\"0.1\"".into());
                        }
                    }
                }
                "base" => file.base = self.single_string(node),
                "prefix" => {
                    let e = entries(node);
                    let pair: Vec<String> = e.args.iter().filter_map(|a| self.string(a)).collect();
                    match pair.as_slice() {
                        [name, iri] if e.props.is_empty() => {
                            file.prefixes.insert(name.clone(), iri.clone());
                        }
                        _ => {
                            self.err("`prefix` takes a name and an IRI", node.span()).help =
                                Some("prefix HP \"http://purl.obolibrary.org/obo/HP_\"".into());
                        }
                    }
                }
                "title" => file.title = self.single_string(node),
                "theme" => file.theme = self.single_string(node),
                "layout" => {
                    file.layout = self.single(node).and_then(|e| self.keyword(e, "layout", LayoutKind::parse, LAYOUTS))
                }
                "direction" => {
                    file.direction =
                        self.single(node).and_then(|e| self.keyword(e, "direction", Direction::parse, DIRECTIONS))
                }
                "style" => {
                    if let Some((name, style)) = self.style_def(node) {
                        if file.styles.insert(name.clone(), style).is_some() {
                            self.warn(format!("style `{name}` is defined twice; the last one wins"), node.span());
                        }
                    }
                }
                "frame" => {
                    if let Some(f) = self.frame(node) {
                        file.frames.push(f);
                    }
                }
                "node" | "-" => {
                    if let Some(n) = self.node(node, None) {
                        file.nodes.push(n);
                    }
                }
                "edge" => file.edges.extend(self.edge(node)),
                other => self.unknown("statement", other, node.name().span(), TOP_LEVEL),
            }
        }
        file
    }

    fn header(&mut self, node: &KdlNode) -> Option<u32> {
        let mut version = None;
        for (key, e) in entries(node).props {
            match key {
                "version" => match e.value() {
                    KdlValue::Integer(v) if (1..=FORMAT_VERSION).contains(v) => version = Some(*v as u32),
                    KdlValue::Integer(v) if *v > FORMAT_VERSION => {
                        self.err(format!("this file needs rhizo format version {v}"), e.span()).help =
                            Some(format!("this build reads version {FORMAT_VERSION}; upgrade rhizo"));
                    }
                    _ => {
                        self.err("version must be a whole number", e.span());
                    }
                },
                other => self.unknown("header property", other, e.span(), &["version"]),
            }
        }
        version
    }

    fn single<'n>(&mut self, node: &'n KdlNode) -> Option<&'n KdlEntry> {
        let e = entries(node);
        if e.args.len() != 1 || !e.props.is_empty() {
            self.err(format!("`{}` takes exactly one value", node.name().value()), node.span());
            return None;
        }
        e.args.first().copied()
    }

    fn single_string(&mut self, node: &KdlNode) -> Option<String> {
        let e = self.single(node)?;
        self.string(e)
    }

    fn style_def(&mut self, node: &KdlNode) -> Option<(String, Style)> {
        let e = entries(node);
        let Some(name) = e.args.first().and_then(|a| self.string(a)) else {
            self.err("a style needs a name", node.span()).help = Some("style \"accent\" fill=\"#fde68a\"".into());
            return None;
        };
        let mut style = Style::default();
        for (key, entry) in e.props {
            if !self.style_prop(&mut style, key, entry) {
                self.unknown("style property", key, entry.span(), STYLE_PROPS);
            }
        }
        self.no_children(node);
        Some((name, style))
    }

    fn no_children(&mut self, node: &KdlNode) {
        if node.children().is_some_and(|c| !c.nodes().is_empty()) {
            self.err(format!("`{}` cannot have a block", node.name().value()), node.span());
        }
    }

    fn frame(&mut self, node: &KdlNode) -> Option<FrameDef> {
        let e = entries(node);
        let mut f = FrameDef { span: span(node.span()), ..Default::default() };
        if let Some(a) = e.args.first() {
            f.label = self.string(a);
        }
        for extra in e.args.iter().skip(1) {
            self.err("a frame takes one label", extra.span());
        }
        for (key, entry) in e.props {
            match key {
                "id" => f.id = self.string(entry),
                "label" => f.label = self.string(entry),
                "layout" => f.layout = self.keyword(entry, "layout", LayoutKind::parse, LAYOUTS),
                "direction" => f.direction = self.keyword(entry, "direction", Direction::parse, DIRECTIONS),
                "x" => f.x = self.number(entry),
                "y" => f.y = self.number(entry),
                "columns" => f.columns = self.uint(entry),
                "type" => f.ty = self.string(entry),
                _ if self.style_prop(&mut f.style, key, entry) => {}
                other => {
                    let known: Vec<&str> = FRAME_PROPS.iter().chain(STYLE_PROPS).copied().collect();
                    self.unknown("frame property", other, entry.span(), &known);
                }
            }
        }
        for child in node.children().map(|c| c.nodes()).unwrap_or_default() {
            let name = child.name().value();
            if is_string_named(child) {
                f.nodes.extend(self.node(child, Some(name.to_string())));
                continue;
            }
            match name {
                "node" | "-" => f.nodes.extend(self.node(child, None)),
                "edge" => f.edges.extend(self.edge(child)),
                "props" => f.props = self.props(child),
                other => {
                    self.unknown("statement in frame", other, child.name().span(), &["node", "-", "edge", "props"])
                }
            }
        }
        Some(f)
    }

    fn node(&mut self, node: &KdlNode, label_from_name: Option<String>) -> Option<NodeDef> {
        let e = entries(node);
        let mut n = NodeDef { span: span(node.span()), ..Default::default() };
        let mut args = e.args.iter();
        let label = match label_from_name {
            Some(l) => Some(l),
            None => args.next().and_then(|a| self.string(a)),
        };
        for extra in args {
            self.err("a node takes one label", extra.span()).help =
                Some("quote labels with spaces: node \"Two words\"".into());
        }
        let mut label = label;
        for (key, entry) in e.props {
            match key {
                "id" => n.id = self.string(entry),
                "label" => label = self.string(entry),
                "class" => n.class = self.list(entry),
                "x" => n.x = self.number(entry),
                "y" => n.y = self.number(entry),
                "note" => n.note = self.string(entry),
                "link" => n.link = self.string(entry),
                "tags" => n.tags = self.list(entry),
                "type" => n.ty = self.string(entry),
                _ if self.style_prop(&mut n.style, key, entry) => {}
                other => {
                    let known: Vec<&str> = NODE_PROPS.iter().chain(STYLE_PROPS).copied().collect();
                    self.unknown("node property", other, entry.span(), &known);
                }
            }
        }
        for child in node.children().map(|c| c.nodes()).unwrap_or_default() {
            let name = child.name().value();
            if is_string_named(child) {
                n.children.extend(self.node(child, Some(name.to_string())));
                continue;
            }
            match name {
                "node" | "-" => n.children.extend(self.node(child, None)),
                "note" => {
                    let text = self.single_string(child);
                    n.note = match (n.note.take(), text) {
                        (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
                        (a, b) => a.or(b),
                    };
                }
                "props" => n.props = self.props(child),
                "port" => n.ports.extend(self.port(child)),
                "accept" => n.accept.extend(self.single_string(child)),
                "prov" => n.prov = self.prov(child),
                "edge" => {
                    self.err("edges go in a frame or at the top level, not inside a node", child.span());
                }
                other => self.unknown(
                    "statement in node",
                    other,
                    child.name().span(),
                    &["node", "-", "note", "props", "port", "accept", "prov"],
                ),
            }
        }
        match label {
            Some(l) => n.label = l,
            None => {
                self.err("a node needs a label", node.span()).help = Some("node \"My idea\"".into());
                return None;
            }
        }
        Some(n)
    }

    /// `edge a b c label="x"` is a path: a→b and b→c, all sharing the properties.
    fn edge(&mut self, node: &KdlNode) -> Vec<EdgeDef> {
        let e = entries(node);
        let ends: Vec<(String, Span)> =
            e.args.iter().filter_map(|a| self.string(a).map(|s| (s, span(a.span())))).collect();
        if ends.len() < 2 {
            self.err("an edge needs at least two nodes", node.span()).help = Some("edge start finish".into());
            return Vec::new();
        }
        let mut template = EdgeDef::default();
        for (key, entry) in e.props {
            match key {
                "label" => template.label = self.string(entry),
                "arrow" => {
                    template.arrow = self.keyword(entry, "arrow", Arrow::parse, &["none", "end", "start", "both"])
                }
                "class" => template.class = self.list(entry),
                "id" => template.id = self.string(entry),
                "type" => template.ty = self.string(entry),
                "kind" => {
                    template.kind =
                        self.keyword(entry, "edge kind", EdgeKind::parse, &["data", "control", "dependency"])
                }
                "from-port" => template.from_port = self.string(entry),
                "to-port" => template.to_port = self.string(entry),
                _ if self.style_prop(&mut template.style, key, entry) => {}
                other => {
                    let known: Vec<&str> = EDGE_PROPS.iter().chain(STYLE_PROPS).copied().collect();
                    self.unknown("edge property", other, entry.span(), &known);
                }
            }
        }
        for child in node.children().map(|c| c.nodes()).unwrap_or_default() {
            match child.name().value() {
                "props" => template.props = self.props(child),
                "prov" => template.prov = self.prov(child),
                other => self.unknown("statement in edge", other, child.name().span(), &["props", "prov"]),
            }
        }
        if template.id.is_some() && ends.len() > 2 {
            self.err("an edge path (`edge a b c`) cannot share one id", node.span()).help =
                Some("write one `edge` per id".into());
        }
        ends.windows(2)
            .map(|w| EdgeDef { from: w[0].0.clone(), to: w[1].0.clone(), span: span(node.span()), ..template.clone() })
            .collect()
    }
}

impl Parser {
    /// A `props` block: a JSON object in strict JSON-in-KDL (JiK 4.0.0) form.
    fn props(&mut self, node: &KdlNode) -> Props {
        match self.jik_object(node) {
            Some(map) => map.into_iter().collect(),
            None => Props::new(),
        }
    }

    fn jik_scalar(&mut self, e: &KdlEntry) -> Option<Value> {
        Some(match e.value() {
            KdlValue::String(s) => Value::String(s.clone()),
            KdlValue::Bool(b) => Value::Bool(*b),
            KdlValue::Null => Value::Null,
            KdlValue::Integer(i) => match (i64::try_from(*i), u64::try_from(*i)) {
                (Ok(v), _) => Value::from(v),
                (_, Ok(v)) => Value::from(v),
                _ => Value::from(*i as f64),
            },
            KdlValue::Float(f) => match serde_json::Number::from_f64(*f) {
                Some(n) => Value::Number(n),
                None => {
                    self.err("props cannot hold NaN or infinity", e.span());
                    return None;
                }
            },
        })
    }

    /// Properties and named children as object keys.
    fn jik_object(&mut self, node: &KdlNode) -> Option<serde_json::Map<String, Value>> {
        let e = entries(node);
        if !e.args.is_empty() {
            self.err("an object takes `key=value` properties and named children, not bare values", e.args[0].span());
        }
        let mut map = serde_json::Map::new();
        for (key, entry) in e.props {
            let v = self.jik_scalar(entry)?;
            if map.insert(key.to_string(), v).is_some() {
                self.err(format!("key `{key}` appears twice"), entry.span());
            }
        }
        for child in node.children().map(|c| c.nodes()).unwrap_or_default() {
            let key = child.name().value();
            if key == "-" {
                self.err("`-` items belong in an array, not an object", child.span());
                continue;
            }
            let v = self.jik_value(child)?;
            if map.insert(key.to_string(), v).is_some() {
                self.err(format!("key `{key}` appears twice"), child.span());
            }
        }
        Some(map)
    }

    /// The JiK value of a node: scalar, array or object by its shape.
    fn jik_value(&mut self, node: &KdlNode) -> Option<Value> {
        let annotation = node.ty().map(|t| t.value().to_string());
        let e = entries(node);
        let children = node.children().map(|c| c.nodes()).unwrap_or_default();
        let items = children.iter().filter(|c| c.name().value() == "-").count();
        let named = children.len() - items;
        let array = |p: &mut Parser| -> Option<Value> {
            if !e.props.is_empty() || named > 0 {
                p.err("an array cannot also have keys; use `-` items for objects in an array", node.span());
                return None;
            }
            let mut out = Vec::new();
            for a in &e.args {
                out.push(p.jik_scalar(a)?);
            }
            for c in children {
                out.push(p.jik_value(c)?);
            }
            Some(Value::Array(out))
        };
        match annotation.as_deref() {
            Some("array") => array(self),
            Some("object") => {
                if items > 0 {
                    self.err("an object cannot hold `-` items", node.span());
                    return None;
                }
                self.jik_object(node).map(Value::Object)
            }
            Some(other) => {
                self.err(format!("unknown annotation `({other})`"), node.span()).help =
                    Some("JiK knows `(array)` and `(object)`".into());
                None
            }
            None if !e.props.is_empty() || named > 0 => {
                if !e.args.is_empty() || items > 0 {
                    self.err("a value is either an object (keys) or an array (values), not both", node.span());
                    return None;
                }
                self.jik_object(node).map(Value::Object)
            }
            None if items > 0 || e.args.len() > 1 => array(self),
            None => match e.args.first() {
                Some(a) => self.jik_scalar(a),
                None => {
                    self.err("no value given", node.span()).help =
                        Some("write `#null`, or `(array)name` / `(object)name` for an empty array or object".into());
                    None
                }
            },
        }
    }

    /// `port out "events" kind=data schema=data:s3-event label="…"`
    fn port(&mut self, node: &KdlNode) -> Option<Port> {
        let e = entries(node);
        let dir = e.args.first().and_then(|a| self.keyword(a, "port direction", PortDir::parse, &["in", "out"]));
        let name = e.args.get(1).and_then(|a| self.string(a));
        let (Some(dir), Some(name)) = (dir, name) else {
            self.err("a port needs a direction and a name", node.span()).help =
                Some("port out \"events\" kind=data".into());
            return None;
        };
        let mut port = Port { name: PortName(name), dir, kind: None, schema: None, label: None };
        for (key, entry) in e.props {
            match key {
                "kind" => {
                    port.kind = self.keyword(entry, "edge kind", EdgeKind::parse, &["data", "control", "dependency"])
                }
                "schema" => port.schema = self.string(entry),
                "label" => port.label = self.string(entry),
                other => self.unknown("port property", other, entry.span(), &["kind", "schema", "label"]),
            }
        }
        self.no_children(node);
        Some(port)
    }

    /// `prov method=… by=… at=… confidence=… { source …; derived-from …; review … }`
    fn prov(&mut self, node: &KdlNode) -> Option<Prov> {
        let e = entries(node);
        let mut method = None;
        let mut prov = Prov::new(ProvMethod::Asserted);
        for (key, entry) in e.props {
            match key {
                "method" => {
                    method = self.keyword(
                        entry,
                        "provenance method",
                        ProvMethod::parse,
                        &["asserted", "imported", "extracted", "inferred", "computed"],
                    )
                }
                "by" => prov.by = self.string(entry),
                "at" => prov.at = self.string(entry),
                "confidence" => {
                    prov.confidence = match entry.value() {
                        KdlValue::String(s) => Some(Confidence::Scheme(s.clone())),
                        KdlValue::Float(f) => Some(Confidence::Number(*f)),
                        KdlValue::Integer(i) => Some(Confidence::Number(*i as f64)),
                        _ => {
                            self.err("confidence is a number or a scheme value like \"grade:moderate\"", entry.span());
                            None
                        }
                    }
                }
                other => self.unknown("prov property", other, entry.span(), &["method", "by", "at", "confidence"]),
            }
        }
        let Some(method) = method else {
            self.err("`prov` needs `method=`", node.span()).help =
                Some("one of asserted, imported, extracted, inferred, computed".into());
            return None;
        };
        prov.method = method;
        for child in node.children().map(|c| c.nodes()).unwrap_or_default() {
            let ce = entries(child);
            match child.name().value() {
                "source" => {
                    let Some(source) = ce.args.first().and_then(|a| self.string(a)) else {
                        self.err("`source` needs the id of a source node", child.span());
                        continue;
                    };
                    let mut c = Citation { source, locator: None, quote: None };
                    for (key, entry) in ce.props {
                        match key {
                            "locator" => c.locator = self.string(entry),
                            "quote" => c.quote = self.string(entry),
                            other => self.unknown("source property", other, entry.span(), &["locator", "quote"]),
                        }
                    }
                    prov.sources.push(c);
                }
                "derived-from" => prov.derived_from.extend(ce.args.iter().filter_map(|a| self.string(a))),
                "review" => {
                    let mut status = None;
                    let mut r = Review { status: ReviewStatus::Proposed, by: None, at: None, note: None };
                    for (key, entry) in ce.props {
                        match key {
                            "status" => {
                                status = self.keyword(
                                    entry,
                                    "review status",
                                    ReviewStatus::parse,
                                    &["proposed", "accepted", "rejected"],
                                )
                            }
                            "by" => r.by = self.string(entry),
                            "at" => r.at = self.string(entry),
                            "note" => r.note = self.string(entry),
                            other => {
                                self.unknown("review property", other, entry.span(), &["status", "by", "at", "note"])
                            }
                        }
                    }
                    match status {
                        Some(st) => {
                            r.status = st;
                            prov.review = Some(r);
                        }
                        None => {
                            self.err("`review` needs `status=`", child.span());
                        }
                    }
                }
                other => {
                    self.unknown("statement in prov", other, child.name().span(), &["source", "derived-from", "review"])
                }
            }
        }
        Some(prov)
    }
}

const LAYOUTS: &[&str] = &["auto", "mindmap", "tree", "layered", "grid", "free"];
const DIRECTIONS: &[&str] = &["right", "left", "down", "up"];

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> File {
        let (f, d) = parse(src);
        assert!(d.iter().all(|d| !d.is_error()), "{d:#?}");
        f
    }

    #[test]
    fn nested_nodes_and_shorthand() {
        let f = ok(r##"
            title "Plan"
            - "Root" {
                - "A" { - "A1"; - "A2" }
                "B" shape=diamond fill="#fde68a"
            }
        "##);
        assert_eq!(f.title.as_deref(), Some("Plan"));
        let root = &f.nodes[0];
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].children[1].label, "A2");
        assert_eq!(root.children[1].style.shape, Some(Shape::Diamond));
    }

    #[test]
    fn edge_paths_expand() {
        let f = ok(r##"edge a b c label="go" line=dashed"##);
        assert_eq!(f.edges.len(), 2);
        assert_eq!((f.edges[1].from.as_str(), f.edges[1].to.as_str()), ("b", "c"));
        assert_eq!(f.edges[1].style.line, Some(LineStyle::Dashed));
    }

    #[test]
    fn frames() {
        let f = ok(r##"
            frame "Flow" layout=flow direction=down columns=2 {
                node "Start" id=s
                edge s t
            }
        "##);
        let fr = &f.frames[0];
        assert_eq!(fr.layout, Some(LayoutKind::Layered));
        assert_eq!(fr.direction, Some(Direction::Down));
        assert_eq!(fr.nodes[0].id.as_deref(), Some("s"));
        assert_eq!(fr.edges.len(), 1);
    }

    #[test]
    fn helpful_errors() {
        let (_, d) = parse(r##"node "A" shap=diamond"##);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].help.as_deref(), Some("did you mean `shape`?"));

        let (_, d) = parse(r##"node "A" fill="#zzz""##);
        assert!(d[0].message.contains("invalid color"));

        let (_, d) = parse("nod \"A\"");
        assert_eq!(d[0].help.as_deref(), Some("did you mean `node`?"));

        let (_, d) = parse("node \"A\" {");
        assert!(d[0].is_error());
    }

    #[test]
    fn notes_as_children() {
        let f = ok(r##"- "A" { note "line one"; note "line two" }"##);
        assert_eq!(f.nodes[0].note.as_deref(), Some("line one\nline two"));
    }
}
