//! The brief's plan: which nodes are work, how they group into tasks
//! (areas, flows, deploys), what each task depends on and in which order
//! they run.

use std::collections::{BTreeSet, HashMap, HashSet};

use rhizo_core::graph::{Dag, stable_toposort};
use rhizo_core::{Document, Frame, LayoutKind, Node, NodeId, Shape};
use rhizo_kit::rhizo::{DECISION, FLOW, GOAL};

use crate::target::TARGETS;

pub(crate) fn ty(n: &Node) -> Option<&str> {
    n.ty.as_ref().map(|t| t.as_str())
}

pub(crate) fn is(n: &Node, t: &str) -> bool {
    ty(n) == Some(t)
}

/// Goals and decisions describe the work; they are not work.
pub(crate) fn meta(n: &Node) -> bool {
    ty(n).is_some_and(|t| t.starts_with("rhizo."))
}

/// A decision point: a diamond, or a label that asks a question.
pub(crate) fn branch(n: &Node) -> bool {
    n.style.shape == Some(Shape::Diamond) || n.label.trim_end().ends_with('?')
}

pub(crate) struct Task<'a> {
    /// The node the task is about; for a flow, its first step.
    pub node: &'a Node,
    /// Set when the task builds a whole frame as one flow.
    pub flow: Option<&'a Frame>,
    /// Set when the task deploys a compiled workflow (a human step).
    pub deploy: Option<&'a Frame>,
    pub area: Option<&'a Node>,
    /// The task node and everything below it, in document order.
    pub nodes: Vec<&'a Node>,
    pub depends: Vec<usize>,
}

impl Task<'_> {
    pub fn id(&self) -> String {
        match (self.flow, self.deploy) {
            (Some(f), _) => format!("T-{}", f.id),
            (_, Some(f)) => format!("T-{}-deploy", f.id),
            _ => format!("T-{}", self.node.id),
        }
    }

    pub fn kind(&self) -> &'static str {
        if self.flow.is_some_and(|f| f.layout == LayoutKind::Sequence) {
            "implement-sequence"
        } else if self.flow.is_some() {
            "implement-flow"
        } else if self.deploy.is_some() {
            "human"
        } else if implement(self.node).is_some() {
            "implement"
        } else {
            "build"
        }
    }
}

/// The title of the task writing code for `n`, if some target says it is
/// that kind of node.
pub(crate) fn implement(n: &Node) -> Option<String> {
    TARGETS.iter().find_map(|t| t.implement(n))
}

pub(crate) struct Plan<'a> {
    /// A mindmap with no goal and nothing typed: thinking, not a build plan.
    pub thinking: bool,
    pub tasks: Vec<Task<'a>>,
    /// Node id → index of the task it belongs to.
    pub owner: HashMap<&'a NodeId, usize>,
    pub goals: Vec<&'a Node>,
    pub decisions: Vec<&'a Node>,
    /// Task indices, dependencies first, otherwise document order.
    pub order: Vec<usize>,
    /// Position of each task in `order`.
    pub pos: Vec<usize>,
}

/// **Tasks.** Every node that no target covers (and that is not a goal or
/// decision) belongs to one task. A top-level node with children is an
/// *area*: each child is a task and everything below it is that task's
/// scope. Any other top-level node is a task on its own. An edge between
/// two tasks makes the target depend on the source (cycles are broken in
/// document order).
pub(crate) fn plan(doc: &Document) -> Plan<'_> {
    let work = |n: &Node| !meta(n) && !TARGETS.iter().any(|t| t.covers(doc, n));
    let depth = |n: &Node| doc.ancestors(&n.id).count();
    let deploys: Vec<_> = TARGETS.iter().flat_map(|t| t.deploys(doc)).collect();
    let deployed: HashSet<&rhizo_core::FrameId> = deploys.iter().map(|d| &d.frame.id).collect();
    let mut tasks: Vec<Task> = Vec::new();
    let mut owner = HashMap::new();
    // A flow is built as one unit: a frame typed `rhizo.flow`, a flat
    // flowchart with a decision in it, or a sequence (an interaction).
    for f in doc.frames.iter().filter(|f| !deployed.contains(&f.id)) {
        let ns: Vec<&Node> = doc.nodes.values().filter(|n| n.frame == f.id && work(n)).collect();
        let roots: Vec<&Node> = ns.iter().copied().filter(|n| n.parent.is_none()).collect();
        let typed = f.ty.as_ref().is_some_and(|t| t.as_str() == FLOW);
        let flowchart = matches!(f.layout, LayoutKind::Layered | LayoutKind::Auto)
            && roots.len() == ns.len()
            && ns.iter().any(|n| branch(n));
        let sequence = f.layout == LayoutKind::Sequence && !ns.is_empty();
        if !sequence && (ns.len() < 2 || !(typed || flowchart)) {
            continue;
        }
        let mut nodes = Vec::new();
        for r in flow_order(doc, &roots) {
            subtree(doc, r, &mut nodes);
        }
        let nodes: Vec<&Node> = nodes.into_iter().filter(|x| work(x)).collect();
        for x in &nodes {
            owner.entry(&x.id).or_insert(tasks.len());
        }
        // Everything in the frame may be goals or decisions: nothing to build.
        let Some(&first) = nodes.first() else { continue };
        tasks.push(Task { node: first, flow: Some(f), deploy: None, area: None, nodes, depends: Vec::new() });
    }
    for n in doc.nodes.values().filter(|n| work(n)) {
        let d = depth(n);
        let parent = n.parent.as_ref().and_then(|p| doc.nodes.get(p));
        let is_area = d == 0 && n.children.iter().filter_map(|c| doc.nodes.get(c)).any(&work);
        let starts_task = !is_area && (d == 0 || (d == 1 || parent.is_some_and(|p| !work(p))));
        if starts_task && !owner.contains_key(&n.id) {
            let area = parent.filter(|p| work(p) && depth(p) == 0);
            let mut nodes = Vec::new();
            subtree(doc, n, &mut nodes);
            let nodes: Vec<&Node> = nodes.into_iter().filter(|x| work(x)).collect();
            for x in &nodes {
                owner.entry(&x.id).or_insert(tasks.len());
            }
            tasks.push(Task { node: n, flow: None, deploy: None, area, nodes, depends: Vec::new() });
        }
    }
    for (i, t) in tasks.iter_mut().enumerate().filter(|(_, t)| t.flow.is_none()) {
        for part in TARGETS.iter().flat_map(|x| x.parts(doc, t.node)) {
            owner.entry(&part.id).or_insert(i);
            t.nodes.push(part);
        }
    }
    // A compiled workflow leaves a person one step: deploy it, after the
    // code in it is written.
    for d in deploys {
        let Some(node) = d.node else { continue };
        let depends: BTreeSet<usize> = d.nodes.iter().filter_map(|n| owner.get(&n.id).copied()).collect();
        for n in &d.nodes {
            owner.entry(&n.id).or_insert(tasks.len());
        }
        let depends = depends.into_iter().collect();
        tasks.push(Task { node, flow: None, deploy: Some(d.frame), area: None, nodes: d.nodes, depends });
    }
    let goals: Vec<&Node> = doc.nodes.values().filter(|n| is(n, GOAL)).collect();
    let decisions: Vec<&Node> = doc.nodes.values().filter(|n| is(n, DECISION)).collect();
    let thinking = goals.is_empty()
        && doc.nodes.values().all(|n| n.ty.is_none())
        && doc.frames.iter().filter(|f| doc.nodes.values().any(|n| n.frame == f.id)).all(|f| {
            // `auto` draws a mindmap when a frame has nesting and no edges.
            let nested = doc.nodes.values().any(|n| n.frame == f.id && n.parent.is_some());
            let linked = doc.edges.iter().any(|e| doc.nodes.get(&e.from).is_some_and(|n| n.frame == f.id));
            f.layout == LayoutKind::Mindmap || (f.layout == LayoutKind::Auto && nested && !linked)
        });
    if thinking {
        // Nothing says what to build: describe the map, plan nothing.
        tasks.clear();
        owner.clear();
    }
    link(doc, &mut tasks, &owner);
    let order = stable_toposort(
        tasks.len(),
        tasks.iter().enumerate().flat_map(|(j, t)| t.depends.iter().map(move |&d| (d, j))),
    );
    let mut pos = vec![0; tasks.len()];
    for (p, &i) in order.iter().enumerate() {
        pos[i] = p;
    }
    Plan { thinking, tasks, owner, goals, decisions, order, pos }
}

/// Edge from task A to task B: B depends on A, unless that closes a
/// cycle. A flow uses what it touches, so it comes after those parts
/// whichever way the edge points.
fn link(doc: &Document, tasks: &mut [Task], owner: &HashMap<&NodeId, usize>) {
    let mut dag = Dag::new(tasks.len());
    let mut linked = HashSet::new();
    for (j, t) in tasks.iter().enumerate() {
        for &d in &t.depends {
            dag.add(d, j);
            linked.insert((d, j));
        }
    }
    let whole = |t: &Task| t.flow.is_some() || t.deploy.is_some();
    for e in &doc.edges {
        let (Some(&a), Some(&b)) = (owner.get(&e.from), owner.get(&e.to)) else { continue };
        let (a, b) = if whole(&tasks[a]) && !whole(&tasks[b]) { (b, a) } else { (a, b) };
        // `add` refuses a == b and anything that would close a cycle.
        if linked.contains(&(a, b)) || !dag.add(a, b) {
            continue;
        }
        linked.insert((a, b));
        tasks[b].depends.push(a);
    }
}

/// `roots` sorted so that edges between them point forward; document order
/// breaks ties and cycles.
pub(crate) fn flow_order<'a>(doc: &Document, roots: &[&'a Node]) -> Vec<&'a Node> {
    let index: HashMap<&NodeId, usize> = roots.iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
    let edges = doc.edges.iter().filter_map(|e| Some((*index.get(&e.from)?, *index.get(&e.to)?)));
    stable_toposort(roots.len(), edges.filter(|(a, b)| a != b)).into_iter().map(|i| roots[i]).collect()
}

pub(crate) fn subtree<'a>(doc: &'a Document, n: &'a Node, out: &mut Vec<&'a Node>) {
    out.push(n);
    for c in n.children.iter().filter_map(|c| doc.nodes.get(c)) {
        subtree(doc, c, out);
    }
}

/// Edges by the node they leave or enter, as indices into `doc.edges` in
/// document order.
pub(crate) struct Adjacency<'a> {
    outgoing: HashMap<&'a NodeId, Vec<usize>>,
    incoming: HashMap<&'a NodeId, Vec<usize>>,
}

impl<'a> Adjacency<'a> {
    pub fn new(doc: &'a Document) -> Self {
        let mut a = Adjacency { outgoing: HashMap::new(), incoming: HashMap::new() };
        for (i, e) in doc.edges.iter().enumerate() {
            a.outgoing.entry(&e.from).or_default().push(i);
            a.incoming.entry(&e.to).or_default().push(i);
        }
        a
    }

    pub fn outgoing(&self, id: &NodeId) -> &[usize] {
        self.outgoing.get(id).map_or(&[], Vec::as_slice)
    }

    pub fn incoming(&self, id: &NodeId) -> &[usize] {
        self.incoming.get(id).map_or(&[], Vec::as_slice)
    }

    /// Edges that leave or enter `id`, each once, in document order.
    pub fn touching(&self, id: &NodeId) -> Vec<usize> {
        let mut v: Vec<usize> = self.outgoing(id).iter().chain(self.incoming(id)).copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}
