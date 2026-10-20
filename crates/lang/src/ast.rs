//! The authoring form of a document: nested, with ids optional and edges
//! referring to nodes by id or label. KDL, JSON and YAML all deserialize to
//! these types, and [`crate::lower`] resolves them into a [`rhizo_core::Document`].

use std::fmt;

use indexmap::IndexMap;
use rhizo_core::{Arrow, Direction, EdgeKind, KitRef, LayoutKind, Port, Props, Prov, Style};
use schemars::JsonSchema;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Byte offset and length in the source, when the file came from text with
/// positions (KDL). Never serialized.
pub type Span = Option<(usize, usize)>;

/// Keys an object had that the schema does not know. Collected instead of
/// silently dropped, and reported by [`crate::parse`].
pub type Extra = IndexMap<String, serde_json::Value>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
#[schemars(title = "rhizo document")]
pub struct File {
    /// Format version. Required (`2`) when typed features are used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rhizo: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `light`, `dark` or `paper`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Layout of the nodes written outside any frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// Kits the document uses.
    #[serde(rename = "use", default, skip_serializing_if = "Vec::is_empty")]
    pub kits: Vec<KitRef>,
    /// Base IRI for local ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// CURIE prefixes, e.g. `HP: http://purl.obolibrary.org/obo/HP_`.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub prefixes: IndexMap<String, String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub styles: IndexMap<String, Style>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<FrameDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<EdgeDef>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct FrameDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u32>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub props: Props,
    #[serde(flatten)]
    pub style: Style,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<EdgeDef>,
    #[serde(flatten, skip_serializing)]
    #[schemars(skip)]
    pub extra: Extra,
    #[serde(skip)]
    #[schemars(skip)]
    pub span: Span,
}

/// A node. In JSON and YAML a bare string is shorthand for `{label: ...}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(remote = "Self", rename_all = "kebab-case")]
#[schemars(transform = string_shorthand)]
pub struct NodeDef {
    pub label: String,
    /// Defaults to a slug of the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub class: Vec<String>,
    #[serde(flatten)]
    pub style: Style,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Namespaced type, e.g. `aws.s3.bucket`. Needs `"rhizo": 2`.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub props: Props,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
    /// Acceptance criteria.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accept: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prov: Option<Prov>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NodeDef>,
    #[serde(flatten, skip_serializing)]
    #[schemars(skip)]
    pub extra: Extra,
    #[serde(skip)]
    #[schemars(skip)]
    pub span: Span,
}

impl NodeDef {
    pub fn new(label: impl Into<String>) -> Self {
        NodeDef { label: label.into(), ..Default::default() }
    }

    pub fn is_typed(&self) -> bool {
        self.ty.is_some()
            || !self.props.is_empty()
            || !self.ports.is_empty()
            || !self.accept.is_empty()
            || self.prov.is_some()
    }

    fn is_label_only(&self) -> bool {
        let NodeDef {
            label: _,
            id,
            class,
            style,
            x,
            y,
            note,
            link,
            tags,
            ty,
            props,
            ports,
            accept,
            prov,
            children,
            extra: _,
            span: _,
        } = self;
        ty.is_none()
            && props.is_empty()
            && ports.is_empty()
            && accept.is_empty()
            && prov.is_none()
            && id.is_none()
            && class.is_empty()
            && style.is_empty()
            && x.is_none()
            && y.is_none()
            && note.is_none()
            && link.is_none()
            && tags.is_empty()
            && children.is_empty()
    }
}

/// An edge. In JSON and YAML, `"a -> b"` is shorthand (`--` for no arrow,
/// `<->` for both ends, `<-` for the start).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(remote = "Self", rename_all = "kebab-case")]
#[schemars(transform = string_shorthand)]
pub struct EdgeDef {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow: Option<Arrow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub class: Vec<String>,
    #[serde(flatten)]
    pub style: Style,
    /// Optional id, unique across nodes and edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The predicate, e.g. `skos:broader`.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<EdgeKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_port: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_port: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub props: Props,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prov: Option<Prov>,
    #[serde(flatten, skip_serializing)]
    #[schemars(skip)]
    pub extra: Extra,
    #[serde(skip)]
    #[schemars(skip)]
    pub span: Span,
}

impl EdgeDef {
    pub fn is_typed(&self) -> bool {
        self.id.is_some()
            || self.ty.is_some()
            || self.kind.is_some()
            || self.from_port.is_some()
            || self.to_port.is_some()
            || !self.props.is_empty()
            || self.prov.is_some()
    }

    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        EdgeDef { from: from.into(), to: to.into(), ..Default::default() }
    }

    /// Parses `a -> b`, `a -- b`, `a <-> b` or `a <- b`. The spaces around
    /// the operator are required, so ids may contain dashes.
    pub fn parse_shorthand(s: &str) -> Option<EdgeDef> {
        for (op, arrow) in [("<->", Arrow::Both), ("->", Arrow::End), ("<-", Arrow::Start), ("--", Arrow::None)] {
            if let Some((a, b)) = s.split_once(&format!(" {op} ")) {
                let (a, b) = (a.trim(), b.trim());
                if a.is_empty() || b.is_empty() {
                    return None;
                }
                let mut e = EdgeDef::new(a, b);
                if arrow != Arrow::End {
                    e.arrow = Some(arrow);
                }
                return Some(e);
            }
        }
        None
    }

    fn shorthand(&self) -> Option<String> {
        if self.is_typed() {
            return None;
        }
        let EdgeDef { from, to, label, arrow, class, style, .. } = self;
        if label.is_some()
            || !class.is_empty()
            || !style.is_empty()
            || from.contains(char::is_whitespace)
            || to.contains(char::is_whitespace)
        {
            return None;
        }
        let op = match arrow.unwrap_or_default() {
            Arrow::End => "->",
            Arrow::None => "--",
            Arrow::Both => "<->",
            Arrow::Start => "<-",
        };
        Some(format!("{from} {op} {to}"))
    }
}

fn string_shorthand(schema: &mut schemars::Schema) {
    let object = std::mem::replace(schema, schemars::Schema::from(true));
    *schema = schemars::json_schema!({
        "anyOf": [{ "type": "string", "description": "Shorthand form" }, object]
    });
}

impl Serialize for NodeDef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.is_label_only() { s.serialize_str(&self.label) } else { NodeDef::serialize(self, s) }
    }
}

impl<'de> Deserialize<'de> for NodeDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = NodeDef;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a node label or a node object")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<NodeDef, E> {
                Ok(NodeDef::new(v))
            }
            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<NodeDef, M::Error> {
                NodeDef::deserialize(de::value::MapAccessDeserializer::new(map))
            }
        }
        d.deserialize_any(V)
    }
}

impl Serialize for EdgeDef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.shorthand() {
            Some(short) => s.serialize_str(&short),
            None => EdgeDef::serialize(self, s),
        }
    }
}

impl<'de> Deserialize<'de> for EdgeDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = EdgeDef;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an edge like \"a -> b\" or an edge object")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<EdgeDef, E> {
                EdgeDef::parse_shorthand(v)
                    .ok_or_else(|| E::custom(format!("invalid edge `{v}`, expected `a -> b`, `a -- b` or `a <-> b`")))
            }
            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<EdgeDef, M::Error> {
                EdgeDef::deserialize(de::value::MapAccessDeserializer::new(map))
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_keys_are_captured_not_dropped() {
        let n: NodeDef =
            serde_json::from_str(r##"{"label":"A","fill":"#fff","colour":"red","type":"aws.s3.bucket"}"##).unwrap();
        assert_eq!(n.ty.as_deref(), Some("aws.s3.bucket"));
        assert!(n.style.fill.is_some());
        assert_eq!(n.extra.keys().collect::<Vec<_>>(), vec!["colour"]);
        let e: EdgeDef = serde_json::from_str(r#"{"from":"a","to":"b","lable":"x"}"#).unwrap();
        assert_eq!(e.extra.keys().collect::<Vec<_>>(), vec!["lable"]);
    }
}
