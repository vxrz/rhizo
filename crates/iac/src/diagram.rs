//! The diagram of a root module: one `infra.environment` frame, a node per
//! resource, data source and module call, the other blocks verbatim on the
//! frame, and an edge per reference between nodes. Nodes whose code rhizo
//! recognizes are folded into neutral types (`rhizo_kit::infra::fold`): they
//! keep their code and gain the props it states, typed edges and nesting.

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use rhizo_core::{Direction, EdgeKind, KitRef, LayoutKind};
use rhizo_kit::infra::fold::{self, Env};
use rhizo_kit::infra::{ENVIRONMENT, Link, MODULE, RESOURCE};
use rhizo_lang::ast::{EdgeDef, File, FrameDef, NodeDef};
use serde_json::{Value, json};

use crate::hcl::{Block, Source};

/// A frame or node id from a folder name: `infra/prod` → `prod`.
pub fn frame_id(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() { "root".into() } else { out.into() }
}

/// What each construct became, for the report.
pub type Placed = Vec<(String, String)>;

/// Builds the diagram. `plan` is `tofu show -json` of the original: its
/// `configuration` says what OpenTofu loaded (each must be in the diagram)
/// and which blocks refer to which. Also returns the report's summary of
/// the folding.
pub fn build(name: &str, src: &Source, plan: &Value) -> Result<(File, Placed, Vec<String>), Vec<String>> {
    let id = frame_id(name);
    let mut placed = Placed::new();
    let mut blocks = Vec::new();
    let mut nodes = Vec::new();
    // Index into `placed` of each node's line.
    let mut lines = Vec::new();
    for b in &src.blocks {
        let Some(address) = b.address() else {
            let mut v = json!({ "type": b.kind });
            if !b.labels.is_empty() {
                v["labels"] = json!(b.labels);
            }
            v["file"] = json!(b.file);
            if !b.body.is_empty() {
                v["body"] = Value::Object(b.body.clone());
            }
            blocks.push(v);
            placed.push((b.title(), "environment block, verbatim".into()));
            continue;
        };
        nodes.push(node(b, &address));
        lines.push(placed.len());
        let ty = if b.kind == "module" { MODULE } else { RESOURCE };
        placed.push((b.title(), format!("node {address} ({ty}, verbatim)")));
    }

    let config = &plan["configuration"]["root_module"];
    let mut missing = Vec::new();
    let has = |a: &str| nodes.iter().any(|n: &NodeDef| n.id.as_deref() == Some(a));
    for r in config["resources"].as_array().into_iter().flatten() {
        let a = r["address"].as_str().unwrap_or_default();
        if !has(a) {
            missing.push(format!("{a}: OpenTofu loads it, but it is not in the diagram"));
        }
    }
    for m in config["module_calls"].as_object().into_iter().flatten().map(|(k, _)| format!("module.{k}")) {
        if !has(&m) {
            missing.push(format!("{m}: OpenTofu loads it, but it is not in the diagram"));
        }
    }
    for (kind, key) in [("variable", "variables"), ("output", "outputs")] {
        for name in config[key].as_object().into_iter().flatten().map(|(k, _)| k) {
            if !src.blocks.iter().any(|b| b.kind == kind && b.labels == [name.as_str()]) {
                missing.push(format!("{kind} \"{name}\": OpenTofu loads it, but it is not in the diagram"));
            }
        }
    }
    if !missing.is_empty() {
        return Err(missing);
    }

    let mut refs = BTreeSet::new();
    let calls = config["module_calls"].as_object().into_iter().flatten().map(|(k, v)| (format!("module.{k}"), v));
    let resources = config["resources"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| (r["address"].as_str().unwrap_or_default().to_string(), r));
    for (from, v) in resources.chain(calls) {
        let mut found = Vec::new();
        references(v, &mut found);
        for to in found.iter().filter_map(|r| fold::address(r)) {
            if to != from && has(&to) {
                refs.insert((from.clone(), to));
            }
        }
    }

    let Folding { edges, parents, mut summary } = fold_nodes(&mut nodes, &refs, &mut placed, &lines);
    let cloud = cloud(&nodes);
    if let Some((c, note)) = &cloud {
        summary.push(format!("cloud: {c} ({note})"));
    }
    let nodes = nest(nodes, &parents);

    let mut props = IndexMap::new();
    if let Some((c, _)) = cloud {
        props.insert("cloud".into(), json!(c));
    }
    props.insert("origin".into(), json!({ "dir": name }));
    if !blocks.is_empty() {
        props.insert("blocks".into(), Value::Array(blocks));
    }
    if let Some(lock) = &src.lock {
        props.insert("lock-file".into(), json!(lock));
        placed.push((crate::hcl::LOCK.into(), "environment lock-file, verbatim".into()));
    }
    let frame = FrameDef {
        label: Some(name.to_string()),
        id: Some(id),
        ty: Some(ENVIRONMENT.into()),
        layout: Some(LayoutKind::Layered),
        direction: Some(Direction::Right),
        props,
        nodes,
        edges,
        ..Default::default()
    };
    let file = File {
        rhizo: Some(2),
        title: Some(format!("{name} (imported from OpenTofu)")),
        kits: vec![KitRef { kit: "infra".into(), version: Some("0.1".into()) }],
        frames: vec![frame],
        ..Default::default()
    };
    Ok((file, placed, summary))
}

/// Folds every node whose code rhizo recognizes into its neutral type, and
/// turns references into edges: typed ones between folded nodes (with
/// `access` where IAM says it), dependencies otherwise.
fn fold_nodes(
    nodes: &mut [NodeDef],
    refs: &BTreeSet<(String, String)>,
    placed: &mut Placed,
    lines: &[usize],
) -> Folding {
    let props: Vec<_> = nodes.iter().map(|n| n.props.clone()).collect();
    let env = Env::new(props.iter());
    assert_eq!(env.blocks.len(), nodes.len(), "every imported node has an address");
    let folds = env.folds();
    let folded_edges = env.edges(&folds);
    let parents: Vec<Option<usize>> = (0..nodes.len()).map(|i| env.parent(i, &folds)).collect();
    let index: BTreeMap<String, usize> =
        nodes.iter().enumerate().map(|(i, n)| (n.id.clone().unwrap_or_default(), i)).collect();
    let ancestor = |a: usize, b: usize| {
        let mut p = parents[a];
        while let Some(x) = p {
            if x == b {
                return true;
            }
            p = parents[x];
        }
        false
    };
    let nested = |a: usize, b: usize| ancestor(a, b) || ancestor(b, a);

    // Edges: references with a generic end stay dependencies; between two
    // folded nodes the code says what they mean.
    // Sorted by address, like the nodes' ids: stable across re-imports.
    let mut edges: BTreeMap<(String, String), EdgeDef> = BTreeMap::new();
    for (a, b) in refs {
        let (Some(&i), Some(&j)) = (index.get(a), index.get(b)) else { continue };
        if folds[i].is_some() && folds[j].is_some() || nested(i, j) {
            continue;
        }
        edges.insert((a.clone(), b.clone()), EdgeDef { kind: Some(EdgeKind::Dependency), ..EdgeDef::new(a, b) });
    }
    let mut no_access = 0;
    for e in folded_edges.iter().filter(|e| !nested(e.from, e.to)) {
        let to_ty = folds[e.to].as_ref().map_or("", |f| f.ty);
        let mut def = EdgeDef {
            kind: Some(fold::edge_kind(e.link, to_ty)),
            ..EdgeDef::new(nodes[e.from].id.clone().unwrap_or_default(), nodes[e.to].id.clone().unwrap_or_default())
        };
        match (e.link, e.access) {
            (Some(Link::Grant), Some(a)) => {
                def.props.insert("access".into(), json!(a));
            }
            (Some(Link::Grant), None) => no_access += 1,
            _ => {}
        }
        edges.insert((def.from.clone(), def.to.clone()), def);
    }

    let mut summary = Vec::new();
    let mut unread = Vec::new();
    for (i, f) in folds.iter().enumerate() {
        let Some(f) = f else { continue };
        let n = &mut nodes[i];
        n.ty = Some(f.ty.to_string());
        for (k, v) in &f.props {
            n.props.insert(k.clone(), v.clone());
        }
        let id = n.id.clone().unwrap_or_default();
        let mut what = format!("node {id} ({}, folded", f.ty);
        if !f.unread.is_empty() {
            what += &format!("; not read: {}", f.unread.join(", "));
            unread.extend(f.unread.iter().map(|p| format!("{id} {p}")));
        }
        placed[lines[i]].1 = what + ")";
        for s in &f.satellites {
            if let Some(&j) = index.get(s) {
                let ty = if nodes[j].ty.as_deref() == Some(MODULE) { MODULE } else { RESOURCE };
                placed[lines[j]].1 = format!("node {s} ({ty}, verbatim; folded into {id})");
            }
        }
    }
    let total = env.blocks.iter().filter(|b| b.mode == "managed").count();
    let folded = folds.iter().filter(|f| f.is_some()).count();
    let satellites: BTreeSet<&String> = folds.iter().flatten().flat_map(|f| &f.satellites).collect();
    summary.push(coverage(folded, satellites.len(), total));
    if !unread.is_empty() {
        summary.push(format!("not read (set by expressions, or out of the kit's range): {}", unread.join(", ")));
    }
    if no_access > 0 {
        summary.push(format!("data edges without access (no IAM statement, role or binding says it): {no_access}"));
    }
    Folding { edges: edges.into_values().collect(), parents, summary }
}

/// What [`fold_nodes`] makes of the nodes.
struct Folding {
    edges: Vec<EdgeDef>,
    /// Each node's parent (where its code places it).
    parents: Vec<Option<usize>>,
    summary: Vec<String>,
}

/// The report's coverage line: resources and module calls (not data
/// sources) that folded, or whose meaning folded into another node.
pub fn coverage(folded: usize, satellites: usize, total: usize) -> String {
    let pct = ((folded + satellites) * 100 + total / 2).checked_div(total).unwrap_or(100);
    format!(
        "semantic coverage: {pct}% ({folded} folded and {satellites} satellites of {total} resources and module calls)"
    )
}

/// The cloud of an environment: the one whose providers most of its nodes
/// use. More than one is noted, not refused.
fn cloud(nodes: &[NodeDef]) -> Option<(&'static str, String)> {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for n in nodes {
        let text = |k: &str| n.props.get(k).and_then(Value::as_str);
        let provider = match (text("tf-type"), text("source")) {
            (Some(t), _) => t.split('_').next().unwrap_or_default().to_string(),
            // namespace/name/provider[//submodule], after an optional registry host.
            (None, Some(s)) => {
                let parts: Vec<&str> = s.split("//").next().unwrap_or(s).split('/').collect();
                parts.get(parts.len().saturating_sub(1)).copied().unwrap_or_default().to_string()
            }
            _ => continue,
        };
        let cloud = match provider.as_str() {
            "aws" => "aws",
            "azurerm" | "azapi" => "azure",
            "google" => "gcp",
            _ => continue,
        };
        *count.entry(cloud).or_default() += 1;
    }
    let best = ["aws", "azure", "gcp"]
        .into_iter()
        .max_by_key(|c| (count.get(c).copied().unwrap_or(0), std::cmp::Reverse(*c)))?;
    let n = *count.get(best)?;
    let others: Vec<String> = count.iter().filter(|(c, _)| **c != best).map(|(c, k)| format!("{c} {k}")).collect();
    let note = if others.is_empty() {
        "from the providers its resources use".to_string()
    } else {
        format!("warning: more than one cloud; chose the majority, {best} {n} against {}", others.join(", "))
    };
    Some((best, note))
}

/// Puts each node inside its parent (children keep their order).
fn nest(nodes: Vec<NodeDef>, parents: &[Option<usize>]) -> Vec<NodeDef> {
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = parents[i] {
            d += 1;
            i = p;
        }
        d
    };
    let mut slots: Vec<Option<NodeDef>> = nodes.into_iter().map(Some).collect();
    let mut order: Vec<usize> = (0..slots.len()).collect();
    order.sort_by_key(|i| std::cmp::Reverse(depth(*i)));
    for i in order {
        if let Some(p) = parents[i] {
            let child = slots[i].take().expect("each node is nested once");
            slots[p].as_mut().expect("parents are nested after their children").children.push(child);
        }
    }
    slots.into_iter().flatten().collect()
}

fn node(b: &Block, address: &str) -> NodeDef {
    let mut props = IndexMap::new();
    let name = b.labels.last().cloned().unwrap_or_default();
    let shape = match b.kind.as_str() {
        "module" => {
            // Literal source and version are props; anything else stays in the body.
            let mut body = b.body.clone();
            for key in ["source", "version"] {
                if let Some(v) = b.literal(key) {
                    body.shift_remove(key);
                    props.insert(key.to_string(), json!(v));
                }
            }
            props.insert("name".into(), json!(name));
            if !body.is_empty() {
                props.insert("body".into(), Value::Object(body));
            }
            "module"
        }
        kind => {
            props.insert("tf-type".into(), json!(b.labels[0]));
            match kind {
                "data" => props.insert("mode".into(), json!("data")),
                "ephemeral" => props.insert("mode".into(), json!("ephemeral")),
                _ => None,
            };
            props.insert("name".into(), json!(name));
            if !b.body.is_empty() {
                props.insert("body".into(), Value::Object(b.body.clone()));
            }
            if kind == "resource" { "resource" } else { kind }
        }
    };
    props.insert("origin".into(), json!({ "address": address, "shape": shape, "file": b.file }));
    NodeDef {
        id: Some(address.to_string()),
        ty: Some(if b.kind == "module" { MODULE } else { RESOURCE }.into()),
        props,
        ..NodeDef::new(name)
    }
}

/// Every reference in a block's `configuration` entry, but not in the
/// configuration of a called module.
fn references(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                match (k.as_str(), v) {
                    ("module", _) => {}
                    ("references" | "depends_on", Value::Array(a)) => {
                        out.extend(a.iter().filter_map(Value::as_str).map(str::to_string))
                    }
                    _ => references(v, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|v| references(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_point_at_nodes() {
        let target = fold::address;
        assert_eq!(target("aws_s3_bucket.b[\"x.y\"].arn").as_deref(), Some("aws_s3_bucket.b"));
        assert_eq!(target("data.aws_iam_policy_document.p.json").as_deref(), Some("data.aws_iam_policy_document.p"));
        assert_eq!(target("module.vpc.private_subnets").as_deref(), Some("module.vpc"));
        assert_eq!(target("var.region"), None);
        assert_eq!(target("aws_s3_bucket"), None);
        assert_eq!(frame_id("My Stack/"), "my-stack");
    }
}
