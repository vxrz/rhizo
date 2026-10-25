//! The `infra` kit's graph model and rules. The OpenTofu compiler reads the
//! model through the same functions, so what `rhizo check` accepts is what
//! gets compiled.
//!
//! An edge means something by the types it joins ([`link`]), not by its
//! kind, which the port check already keeps consistent.

use rhizo_core::{Document, Edge, EdgeKind, Frame, Node, NodeId};
use rhizo_lang::Diagnostic;
use serde_json::Value;

use crate::{Finding, Subject};

pub const KIT: &str = "infra";

pub const ENVIRONMENT: &str = "infra.environment";
pub const VPC: &str = "aws.vpc";
pub const SUBNET: &str = "aws.subnet";
pub const BUCKET: &str = "aws.s3.bucket";
pub const TABLE: &str = "aws.dynamodb.table";
pub const POSTGRES: &str = "aws.rds.postgres";
pub const QUEUE: &str = "aws.sqs.queue";
pub const TOPIC: &str = "aws.sns.topic";
pub const KEY: &str = "aws.kms.key";
pub const FUNCTION: &str = "aws.lambda.function";
pub const HTTP_API: &str = "aws.apigateway.http";
/// Any resource or data source, carried verbatim (imported code).
pub const RESOURCE: &str = "infra.resource";
/// Any module call, carried verbatim (imported code).
pub const MODULE: &str = "infra.module";

/// The cloud-neutral types and the AWS type each one compiles as on aws
/// (docs/design/infra-cloud-neutral.md). [`ty`] answers with the AWS type, so
/// the rules and the compiler treat both alike.
pub const NEUTRAL: [(&str, &str); 10] = [
    ("infra.network", VPC),
    ("infra.subnet", SUBNET),
    ("infra.bucket", BUCKET),
    ("infra.table", TABLE),
    ("infra.database", POSTGRES),
    ("infra.queue", QUEUE),
    ("infra.topic", TOPIC),
    ("infra.key", KEY),
    ("infra.function", FUNCTION),
    ("infra.api", HTTP_API),
];

/// Props a neutral type names differently: (AWS name, neutral name).
const RENAMED: [(&str, &str); 10] = [
    ("multi-az", "high-availability"),
    ("hash-key", "partition-key"),
    ("hash-key-type", "partition-key-type"),
    ("range-key", "sort-key"),
    ("range-key-type", "sort-key-type"),
    ("fifo", "ordered"),
    ("visibility-timeout-s", "lock-timeout-s"),
    ("dlq", "dead-letter"),
    ("max-receives", "max-deliveries"),
    ("azs", "zones"),
];

/// The cloud an environment's neutral types compile for.
pub fn cloud(f: &Frame) -> &str {
    f.props.get("cloud").and_then(Value::as_str).unwrap_or("aws")
}

/// A node's type as written, before neutral types map to AWS ones.
pub fn written_ty(n: &Node) -> &str {
    n.ty.as_ref().map_or("", |t| t.as_str())
}

fn is_neutral(ty: &str) -> bool {
    NEUTRAL.iter().any(|(n, _)| *n == ty)
}

/// The word for a node in messages: the AWS one, or the neutral one when the
/// node has a neutral type.
fn says(n: &Node, aws: &'static str, neutral: &'static str) -> &'static str {
    if is_neutral(written_ty(n)) { neutral } else { aws }
}

/// A prop by its AWS name. On a neutral node it comes from the `aws` block
/// or from the prop's neutral name.
pub fn prop<'a>(n: &'a Node, key: &str) -> Option<&'a Value> {
    if !is_neutral(written_ty(n)) {
        return n.props.get(key);
    }
    setting(n, "aws", key)
}

/// A neutral node's setting for one cloud: from the block named after the
/// cloud (`azure { sku-name … }`), else the neutral prop. `key` may be the
/// AWS name of a renamed prop.
pub fn setting<'a>(n: &'a Node, cloud: &str, key: &str) -> Option<&'a Value> {
    let neutral = RENAMED.iter().find(|(a, _)| *a == key).map_or(key, |(_, b)| b);
    n.props.get(cloud).and_then(|b| b.get(key)).or_else(|| n.props.get(neutral))
}

/// The clouds a neutral node can carry a settings block for.
pub const CLOUDS: [&str; 3] = ["aws", "azure", "gcp"];

/// A generic type: its HCL is carried verbatim in its `body` prop.
pub fn is_generic(ty: &str) -> bool {
    matches!(ty, RESOURCE | MODULE)
}

/// Whether an environment was imported from OpenTofu code: it then compiles
/// back as that root module (preserve mode).
pub fn imported(f: &Frame) -> bool {
    f.props.contains_key("origin")
}

/// Things that hold data: they can be encrypted with a KMS key and be
/// marked `sensitive`.
pub const STORES: [&str; 5] = [BUCKET, TABLE, POSTGRES, QUEUE, TOPIC];

/// What an edge between two infra nodes means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// HTTP API → function: routes plus the invoke permission.
    Route,
    /// Function → store: a least-privilege IAM statement (`access` prop).
    Grant,
    /// Function or database → VPC or subnet tier: where it runs.
    Place,
    /// Queue → function: an event source mapping.
    Trigger,
    /// Topic → queue or function: a subscription.
    Subscribe,
    /// Store → KMS key: encrypted with that key.
    Encrypt,
}

pub fn link(from: &str, to: &str) -> Option<Link> {
    Some(match (from, to) {
        (HTTP_API, FUNCTION) => Link::Route,
        (FUNCTION, t) if STORES.contains(&t) => Link::Grant,
        (FUNCTION | POSTGRES, VPC | SUBNET) => Link::Place,
        (QUEUE, FUNCTION) => Link::Trigger,
        (TOPIC, QUEUE | FUNCTION) => Link::Subscribe,
        (s, KEY) if STORES.contains(&s) => Link::Encrypt,
        _ => return None,
    })
}

/// A node's type, with a neutral type answered as the AWS type it compiles as.
pub fn ty(n: &Node) -> &str {
    let t = written_ty(n);
    NEUTRAL.iter().find(|(n, _)| *n == t).map_or(t, |(_, aws)| aws)
}

pub fn is_infra(ty: &str) -> bool {
    ty.starts_with("aws.") || ty.starts_with("infra.")
}

pub fn text<'a>(n: &'a Node, key: &str) -> Option<&'a str> {
    prop(n, key).and_then(Value::as_str)
}

pub fn int(n: &Node, key: &str, default: i64) -> i64 {
    prop(n, key).and_then(Value::as_i64).unwrap_or(default)
}

pub fn flag(n: &Node, key: &str, default: bool) -> bool {
    prop(n, key).and_then(Value::as_bool).unwrap_or(default)
}

/// The environment a node belongs to: its frame, when that is typed
/// `infra.environment`.
pub fn environment<'a>(doc: &'a Document, n: &Node) -> Option<&'a Frame> {
    doc.frame(&n.frame).filter(|f| f.ty.as_ref().is_some_and(|t| t.as_str() == ENVIRONMENT))
}

/// Edges between two typed nodes with what they mean, in document order.
pub fn links(doc: &Document) -> impl Iterator<Item = (usize, &Edge, &Node, &Node, Link)> {
    doc.edges.iter().enumerate().filter_map(|(i, e)| {
        let (a, b) = (doc.nodes.get(&e.from)?, doc.nodes.get(&e.to)?);
        Some((i, e, a, b, link(ty(a), ty(b))?))
    })
}

/// Where a function or database runs.
pub struct Placement<'a> {
    pub vpc: &'a Node,
    /// The subnet tier node, when it was placed in or linked to one.
    pub subnet: Option<&'a Node>,
    /// `public`, `private` or `database`.
    pub tier: &'a str,
}

/// The default tier inside a VPC: databases get the database subnets,
/// everything else the private ones.
fn default_tier(n: &Node) -> &'static str {
    if ty(n) == POSTGRES { "database" } else { "private" }
}

/// A node's placement: the nearest enclosing subnet tier or VPC, else the
/// first dependency edge to one.
pub fn placement<'a>(doc: &'a Document, n: &'a Node) -> Option<Placement<'a>> {
    let in_vpc = |s: &'a Node| doc.ancestors(&s.id).filter_map(|a| doc.nodes.get(a)).find(|a| ty(a) == VPC);
    let at = |t: &'a Node| -> Option<Placement<'a>> {
        match ty(t) {
            SUBNET => Some(Placement { vpc: in_vpc(t)?, subnet: Some(t), tier: text(t, "tier").unwrap_or("private") }),
            VPC => Some(Placement { vpc: t, subnet: None, tier: default_tier(n) }),
            _ => None,
        }
    };
    let enclosing = doc.ancestors(&n.id).filter_map(|a| doc.nodes.get(a)).find(|a| matches!(ty(a), SUBNET | VPC));
    if let Some(p) = enclosing.and_then(at) {
        return Some(p);
    }
    doc.edges
        .iter()
        .filter(|e| e.from == n.id && e.kind.is_none_or(|k| k == EdgeKind::Dependency))
        .filter_map(|e| doc.nodes.get(&e.to))
        .find_map(at)
}

/// The KMS key a store is encrypted with.
pub fn key<'a>(doc: &'a Document, n: &Node) -> Option<&'a Node> {
    doc.edges.iter().filter(|e| e.from == n.id).filter_map(|e| doc.nodes.get(&e.to)).find(|k| ty(k) == KEY)
}

/// What a function may do with a store: (read, write). The `access` edge
/// prop is `read`, `write` or `read-write`; queues and topics default to
/// `write`, everything else to `read-write`.
pub fn access(e: &Edge, store: &Node) -> Result<(bool, bool), String> {
    let default = if matches!(ty(store), QUEUE | TOPIC) { "write" } else { "read-write" };
    let a = e.props.get("access").map_or(Some(default), Value::as_str);
    let rw = match a {
        Some("read") => (true, false),
        Some("write") => (false, true),
        Some("read-write") => (true, true),
        _ => return Err(format!("access {} is not \"read\", \"write\" or \"read-write\"", e.props["access"])),
    };
    if ty(store) == TOPIC && rw != (false, true) {
        return Err("a function can only publish to a topic: use access=\"write\"".into());
    }
    Ok(rw)
}

/// The `data.operation`s a function implements: dependency edges from it.
pub fn operations<'a>(doc: &'a Document, function: &Node) -> Vec<&'a Node> {
    doc.edges
        .iter()
        .filter(|e| e.from == function.id && e.kind.is_none_or(|k| k == EdgeKind::Dependency))
        .filter_map(|e| doc.nodes.get(&e.to))
        .filter(|o| ty(o) == crate::data::OPERATION)
        .collect()
}

/// Route keys of an API → function edge: its `route` prop (one route, or a
/// list of them), else one per operation the function implements, else
/// `$default` (everything).
pub fn routes(doc: &Document, e: &Edge) -> Vec<String> {
    match e.props.get("route") {
        Some(Value::String(r)) => return vec![r.clone()],
        Some(Value::Array(rs)) => return rs.iter().map(|r| r.as_str().unwrap_or_default().to_string()).collect(),
        _ => {}
    }
    let ops = doc.nodes.get(&e.to).map(|f| operations(doc, f)).unwrap_or_default();
    if ops.is_empty() {
        return vec!["$default".into()];
    }
    ops.iter()
        .map(|o| {
            let method = text(o, "method").unwrap_or("get").to_uppercase();
            // OpenAPI `{id}` and API Gateway `{id}` agree.
            format!("{method} {}", text(o, "path").unwrap_or("/"))
        })
        .collect()
}

fn valid_route(r: &str) -> bool {
    const METHODS: [&str; 8] = ["ANY", "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
    r == "$default" || r.split_once(' ').is_some_and(|(m, p)| METHODS.contains(&m) && p.starts_with('/'))
}

/// Whether a store holds sensitive data: marked so, or a data edge brings
/// in a `data.entity` with a field marked `pii="sensitive"`.
pub fn sensitive(doc: &Document, store: &Node) -> bool {
    if flag(store, "sensitive", false) {
        return true;
    }
    doc.edges
        .iter()
        .filter(|e| e.to == store.id && e.kind.is_none_or(|k| k == EdgeKind::Data))
        .filter_map(|e| doc.nodes.get(&e.from))
        .filter(|en| ty(en) == crate::data::ENTITY)
        .flat_map(|en| en.children.iter().filter_map(|c| doc.nodes.get(c)))
        .any(|f| ty(f) == crate::data::FIELD && text(f, "pii") == Some("sensitive"))
}

/// `10.20.0.0/16` as (address, prefix length).
pub fn parse_cidr(s: &str) -> Option<(u32, u32)> {
    let (ip, len) = s.split_once('/')?;
    let ip: std::net::Ipv4Addr = ip.parse().ok()?;
    Some((u32::from(ip), len.parse().ok().filter(|l| *l <= 32)?))
}

/// The rules of the `infra` kit (design doc §3.4, layer 4).
pub(crate) struct InfraRules;

impl crate::Rules for InfraRules {
    fn kit(&self) -> &'static str {
        KIT
    }

    fn check(&self, doc: &Document, out: &mut Vec<Finding>) {
        out.extend(rules(doc));
    }
}

fn rules(doc: &Document) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |subject: Subject, error: bool, msg: String, help: Option<String>| {
        let mut d = if error { Diagnostic::error(msg, None) } else { Diagnostic::warning(msg, None) };
        d.help = help;
        out.push(Finding { subject, diagnostic: d });
    };
    let node = |id: &NodeId| doc.nodes.get(id);

    for f in doc.frames.iter().filter(|f| f.ty.as_ref().is_some_and(|t| t.as_str() == ENVIRONMENT)) {
        if imported(f) {
            // Its backend, if any, is one of its verbatim blocks.
            imported_rules(doc, f, &mut push);
        } else if cloud(f) == "azure" {
            if azure_state(f).is_none() {
                push(
                    Subject::Frame(f.id.clone()),
                    false,
                    format!("[infra.state.remote] environment `{}` keeps its state on the machine that runs tofu", f.id),
                    Some("add props { azure state-storage-account=\"<existing account>\" state-container=\"tfstate\" } for remote, locked state".into()),
                );
            }
            for g in azure::gaps(doc, f) {
                push(g.subject, g.error, g.message, g.help);
            }
        } else if cloud(f) == "gcp" {
            if gcp_state(f).is_none() {
                push(
                    Subject::Frame(f.id.clone()),
                    false,
                    format!(
                        "[infra.state.remote] environment `{}` keeps its state on the machine that runs tofu",
                        f.id
                    ),
                    Some(
                        "add props { gcp state-bucket=\"<existing bucket>\" } for remote, encrypted, locked state"
                            .into(),
                    ),
                );
            }
            for g in gcp::gaps(doc, f) {
                push(g.subject, g.error, g.message, g.help);
            }
        } else if !f.props.contains_key("state-bucket") {
            push(
                Subject::Frame(f.id.clone()),
                false,
                format!("[infra.state.remote] environment `{}` keeps its state on the machine that runs tofu", f.id),
                Some("add props state-bucket=\"<existing bucket>\" for remote, encrypted, locked state".into()),
            );
        }
    }

    for n in doc.nodes.values() {
        let at = || Subject::Node(n.id.clone());
        let env = environment(doc, n).filter(|f| !imported(f) && is_neutral(written_ty(n)));
        if let Some(f) = env
            && let Some(key) = fold::CODE_PROPS.iter().find(|k| n.props.contains_key(**k))
        {
            push(
                at(),
                true,
                format!(
                    "[infra.import.code] `{}` has `{key}`, which only nodes of an imported environment carry; environment `{}` is designed, so it would be ignored",
                    n.id, f.id
                ),
                Some("remove the imported props, or move the node into its imported environment".into()),
            );
        }
        // Imported code is audited; adopted nodes are rhizo's code now.
        let audit = environment(doc, n).is_some_and(imported) && !adopt::adopted(&n.props);
        for (f, other) in env.into_iter().flat_map(|f| CLOUDS.iter().map(move |c| (f, *c))) {
            if other != cloud(f) && n.props.contains_key(other) {
                push(
                    at(),
                    false,
                    format!(
                        "[infra.cloud.setting] `{}`: its {other} settings apply only on {other}, and environment `{}` targets {}",
                        n.id,
                        f.id,
                        cloud(f)
                    ),
                    Some(format!("remove the {other} block, or keep it for when the environment targets {other}")),
                );
            }
        }
        match ty(n) {
            VPC => {
                let cidr = text(n, "cidr").unwrap_or("10.0.0.0/16");
                if let Some((ip, len)) = parse_cidr(cidr)
                    && len > 0
                    && ip & (u32::MAX >> len) != 0
                {
                    push(
                        at(),
                        true,
                        format!("[infra.vpc.cidr] `{}`: {cidr} has host bits set", n.id),
                        Some("use the network address, e.g. 10.20.0.0/16".into()),
                    );
                }
                let tiers: Vec<&str> = n
                    .children
                    .iter()
                    .filter_map(node)
                    .filter(|c| ty(c) == SUBNET)
                    .filter_map(|c| text(c, "tier"))
                    .collect();
                // Imported subnets are per zone, not tiers.
                for (i, t) in tiers.iter().enumerate().filter(|_| !audit) {
                    if tiers[..i].iter().filter(|x| *x == t).count() == 1 {
                        push(
                            at(),
                            true,
                            format!(
                                "[infra.subnet.tier] {} `{}` has more than one {t} subnet tier",
                                says(n, "VPC", "network"),
                                n.id
                            ),
                            Some("a subnet tier already spans every availability zone: use one node per tier".into()),
                        );
                    }
                }
            }
            POSTGRES => match placement(doc, n) {
                // Imported code may place it in ways the diagram does not draw (a subnet group).
                None if audit => {}
                None => push(
                    at(),
                    true,
                    format!("[infra.rds.placement] database `{}` is not in a {}", n.id, says(n, "VPC", "network")),
                    Some(format!(
                        "put it inside an {} (tier=\"database\" or \"private\"), or add: edge {} {} kind=dependency",
                        says(n, "aws.subnet", "infra.subnet"),
                        n.id,
                        says(n, "<vpc id>", "<network id>")
                    )),
                ),
                Some(p) if p.tier == "public" => push(
                    at(),
                    true,
                    format!(
                        "[infra.rds.private] database `{}` is in a public subnet, reachable from the internet",
                        n.id
                    ),
                    Some("place it in a private or database subnet tier".into()),
                ),
                Some(_) => {}
            },
            _ => {}
        }
        if STORES.contains(&ty(n)) && sensitive(doc, n) && key(doc, n).is_none() {
            push(
                at(),
                true,
                format!(
                    "[infra.kms.sensitive] `{}` holds sensitive data but has no customer-managed {}",
                    n.id,
                    says(n, "KMS key", "key")
                ),
                Some(format!(
                    "add a node of type {} and: edge {} <key id> kind=dependency",
                    says(n, "aws.kms.key", "infra.key"),
                    n.id
                )),
            );
        }
        if ty(n) == FUNCTION
            && let Some(p) = placement(doc, n)
        {
            let nat = text(p.vpc, "nat").unwrap_or("single");
            let calls_apis = doc
                .edges
                .iter()
                .filter(|e| e.from == n.id)
                .filter_map(|e| node(&e.to))
                .any(|s| matches!(ty(s), BUCKET | TABLE | QUEUE | TOPIC));
            // On gcp every subnet has Private Google Access: Google APIs need no NAT.
            let gcp = environment(doc, n).is_some_and(|f| cloud(f) == "gcp");
            if calls_apis && !gcp && (p.tier != "private" || nat == "none") {
                let why = if p.tier != "private" {
                    format!("{} subnets have no outbound route for functions", p.tier)
                } else {
                    format!("{} `{}` has nat=\"none\"", says(p.vpc, "VPC", "network"), p.vpc.id)
                };
                let services = match environment(doc, n).map(cloud) {
                    Some("azure") => "Azure",
                    _ => "AWS",
                };
                push(
                    at(),
                    false,
                    format!("[infra.lambda.egress] `{}` cannot reach the {services} services it uses: {why}", n.id),
                    Some(format!("run it in a private subnet tier of a {} with NAT", says(n, "VPC", "network"))),
                );
            }
        }
    }

    let mut routes_seen: Vec<(&NodeId, String)> = Vec::new();
    for (i, e) in doc.edges.iter().enumerate() {
        let (Some(a), Some(b)) = (node(&e.from), node(&e.to)) else { continue };
        if !(is_infra(ty(a)) && is_infra(ty(b))) {
            continue;
        }
        let at = || Subject::Edge(i);
        let (ea, eb) = (environment(doc, a).map(|f| &f.id), environment(doc, b).map(|f| &f.id));
        if ea != eb {
            push(
                at(),
                true,
                format!("[infra.edge.environment] `{}` → `{}` crosses environments", a.id, b.id),
                Some("each environment is deployed on its own; connect things within one environment".into()),
            );
            continue;
        }
        let Some(l) = link(ty(a), ty(b)) else {
            // Edges to generic nodes, and in imported code, are views of references in its HCL.
            if is_generic(ty(a)) || is_generic(ty(b)) || environment(doc, a).is_some_and(imported) {
                continue;
            }
            push(
                at(),
                false,
                format!(
                    "[infra.edge.meaning] {} → {} has no meaning in the infra kit and is not compiled",
                    ty(a),
                    ty(b)
                ),
                None,
            );
            continue;
        };
        match l {
            Link::Grant => {
                if let Err(msg) = access(e, b) {
                    push(at(), true, format!("[infra.edge.access] `{}` → `{}`: {msg}", a.id, b.id), None);
                }
                if ty(b) == POSTGRES {
                    let (pa, pb) = (placement(doc, a), placement(doc, b));
                    let same = matches!((&pa, &pb), (Some(x), Some(y)) if x.vpc.id == y.vpc.id);
                    if pb.is_some() && !same {
                        push(
                            at(),
                            true,
                            format!(
                                "[infra.lambda.vpc] `{}` connects to database `{}` but does not run in its {}",
                                a.id,
                                b.id,
                                says(a, "VPC", "network")
                            ),
                            Some(format!(
                                "add: edge {} {} kind=dependency",
                                a.id,
                                pb.map_or("<vpc id>".into(), |p| p.vpc.id.to_string())
                            )),
                        );
                    }
                }
            }
            // Azure Functions renew a Service Bus message lock while they run.
            Link::Trigger if environment(doc, a).is_some_and(|f| cloud(f) == "azure") => {}
            Link::Trigger => {
                let (vis, timeout) = (int(a, "visibility-timeout-s", 30), int(b, "timeout-s", 10));
                if vis < timeout {
                    push(
                        at(),
                        true,
                        format!(
                            "[infra.sqs.visibility] queue `{}` hides a message for {vis} s, less than the {timeout} s timeout of `{}`",
                            a.id, b.id
                        ),
                        Some(format!(
                            "set visibility-timeout-s to at least {} (AWS recommends six times the timeout)",
                            timeout
                        )),
                    );
                }
            }
            // Imported routes are in the API's code, not on its edges.
            Link::Route if environment(doc, a).is_some_and(imported) && !adopt::adopted(&a.props) => {}
            Link::Route => {
                for r in routes(doc, e) {
                    if !valid_route(&r) {
                        push(
                            at(),
                            true,
                            format!("[infra.api.route] route `{r}` is not `METHOD /path` or `$default`"),
                            Some("e.g. props route=\"POST /orders\"".into()),
                        );
                    } else if routes_seen.iter().any(|(api, x)| *api == &a.id && *x == r) {
                        push(at(), true, format!("[infra.api.route] API `{}` has route `{r}` twice", a.id), None);
                    } else {
                        routes_seen.push((&a.id, r));
                    }
                }
            }
            Link::Place | Link::Subscribe | Link::Encrypt => {}
        }
    }
    audit(doc, out)
}

/// Kit rules on the nodes and edges of imported environments report on
/// existing code: warnings, prefixed so, that never block compiling it back.
fn audit(doc: &Document, findings: Vec<Finding>) -> Vec<Finding> {
    let in_import = |id: &NodeId| doc.nodes.get(id).and_then(|n| environment(doc, n)).is_some_and(imported);
    let adopted = |id: &NodeId| doc.nodes.get(id).is_some_and(|n| adopt::adopted(&n.props));
    findings
        .into_iter()
        .map(|mut f| {
            let imported_code = match &f.subject {
                Subject::Node(id) => in_import(id) && !adopted(id),
                Subject::Edge(i) => {
                    doc.edges.get(*i).is_some_and(|e| in_import(&e.from) && !adopted(&e.from) && !adopted(&e.to))
                }
                Subject::Frame(_) | Subject::Document => false,
            };
            let d = &mut f.diagnostic;
            if imported_code
                && !d.message.starts_with("[infra.import.")
                && !d.message.starts_with("[infra.adopt.")
                && !d.message.starts_with("audit: ")
            {
                let mut w = Diagnostic::warning(format!("audit: {}", d.message), None);
                w.help = Some(format!(
                    "{}this reports on the imported code: change the code and re-import",
                    d.help.as_ref().map_or(String::new(), |h| format!("{h}; "))
                ));
                *d = w;
            }
            f
        })
        .collect()
}

/// Imported environments: kit types only as folded views of their code,
/// whose props are what the code says ([`fold`]); audit findings on that code.
fn imported_rules(doc: &Document, f: &Frame, push: &mut impl FnMut(Subject, bool, String, Option<String>)) {
    let nodes: Vec<&Node> = doc.nodes.values().filter(|n| n.frame == f.id).collect();
    // Adopted nodes are rhizo's code: not folded, and their props are editable.
    let code: Vec<&Node> =
        nodes.iter().copied().filter(|n| n.props.contains_key("origin") && !adopt::adopted(&n.props)).collect();
    let env = fold::Env::new(code.iter().map(|n| &n.props));
    for n in nodes.iter().filter(|n| is_infra(ty(n)) && !is_generic(ty(n))) {
        let at = || Subject::Node(n.id.clone());
        if adopt::adopted(&n.props) && is_neutral(written_ty(n)) {
            continue;
        }
        let Some(i) = code.iter().position(|c| c.id == n.id).filter(|_| is_neutral(written_ty(n))) else {
            push(
                at(),
                true,
                format!(
                    "[infra.import.kit-type] `{}` is a {} in the imported environment `{}`, which compiles back as its original code",
                    n.id,
                    written_ty(n),
                    f.id
                ),
                Some(
                    "use infra.resource or infra.module here, or put kit-typed nodes in an environment of their own"
                        .into(),
                ),
            );
            continue;
        };
        let help = || {
            Some("props of an imported node are read from its code: edit the HCL in `body`, or change the code and re-import".to_string())
        };
        let Some(folded) = env.fold(i).filter(|x| x.ty == written_ty(n)) else {
            let is = env
                .fold(i)
                .map_or("not code rhizo recognizes as one".to_string(), |x| format!("code that folds to {}", x.ty));
            push(
                at(),
                true,
                format!("[infra.import.derived] `{}` is an {}, but its code is {is}", n.id, written_ty(n)),
                Some(format!(
                    "use type={}",
                    env.fold(i).map_or(if n.props.contains_key("tf-type") { RESOURCE } else { MODULE }, |x| x.ty)
                )),
            );
            continue;
        };
        let own = n
            .props
            .iter()
            .filter(|(k, _)| !fold::CODE_PROPS.contains(&k.as_str()) && !fold::ANNOTATIONS.contains(&k.as_str()));
        let keys: std::collections::BTreeSet<&str> =
            own.map(|(k, _)| k.as_str()).chain(folded.props.keys().map(String::as_str)).collect();
        for key in keys {
            let (have, want) = (n.props.get(key), folded.props.get(key));
            if have == want {
                continue;
            }
            // Name the setting inside a cloud block.
            let name = match (have, want) {
                (Some(Value::Object(a)), Some(Value::Object(b))) => {
                    let inner = a.keys().chain(b.keys()).find(|k| a.get(*k) != b.get(*k));
                    inner.map_or(key.to_string(), |k| format!("{key}.{k}"))
                }
                (Some(Value::Object(a)), None) | (None, Some(Value::Object(a))) => {
                    a.keys().next().map_or(key.to_string(), |k| format!("{key}.{k}"))
                }
                _ => key.to_string(),
            };
            let code = want.map_or_else(|| "does not set it".to_string(), |v| format!("says {v}"));
            let diagram = have.map_or_else(|| "leaves it out".to_string(), |v| format!("says {v}"));
            push(
                at(),
                true,
                format!(
                    "[infra.import.derived] `{}`: prop `{name}` is read-only: its code {code}, the diagram {diagram}",
                    n.id
                ),
                help(),
            );
        }
        for (rule, msg) in &folded.audit {
            push(
                at(),
                false,
                format!("audit: [{rule}] `{}`: {msg}", n.id),
                Some("this reports on the imported code: change the code and re-import".into()),
            );
        }
    }
    adopted_rules(doc, f, push);
}

/// Adopted nodes in an imported environment: rhizo's code for them refers
/// to the nodes they are linked with as rhizo's modules, so those must be
/// rhizo's code too (adopted, or imported module calls at `module.<name>`).
fn adopted_rules(doc: &Document, f: &Frame, push: &mut impl FnMut(Subject, bool, String, Option<String>)) {
    let module_shaped = |n: &Node| adopt::rhizo_shaped(n, cloud(f));
    for (i, _, a, b, l) in links(doc) {
        if a.frame != f.id || b.frame != f.id {
            continue;
        }
        // Which end's code refers to the other: the start of every link (a function to what it
        // is granted or placed in, a store to its key, an API to its functions), and the end of
        // a subscription, trigger or route; a queue to the function it triggers only on gcp
        // (a push subscription).
        let from_refers = l != Link::Trigger || cloud(f) == "gcp";
        let (refers, other) = match (adopt::adopted(&a.props), adopt::adopted(&b.props), l) {
            (true, _, _) if from_refers && !module_shaped(b) => (a, b),
            (_, true, Link::Subscribe | Link::Trigger | Link::Route) if !module_shaped(a) => (b, a),
            _ => continue,
        };
        let at = other.props.get("origin").and_then(|o| o.get("address")).and_then(Value::as_str).unwrap_or_default();
        push(
            Subject::Edge(i),
            true,
            format!(
                "[infra.adopt.edge] `{}` → `{}`: rhizo's code for adopted `{}` refers to `{}` as rhizo's own code, but `{}` is verbatim code of another shape at `{at}`",
                a.id, b.id, refers.id, other.id, other.id
            ),
            Some(format!("adopt `{}` too, or remove the edge", other.id)),
        );
    }
    // A satellite left verbatim when it was adopted, which rhizo's code now writes too.
    let nodes: Vec<&Node> = doc.nodes.values().filter(|n| n.frame == f.id).collect();
    for n in nodes.iter().filter(|n| adopt::adopted(&n.props)) {
        let address = adopt::moves(&n.props).into_iter().next().unwrap_or_default();
        let Some(raw) = adopt::resource_type(&address).and_then(adopt::raw) else { continue };
        let main = format!("module.{}.", adopt::code_name(&n.props).unwrap_or_default());
        for x in nodes.iter().filter(|x| !adopt::adopted(&x.props)) {
            let ty = x.props.get("tf-type").and_then(Value::as_str).unwrap_or_default();
            let Some(sat) = raw.satellites.iter().find(|s| s.ty == ty) else { continue };
            let refers = x.props.get("body").is_some_and(|b| b.to_string().contains(&main));
            if refers && (sat.manages)(n) {
                push(
                    Subject::Node(x.id.clone()),
                    true,
                    format!(
                        "[infra.adopt.satellite] `{}` configures adopted `{}`, whose code now writes a {ty} itself: the two would overwrite each other",
                        x.id, n.id
                    ),
                    Some(format!("delete `{}`, or change the props of `{}` back", x.id, n.id)),
                );
            }
        }
    }
}

/// One finding about an environment for a cloud other than aws: what does
/// not carry over as drawn.
pub struct Gap {
    pub subject: Subject,
    pub error: bool,
    pub message: String,
    pub help: Option<String>,
}

/// An `aws.*` node in an environment for another cloud: those types
/// compile only for aws.
pub fn aws_type(n: &Node, f: &Frame) -> Option<Gap> {
    let written = written_ty(n);
    if !written.starts_with("aws.") {
        return None;
    }
    let neutral = NEUTRAL.iter().find(|(_, a)| *a == written).map(|(n, _)| *n);
    Some(Gap {
        subject: Subject::Node(n.id.clone()),
        error: true,
        message: format!(
            "[infra.cloud.type] `{}` is an {written}, which compiles only for aws; environment `{}` targets {}",
            n.id,
            f.id,
            cloud(f)
        ),
        help: Some(neutral.map_or("use a cloud-neutral infra.* type".to_string(), |t| format!("use type={t}"))),
    })
}

/// A gcp environment's remote state: (bucket, prefix) from its `gcp` block.
pub fn gcp_state(f: &Frame) -> Option<(&str, Option<&str>)> {
    let b = f.props.get("gcp")?;
    Some((b.get("state-bucket")?.as_str()?, b.get("state-prefix").and_then(Value::as_str)))
}

/// An azure environment's remote state: (storage account, container,
/// resource group) from its `azure` block.
pub fn azure_state(f: &Frame) -> Option<(&str, &str, Option<&str>)> {
    let b = f.props.get("azure")?;
    let get = |k: &str| b.get(k).and_then(Value::as_str);
    Some((get("state-storage-account")?, get("state-container").unwrap_or("tfstate"), get("state-resource-group")))
}

pub mod adopt;
pub mod azure;
pub mod fold;
pub mod gcp;

#[cfg(test)]
mod tests;
