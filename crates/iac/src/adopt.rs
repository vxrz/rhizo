//! Adopting imported nodes (docs/design/iac-import.md, "Adopt"): a node
//! folded from imported code switches to rhizo's own code, so its props
//! become editable. The diagram is rewritten in place (the format-keeping
//! [`Editor`]): the node loses its verbatim code and keeps, in `origin`,
//! where it came from (`address`, and the satellites folded into it as
//! `also`), which the compiler turns into `moved` blocks, plus the
//! arguments that name or tag what exists (`keep`).
//!
//! Nothing is written unless **L1-adopt** passes: the code before and after
//! are planned offline (mocked providers, empty state, as L1); every
//! instance the original plans must be in the adopted plan, at its own
//! address or where a `moved` block takes it. Every attribute that
//! changes, and every instance only the adopted code creates, is listed.
//! Whether the real resources would be replaced only a live `tofu plan`
//! can show; the report says so.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hcl_edit::Span;
use hcl_edit::expr::{Expression, TraversalOperator};
use hcl_edit::visit::{Visit, visit_traversal};
use rhizo_core::{Document, FrameId, Node, NodeId};
use rhizo_kit::infra::{self as k, adopt as ka, fold};
use rhizo_lang::edit::Editor;
use serde_json::{Map, Value, json};

mod raw;

use crate::Error;
use crate::hcl::Source;
use crate::tofu::Run;

/// A finished adopt.
#[derive(Debug)]
pub struct Adopted {
    /// The rewritten diagram (the input with only the adopt's edits).
    pub kdl: String,
    pub report: String,
    /// The ids of the nodes adopted.
    pub nodes: Vec<String>,
    /// L1-adopt lists no attribute change and no create: only moves.
    pub quiet: bool,
}

/// Arguments a node must not have: rhizo's code would drop what they say.
const META: [&str; 6] = ["count", "for_each", "provider", "providers", "depends_on", "lifecycle"];

/// What adopting one node does.
struct Pick {
    id: NodeId,
    frame: FrameId,
    /// Where its code is now: `aws_s3_bucket.receipts`, `module.receipts`.
    address: String,
    /// The name of rhizo's code for it: `module.<name>` for a raw resource.
    name: String,
    /// A raw resource rhizo moves into a module call at `module.<name>`.
    raw: bool,
    /// Satellites folded into it whose code moves into rhizo's code.
    absorbed: Vec<String>,
    /// Satellites rhizo's code does not manage with these props: they stay
    /// verbatim, pointing at the adopted code.
    stay: Vec<String>,
    keep: Map<String, Value>,
    /// The original module call's source and version.
    module: Option<(String, String)>,
    /// Props its code sets with expressions: rhizo's code uses the kit's
    /// defaults for them.
    unread: Vec<String>,
    /// An API's routes, by the function they go to: they become the
    /// `route` prop of its edges.
    routes: Vec<(NodeId, Vec<String>)>,
    /// What rhizo's code says with the node's label (a function's
    /// description), read from its code: the node's new label.
    label_from: Option<String>,
}

impl Pick {
    /// Where rhizo's code for it lives.
    fn main(&self) -> String {
        if self.raw { format!("module.{}", self.name) } else { self.address.clone() }
    }
}

/// Adopts `ids` (or, with `all`, every node that can be) in the diagram
/// `kdl`. `inputs` are variable files both plans of L1-adopt read.
pub fn adopt(
    kdl: &str,
    ids: &[String],
    all: bool,
    opts: &crate::Options,
    inputs: &[PathBuf],
) -> Result<Adopted, Error> {
    let doc = load(kdl, "the diagram")?;
    let mut refused = Vec::new();
    let mut skipped = Vec::new();
    let targets: Vec<&Node> = if all {
        doc.nodes
            .values()
            .filter(|n| k::environment(&doc, n).is_some_and(k::imported))
            .filter(|n| k::NEUTRAL.iter().any(|(t, _)| *t == k::written_ty(n)) && !ka::adopted(&n.props))
            .collect()
    } else {
        ids.iter()
            .filter_map(|id| {
                let n = doc.nodes.get(&NodeId::from(id.as_str()));
                if n.is_none() {
                    refused.push(format!("no node `{id}`"));
                }
                n
            })
            .collect()
    };
    let mut picks = Vec::new();
    for n in targets {
        match pick(&doc, n) {
            Ok(p) => picks.push(p),
            Err(why) if all => skipped.push(why),
            Err(why) => refused.push(why),
        }
    }
    // A raw resource becomes `module.<name>`: its own name, else one with
    // its type (`aws_kms_key.events` and `aws_sns_topic.events` →
    // `module.events`, `module.events_topic`).
    let mut taken: BTreeSet<String> = doc
        .nodes
        .values()
        .filter_map(|x| x.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str))
        .filter(|a| a.starts_with("module."))
        .map(str::to_string)
        .collect();
    for p in picks.iter_mut().filter(|p| p.raw) {
        let ty = k::written_ty(&doc.nodes[&p.id]).trim_start_matches("infra.");
        let free =
            [p.name.clone(), format!("{}_{ty}", p.name)].into_iter().find(|n| !taken.contains(&format!("module.{n}")));
        match free {
            Some(n) => {
                taken.insert(format!("module.{n}"));
                p.name = n;
            }
            None => refused.push(format!(
                "cannot adopt `{}`: rhizo's code for it would be `module.{}`, which exists already",
                p.address, p.name
            )),
        }
    }
    if !refused.is_empty() {
        return Err(Error::NotAdopted(refused));
    }
    let run = Run::new(opts)?;
    // Planned once, after the first shape check: refusals need no OpenTofu.
    let mut original: Option<Original> = None;
    let mut attempt = 0;
    loop {
        attempt += 1;
        if picks.is_empty() {
            let mut why = vec!["nothing to adopt".to_string()];
            why.extend(skipped);
            return Err(Error::NotAdopted(why));
        }
        let new = rewrite(kdl, &doc, &picks)?;
        let after = load(&new, "the adopted diagram")?;
        let errors: Vec<String> = rhizo_kit::check(&after, &rhizo_kit::Kits::builtin())
            .into_iter()
            .filter(|f| f.diagnostic.is_error())
            .map(|f| format!("the adopted diagram fails its kit check: {}", f.diagnostic.message))
            .collect();
        if !errors.is_empty() {
            return Err(Error::NotAdopted(errors));
        }
        opts.step("compiling the adopted code");
        let cmp = Compared::new(&run, &after, &format!("adopted-{attempt}"), inputs)?;
        // rhizo's code must stand where the node does: its own module, or the raw resource it moves.
        let mut wrong = Vec::new();
        for p in &picks {
            if let Err(why) = cmp.shape(p, &doc) {
                wrong.push((p.id.clone(), why));
            }
        }
        if !wrong.is_empty() {
            if !all {
                return Err(Error::NotAdopted(wrong.into_iter().map(|(_, w)| w).collect()));
            }
            picks.retain(|p| !wrong.iter().any(|(id, _)| *id == p.id));
            skipped.extend(wrong.into_iter().map(|(_, w)| w));
            continue;
        }
        if original.is_none() {
            opts.step("planning the original code");
            original = Some(Original::new(&run, &doc, inputs)?);
        }
        opts.step(&match attempt {
            1 => "planning the adopted code".to_string(),
            n => format!("planning the adopted code (try {n}, without the nodes that change)"),
        });
        let l1 = cmp.l1(original.as_ref().expect("planned above"))?;
        // --all adopts only what switches shape without a change: a node whose
        // code would lose an instance, change an attribute or create one is left
        // (adopting it by name accepts the listed changes).
        let owner = |a: &str| {
            picks.iter().position(|p| {
                let inside = |x: &str| a == x || a.starts_with(&format!("{x}.")) || a.starts_with(&format!("{x}["));
                inside(&p.address) || inside(&p.main()) || p.absorbed.iter().any(|s| inside(s))
            })
        };
        if all {
            let mut blame: BTreeMap<usize, Vec<String>> = BTreeMap::new();
            for (a, _) in &l1.lost {
                if let Some(i) = owner(a) {
                    blame.entry(i).or_default().push(format!("would destroy {a}"));
                }
            }
            for (a, _, change) in &l1.changes {
                if let Some(i) = owner(a) {
                    blame.entry(i).or_default().push(format!("changes {a}: {change}"));
                }
            }
            for a in &l1.creates {
                if let Some(i) = owner(a) {
                    blame.entry(i).or_default().push(format!("creates {a}"));
                }
            }
            if !blame.is_empty() {
                for (i, why) in blame.iter().rev() {
                    let p = picks.remove(*i);
                    skipped.push(format!(
                        "`{}`: adopting it {} (L1-adopt; adopt it by name to accept that)",
                        p.address,
                        why.join("; ")
                    ));
                }
                continue;
            }
        }
        let l1 = l1.check()?;
        let mut report = String::from("rhizo adopt\n");
        for p in &picks {
            report += &describe(p, &doc);
        }
        for s in &skipped {
            report += &format!("  not adopted: {s}\n");
        }
        report += &l1.text();
        let quiet = l1.changes.is_empty() && l1.creates.is_empty();
        let nodes = picks.iter().map(|p| p.id.to_string()).collect();
        return Ok(Adopted { kdl: new, report, nodes, quiet });
    }
}

/// Whether node `id` of `doc` can be adopted, as far as the diagram tells
/// (no OpenTofu runs): `Err` names why not. L1-adopt may still refuse it.
pub fn adoptable(doc: &Document, id: &NodeId) -> Result<(), String> {
    let n = doc.nodes.get(id).ok_or_else(|| format!("no node `{id}`"))?;
    pick(doc, n).map(|_| ())
}

/// L1-adopt for a diagram changed since it was imported (or adopted):
/// `before` and `after` compile to code that OpenTofu plans with every
/// original instance accounted for. Returns the report, or refuses with
/// the instances that would be destroyed.
pub fn verify_adopted(before: &str, after: &str, opts: &crate::Options, inputs: &[PathBuf]) -> Result<String, Error> {
    let (a, b) = (load(before, "the original diagram")?, load(after, "the changed diagram")?);
    let run = Run::new(opts)?;
    let original = Original::new(&run, &a, inputs)?;
    Ok(Compared::new(&run, &b, "changed", inputs)?.l1(&original)?.check()?.text())
}

fn load(kdl: &str, what: &str) -> Result<Document, Error> {
    rhizo_lang::load(kdl, rhizo_lang::Syntax::Kdl)
        .into_result()
        .map(|(d, _)| d)
        .map_err(|d| Error::NotAdopted(d.into_iter().map(|d| format!("{what} does not load: {}", d.message)).collect()))
}

fn text<'a>(n: &'a Node, key: &str) -> Option<&'a str> {
    n.props.get(key).and_then(Value::as_str)
}

/// Whether the bucket policy at `policy` is a TLS-only document: a policy
/// document data source with one Deny statement on `aws:SecureTransport`.
/// rhizo's bucket code writes exactly that, and the mocked offline plan cannot
/// compare policy contents, so any other policy would be replaced unseen.
fn tls_only_policy(code: &[&Node], policy: &str) -> bool {
    let at = |a: &str| {
        code.iter().find(|x| x.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str) == Some(a))
    };
    let Some(expr) = at(policy).and_then(|p| p.props.get("body")?.get("policy")?.as_str()) else { return false };
    let Some(document) = expr.strip_suffix(".json").filter(|d| d.starts_with("data.aws_iam_policy_document.")) else {
        return false;
    };
    let statements = at(document).and_then(|d| d.props.get("body")?.get("statement")?.as_array());
    statements.is_some_and(|s| {
        let text = s.first().map(Value::to_string).unwrap_or_default();
        s.len() == 1 && text.contains("aws:SecureTransport") && text.contains("Deny")
    })
}

/// What adopting `n` takes, or why it cannot be adopted.
fn pick(doc: &Document, n: &Node) -> Result<Pick, String> {
    let id = &n.id;
    let env = k::environment(doc, n).filter(|f| k::imported(f));
    let Some(env) = env else { return Err(format!("`{id}` is not in an imported environment")) };
    if ka::adopted(&n.props) {
        return Err(format!("`{id}` is adopted already"));
    }
    let written = k::written_ty(n);
    if !k::NEUTRAL.iter().any(|(t, _)| *t == written) {
        return Err(format!(
            "cannot adopt `{id}`: it is an {written}; only nodes folded into a cloud-neutral infra.* type can be"
        ));
    }
    let (Some(label), Some(address)) =
        (text(n, "name"), n.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str))
    else {
        return Err(format!("cannot adopt `{id}`: it has no imported code"));
    };
    if !label.starts_with(|c: char| c.is_ascii_lowercase())
        || !label.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!(
            "cannot adopt `{address}` yet: rhizo's code names blocks in lower case, so its name `{label}` would change"
        ));
    }
    let body = n.props.get("body").and_then(Value::as_object);
    // What its code folds to now, with its satellites.
    let code: Vec<&Node> = doc
        .nodes
        .values()
        .filter(|x| x.frame == env.id && x.props.contains_key("origin") && !ka::adopted(&x.props))
        .collect();
    let fenv = fold::Env::new(code.iter().map(|x| &x.props));
    let i = code.iter().position(|x| x.id == *id).ok_or_else(|| format!("cannot adopt `{id}`: no code"))?;
    let folded = fenv.fold(i).filter(|f| f.ty == written).ok_or_else(|| {
        format!("cannot adopt `{address}`: its code does not fold to {written} (`rhizo check` names the difference)")
    })?;
    let cloud = k::cloud(env);
    let tf_type = text(n, "tf-type");
    // rhizo's own output, imported, has rhizo's helpers (`local.project`, `local.location`).
    let rhizos_env = raw::locals(env).iter().any(|l| matches!(*l, "project" | "location"));
    if let Some(why) = tf_type.and_then(|t| raw::not_yet(t, rhizos_env)) {
        return Err(format!(
            "cannot adopt `{address}` from shape resources (`{}`) yet: {why}",
            tf_type.unwrap_or_default()
        ));
    }
    let raw = tf_type.and_then(ka::raw).filter(|r| r.cloud == cloud && r.neutral == written);
    // Raw code of another shape, compiled into rhizo's: its provider must be on rhizo's pin.
    if raw.is_some() || (tf_type.is_some() && !rhizos_env) {
        raw::provider_version(env, cloud, address)?;
    }
    let (mut absorbed, mut stay) = (Vec::new(), Vec::new());
    for s in &folded.satellites {
        let manages = match raw {
            Some(r) => {
                let ty = ka::resource_type(s).unwrap_or_default();
                r.satellites.iter().any(|x| x.ty == ty && (x.manages)(n))
            }
            // rhizo's own shape: its code writes the satellites too.
            None => true,
        };
        if manages { absorbed.push(s.clone()) } else { stay.push(s.clone()) }
    }
    if let Some(p) = absorbed.iter().find(|s| ka::resource_type(s) == Some("aws_s3_bucket_policy"))
        && !tls_only_policy(&code, p)
    {
        return Err(format!(
            "cannot adopt `{address}` yet: its bucket policy `{p}` is not the TLS-only policy rhizo's code writes, so adopting would replace its statements"
        ));
    }
    // The resource it sits in (a key's vault or key ring) moves into rhizo's code with it.
    let mut parents: Vec<&Node> = Vec::new();
    for sat in raw.into_iter().flat_map(|r| r.satellites).filter(|s| s.via.is_some()) {
        let via = sat.via.unwrap_or_default();
        let target = body.and_then(|b| b.get(via)).and_then(Value::as_str).and_then(|e| {
            let mut refs = Vec::new();
            Refs(&mut refs).visit_expr(&hcl_edit::parser::parse_expr(e).ok()?);
            refs.iter().find_map(|r| fold::address(r))
        });
        let parent = target
            .as_deref()
            .and_then(|t| code.iter().find(|x| text(x, "tf-type") == Some(sat.ty) && origin_address(x) == Some(t)));
        let Some(parent) = parent else {
            return Err(format!(
                "cannot adopt `{address}` yet: its `{via}` does not name a {} of this environment's code",
                sat.ty
            ));
        };
        absorbed.push(origin_address(parent).unwrap_or_default().to_string());
        parents.push(parent);
    }
    match tf_type {
        Some("azurerm_key_vault_key") => {
            for v in &parents {
                raw::vault_is_rhizos(&code, v, address)?;
            }
        }
        Some("google_storage_bucket") if raw.is_some() => raw::bucket_is_rhizos(&code, n, address)?,
        _ => {}
    }
    let mut keep = Map::new();
    match raw.filter(|r| !r.keep.is_empty()) {
        Some(r) => {
            for kp in r.keep {
                let from = match kp.of {
                    None => body,
                    Some(ty) => parents
                        .iter()
                        .find(|x| text(x, "tf-type") == Some(ty))
                        .and_then(|x| x.props.get("body"))
                        .and_then(Value::as_object),
                };
                if let Some(v) = from.and_then(|b| b.get(kp.arg)).and_then(Value::as_str) {
                    keep.insert(kp.to.to_string(), json!(if kp.list { format!("[{v}]") } else { v.to_string() }));
                }
            }
        }
        None => {
            for key in ka::keep_keys(cloud, written) {
                if let Some(v) = body.and_then(|b| b.get(*key)).filter(|v| v.is_string()) {
                    keep.insert((*key).to_string(), v.clone());
                }
            }
        }
    }
    // rhizo's Google code names the project: the resource's own, else the provider's.
    if cloud == "gcp" && tf_type.is_some() && !rhizos_env {
        let own = keep.contains_key("project_id") || keep.contains_key("project");
        if !own && raw::provider_arg(env, "google", "project").is_none() {
            return Err(format!(
                "cannot adopt `{address}` yet: rhizo's code names the project, and neither its code nor the environment's google provider configuration does"
            ));
        }
    }
    let module = match (tf_type, text(n, "source")) {
        (None, Some(s)) => Some((s.to_string(), text(n, "version").unwrap_or_default().to_string())),
        _ => None,
    };
    let routes = match (written, cloud) {
        ("infra.api", "aws") => match body.and_then(|b| b.get("routes")).and_then(Value::as_str) {
            Some(text) => api_routes(doc, &env.id, address, text)?,
            None => Vec::new(),
        },
        ("infra.api", "azure") => azure_routes(doc, &env.id, address, body)?,
        _ => Vec::new(),
    };
    // rhizo writes the node's label into this argument: its value becomes the label.
    let label_from = ka::label_arg(cloud, written).and_then(|arg| {
        let v = body.and_then(|b| b.get(arg)).and_then(Value::as_str)?;
        Some(fold::literal_tree(&json!({ "x": v }))["x"].as_str()?.to_string())
    });
    Ok(Pick {
        routes,
        label_from,
        id: id.clone(),
        frame: env.id.clone(),
        address: address.to_string(),
        name: label.to_string(),
        raw: raw.is_some(),
        absorbed,
        stay,
        keep,
        module,
        unread: folded.unread,
    })
}

/// The routes of an API module call (`routes = { "POST /orders" = {
/// integration = { uri = module.f.lambda_function_arn } } }`), by the node
/// each one integrates.
fn api_routes(doc: &Document, f: &FrameId, address: &str, text: &str) -> Result<Vec<(NodeId, Vec<String>)>, String> {
    let cannot = |why: String| format!("cannot adopt `{address}` yet: {why}");
    let Ok(Expression::Object(o)) = hcl_edit::parser::parse_expr(text) else {
        return Err(cannot("its routes are not a literal map".into()));
    };
    let mut out: Vec<(NodeId, Vec<String>)> = Vec::new();
    for (key, v) in o.iter() {
        let route = match key {
            hcl_edit::expr::ObjectKey::Expression(Expression::String(s)) => s.value().to_string(),
            _ => return Err(cannot("a route key is not a string".into())),
        };
        let mut refs = Vec::new();
        Refs(&mut refs).visit_expr(v.expr());
        let targets: BTreeSet<String> = refs.iter().filter_map(|r| fold::address(r)).collect();
        let [target] = targets.iter().collect::<Vec<_>>()[..] else {
            return Err(cannot(format!("route `{route}` does not integrate exactly one function")));
        };
        let Some(id) = node_at(doc, f, target) else {
            return Err(cannot(format!("route `{route}` goes to `{target}`, which is not a node")));
        };
        match out.iter_mut().find(|(x, _)| x == id) {
            Some((_, rs)) => rs.push(route),
            None => out.push((id.clone(), vec![route])),
        }
    }
    Ok(out)
}

/// The routes of an API Management module call as rhizo writes it: each
/// operation's `display_name` is its route, and its policy's
/// `set-backend-service backend-id` names the function's module.
fn azure_routes(
    doc: &Document,
    f: &FrameId,
    address: &str,
    body: Option<&Map<String, Value>>,
) -> Result<Vec<(NodeId, Vec<String>)>, String> {
    let cannot = |why: String| format!("cannot adopt `{address}` yet: {why}");
    let t = fold::literal_tree(&Value::Object(body.cloned().unwrap_or_default()));
    let mut out: Vec<(NodeId, Vec<String>)> = Vec::new();
    let apis = t.get("apis").and_then(Value::as_object);
    for api in apis.into_iter().flat_map(|m| m.values()) {
        let ops = api.get("operations").and_then(Value::as_object);
        for op in ops.into_iter().flat_map(|m| m.values()) {
            let route = op.get("display_name").and_then(Value::as_str);
            let xml = op.get("policy").and_then(|p| p.get("xml_content")).and_then(Value::as_str);
            let backend = xml
                .and_then(|x| x.split_once("backend-id=\""))
                .and_then(|(_, r)| r.split_once('"'))
                .map(|(b, _)| format!("module.{b}"));
            let (Some(route), Some(target)) = (route, backend) else {
                return Err(cannot("an operation is not one rhizo writes (a literal route and backend)".into()));
            };
            let Some(id) = node_at(doc, f, &target) else {
                return Err(cannot(format!("route `{route}` goes to `{target}`, which is not a node")));
            };
            match out.iter_mut().find(|(x, _)| x == id) {
                Some((_, rs)) => rs.push(route.to_string()),
                None => out.push((id.clone(), vec![route.to_string()])),
            }
        }
    }
    Ok(out)
}

/// Collects the references in an expression as text: `module.f.arn`.
struct Refs<'o>(&'o mut Vec<String>);

impl Visit for Refs<'_> {
    fn visit_traversal(&mut self, t: &hcl_edit::expr::Traversal) {
        if let Expression::Variable(root) = &t.expr {
            let mut s = root.as_str().to_string();
            for op in &t.operators {
                match op.value() {
                    TraversalOperator::GetAttr(a) => s += &format!(".{}", a.as_str()),
                    _ => break,
                }
            }
            self.0.push(s);
        }
        visit_traversal(self, t);
    }
}

/// The report's lines for one adopted node.
fn describe(p: &Pick, doc: &Document) -> String {
    let n = &doc.nodes[&p.id];
    let shape = match (&p.module, p.raw) {
        (Some((s, v)), _) => format!("module {s}@{v}"),
        (None, true) => format!("resources ({})", text(n, "tf-type").unwrap_or_default()),
        (None, false) => "rhizo's own resources".into(),
    };
    let mut out = format!("  adopt {} ({}) from {shape}: rhizo's code at {}\n", p.id, k::written_ty(n), p.main());
    if !p.absorbed.is_empty() {
        out += &format!("    satellites folded into it, moved into rhizo's code: {}\n", p.absorbed.join(", "));
    }
    if !p.stay.is_empty() {
        out += &format!(
            "    satellites kept verbatim (rhizo's code does not manage them with these props): {}\n",
            p.stay.join(", ")
        );
    }
    if !p.keep.is_empty() {
        let keys: Vec<&str> = p.keep.keys().map(String::as_str).collect();
        out += &format!("    kept from its code (names, places and tags of what exists): {}\n", keys.join(", "));
    }
    if !p.unread.is_empty() {
        out += &format!(
            "    not read from its code (set by expressions): {}; rhizo's code uses the props as they are now\n",
            p.unread.join(", ")
        );
    }
    out
}

// --- Rewriting the diagram --------------------------------------------------

/// The diagram with `picks` adopted: their code props gone, `origin`
/// saying where they came from, absorbed satellites removed, and
/// references to moved addresses rewritten in the code that stays.
fn rewrite(kdl: &str, doc: &Document, picks: &[Pick]) -> Result<String, Error> {
    // Addresses that move: raw resources into `module.<label>`, with their outputs.
    let mut moved: Moved = BTreeMap::new();
    for p in picks.iter().filter(|p| p.raw) {
        let ty = text(&doc.nodes[&p.id], "tf-type").unwrap_or_default();
        moved.insert(p.address.clone(), (p.main(), ka::raw(ty).map(|r| r.outputs)));
        for s in &p.absorbed {
            let outputs = ka::resource_type(s).and_then(ka::satellite).map(|x| x.outputs).filter(|o| !o.is_empty());
            moved.insert(s.clone(), (p.main(), outputs));
        }
    }
    let gone: BTreeSet<&NodeId> = picks
        .iter()
        .flat_map(|p| {
            let absorbed = p.absorbed.iter().filter_map(|a| node_at(doc, &p.frame, a));
            std::iter::once(&p.id).chain(absorbed)
        })
        .collect();
    let mut problems = Vec::new();
    let mut ed = Editor::open(kdl);
    let edit =
        |r: Result<(), rhizo_lang::edit::EditError>| r.map_err(|e| Error::Io(format!("rewriting the diagram: {e}")));
    if !moved.is_empty() {
        let frames: BTreeSet<&FrameId> = picks.iter().map(|p| &p.frame).collect();
        for n in doc.nodes.values().filter(|n| frames.contains(&n.frame) && !gone.contains(&n.id)) {
            if let Some(body) = n.props.get("body") {
                let new = rewrite_body(body, &moved, &n.id.to_string(), &mut problems);
                if new != *body {
                    edit(ed.set_prop(&n.id, "body", &new))?;
                }
            }
        }
        for f in doc.frames.iter().filter(|f| frames.contains(&f.id)) {
            if let Some(Value::Array(blocks)) = f.props.get("blocks") {
                let new: Vec<Value> = blocks
                    .iter()
                    .map(|b| {
                        let mut b = b.clone();
                        let what = block_title(&b);
                        if let Some(body) = b.get("body") {
                            b["body"] = rewrite_body(body, &moved, &what, &mut problems);
                        }
                        b
                    })
                    .collect();
                if new != *blocks {
                    edit(ed.set_frame_prop(&f.id, "blocks", &Value::Array(new)))?;
                }
            }
        }
    }
    if !problems.is_empty() {
        return Err(Error::NotAdopted(problems));
    }
    for p in picks {
        for (to, routes) in &p.routes {
            let at = ed.document().edges.iter().position(|e| e.from == p.id && e.to == *to);
            let Some(at) = at else {
                return Err(Error::NotAdopted(vec![format!(
                    "cannot adopt `{}`: its routes go to `{to}`, but no edge joins them",
                    p.address
                )]));
            };
            let v = match routes.as_slice() {
                [one] => json!(one),
                _ => json!(routes),
            };
            edit(ed.set_edge_prop(at, "route", &v))?;
        }
        if let Some(l) = &p.label_from {
            edit(ed.rename(&p.id, l))?;
        }
        let mut origin = json!({ "address": p.address, "shape": "rhizo" });
        if p.name != ka::label(&p.address) {
            origin["name"] = json!(p.name);
        }
        if !p.absorbed.is_empty() {
            origin["also"] = json!(p.absorbed);
        }
        if !p.keep.is_empty() {
            origin["keep"] = Value::Object(p.keep.clone());
        }
        edit(ed.set_prop(&p.id, "origin", &origin))?;
        for key in ["tf-type", "mode", "name", "source", "version", "body"] {
            edit(ed.remove_prop(&p.id, key))?;
        }
    }
    for p in picks {
        for s in &p.absorbed {
            if let Some(id) = node_at(doc, &p.frame, s) {
                edit(ed.delete(id))?;
            }
        }
    }
    Ok(ed.source().to_string())
}

fn origin_address(n: &Node) -> Option<&str> {
    n.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str)
}

/// The node of frame `f` whose code is at `address`.
fn node_at<'d>(doc: &'d Document, f: &FrameId, address: &str) -> Option<&'d NodeId> {
    doc.nodes
        .values()
        .find(|n| {
            n.frame == *f
                && n.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str) == Some(address)
        })
        .map(|n| &n.id)
}

/// `output "bucket"`, for messages.
fn block_title(b: &Value) -> String {
    let labels = b.get("labels").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
    std::iter::once(b.get("type").and_then(Value::as_str).unwrap_or_default().to_string())
        .chain(labels.map(|l| format!("\"{l}\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

type Moved<'a> = BTreeMap<String, (String, Option<&'a [(&'a str, &'a str)]>)>;

/// A body (strings are expressions, arrays nested blocks) with references
/// to moved addresses rewritten.
fn rewrite_body(v: &Value, moved: &Moved, what: &str, problems: &mut Vec<String>) -> Value {
    match v {
        Value::String(e) => match rewrite_expr(e, moved) {
            Ok(s) => Value::String(s),
            Err(why) => {
                problems.push(format!("cannot adopt: {what} {why}"));
                v.clone()
            }
        },
        Value::Array(a) => Value::Array(a.iter().map(|x| rewrite_body(x, moved, what, problems)).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, x)| (k.clone(), rewrite_body(x, moved, what, problems))).collect())
        }
        _ => v.clone(),
    }
}

/// An HCL expression with each reference to a moved address rewritten
/// (`aws_s3_bucket.b.arn` → `module.b.s3_bucket_arn`; a bare address, as
/// in `depends_on`, → `module.b`). The rest of the text stays as written.
fn rewrite_expr(text: &str, moved: &Moved) -> Result<String, String> {
    let Ok(e) = hcl_edit::parser::parse_expr(text) else { return Ok(text.to_string()) };
    let mut found = Finder { moved, out: Vec::new() };
    found.visit_expr(&e);
    let mut out = text.to_string();
    let mut splices = found.out.into_iter().collect::<Result<Vec<_>, _>>()?;
    splices.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    for (r, new) in splices {
        out.replace_range(r, &new);
    }
    Ok(out)
}

struct Finder<'m> {
    moved: &'m Moved<'m>,
    out: Vec<Result<(std::ops::Range<usize>, String), String>>,
}

impl Visit for Finder<'_> {
    fn visit_traversal(&mut self, t: &hcl_edit::expr::Traversal) {
        if let Expression::Variable(root) = &t.expr {
            let names: Vec<(String, Option<std::ops::Range<usize>>)> = t
                .operators
                .iter()
                .map_while(|op| match op.value() {
                    TraversalOperator::GetAttr(a) => Some((a.as_str().to_string(), op.span())),
                    _ => None,
                })
                .collect();
            let start = t.expr.span().map(|s| s.start);
            // `aws_s3_bucket.b`, `module.m`: one name after the root; `data.t.n`: two.
            let len = if matches!(root.as_str(), "data" | "ephemeral") { 2 } else { 1 };
            if let (Some(start), true) = (start, names.len() >= len) {
                let (head, rest) = names.split_at(len);
                let address = std::iter::once(root.as_str().to_string())
                    .chain(head.iter().map(|(n, _)| n.clone()))
                    .collect::<Vec<_>>()
                    .join(".");
                let end = &head[len - 1].1;
                if let (Some((to, outputs)), Some(end)) = (self.moved.get(&address), end) {
                    let found = match (rest.first(), outputs) {
                        (None, _) => Ok((start..end.end, to.clone())),
                        (Some((attr, Some(e))), Some(outs)) => match outs.iter().find(|(a, _)| a == attr) {
                            Some((_, o)) => Ok((start..e.end, format!("{to}.{o}"))),
                            None => Err(format!(
                                "reads `{address}.{attr}`, which rhizo's code for it ({to}) has no output for"
                            )),
                        },
                        (Some((attr, _)), _) => Err(format!(
                            "reads `{address}.{attr}`, whose code adopting moves into {to}, which has no output for it"
                        )),
                    };
                    self.out.push(found);
                }
            }
        }
        visit_traversal(self, t);
    }
}

// --- L1-adopt ----------------------------------------------------------------

/// The original diagram's code, planned offline once.
struct Original {
    plan: Planned,
}

impl Original {
    fn new(run: &Run, doc: &Document, inputs: &[PathBuf]) -> Result<Original, Error> {
        let dir = write_env(run, doc, "original", inputs)?;
        let blocks = Source::read(&dir).map_err(Error::Refused)?.blocks;
        let (plan, schema) = run.plan_with_schema(&dir, &blocks, "original")?;
        dump("original.json", &plan);
        Ok(Original { plan: Planned::new(&plan, &schema) })
    }
}

/// The managed resource instances a plan creates.
#[derive(Default)]
struct Planned {
    /// Address → planned values.
    values: BTreeMap<String, Value>,
    /// Address → the attributes its provider computes and its configuration
    /// does not set: mocked, so they differ by address and mean nothing.
    free: BTreeMap<String, BTreeSet<String>>,
}

impl Planned {
    fn new(plan: &Value, schema: &Value) -> Planned {
        // Attributes each resource type computes, by any provider.
        let mut computed: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for p in schema["provider_schemas"].as_object().into_iter().flatten().map(|(_, p)| p) {
            for (ty, r) in p["resource_schemas"].as_object().into_iter().flatten() {
                let attrs = r["block"]["attributes"].as_object().into_iter().flatten();
                computed
                    .entry(ty)
                    .or_default()
                    .extend(attrs.filter(|(_, a)| a["computed"] == true).map(|(k, _)| k.as_str()));
            }
        }
        // What each resource's configuration sets, by its address without instance keys.
        let mut set: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        fn walk(m: &Value, prefix: &str, set: &mut BTreeMap<String, BTreeSet<String>>) {
            for r in m["resources"].as_array().into_iter().flatten() {
                let keys = r["expressions"].as_object().into_iter().flatten().map(|(k, _)| k.clone());
                set.entry(format!("{prefix}{}", r["address"].as_str().unwrap_or_default())).or_default().extend(keys);
            }
            for (name, call) in m["module_calls"].as_object().into_iter().flatten() {
                walk(&call["module"], &format!("{prefix}module.{name}."), set);
            }
        }
        walk(&plan["configuration"]["root_module"], "", &mut set);
        let mut out = Planned::default();
        for r in plan["resource_changes"].as_array().into_iter().flatten().filter(|r| r["mode"] == "managed") {
            let Some(address) = r["address"].as_str() else { continue };
            let ty = r["type"].as_str().unwrap_or_default();
            let configured = set.get(&without_keys(address));
            let free = computed.get(ty).into_iter().flatten().filter(|a| !configured.is_some_and(|c| c.contains(**a)));
            out.free.insert(address.to_string(), free.map(|a| a.to_string()).collect());
            out.values.insert(address.to_string(), r["change"]["after"].clone());
        }
        out
    }
}

/// `module.a["x"].aws_s3_bucket.b[0]` → `module.a.aws_s3_bucket.b`.
fn without_keys(address: &str) -> String {
    let (mut out, mut depth, mut quoted) = (String::new(), 0, false);
    for c in address.chars() {
        match c {
            '"' if depth > 0 => quoted = !quoted,
            '[' if !quoted => depth += 1,
            ']' if !quoted => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// With `RHIZO_ADOPT_DUMP=<dir>`, the plans L1-adopt compares are written there.
fn dump(name: &str, plan: &Value) {
    if let Some(d) = std::env::var_os("RHIZO_ADOPT_DUMP") {
        let _ = std::fs::write(Path::new(&d).join(name), plan.to_string());
    }
}

/// A changed diagram's code, compiled into a root module.
struct Compared<'r> {
    run: &'r Run,
    dir: PathBuf,
    /// The adopted code's blocks in `rhizo.tf`.
    adopted: Vec<crate::hcl::Block>,
}

/// What L1-adopt found.
#[derive(Debug)]
struct L1Adopt {
    /// (original, adopted) for each instance a `moved` block takes.
    pub moves: Vec<(String, String)>,
    pub unchanged: usize,
    /// (address in the adopted code, from, path: before → after).
    pub changes: Vec<(String, Option<String>, String)>,
    pub creates: Vec<String>,
    /// Original instances the adopted code lacks: (address, why).
    pub lost: Vec<(String, String)>,
    /// Differences in attributes only the (mocked) provider computes.
    pub mocked: usize,
}

impl L1Adopt {
    /// The refusal when an original instance is not accounted for.
    fn check(self) -> Result<L1Adopt, Error> {
        if self.lost.is_empty() {
            return Ok(self);
        }
        let mut why = vec!["L1-adopt: every instance the original code plans must be in the adopted code".to_string()];
        why.extend(self.lost.into_iter().map(|(_, w)| w));
        Err(Error::NotAdopted(why))
    }

    fn text(&self) -> String {
        let mut out = format!(
            "L1-adopt (offline: both codes planned with mocked providers against an empty state): every original instance is accounted for: {} moved, {} at their own address\n",
            self.moves.len(),
            self.unchanged
        );
        for (from, to) in &self.moves {
            out += &format!("  moves {from} -> {to}\n");
        }
        if self.changes.is_empty() {
            out += "  attribute differences: none\n";
        }
        for (at, from, change) in &self.changes {
            let from = from.as_ref().map_or(String::new(), |f| format!(" (from {f})"));
            out += &format!("  adopting changes {at}{from}: {change}\n");
        }
        if self.mocked > 0 {
            out += &format!(
                "  not compared: {} attributes the provider computes and neither code sets (their mocked values differ by address)\n",
                self.mocked
            );
        }
        if self.creates.is_empty() {
            out += "  creates: none\n";
        }
        for c in &self.creates {
            out += &format!("  adopting creates {c}\n");
        }
        out += "Live (a person's step): `tofu plan` against the real state must show only these moves, these in-place updates and these creates; any replace or destroy is a stop. Only a live plan shows whether a provider replaces a resource for a changed attribute.\n";
        out
    }
}

impl<'r> Compared<'r> {
    fn new(run: &'r Run, after: &Document, name: &str, inputs: &[PathBuf]) -> Result<Compared<'r>, Error> {
        let dir = write_env(run, after, name, inputs)?;
        let adopted = Source::read(&dir)
            .map_err(Error::Refused)?
            .blocks
            .into_iter()
            .filter(|b| b.file == rhizo_compile::ADOPTED_FILE)
            .collect();
        Ok(Compared { run, dir, adopted })
    }

    /// Whether rhizo's code for `p` stands where `p` does.
    fn shape(&self, p: &Pick, doc: &Document) -> Result<(), String> {
        let n = &doc.nodes[&p.id];
        let main = p.main();
        let block = self.adopted.iter().find(|b| b.address().as_deref() == Some(main.as_str()));
        let Some(block) = block else {
            let ty = text(n, "tf-type").unwrap_or("module");
            return Err(format!(
                "cannot adopt `{}` from shape resources (`{ty}`) yet: rhizo's code for an {} is not one, and rhizo has no map from it into rhizo's code",
                p.address,
                k::written_ty(n)
            ));
        };
        // Meta-arguments and dynamic blocks rhizo's code would drop or say otherwise.
        let body = n.props.get("body").and_then(Value::as_object);
        for (key, v) in body.into_iter().flatten() {
            if (META.contains(&key.as_str()) || key.starts_with("dynamic ")) && block.body.get(key) != Some(v) {
                return Err(format!(
                    "cannot adopt `{}` yet: it uses `{key}`, which rhizo's code would drop",
                    p.address
                ));
            }
        }
        if let Some((source, version)) = &p.module {
            let (s, v) = (block.literal("source").unwrap_or_default(), block.literal("version").unwrap_or_default());
            let norm = |x: &str| x.to_ascii_lowercase().trim_start_matches("registry.opentofu.org/").to_string();
            if norm(source) != norm(&s) || *version != v {
                return Err(format!(
                    "cannot adopt `{}` from shape module {source}@{version} yet: rhizo's code for it is {s}@{v}",
                    p.address
                ));
            }
        }
        Ok(())
    }

    fn l1(&self, original: &Original) -> Result<L1Adopt, Error> {
        let blocks = Source::read(&self.dir).map_err(Error::Refused)?.blocks;
        let (plan, schema) = self.run.plan_with_schema(&self.dir, &blocks, "adopted")?;
        dump("adopted.json", &plan);
        let moves: Vec<(String, String)> = self
            .adopted
            .iter()
            .filter(|b| b.kind == "moved")
            .filter_map(|b| {
                Some((b.body.get("from")?.as_str()?.trim().to_string(), b.body.get("to")?.as_str()?.trim().to_string()))
            })
            .collect();
        Ok(compare(&original.plan, &Planned::new(&plan, &schema), &moves))
    }
}

/// Writes the root module a diagram's one imported environment compiles to.
fn write_env(run: &Run, doc: &Document, name: &str, inputs: &[PathBuf]) -> Result<PathBuf, Error> {
    let envs: Vec<_> = doc.frames.iter().filter(|f| k::imported(f)).collect();
    let [env] = envs.as_slice() else {
        return Err(Error::NotAdopted(vec![format!(
            "the diagram has {} imported environments; adopt works on exactly one",
            envs.len()
        )]));
    };
    let bundle = rhizo_compile::compile(doc, &rhizo_kit::Kits::builtin(), "adopt.rhizo", "").map_err(|f| {
        Error::NotAdopted(
            f.into_iter().map(|f| format!("the diagram does not compile: {}", f.diagnostic.message)).collect(),
        )
    })?;
    let prefix = format!("{}/", rhizo_compile::environment_dir(env.id.as_str()));
    let dir = run.workdir(name)?;
    let io = |e: std::io::Error| Error::Io(e.to_string());
    for (path, f) in &bundle.files {
        if let Some(rel) = path.strip_prefix(&prefix) {
            std::fs::write(dir.join(rel), &f.contents).map_err(io)?;
        }
    }
    // OpenTofu reads `*.auto.tfvars` (and `.json`) by itself.
    for (i, input) in inputs.iter().enumerate() {
        let json = input.extension().is_some_and(|x| x == "json");
        let name = format!("zz_rhizo_{i}.auto.tfvars{}", if json { ".json" } else { "" });
        std::fs::copy(input, dir.join(name)).map_err(|e| Error::Io(format!("{}: {e}", input.display())))?;
    }
    Ok(dir)
}

/// Where a `moved` block takes an instance: `from` itself, an instance of
/// it (`from[0]`), or something inside a moved module (`from.x`).
fn moved_to(address: &str, moves: &[(String, String)]) -> Option<String> {
    moves.iter().find_map(|(from, to)| {
        let rest = address.strip_prefix(from.as_str())?;
        (rest.is_empty() || rest.starts_with('[') || rest.starts_with('.')).then(|| format!("{to}{rest}"))
    })
}

/// Accounts for every original instance in the adopted plan and lists what
/// changes and what is created.
fn compare(planned: &Planned, adopted: &Planned, moves: &[(String, String)]) -> L1Adopt {
    let (before, after) = (&planned.values, &adopted.values);
    let mut out = L1Adopt {
        moves: Vec::new(),
        unchanged: 0,
        changes: Vec::new(),
        creates: Vec::new(),
        lost: Vec::new(),
        mocked: 0,
    };
    let mut lost = Vec::new();
    let mut paired = BTreeSet::new();
    for (address, values) in before {
        let to = moved_to(address, moves);
        let at = to.clone().unwrap_or_else(|| address.clone());
        let Some(new) = after.get(&at) else {
            lost.push((
                address.clone(),
                match &to {
                    Some(t) => {
                        format!("{address}: moved to {t}, which rhizo's code does not create: it would be destroyed")
                    }
                    None => format!("{address}: not in the adopted code: it would be destroyed"),
                },
            ));
            continue;
        };
        paired.insert(at.clone());
        match &to {
            Some(t) => out.moves.push((address.clone(), t.clone())),
            None => out.unchanged += 1,
        }
        let mut diffs = Vec::new();
        differences("", values, new, &mut diffs);
        for (path, u, v) in diffs {
            // Computed by the provider on both sides, set by neither configuration: a mock.
            let key = path.split(['.', '[']).next().unwrap_or_default();
            let free = |p: &Planned, a: &str| p.free.get(a).is_some_and(|f| f.contains(key));
            if free(planned, address) && free(adopted, &at) {
                out.mocked += 1;
                continue;
            }
            // A value read from another instance's mocked attribute: the same when
            // that instance is the one this side pairs with.
            let (su, sv) = (sources(before, address, &u), sources(after, &at, &v));
            let same = su.iter().any(|(a, x)| {
                let pa = moved_to(a, moves).unwrap_or_else(|| a.to_string());
                sv.iter().any(|(b, y)| *b == pa && x == y)
            });
            if same {
                continue;
            }
            let change = match (su.first(), sv.first()) {
                // The original's code does not set it: its value is the mock's (which
                // another instance's mocked attribute may happen to equal).
                _ if free(planned, address) => format!(
                    "{path}: {} → {} (the original code does not set it: its value is mocked; a live plan shows the real one)",
                    short(&u),
                    short(&v)
                ),
                (Some((a, x)), Some((b, y))) => format!(
                    "{path}: reads {b}.{y} instead of {a}.{x} (their mocked values differ; a live plan shows whether the real ones do)"
                ),
                _ => format!("{path}: {} → {}", short(&u), short(&v)),
            };
            out.changes.push((at.clone(), to.as_ref().map(|_| address.clone()), change));
        }
    }
    out.lost = lost;
    out.creates = after.keys().filter(|a| !paired.contains(*a)).cloned().collect();
    out
}

/// Every place `a` and `b` differ: (path, a, b).
fn differences(path: &str, a: &Value, b: &Value, out: &mut Vec<(String, Value, Value)>) {
    if a == b {
        return;
    }
    let at = |k: &str| if path.is_empty() { k.to_string() } else { format!("{path}.{k}") };
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                differences(&at(key), x.get(key).unwrap_or(&Value::Null), y.get(key).unwrap_or(&Value::Null), out);
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                differences(&format!("{path}[{i}]"), p, q, out);
            }
        }
        _ => out.push((if path.is_empty() { "(all)".into() } else { path.to_string() }, a.clone(), b.clone())),
    }
}

/// The other instances whose top-level attribute holds string `v`: (address, attribute).
fn sources<'p>(plan: &'p BTreeMap<String, Value>, own: &str, v: &Value) -> Vec<(&'p str, &'p str)> {
    let Some(v) = v.as_str().filter(|s| !s.is_empty()) else { return Vec::new() };
    plan.iter()
        .filter(|(a, _)| a.as_str() != own)
        .flat_map(|(a, x)| x.as_object().into_iter().flatten().map(move |(k, y)| (a.as_str(), k.as_str(), y)))
        .filter(|(_, _, y)| y.as_str() == Some(v))
        .map(|(a, k, _)| (a, k))
        .collect()
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 200 { s.chars().take(197).collect::<String>() + "…" } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_tls_only_bucket_policy_is_taken_over() {
        let doc = |statements: &str| {
            format!(
                r#"rhizo version=2
use infra version="0.1"
frame "e" id=e type=infra.environment {{
    node "p" id=aws_s3_bucket_policy.p type=infra.resource {{
        props tf-type="aws_s3_bucket_policy" name="p" {{
            body policy="data.aws_iam_policy_document.d.json"
            origin address="aws_s3_bucket_policy.p" shape="resource" file="main.tf"
        }}
    }}
    node "d" id=data.aws_iam_policy_document.d type=infra.resource {{
        props tf-type="aws_iam_policy_document" mode="data" name="d" {{
            body {{
                statement {{
{statements}
                }}
            }}
            origin address="data.aws_iam_policy_document.d" shape="resource" file="main.tf"
        }}
    }}
}}
"#
            )
        };
        let tls = r#"                    - effect="\"Deny\"" actions="[\"s3:*\"]" {
                        condition {
                            - test="\"Bool\"" variable="\"aws:SecureTransport\"" values="[\"false\"]"
                        }
                    }"#;
        let grant = r#"                    - effect="\"Allow\"" actions="[\"s3:GetObject\"]""#;
        let check = |src: String| {
            let loaded = rhizo_lang::load(&src, rhizo_lang::Syntax::Kdl);
            assert!(!loaded.has_errors(), "{:?}", loaded.diagnostics);
            let code: Vec<&Node> = loaded.document.nodes.values().collect();
            tls_only_policy(&code, "aws_s3_bucket_policy.p")
        };
        assert!(check(doc(tls)));
        assert!(!check(doc(&format!("{tls}\n{grant}"))), "an extra statement would be replaced");
        assert!(!check(doc(grant)));
        assert!(!check(doc(tls).replace("data.aws_iam_policy_document.d.json", "jsonencode({})")));
    }

    #[test]
    fn references_to_moved_addresses_are_rewritten() {
        let outputs: &'static [(&'static str, &'static str)] = &[("arn", "s3_bucket_arn"), ("id", "s3_bucket_id")];
        let mut moved: Moved = BTreeMap::new();
        moved.insert("aws_s3_bucket.receipts".into(), ("module.receipts".into(), Some(outputs)));
        moved.insert("aws_s3_bucket_versioning.receipts".into(), ("module.receipts".into(), None));
        let r = |s: &str| rewrite_expr(s, &moved);
        assert_eq!(r("aws_s3_bucket.receipts.id").unwrap(), "module.receipts.s3_bucket_id");
        assert_eq!(r("\"${aws_s3_bucket.receipts.arn}/*\"").unwrap(), "\"${module.receipts.s3_bucket_arn}/*\"");
        assert_eq!(
            r("[aws_s3_bucket.receipts.arn,  aws_s3_bucket.other.arn]").unwrap(),
            "[module.receipts.s3_bucket_arn,  aws_s3_bucket.other.arn]"
        );
        assert_eq!(r("[aws_s3_bucket_versioning.receipts]").unwrap(), "[module.receipts]");
        assert!(r("aws_s3_bucket.receipts.website_endpoint").unwrap_err().contains("no output"));
        assert!(r("aws_s3_bucket_versioning.receipts.id").unwrap_err().contains("versioning"));
        assert_eq!(r("var.x").unwrap(), "var.x");
    }

    #[test]
    fn every_original_instance_is_accounted_for() {
        let map = |v: &[(&str, Value)]| Planned {
            values: v.iter().map(|(a, x)| (a.to_string(), x.clone())).collect(),
            free: BTreeMap::new(),
        };
        let before = map(&[("aws_s3_bucket.b", json!({"tags": {"a": "1"}})), ("aws_sqs_queue.q", json!({"n": 1}))]);
        let after = map(&[
            ("module.b.aws_s3_bucket.this[0]", json!({"tags": {"a": "2"}})),
            ("module.b.aws_s3_bucket_policy.this[0]", json!({})),
            ("aws_sqs_queue.q", json!({"n": 1})),
        ]);
        let moves = vec![("aws_s3_bucket.b".to_string(), "module.b.aws_s3_bucket.this[0]".to_string())];
        let r = compare(&before, &after, &moves);
        assert_eq!(r.moves, moves);
        assert_eq!(r.unchanged, 1);
        assert_eq!(r.changes.len(), 1, "{:?}", r.changes);
        assert!(r.changes[0].2.starts_with("tags.a: "), "{:?}", r.changes);
        assert_eq!(r.creates, ["module.b.aws_s3_bucket_policy.this[0]"]);
        // Without the move, the bucket would be destroyed.
        match compare(&before, &after, &[]).check() {
            Err(Error::NotAdopted(why)) => {
                assert!(why[1].contains("aws_s3_bucket.b: not in the adopted code"), "{why:?}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(without_keys("module.a[\"x[1]\"].b.c[0]"), "module.a.b.c");
        assert_eq!(
            moved_to("module.a.x.y[0]", &[("module.a".into(), "module.b".into())]).as_deref(),
            Some("module.b.x.y[0]")
        );
        assert_eq!(moved_to("module.ab.x", &[("module.a".into(), "module.b".into())]), None);
    }
}
