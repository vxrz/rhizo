//! What a `flow.workflow` is made of, shared by the kit's rules and the
//! flow compilers: its steps, the control edges between them and what each
//! target can express.

use rhizo_core::{Document, Edge, Frame, Node};

pub const KIT: &str = "flow";

pub const WORKFLOW: &str = "flow.workflow";
pub const TRIGGER_WEBHOOK: &str = "flow.trigger.webhook";
pub const TRIGGER_SCHEDULE: &str = "flow.trigger.schedule";
pub const TRIGGER_EVENT: &str = "flow.trigger.event";
pub const HTTP_REQUEST: &str = "flow.http-request";
pub const SWITCH: &str = "flow.switch";
pub const TRANSFORM: &str = "flow.transform";
pub const DELAY: &str = "flow.delay";
pub const CALL: &str = "flow.call";
pub const SCRIPT: &str = "flow.script";

pub const SERVERLESS_WORKFLOW: &str = "serverless-workflow";
pub const NODE_RED: &str = "node-red";
pub const TARGETS: [&str; 2] = [SERVERLESS_WORKFLOW, NODE_RED];

fn ty(n: &Node) -> &str {
    n.ty.as_ref().map_or("", |t| t.as_str())
}

pub fn is_workflow(f: &Frame) -> bool {
    f.ty.as_ref().is_some_and(|t| t.as_str() == WORKFLOW)
}

/// A typed node of the flow kit: a trigger or a step.
pub fn is_step(n: &Node) -> bool {
    ty(n).starts_with("flow.")
}

pub fn is_trigger(n: &Node) -> bool {
    ty(n).starts_with("flow.trigger.")
}

/// The workflow's triggers and steps, in document order.
pub fn steps<'a>(doc: &'a Document, f: &Frame) -> Vec<&'a Node> {
    doc.nodes.values().filter(|n| n.frame == f.id && is_step(n)).collect()
}

/// Control edges out of `n`: edges to other steps of the same workflow, in
/// document order. Edges to anything else (an API operation, a goal) are
/// references, not control flow.
pub fn next<'a>(doc: &'a Document, n: &Node) -> Vec<&'a Edge> {
    doc.edges
        .iter()
        .filter(|e| e.from == n.id)
        .filter(|e| doc.nodes.get(&e.to).is_some_and(|t| t.frame == n.frame && is_step(t)))
        .collect()
}

/// The `data.operation` a `flow.http-request` calls, through an edge to it.
pub fn operation<'a>(doc: &'a Document, n: &Node) -> Option<&'a Node> {
    if ty(n) != HTTP_REQUEST {
        return None;
    }
    doc.edges
        .iter()
        .filter(|e| e.from == n.id)
        .find_map(|e| doc.nodes.get(&e.to).filter(|t| ty(t) == crate::data::OPERATION))
}

/// The HTTP method a `flow.http-request` uses, lowercase: its operation's,
/// else its own `method`, else `get`.
pub fn method<'a>(doc: &'a Document, n: &'a Node) -> &'a str {
    let own = |m: &'a Node| m.props.get("method").and_then(|v| v.as_str());
    operation(doc, n).and_then(own).or_else(|| own(n)).unwrap_or("get")
}

/// The first server of the API an operation belongs to.
pub fn server<'a>(doc: &'a Document, op: &Node) -> Option<&'a str> {
    doc.frame(&op.frame)?.props.get("servers")?.as_array()?.first()?.as_str()
}

/// The targets a workflow compiles to (`targets`, else all).
pub fn targets(f: &Frame) -> Vec<&str> {
    match f.props.get("targets").and_then(|t| t.as_array()) {
        Some(list) => list.iter().filter_map(|t| t.as_str()).filter(|t| TARGETS.contains(t)).collect(),
        None => TARGETS.to_vec(),
    }
}

/// Why `target` cannot express step `n`, if it cannot.
pub fn unsupported(n: &Node, target: &str) -> Option<&'static str> {
    match (ty(n), target) {
        (TRIGGER_EVENT, NODE_RED) => Some("Node-RED has no core node that receives CloudEvents"),
        (CALL, NODE_RED) => Some("Node-RED has no core node that runs another flow and waits for its result"),
        _ => None,
    }
}

/// An edge label that marks a switch's fallback branch.
pub fn is_default(label: &str) -> bool {
    matches!(label.trim(), "default" | "else" | "otherwise")
}
