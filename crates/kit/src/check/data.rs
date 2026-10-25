//! Rules of the built-in `data` kit: relationships between entities say
//! how many, and an enum has values.

use rhizo_core::{Document, EdgeKind, NodeId};

use super::{Finding, Rules, Rx, Subject};
use crate::data::{ENTITY, ENUM, KIT};

pub(crate) struct DataRules;

impl Rules for DataRules {
    fn kit(&self) -> &'static str {
        KIT
    }

    fn check(&self, doc: &Document, out: &mut Vec<Finding>) {
        let mut rx = Rx { doc, out };
        let is =
            |id: &NodeId, ty: &str| doc.nodes.get(id).and_then(|n| n.ty.as_ref()).is_some_and(|t| t.as_str() == ty);
        for (i, e) in doc.edges.iter().enumerate() {
            if !(is(&e.from, ENTITY) && is(&e.to, ENTITY)) {
                continue;
            }
            if e.kind.is_some_and(|k| k != EdgeKind::Data) {
                continue; // reported by the port check
            }
            match e.props.get("cardinality") {
                None => {}
                Some(v) if v == "one" || v == "many" => {}
                Some(v) => rx.error(
                    Subject::Edge(i),
                    format!("cardinality {v} between entities: use \"one\" or \"many\""),
                    None,
                ),
            }
        }
        for n in doc.nodes.values() {
            if n.ty.as_ref().is_some_and(|t| t.as_str() == ENUM) && n.children.is_empty() {
                rx.warning(
                    Subject::Node(n.id.clone()),
                    format!("enum `{}` has no values", n.id.as_str()),
                    Some("add one child node per value".into()),
                );
            }
        }
    }
}
