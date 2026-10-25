//! The `infra` kit compiler: each `infra.environment` frame becomes one
//! OpenTofu root module in `infra/<env>/` (design doc §3.7, §4.3, §5).
//!
//! - `versions.tf`: OpenTofu and providers pinned exactly, the S3 backend.
//! - `encryption.tofu`: state and plan encryption. It is OpenTofu-only
//!   syntax, so it lives in a `.tofu` file that TFLint and Checkov skip.
//! - `providers.tf`, `main.tf` (registry modules pinned to exact versions),
//!   `iam.tf` (least-privilege policies from data edges), `network.tf`
//!   (security groups from edges to databases), `variables.tf` (everything
//!   environment-specific, without defaults) and `outputs.tf`.
//!
//! Addresses come from node ids (`orders-db` → `module.orders_db`), output
//! is in `tofu fmt` canonical form, and every module carries a
//! `rhizo:node` tag and comment.
//!
//! **Preserve mode.** An environment imported from OpenTofu code (it has an
//! `origin`) compiles back as that root module: its verbatim `blocks`, then
//! one block per `infra.resource` / `infra.module` node (or node folded from
//! its code, docs/design/iac-import.md phase 2) at its address, each
//! in the file it came from (docs/design/iac-import.md).

use std::collections::{BTreeMap, BTreeSet};

use rhizo_core::{Document, Edge, EdgeKind, Frame, Node};
use rhizo_kit::Finding;
use rhizo_kit::data::OPERATION;
use rhizo_kit::infra::{
    self as k, BUCKET, ENVIRONMENT, FUNCTION, HTTP_API, KEY, Link, POSTGRES, QUEUE, SUBNET, TABLE, TOPIC, VPC,
};
use serde_json::Value;

use crate::brief::{Criterion, Task, TaskOut};
use crate::target::{Cx, Generated, Target};
use crate::{Files, OutFile};
use rhizo_kit::infra::adopt;

/// The catalog: exact versions, checked against registry.opentofu.org on
/// 2026-10-21 (latest release of each).
pub(crate) const OPENTOFU: &str = ">= 1.11.1, < 2.0.0"; // the rds module needs 1.11.1
const AWS: &str = "6.69.1";
const EXTERNAL: &str = "2.4.2"; // the lambda module needs external, local and null
const LOCAL: &str = "2.9.1";
const NULL: &str = "3.3.2";
const RANDOM: &str = "3.9.1"; // the rds module needs random
fn module(ty: &str) -> (&'static str, &'static str) {
    match ty {
        VPC => ("terraform-aws-modules/vpc/aws", "6.7.3"),
        BUCKET => ("terraform-aws-modules/s3-bucket/aws", "5.17.0"),
        TABLE => ("terraform-aws-modules/dynamodb-table/aws", "5.5.2"),
        POSTGRES => ("terraform-aws-modules/rds/aws", "7.2.2"),
        QUEUE => ("terraform-aws-modules/sqs/aws", "5.2.2"),
        TOPIC => ("terraform-aws-modules/sns/aws", "7.2.0"),
        KEY => ("terraform-aws-modules/kms/aws", "4.2.2"),
        FUNCTION => ("terraform-aws-modules/lambda/aws", "8.12.0"),
        HTTP_API => ("terraform-aws-modules/apigateway-v2/aws", "6.1.1"),
        _ => unreachable!("only compiled types have modules"),
    }
}

/// Types the compiler covers completely: nothing is left to build by hand.
/// A Lambda function's infrastructure is generated, but its code is a gap.
fn generated_only(ty: &str) -> bool {
    k::is_infra(ty) && ty != FUNCTION
}

/// The Lambda function a `data.operation` is implemented by: a dependency
/// edge from the function to it.
fn implementer<'a>(doc: &'a Document, op: &Node) -> Option<&'a Node> {
    if k::ty(op) != OPERATION {
        return None;
    }
    doc.edges
        .iter()
        .filter(|e| e.to == op.id && e.kind.is_none_or(|k| k == EdgeKind::Dependency))
        .find_map(|e| doc.nodes.get(&e.from).filter(|f| k::ty(f) == FUNCTION))
}

/// One OpenTofu root module per environment under `infra/`.
pub(crate) struct Infra;

impl Target for Infra {
    fn kit(&self) -> &'static str {
        k::KIT
    }

    fn dir(&self) -> &'static str {
        "infra"
    }

    fn generated(&self) -> Generated {
        Generated::Infra
    }

    fn emit(&self, cx: &Cx, files: &mut Files) -> Result<(), Vec<Finding>> {
        emit(cx.doc, cx.source_name, files);
        Ok(())
    }

    /// Infrastructure is generated, except function code; an operation a
    /// function implements is built as part of that function. Imported code
    /// (with its functions) exists already.
    fn covers(&self, doc: &Document, n: &Node) -> bool {
        generated_only(k::ty(n))
            || implementer(doc, n).is_some()
            || (k::is_infra(k::ty(n)) && n.props.contains_key("origin"))
    }

    fn parts<'a>(&self, doc: &'a Document, n: &Node) -> Vec<&'a Node> {
        if k::ty(n) == FUNCTION { k::operations(doc, n) } else { Vec::new() }
    }

    fn implement(&self, n: &Node) -> Option<String> {
        (k::ty(n) == FUNCTION).then(|| format!("Implement the function \"{}\"", n.label))
    }

    fn criteria(&self, doc: &Document, _files: &Files, t: &Task, inputs: &[String]) -> Vec<Criterion> {
        if k::ty(t.node) != FUNCTION || t.flow.is_some() {
            return Vec::new();
        }
        let mut out: Vec<Criterion> = t
            .nodes
            .iter()
            .filter(|n| k::ty(n) == OPERATION)
            .flat_map(|op| crate::data::operation_criteria(doc, inputs, op, true))
            .collect();
        out.extend(function_criteria(doc, t.node));
        out
    }

    fn questions(&self, doc: &Document, _decided: &dyn Fn(&[&str]) -> bool) -> Vec<String> {
        cloud_questions(doc)
    }

    /// One human task per environment: set its variables and apply it,
    /// after every task that builds something in it (design doc §4.6).
    fn human_tasks<'a>(&self, doc: &'a Document, files: &Files, order: &[&Task<'a>]) -> Vec<TaskOut<'a>> {
        let envs = doc.frames.iter().filter(|f| f.ty.as_ref().is_some_and(|t| t.as_str() == ENVIRONMENT));
        envs.map(|f| {
            let dir = dir(f);
            let deps: Vec<String> =
                order.iter().filter(|t| t.nodes.iter().any(|n| n.frame == f.id)).map(|t| t.id()).collect();
            let inputs: Vec<String> = files.keys().filter(|p| p.starts_with(&format!("{dir}/"))).cloned().collect();
            let first = if k::imported(f) {
                let vars: Vec<String> = verbatim_blocks(f)
                    .filter(|b| text(b, "type") == Some("variable"))
                    .filter(|b| b.get("body").and_then(|x| x.get("default")).is_none())
                    .filter_map(|b| Some(format!("`{}`", b.get("labels")?.get(0)?.as_str()?)))
                    .collect();
                format!("Every variable without a default has a value outside the bundle (CI variables or a `.tfvars` file that is not committed): {}", vars.join(", "))
            } else {
                let blocks = if k::cloud(f) == "azure" {
                    azure::Az::new(doc, f, "").variable_blocks()
                } else if k::cloud(f) == "gcp" {
                    gcp::Gcp::new(doc, f, "").variable_blocks()
                } else {
                    Env::new(doc, f, "").variable_blocks()
                };
                let vars: Vec<String> = blocks.into_iter().map(|(v, _)| format!("`{v}`")).collect();
                format!("Every variable in `{dir}/variables.tf` has a value outside the bundle (CI variables or a `.tfvars` file that is not committed): {}", vars.join(", "))
            };
            let azure = !k::imported(f) && k::cloud(f) == "azure";
            let gcp = !k::imported(f) && k::cloud(f) == "gcp";
            let mut acceptance = vec![Criterion::new("infra", first)];
            if azure {
                acceptance.push(Criterion::new("infra", "`az login` and `az account set --subscription <subscription_id>` ran first; the signed-in identity may create role assignments and custom roles (Owner, or Contributor plus Role Based Access Control Administrator)"));
            }
            if gcp {
                acceptance.push(Criterion::new("infra", "`gcloud auth application-default login` ran first, for an existing project (`project_id`; rhizo creates no projects); the signed-in account may enable APIs, create service accounts and set IAM policies on the project and its resources (Owner, or Editor plus Security Admin and Service Account User)"));
                if f.props.contains_key("budget-monthly") {
                    acceptance.push(Criterion::new("infra", "The signed-in account may create budgets on the billing account `billing_account_id` (Billing Account Costs Manager)"));
                }
            }
            acceptance.extend([
                Criterion::new("infra", format!("`tofu init`, `tofu validate` and `tofu fmt -check` pass in `{dir}`")),
                Criterion::new("infra", "A person reviewed `tofu plan` before `tofu apply`; the plan creates only what the diagram shows"),
            ]);
            let adopted: Vec<String> = doc
                .nodes
                .values()
                .filter(|n| n.frame == f.id && k::imported(f) && adopt::adopted(&n.props))
                .map(|n| format!("`{}`", n.id))
                .collect();
            if !adopted.is_empty() {
                acceptance.push(Criterion::new("infra", format!("Adopted nodes ({}) are rhizo's code in `{dir}/{ADOPTED_FILE}`, with `moved` blocks from the code they came from: `tofu plan` against the real state shows only those moves, the in-place updates and the creates the adopt report lists; any replace or destroy is a stop", adopted.join(", "))));
            }
            if let Some(b) = f.props.get("state-bucket").and_then(Value::as_str).filter(|_| !azure && !gcp) {
                acceptance.push(Criterion::new("infra", format!("State is stored encrypted in the `{b}` bucket (`encryption.tofu`), never on a laptop")));
            }
            if let Some((bucket, _)) = k::gcp_state(f).filter(|_| gcp) {
                acceptance.push(Criterion::new("infra", format!("State is stored encrypted with the `state_kms_key` key in the `{bucket}` bucket (`encryption.tofu`), never on a laptop")));
            }
            if let Some((account, container, _)) = k::azure_state(f).filter(|_| azure) {
                acceptance.push(Criterion::new("infra", format!("State is stored in container `{container}` of the `{account}` storage account (Entra ID auth, encrypted at rest by Azure Storage), never on a laptop")));
            }
            let functions: Vec<String> = doc
                .nodes
                .values()
                .filter(|n| azure && n.frame == f.id && k::ty(n) == FUNCTION)
                .map(|n| format!("`{}_function_name`", addr(n.id.as_str())))
                .collect();
            if !functions.is_empty() {
                acceptance.push(Criterion::new("infra", format!("After apply, each function's code is published to its Function App (outputs {})", functions.join(", "))));
            }
            let label = f.label.clone().unwrap_or_else(|| f.id.to_string());
            TaskOut {
                id: format!("T-{}-apply", f.id),
                title: format!("Apply the {label} environment"),
                kind: "human",
                node: None,
                area: None,
                frame: Some(f.id.as_str()),
                depends_on: deps,
                deploys: None,
                extras: Default::default(),
                type_description: None,
                scope: Vec::new(),
                connections: None,
                inputs,
                goals: None,
                decisions: None,
                acceptance,
            }
        })
        .collect()
    }
}

/// What an azure or gcp environment leaves open: the gaps the rules warn
/// about, and the network exposure of its stores.
fn cloud_questions(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    for f in doc.frames.iter().filter(|f| f.ty.as_ref().is_some_and(|t| t.as_str() == ENVIRONMENT)) {
        let gaps = match k::cloud(f) {
            _ if k::imported(f) => continue,
            "azure" => k::azure::gaps(doc, f),
            "gcp" => k::gcp::gaps(doc, f),
            _ => continue,
        };
        for g in gaps.into_iter().filter(|g| !g.error) {
            let msg = g.message.split_once("] ").map_or(g.message.as_str(), |(_, m)| m);
            out.push(format!(
                "{msg}. Is that acceptable?{}",
                g.help.map_or(String::new(), |h| format!(" If not: {h}."))
            ));
        }
        let stores =
            doc.nodes.values().any(|n| n.frame == f.id && matches!(k::ty(n), BUCKET | TABLE | QUEUE | TOPIC | KEY));
        if stores && k::cloud(f) == "gcp" {
            out.push(format!(
                "In environment `{}`, Cloud Storage, Firestore, Pub/Sub and Cloud KMS accept IAM-authorized requests from any network, as on AWS; do they need a VPC Service Controls perimeter instead?",
                f.id
            ));
        } else if stores {
            out.push(format!(
                "In environment `{}`, storage accounts, Cosmos DB, Service Bus and Key Vault accept Entra ID requests from any network, as on AWS; do they need private endpoints instead?",
                f.id
            ));
        }
    }
    out
}

/// What the generated infrastructure expects of a function's code.
fn function_criteria(doc: &Document, f: &Node) -> Vec<Criterion> {
    let Some(frame) = k::environment(doc, f) else { return Vec::new() };
    if k::cloud(frame) == "azure" && !k::imported(frame) {
        return azure::function_criteria(doc, frame, f);
    }
    if k::cloud(frame) == "gcp" && !k::imported(frame) {
        return gcp::function_criteria(doc, frame, f);
    }
    let dir = dir(frame);
    let text = |key: &str, d: &str| k::text(f, key).unwrap_or(d).to_string();
    let mut out = vec![Criterion::new(
        "infra",
        format!(
            "Builds into a zip for runtime `{}` with handler `{}`; `var.{}_package` in `{dir}/variables.tf` points at it",
            text("runtime", "nodejs22.x"),
            text("handler", "index.handler"),
            addr(f.id.as_str())
        ),
    )];
    let env = function_env(doc, f);
    if !env.is_empty() {
        let names: Vec<String> = env.iter().map(|v| format!("`{v}`")).collect();
        out.push(Criterion::new("infra", format!(
            "Finds its resources through the environment variables set in `{dir}/main.tf` ({}), never hard-coded names",
            names.join(", ")
        )));
    }
    for (_, _, a, b, l) in k::links(doc) {
        match l {
            Link::Grant if a.id == f.id && k::ty(b) == POSTGRES => out.push(Criterion::new("infra", format!(
                "Connects to `{}` over TLS as database user `{}` with IAM authentication (an RDS auth token, no password); a migration creates that user and grants it `rds_iam`",
                b.id, db_user(f)
            ))),
            Link::Trigger if b.id == f.id => out.push(Criterion::new("infra", format!(
                "Processes message batches from `{}` and reports failed messages in `batchItemFailures`, so only those are retried",
                a.id
            ))),
            Link::Subscribe if b.id == f.id => out.push(Criterion::new("infra", format!(
                "Handles the notifications of `{}` idempotently (SNS may deliver a message more than once)",
                a.id
            ))),
            _ => {}
        }
    }
    if k::links(doc).any(|(_, _, a, _, l)| a.id == f.id && l == Link::Grant) {
        out.push(Criterion::new(
            "infra",
            format!(
                "Needs no permissions beyond those granted in `{dir}/iam.tf`; a missing one is a change to the diagram"
            ),
        ));
    }
    out
}

fn emit(doc: &Document, source_name: &str, files: &mut Files) {
    for f in doc.frames.iter().filter(|f| f.ty.as_ref().is_some_and(|t| t.as_str() == ENVIRONMENT)) {
        if k::imported(f) {
            preserve(doc, f, source_name, files);
        } else if k::cloud(f) == "azure" {
            azure::Az::new(doc, f, source_name).emit(files);
        } else if k::cloud(f) == "gcp" {
            gcp::Gcp::new(doc, f, source_name).emit(files);
        } else {
            Env::new(doc, f, source_name).emit(files);
        }
    }
}

// --- Verbatim HCL: imported environments and generic nodes -----------------

fn text<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

/// An environment's verbatim blocks (`terraform`, `variable`, …).
fn verbatim_blocks(f: &Frame) -> impl Iterator<Item = &Value> {
    f.props.get("blocks").and_then(Value::as_array).into_iter().flatten()
}

/// A block label: a quoted string literal. Labels hold no templates, so
/// only quotes, backslashes and line breaks need escaping.
fn label(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r"))
}

/// A `body` prop as HCL: strings are expressions as written, arrays the
/// bodies of nested blocks named by their key.
fn verbatim(body: Option<&Value>) -> Vec<Item> {
    let mut out = Vec::new();
    for (key, v) in body.and_then(Value::as_object).into_iter().flatten() {
        match v {
            Value::String(e) => out.push(a(key, e.clone())),
            Value::Array(blocks) => out.extend(blocks.iter().map(|b| block(key.clone(), verbatim(Some(b))))),
            _ => {} // the schema allows nothing else
        }
    }
    out
}

/// The address and block of an `infra.resource` or `infra.module` node, or
/// of a node folded from imported code, plus a `moved` block when it was
/// imported from another address.
fn generic(n: &Node) -> Vec<Item> {
    // Read as written: a folded node's neutral props are views of this code.
    let own = |key: &str| n.props.get(key).and_then(Value::as_str);
    let name = own("name").map_or_else(|| addr(n.id.as_str()), str::to_string);
    let mut items = Vec::new();
    let (header, address) = if !n.props.contains_key("tf-type") {
        for key in ["source", "version"] {
            if let Some(v) = own(key) {
                items.push(a(key, q(v)));
            }
        }
        (format!("module {}", label(&name)), format!("module.{name}"))
    } else {
        let ty = own("tf-type").unwrap_or_default();
        match own("mode") {
            Some("data") => (format!("data {} {}", label(ty), label(&name)), format!("data.{ty}.{name}")),
            Some("ephemeral") => {
                (format!("ephemeral {} {}", label(ty), label(&name)), format!("ephemeral.{ty}.{name}"))
            }
            _ => (format!("resource {} {}", label(ty), label(&name)), format!("{ty}.{name}")),
        }
    };
    items.extend(verbatim(n.props.get("body")));
    let mut out = vec![comment(format!("rhizo:node={}", n.id)), block(header, items)];
    // Renamed since the import: move what the old address manages (data
    // sources and ephemeral resources keep nothing in state).
    let from = n.props.get("origin").and_then(|o| text(o, "address")).filter(|f| *f != address);
    if let Some(from) = from.filter(|f| !f.starts_with("data.") && !f.starts_with("ephemeral.")) {
        out.extend([Item::Blank, block("moved", vec![a("from", from), a("to", address)])]);
    }
    out
}

/// The file an imported block goes back into: the one it came from, when
/// that is a plain `.tf`/`.tofu` name that is not an override file.
fn source_file(v: Option<&str>) -> String {
    let plain = |f: &&str| {
        let stem = f.trim_end_matches(".tf").trim_end_matches(".tofu");
        (f.ends_with(".tf") || f.ends_with(".tofu"))
            && !stem.is_empty()
            && !f.starts_with('.')
            && !stem.ends_with("override")
            && f.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    v.filter(plain).unwrap_or("main.tf").to_string()
}

/// An imported environment, compiled back as the root module it came from.
fn preserve(doc: &Document, f: &Frame, source_name: &str, files: &mut Files) {
    let header = format!(
        "@generated by rhizo {} from {} (environment {}, imported). Do not edit: change the diagram and recompile.",
        crate::VERSION,
        crate::one_line(source_name),
        crate::one_line(f.id.as_str())
    );
    let mut out: BTreeMap<String, (Vec<Vec<Item>>, Vec<String>)> = BTreeMap::new();
    for b in verbatim_blocks(f) {
        let labels = b.get("labels").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
        let head: Vec<String> =
            std::iter::once(text(b, "type").unwrap_or_default().to_string()).chain(labels.map(label)).collect();
        out.entry(source_file(text(b, "file")))
            .or_default()
            .0
            .push(vec![block(head.join(" "), verbatim(b.get("body")))]);
    }
    // Generic nodes, and folded ones: typed views of the same verbatim code.
    let code = |n: &Node| {
        (k::is_generic(k::ty(n)) || (k::is_infra(k::ty(n)) && n.props.contains_key("origin")))
            && !adopt::adopted(&n.props)
    };
    for n in doc.nodes.values().filter(|n| n.frame == f.id && code(n)) {
        let file = source_file(n.props.get("origin").and_then(|o| text(o, "file")));
        let entry = out.entry(file).or_default();
        entry.0.push(generic(n));
        entry.1.push(n.id.to_string());
    }
    // Adopted nodes: rhizo's code, with moves from what they came from.
    let adopted = adopted_code(doc, f);
    if !adopted.is_empty() {
        let entry = out.entry(ADOPTED_FILE.into()).or_default();
        for (id, groups) in adopted {
            entry.0.extend(groups);
            entry.1.push(id);
        }
    }
    for (name, (groups, mut nodes)) in out {
        nodes.sort();
        files.insert(format!("{}/{name}", dir(f)), OutFile { contents: file(&header, groups), nodes });
    }
    if let Some(lock) = f.props.get("lock-file").and_then(Value::as_str) {
        files.insert(
            format!("{}/.terraform.lock.hcl", dir(f)),
            OutFile { contents: lock.to_string(), nodes: Vec::new() },
        );
    }
}

/// The file adopted nodes compile into, in an imported environment.
pub const ADOPTED_FILE: &str = "rhizo.tf";

/// The imported environment `f` as the cloud emitters see it: adopted
/// nodes, and module calls folded from imported code, go by the name of
/// their code (`module.orders_db` → `orders-db`), so rhizo's code refers to
/// them at `module.orders_db`. Every other node keeps its id (an address,
/// which the emitters never name). Only `f` and its nodes are in it.
fn view(doc: &Document, f: &Frame) -> Document {
    let renamed = |n: &Node| -> Option<String> {
        let name = adopt::code_name(&n.props)?;
        adopt::rhizo_shaped(n, k::cloud(f)).then(|| name.replace('_', "-"))
    };
    let mut ids: BTreeMap<&rhizo_core::NodeId, rhizo_core::NodeId> = BTreeMap::new();
    let mut taken: BTreeSet<rhizo_core::NodeId> =
        doc.nodes.values().filter(|n| n.frame == f.id).map(|n| n.id.clone()).collect();
    for n in doc.nodes.values().filter(|n| n.frame == f.id) {
        // Two nodes of one name: the second keeps its id (and rhizo's code cannot refer to it).
        let id = renamed(n).map(|r| rhizo_core::NodeId::from(r.as_str())).filter(|r| taken.insert(r.clone()));
        ids.insert(&n.id, id.unwrap_or_else(|| n.id.clone()));
    }
    let mut out = Document { frames: vec![f.clone()], kits: doc.kits.clone(), ..Document::default() };
    for n in doc.nodes.values().filter(|n| n.frame == f.id) {
        let mut v = n.clone();
        v.id = ids[&n.id].clone();
        v.parent = n.parent.as_ref().and_then(|p| ids.get(p)).cloned();
        v.children = n.children.iter().filter_map(|c| ids.get(c)).cloned().collect();
        out.nodes.insert(v.id.clone(), v);
    }
    for e in &doc.edges {
        if let (Some(from), Some(to)) = (ids.get(&e.from), ids.get(&e.to)) {
            out.edges.push(Edge { from: from.clone(), to: to.clone(), ..e.clone() });
        }
    }
    out
}

/// A top-level block's address: `module "x"` → `module.x`,
/// `resource "t" "n"` → `t.n`, `data "t" "n"` → `data.t.n`.
fn block_address(header: &str) -> Option<String> {
    let parts: Vec<&str> = header.split(' ').map(|p| p.trim_matches('"')).collect();
    Some(match parts.as_slice() {
        ["module", name] => format!("module.{name}"),
        ["resource", ty, name] => format!("{ty}.{name}"),
        [mode @ ("data" | "ephemeral"), ty, name] => format!("{mode}.{ty}.{name}"),
        _ => return None,
    })
}

/// rhizo's code for each adopted node of the imported environment `f`, by
/// node id: the blocks its cloud's emitter writes for it at the addresses
/// it stands for (`module.<name>`, where it came from, its satellites), its
/// kept arguments, and a `moved` block for each address it came from that
/// its code does not keep.
fn adopted_code(doc: &Document, f: &Frame) -> Vec<(String, Vec<Vec<Item>>)> {
    let nodes: Vec<&Node> = doc.nodes.values().filter(|n| n.frame == f.id && adopt::adopted(&n.props)).collect();
    if nodes.is_empty() {
        return Vec::new();
    }
    let view = view(doc, f);
    let frame = &view.frames[0];
    let mut out = Vec::new();
    for n in nodes {
        let moves = adopt::moves(&n.props);
        let Some(origin) = moves.first() else { continue };
        let label = adopt::code_name(&n.props).unwrap_or_default();
        let main = format!("module.{label}");
        let Some(v) = view.nodes.get(&rhizo_core::NodeId::from(label.replace('_', "-").as_str())) else { continue };
        let groups = match k::cloud(f) {
            "azure" => azure::Az::new(&view, frame, "").node_groups(v),
            "gcp" => gcp::Gcp::new(&view, frame, "").node_groups(v),
            _ => Env::new(&view, frame, "").node_groups(v),
        };
        let mut items: Vec<Vec<Item>> = Vec::new();
        let mut emitted = Vec::new();
        for item in groups.into_iter().flatten() {
            let Item::Block(header, mut body) = item else { continue };
            let Some(at) = block_address(&header).filter(|a| *a == main || moves.contains(a)) else { continue };
            if at == main || at == *origin {
                keep(&mut body, &adopt::keep(&n.props));
            }
            items.push(vec![comment(format!("rhizo:node={} (adopted)", n.id)), block(header, body)]);
            emitted.push(at);
        }
        for from in moves.iter().filter(|m| !emitted.contains(m)) {
            let Some(inside) = adopt::resource_type(from).and_then(adopt::inside) else { continue };
            items.push(vec![block("moved", vec![a("from", from.clone()), a("to", format!("{main}.{inside}"))])]);
        }
        out.push((n.id.to_string(), items));
    }
    // rhizo's code refers to helpers its own environments have; code of
    // another shape gets `rhizo_` ones, from its provider configuration.
    let mut used = BTreeSet::new();
    let helpers = helpers(doc, f);
    for (_, items) in &mut out {
        for group in items.iter_mut() {
            for (from, to, _) in &helpers {
                substitute(group, from, to, &mut used);
            }
        }
    }
    let defs: Vec<Item> =
        helpers.into_iter().filter(|(_, to, _)| used.contains(to)).filter_map(|(_, _, d)| d).collect();
    let (locals, others): (Vec<Item>, Vec<Item>) = defs.into_iter().partition(|i| matches!(i, Item::Attr(..)));
    let mut head: Vec<Vec<Item>> = Vec::new();
    if !locals.is_empty() {
        head.push(vec![
            comment("rhizo's code for adopted nodes: what it needs from this environment, under rhizo_ names"),
            block("locals", locals),
        ]);
    }
    head.extend(others.into_iter().map(|b| vec![b]));
    if let (false, Some((_, first))) = (head.is_empty(), out.first_mut()) {
        first.splice(0..0, head);
    }
    out
}

/// The helpers rhizo's code for a cloud refers to that the imported
/// environment `f` lacks: (reference, what it becomes, the definition to
/// add: a `locals` entry or a block). A reference the environment defines
/// (rhizo's own output, imported) stays.
fn helpers(doc: &Document, f: &Frame) -> Vec<(&'static str, String, Option<Item>)> {
    let locals: BTreeSet<&str> = verbatim_blocks(f)
        .filter(|b| text(b, "type") == Some("locals"))
        .flat_map(|b| b.get("body").and_then(Value::as_object).into_iter().flatten().map(|(k_, _)| k_.as_str()))
        .collect();
    let has_node = |address: &str| {
        doc.nodes.values().any(|n| {
            n.frame == f.id
                && n.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str) == Some(address)
        })
    };
    // The default (unaliased) provider configuration's argument.
    let provider = |name: &str, arg: &str| -> Option<String> {
        verbatim_blocks(f)
            .filter(|b| text(b, "type") == Some("provider"))
            .filter(|b| {
                b.get("labels").and_then(Value::as_array).and_then(|l| l.first()).and_then(Value::as_str) == Some(name)
            })
            .filter(|b| b.get("body").and_then(|x| x.get("alias")).is_none())
            .find_map(|b| b.get("body")?.get(arg)?.as_str().map(str::to_string))
    };
    let mut out = Vec::new();
    match k::cloud(f) {
        "gcp" => {
            let project = provider("google", "project");
            let suffix = project.as_ref().map(|p| format!("substr(sha1({p}), 0, 6)"));
            for (reference, name, value) in [
                ("local.project", "project", project),
                ("local.region", "region", provider("google", "region")),
                ("local.suffix", "suffix", suffix),
            ] {
                if !locals.contains(name) {
                    out.push((reference, format!("local.rhizo_{name}"), value.map(|v| a(&format!("rhizo_{name}"), v))));
                }
            }
        }
        "azure" => {
            if !has_node("data.azurerm_client_config.current") {
                out.push((
                    "data.azurerm_client_config.current",
                    "data.azurerm_client_config.rhizo_current".to_string(),
                    Some(block("data \"azurerm_client_config\" \"rhizo_current\"", vec![])),
                ));
            }
            if !locals.contains("tags") {
                out.push(("local.tags", "{}".to_string(), None));
            }
        }
        _ => {}
    }
    out
}

/// Replaces reference `from` (not a longer name it starts) in every
/// expression of `items` with `to`, noting `to` in `used` when it did.
fn substitute(items: &mut [Item], from: &str, to: &str, used: &mut BTreeSet<String>) {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let fix = |e: &mut String, used: &mut BTreeSet<String>| {
        let mut out = String::new();
        let mut rest = e.as_str();
        while let Some(i) = rest.find(from) {
            let before = rest[..i].chars().last().or_else(|| out.chars().last());
            let after = rest[i + from.len()..].chars().next();
            out += &rest[..i];
            if before.is_some_and(|c| ident(c) || c == '.') || after.is_some_and(ident) {
                out += from;
            } else {
                out += to;
                used.insert(to.to_string());
            }
            rest = &rest[i + from.len()..];
        }
        out += rest;
        *e = out;
    };
    for i in items {
        match i {
            Item::Attr(_, Val::E(e)) => fix(e, used),
            Item::Attr(_, Val::Obj(inner)) | Item::Block(_, inner) => substitute(inner, from, to, used),
            Item::Attr(_, Val::Objs(objs)) => objs.iter_mut().for_each(|o| substitute(o, from, to, used)),
            Item::Comment(_) | Item::Blank => {}
        }
    }
}

/// Puts an adopted node's kept arguments into rhizo's block for it, in
/// place of what rhizo writes for them (or its alternative: a kept
/// `bucket` replaces `bucket_prefix`). Keys rhizo's block lacks are not added.
fn keep(body: &mut [Item], kept: &[(&str, &str)]) {
    for (key, expr) in kept {
        // A path into one of rhizo's inputs: `keys.key.name`.
        if let Some((head, rest)) = key.split_once('.') {
            if let Some(Item::Attr(_, Val::Obj(inner))) =
                body.iter_mut().find(|i| matches!(i, Item::Attr(k_, _) if k_ == head))
            {
                keep(inner, &[(rest, expr)]);
            }
            continue;
        }
        let alt = adopt::ALTERNATIVES.iter().find_map(|(x, y)| match *key {
            k_ if k_ == *x => Some(*y),
            k_ if k_ == *y => Some(*x),
            _ => None,
        });
        let at = |want: &str| body.iter().position(|i| matches!(i, Item::Attr(k_, _) if k_ == want));
        if let Some(i) = at(key).or_else(|| alt.and_then(at)) {
            body[i] = a(key, expr.to_string());
        }
    }
}

/// The folder of an environment's root module.
fn dir(f: &Frame) -> String {
    environment_dir(f.id.as_str())
}

/// The folder of the root module of the environment with this frame id.
pub fn environment_dir(id: &str) -> String {
    format!("infra/{}", name(id))
}

/// `Orders DB` → `orders-db`-style AWS name part: lowercase, `-` between words.
fn name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// `orders-db` → `orders_db`, an OpenTofu identifier.
fn addr(id: &str) -> String {
    let s: String = id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    if s.starts_with(|c: char| c.is_ascii_alphabetic()) { s } else { format!("n_{s}") }
}

/// `orders-db` → `OrdersDb`, for statement ids.
fn pascal(id: &str) -> String {
    addr(id).split('_').filter(|w| !w.is_empty()).map(|w| w[..1].to_uppercase() + &w[1..]).collect()
}

/// `orders-db` → `ORDERS_DB`, for environment variables.
fn upper(id: &str) -> String {
    addr(id).trim_start_matches("n_").to_uppercase()
}

/// An HCL string literal.
fn q(s: &str) -> String {
    let mut out = String::from('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            '$' | '%' if chars.peek() == Some(&'{') => {
                out.push(c);
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The inside of an HCL string literal.
fn esc(s: &str) -> String {
    let quoted = q(s);
    quoted[1..quoted.len() - 1].to_string()
}

fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(", "))
}

// --- A small HCL writer that formats like `tofu fmt` -----------------------

enum Item {
    Attr(String, Val),
    Block(String, Vec<Item>),
    Comment(String),
    Blank,
}

enum Val {
    /// A one-line expression.
    E(String),
    /// A multi-line object.
    Obj(Vec<Item>),
    /// A list of multi-line objects.
    Objs(Vec<Vec<Item>>),
}

fn a(key: &str, expr: impl Into<String>) -> Item {
    Item::Attr(key.into(), Val::E(expr.into()))
}

fn obj(key: &str, items: Vec<Item>) -> Item {
    Item::Attr(key.into(), Val::Obj(items))
}

fn block(header: impl Into<String>, items: Vec<Item>) -> Item {
    Item::Block(header.into(), items)
}

/// A `# …` line. Author text goes in here, so control characters become
/// spaces: a line break must never end the comment and start real HCL.
fn comment(c: impl Into<String>) -> Item {
    Item::Comment(crate::one_line(&c.into()))
}

fn tags(id: &str) -> Item {
    a("tags", format!("{{ \"rhizo:node\" = {} }}", q(id)))
}

/// Writes `items` at `depth` the way `tofu fmt` lays them out: the `=` of
/// consecutive one-line attributes line up; comments, blank lines, blocks
/// and multi-line values end such a run.
fn render(items: &[Item], depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let one_line = |i: &Item| matches!(i, Item::Attr(_, Val::E(_)));
    let mut i = 0;
    while i < items.len() {
        if one_line(&items[i]) {
            let end = items[i..].iter().position(|x| !one_line(x)).map_or(items.len(), |p| i + p);
            let width = items[i..end].iter().map(|x| if let Item::Attr(k, _) = x { k.len() } else { 0 }).max();
            for x in &items[i..end] {
                if let Item::Attr(k, Val::E(e)) = x {
                    *out += &format!("{pad}{k:<w$} = {e}\n", w = width.unwrap_or(0));
                }
            }
            i = end;
            continue;
        }
        match &items[i] {
            Item::Attr(k, Val::Obj(inner)) => {
                *out += &format!("{pad}{k} = {{\n");
                render(inner, depth + 1, out);
                *out += &format!("{pad}}}\n");
            }
            Item::Attr(k, Val::Objs(objs)) if objs.len() == 1 => {
                *out += &format!("{pad}{k} = [{{\n");
                render(&objs[0], depth + 1, out);
                *out += &format!("{pad}}}]\n");
            }
            Item::Attr(k, Val::Objs(objs)) => {
                *out += &format!("{pad}{k} = [\n");
                for o in objs {
                    *out += &format!("{pad}  {{\n");
                    render(o, depth + 2, out);
                    *out += &format!("{pad}  }},\n");
                }
                *out += &format!("{pad}]\n");
            }
            Item::Block(h, inner) if inner.is_empty() => *out += &format!("{pad}{h} {{}}\n"),
            Item::Block(h, inner) => {
                *out += &format!("{pad}{h} {{\n");
                render(inner, depth + 1, out);
                *out += &format!("{pad}}}\n");
            }
            Item::Comment(c) => *out += &format!("{pad}# {c}\n"),
            Item::Blank => *out += "\n",
            Item::Attr(_, Val::E(_)) => unreachable!("handled above"),
        }
        i += 1;
    }
}

/// A file: the header, then top-level items separated by blank lines.
fn file(header: &str, items: Vec<Vec<Item>>) -> String {
    // The header names the source and environment: a line break must not end the comment.
    let mut out = format!("# {}\n", crate::one_line(header));
    for group in items {
        out += "\n";
        render(&group, 0, &mut out);
    }
    out
}

// --- One environment ---------------------------------------------------------

struct Env<'a> {
    doc: &'a Document,
    frame: &'a Frame,
    /// `prod`: prefix of every AWS name.
    name: String,
    region: String,
    /// `shop` for `shop.rhizo` or a project folder `shop`.
    source: String,
    header: String,
    /// The environment's infra nodes, in document order.
    nodes: Vec<&'a Node>,
}

impl<'a> Env<'a> {
    fn new(doc: &'a Document, frame: &'a Frame, source_name: &str) -> Self {
        let name = name(frame.id.as_str());
        let header = format!(
            "@generated by rhizo {} from {source_name} (environment {}). Do not edit: change the diagram and recompile.",
            crate::VERSION,
            frame.id
        );
        let nodes = doc.nodes.values().filter(|n| n.frame == frame.id && k::is_infra(k::ty(n))).collect();
        let region = frame.props.get("region").and_then(Value::as_str).unwrap_or("us-east-1").to_string();
        let source = self::name(source_name.trim_end_matches(".rhizo").trim_end_matches(".kdl"));
        Env { doc, frame, name, region, source, header, nodes }
    }

    fn of(&self, ty: &str) -> impl Iterator<Item = &'a Node> + '_ {
        let ty = ty.to_string();
        self.nodes.iter().copied().filter(move |n| k::ty(n) == ty)
    }

    fn any(&self, ty: &str) -> bool {
        self.of(ty).next().is_some()
    }

    /// Links in this environment, in document order.
    fn links(&self) -> Vec<(&'a Edge, &'a Node, &'a Node, Link)> {
        k::links(self.doc)
            .filter(|(_, _, a, b, _)| a.frame == self.frame.id && b.frame == self.frame.id)
            .map(|(_, e, a, b, l)| (e, a, b, l))
            .collect()
    }

    fn links_from(&self, n: &Node, kind: Link) -> Vec<(&'a Edge, &'a Node)> {
        self.links().into_iter().filter(|(_, a, _, l)| a.id == n.id && *l == kind).map(|(e, _, b, _)| (e, b)).collect()
    }

    fn links_to(&self, n: &Node, kind: Link) -> Vec<(&'a Edge, &'a Node)> {
        self.links().into_iter().filter(|(_, _, b, l)| b.id == n.id && *l == kind).map(|(e, a, _, _)| (e, a)).collect()
    }

    /// `prod-orders-db`.
    fn aws_name(&self, n: &Node) -> String {
        format!("{}-{}", self.name, name(n.id.as_str()))
    }

    fn m(&self, n: &Node) -> String {
        format!("module.{}", addr(n.id.as_str()))
    }

    fn dir(&self) -> String {
        dir(self.frame)
    }

    fn emit(&self, files: &mut Files) {
        let ids = |ns: &[&Node]| -> Vec<String> {
            let mut v: Vec<String> = ns.iter().map(|n| n.id.to_string()).collect();
            v.sort();
            v
        };
        let all = ids(&self.nodes);
        let lambdas: Vec<&Node> = self.of(FUNCTION).collect();
        let mut put = |file: &str, contents: String, nodes: Vec<String>| {
            files.insert(format!("{}/{file}", self.dir()), OutFile { contents, nodes });
        };
        put("versions.tf", self.versions(), Vec::new());
        if self.state_bucket().is_some() {
            put("encryption.tofu", self.encryption(), Vec::new());
        }
        put("providers.tf", self.providers(), Vec::new());
        put("main.tf", self.main(), all.clone());
        let (iam, iam_nodes) = self.iam();
        if !iam.is_empty() {
            put("iam.tf", file(&self.header, iam), iam_nodes);
        }
        let (net, net_nodes) = self.network();
        if !net.is_empty() {
            put("network.tf", file(&self.header, net), net_nodes);
        }
        put("variables.tf", self.variables(), ids(&lambdas));
        put("outputs.tf", self.outputs(), all);
    }

    fn state_bucket(&self) -> Option<&str> {
        self.frame.props.get("state-bucket").and_then(Value::as_str)
    }

    fn versions(&self) -> String {
        let provider = |p: &str, v: &str| {
            obj(p, vec![a("source", q(&format!("hashicorp/{p}"))), a("version", q(&format!("= {v}")))])
        };
        let mut providers = vec![provider("aws", AWS)];
        if self.any(FUNCTION) {
            providers.extend([provider("external", EXTERNAL), provider("local", LOCAL), provider("null", NULL)]);
        }
        if self.any(POSTGRES) {
            providers.push(provider("random", RANDOM));
        }
        let mut items = vec![
            a("required_version", q(OPENTOFU)),
            Item::Blank,
            comment("Exact versions from the infra kit's catalog; upgrading the kit is the only way they change."),
            block("required_providers", providers),
        ];
        if let Some(bucket) = self.state_bucket() {
            let stem = &self.source;
            items.extend([
                Item::Blank,
                block(
                    "backend \"s3\"",
                    vec![
                        a("bucket", q(bucket)),
                        a("key", q(&format!("{stem}/{}.tfstate", self.name))),
                        a("region", q(&self.region)),
                        a("encrypt", "true"),
                        a("use_lockfile", "true"),
                    ],
                ),
            ]);
        }
        file(&self.header, vec![vec![block("terraform", items)]])
    }

    fn encryption(&self) -> String {
        let method = |which: &str| block(which, vec![a("method", "method.aes_gcm.state"), a("enforced", "true")]);
        let enc = block(
            "encryption",
            vec![
                block(
                    "key_provider \"aws_kms\" \"state\"",
                    vec![
                        a("kms_key_id", "var.state_kms_key_id"),
                        a("region", q(&self.region)),
                        a("key_spec", q("AES_256")),
                    ],
                ),
                block("method \"aes_gcm\" \"state\"", vec![a("keys", "key_provider.aws_kms.state")]),
                method("state"),
                method("plan"),
            ],
        );
        file(
            &self.header,
            vec![vec![
                comment("OpenTofu-only syntax, kept out of .tf files so TFLint and Checkov never see it."),
                block("terraform", vec![enc]),
            ]],
        )
    }

    fn providers(&self) -> String {
        let source = &self.source;
        let tags = obj(
            "tags",
            vec![
                a("\"rhizo:source\"", q(source)),
                a("\"rhizo:environment\"", q(self.frame.id.as_str())),
                a("\"managed-by\"", q("opentofu")),
            ],
        );
        let p = block(
            "provider \"aws\"",
            vec![
                a("region", q(&self.region)),
                a("allowed_account_ids", "[var.aws_account_id]"),
                Item::Blank,
                block("default_tags", vec![tags]),
            ],
        );
        file(&self.header, vec![vec![p]])
    }

    fn main(&self) -> String {
        let mut groups: Vec<Vec<Item>> = Vec::new();
        if self.any(VPC) {
            groups.push(vec![block("data \"aws_availability_zones\" \"available\"", vec![a("state", q("available"))])]);
        }
        if self.links().iter().any(|(_, _, b, l)| *l == Link::Grant && k::ty(b) == POSTGRES) {
            groups.push(vec![block("data \"aws_caller_identity\" \"current\"", vec![])]);
            groups.push(vec![block("data \"aws_partition\" \"current\"", vec![])]);
        }
        for n in &self.nodes {
            groups.extend(self.node_groups(n));
        }
        if let Some(budget) = self.frame.props.get("budget-monthly") {
            groups.push(vec![
                comment(format!("rhizo:frame={} budget-monthly (whole account, by AWS Budgets)", self.frame.id)),
                block(
                    "resource \"aws_budgets_budget\" \"monthly\"",
                    vec![
                        a("name", q(&format!("{}-monthly", self.name))),
                        a("budget_type", q("COST")),
                        a("limit_amount", q(&budget.to_string())),
                        a("limit_unit", q("USD")),
                        a("time_unit", q("MONTHLY")),
                        Item::Blank,
                        block(
                            "notification",
                            vec![
                                a("comparison_operator", q("GREATER_THAN")),
                                a("threshold", "80"),
                                a("threshold_type", q("PERCENTAGE")),
                                a("notification_type", q("FORECASTED")),
                                a("subscriber_email_addresses", "var.budget_alert_emails"),
                            ],
                        ),
                    ],
                ),
            ]);
        }
        file(&self.header, groups)
    }

    /// The code of one node in `main.tf`: its module call, or a generic
    /// node's block.
    fn node_groups(&self, n: &Node) -> Vec<Vec<Item>> {
        let body = match k::ty(n) {
            VPC => self.vpc(n),
            BUCKET => self.bucket(n),
            TABLE => self.table(n),
            POSTGRES => self.postgres(n),
            QUEUE => self.queue(n),
            TOPIC => self.topic(n),
            KEY => self.key(n),
            FUNCTION => self.function(n),
            HTTP_API => self.api(n),
            k::RESOURCE | k::MODULE => return vec![generic(n)],
            _ => return Vec::new(), // subnets are tiers of their VPC
        };
        let (source, version) = module(k::ty(n));
        let mut items = vec![a("source", q(source)), a("version", q(version)), Item::Blank];
        items.extend(body);
        items.extend([Item::Blank, tags(n.id.as_str())]);
        vec![vec![comment(format!("rhizo:node={}", n.id)), block(format!("module \"{}\"", addr(n.id.as_str())), items)]]
    }

    /// Subnet tiers of a VPC: public and private always, database when a
    /// database is placed there or a database tier is drawn.
    fn tiers(&self, vpc: &Node) -> Vec<&'static str> {
        let drawn = |t: &str| {
            vpc.children
                .iter()
                .filter_map(|c| self.doc.nodes.get(c))
                .any(|c| k::ty(c) == SUBNET && k::text(c, "tier") == Some(t))
        };
        let placed_db = self
            .nodes
            .iter()
            .any(|n| k::placement(self.doc, n).is_some_and(|p| p.vpc.id == vpc.id && p.tier == "database"));
        let mut t = vec!["public", "private"];
        if placed_db || drawn("database") {
            t.push("database");
        }
        t
    }

    fn vpc(&self, n: &Node) -> Vec<Item> {
        let cidr = k::text(n, "cidr").unwrap_or("10.0.0.0/16");
        let azs = k::int(n, "azs", 2);
        let (base, len) = k::parse_cidr(cidr).unwrap_or((0x0a00_0000, 16));
        // Private /len+4 subnets from the bottom; public and database /len+8 above them.
        let subnet = |bits: u32, index: u32| {
            let ip = std::net::Ipv4Addr::from(base | (index << (32 - len - bits)));
            q(&format!("{ip}/{}", len + bits))
        };
        let range = |bits, first: u32| list((0..azs as u32).map(|i| subnet(bits, first + i)));
        let tiers = self.tiers(n);
        let mut items = vec![
            a("name", q(&self.aws_name(n))),
            a("cidr", q(cidr)),
            a("azs", format!("slice(data.aws_availability_zones.available.names, 0, {azs})")),
            Item::Blank,
            a("public_subnets", range(8, 48)),
            a("private_subnets", range(4, 0)),
        ];
        if tiers.contains(&"database") {
            items.extend([a("database_subnets", range(8, 64)), a("create_database_subnet_group", "true")]);
        }
        let nat = k::text(n, "nat").unwrap_or("single");
        items.extend([
            Item::Blank,
            comment(format!("nat={nat}")),
            a("enable_nat_gateway", (nat != "none").to_string()),
            a("single_nat_gateway", (nat == "single").to_string()),
            a("one_nat_gateway_per_az", (nat == "per-az").to_string()),
            Item::Blank,
            a("manage_default_security_group", "true"),
            a("default_security_group_ingress", "[]"),
            a("default_security_group_egress", "[]"),
        ]);
        if k::flag(n, "flow-logs", true) {
            items.extend([
                Item::Blank,
                a("enable_flow_log", "true"),
                a("create_flow_log_cloudwatch_log_group", "true"),
                a("create_flow_log_cloudwatch_iam_role", "true"),
                a("flow_log_cloudwatch_log_group_retention_in_days", "365"),
            ]);
        }
        items
    }

    fn sse(&self, n: &Node) -> Option<String> {
        k::key(self.doc, n).map(|key| format!("{}.key_arn", self.m(key)))
    }

    fn bucket(&self, n: &Node) -> Vec<Item> {
        let mut items = vec![a("bucket_prefix", q(&format!("{}-", self.aws_name(n))))];
        let versioning = k::flag(n, "versioning", true);
        if versioning {
            items.push(a("versioning", "{ enabled = true }"));
        }
        items.extend([
            Item::Blank,
            comment("Baseline: no public access, owner-enforced objects, TLS only."),
            a("block_public_acls", "true"),
            a("block_public_policy", "true"),
            a("ignore_public_acls", "true"),
            a("restrict_public_buckets", "true"),
            a("control_object_ownership", "true"),
            a("object_ownership", q("BucketOwnerEnforced")),
            a("attach_deny_insecure_transport_policy", "true"),
            a("attach_require_latest_tls_policy", "true"),
            Item::Blank,
        ]);
        let default = match self.sse(n) {
            Some(key) => vec![a("sse_algorithm", q("aws:kms")), a("kms_master_key_id", key)],
            None => vec![a("sse_algorithm", q("AES256"))],
        };
        let mut rule = vec![obj("apply_server_side_encryption_by_default", default)];
        if self.sse(n).is_some() {
            rule.push(a("bucket_key_enabled", "true"));
        }
        items.push(obj("server_side_encryption_configuration", vec![obj("rule", rule)]));
        let rules: Vec<Vec<Item>> = n
            .props
            .get("lifecycle")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, r)| {
                let prefix = r.get("prefix").and_then(Value::as_str).unwrap_or("");
                let days = r.get("expire-after-days").map(Value::to_string).unwrap_or_else(|| "1".into());
                let id =
                    if prefix.is_empty() { format!("expire-{}", i + 1) } else { format!("expire-{}", name(prefix)) };
                let mut rule = vec![
                    a("id", q(&id)),
                    a("enabled", "true"),
                    a("filter", format!("{{ prefix = {} }}", q(prefix))),
                    a("expiration", format!("{{ days = {days} }}")),
                    a("abort_incomplete_multipart_upload_days", "7"),
                ];
                if versioning {
                    rule.push(a("noncurrent_version_expiration", format!("{{ noncurrent_days = {days} }}")));
                }
                rule
            })
            .collect();
        if !rules.is_empty() {
            items.extend([Item::Blank, Item::Attr("lifecycle_rule".into(), Val::Objs(rules))]);
        }
        items
    }

    fn table(&self, n: &Node) -> Vec<Item> {
        let mut items = vec![a("name", q(&self.aws_name(n))), a("billing_mode", q("PAY_PER_REQUEST"))];
        let mut attrs = Vec::new();
        for (key, ty, attr) in [("hash-key", "hash-key-type", "hash_key"), ("range-key", "range-key-type", "range_key")]
        {
            if let Some(k_) = k::text(n, key) {
                items.push(a(attr, q(k_)));
                attrs.push(vec![a("name", q(k_)), a("type", q(k::text(n, ty).unwrap_or("S")))]);
            }
        }
        items.push(Item::Attr("attributes".into(), Val::Objs(attrs)));
        if let Some(ttl) = k::text(n, "ttl-attribute") {
            items.extend([Item::Blank, a("ttl_enabled", "true"), a("ttl_attribute_name", q(ttl))]);
        }
        items.extend([
            Item::Blank,
            a("point_in_time_recovery_enabled", k::flag(n, "point-in-time-recovery", true).to_string()),
            a("deletion_protection_enabled", k::flag(n, "deletion-protection", true).to_string()),
            a("server_side_encryption_enabled", "true"),
        ]);
        if let Some(key) = self.sse(n) {
            items.push(a("server_side_encryption_kms_key_arn", key));
        }
        items
    }

    fn postgres(&self, n: &Node) -> Vec<Item> {
        let version = k::text(n, "engine-version").unwrap_or("17");
        let major = version.split('.').next().unwrap_or(version);
        let db = k::text(n, "database").map_or_else(|| addr(n.id.as_str()), str::to_string);
        let mut items = vec![
            a("identifier", q(&self.aws_name(n))),
            a("engine", q("postgres")),
            a("engine_version", q(version)),
            a("family", q(&format!("postgres{major}"))),
            a("major_engine_version", q(major)),
            a("instance_class", q(k::text(n, "instance-class").unwrap_or("db.t4g.micro"))),
            a("allocated_storage", k::int(n, "storage-gb", 20).to_string()),
            a("multi_az", k::flag(n, "multi-az", true).to_string()),
            Item::Blank,
            a("db_name", q(&db)),
            a("username", q("dbadmin")),
            a("port", "5432"),
            a("manage_master_user_password", "true"),
            a("iam_database_authentication_enabled", "true"),
            Item::Blank,
        ];
        let sg = format!("[aws_security_group.{}.id]", addr(n.id.as_str()));
        match k::placement(self.doc, n) {
            Some(p) if p.tier == "database" => {
                items.push(a("db_subnet_group_name", format!("{}.database_subnet_group_name", self.m(p.vpc))));
            }
            Some(p) => {
                items.extend([
                    a("create_db_subnet_group", "true"),
                    a("subnet_ids", format!("{}.{}_subnets", self.m(p.vpc), p.tier)),
                ]);
            }
            None => {}
        }
        items.extend([a("vpc_security_group_ids", sg), Item::Blank]);
        items.extend([
            comment("Baseline: private, encrypted, protected, backed up, logged."),
            a("publicly_accessible", "false"),
            a("storage_encrypted", "true"),
        ]);
        if let Some(key) = self.sse(n) {
            items.push(a("kms_key_id", key));
        }
        items.extend([
            a("deletion_protection", k::flag(n, "deletion-protection", true).to_string()),
            a("backup_retention_period", k::int(n, "backup-days", 7).to_string()),
            a("copy_tags_to_snapshot", "true"),
            a("enabled_cloudwatch_logs_exports", list([q("postgresql"), q("upgrade")])),
            a("create_db_option_group", "false"),
        ]);
        items
    }

    fn queue(&self, n: &Node) -> Vec<Item> {
        let mut items = vec![a("name", q(&self.aws_name(n)))];
        if k::flag(n, "fifo", false) {
            items.push(a("fifo_queue", "true"));
        }
        items.extend([
            a("message_retention_seconds", k::int(n, "retention-s", 345_600).to_string()),
            a("visibility_timeout_seconds", k::int(n, "visibility-timeout-s", 30).to_string()),
            Item::Blank,
        ]);
        match self.sse(n) {
            Some(key) => items.extend([a("kms_master_key_id", key), a("kms_data_key_reuse_period_seconds", "300")]),
            None => items.push(a("sqs_managed_sse_enabled", "true")),
        }
        if k::flag(n, "dlq", true) {
            items.extend([
                Item::Blank,
                a("create_dlq", "true"),
                a("dlq_message_retention_seconds", "1209600"),
                obj("redrive_policy", vec![a("maxReceiveCount", k::int(n, "max-receives", 5).to_string())]),
            ]);
        }
        let topics = self.links_to(n, Link::Subscribe);
        if !topics.is_empty() {
            let statements: Vec<Item> = topics
                .iter()
                .map(|(_, t)| {
                    obj(
                        &addr(t.id.as_str()),
                        vec![
                            a("sid", q(&format!("{}Send", pascal(t.id.as_str())))),
                            a("actions", list([q("sqs:SendMessage")])),
                            Item::Attr(
                                "principals".into(),
                                Val::Objs(vec![vec![
                                    a("type", q("Service")),
                                    a("identifiers", list([q("sns.amazonaws.com")])),
                                ]]),
                            ),
                            Item::Attr(
                                "condition".into(),
                                Val::Objs(vec![vec![
                                    a("test", q("ArnEquals")),
                                    a("variable", q("aws:SourceArn")),
                                    a("values", list([format!("{}.topic_arn", self.m(t))])),
                                ]]),
                            ),
                        ],
                    )
                })
                .collect();
            items.extend([
                Item::Blank,
                comment(format!(
                    "rhizo:edge={}",
                    topics.iter().map(|(_, t)| format!("{}->{}", t.id, n.id)).collect::<Vec<_>>().join(" ")
                )),
                a("create_queue_policy", "true"),
                obj("queue_policy_statements", statements),
            ]);
        }
        items
    }

    fn topic(&self, n: &Node) -> Vec<Item> {
        let mut items = vec![a("name", q(&self.aws_name(n)))];
        if k::flag(n, "fifo", false) {
            items.push(a("fifo_topic", "true"));
        }
        let key = k::key(self.doc, n).map_or_else(|| q("alias/aws/sns"), |key| format!("{}.key_id", self.m(key)));
        items.push(a("kms_master_key_id", key));
        let subs = self.links_from(n, Link::Subscribe);
        if !subs.is_empty() {
            let entries = subs
                .iter()
                .map(|(_, s)| {
                    let (protocol, endpoint) = if k::ty(s) == QUEUE {
                        ("sqs", format!("{}.queue_arn", self.m(s)))
                    } else {
                        ("lambda", format!("{}.lambda_function_arn", self.m(s)))
                    };
                    obj(&addr(s.id.as_str()), vec![a("protocol", q(protocol)), a("endpoint", endpoint)])
                })
                .collect();
            items.extend([Item::Blank, obj("subscriptions", entries)]);
        }
        items
    }

    fn key(&self, n: &Node) -> Vec<Item> {
        let mut items = vec![
            a("description", q(k::text(n, "description").unwrap_or(&n.label))),
            a("deletion_window_in_days", k::int(n, "deletion-window-days", 30).to_string()),
            a("enable_key_rotation", "true"),
            a("aliases", list([q(&self.aws_name(n))])),
        ];
        // SNS delivers to queues encrypted with this key only if the key policy lets it.
        let sns_queue = self.links().iter().any(|(_, _, b, l)| {
            *l == Link::Subscribe && k::ty(b) == QUEUE && k::key(self.doc, b).is_some_and(|x| x.id == n.id)
        });
        if sns_queue {
            items.extend([
                Item::Blank,
                Item::Attr(
                    "key_statements".into(),
                    Val::Objs(vec![vec![
                        a("sid", q("SnsDeliveryToEncryptedQueues")),
                        a("actions", list([q("kms:Decrypt"), q("kms:GenerateDataKey*")])),
                        a("resources", list([q("*")])),
                        Item::Attr(
                            "principals".into(),
                            Val::Objs(vec![vec![
                                a("type", q("Service")),
                                a("identifiers", list([q("sns.amazonaws.com")])),
                            ]]),
                        ),
                    ]]),
                ),
            ]);
        }
        items
    }

    /// Environment variables a function finds its resources by, from its edges.
    fn wiring(&self, n: &Node) -> Vec<(String, String)> {
        let mut vars = Vec::new();
        for (_, s) in self.links_from(n, Link::Grant) {
            let (m, up) = (self.m(s), upper(s.id.as_str()));
            match k::ty(s) {
                BUCKET => vars.push((format!("{up}_BUCKET"), format!("{m}.s3_bucket_id"))),
                TABLE => vars.push((format!("{up}_TABLE"), format!("{m}.dynamodb_table_id"))),
                QUEUE => vars.push((format!("{up}_QUEUE_URL"), format!("{m}.queue_url"))),
                TOPIC => vars.push((format!("{up}_TOPIC_ARN"), format!("{m}.topic_arn"))),
                POSTGRES => vars.extend([
                    (format!("{up}_HOST"), format!("{m}.db_instance_address")),
                    (format!("{up}_PORT"), q("5432")),
                    (format!("{up}_NAME"), format!("{m}.db_instance_name")),
                    (format!("{up}_USER"), q(&db_user(n))),
                ]),
                _ => {}
            }
        }
        vars
    }

    fn function(&self, n: &Node) -> Vec<Item> {
        let a_ = addr(n.id.as_str());
        let mut items = vec![
            a("function_name", q(&self.aws_name(n))),
            a("description", q(&n.label)),
            a("runtime", q(k::text(n, "runtime").unwrap_or("nodejs22.x"))),
            a("handler", q(k::text(n, "handler").unwrap_or("index.handler"))),
            a("architectures", list([q(k::text(n, "architecture").unwrap_or("arm64"))])),
            a("memory_size", k::int(n, "memory-mb", 256).to_string()),
            a("timeout", k::int(n, "timeout-s", 10).to_string()),
            Item::Blank,
            comment("The code is built outside this module (task T-".to_string() + n.id.as_str() + ")."),
            a("create_package", "false"),
            a("local_existing_package", format!("var.{a_}_package")),
        ];
        let mut env: Vec<(String, String)> = self.wiring(n);
        if let Some(user) = n.props.get("environment").and_then(Value::as_object) {
            for (key, v) in user {
                env.retain(|(x, _)| x != key);
                env.push((key.clone(), q(v.as_str().unwrap_or(""))));
            }
        }
        if !env.is_empty() {
            let vars = env.into_iter().map(|(k_, v)| a(&k_, v)).collect();
            items.extend([Item::Blank, obj("environment_variables", vars)]);
        }
        if let Some(p) = k::placement(self.doc, n) {
            let subnets = format!("{}.{}_subnets", self.m(p.vpc), p.tier);
            items.extend([
                Item::Blank,
                a("vpc_subnet_ids", subnets),
                a("vpc_security_group_ids", format!("[aws_security_group.{a_}.id]")),
                a("attach_network_policy", "true"),
            ]);
        }
        items.extend([
            Item::Blank,
            a("tracing_mode", q("Active")),
            a("attach_tracing_policy", "true"),
            a("cloudwatch_logs_retention_in_days", k::int(n, "log-retention-days", 30).to_string()),
        ]);
        if self.policy(n).is_some() {
            items.extend([
                Item::Blank,
                a("attach_policy_json", "true"),
                a("policy_json", format!("data.aws_iam_policy_document.{a_}.json")),
            ]);
        }
        let mut triggers = Vec::new();
        for (_, api) in self.links_to(n, Link::Route) {
            triggers.push(obj(
                &addr(api.id.as_str()),
                vec![
                    a("service", q("apigateway")),
                    a("source_arn", format!("\"${{{}.api_execution_arn}}/*/*\"", self.m(api))),
                ],
            ));
        }
        for (_, t) in self.links_to(n, Link::Subscribe) {
            triggers.push(obj(
                &addr(t.id.as_str()),
                vec![a("service", q("sns")), a("source_arn", format!("{}.topic_arn", self.m(t)))],
            ));
        }
        if !triggers.is_empty() {
            items.extend([
                Item::Blank,
                comment("Unqualified permissions: the function is not published as versions."),
                a("create_current_version_allowed_triggers", "false"),
                obj("allowed_triggers", triggers),
            ]);
        }
        let sources: Vec<Item> = self
            .links_to(n, Link::Trigger)
            .iter()
            .map(|(_, qn)| {
                obj(
                    &addr(qn.id.as_str()),
                    vec![
                        a("event_source_arn", format!("{}.queue_arn", self.m(qn))),
                        a("function_response_types", list([q("ReportBatchItemFailures")])),
                    ],
                )
            })
            .collect();
        if !sources.is_empty() {
            items.extend([Item::Blank, obj("event_source_mapping", sources)]);
        }
        items
    }

    fn api(&self, n: &Node) -> Vec<Item> {
        let mut items = vec![
            a("name", q(&self.aws_name(n))),
            a("description", q(&n.label)),
            a("protocol_type", q("HTTP")),
            a("create_domain_name", "false"),
        ];
        let mut routes: Vec<Item> = Vec::new();
        let mut methods: BTreeSet<String> = BTreeSet::new();
        for (e, f) in self.links_from(n, Link::Route) {
            for r in k::routes(self.doc, e) {
                methods.insert(r.split(' ').next().filter(|_| r != "$default").unwrap_or("*").to_string());
                routes.push(obj(
                    &q(&r),
                    vec![obj(
                        "integration",
                        vec![
                            a("uri", format!("{}.lambda_function_arn", self.m(f))),
                            a("payload_format_version", q("2.0")),
                        ],
                    )],
                ));
            }
        }
        let origins: Vec<String> = n
            .props
            .get("cors-origins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(q)
            .collect();
        if !origins.is_empty() {
            let methods: Vec<String> = if methods.contains("*") || methods.contains("ANY") {
                vec![q("*")]
            } else {
                methods.iter().map(|m| q(m)).collect()
            };
            items.extend([
                Item::Blank,
                obj(
                    "cors_configuration",
                    vec![
                        a("allow_origins", list(origins)),
                        a("allow_methods", list(methods)),
                        a("allow_headers", list([q("authorization"), q("content-type")])),
                        a("max_age", "3600"),
                    ],
                ),
            ]);
        }
        if !routes.is_empty() {
            items.extend([Item::Blank, obj("routes", routes)]);
        }
        items
    }

    /// The IAM policy document of a function: one statement per grant or
    /// trigger edge, plus the KMS use those need.
    fn policy(&self, n: &Node) -> Option<Vec<Item>> {
        let mut out: Vec<Item> = Vec::new();
        let mut statement = |edge: String, sid: String, actions: Vec<&str>, resources: Vec<String>| {
            if !out.is_empty() {
                out.push(Item::Blank);
            }
            out.extend([
                comment(edge),
                block(
                    "statement",
                    vec![
                        a("sid", q(&sid)),
                        a("actions", list(actions.into_iter().map(q))),
                        a("resources", list(resources)),
                    ],
                ),
            ]);
        };
        for (e, s) in self.links_from(n, Link::Grant) {
            let Ok((read, write)) = k::access(e, s) else { continue };
            let access = match (read, write) {
                (true, true) => "read-write",
                (true, false) => "read",
                _ => "write",
            };
            let (m, sid) = (self.m(s), pascal(s.id.as_str()));
            let mut trace = format!("rhizo:edge={}->{} access={access}", n.id, s.id);
            let pick = |r: &[&'static str], w: &[&'static str]| -> Vec<&'static str> {
                let mut v: Vec<&str> = Vec::new();
                if read {
                    v.extend(r);
                }
                if write {
                    v.extend(w);
                }
                v.sort();
                v.dedup();
                v
            };
            let (actions, resources) = match k::ty(s) {
                BUCKET => {
                    let prefix = e.props.get("prefix").and_then(Value::as_str).unwrap_or("");
                    if !prefix.is_empty() {
                        trace += &format!(" prefix={prefix}");
                    }
                    let objects = format!("\"${{{m}.s3_bucket_arn}}/{}*\"", esc(prefix));
                    (pick(&["s3:GetObject"], &["s3:PutObject"]), vec![objects])
                }
                TABLE => (
                    pick(
                        &[
                            "dynamodb:BatchGetItem",
                            "dynamodb:ConditionCheckItem",
                            "dynamodb:GetItem",
                            "dynamodb:Query",
                            "dynamodb:Scan",
                        ],
                        &["dynamodb:BatchWriteItem", "dynamodb:DeleteItem", "dynamodb:PutItem", "dynamodb:UpdateItem"],
                    ),
                    vec![format!("{m}.dynamodb_table_arn"), format!("\"${{{m}.dynamodb_table_arn}}/index/*\"")],
                ),
                QUEUE => (
                    pick(
                        &[
                            "sqs:ChangeMessageVisibility",
                            "sqs:DeleteMessage",
                            "sqs:GetQueueAttributes",
                            "sqs:ReceiveMessage",
                        ],
                        &["sqs:GetQueueAttributes", "sqs:SendMessage"],
                    ),
                    vec![format!("{m}.queue_arn")],
                ),
                TOPIC => (vec!["sns:Publish"], vec![format!("{m}.topic_arn")]),
                POSTGRES => {
                    trace += &format!(" (IAM database auth as user {})", db_user(n));
                    let arn = format!(
                        "\"arn:${{data.aws_partition.current.partition}}:rds-db:{}:${{data.aws_caller_identity.current.account_id}}:dbuser:${{{m}.db_instance_resource_id}}/{}\"",
                        self.region,
                        db_user(n)
                    );
                    (vec!["rds-db:connect"], vec![arn])
                }
                _ => continue,
            };
            let verb = match k::ty(s) {
                POSTGRES => "Connect".to_string(),
                _ => pascal(access),
            };
            statement(trace, format!("{sid}{verb}"), actions, resources);
            if let Some(key) = k::key(self.doc, s).filter(|_| matches!(k::ty(s), BUCKET | QUEUE | TOPIC)) {
                let actions = if write { vec!["kms:Decrypt", "kms:GenerateDataKey"] } else { vec!["kms:Decrypt"] };
                statement(
                    format!("rhizo:edge={}->{} (encrypted with {})", n.id, s.id, key.id),
                    format!("{sid}Key"),
                    actions,
                    vec![format!("{}.key_arn", self.m(key))],
                );
            }
        }
        for (_, qn) in self.links_to(n, Link::Trigger) {
            let m = self.m(qn);
            statement(
                format!("rhizo:edge={}->{} (event source)", qn.id, n.id),
                format!("{}Consume", pascal(qn.id.as_str())),
                vec![
                    "sqs:ChangeMessageVisibility",
                    "sqs:DeleteMessage",
                    "sqs:GetQueueAttributes",
                    "sqs:ReceiveMessage",
                ],
                vec![format!("{m}.queue_arn")],
            );
            if let Some(key) = k::key(self.doc, qn) {
                statement(
                    format!("rhizo:edge={}->{} (encrypted with {})", qn.id, n.id, key.id),
                    format!("{}Key", pascal(qn.id.as_str())),
                    vec!["kms:Decrypt"],
                    vec![format!("{}.key_arn", self.m(key))],
                );
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    fn iam(&self) -> (Vec<Vec<Item>>, Vec<String>) {
        let mut groups = Vec::new();
        let mut nodes = BTreeSet::new();
        for n in self.of(FUNCTION) {
            let Some(statements) = self.policy(n) else { continue };
            nodes.insert(n.id.to_string());
            for (_, s) in self.links_from(n, Link::Grant).iter().chain(&self.links_to(n, Link::Trigger)) {
                nodes.insert(s.id.to_string());
            }
            groups.push(vec![
                comment(format!("rhizo:node={}", n.id)),
                block(format!("data \"aws_iam_policy_document\" \"{}\"", addr(n.id.as_str())), statements),
            ]);
        }
        (groups, nodes.into_iter().collect())
    }

    /// Security groups for everything placed in a VPC, and the rules that
    /// edges to databases imply.
    fn network(&self) -> (Vec<Vec<Item>>, Vec<String>) {
        let mut groups = Vec::new();
        let mut nodes = BTreeSet::new();
        for n in self.nodes.iter().filter(|n| matches!(k::ty(n), FUNCTION | POSTGRES)) {
            let Some(p) = k::placement(self.doc, n) else { continue };
            nodes.insert(n.id.to_string());
            let a_ = addr(n.id.as_str());
            groups.push(vec![
                comment(format!("rhizo:node={}", n.id)),
                block(
                    format!("resource \"aws_security_group\" \"{a_}\""),
                    vec![
                        a("name_prefix", q(&format!("{}-", self.aws_name(n)))),
                        a("description", q(&n.label)),
                        a("vpc_id", format!("{}.vpc_id", self.m(p.vpc))),
                        tags(n.id.as_str()),
                        Item::Blank,
                        block("lifecycle", vec![a("create_before_destroy", "true")]),
                    ],
                ),
            ]);
            let calls_apis =
                self.links_from(n, Link::Grant).iter().any(|(_, s)| matches!(k::ty(s), BUCKET | TABLE | QUEUE | TOPIC));
            if k::ty(n) == FUNCTION && calls_apis {
                groups.push(vec![
                    comment(format!("rhizo:node={}: AWS APIs over HTTPS", n.id)),
                    block(
                        format!("resource \"aws_vpc_security_group_egress_rule\" \"{a_}_https\""),
                        vec![
                            a("security_group_id", format!("aws_security_group.{a_}.id")),
                            a("description", q("HTTPS to AWS APIs")),
                            a("cidr_ipv4", q("0.0.0.0/0")),
                            a("ip_protocol", q("tcp")),
                            a("from_port", "443"),
                            a("to_port", "443"),
                        ],
                    ),
                ]);
            }
        }
        for (_, f, db, l) in self.links() {
            if l != Link::Grant || k::ty(db) != POSTGRES || k::placement(self.doc, f).is_none() {
                continue;
            }
            let (fa, da) = (addr(f.id.as_str()), addr(db.id.as_str()));
            let rule = |kind: &str, name: String, own: &str, other: &str, desc: String| {
                vec![
                    comment(format!("rhizo:edge={}->{}", f.id, db.id)),
                    block(
                        format!("resource \"aws_vpc_security_group_{kind}_rule\" \"{name}\""),
                        vec![
                            a("security_group_id", format!("aws_security_group.{own}.id")),
                            a("referenced_security_group_id", format!("aws_security_group.{other}.id")),
                            a("description", q(&desc)),
                            a("ip_protocol", q("tcp")),
                            a("from_port", "5432"),
                            a("to_port", "5432"),
                        ],
                    ),
                ]
            };
            groups.push(rule("ingress", format!("{da}_from_{fa}"), &da, &fa, format!("Postgres from {}", f.id)));
            groups.push(rule("egress", format!("{fa}_to_{da}"), &fa, &da, format!("Postgres to {}", db.id)));
        }
        (groups, nodes.into_iter().collect())
    }

    fn variables(&self) -> String {
        file(&self.header, self.variable_blocks().into_iter().map(|(_, b)| vec![b]).collect())
    }

    /// The root module's variables: each name with its block.
    fn variable_blocks(&self) -> Vec<(String, Item)> {
        let var = |name: &str, ty: &str, desc: String, extra: Vec<Item>| {
            let mut items = vec![a("description", q(&desc)), a("type", ty)];
            if !extra.is_empty() {
                items.push(Item::Blank);
                items.extend(extra);
            }
            (name.to_string(), block(format!("variable \"{name}\""), items))
        };
        let mut groups = vec![var(
            "aws_account_id",
            "string",
            format!(
                "The AWS account the {} environment is deployed to; the provider refuses any other.",
                self.frame.id
            ),
            vec![block(
                "validation",
                vec![
                    a("condition", "can(regex(\"^[0-9]{12}$\", var.aws_account_id))"),
                    a("error_message", q("aws_account_id is a 12-digit AWS account id.")),
                ],
            )],
        )];
        if self.state_bucket().is_some() {
            groups.push(var(
                "state_kms_key_id",
                "string",
                "ARN of the existing KMS key that encrypts state and plans (encryption.tofu).".into(),
                vec![],
            ));
        }
        if self.frame.props.contains_key("budget-monthly") {
            groups.push(var("budget_alert_emails", "list(string)", "Who gets budget alerts.".into(), vec![]));
        }
        for n in self.of(FUNCTION) {
            groups.push(var(
                &format!("{}_package", addr(n.id.as_str())),
                "string",
                format!("Path to the deployment zip of {} (built by task T-{}).", n.label, n.id),
                vec![],
            ));
        }
        groups
    }

    fn outputs(&self) -> String {
        let mut groups = Vec::new();
        for n in &self.nodes {
            let (suffix, value, what) = match k::ty(n) {
                VPC => ("vpc_id", "vpc_id", "VPC id"),
                BUCKET => ("bucket", "s3_bucket_id", "Bucket name"),
                TABLE => ("table", "dynamodb_table_id", "Table name"),
                POSTGRES => ("endpoint", "db_instance_endpoint", "Database endpoint (host:port)"),
                QUEUE => ("queue_url", "queue_url", "Queue URL"),
                TOPIC => ("topic_arn", "topic_arn", "Topic ARN"),
                KEY => ("key_arn", "key_arn", "Key ARN"),
                FUNCTION => ("function_name", "lambda_function_name", "Function name"),
                HTTP_API => ("url", "api_endpoint", "Base URL"),
                _ => continue,
            };
            let a_ = addr(n.id.as_str());
            groups.push(vec![block(
                format!("output \"{a_}_{suffix}\""),
                vec![
                    a("description", q(&format!("{what} of {} ({})", n.label, n.id))),
                    a("value", format!("module.{a_}.{value}")),
                ],
            )]);
            if k::ty(n) == POSTGRES {
                groups.push(vec![block(
                    format!("output \"{a_}_master_secret_arn\""),
                    vec![
                        a("description", q(&format!("Secrets Manager secret with the master password of {}", n.id))),
                        a("value", format!("module.{a_}.db_instance_master_user_secret_arn")),
                    ],
                )]);
            }
        }
        file(&self.header, groups)
    }
}

/// The database user a function connects as with IAM authentication.
fn db_user(function: &Node) -> String {
    addr(function.id.as_str())
}

/// Environment variable names a function reads its resources from.
fn function_env(doc: &Document, function: &Node) -> Vec<String> {
    let Some(frame) = k::environment(doc, function) else { return Vec::new() };
    let env = Env::new(doc, frame, "");
    env.wiring(function).into_iter().map(|(k, _)| k).collect()
}

mod azure;
mod gcp;

#[cfg(test)]
mod tests;
