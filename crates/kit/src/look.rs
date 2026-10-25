//! How a kit wants its types drawn: the `look` block of a type in `kit.kdl`.
//!
//! ```kdl
//! type data.operation {
//!     look {
//!         badge "{method}" get=green post=blue delete=red
//!         subtitle "{path}"
//!     }
//! }
//! type data.entity {
//!     look {
//!         tone blue
//!         rows data.field {
//!             detail "{format|type}"
//!             strong "{required}"
//!             marker "{pii}" personal=amber sensitive=red text=PII
//!         }
//!     }
//! }
//! ```
//!
//! Templates take `{prop}` parts, with `{a|b}` meaning `a`, else `b`. Props
//! fall back to their schema `default`. A template with any part left empty
//! shows nothing, so `"{method} {path}"` never draws half.
//!
//! Tags (`badge`, `marker`) and `state` map the rendered value to a tone or a
//! state: `post=blue`. With a map, unmapped values show only if `tone=` gives
//! a fallback; `text=` replaces the shown text. Badges read in capitals.

use indexmap::IndexMap;
use rhizo_core::{Badge, Decor, DecorRow, DecorState, Document, Node, NodeId};

use crate::TypeDef;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub badge: Option<Tag>,
    pub subtitle: Option<String>,
    /// Values map to `normal`, `draft` or `void`.
    pub state: Option<Tag>,
    /// Accent of a card's title band or a container's outline.
    pub tone: Option<String>,
    /// Draw as a card even without rows.
    pub card: bool,
    pub container: bool,
    pub rows: Option<Rows>,
}

/// A template whose value picks a tone (tags) or a state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tag {
    pub template: String,
    pub text: Option<String>,
    pub tone: Option<String>,
    pub values: IndexMap<String, String>,
}

/// Children of type `of` (`*`: any) drawn as rows of a card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rows {
    pub of: String,
    pub detail: Option<String>,
    pub strong: Option<String>,
    pub marker: Option<Tag>,
}

impl Tag {
    /// The text and tone, if this value shows.
    fn eval(&self, n: &Node, def: Option<&TypeDef>) -> Option<(String, String)> {
        let v = render(&self.template, n, def)?;
        let tone = match self.values.get(&v) {
            Some(t) => t.clone(),
            None if self.values.is_empty() => self.tone.clone().unwrap_or_default(),
            None => self.tone.clone()?,
        };
        Some((self.text.clone().unwrap_or(v), tone))
    }

    fn badge(&self, n: &Node, def: Option<&TypeDef>) -> Option<Badge> {
        self.eval(n, def).map(|(text, tone)| Badge { text: text.to_uppercase(), tone })
    }
}

/// `template` filled from `n`'s props, or `None` if any part is empty.
pub fn render(template: &str, n: &Node, def: Option<&TypeDef>) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}')? + open;
        let value = rest[open + 1..close].split('|').find_map(|key| prop_text(n, def, key.trim()))?;
        out.push_str(&value);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    (!out.trim().is_empty()).then_some(out)
}

fn prop_text(n: &Node, def: Option<&TypeDef>, key: &str) -> Option<String> {
    let default = || def?.schema.get("properties")?.get(key)?.get("default");
    let v = n.props.get(key).or_else(default)?;
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(x) => x.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

fn truthy(s: &str) -> bool {
    !matches!(s, "" | "false" | "0" | "none")
}

/// Works out every typed node's decor from its kit's look, keyed by node id.
pub(crate) fn decorate<'k>(doc: &Document, def_of: impl Fn(&Node) -> Option<&'k TypeDef>) -> IndexMap<NodeId, Decor> {
    let mut out: IndexMap<NodeId, Decor> = IndexMap::new();
    for n in doc.nodes.values() {
        let def = def_of(n);
        let Some(look) = def.and_then(|d| d.look.as_ref()) else { continue };
        let decor = Decor {
            badge: look.badge.as_ref().and_then(|t| t.badge(n, def)),
            subtitle: look.subtitle.as_deref().and_then(|t| render(t, n, def)),
            state: look
                .state
                .as_ref()
                .and_then(|t| t.eval(n, def))
                .and_then(|(_, state)| DecorState::parse(&state))
                .unwrap_or_default(),
            tone: look.tone.clone().unwrap_or_default(),
            card: look.card || look.rows.is_some(),
            container: look.container,
            row: None,
        };
        // A row set by the parent's card (seen first) stays.
        let e = out.entry(n.id.clone()).or_default();
        *e = Decor { row: e.row.take(), ..decor };
        let Some(rows) = &look.rows else { continue };
        for c in n.children.iter().filter_map(|c| doc.node(c)) {
            let of_type = rows.of == "*" || c.ty.as_ref().is_some_and(|t| t.as_str() == rows.of);
            if !of_type || !c.children.is_empty() {
                continue;
            }
            let cdef = def_of(c);
            let row = DecorRow {
                detail: rows.detail.as_deref().and_then(|t| render(t, c, cdef)),
                strong: rows.strong.as_deref().and_then(|t| render(t, c, cdef)).is_some_and(|s| truthy(&s)),
                marker: rows.marker.as_ref().and_then(|t| t.badge(c, cdef)),
            };
            out.entry(c.id.clone()).or_default().row = Some(row);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use rhizo_core::{DecorState, NodeId};

    use crate::Kits;

    const SRC: &str = r#"rhizo version=2
use data version="0.1"
node "Customer" id=customer type=data.entity {
    node "id" id=cid type=data.field { props format="uuid" required=#true }
    node "name" id=name type=data.field { props pii="personal" }
    node "age" id=age type=data.field { props type="integer" pii="none" }
}
node "Status" id=status type=data.enum { - "open"; - "paid" }
node "List" id=list type=data.operation { props method="get" path="/customers" }
node "Draft" id=draft type=rhizo.decision
node "No" id=no type=rhizo.decision { props status="rejected" }
node "Plain" id=plain
"#;

    fn decor() -> indexmap::IndexMap<NodeId, rhizo_core::Decor> {
        let mut doc = rhizo_lang::load(SRC, rhizo_lang::Syntax::Kdl).document;
        Kits::builtin().apply_style_defaults(&mut doc);
        doc.decor
    }

    #[test]
    fn entities_become_cards_and_fields_rows() {
        let d = decor();
        let get = |id: &str| &d[&NodeId::from(id)];
        assert!(get("customer").card);
        let id = get("cid").row.as_ref().unwrap();
        assert_eq!((id.detail.as_deref(), id.strong, id.marker.as_ref()), (Some("uuid"), true, None));
        // No format: the type, else the schema's default type.
        let name = get("name").row.as_ref().unwrap();
        assert_eq!((name.detail.as_deref(), name.strong), (Some("string"), false));
        assert_eq!(name.marker.as_ref().map(|m| (m.text.as_str(), m.tone.as_str())), Some(("PII", "amber")));
        // An unmapped value (`none`) with no fallback tone shows no marker.
        let age = get("age").row.as_ref().unwrap();
        assert_eq!((age.detail.as_deref(), age.marker.as_ref()), (Some("integer"), None));
        // Enum values are rows of any type.
        assert!(get("status").card && get("open").row.is_some());
    }

    #[test]
    fn badges_subtitles_and_states() {
        let d = decor();
        let get = |id: &str| &d[&NodeId::from(id)];
        let op = get("list");
        assert_eq!(op.badge.as_ref().map(|b| (b.text.as_str(), b.tone.as_str())), Some(("GET", "green")));
        assert_eq!(op.subtitle.as_deref(), Some("/customers"));
        // The schema default (`proposed`) counts.
        assert_eq!(get("draft").state, DecorState::Draft);
        assert_eq!(get("draft").badge.as_ref().map(|b| b.text.as_str()), Some("PROPOSED"));
        assert_eq!(get("no").state, DecorState::Void);
        // A template with an empty part shows nothing.
        assert_eq!(get("draft").subtitle, None);
        assert!(!d.contains_key(&NodeId::from("plain")));
    }

    #[test]
    fn looks_reject_unknown_statements_and_states() {
        let kit = |look: &str| {
            let src = format!("kit k version=\"0.1.0\" {{ namespaces k }}\ntype k.t {{ look {{ {look} }} }}");
            crate::Kit::parse(&src, |_| None).map(|_| ())
        };
        assert!(kit("badge \"{x}\" a=red; subtitle \"{y}\"").is_ok());
        assert!(kit("sparkle").unwrap_err().0.contains("unknown statement `sparkle`"));
        assert!(kit("state \"{s}\" a=gone").unwrap_err().0.contains("unknown state `gone`"));
    }
}
