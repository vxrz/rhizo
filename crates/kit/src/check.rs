//! Validation layer 2 (schema): kits used, types known, props against their
//! schema, containment, ports and edge kinds. Then the [`Rules`] of the
//! kits the document uses.

use std::collections::HashSet;

use rhizo_core::{Document, EdgeKind, FrameId, Node, NodeId, PortDir, Props};
use rhizo_lang::Diagnostic;
use rhizo_lang::diag::suggest;

use crate::{Kit, Kits, TypeDef, kind_name};

/// What a [`Finding`] is about, so callers can attach a source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    Document,
    Frame(FrameId),
    Node(NodeId),
    /// Index into `Document::edges`.
    Edge(usize),
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub subject: Subject,
    /// Without a span; the caller knows where `subject` was written.
    pub diagnostic: Diagnostic,
}

/// Gives findings the source span of the statement they are about, when
/// `src` is in one of the rhizo text forms (`syntax`).
pub fn locate(src: &str, syntax: Option<rhizo_lang::Syntax>, findings: Vec<Finding>) -> Vec<Diagnostic> {
    if findings.is_empty() {
        return Vec::new();
    }
    let sources = syntax.map(|s| rhizo_lang::lower::lower_with_sources(&rhizo_lang::parse(src, s).0));
    findings
        .into_iter()
        .map(|f| {
            let mut d = f.diagnostic;
            d.span = sources.as_ref().and_then(|l| match &f.subject {
                Subject::Node(id) => l.node_spans.get(id).copied().flatten(),
                Subject::Edge(i) => l.edge_spans.get(*i).copied().flatten(),
                Subject::Frame(_) | Subject::Document => None,
            });
            d
        })
        .collect()
}

/// Checks `doc` against the kits it uses. Types from kits the document does
/// not use, or that rhizo does not have, are left alone.
pub fn check(doc: &Document, kits: &Kits) -> Vec<Finding> {
    let mut cx = Cx { doc, out: Vec::new(), used: Vec::new() };
    cx.uses(kits);
    let mut nagged = HashSet::new();
    for f in &doc.frames {
        let Some(ty) = &f.ty else { continue };
        let at = || Subject::Frame(f.id.clone());
        if let Some(def) = cx.resolve(kits, ty.as_str(), at(), &mut nagged) {
            if !def.frame {
                cx.error(at(), format!("`{ty}` is a node type, not a frame type"), None);
            }
            cx.props(at(), &format!("frame `{}`", f.id.as_str()), def, &f.props);
        }
    }
    for n in doc.nodes.values() {
        let Some(ty) = &n.ty else { continue };
        let at = || Subject::Node(n.id.clone());
        if let Some(def) = cx.resolve(kits, ty.as_str(), at(), &mut nagged) {
            if def.frame {
                cx.error(at(), format!("`{ty}` is a frame type"), Some(format!("frame \"…\" type={ty} {{ … }}")));
            }
            cx.props(at(), &format!("{} `{}`", def.label.to_lowercase(), n.id.as_str()), def, &n.props);
            cx.containment(n, def);
        }
    }
    cx.edges();
    let used = kits.used_by(doc);
    for r in RULES.iter().filter(|r| used.iter().any(|k| k.name == r.kit())) {
        r.check(doc, &mut cx.out);
    }
    cx.out
}

/// What a kit checks beyond its schemas, ports and containment: the rules
/// that need the whole graph (a workflow has one trigger, a database is in
/// a VPC). [`check`] runs the rules of every kit a document uses.
pub trait Rules: Sync {
    /// The kit whose rules these are.
    fn kit(&self) -> &'static str;
    fn check(&self, doc: &Document, out: &mut Vec<Finding>);
}

/// The rules of the built-in kits, in the order they run (and report).
pub static RULES: &[&dyn Rules] = &[&data::DataRules, &flow::FlowRules, &crate::infra::InfraRules];

/// Findings of one kit's rules.
struct Rx<'a> {
    doc: &'a Document,
    out: &'a mut Vec<Finding>,
}

impl Rx<'_> {
    fn error(&mut self, subject: Subject, msg: String, help: Option<String>) {
        let mut d = Diagnostic::error(msg, None);
        d.help = help;
        self.out.push(Finding { subject, diagnostic: d });
    }

    fn warning(&mut self, subject: Subject, msg: String, help: Option<String>) {
        let mut d = Diagnostic::warning(msg, None);
        d.help = help;
        self.out.push(Finding { subject, diagnostic: d });
    }
}

struct Cx<'a> {
    doc: &'a Document,
    out: Vec<Finding>,
    used: Vec<&'a Kit>,
}

impl<'a> Cx<'a> {
    fn push(&mut self, subject: Subject, diagnostic: Diagnostic) {
        self.out.push(Finding { subject, diagnostic });
    }

    fn error(&mut self, subject: Subject, msg: String, help: Option<String>) {
        let mut d = Diagnostic::error(msg, None);
        d.help = help;
        self.push(subject, d);
    }

    fn warning(&mut self, subject: Subject, msg: String, help: Option<String>) {
        let mut d = Diagnostic::warning(msg, None);
        d.help = help;
        self.push(subject, d);
    }

    /// `use` statements: known kit, version requirement met.
    fn uses(&mut self, kits: &'a Kits) {
        let available = || kits.iter().map(|k| k.name.as_str()).collect::<Vec<_>>().join(", ");
        for r in &self.doc.kits {
            let Some(kit) = kits.get(&r.kit) else {
                let help = match suggest(&r.kit, kits.iter().map(|k| k.name.as_str())) {
                    Some(close) => format!("did you mean `{close}`?"),
                    None => format!("available kits: {}", available()),
                };
                self.warning(
                    Subject::Document,
                    format!("kit `{}` is not available, so its types are not checked", r.kit),
                    Some(help),
                );
                continue;
            };
            if let Some(req) = &r.version {
                match semver::VersionReq::parse(req) {
                    Ok(v) if !v.matches(&kit.version) => self.error(
                        Subject::Document,
                        format!("this document needs {} {req}, but rhizo has {} {}", kit.name, kit.name, kit.version),
                        Some(format!("use {} version=\"{}.{}\"", kit.name, kit.version.major, kit.version.minor)),
                    ),
                    Ok(_) => {}
                    Err(e) => {
                        self.error(Subject::Document, format!("version `{req}` of kit `{}`: {e}", kit.name), None)
                    }
                }
            }
            self.used.push(kit);
        }
        if let Some(k) = kits.get(crate::IMPLICIT).filter(|k| !self.used.iter().any(|u| u.name == k.name)) {
            self.used.push(k);
        }
    }

    /// The definition of `ty` when a used kit owns it. Reports unknown types
    /// in a used kit and types from an available kit the document forgot to
    /// `use` (once per kit).
    fn resolve(&mut self, kits: &'a Kits, ty: &str, at: Subject, nagged: &mut HashSet<String>) -> Option<&'a TypeDef> {
        let (kit, def) = kits.owner(ty)?;
        if !self.used.iter().any(|k| std::ptr::eq(*k, kit)) {
            if nagged.insert(kit.name.clone()) {
                self.error(
                    at,
                    format!("`{ty}` comes from the `{}` kit, which this document does not use", kit.name),
                    Some(format!("add: use {} version=\"{}.{}\"", kit.name, kit.version.major, kit.version.minor)),
                );
            }
            return None;
        }
        if def.is_none() {
            let help = match suggest(ty, kit.types.keys().map(String::as_str)) {
                Some(close) => format!("did you mean `{close}`?"),
                None => format!("`rhizo types {}` lists them", kit.name),
            };
            self.error(at, format!("the `{}` kit has no type `{ty}`", kit.name), Some(help));
        }
        def
    }

    fn props(&mut self, at: Subject, what: &str, def: &TypeDef, props: &Props) {
        let instance = serde_json::Value::Object(props.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        // Unknown keys first, with a suggestion; the validator would only say "unexpected".
        let known = def.prop_names();
        let closed = def.closed();
        if closed {
            for key in props.keys().filter(|k| !known.contains(&k.as_str())) {
                let help = match suggest(key, known.iter().copied()) {
                    Some(close) => format!("did you mean `{close}`?"),
                    None if known.is_empty() => format!("{} takes no props", def.name),
                    None => format!("{} takes: {}", def.name, known.join(", ")),
                };
                self.error(at.clone(), format!("{what}: unknown prop `{key}`"), Some(help));
            }
        }
        let errors: Vec<String> = def
            .validator
            .iter_errors(&instance)
            .filter(|e| {
                !(closed
                    && e.instance_path().as_str().is_empty()
                    && matches!(e.kind(), jsonschema::error::ValidationErrorKind::AdditionalProperties { .. }))
            })
            .map(|e| {
                let path = e.instance_path().as_str().trim_start_matches('/').replace('/', ".");
                // The validator shortens long enums to "or N other candidates"; list them all.
                let msg = match e.kind() {
                    jsonschema::error::ValidationErrorKind::Enum { options } => {
                        let all: Vec<String> =
                            options.as_array().into_iter().flatten().map(|o| o.to_string()).collect();
                        format!("{} is not one of {}", e.instance(), or_list(&all))
                    }
                    _ => e.to_string(),
                };
                if path.is_empty() { format!("{what}: {msg}") } else { format!("{what}, prop `{path}`: {msg}") }
            })
            .collect();
        for msg in errors {
            self.error(at.clone(), msg, None);
        }
    }

    /// `parents` names the types allowed as the nearest typed ancestor: a
    /// parent node, or else the frame.
    fn containment(&mut self, n: &Node, def: &TypeDef) {
        let container = crate::container_of(self.doc, &n.id);
        if def.parents.is_empty() || def.fits(container) {
            return;
        }
        let allowed = def.parents.iter().map(|p| format!("a {p}")).collect::<Vec<_>>().join(" or ");
        let found = container.map_or("not inside a typed node or frame".to_string(), |c| format!("inside a {c}"));
        self.error(
            Subject::Node(n.id.clone()),
            format!("{} `{}` belongs inside {allowed}, but is {found}", def.label.to_lowercase(), n.id.as_str()),
            None,
        );
    }

    fn def_of(&self, id: &NodeId) -> Option<(&'a Node, &'a TypeDef)> {
        let n = self.doc.nodes.get(id)?;
        let ty = n.ty.as_ref()?;
        let def = self.used.iter().find_map(|k| k.types.get(ty.as_str()))?;
        Some((n, def))
    }

    /// Ports named on edges exist and face the right way; edge kinds match
    /// the kind of the port they use.
    fn edges(&mut self) {
        for (i, e) in self.doc.edges.iter().enumerate() {
            let mut kind_reported = false;
            for (end, port, dir) in [(&e.from, &e.from_port, PortDir::Out), (&e.to, &e.to_port, PortDir::In)] {
                // Nothing flows along a dependency, so it uses no port unless it names one.
                if e.kind == Some(EdgeKind::Dependency) && port.is_none() {
                    continue;
                }
                let Some((n, def)) = self.def_of(end) else { continue };
                // Ports written on the node itself refine or add to the kit's.
                let local = port.as_ref().and_then(|p| n.ports.iter().find(|q| &q.name == p));
                let kit_port = def.port(port.as_ref().map(|p| p.as_str()), dir);
                let (kind, actual_dir) = match (local, kit_port) {
                    (Some(p), _) => (p.kind, p.dir),
                    (None, Some(p)) => (p.port.kind, p.port.dir),
                    (None, None) => {
                        if let Some(p) = port {
                            let names: Vec<&str> = def
                                .ports
                                .iter()
                                .map(|q| q.port.name.as_str())
                                .chain(n.ports.iter().map(|q| q.name.as_str()))
                                .collect();
                            let help = match suggest(p.as_str(), names.iter().copied()) {
                                Some(close) => format!("did you mean `{close}`?"),
                                None if names.is_empty() => format!("{} has no ports", def.name),
                                None => format!("ports of {}: {}", def.name, names.join(", ")),
                            };
                            self.error(Subject::Edge(i), format!("`{}` has no port `{p}`", end.as_str()), Some(help));
                        }
                        continue;
                    }
                };
                if actual_dir != dir {
                    let p = port.as_ref().map_or("", |p| p.as_str());
                    let (want, side) = if dir == PortDir::Out { ("out", "start") } else { ("in", "end") };
                    self.error(
                        Subject::Edge(i),
                        format!(
                            "port `{p}` of `{}` is an {} port; an edge {side}s at an {want} port",
                            end.as_str(),
                            actual_dir.name()
                        ),
                        None,
                    );
                }
                if let (Some(want), Some(have)) = (kind, e.kind)
                    && want != have
                    && !kind_reported
                {
                    kind_reported = true;
                    self.error(
                        Subject::Edge(i),
                        format!(
                            "a {} edge cannot use the {} port of `{}`",
                            kind_name(Some(have)),
                            kind_name(Some(want)),
                            end.as_str()
                        ),
                        Some(format!("use kind={}", want.name())),
                    );
                }
            }
        }
    }
}

/// `a`, `a or b`, `a, b or c`.
fn or_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

mod data;
mod flow;

#[cfg(test)]
mod tests;
