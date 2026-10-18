//! The authoring form of a document: nested, with ids optional and edges
//! referring to nodes by id or label. KDL, JSON and YAML all deserialize to
//! these types, and [`crate::lower`] resolves them into a [`rhizo_core::Document`].

use std::fmt;

use indexmap::IndexMap;
use rhizo_core::{Arrow, Direction, LayoutKind, Style};
use schemars::JsonSchema;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Byte offset and length in the source, when the file came from text with
/// positions (KDL). Never serialized.
pub type Span = Option<(usize, usize)>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
#[schemars(title = "rhizo document")]
pub struct File {
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
    #[serde(flatten)]
    pub style: Style,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<EdgeDef>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NodeDef>,
    #[serde(skip)]
    #[schemars(skip)]
    pub span: Span,
}

impl NodeDef {
    pub fn new(label: impl Into<String>) -> Self {
        NodeDef { label: label.into(), ..Default::default() }
    }

    fn is_label_only(&self) -> bool {
        let NodeDef { label: _, id, class, style, x, y, note, link, tags, children, span: _ } = self;
        id.is_none()
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
    #[serde(skip)]
    #[schemars(skip)]
    pub span: Span,
}

impl EdgeDef {
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
        let EdgeDef { from, to, label, arrow, class, style, span: _ } = self;
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
