//! Typed-model building blocks (format version 2): types, props, ports, edge
//! kinds and provenance. Core only stores and structurally checks them; what
//! a type means is up to the kit that defines it.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{EdgeId, PortName, TypeName};

/// Typed properties: a JSON object in the author's key order.
pub type Props = IndexMap<String, serde_json::Value>;

/// A kit the document uses (`use infra version="0.1"`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct KitRef {
    pub kit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PortDir {
    In,
    Out,
}

impl PortDir {
    pub fn name(self) -> &'static str {
        match self {
            PortDir::In => "in",
            PortDir::Out => "out",
        }
    }

    pub fn parse(s: &str) -> Option<PortDir> {
        match s {
            "in" => Some(PortDir::In),
            "out" => Some(PortDir::Out),
            _ => None,
        }
    }
}

/// The semantic family of an edge. `None` on an edge means a plain drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    /// Values flow along the edge.
    Data,
    /// Ordering or triggering.
    Control,
    /// "Needs to exist first" / "is configured by". Nothing flows.
    Dependency,
}

impl EdgeKind {
    pub fn name(self) -> &'static str {
        match self {
            EdgeKind::Data => "data",
            EdgeKind::Control => "control",
            EdgeKind::Dependency => "dependency",
        }
    }

    pub fn parse(s: &str) -> Option<EdgeKind> {
        match s {
            "data" => Some(EdgeKind::Data),
            "control" => Some(EdgeKind::Control),
            "dependency" => Some(EdgeKind::Dependency),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Port {
    pub name: PortName,
    pub dir: PortDir,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<EdgeKind>,
    /// Reference to the schema of values passing through, e.g. `data:order`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProvMethod {
    /// A person wrote it.
    Asserted,
    /// From a vocabulary pack or an external graph.
    Imported,
    /// An agent read it from a source.
    Extracted,
    /// Derived from other statements.
    Inferred,
    /// Produced by an evaluator.
    Computed,
}

impl ProvMethod {
    pub fn name(self) -> &'static str {
        match self {
            ProvMethod::Asserted => "asserted",
            ProvMethod::Imported => "imported",
            ProvMethod::Extracted => "extracted",
            ProvMethod::Inferred => "inferred",
            ProvMethod::Computed => "computed",
        }
    }

    pub fn parse(s: &str) -> Option<ProvMethod> {
        Some(match s {
            "asserted" => ProvMethod::Asserted,
            "imported" => ProvMethod::Imported,
            "extracted" => ProvMethod::Extracted,
            "inferred" => ProvMethod::Inferred,
            "computed" => ProvMethod::Computed,
            _ => return None,
        })
    }
}

/// A pointer into a source node (a `kg.source`), with a pinpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Citation {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}

/// A bare number in [0, 1], or a named scheme value such as `grade:moderate`.
/// Core attaches no meaning to either.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Confidence {
    Number(f64),
    Scheme(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewStatus {
    Proposed,
    Accepted,
    Rejected,
}

impl ReviewStatus {
    pub fn name(self) -> &'static str {
        match self {
            ReviewStatus::Proposed => "proposed",
            ReviewStatus::Accepted => "accepted",
            ReviewStatus::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<ReviewStatus> {
        Some(match s {
            "proposed" => ReviewStatus::Proposed,
            "accepted" => ReviewStatus::Accepted,
            "rejected" => ReviewStatus::Rejected,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Review {
    pub status: ReviewStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Where a statement came from. Stored on nodes and edges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Prov {
    pub method: ProvMethod,
    /// `human:<id>`, `agent:<model>@<run>`, `rule:<id>` or `import:<graph>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// ISO 8601 date or date-time, written by the author, never from a clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Citation>,
    /// Node or edge ids this was derived from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived_from: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
}

impl Prov {
    pub fn new(method: ProvMethod) -> Prov {
        Prov {
            method,
            by: None,
            at: None,
            sources: Vec::new(),
            derived_from: Vec::new(),
            confidence: None,
            review: None,
        }
    }
}

/// What a typed node shows besides its label, worked out from its kit's
/// `look`: a badge, a subtitle, a state, and whether it is drawn as a card
/// with rows. For drawing only: kits fill [`crate::Document::decor`] before
/// layout, and it is never written to files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decor {
    /// A small tag next to the label: `POST`, `LAMBDA`, `ACCEPTED`.
    pub badge: Option<Badge>,
    /// A muted line under the label: `/orders`.
    pub subtitle: Option<String>,
    pub state: DecorState,
    /// Accent for a card's title band or a container's outline: a tone name
    /// or a color, empty for neutral.
    pub tone: String,
    /// Drawn as a card: a title band, then one row per child that has
    /// [`Decor::row`] set (an entity and its fields).
    pub card: bool,
    /// Drawn as a container: its children are drawn inside it (a VPC and
    /// its subnets) instead of as separate boxes joined by branches.
    pub container: bool,
    /// Set on a child drawn as a row of its parent's card.
    pub row: Option<DecorRow>,
}

/// A small colored tag. `tone` is a tone name (`blue`, `green`, …) or a
/// color; empty means neutral.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Badge {
    pub text: String,
    pub tone: String,
}

/// One row of a card: the label on the left, `detail` on the right.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecorRow {
    pub detail: Option<String>,
    /// Required: drawn bold with a marker.
    pub strong: bool,
    /// A tag after the label, such as `PII`.
    pub marker: Option<Badge>,
}

/// How settled a node is: a proposal draws dashed and muted, a rejected or
/// superseded one greyed out and struck through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecorState {
    #[default]
    Normal,
    Draft,
    Void,
}

impl DecorState {
    pub fn name(self) -> &'static str {
        match self {
            DecorState::Normal => "normal",
            DecorState::Draft => "draft",
            DecorState::Void => "void",
        }
    }

    pub fn parse(s: &str) -> Option<DecorState> {
        Some(match s {
            "normal" => DecorState::Normal,
            "draft" => DecorState::Draft,
            "void" => DecorState::Void,
            _ => return None,
        })
    }
}

/// `aws.s3.bucket`: two or more dot-separated `[a-z][a-z0-9-]*` segments.
pub fn is_dotted_type(s: &str) -> bool {
    let mut n = 0;
    for seg in s.split('.') {
        let mut chars = seg.chars();
        let ok = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return false;
        }
        n += 1;
    }
    n >= 2
}

/// The prefix of a CURIE (`HP:0001649` → `HP`), or `None` for plain ids and
/// absolute IRIs with a well-known scheme.
pub fn curie_prefix(id: &str) -> Option<&str> {
    let (prefix, local) = id.split_once(':')?;
    if matches!(prefix, "http" | "https" | "urn" | "mailto" | "file") || local.starts_with("//") {
        return None;
    }
    let mut chars = prefix.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    valid.then_some(prefix)
}

/// `YYYY-MM-DD`, optionally followed by `T` and a time. Checks shape and
/// ranges, not calendars (Feb 30 passes).
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r.clone()).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    if b.len() < 10 || !digits(0..4) || b[4] != b'-' || !digits(5..7) || b[7] != b'-' || !digits(8..10) {
        return false;
    }
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    match b.get(10) {
        None => true,
        Some(b'T') => {
            b.len() >= 16
                && digits(11..13)
                && b[13] == b':'
                && digits(14..16)
                && s[11..13].parse::<u32>().unwrap_or(99) < 24
        }
        _ => false,
    }
}

/// Edge ids are optional; this helper gives every edge a stable display name.
pub fn edge_label(id: Option<&EdgeId>, from: &str, to: &str) -> String {
    match id {
        Some(id) => format!("`{id}`"),
        None => format!("`{from}` → `{to}`"),
    }
}

/// Type names are either dotted (kit-defined) or CURIEs (from an ontology).
pub fn type_name_ok(ty: &TypeName, prefixes: &IndexMap<String, String>) -> bool {
    is_dotted_type(ty.as_str()) || curie_prefix(ty.as_str()).is_some_and(|p| prefixes.contains_key(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names() {
        assert!(is_dotted_type("aws.s3.bucket"));
        assert!(is_dotted_type("flow.http-request"));
        assert!(!is_dotted_type("aws"));
        assert!(!is_dotted_type("AWS.s3"));
        assert!(!is_dotted_type("aws..s3"));
        assert!(!is_dotted_type("aws.3d"));
    }

    #[test]
    fn curies() {
        assert_eq!(curie_prefix("HP:0001649"), Some("HP"));
        assert_eq!(curie_prefix("schema:Person"), Some("schema"));
        assert_eq!(curie_prefix("https://example.org/x"), None);
        assert_eq!(curie_prefix("plain-id"), None);
        assert_eq!(curie_prefix("9x:1"), None);
    }

    #[test]
    fn dates() {
        assert!(is_iso_date("2026-10-19"));
        assert!(is_iso_date("2026-10-19T08:30"));
        assert!(is_iso_date("2026-10-19T08:30:00Z"));
        assert!(!is_iso_date("2026-13-01"));
        assert!(!is_iso_date("19.10.2026"));
        assert!(!is_iso_date("2026-10-19 08:30"));
    }

    #[test]
    fn confidence_serde() {
        let n: Confidence = serde_json::from_str("0.8").unwrap();
        assert_eq!(n, Confidence::Number(0.8));
        let s: Confidence = serde_json::from_str("\"grade:moderate\"").unwrap();
        assert_eq!(s, Confidence::Scheme("grade:moderate".into()));
    }
}
