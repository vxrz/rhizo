//! The build brief: what an agent needs to build what the diagram describes
//! but no compiler generates. Written as `AGENTS.md` (for agents and people)
//! and `tasks/tasks.json` (for tools). Like everything in the bundle it is
//! derived, deterministic and regenerated on every compile (design doc §4.6).
//!
//! [`graph`] plans the tasks and their order; this module turns the plan
//! into typed records ([`TaskOut`] and friends), which serialize to
//! `tasks.json` and which [`md`] writes as `AGENTS.md`. Kit-specific parts
//! (what is generated, derived criteria, human tasks) come from the
//! compile targets ([`crate::target`]).

mod graph;
mod md;

use std::collections::{HashMap, HashSet};

use rhizo_core::{Document, Edge, Frame, LayoutKind, Node, NodeId, Props};
use rhizo_kit::Kits;
use serde::Serialize;
use serde_json::Value;

pub(crate) use graph::Task;
use graph::{Adjacency, Plan, branch, implement, meta, ty};

use crate::target::{Generated, TARGETS, of_path};
use crate::{Files, OutFile};

pub const AGENTS: &str = "AGENTS.md";
pub const TASKS: &str = "tasks/tasks.json";

/// `tasks/tasks.json`.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct TasksFile<'a> {
    pub rhizo_tasks: u32,
    pub comment: &'a str,
    pub source: &'a str,
    pub goals: Vec<Described<'a>>,
    pub decisions: Vec<Decision<'a>>,
    pub open_questions: Vec<String>,
    pub tasks: Vec<TaskOut<'a>>,
}

/// One task. Fields are written in this order, empty ones left out where
/// marked.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct TaskOut<'a> {
    pub id: String,
    pub title: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<&'a str>,
    /// The label of the area the task is in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<&'a str>,
    /// The frame a flow task builds, or an environment task applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<&'a str>,
    pub depends_on: Vec<String>,
    /// The workflow a deploy task deploys (written after `depends-on`).
    #[serde(rename = "frame", skip_serializing_if = "Option::is_none")]
    pub deploys: Option<&'a str>,
    #[serde(flatten)]
    pub extras: Extras<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_description: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<ScopeItem<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections: Option<Vec<Connection<'a>>>,
    pub inputs: Vec<String>,
    /// Ids of the goals the task serves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goals: Option<Vec<&'a str>>,
    /// Ids of the decisions the task follows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<&'a str>>,
    pub acceptance: Vec<Criterion>,
}

/// A node's type, note, link, tags, props and criteria, when present.
#[derive(Default, Serialize)]
pub(crate) struct Extras<'a> {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<&'a str>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub tags: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub props: Option<&'a Props>,
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub accept: &'a [String],
}

impl<'a> Extras<'a> {
    fn of(n: &'a Node) -> Self {
        Extras {
            ty: ty(n),
            note: n.note.as_deref(),
            link: n.link.as_deref(),
            tags: &n.tags,
            props: props(&n.props),
            accept: &n.accept,
        }
    }
}

fn props(p: &Props) -> Option<&Props> {
    (!p.is_empty()).then_some(p)
}

/// A step of a flow, or a part of a task's scope.
#[derive(Serialize)]
pub(crate) struct ScopeItem<'a> {
    pub node: &'a str,
    pub label: String,
    #[serde(flatten)]
    pub extras: Extras<'a>,
    /// Where a step leads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<ScopeItem<'a>>,
}

/// An edge between a task's nodes and something outside it.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct Connection<'a> {
    /// `in` or `out`.
    pub direction: &'static str,
    pub node: &'a str,
    pub label: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_label: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_props: Option<&'a Props>,
}

/// An acceptance criterion and where it comes from: `authored` (an
/// `accept` line), or what derived it (`flow`, `data`, `infra`, …).
#[derive(Serialize)]
pub(crate) struct Criterion {
    pub source: &'static str,
    pub text: String,
}

impl Criterion {
    pub fn new(source: &'static str, text: impl Into<String>) -> Self {
        Criterion { source, text: text.into() }
    }
}

/// A goal.
#[derive(Serialize)]
pub(crate) struct Described<'a> {
    pub id: &'a str,
    pub label: &'a str,
    #[serde(flatten)]
    pub extras: Extras<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct Decision<'a> {
    #[serde(flatten)]
    pub node: Described<'a>,
    pub applies_to: AppliesTo,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum AppliesTo {
    /// `"everything"`: the decision is linked to nothing.
    Everything(&'static str),
    /// Labels of the nodes it is linked to.
    Nodes(Vec<String>),
}

/// Adds `AGENTS.md` and `tasks/tasks.json` to `files` (which already holds
/// the generated specs, so tasks can point at them).
pub(crate) fn emit(
    doc: &Document,
    kits: &Kits,
    header: &str,
    source_name: &str,
    origins: Option<&crate::Origins>,
    files: &mut Files,
) {
    let b = Brief::new(doc, kits, origins, files);
    let mut tasks: Vec<TaskOut> = b.plan.order.iter().map(|&i| b.task_out(i)).collect();
    let order: Vec<&Task> = b.plan.order.iter().map(|&i| &b.plan.tasks[i]).collect();
    tasks.extend(TARGETS.iter().flat_map(|t| t.human_tasks(doc, files, &order)));
    let out = TasksFile {
        rhizo_tasks: 1,
        comment: header,
        source: source_name,
        goals: b.plan.goals.iter().map(|g| Described::of(g)).collect(),
        decisions: b.plan.decisions.iter().map(|d| b.decision(d)).collect(),
        open_questions: b.questions(),
        tasks,
    };
    let md = md::markdown(&b, header, source_name, &out);
    let json = serde_json::to_string_pretty(&out).expect("the brief serializes") + "\n";
    let mut nodes: Vec<String> = doc.nodes.keys().map(|id| id.to_string()).collect();
    nodes.sort();
    files.insert(AGENTS.into(), OutFile { contents: md, nodes: nodes.clone() });
    files.insert(TASKS.into(), OutFile { contents: json, nodes });
}

impl<'a> Described<'a> {
    fn of(n: &'a Node) -> Self {
        Described { id: n.id.as_str(), label: &n.label, extras: Extras::of(n) }
    }
}

pub(crate) struct Brief<'a> {
    doc: &'a Document,
    kits: &'a Kits,
    /// For a merged project: which file each edge and node came from.
    origins: Option<&'a crate::Origins>,
    files: &'a Files,
    plan: Plan<'a>,
    edges: Adjacency<'a>,
    /// How many nodes have each label.
    labels: HashMap<&'a str, usize>,
    /// Generated files in path order, with how the brief uses each.
    generated: Vec<(&'a String, &'a OutFile, Option<Generated>)>,
    /// Node id → indices into `generated` of the files about it.
    files_of: HashMap<&'a str, Vec<usize>>,
}

impl<'a> Brief<'a> {
    fn new(doc: &'a Document, kits: &'a Kits, origins: Option<&'a crate::Origins>, files: &'a Files) -> Self {
        let mut labels = HashMap::new();
        for n in doc.nodes.values() {
            *labels.entry(n.label.as_str()).or_insert(0) += 1;
        }
        let generated: Vec<_> = files.iter().map(|(p, f)| (p, f, of_path(p).map(|t| t.generated()))).collect();
        let mut files_of: HashMap<&str, Vec<usize>> = HashMap::new();
        for (k, (_, f, _)) in generated.iter().enumerate() {
            for n in &f.nodes {
                files_of.entry(n.as_str()).or_default().push(k);
            }
        }
        Brief {
            doc,
            kits,
            origins,
            files,
            plan: graph::plan(doc),
            edges: Adjacency::new(doc),
            labels,
            generated,
            files_of,
        }
    }

    /// What the diagram leaves open, as questions for the person. Derived
    /// deterministically from the model, never guessed.
    fn questions(&self) -> Vec<String> {
        let p = &self.plan;
        let mut q = Vec::new();
        if p.thinking {
            return vec![
                "What should come of this map? It has no goal (`rhizo.goal`), so it is described, not planned.".into(),
            ];
        }
        if p.goals.is_empty() {
            q.push("What does done look like? The diagram has no goal (`rhizo.goal`).".to_string());
        }
        let decided = |words: &[&str]| {
            p.decisions.iter().any(|d| {
                let text = format!(
                    "{} {} {}",
                    d.label,
                    d.note.clone().unwrap_or_default(),
                    serde_json::to_string(&d.props).unwrap_or_default()
                )
                .to_lowercase();
                words.iter().any(|w| text.contains(w))
            })
        };
        for t in TARGETS {
            q.extend(t.questions(self.doc, &decided));
        }
        let kinds = || p.order.iter().map(|&i| p.tasks[i].kind());
        let builds_code = kinds().any(|k| matches!(k, "implement" | "implement-flow" | "implement-sequence"));
        if builds_code && !decided(&["repo", "monorepo", "folder", "layout"]) {
            q.push("Where does the code go: which repository, and how are its folders laid out?".into());
        }
        let accepted = p.decisions.iter().any(|d| d.props.get("status").and_then(Value::as_str) == Some("accepted"));
        if !p.order.is_empty() && !accepted {
            q.push(
                "Which technology stack, platforms and external systems should this use? The diagram records no \
                 accepted decision (`rhizo.decision`)."
                    .to_string(),
            );
        }
        for d in &p.decisions {
            let status = d.props.get("status").and_then(Value::as_str).unwrap_or("proposed");
            if status == "proposed" {
                q.push(format!("Decide: {} (still proposed).", d.label));
            }
        }
        // Messages without a label: what is sent is unknown.
        for f in self.doc.frames.iter().filter(|f| f.layout == LayoutKind::Sequence) {
            let blank: Vec<String> = self
                .interaction(f)
                .1
                .iter()
                .enumerate()
                .filter(|(_, e)| e.label.is_none())
                .map(|(k, e)| format!("{} ({} → {})", k + 1, self.label(&e.from), self.label(&e.to)))
                .collect();
            if !blank.is_empty() {
                q.push(format!(
                    "In \"{}\", what do these messages carry: {}? Label them.",
                    frame_name(self.doc, f),
                    blank.join(", ")
                ));
            }
        }
        // Branches whose ways out are not labelled: the condition is unknown.
        for n in self.doc.nodes.values().filter(|n| !meta(n)) {
            let outs: Vec<&Edge> =
                self.edges.outgoing(&n.id).iter().map(|&k| &self.doc.edges[k]).filter(|e| e.to != n.id).collect();
            if branch(n) && outs.len() >= 2 && outs.iter().any(|e| e.label.is_none()) {
                let targets: Vec<String> = outs.iter().map(|e| format!("\"{}\"", self.label(&e.to))).collect();
                q.push(format!(
                    "At \"{}\", what decides between {}? Label the edges with the conditions.",
                    n.label,
                    or_join(&targets)
                ));
            }
        }
        let unchecked: Vec<String> = p
            .order
            .iter()
            .map(|&i| &p.tasks[i])
            .filter(|t| t.nodes.iter().all(|n| n.accept.is_empty()) && !matches!(t.kind(), "implement" | "human"))
            .map(|t| format!("`{}`", t.id()))
            .collect();
        match unchecked.as_slice() {
            [] => {}
            [one] => q.push(format!("How will we know {one} is done? It has no acceptance criteria (`accept \"…\"`).")),
            many => q.push(format!(
                "How will we know these are done: {}? They have no acceptance criteria (`accept \"…\"`).",
                many.join(", ")
            )),
        }
        q
    }

    /// A sequence's participants (its own nodes, then others it talks to,
    /// in order of appearance) and its messages in order: every edge that
    /// starts or ends in the frame.
    fn interaction(&self, f: &Frame) -> (Vec<&'a NodeId>, Vec<&'a Edge>) {
        let doc = self.doc;
        let mut people: Vec<&NodeId> =
            doc.nodes.values().filter(|n| n.frame == f.id && !meta(n)).map(|n| &n.id).collect();
        let msgs: Vec<&Edge> = doc
            .edges
            .iter()
            .filter(|e| {
                doc.nodes.get(&e.from).is_some_and(|n| n.frame == f.id)
                    || doc.nodes.get(&e.to).is_some_and(|n| n.frame == f.id)
            })
            .collect();
        for e in &msgs {
            for end in [&e.from, &e.to] {
                if !people.contains(&end) {
                    people.push(end);
                }
            }
        }
        (people, msgs)
    }

    /// `Customer → Web app: Place order`, `(reply)` for dashed lines.
    fn message(&self, e: &Edge) -> String {
        let to = if e.from == e.to { "itself".to_string() } else { self.label(&e.to) };
        let what = e.label.as_deref().unwrap_or("(no label)");
        let reply = if e.line() == rhizo_core::LineStyle::Dashed { " (reply)" } else { "" };
        format!("{} → {to}: {what}{reply}", self.label(&e.from))
    }

    /// A flow's steps in order, each with where it leads; a sequence's
    /// messages in order.
    fn steps(&self, t: &Task<'a>) -> Vec<ScopeItem<'a>> {
        if let Some(f) = t.flow.filter(|f| f.layout == LayoutKind::Sequence) {
            return self
                .interaction(f)
                .1
                .iter()
                .enumerate()
                .map(|(k, e)| ScopeItem {
                    node: e.from.as_str(),
                    label: format!("{}. {}", k + 1, self.message(e)),
                    extras: Extras::default(),
                    next: Vec::new(),
                    scope: Vec::new(),
                })
                .collect();
        }
        let inside: HashSet<&NodeId> = t.nodes.iter().map(|n| &n.id).collect();
        t.nodes
            .iter()
            .map(|n| {
                let next = self
                    .edges
                    .outgoing(&n.id)
                    .iter()
                    .map(|&k| {
                        let e = &self.doc.edges[k];
                        let mut s = self.label(&e.to);
                        if let Some(l) = &e.label {
                            s += &format!(" ({l})");
                        }
                        // A step outside the flow: say whose it is.
                        if !inside.contains(&e.to)
                            && let Some(&o) = self.plan.owner.get(&e.to)
                        {
                            s += &format!(", see `{}`", self.plan.tasks[o].id());
                        }
                        s
                    })
                    .collect();
                ScopeItem {
                    node: n.id.as_str(),
                    label: n.label.clone(),
                    extras: Extras::of(n),
                    next,
                    scope: Vec::new(),
                }
            })
            .collect()
    }

    fn title(&self, t: &Task) -> String {
        let n = t.node;
        if let Some(f) = t.flow {
            let what = if f.layout == LayoutKind::Sequence { "sequence" } else { "flow" };
            format!("Implement the {what} \"{}\"", frame_name(self.doc, f))
        } else if let Some(f) = t.deploy {
            format!("Deploy the workflow \"{}\"", frame_name(self.doc, f))
        } else {
            implement(n).unwrap_or_else(|| n.label.clone())
        }
    }

    /// A node's label, with its id when another node has the same label.
    fn label(&self, id: &NodeId) -> String {
        match self.doc.nodes.get(id) {
            None => id.to_string(),
            Some(n) if self.labels.get(n.label.as_str()).is_some_and(|&c| c > 1) => format!("{} [{}]", n.label, n.id),
            Some(n) => n.label.clone(),
        }
    }

    /// Whether edge `i` was drawn in the same file as node `id` (always true
    /// outside a merged project).
    fn same_file(&self, i: usize, id: &NodeId) -> bool {
        let Some(o) = self.origins else { return true };
        match (o.edge_file.get(i), o.node_file.get(id)) {
            (Some(e), Some(n)) => e == n,
            _ => true,
        }
    }

    /// Edges touching the task's nodes from outside it.
    fn connections(&self, t: &Task<'a>) -> Vec<Connection<'a>> {
        let inside: HashSet<&NodeId> = t.nodes.iter().map(|n| &n.id).collect();
        let home = &t.node.id;
        let mut touching: Vec<usize> = t.nodes.iter().flat_map(|n| self.edges.touching(&n.id)).collect();
        touching.sort_unstable();
        touching.dedup();
        let mut out = Vec::new();
        for k in touching {
            let e: &'a Edge = &self.doc.edges[k];
            // Edges drawn in another diagram belong to that diagram's story.
            if !self.same_file(k, home) {
                continue;
            }
            let (direction, other) = match (inside.contains(&e.from), inside.contains(&e.to)) {
                (true, false) => ("out", &e.to),
                (false, true) => ("in", &e.from),
                _ => continue,
            };
            out.push(Connection {
                direction,
                node: other.as_str(),
                label: self.label(other),
                ty: self.doc.nodes.get(other).and_then(ty),
                edge_label: e.label.as_deref(),
                kind: e.kind.map(|k| k.name()),
                edge_props: props(&e.props),
            });
        }
        out
    }

    /// Goals or decisions linked to the task (by an edge to any of its nodes
    /// or its area), or all unlinked ones, which apply everywhere.
    fn applying(&self, t: &Task<'a>, of: &[&'a Node]) -> Vec<&'a str> {
        let mine: HashSet<&NodeId> = t.nodes.iter().map(|n| &n.id).chain(t.area.map(|a| &a.id)).collect();
        of.iter()
            .filter(|m| {
                let links = self.edges.touching(&m.id);
                let other = |k: &usize| {
                    let e = &self.doc.edges[*k];
                    if e.from == m.id { &e.to } else { &e.from }
                };
                links.is_empty() || links.iter().any(|k| mine.contains(other(k)))
            })
            .map(|m| m.id.as_str())
            .collect()
    }

    fn acceptance(&self, t: &Task, inputs: &[String]) -> Vec<Criterion> {
        let mut out = Vec::new();
        // An unwritten script's criteria belong to its own task.
        for n in t.nodes.iter().filter(|n| t.deploy.is_none() || implement(n).is_none()) {
            for a in &n.accept {
                let text = if std::ptr::eq(*n, t.node) { a.clone() } else { format!("{}: {a}", n.label) };
                out.push(Criterion::new("authored", text));
            }
        }
        if t.flow.is_some_and(|f| f.layout == LayoutKind::Sequence) {
            out.push(Criterion::new(
                "sequence",
                "Each message goes between the participants shown, in the order shown, and carries what its label says",
            ));
        } else if t.flow.is_some() {
            let text = if t.nodes.iter().any(|n| branch(n)) {
                "Runs the steps in the order shown and takes each branch on its labelled condition"
            } else {
                "Runs the steps in the order shown"
            };
            out.push(Criterion::new("flow", text));
        }
        for target in TARGETS {
            out.extend(target.criteria(self.doc, self.files, t, inputs));
        }
        out
    }

    /// Generated files that describe the task's nodes or their neighbors.
    fn inputs(&self, t: &Task) -> Vec<String> {
        let mine: HashSet<&str> = t.nodes.iter().map(|n| n.id.as_str()).collect();
        let mut near = mine.clone();
        // Direct neighbors only: a file about something two edges away is not an input.
        for n in &t.nodes {
            near.extend(self.edges.outgoing(&n.id).iter().map(|&k| self.doc.edges[k].to.as_str()));
            near.extend(self.edges.incoming(&n.id).iter().map(|&k| self.doc.edges[k].from.as_str()));
        }
        let mut hits: Vec<usize> = near.iter().filter_map(|n| self.files_of.get(n)).flatten().copied().collect();
        hits.sort_unstable();
        hits.dedup();
        hits.into_iter()
            .map(|k| self.generated[k])
            .filter(|(_, f, g)| match g {
                Some(Generated::Spec | Generated::Infra) => true,
                // A compiled workflow is an input to the tasks of its own steps only.
                Some(Generated::Flow) => f.nodes.iter().any(|n| mine.contains(n.as_str())),
                None => false,
            })
            .map(|(p, _, _)| p.clone())
            .collect()
    }

    fn scope(&self, n: &'a Node) -> Vec<ScopeItem<'a>> {
        n.children
            .iter()
            .filter_map(|c| self.doc.nodes.get(c))
            .filter(|c| !meta(c))
            .map(|c| ScopeItem {
                node: c.id.as_str(),
                label: c.label.clone(),
                extras: Extras::of(c),
                next: Vec::new(),
                scope: self.scope(c),
            })
            .collect()
    }

    fn task_out(&self, i: usize) -> TaskOut<'a> {
        let p = &self.plan;
        let t = &p.tasks[i];
        let mut deps: Vec<usize> = t.depends.clone();
        deps.sort_by_key(|&d| p.pos[d]);
        let extras = match (t.flow, t.deploy) {
            (_, Some(f)) | (Some(f), None) => Extras { props: props(&f.props), ..Extras::default() },
            (None, None) => Extras::of(t.node),
        };
        let whole = t.flow.is_some() || t.deploy.is_some();
        let type_description = ty(t.node)
            .filter(|_| !whole)
            .and_then(|x| self.kits.find_type(x))
            .map(|def| def.description.as_deref().unwrap_or_default());
        let inputs = self.inputs(t);
        TaskOut {
            id: t.id(),
            title: self.title(t),
            kind: t.kind(),
            node: Some(t.node.id.as_str()),
            area: t.area.map(|a| a.label.as_str()),
            frame: t.flow.map(|f| f.id.as_str()),
            depends_on: deps.iter().map(|&d| p.tasks[d].id()).collect(),
            deploys: t.deploy.map(|f| f.id.as_str()),
            extras,
            type_description,
            scope: if whole { self.steps(t) } else { self.scope(t.node) },
            connections: Some(self.connections(t)),
            goals: Some(self.applying(t, &p.goals)),
            decisions: Some(self.applying(t, &p.decisions)),
            acceptance: self.acceptance(t, &inputs),
            inputs,
        }
    }

    fn decision(&self, d: &'a Node) -> Decision<'a> {
        let applies: Vec<String> = self
            .edges
            .touching(&d.id)
            .into_iter()
            .map(|k| {
                let e = &self.doc.edges[k];
                if e.from == d.id { self.label(&e.to) } else { self.label(&e.from) }
            })
            .collect();
        let applies_to =
            if applies.is_empty() { AppliesTo::Everything("everything") } else { AppliesTo::Nodes(applies) };
        Decision { node: Described::of(d), applies_to }
    }
}

/// A frame's name: its label, else the document title for the implicit
/// frame (or `Main`), else its id.
pub(crate) fn frame_name(doc: &Document, f: &Frame) -> String {
    f.label.clone().unwrap_or_else(|| {
        if f.implicit { doc.title.clone().unwrap_or_else(|| "Main".into()) } else { f.id.to_string() }
    })
}

/// `a`, `a or b`, `a, b or c`.
fn or_join(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}
