//! The rhizo text format and its JSON/YAML twins.
//!
//! All three share one schema ([`ast::File`]): KDL for writing by hand, JSON
//! for tools, YAML for config-style authoring. [`load`] picks by [`Syntax`]
//! and returns a resolved [`rhizo_core::Document`] plus diagnostics.

pub mod ast;
pub mod diag;
pub mod kdl;
pub mod lower;
pub mod write;

pub use diag::{Diagnostic, Report, Severity};
pub use lower::{lower, raise};

use rhizo_core::Document;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Kdl,
    Json,
    Yaml,
}

/// The outcome of loading a document. `document` is usable (best effort) even
/// when `diagnostics` contains errors.
#[derive(Debug)]
pub struct Loaded {
    pub document: Document,
    pub diagnostics: Vec<Diagnostic>,
}

impl Loaded {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// The document, or every diagnostic if any of them is an error.
    pub fn into_result(self) -> Result<(Document, Vec<Diagnostic>), Vec<Diagnostic>> {
        if self.has_errors() { Err(self.diagnostics) } else { Ok((self.document, self.diagnostics)) }
    }
}

pub fn parse(src: &str, syntax: Syntax) -> (ast::File, Vec<Diagnostic>) {
    match syntax {
        Syntax::Kdl => kdl::parse(src),
        Syntax::Json => match serde_json::from_str(src) {
            Ok(f) => (f, Vec::new()),
            Err(e) => {
                let offset = offset_of(src, e.line(), e.column());
                (ast::File::default(), vec![Diagnostic::error(strip_position(&e.to_string()), Some((offset, 0)))])
            }
        },
        Syntax::Yaml => match serde_yaml_ng::from_str(src) {
            Ok(f) => (f, Vec::new()),
            Err(e) => {
                let span = e.location().map(|l| (l.index(), 0));
                (ast::File::default(), vec![Diagnostic::error(strip_position(&e.to_string()), span)])
            }
        },
    }
}

pub fn load(src: &str, syntax: Syntax) -> Loaded {
    let (file, mut diagnostics) = parse(src, syntax);
    if diagnostics.iter().any(Diagnostic::is_error) {
        return Loaded { document: Document::default(), diagnostics };
    }
    let (document, more) = lower(&file);
    diagnostics.extend(more);
    Loaded { document, diagnostics }
}

pub fn to_kdl(doc: &Document) -> String {
    write::write(&raise(doc))
}

pub fn to_json(doc: &Document) -> String {
    serde_json::to_string_pretty(&raise(doc)).expect("the AST always serializes") + "\n"
}

pub fn to_yaml(doc: &Document) -> String {
    serde_yaml_ng::to_string(&raise(doc)).expect("the AST always serializes")
}

/// JSON Schema for the JSON and YAML forms.
pub fn json_schema() -> serde_json::Value {
    let mut schema = schemars::schema_for!(ast::File);
    schema.insert("$id".into(), "https://rhizo.dev/schema/v1.json".into());
    serde_json::to_value(schema).expect("schemas serialize")
}

fn offset_of(src: &str, line: usize, col: usize) -> usize {
    let start: usize = src.split_inclusive('\n').take(line.saturating_sub(1)).map(str::len).sum();
    (start + col.saturating_sub(1)).min(src.len())
}

/// serde appends " at line X column Y"; the span already says that.
fn strip_position(msg: &str) -> String {
    match msg.rfind(" at line ") {
        Some(i) => msg[..i].to_string(),
        None => msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const SAMPLE: &str = r##"title "Launch"
style "risk" fill="#fee2e2" stroke="#ef4444"

- "Launch" {
    - "Users" { - "Personas"; - "Jobs" }
    - "Pricing" class=risk note="Needs a decision"
    - "Task"
    - "Task"
}

frame "Release" layout=layered direction=down {
    node "Build" shape=pill
    node "Tests pass?" id=check shape=diamond
    edge build check "Ship"
    edge check "Build" label=no line=dashed
}

edge pricing "Tests pass?" arrow=both
"##;

    #[test]
    fn kdl_lowers_and_resolves_refs() {
        let loaded = load(SAMPLE, Syntax::Kdl);
        assert!(!loaded.has_errors(), "{:#?}", loaded.diagnostics);
        let d = loaded.document;
        assert_eq!(d.frames.len(), 2);
        assert!(d.nodes.contains_key(&rhizo_core::NodeId::from("task-2")));
        assert_eq!(d.edges.len(), 4);
        assert_eq!(d.edges[1].to.as_str(), "ship");
        assert_eq!(d.edges[3].from.as_str(), "pricing");
        assert_eq!(d.edges[3].to.as_str(), "check");
    }

    #[test]
    fn kdl_roundtrip_is_stable() {
        let doc = load(SAMPLE, Syntax::Kdl).document;
        let once = to_kdl(&doc);
        let doc2 = load(&once, Syntax::Kdl).into_result().unwrap().0;
        assert_eq!(doc2, doc);
        assert_eq!(to_kdl(&doc2), once);
    }

    #[test]
    fn json_and_yaml_roundtrip() {
        let doc = load(SAMPLE, Syntax::Kdl).document;
        let json = to_json(&doc);
        assert_eq!(load(&json, Syntax::Json).into_result().unwrap().0, doc);
        let yaml = to_yaml(&doc);
        assert_eq!(load(&yaml, Syntax::Yaml).into_result().unwrap().0, doc);
    }

    #[test]
    fn yaml_shorthand() {
        let src = "title: Plan\nnodes:\n  - label: Root\n    children: [A, B]\nedges:\n  - a -> b\n  - from: b\n    to: root\n    label: back\n";
        let doc = load(src, Syntax::Yaml).into_result().unwrap().0;
        assert_eq!(doc.nodes.len(), 3);
        assert_eq!(doc.edges[0].from.as_str(), "a");
        assert_eq!(doc.edges[1].label.as_deref(), Some("back"));
    }

    #[test]
    fn json_errors_have_positions() {
        let loaded = load("{\"nodes\": [{\"label\": \"A\", \"shape\": \"blob\"}]}", Syntax::Json);
        assert!(loaded.has_errors());
        assert!(loaded.diagnostics[0].span.is_some());
    }

    #[test]
    fn unknown_ref_suggests() {
        let loaded = load("- \"Alpha\"\n- \"Beta\"\nedge alpha btea", Syntax::Kdl);
        assert!(!loaded.has_errors());
        assert_eq!(loaded.document.nodes.len(), 3);
        let d = &loaded.diagnostics[0];
        assert_eq!(d.severity, Severity::Warning);
        assert!(d.help.as_deref().unwrap().starts_with("did you mean `beta`?"), "{d:?}");
    }

    #[test]
    fn edges_create_new_nodes() {
        let src =
            "frame \"Flow\" layout=layered {\n    edge \"Start\" \"Check stock\" \"Ship\"\n}\nedge ship \"Notify\"";
        let doc = load(src, Syntax::Kdl).into_result().unwrap().0;
        assert_eq!(doc.nodes.len(), 4);
        assert!(doc.nodes.values().all(|n| n.frame.as_str() == "flow"));
        assert_eq!(doc.edges.len(), 3);
    }

    #[test]
    fn short_names_are_new_nodes_not_typos() {
        let loaded = load("- \"A\"\nedge a b c", Syntax::Kdl);
        assert!(!loaded.has_errors(), "{:#?}", loaded.diagnostics);
        assert_eq!(loaded.document.nodes.len(), 3);
    }

    #[test]
    fn duplicate_ids() {
        let loaded = load("- \"A\" id=x\n- \"B\" id=x", Syntax::Kdl);
        assert!(loaded.diagnostics[0].message.contains("more than once"));
    }

    #[test]
    fn schema_mentions_shorthand() {
        let s = json_schema().to_string();
        assert!(s.contains("Shorthand form"));
        assert!(s.contains("stroke-width"));
    }
}
