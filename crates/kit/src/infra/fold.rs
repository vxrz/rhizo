//! Folding imported OpenTofu into cloud-neutral types (docs/design/iac-import.md,
//! phase 2). A folded node is a typed view of verbatim code: it keeps its HCL
//! in `body` (and `tf-type`, `name`, `source`, `origin`), and it gains the
//! neutral props its *literal* arguments state, plus a cloud block for
//! settings only one cloud has. Expressions derive nothing; they are counted.
//!
//! The importer and the `[infra.import.derived]` rule both call [`Env::fold`],
//! so a folded node's props are always what its code says.
//!
//! An [`Env`] is the imported nodes of one environment. Recognizers live per
//! cloud (`fold/aws.rs`, `fold/azure.rs`, `fold/gcp.rs`): one per raw
//! resource type and one per module family rhizo itself emits. Satellites
//! (resources that configure one main resource, like `aws_s3_bucket_versioning`)
//! stay nodes of their own; their meaning folds into the main node's props.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use hcl_edit::expr::{Expression, ObjectKey, TraversalOperator};
use hcl_edit::template::Element;
use hcl_edit::visit::{Visit, visit_traversal};
use rhizo_core::{EdgeKind, Props};
use serde_json::{Map, Value, json};

use super::{FUNCTION, Link, NEUTRAL, POSTGRES, STORES, SUBNET, TOPIC, VPC, link};

mod aws;
mod azure;
mod gcp;

/// Props every imported node carries, folded or not: its code.
pub const CODE_PROPS: [&str; 7] = ["tf-type", "mode", "name", "source", "version", "body", "origin"];

/// Props a folded node may carry that its code does not state: diagram
/// annotations, not settings.
pub const ANNOTATIONS: [&str; 1] = ["sensitive"];

/// What one node's code folds to.
#[derive(Clone, Debug, PartialEq)]
pub struct Fold {
    /// The neutral type, e.g. `infra.bucket`.
    pub ty: &'static str,
    /// The props its literal arguments state (neutral props and cloud blocks).
    pub props: Props,
    /// Props its code sets with an expression, or with a value the kit
    /// cannot hold: not derived.
    pub unread: Vec<String>,
    /// Findings about the code itself: (rule, message).
    pub audit: Vec<(&'static str, String)>,
    /// Addresses of the satellite resources whose meaning folded into it.
    pub satellites: Vec<String>,
}

/// One imported node: its address, its code, and what it refers to.
pub struct Block<'a> {
    pub address: String,
    /// `aws_s3_bucket`; `None` for a module call.
    pub tf_type: Option<&'a str>,
    /// `managed`, `data` or `ephemeral`.
    pub mode: &'a str,
    /// A module call's source, without registry host, in lower case.
    pub source: Option<String>,
    pub props: &'a Props,
    /// The body with every literal argument as its value; anything else is
    /// a marker ([`is_expr`]).
    tree: Value,
    /// (top-level argument, reference) for every reference in the body.
    refs: Vec<(String, String)>,
}

impl Block<'_> {
    fn managed(&self, ty: &str) -> bool {
        self.mode == "managed" && self.tf_type == Some(ty)
    }

    /// The node addresses this block refers to.
    fn targets(&self) -> impl Iterator<Item = String> + '_ {
        self.refs.iter().filter_map(|(_, r)| address(r))
    }

    fn refers_to(&self, a: &str) -> bool {
        self.targets().any(|t| t == a)
    }
}

/// The imported nodes of one environment.
pub struct Env<'a> {
    pub blocks: Vec<Block<'a>>,
    index: BTreeMap<String, usize>,
}

impl<'a> Env<'a> {
    /// `nodes` are the props of an environment's imported nodes, in any order.
    pub fn new(nodes: impl IntoIterator<Item = &'a Props>) -> Env<'a> {
        let mut blocks = Vec::new();
        for props in nodes {
            let text = |k: &str| props.get(k).and_then(Value::as_str);
            let name = text("name").or_else(|| props.get("origin").and_then(|o| o.get("address")?.as_str()));
            let Some(name) = name else { continue };
            let tf_type = text("tf-type");
            let mode = text("mode").unwrap_or("managed");
            let address = match (tf_type, mode) {
                (None, _) => format!("module.{name}"),
                (Some(t), "managed") => format!("{t}.{name}"),
                (Some(t), m) => format!("{m}.{t}.{name}"),
            };
            let source = text("source").filter(|_| tf_type.is_none()).map(normalize_source);
            let body = props.get("body").cloned().unwrap_or(Value::Object(Map::new()));
            let mut refs = Vec::new();
            for (k, v) in body.as_object().into_iter().flatten() {
                for r in refs_in(v) {
                    refs.push((k.clone(), r));
                }
            }
            blocks.push(Block { address, tf_type, mode, source, props, tree: tree(&body), refs });
        }
        let index = blocks.iter().enumerate().map(|(i, b)| (b.address.clone(), i)).collect();
        Env { blocks, index }
    }

    fn get(&self, address: &str) -> Option<&Block<'a>> {
        self.index.get(address).map(|i| &self.blocks[*i])
    }

    /// What block `i` folds to, if rhizo recognizes it.
    pub fn fold(&self, i: usize) -> Option<Fold> {
        let b = &self.blocks[i];
        let mut o = Out::default();
        let ty = match (b.tf_type, &b.source) {
            (Some(_), _) if b.mode != "managed" => None,
            (Some(t), _) if t.starts_with("aws_") => aws::resource(self, b, &mut o),
            (Some(t), _) if t.starts_with("azurerm_") => azure::resource(self, b, &mut o),
            (Some(t), _) if t.starts_with("google_") => gcp::resource(self, b, &mut o),
            (None, Some(s)) => {
                aws::module(s, b, &mut o).or_else(|| azure::module(s, b, &mut o)).or_else(|| gcp::module(s, b, &mut o))
            }
            _ => None,
        }?;
        // Only what the kit can hold: a value out of its range is not derived.
        let def = kits().find_type(ty).expect("neutral types are in the infra kit");
        for key in o.props.keys().cloned().collect::<Vec<_>>() {
            if def.prop_problem(&o.props, &key).is_some() {
                o.props.shift_remove(&key);
                o.unread(&key);
            }
        }
        if ty == "infra.bucket" && o.props.get("versioning") == Some(&Value::Bool(false)) {
            o.audit.push((
                "infra.audit.versioning",
                "versioning is off: an overwritten or deleted object cannot be restored".into(),
            ));
        }
        o.props.sort_keys();
        o.unread.sort();
        Some(Fold { ty, props: o.props, unread: o.unread, audit: o.audit, satellites: o.satellites })
    }

    /// [`Env::fold`] for every block, in order.
    pub fn folds(&self) -> Vec<Option<Fold>> {
        (0..self.blocks.len()).map(|i| self.fold(i)).collect()
    }

    /// The satellites of `main` of type `ty`: resources of that type that
    /// refer to it.
    fn satellites(&self, main: &Block, ty: &str) -> Vec<&Block<'a>> {
        self.blocks
            .iter()
            .filter(|s| s.managed(ty) && s.address != main.address && s.refers_to(&main.address))
            .collect()
    }

    /// The one satellite of type `ty`, recorded as folded into `o`.
    fn satellite(&self, main: &Block, ty: &str, o: &mut Out) -> Sat<'_, 'a> {
        let all = self.satellites(main, ty);
        o.satellites.extend(all.iter().map(|s| s.address.clone()));
        match all.as_slice() {
            [] => Sat::None,
            [one] => Sat::One(one),
            _ => Sat::Many,
        }
    }

    /// The edges between folded blocks, read from their code: references
    /// between two folded blocks (in the direction [`link`] gives them),
    /// generic blocks that join two folded ones (an event source mapping, a
    /// subscription), and grants through IAM (with `access` when the
    /// statements, roles or bindings say it).
    pub fn edges(&self, folds: &[Option<Fold>]) -> Vec<FoldEdge> {
        let ty = |i: usize| folds[i].as_ref().map(|f| aws_ty(f.ty));
        let mut out: BTreeMap<(usize, usize), Option<Link>> = BTreeMap::new();
        let mut add = |from: usize, to: usize, l: Option<Link>| {
            let e = out.entry((from, to)).or_insert(l);
            if e.is_none() {
                *e = l;
            }
        };
        for (i, b) in self.blocks.iter().enumerate() {
            let Some(ti) = ty(i) else { continue };
            for (arg, r) in &b.refs {
                let Some(j) = address(r).and_then(|a| self.index.get(&a).copied()).filter(|j| *j != i) else {
                    continue;
                };
                let Some(tj) = ty(j) else { continue };
                // A function's event source mapping: the queue triggers it.
                if TRIGGER_ARGS.contains(&arg.as_str()) && link(tj, ti) == Some(Link::Trigger) {
                    add(j, i, Some(Link::Trigger));
                } else if let Some(l) = link(ti, tj) {
                    add(i, j, Some(l));
                } else if let Some(l) = link(tj, ti) {
                    add(j, i, Some(l));
                } else {
                    add(i, j, None);
                }
            }
        }
        // Generic blocks that join exactly two folded ones.
        for b in self.blocks.iter().enumerate().filter(|(i, _)| folds[*i].is_none()).map(|(_, b)| b) {
            let joined: BTreeSet<usize> =
                b.targets().filter_map(|a| self.index.get(&a).copied()).filter(|j| folds[*j].is_some()).collect();
            if let [a, c] = joined.iter().copied().collect::<Vec<_>>()[..] {
                let (ta, tc) = (ty(a).unwrap_or_default(), ty(c).unwrap_or_default());
                match (link(ta, tc), link(tc, ta)) {
                    (Some(l), _) if l != Link::Grant => add(a, c, Some(l)),
                    (_, Some(l)) if l != Link::Grant => add(c, a, Some(l)),
                    _ => {}
                }
            }
        }
        // Grants through IAM: function → store.
        let mut access: BTreeMap<(usize, usize), (bool, bool)> = BTreeMap::new();
        for f in (0..self.blocks.len()).filter(|i| ty(*i) == Some(FUNCTION)) {
            for (s, rw) in self.grants(f, folds) {
                let e = access.entry((f, s)).or_default();
                e.0 |= rw.0;
                e.1 |= rw.1;
            }
        }
        // Consuming the queue that triggers it is the trigger's, not a grant.
        let triggers: BTreeSet<(usize, usize)> =
            out.iter().filter(|(_, l)| **l == Some(Link::Trigger)).map(|(k, _)| *k).collect();
        for ((f, s), rw) in &access {
            if *rw == (true, false) && triggers.contains(&(*s, *f)) {
                continue;
            }
            out.entry((*f, *s)).or_insert(Some(Link::Grant)).get_or_insert(Link::Grant);
        }
        out.into_iter()
            .map(|((from, to), link)| {
                let access = (link == Some(Link::Grant))
                    .then(|| access.get(&(from, to)))
                    .flatten()
                    .and_then(|rw| access_word(*rw, ty(to) == Some(TOPIC)));
                FoldEdge { from, to, link, access }
            })
            .collect()
    }

    /// The folded node block `i` sits in: a subnet in its network; a
    /// function or database in the one subnet (or network) its code places
    /// it in.
    pub fn parent(&self, i: usize, folds: &[Option<Fold>]) -> Option<usize> {
        let ty = |j: usize| folds[j].as_ref().map(|f| aws_ty(f.ty));
        let of = |want: &str| -> BTreeSet<usize> {
            self.blocks[i]
                .targets()
                .filter_map(|a| self.index.get(&a).copied())
                .filter(|j| *j != i && ty(*j) == Some(want))
                .collect()
        };
        let one = |s: BTreeSet<usize>| if s.len() == 1 { s.into_iter().next() } else { None };
        match ty(i)? {
            SUBNET => one(of(VPC)),
            FUNCTION | POSTGRES => {
                let subnets = of(SUBNET);
                if subnets.is_empty() {
                    return one(of(VPC));
                }
                // Several subnets of one network: the network.
                let nets: BTreeSet<Option<usize>> = subnets.iter().map(|s| self.parent(*s, folds)).collect();
                match (subnets.len(), nets.into_iter().collect::<Vec<_>>().as_slice()) {
                    (1, _) => subnets.into_iter().next(),
                    (_, [Some(n)]) => Some(*n),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The stores function `f` is granted access to through IAM, with
    /// (read, write).
    fn grants(&self, f: usize, folds: &[Option<Fold>]) -> Vec<(usize, (bool, bool))> {
        let fb = &self.blocks[f];
        // Who the function acts as: itself, and the identities it names.
        let mut keys: BTreeSet<String> = BTreeSet::from([fb.address.clone()]);
        for (_, r) in &fb.refs {
            if let Some(b) = address(r).and_then(|a| self.get(&a)).filter(|b| is_identity(b)) {
                keys.insert(identity_key(r, b));
            }
        }
        // Blocks that grant to those identities, or policies the function names.
        let mut grants: Vec<&Block> = Vec::new();
        for (i, b) in self.blocks.iter().enumerate() {
            if folds[i].is_some() {
                continue;
            }
            let names_identity = b.refs.iter().any(|(_, r)| {
                address(r)
                    .and_then(|a| self.get(&a))
                    .is_some_and(|t| (is_identity(t) || t.address == fb.address) && keys.contains(&identity_key(r, t)))
            });
            if names_identity || (is_policy(b) && fb.refers_to(&b.address)) {
                grants.push(b);
            }
        }
        let mut out = Vec::new();
        let stores = |b: &Block| -> BTreeSet<usize> {
            b.targets()
                .filter_map(|a| self.index.get(&a).copied())
                .filter(|j| folds[*j].as_ref().is_some_and(|x| STORES.contains(&aws_ty(x.ty))))
                .collect()
        };
        // The function's own inline statements (the lambda module's `policy_statements`).
        let own = fb.props.get("body").and_then(|b| b.get("policy_statements")).and_then(Value::as_str);
        let mut statements: Vec<Statement> = own.map(statements_in_text).unwrap_or_default();
        for g in grants {
            let mut docs = vec![g];
            // Policies and role definitions the grant names (two hops: attachment → policy → document).
            for _ in 0..2 {
                let more: Vec<&Block> = docs
                    .iter()
                    .flat_map(|d| d.targets())
                    .filter_map(|a| self.get(&a))
                    .filter(|d| is_policy(d) && !docs.iter().any(|x| x.address == d.address))
                    .collect();
                docs.extend(more);
            }
            for d in &docs {
                statements.extend(statements_of(d));
            }
            // Role names and role definitions: the grant covers every store it names.
            if let Some(rw) = role(g, &docs) {
                for s in docs.iter().flat_map(|d| stores(d)).collect::<BTreeSet<_>>() {
                    out.push((s, rw));
                }
            }
        }
        for st in statements.iter().filter(|s| !s.deny) {
            let rw = classify(&st.actions);
            if rw == (false, false) {
                continue;
            }
            let targets: BTreeSet<usize> = st
                .refs
                .iter()
                .filter_map(|r| address(r).and_then(|a| self.index.get(&a).copied()))
                .filter(|j| folds[*j].as_ref().is_some_and(|x| STORES.contains(&aws_ty(x.ty))))
                .collect();
            out.extend(targets.into_iter().map(|s| (s, rw)));
        }
        out
    }
}

/// A function argument whose references are event sources that trigger it.
const TRIGGER_ARGS: [&str; 1] = ["event_source_mapping"];

/// An edge between two folded blocks (indices into [`Env::blocks`]).
#[derive(Clone, Debug, PartialEq)]
pub struct FoldEdge {
    pub from: usize,
    pub to: usize,
    /// What the edge means; `None` for a reference that means nothing in
    /// the kit (drawn as a dependency).
    pub link: Option<Link>,
    /// `read`, `write` or `read-write` on a grant, when IAM says it.
    pub access: Option<&'static str>,
}

/// The edge kind a link is drawn with, so the kit's port checks pass. `to`
/// is the (neutral) type of the node it ends at.
pub fn edge_kind(l: Option<Link>, to: &str) -> EdgeKind {
    match l {
        Some(Link::Grant) => EdgeKind::Data,
        Some(Link::Route | Link::Trigger) => EdgeKind::Control,
        Some(Link::Subscribe) if aws_ty(to) == FUNCTION => EdgeKind::Control,
        Some(Link::Subscribe) => EdgeKind::Data,
        Some(Link::Place | Link::Encrypt) | None => EdgeKind::Dependency,
    }
}

/// The AWS type a neutral type compiles as (what [`link`] speaks).
fn aws_ty(neutral: &str) -> &'static str {
    NEUTRAL.iter().find(|(n, _)| *n == neutral).map_or("", |(_, a)| a)
}

fn kits() -> &'static crate::Kits {
    static KITS: OnceLock<crate::Kits> = OnceLock::new();
    KITS.get_or_init(crate::Kits::builtin)
}

/// `registry.opentofu.org/Azure/avm-…/azurerm` → `azure/avm-…/azurerm`.
fn normalize_source(s: &str) -> String {
    let s = s.to_ascii_lowercase();
    let s = s.strip_prefix("registry.terraform.io/").or_else(|| s.strip_prefix("registry.opentofu.org/")).unwrap_or(&s);
    s.to_string()
}

enum Sat<'e, 'a> {
    None,
    One(&'e Block<'a>),
    Many,
}

// --- Literal values -----------------------------------------------------------

/// The key of the marker that stands for an expression in a literal tree.
const EXPR: &str = "$expr";

fn expr_marker() -> Value {
    json!({ EXPR: true })
}

/// The marker for a reference: an expression, but one that is set.
fn ref_marker() -> Value {
    json!({ EXPR: "ref" })
}

fn is_expr(v: &Value) -> bool {
    v.get(EXPR).is_some()
}

/// A body's literal arguments as JSON, as the recognizers read them:
/// anything that is not literal is an object with the key `$expr`.
pub fn literal_tree(body: &Value) -> Value {
    tree(body)
}

/// A body as values: literal arguments as JSON, nested blocks as arrays of
/// bodies, everything else (and `dynamic` blocks) as the expression marker.
fn tree(body: &Value) -> Value {
    match body {
        Value::Object(m) => {
            let mut out = Map::new();
            for (k, v) in m {
                match k.strip_prefix("dynamic ") {
                    Some(label) => {
                        out.insert(label.trim_matches('"').to_string(), expr_marker());
                    }
                    None => {
                        out.insert(k.clone(), tree(v));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(blocks) => Value::Array(blocks.iter().map(tree).collect()),
        Value::String(e) => hcl_edit::parser::parse_expr(e).map_or_else(|_| expr_marker(), |e| lit(&e)),
        _ => expr_marker(),
    }
}

/// An expression's value when it is literal, piece by piece.
fn lit(e: &Expression) -> Value {
    match e {
        Expression::Null(_) => Value::Null,
        Expression::Bool(b) => Value::Bool(*b.value()),
        Expression::Number(n) => {
            let n = n.value();
            n.as_i64().map(Value::from).or_else(|| n.as_f64().map(Value::from)).unwrap_or_else(expr_marker)
        }
        Expression::String(s) => Value::String(s.value().clone()),
        Expression::Array(a) => Value::Array(a.iter().map(lit).collect()),
        Expression::Object(o) => {
            let mut m = Map::new();
            for (k, v) in o.iter() {
                let key = match k {
                    ObjectKey::Ident(i) => Some(i.as_str().to_string()),
                    ObjectKey::Expression(Expression::String(s)) => Some(s.value().clone()),
                    ObjectKey::Expression(_) => None,
                };
                match key {
                    Some(k) => {
                        m.insert(k, lit(v.expr()));
                    }
                    // A computed key: the object is not literal.
                    None => return expr_marker(),
                }
            }
            Value::Object(m)
        }
        Expression::Parenthesis(p) => lit(p.inner()),
        Expression::StringTemplate(t) => template(t.iter()),
        Expression::HeredocTemplate(h) => template(h.template.iter()),
        Expression::Traversal(_) => ref_marker(),
        // jsonencode({...}) and toset([...]) hold their argument's value.
        Expression::FuncCall(f)
            if f.name.namespace.is_empty()
                && matches!(f.name.name.as_str(), "jsonencode" | "tolist" | "toset" | "tomap")
                && f.args.iter().count() == 1 =>
        {
            f.args.iter().next().map_or_else(expr_marker, lit)
        }
        _ => expr_marker(),
    }
}

fn template<'t>(elements: impl Iterator<Item = &'t Element>) -> Value {
    let mut s = String::new();
    for e in elements {
        match e {
            Element::Literal(l) => s.push_str(l.value()),
            _ => return expr_marker(),
        }
    }
    Value::String(s)
}

/// What an argument says.
#[derive(Debug, PartialEq)]
enum Arg<'v> {
    Absent,
    Expr,
    Lit(&'v Value),
}

/// The argument at `path` (`versioning_configuration.status`): nested
/// blocks and single-element lists are stepped into.
fn at<'v>(t: &'v Value, path: &str) -> Arg<'v> {
    let mut cur = t;
    for seg in path.split('.') {
        if let Value::Array(items) = cur {
            match items.as_slice() {
                [one] => cur = one,
                _ => return Arg::Expr,
            }
        }
        if is_expr(cur) {
            return Arg::Expr;
        }
        match cur.get(seg) {
            None => return Arg::Absent,
            Some(v) => cur = v,
        }
    }
    if is_expr(cur) || contains_expr(cur) { Arg::Expr } else { Arg::Lit(cur) }
}

fn contains_expr(v: &Value) -> bool {
    match v {
        Value::Object(m) => is_expr(v) || m.values().any(contains_expr),
        Value::Array(a) => a.iter().any(contains_expr),
        _ => false,
    }
}

/// Whether an argument is set (not absent or null); `None` when that
/// depends on an expression.
fn has(t: &Value, key: &str) -> Option<bool> {
    match t.get(key) {
        None | Some(Value::Null) => Some(false),
        // A reference is set; a conditional or a function may be null.
        Some(v) if is_expr(v) => (v[EXPR] == "ref").then_some(true),
        Some(_) => Some(true),
    }
}

/// The items of a list argument or of repeated blocks; `None` when it is an
/// expression.
fn items<'v>(t: &'v Value, key: &str) -> Option<Vec<&'v Value>> {
    match t.get(key) {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(v) if is_expr(v) => None,
        Some(Value::Array(a)) => Some(a.iter().collect()),
        // A map of objects (`queues = { jobs = {…} }`): its values.
        Some(Value::Object(m)) => Some(m.values().collect()),
        Some(_) => None,
    }
}

/// Derived props, collected by a recognizer.
#[derive(Default)]
struct Out {
    props: Props,
    unread: Vec<String>,
    audit: Vec<(&'static str, String)>,
    satellites: Vec<String>,
}

impl Out {
    /// Sets `prop`; `aws.instance-class` goes into the `aws` block.
    fn set(&mut self, prop: &str, v: Value) {
        match prop.split_once('.') {
            Some((cloud, key)) => {
                let block = self.props.entry(cloud.to_string()).or_insert_with(|| json!({}));
                if let Some(m) = block.as_object_mut() {
                    m.insert(key.to_string(), v);
                }
            }
            None => {
                self.props.insert(prop.to_string(), v);
            }
        }
    }

    fn unread(&mut self, prop: &str) {
        if !self.unread.iter().any(|p| p == prop) {
            self.unread.push(prop.to_string());
        }
    }

    /// `prop` from the argument at `path`: when it is absent (or null), the
    /// value `absent` (the provider's or module's default), when it is an
    /// expression nothing, else what `f` makes of the literal.
    fn read(
        &mut self,
        t: &Value,
        path: &str,
        prop: &str,
        absent: Option<Value>,
        f: impl FnOnce(&Value) -> Option<Value>,
    ) {
        match at(t, path) {
            Arg::Absent | Arg::Lit(Value::Null) => {
                if let Some(v) = absent {
                    self.set(prop, v);
                }
            }
            Arg::Expr => self.unread(prop),
            Arg::Lit(v) => match f(v) {
                Some(v) => self.set(prop, v),
                None => self.unread(prop),
            },
        }
    }
}

fn boolean(v: &Value) -> Option<Value> {
    match v {
        Value::Bool(b) => Some(Value::Bool(*b)),
        Value::String(s) if s == "true" || s == "false" => Some(Value::Bool(s == "true")),
        _ => None,
    }
}

fn integer(v: &Value) -> Option<Value> {
    match v {
        Value::Number(n) => n.as_i64().map(Value::from),
        Value::String(s) => s.parse::<i64>().ok().map(Value::from),
        _ => None,
    }
}

fn string(v: &Value) -> Option<Value> {
    match v {
        Value::String(s) => Some(Value::String(s.clone())),
        Value::Number(n) => Some(Value::String(n.to_string())),
        _ => None,
    }
}

/// `true` when a literal equals `want`.
fn is(want: &'static str) -> impl Fn(&Value) -> Option<Value> {
    move |v| v.as_str().map(|s| Value::Bool(s == want))
}

/// A list of strings, e.g. allowed origins.
fn strings(v: &Value) -> Option<Value> {
    let a = v.as_array()?;
    a.iter().all(Value::is_string).then(|| v.clone())
}

/// A map of literal strings, e.g. environment variables.
fn string_map(v: &Value) -> Option<Value> {
    let m = v.as_object()?;
    let out: Option<Map<String, Value>> = m.iter().map(|(k, v)| Some((k.clone(), string(v)?))).collect();
    out.map(Value::Object)
}

/// `"604800s"` → 604800.
fn seconds(v: &Value) -> Option<i64> {
    v.as_str()?.strip_suffix('s')?.parse().ok()
}

/// An ISO 8601 duration (`PT120S`, `P14D`, `PT1M`) in seconds.
fn iso_seconds(v: &Value) -> Option<i64> {
    let s = v.as_str()?.strip_prefix('P')?;
    let (days, time) = s.split_once('T').unwrap_or((s, ""));
    let mut total = 0i64;
    let mut num = String::new();
    for (part, units) in [(days, [('D', 86400), ('W', 604800), ('Y', -1)]), (time, [('H', 3600), ('M', 60), ('S', 1)])]
    {
        for c in part.chars() {
            if c.is_ascii_digit() {
                num.push(c);
            } else {
                let (_, mult) = units.iter().find(|(u, _)| *u == c)?;
                if *mult < 0 {
                    return None;
                }
                total += num.parse::<i64>().ok()? * mult;
                num.clear();
            }
        }
        if !num.is_empty() {
            return None;
        }
    }
    Some(total)
}

/// One lifecycle entry of the neutral `lifecycle` prop.
fn expiry(prefix: &str, days: i64) -> Value {
    let mut m = Map::new();
    if !prefix.is_empty() {
        m.insert("prefix".into(), json!(prefix));
    }
    m.insert("expire-after-days".into(), json!(days));
    Value::Object(m)
}

// --- References ---------------------------------------------------------------

/// Every reference in a body value, as text: `aws_s3_bucket.b["x"].arn`.
fn refs_in(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match v {
        Value::String(e) => {
            if let Ok(e) = hcl_edit::parser::parse_expr(e) {
                Refs(&mut out).visit_expr(&e);
            }
        }
        Value::Array(a) => a.iter().for_each(|b| out.extend(refs_in(b))),
        Value::Object(m) => m.values().for_each(|b| out.extend(refs_in(b))),
        _ => {}
    }
    out
}

struct Refs<'o>(&'o mut Vec<String>);

impl Visit for Refs<'_> {
    fn visit_traversal(&mut self, t: &hcl_edit::expr::Traversal) {
        if let Expression::Variable(root) = &t.expr {
            let mut s = root.as_str().to_string();
            for op in &t.operators {
                match op.value() {
                    TraversalOperator::GetAttr(a) => s += &format!(".{}", a.as_str()),
                    TraversalOperator::Index(Expression::String(k)) => s += &format!("[{:?}]", k.value()),
                    TraversalOperator::Index(Expression::Number(n)) => s += &format!("[{}]", n.value()),
                    TraversalOperator::LegacyIndex(n) => s += &format!("[{}]", n.value()),
                    _ => break,
                }
            }
            self.0.push(s);
        }
        visit_traversal(self, t);
    }
}

/// The node address a reference points into: `aws_s3_bucket.b["x"].arn` →
/// `aws_s3_bucket.b`, `module.vpc.vpc_id` → `module.vpc`,
/// `data.aws_iam_policy_document.p.json` → `data.aws_iam_policy_document.p`.
pub fn address(r: &str) -> Option<String> {
    let head = r.split('[').next().unwrap_or(r);
    let parts: Vec<&str> = head.split('.').collect();
    let n = match parts[0] {
        "var" | "local" | "each" | "count" | "path" | "self" | "terraform" => return None,
        "data" | "ephemeral" => 3,
        _ => 2,
    };
    (parts.len() >= n).then(|| parts[..n].join("."))
}

// --- Identities and grants -------------------------------------------------------

/// A block an identity is: a role, a managed identity, a service account.
fn is_identity(b: &Block) -> bool {
    matches!(b.tf_type, Some("aws_iam_role" | "azurerm_user_assigned_identity" | "google_service_account"))
        || b.source.as_deref().is_some_and(|s| {
            s.contains("avm-res-managedidentity-userassignedidentity")
                || s.starts_with("terraform-google-modules/service-accounts/")
        })
}

/// Which identity a reference names: the block, or for a module that makes
/// several (`service_accounts_map["worker"]`) the entry.
fn identity_key(r: &str, b: &Block) -> String {
    if b.tf_type.is_none()
        && let Some(end) = r.find(']')
    {
        return r[..=end].to_string();
    }
    b.address.clone()
}

/// Blocks that hold permissions a grant refers to.
fn is_policy(b: &Block) -> bool {
    matches!(
        b.tf_type,
        Some(
            "aws_iam_policy_document"
                | "aws_iam_policy"
                | "azurerm_role_definition"
                | "azurerm_cosmosdb_sql_role_definition"
                | "google_project_iam_custom_role"
        )
    )
}

/// One IAM statement: its actions and what it refers to.
struct Statement {
    actions: Vec<String>,
    refs: Vec<String>,
    deny: bool,
}

/// The statements of a policy document, an inline policy or a role policy.
fn statements_of(b: &Block) -> Vec<Statement> {
    let body = b.props.get("body");
    let mut out = Vec::new();
    // `statement { actions = […] resources = […] }` blocks (aws_iam_policy_document).
    for st in body.and_then(|v| v.get("statement")).and_then(Value::as_array).into_iter().flatten() {
        let t = tree(st);
        let actions = match at(&t, "actions") {
            Arg::Lit(v) => v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        let deny = matches!(at(&t, "effect"), Arg::Lit(Value::String(e)) if e == "Deny");
        out.push(Statement { actions, refs: refs_in(st), deny });
    }
    // `policy = jsonencode({ Statement = […] })`.
    if let Some(p) = body.and_then(|v| v.get("policy")).and_then(Value::as_str) {
        out.extend(statements_in_text(p));
    }
    out
}

/// Statements of an expression: `jsonencode({ Statement = [{ Action, Resource }] })`
/// or the lambda module's `{ name = { actions, resources } }`.
fn statements_in_text(text: &str) -> Vec<Statement> {
    let Ok(e) = hcl_edit::parser::parse_expr(text) else { return Vec::new() };
    let mut out = Vec::new();
    collect_statements(&e, &mut out);
    out
}

fn collect_statements(e: &Expression, out: &mut Vec<Statement>) {
    match e {
        Expression::FuncCall(f) => f.args.iter().for_each(|a| collect_statements(a, out)),
        Expression::Array(a) => a.iter().for_each(|x| collect_statements(x, out)),
        Expression::Parenthesis(p) => collect_statements(p.inner(), out),
        Expression::Object(o) => {
            let key = |k: &ObjectKey| match k {
                ObjectKey::Ident(i) => i.as_str().to_ascii_lowercase(),
                ObjectKey::Expression(Expression::String(s)) => s.value().to_ascii_lowercase(),
                _ => String::new(),
            };
            let actions = o.iter().find(|(k, _)| matches!(key(k).as_str(), "action" | "actions"));
            if let Some((_, a)) = actions {
                let acts = match lit(a.expr()) {
                    Value::String(s) => vec![s],
                    Value::Array(v) => v.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                    _ => Vec::new(),
                };
                let deny = o.iter().any(|(k, v)| key(k) == "effect" && lit(v.expr()) == json!("Deny"));
                let mut refs = Vec::new();
                for (_, v) in o.iter() {
                    Refs(&mut refs).visit_expr(v.expr());
                }
                out.push(Statement { actions: acts, refs, deny });
            } else {
                o.iter().for_each(|(_, v)| collect_statements(v.expr(), out));
            }
        }
        _ => {}
    }
}

/// (read, write) from IAM actions or permissions of any cloud.
fn classify(actions: &[String]) -> (bool, bool) {
    let (mut r, mut w) = (false, false);
    for a in actions.iter().map(|a| a.to_ascii_lowercase()) {
        let verb = a.rsplit([':', '/', '.']).next().unwrap_or(&a).to_string();
        let both = a == "*" || a.ends_with(":*") || a == "rds-db:connect";
        // Consuming from a queue reads it, even though it deletes the message.
        let consume = matches!(a.as_str(), "sqs:deletemessage" | "sqs:changemessagevisibility");
        let read = consume
            || ["get", "list", "describe", "receive", "query", "scan", "batchget", "select", "read", "head"]
                .iter()
                .any(|p| verb.starts_with(p));
        let write = !consume
            && (["put", "delete", "send", "publish", "update", "batchwrite", "create", "write", "abort", "restore"]
                .iter()
                .any(|p| verb.starts_with(p))
                || a.ends_with("/add/action")
                || a.ends_with("/send/action"));
        r |= both || read;
        w |= both || write;
    }
    (r, w)
}

/// What a grant's role allows, from its literal name or a role definition.
fn role(g: &Block, docs: &[&Block]) -> Option<(bool, bool)> {
    let t = &g.tree;
    // Azure: a built-in role by name.
    if let Arg::Lit(Value::String(name)) = at(t, "role_definition_name") {
        return azure_role(name);
    }
    // Google: a predefined role.
    if let Arg::Lit(Value::String(name)) = at(t, "role")
        && let Some(rw) = gcp_role(name)
    {
        return Some(rw);
    }
    // Cosmos DB built-in data roles, by their fixed ids.
    if let Some(Value::String(id)) = g.props.get("body").and_then(|b| b.get("role_definition_id")) {
        if id.contains("00000000-0000-0000-0000-000000000001") {
            return Some((true, false));
        }
        if id.contains("00000000-0000-0000-0000-000000000002") {
            return Some((true, true));
        }
    }
    // A custom role: its permissions.
    let mut perms = Vec::new();
    for d in docs.iter().filter(|d| d.address != g.address) {
        for path in ["permissions.data_actions", "permissions.actions", "permissions", "permissions.data_actions"] {
            if let Arg::Lit(Value::Array(a)) = at(&d.tree, path) {
                perms.extend(a.iter().filter_map(Value::as_str).map(str::to_string));
            }
        }
    }
    let rw = classify(&perms);
    (rw != (false, false)).then_some(rw)
}

fn azure_role(name: &str) -> Option<(bool, bool)> {
    Some(match name {
        "Storage Blob Data Reader" | "Azure Service Bus Data Receiver" | "Cosmos DB Built-in Data Reader" => {
            (true, false)
        }
        "Azure Service Bus Data Sender" => (false, true),
        "Storage Blob Data Contributor"
        | "Storage Blob Data Owner"
        | "Azure Service Bus Data Owner"
        | "Cosmos DB Built-in Data Contributor" => (true, true),
        _ => return None,
    })
}

fn gcp_role(name: &str) -> Option<(bool, bool)> {
    Some(match name.strip_prefix("roles/")? {
        "storage.objectViewer"
        | "storage.legacyObjectReader"
        | "pubsub.subscriber"
        | "pubsub.viewer"
        | "datastore.viewer" => (true, false),
        "storage.objectCreator" | "pubsub.publisher" => (false, true),
        "storage.objectAdmin"
        | "storage.objectUser"
        | "storage.admin"
        | "pubsub.editor"
        | "datastore.user"
        | "datastore.owner"
        | "cloudsql.client"
        | "cloudsql.instanceUser" => (true, true),
        _ => return None,
    })
}

/// The `access` word for (read, write); a function can only publish to a topic.
fn access_word(rw: (bool, bool), topic: bool) -> Option<&'static str> {
    match rw {
        (false, true) => Some("write"),
        _ if topic => None,
        (true, false) => Some("read"),
        (true, true) => Some("read-write"),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests;

// Types the recognizers name.
const BUCKET_N: &str = "infra.bucket";
const QUEUE_N: &str = "infra.queue";
const TOPIC_N: &str = "infra.topic";
const KEY_N: &str = "infra.key";
const FUNCTION_N: &str = "infra.function";
const TABLE_N: &str = "infra.table";
const DATABASE_N: &str = "infra.database";
const NETWORK_N: &str = "infra.network";
const SUBNET_N: &str = "infra.subnet";
const API_N: &str = "infra.api";
