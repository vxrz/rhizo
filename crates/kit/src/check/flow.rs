//! Rules of the built-in `flow` kit, per `flow.workflow` frame: one
//! trigger, every step reachable from it, no loops, one way on from each
//! step except a switch, switch edges labelled with valid conditions, and
//! only steps the chosen targets can express.

use std::collections::{HashMap, HashSet};

use rhizo_core::{Frame, Node, NodeId};

use super::{Finding, Rules, Rx, Subject};
use crate::flow::{self, CALL, HTTP_REQUEST, SWITCH, TRANSFORM, is_default};

/// `alias:id` where `alias` is an import: a node of another file.
fn reference(doc: &rhizo_core::Document, id: &NodeId) -> bool {
    rhizo_core::typed::curie_prefix(id.as_str()).is_some_and(|p| doc.imports.contains_key(p))
}

fn ty(n: &Node) -> &str {
    n.ty.as_ref().map_or("", |t| t.as_str())
}

pub(crate) struct FlowRules;

impl Rules for FlowRules {
    fn kit(&self) -> &'static str {
        flow::KIT
    }

    fn check(&self, doc: &rhizo_core::Document, out: &mut Vec<Finding>) {
        Rx { doc, out }.flow_rules();
    }
}

impl Rx<'_> {
    fn flow_rules(&mut self) {
        let doc = self.doc;
        for f in doc.frames.iter().filter(|f| flow::is_workflow(f)) {
            self.workflow(f);
        }
        // Steps talk to other workflows only through flow.call.
        for (i, e) in doc.edges.iter().enumerate() {
            let (Some(a), Some(b)) = (doc.nodes.get(&e.from), doc.nodes.get(&e.to)) else { continue };
            if flow::is_step(a) && flow::is_step(b) && a.frame != b.frame {
                self.error(
                    Subject::Edge(i),
                    format!("`{}` and `{}` are steps of different workflows", a.id, b.id),
                    Some("run the other workflow with a flow.call step".into()),
                );
            }
        }
    }

    fn workflow(&mut self, f: &Frame) {
        let doc = self.doc;
        let steps = flow::steps(doc, f);
        let name = f.id.as_str();
        let triggers: Vec<&Node> = steps.iter().copied().filter(|n| flow::is_trigger(n)).collect();
        match triggers.as_slice() {
            [] => self.error(
                Subject::Frame(f.id.clone()),
                format!("workflow `{name}` has no trigger"),
                Some("add a flow.trigger.webhook, flow.trigger.schedule or flow.trigger.event".into()),
            ),
            [_] => {}
            [first, rest @ ..] => {
                for t in rest {
                    self.error(
                        Subject::Node(t.id.clone()),
                        format!("workflow `{name}` has a second trigger `{}` (the first is `{}`)", t.id, first.id),
                        Some("one trigger per workflow: split it into two workflows".into()),
                    );
                }
            }
        }

        let targets = flow::targets(f);
        let mut succ: HashMap<&NodeId, Vec<&NodeId>> = HashMap::new();
        for n in &steps {
            for target in &targets {
                if let Some(why) = flow::unsupported(n, target) {
                    self.error(
                        Subject::Node(n.id.clone()),
                        format!("{} `{}` cannot be compiled to {target}: {why}", ty(n), n.id),
                        Some(format!("leave {target} out of the workflow's `targets`")),
                    );
                }
            }
            let outs = flow::next(doc, n);
            succ.insert(&n.id, outs.iter().map(|e| &e.to).collect());
            if flow::is_trigger(n)
                && let Some(i) =
                    doc.edges.iter().position(|e| e.to == n.id && doc.nodes.get(&e.from).is_some_and(flow::is_step))
            {
                self.error(
                    Subject::Edge(i),
                    format!("trigger `{}` starts the workflow: nothing leads into it", n.id),
                    None,
                );
            }
            if ty(n) == SWITCH {
                self.switch(n);
            } else if outs.len() > 1 {
                let to: Vec<&str> = outs.iter().map(|e| e.to.as_str()).collect();
                self.error(
                    Subject::Node(n.id.clone()),
                    format!("`{}` leads to {} steps ({}); only a flow.switch branches", n.id, to.len(), to.join(", ")),
                    Some("put a flow.switch after it and label its edges with conditions".into()),
                );
            }
            match ty(n) {
                HTTP_REQUEST => self.http_request(n),
                TRANSFORM => {
                    if let Some(set) = n.props.get("set") {
                        self.templates(n, set);
                    }
                }
                CALL => {
                    let target = n.props.get("workflow").and_then(|w| w.as_str()).unwrap_or("");
                    let exists = doc.frames.iter().any(|g| g.id.as_str() == target && flow::is_workflow(g));
                    if !exists || target == name {
                        let others: Vec<&str> = doc
                            .frames
                            .iter()
                            .filter(|g| flow::is_workflow(g) && g.id != f.id)
                            .map(|g| g.id.as_str())
                            .collect();
                        let help = if others.is_empty() {
                            "no other workflow here".to_string()
                        } else {
                            format!("workflows: {}", others.join(", "))
                        };
                        self.error(
                            Subject::Node(n.id.clone()),
                            format!("`{}` calls `{target}`, which is not another workflow", n.id),
                            Some(help),
                        );
                    }
                }
                _ => {}
            }
        }

        // Reachable from the trigger.
        if let Some(t) = triggers.first() {
            let mut seen: HashSet<&NodeId> = HashSet::from([&t.id]);
            let mut stack = vec![&t.id];
            while let Some(id) = stack.pop() {
                for &s in succ.get(id).into_iter().flatten() {
                    if seen.insert(s) {
                        stack.push(s);
                    }
                }
            }
            // Other triggers are reported above.
            for n in steps.iter().filter(|n| !seen.contains(&n.id) && !flow::is_trigger(n)) {
                self.error(
                    Subject::Node(n.id.clone()),
                    format!("step `{}` is never reached from trigger `{}`", n.id, t.id),
                    Some("connect it with an edge from an earlier step, or remove it".into()),
                );
            }
        }

        // No loops: a depth-first search that reports each back edge.
        let mut state: HashMap<&NodeId, u8> = HashMap::new(); // 1 = on the path, 2 = done
        for n in &steps {
            let mut stack: Vec<(&NodeId, usize)> = vec![(&n.id, 0)];
            if state.contains_key(&n.id) {
                continue;
            }
            state.insert(&n.id, 1);
            while let Some((id, k)) = stack.pop() {
                let outs = succ.get(id).map_or(&[][..], Vec::as_slice);
                if let Some(&next) = outs.get(k) {
                    stack.push((id, k + 1));
                    match state.get(next) {
                        Some(1) => {
                            let i = doc.edges.iter().position(|e| &e.from == id && &e.to == next);
                            self.error(
                                i.map_or(Subject::Frame(f.id.clone()), Subject::Edge),
                                format!("the edge `{id}` → `{next}` closes a loop; workflows cannot loop yet"),
                                None,
                            );
                        }
                        Some(_) => {}
                        None => {
                            state.insert(next, 1);
                            stack.push((next, 0));
                        }
                    }
                } else {
                    state.insert(id, 2);
                }
            }
        }
    }

    /// Every edge out of a switch carries a condition, or `default` once.
    fn switch(&mut self, n: &Node) {
        let doc = self.doc;
        let outs = flow::next(doc, n);
        if outs.is_empty() {
            self.error(
                Subject::Node(n.id.clone()),
                format!("switch `{}` leads nowhere", n.id),
                Some("add an edge per branch, labelled with its condition".into()),
            );
        }
        let mut defaults = 0;
        for e in outs {
            let i = doc.edges.iter().position(|x| std::ptr::eq(x, e)).expect("the edge is in the document");
            match e.label.as_deref() {
                None => self.error(
                    Subject::Edge(i),
                    format!("the edge `{}` → `{}` leaves a switch without a condition", e.from, e.to),
                    Some("label it with a condition, like label=\"total > 1000\", or label=\"default\"".into()),
                ),
                Some(l) if is_default(l) => {
                    defaults += 1;
                    if defaults == 2 {
                        self.error(Subject::Edge(i), format!("switch `{}` has more than one default edge", n.id), None);
                    }
                }
                Some(l) => {
                    if let Err(why) = crate::expr::parse(l) {
                        self.error(
                            Subject::Edge(i),
                            format!("condition `{l}` on the edge `{}` → `{}`: {why}", e.from, e.to),
                            None,
                        );
                    }
                }
            }
        }
    }

    fn http_request(&mut self, n: &Node) {
        let doc = self.doc;
        let url = n.props.contains_key("url");
        // An edge to a node of an imported file: checked once the project is
        // merged, on that edge (findings on a cross-file edge are kept).
        if doc.edges.iter().any(|e| e.from == n.id && reference(doc, &e.to)) {
            return;
        }
        let op = flow::operation(doc, n);
        let at = match op {
            Some(op) => Subject::Edge(
                doc.edges.iter().position(|e| e.from == n.id && e.to == op.id).expect("the operation has an edge"),
            ),
            None => Subject::Node(n.id.clone()),
        };
        match op {
            Some(op) if url => self.error(
                at.clone(),
                format!("`{}` calls operation `{}` and also sets `url`", n.id, op.id),
                Some("drop `url`: the operation and its API give it".into()),
            ),
            Some(op) if flow::server(doc, op).is_none() => self.error(
                at.clone(),
                format!("`{}` calls operation `{}`, but its API lists no `servers`", n.id, op.id),
                Some("give the data.api frame a `servers` list".into()),
            ),
            Some(op) if n.props.contains_key("method") => self.error(
                at.clone(),
                format!("`{}` calls operation `{}`, which gives the method", n.id, op.id),
                Some("drop `method`".into()),
            ),
            Some(_) => {}
            None if !url => self.error(
                at.clone(),
                format!("`{}` has no `url`", n.id),
                Some("set url=\"https://…\", or draw an edge to the data.operation it calls".into()),
            ),
            None => {}
        }
        if let Some(body) = n.props.get("body") {
            if flow::method(doc, n) == "get" {
                self.error(
                    at,
                    format!("`{}` sends a body with a GET request", n.id),
                    Some("use another method, or drop `body`".into()),
                );
            }
            self.templates(n, body);
        }
    }

    fn templates(&mut self, n: &Node, v: &serde_json::Value) {
        match v {
            serde_json::Value::String(s) => {
                if let Err(why) = crate::expr::template(s) {
                    self.error(Subject::Node(n.id.clone()), format!("`{}`: template \"{s}\": {why}", n.id), None);
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| self.templates(n, x)),
            serde_json::Value::Object(o) => o.values().for_each(|x| self.templates(n, x)),
            _ => {}
        }
    }
}
