//! What the neutral types mean on Azure, and what does not carry over
//! (docs/design/infra-cloud-neutral.md, step 3). The compiler and the rules
//! read the same functions, so a gap the rules accept is one the compiler
//! can emit, and a gap they reject never reaches it.

use rhizo_core::{Document, Frame, Node};
use serde_json::Value;

use super::{FUNCTION, Link, POSTGRES, QUEUE, TABLE, TOPIC, VPC};
use super::{aws_type, environment, key, links, placement, routes, setting, ty};
use crate::Subject;

/// The Function App memory sizes Flex Consumption offers.
pub const FLEX_MEMORY: [i64; 3] = [512, 2048, 4096];

/// The PostgreSQL Flexible Server SKU when the `azure` block names none:
/// general purpose, since burstable SKUs cannot be highly available.
pub const DEFAULT_SKU: &str = "GP_Standard_D2ds_v5";

/// Service Bus holds a message lock for at most five minutes.
pub const MAX_LOCK_S: i64 = 300;

pub use super::Gap;

/// A neutral node's setting on Azure (its `azure` block, else the neutral
/// prop).
pub fn get<'a>(n: &'a Node, key: &str) -> Option<&'a Value> {
    setting(n, "azure", key)
}

pub fn text<'a>(n: &'a Node, key: &str) -> Option<&'a str> {
    get(n, key).and_then(Value::as_str)
}

pub fn int(n: &Node, key: &str, default: i64) -> i64 {
    get(n, key).and_then(Value::as_i64).unwrap_or(default)
}

pub fn flag(n: &Node, key: &str, default: bool) -> bool {
    get(n, key).and_then(Value::as_bool).unwrap_or(default)
}

/// A function's `runtime` as a Flex Consumption runtime name and version:
/// `nodejs22.x` is (`node`, `22`). Ruby and custom runtimes have none.
pub fn flex_runtime(runtime: &str) -> Option<(&'static str, String)> {
    if let Some(v) = runtime.strip_prefix("nodejs").and_then(|r| r.strip_suffix(".x")) {
        return Some(("node", v.to_string()));
    }
    if let Some(v) = runtime.strip_prefix("python") {
        return Some(("python", v.to_string()));
    }
    if let Some(v) = runtime.strip_prefix("java") {
        return Some(("java", v.to_string()));
    }
    if let Some(v) = runtime.strip_prefix("dotnet") {
        return Some(("dotnet-isolated", format!("{v}.0")));
    }
    None
}

/// A function's instance memory: `azure.instance-memory-mb`, else the
/// smallest Flex Consumption size that holds `memory-mb`.
pub fn flex_memory(n: &Node) -> Option<i64> {
    if let Some(m) = n.props.get("azure").and_then(|b| b.get("instance-memory-mb")).and_then(Value::as_i64) {
        return Some(m);
    }
    let want = n.props.get("memory-mb").and_then(Value::as_i64).unwrap_or(256);
    FLEX_MEMORY.into_iter().find(|m| *m >= want)
}

/// Function and database nodes placed in each subnet tier of a network:
/// (network, tier, nodes). On Azure each tier is one subnet, delegated to
/// one service.
pub fn tenants<'a>(doc: &'a Document, f: &Frame) -> Vec<(&'a Node, &'a str, Vec<&'a Node>)> {
    let mut out: Vec<(&Node, &str, Vec<&Node>)> = Vec::new();
    for n in doc.nodes.values().filter(|n| n.frame == f.id && matches!(ty(n), FUNCTION | POSTGRES)) {
        let Some(p) = placement(doc, n) else { continue };
        match out.iter_mut().find(|(v, t, _)| v.id == p.vpc.id && *t == p.tier) {
            Some((_, _, ns)) => ns.push(n),
            None => out.push((p.vpc, p.tier, vec![n])),
        }
    }
    out
}

/// The key that encrypts the environment's Service Bus namespace, when its
/// queues and topics agree on one. `Err` names two that disagree.
pub fn service_bus_key<'a>(doc: &'a Document, f: &Frame) -> Result<Option<&'a Node>, (&'a Node, &'a Node)> {
    let mut found: Option<(&Node, &Node)> = None;
    for n in doc.nodes.values().filter(|n| n.frame == f.id && matches!(ty(n), QUEUE | TOPIC)) {
        let Some(k) = key(doc, n) else { continue };
        match found {
            Some((first, k0)) if k0.id != k.id => return Err((first, n)),
            Some(_) => {}
            None => found = Some((n, k)),
        }
    }
    Ok(found.map(|(_, k)| k))
}

/// Everything in an azure environment that does not carry over as drawn.
/// Errors cannot be compiled faithfully; warnings compile, with the
/// difference named.
pub fn gaps(doc: &Document, f: &Frame) -> Vec<Gap> {
    let mut out = Vec::new();
    let mut push = |n: &Node, error: bool, message: String, help: Option<&str>| {
        out.push(Gap { subject: Subject::Node(n.id.clone()), error, message, help: help.map(str::to_string) });
    };
    for n in doc.nodes.values().filter(|n| n.frame == f.id) {
        if let Some(g) = aws_type(n, f) {
            push(n, true, g.message, g.help.as_deref());
            continue;
        }
        match ty(n) {
            VPC if flag(n, "flow-logs", true) => push(
                n,
                false,
                format!(
                    "[infra.azure.flow-logs] network `{}`: flow logs are not compiled for azure (VNet flow logs need the subscription's Network Watcher and a storage account)",
                    n.id
                ),
                Some("set flow-logs=#false, or add VNet flow logs to the environment by hand"),
            ),
            FUNCTION => {
                let runtime = text(n, "runtime").unwrap_or("nodejs22.x");
                if flex_runtime(runtime).is_none() {
                    push(
                        n,
                        true,
                        format!("[infra.azure.runtime] function `{}`: Flex Consumption has no {runtime} runtime", n.id),
                        Some("use a nodejs, python, java or dotnet runtime"),
                    );
                }
                if flex_memory(n).is_none() {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.azure.memory] function `{}`: Flex Consumption instances have at most 4096 MB",
                            n.id
                        ),
                        Some("set memory-mb to 4096 or less"),
                    );
                }
                if n.props.contains_key("handler") {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.azure.handler] function `{}`: Azure Functions find their entry points in the code, so `handler` is not compiled",
                            n.id
                        ),
                        Some("remove handler, or keep it for aws only in an aws block"),
                    );
                }
            }
            QUEUE => {
                let lock = int(n, "visibility-timeout-s", 30);
                if lock > MAX_LOCK_S {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.azure.lock] queue `{}`: Service Bus locks a message for at most {MAX_LOCK_S} s, not {lock} s",
                            n.id
                        ),
                        Some("set lock-timeout-s to 300 or less; functions renew the lock while they run"),
                    );
                }
                if !flag(n, "dlq", true) {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.azure.dead-letter] queue `{}`: Service Bus always moves a message to the dead-letter subqueue after max-deliveries; dead-letter=#false only keeps expired messages out of it",
                            n.id
                        ),
                        None,
                    );
                }
            }
            TABLE => {
                if let Some(attr) = text(n, "ttl-attribute") {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.azure.ttl] table `{}`: Cosmos DB expires an item by its `ttl` field (seconds after its last write), not by `{attr}` as an epoch time",
                            n.id
                        ),
                        Some("write `ttl` in seconds; the compiled container has time to live on"),
                    );
                }
                if let Some(sort) = text(n, "range-key") {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.azure.sort-key] table `{}`: Cosmos DB has no sort key; `{sort}` is an ordinary field, and items are unique by `id` within a partition",
                            n.id
                        ),
                        Some("use the sort key as the item id, or ORDER BY it in queries"),
                    );
                }
            }
            POSTGRES => {
                let days = int(n, "backup-days", 7);
                if days < 7 {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.azure.backup] database `{}`: PostgreSQL Flexible Server keeps backups for 7 to 35 days, not {days}",
                            n.id
                        ),
                        Some("set backup-days to 7 or more"),
                    );
                }
                let sku = text(n, "sku-name").unwrap_or(DEFAULT_SKU);
                if flag(n, "multi-az", true) && sku.starts_with("B_") {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.azure.sku] database `{}`: burstable SKU {sku} cannot be highly available",
                            n.id
                        ),
                        Some("set high-availability=#false, or a GP_ or MO_ sku-name in the azure block"),
                    );
                }
                if text(n, "engine-version").is_some_and(|v| v.contains('.')) {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.azure.minor-version] database `{}`: Azure chooses the minor version; only the major version is compiled",
                            n.id
                        ),
                        Some("set engine-version to the major version, e.g. \"17\""),
                    );
                }
            }
            _ => {}
        }
    }
    for (net, tier, ns) in tenants(doc, f) {
        let (fs, dbs): (Vec<&Node>, Vec<&Node>) = ns.iter().partition(|n| ty(n) == FUNCTION);
        if let (Some(fun), Some(db)) = (fs.first(), dbs.first()) {
            push(
                db,
                true,
                format!(
                    "[infra.azure.subnet] database `{}` and function `{}` share the {tier} subnet tier of `{}`; on Azure each needs a subnet delegated to its own service",
                    db.id, fun.id, net.id
                ),
                Some("put the database in a database subnet tier"),
            );
        }
    }
    if let Err((a, b)) = service_bus_key(doc, f) {
        push(
            b,
            true,
            format!(
                "[infra.azure.key] `{}` and `{}` are encrypted with different keys, but one Service Bus namespace holds every queue and topic of the environment and has one key",
                a.id, b.id
            ),
            Some("encrypt the environment's queues and topics with one key"),
        );
    }
    for (_, e, api, _, l) in links(doc) {
        if l != Link::Route || environment(doc, api).is_none_or(|e| e.id != f.id) {
            continue;
        }
        for r in routes(doc, e) {
            if r.contains("+}") {
                push(
                    api,
                    true,
                    format!("[infra.azure.route] route `{r}`: API Management has no greedy path parameters"),
                    Some("use `/*` at the end of the path for everything below it"),
                );
            }
        }
    }
    if f.props.contains_key("budget-monthly") {
        let at = Subject::Frame(f.id.clone());
        out.push(Gap {
            subject: at,
            error: false,
            message: format!(
                "[infra.azure.budget] environment `{}`: Azure budgets count in the billing account's currency, which may not be USD",
                f.id
            ),
            help: Some("check the currency of the subscription's billing account".into()),
        });
    }
    out
}
