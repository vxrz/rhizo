//! What the neutral types mean on Google Cloud, and what does not carry over
//! (docs/design/infra-cloud-neutral.md, step 4). The compiler and the rules
//! read the same functions, so a gap the rules accept is one the compiler
//! can emit, and a gap they reject never reaches it.

use rhizo_core::{Document, Frame, Node};
use serde_json::Value;

use super::{FUNCTION, Gap, HTTP_API, Link, POSTGRES, QUEUE, TABLE};
use super::{aws_type, environment, links, placement, routes, setting, ty};
use crate::Subject;

/// The regions API Gateway runs gateways in
/// (cloud.google.com/api-gateway/docs/deployment-model, 2026-10-22).
pub const GATEWAY_REGIONS: [&str; 10] = [
    "asia-northeast1",
    "australia-southeast1",
    "europe-west1",
    "europe-west2",
    "us-central1",
    "us-east1",
    "us-east4",
    "us-west2",
    "us-west3",
    "us-west4",
];

/// The Cloud SQL tier when the `gcp` block names none: a dedicated core,
/// since shared-core tiers have no SLA.
pub const DEFAULT_TIER: &str = "db-custom-1-3840";

/// Cloud Run functions memory sizes in MB (each comes with its CPU share).
pub const MEMORY_MB: [i64; 9] = [128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768];

/// A Pub/Sub ack deadline, in seconds: at least 10, at most 600.
pub const ACK_S: (i64, i64) = (10, 600);

/// Pub/Sub delivery attempts before dead-lettering: 5 to 100.
pub const DELIVERIES: (i64, i64) = (5, 100);

/// A subscription keeps messages for at least 10 minutes.
pub const MIN_RETENTION_S: i64 = 600;

/// The longest an API Gateway (non-streaming) waits for a backend, and the
/// longest a Pub/Sub push waits for an acknowledgement.
pub const MAX_WAIT_S: i64 = 600;

/// Cloud SQL Enterprise edition keeps transaction logs for point-in-time
/// recovery for at most 7 days.
pub const MAX_LOG_DAYS: i64 = 7;

/// A neutral node's setting on Google Cloud (its `gcp` block, else the
/// neutral prop).
pub fn get<'a>(n: &'a Node, key: &str) -> Option<&'a Value> {
    setting(n, "gcp", key)
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

/// A function's `runtime` as a Cloud Run functions runtime: `nodejs22.x` is
/// `nodejs22`, `python3.12` is `python312`. Custom runtimes have none.
pub fn runtime(runtime: &str) -> Option<String> {
    if let Some(v) = runtime.strip_prefix("nodejs").and_then(|r| r.strip_suffix(".x")) {
        return Some(format!("nodejs{v}"));
    }
    for lang in ["python", "ruby"] {
        if let Some(v) = runtime.strip_prefix(lang) {
            return Some(format!("{lang}{}", v.replace('.', "")));
        }
    }
    ["java", "dotnet"].iter().any(|l| runtime.starts_with(l)).then(|| runtime.to_string())
}

/// A function's memory: the smallest Cloud Run functions size that holds
/// `memory-mb`.
pub fn memory(n: &Node) -> i64 {
    let want = int(n, "memory-mb", 256);
    MEMORY_MB.into_iter().find(|m| *m >= want).unwrap_or(want)
}

/// Where an API's gateway runs: its `gcp` block's region, else the
/// environment's.
pub fn gateway_region<'a>(api: &'a Node, f: &'a Frame) -> &'a str {
    text(api, "region").unwrap_or_else(|| region(f))
}

/// An environment's region.
pub fn region(f: &Frame) -> &str {
    f.props.get("region").and_then(Value::as_str).unwrap_or("us-central1")
}

/// Everything in a gcp environment that does not carry over as drawn.
/// Errors cannot be compiled faithfully; warnings compile, with the
/// difference named.
pub fn gaps(doc: &Document, f: &Frame) -> Vec<Gap> {
    let mut out = Vec::new();
    let mut push = |n: &Node, error: bool, message: String, help: Option<&str>| {
        out.push(Gap { subject: Subject::Node(n.id.clone()), error, message, help: help.map(str::to_string) });
    };
    let waits = |fun: &Node| {
        links(doc)
            .any(|(_, _, a, b, l)| b.id == fun.id && matches!(l, Link::Route | Link::Subscribe) && a.frame == f.id)
    };
    for n in doc.nodes.values().filter(|n| n.frame == f.id) {
        if let Some(g) = aws_type(n, f) {
            push(n, true, g.message, g.help.as_deref());
            continue;
        }
        match ty(n) {
            FUNCTION => {
                let rt = text(n, "runtime").unwrap_or("nodejs22.x");
                if runtime(rt).is_none() {
                    push(
                        n,
                        true,
                        format!("[infra.gcp.runtime] function `{}`: Cloud Run functions have no {rt} runtime", n.id),
                        Some("use a nodejs, python, java, dotnet or ruby runtime"),
                    );
                }
                if placement(doc, n).is_some_and(|p| p.tier == "database") {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.gcp.tier] function `{}`: on Google Cloud the database tier is the private services access range of Cloud SQL, where nothing else runs",
                            n.id
                        ),
                        Some("place the function in the private subnet tier"),
                    );
                }
                let timeout = int(n, "timeout-s", 10);
                if timeout > MAX_WAIT_S && waits(n) {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.gcp.wait] function `{}`: API Gateway and Pub/Sub push wait at most {MAX_WAIT_S} s for it, not its {timeout} s timeout",
                            n.id
                        ),
                        Some("set timeout-s to 600 or less, or answer early and continue in the background"),
                    );
                }
            }
            QUEUE => {
                let pushed: Vec<&Node> = links(doc)
                    .filter(|(_, _, a, _, l)| a.id == n.id && *l == Link::Trigger)
                    .map(|(_, _, _, b, _)| b)
                    .collect();
                let readers = links(doc).any(|(_, e, _, b, l)| {
                    b.id == n.id && l == Link::Grant && super::access(e, b).is_ok_and(|(read, _)| read)
                });
                if pushed.len() > 1 || (!pushed.is_empty() && readers) {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.gcp.trigger] queue `{}`: its Pub/Sub subscription pushes to `{}`, so no other function can be triggered by it or read it",
                            n.id, pushed[0].id
                        ),
                        Some("give each consumer its own queue, fed by a topic"),
                    );
                }
                let (lo, hi) = ACK_S;
                let ack = int(n, "visibility-timeout-s", 30);
                if !(lo..=hi).contains(&ack) {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.gcp.ack-deadline] queue `{}`: a Pub/Sub ack deadline is {lo} to {hi} s, not {ack} s",
                            n.id
                        ),
                        Some("set lock-timeout-s between 10 and 600"),
                    );
                }
                let (lo, hi) = DELIVERIES;
                let tries = int(n, "max-receives", 5);
                if flag(n, "dlq", true) && !(lo..=hi).contains(&tries) {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.gcp.deliveries] queue `{}`: Pub/Sub dead-letters after {lo} to {hi} delivery attempts, not {tries}",
                            n.id
                        ),
                        Some("set max-deliveries between 5 and 100"),
                    );
                }
                let kept = int(n, "retention-s", 345_600);
                if kept < MIN_RETENTION_S {
                    push(
                        n,
                        true,
                        format!(
                            "[infra.gcp.retention] queue `{}`: a Pub/Sub subscription keeps messages for at least {MIN_RETENTION_S} s, not {kept} s",
                            n.id
                        ),
                        Some("set retention-s to 600 or more"),
                    );
                }
            }
            TABLE => {
                // A partition key alone is the document id; only a sort key has no Firestore meaning.
                if let Some(sort) = text(n, "range-key") {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.gcp.keys] table `{}`: Firestore has no sort key; documents are found by their id (the partition key), so `{sort}` is an ordinary field",
                            n.id
                        ),
                        Some("query and order by that field (with an index), or join both keys into the document id"),
                    );
                }
                if let Some(attr) = text(n, "ttl-attribute") {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.gcp.ttl] table `{}`: Firestore expires a document when its `{attr}` timestamp has passed; it must be a timestamp, not epoch seconds",
                            n.id
                        ),
                        Some(&format!("write `{attr}` as a timestamp value")),
                    );
                }
            }
            POSTGRES => {
                if text(n, "engine-version").is_some_and(|v| v.contains('.')) {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.gcp.minor-version] database `{}`: Cloud SQL chooses the minor version; only the major version is compiled",
                            n.id
                        ),
                        Some("set engine-version to the major version, e.g. \"17\""),
                    );
                }
                let days = int(n, "backup-days", 7);
                if days > MAX_LOG_DAYS {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.gcp.pitr] database `{}`: Cloud SQL keeps {days} daily backups, but point-in-time recovery reaches back {MAX_LOG_DAYS} days only",
                            n.id
                        ),
                        Some("keep backup-days, or set it to 7 if older restores are not needed"),
                    );
                }
            }
            HTTP_API => {
                let at = gateway_region(n, f);
                if !GATEWAY_REGIONS.contains(&at) {
                    push(
                        n,
                        true,
                        format!("[infra.gcp.gateway-region] API `{}`: API Gateway has no gateways in {at}", n.id),
                        Some(&format!(
                            "set the gateway's region in a gcp block, e.g. gcp region=\"europe-west1\" (one of {})",
                            GATEWAY_REGIONS.join(", ")
                        )),
                    );
                }
                if n.props.get("cors-origins").and_then(Value::as_array).is_some_and(|o| !o.is_empty()) {
                    push(
                        n,
                        false,
                        format!(
                            "[infra.gcp.cors] API `{}`: API Gateway has no CORS policy; it passes preflight requests to the functions, which answer them for the origins in CORS_ALLOWED_ORIGINS",
                            n.id
                        ),
                        Some("implement CORS in the functions (a criterion of their tasks)"),
                    );
                }
            }
            _ => {}
        }
    }
    for (_, e, a, b, l) in links(doc) {
        if environment(doc, a).is_none_or(|x| x.id != f.id) {
            continue;
        }
        match l {
            Link::Route => {
                for r in routes(doc, e) {
                    if r == "$default" || r.contains("+}") {
                        push(
                            a,
                            true,
                            format!(
                                "[infra.gcp.route] route `{r}`: API Gateway serves the methods and paths in its OpenAPI document, without catch-alls or greedy path parameters"
                            ),
                            Some(
                                "give the edge a route like \"POST /orders\", or let the function implement data.operations",
                            ),
                        );
                    }
                }
            }
            Link::Subscribe if ty(b) == QUEUE => push(
                b,
                false,
                format!(
                    "[infra.gcp.forward] `{}` → `{}`: Pub/Sub cannot forward a topic into another topic, so queue `{}` gets a second subscription, on topic `{}`, with its settings",
                    a.id, b.id, b.id, a.id
                ),
                Some(
                    "consumers of the queue read both subscriptions; draw the topic → consumer edges directly if that is clearer",
                ),
            ),
            _ => {}
        }
    }
    if f.props.contains_key("budget-monthly") {
        out.push(Gap {
            subject: Subject::Frame(f.id.clone()),
            error: false,
            message: format!(
                "[infra.gcp.budget] environment `{}`: Cloud Billing budgets count in the billing account's currency, which may not be USD",
                f.id
            ),
            help: Some("check the currency of the project's billing account".into()),
        });
    }
    out
}
