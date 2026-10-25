//! The `flow` kit compilers. Each `flow.workflow` frame becomes
//!
//! - `flows/serverless-workflow/<name>.sw.yaml`: a Serverless Workflow DSL
//!   1.0.1 definition. Steps are tasks named by node id, in flow order;
//!   `then` jumps where the next step is not the next task.
//! - `flows/node-red/<name>.json`: an importable Node-RED flow (a tab and
//!   its nodes, wired from the control edges). Node ids are 16 hex digits
//!   hashed from (document, workflow, node), so re-imports replace nodes
//!   instead of duplicating them.
//!
//! Both carry the rhizo node id on every task or node (SW `metadata`,
//! Node-RED `info`). Conditions and templates are portable expressions
//! (`rhizo_kit::expr`), compiled to jq and JSONata. A workflow compiles only
//! to the targets it names; the kit's rules reject steps a target cannot
//! express, so nothing is approximated here.

use std::collections::HashMap;

use rhizo_core::graph::stable_toposort;
use rhizo_core::{Document, Frame, Node, NodeId};
use rhizo_kit::expr::{self, Part};
use rhizo_kit::flow::{
    self, CALL, DELAY, HTTP_REQUEST, NODE_RED, SCRIPT, SERVERLESS_WORKFLOW, SWITCH, TRANSFORM, TRIGGER_EVENT,
    TRIGGER_SCHEDULE, TRIGGER_WEBHOOK,
};
use serde_json::{Map, Value, json};

use rhizo_kit::Finding;

use crate::brief::{Criterion, Task};
use crate::target::{Cx, Deploy, Generated, Target};
use crate::{Files, OutFile, json};

/// The DSL version the Serverless Workflow output declares (and the schema
/// the tests validate it against).
pub const DSL: &str = "1.0.1";

/// Serverless Workflow and Node-RED under `flows/`.
pub(crate) struct Flow;

impl Target for Flow {
    fn kit(&self) -> &'static str {
        flow::KIT
    }

    fn dir(&self) -> &'static str {
        "flows"
    }

    fn generated(&self) -> Generated {
        Generated::Flow
    }

    fn emit(&self, cx: &Cx, files: &mut Files) -> Result<(), Vec<Finding>> {
        emit(cx.doc, cx.source_name, cx.header, files);
        Ok(())
    }

    fn covers(&self, _doc: &Document, n: &Node) -> bool {
        compiled(n)
    }

    fn implement(&self, n: &Node) -> Option<String> {
        open_script(n).then(|| format!("Implement the script step \"{}\"", n.label))
    }

    fn deploys<'a>(&self, doc: &'a Document) -> Vec<Deploy<'a>> {
        doc.frames
            .iter()
            .filter(|f| flow::is_workflow(f))
            .map(|f| {
                let nodes = flow::steps(doc, f);
                let node = nodes.iter().copied().find(|n| flow::is_trigger(n)).or(nodes.first().copied());
                Deploy { frame: f, node, nodes }
            })
            .collect()
    }

    fn criteria(&self, doc: &Document, files: &Files, t: &Task, _inputs: &[String]) -> Vec<Criterion> {
        if let Some(f) = t.deploy {
            deploy_criteria(doc, f, files).into_iter().map(|text| Criterion::new("flow", text)).collect()
        } else if open_script(t.node) && t.flow.is_none() {
            vec![Criterion::new(
                "flow",
                "The step's `code` prop holds its implementation: a JavaScript function body that gets `data` (the step's input) and returns its output. It is compiled into every target, so it survives recompiling",
            )]
        } else {
            Vec::new()
        }
    }
}

/// What a person checks when deploying a compiled workflow.
fn deploy_criteria(doc: &Document, f: &Frame, files: &Files) -> Vec<String> {
    let steps = flow::steps(doc, f);
    let mine: Vec<&String> = files
        .iter()
        .filter(|(p, x)| p.starts_with("flows/") && steps.iter().any(|n| x.nodes.contains(&n.id.to_string())))
        .map(|(p, _)| p)
        .collect();
    let mut out = Vec::new();
    for p in &mine {
        if p.starts_with("flows/node-red/") {
            out.push(format!("Node-RED imports `{p}` and deploys it without errors"));
        } else {
            out.push(format!("A Serverless Workflow engine (DSL {DSL}) accepts `{p}`"));
        }
    }
    let sw = mine.iter().any(|p| p.starts_with("flows/serverless-workflow/"));
    for t in steps.iter().filter(|n| ty(n) == TRIGGER_WEBHOOK) {
        let p = |k: &str| text(t, k);
        if sw {
            out.push(format!(
                "{} {} starts the workflow on the Serverless Workflow engine too (engines start workflows through their own API: expose it there)",
                p("method").unwrap_or("post").to_uppercase(),
                p("path").unwrap_or("/")
            ));
        }
    }
    if steps.iter().any(|n| ty(n) == HTTP_REQUEST) {
        out.push(
            "Every endpoint the HTTP steps call is reachable from the runtime, with its credentials set there (the generated files hold no secrets)"
                .into(),
        );
    }
    out
}

fn emit(doc: &Document, source_name: &str, header: &str, files: &mut Files) {
    for f in doc.frames.iter().filter(|f| flow::is_workflow(f)) {
        let w = Workflow::new(doc, f);
        let nodes: Vec<String> = flow::steps(doc, f).iter().map(|n| n.id.to_string()).collect();
        let targets = flow::targets(f);
        if targets.contains(&SERVERLESS_WORKFLOW) {
            let yaml = serde_yaml_ng::to_string(&w.serverless(source_name)).expect("JSON values serialize as YAML");
            files.insert(
                format!("flows/serverless-workflow/{}.sw.yaml", w.name),
                OutFile { contents: format!("# {header}\n{yaml}"), nodes: nodes.clone() },
            );
        }
        if targets.contains(&NODE_RED) {
            files.insert(
                format!("flows/node-red/{}.json", w.name),
                OutFile { contents: json(&w.node_red(source_name, header)), nodes: nodes.clone() },
            );
        }
    }
}

fn ty(n: &Node) -> &str {
    n.ty.as_ref().map_or("", |t| t.as_str())
}

/// A script step nobody has written yet: a placeholder in the output and a
/// task in the brief.
fn open_script(n: &Node) -> bool {
    ty(n) == SCRIPT && !n.props.contains_key("code")
}

/// Flow nodes the compilers cover completely: nothing is left to build.
fn compiled(n: &Node) -> bool {
    flow::is_step(n) && !open_script(n)
}

/// A DSL name: lowercase letters, digits and single dashes, at most 63.
pub(crate) fn dsl_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(63);
    let out = out.trim_end_matches('-');
    if out.is_empty() { "workflow".into() } else { out.into() }
}

fn text<'a>(n: &'a Node, key: &str) -> Option<&'a str> {
    n.props.get(key).and_then(Value::as_str)
}

fn parts(s: &str) -> Vec<Part> {
    // The kit's rules checked every template already.
    expr::template(s).unwrap_or_else(|_| vec![Part::Text(s.to_string())])
}

/// A prop value with its templates as SW runtime expressions.
fn sw_value(v: &Value) -> Value {
    match v {
        Value::String(s) => match expr::template_jq(&parts(s)) {
            Some(jq) => format!("${{ {jq} }}").into(),
            None => v.clone(),
        },
        Value::Array(a) => a.iter().map(sw_value).collect(),
        Value::Object(o) => o.iter().map(|(k, x)| (k.clone(), sw_value(x))).collect::<Map<_, _>>().into(),
        _ => v.clone(),
    }
}

/// A prop value as a JSONata expression building it.
fn jsonata_value(v: &Value) -> String {
    match v {
        Value::String(s) => expr::template_jsonata(&parts(s)),
        Value::Array(a) => format!("[{}]", a.iter().map(jsonata_value).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => {
            let kv: Vec<String> =
                o.iter().map(|(k, x)| format!("{}: {}", Value::from(k.as_str()), jsonata_value(x))).collect();
            format!("{{{}}}", kv.join(", "))
        }
        other => other.to_string(),
    }
}

/// The step's code as the body of `run(data)`, then `after`.
fn script(n: &Node, after: &str) -> String {
    let body = match text(n, "code") {
        Some(code) => code.trim_end().to_string(),
        None => {
            // Author text only ever lands inside `//` comments, one line each.
            let mut s = format!(
                "// TODO(rhizo T-{id}): implement \"{label}\", then put this body in the `code` prop of node `{id}`.\n",
                id = crate::one_line(n.id.as_str()),
                label = crate::one_line(&n.label)
            );
            for text in n.note.iter().chain(&n.accept) {
                for line in text.lines() {
                    s += &format!("// {}\n", crate::one_line(line));
                }
            }
            s + "return data;"
        }
    };
    let indented: Vec<String> =
        body.lines().map(|l| if l.is_empty() { String::new() } else { format!("  {l}") }).collect();
    format!("const run = (data) => {{\n{}\n}};\n{after}", indented.join("\n"))
}

struct Workflow<'a> {
    doc: &'a Document,
    frame: &'a Frame,
    name: String,
    trigger: Option<&'a Node>,
    /// The steps after the trigger, in flow order: each after the steps
    /// leading to it, document order otherwise.
    order: Vec<&'a Node>,
}

impl<'a> Workflow<'a> {
    fn new(doc: &'a Document, frame: &'a Frame) -> Self {
        let steps = flow::steps(doc, frame);
        let trigger = steps.iter().copied().find(|n| flow::is_trigger(n));
        let rest: Vec<&Node> = steps.iter().copied().filter(|n| !flow::is_trigger(n)).collect();
        let index: HashMap<&NodeId, usize> = rest.iter().enumerate().map(|(i, n)| (&n.id, i)).collect();
        let index = &index;
        let edges: Vec<(usize, usize)> = rest
            .iter()
            .enumerate()
            .flat_map(|(i, n)| flow::next(doc, n).into_iter().filter_map(move |e| Some((i, *index.get(&e.to)?))))
            .collect();
        // The rules forbid loops; on one anyway, the first step left goes next.
        let order = stable_toposort(rest.len(), edges).into_iter().map(|i| rest[i]).collect();
        Workflow { doc, frame, name: dsl_name(frame.id.as_str()), trigger, order }
    }

    fn title(&self) -> String {
        self.frame.label.clone().unwrap_or_else(|| self.frame.id.to_string())
    }

    fn next(&self, n: &Node) -> Option<&'a NodeId> {
        flow::next(self.doc, n).first().map(|e| &e.to)
    }

    /// Endpoint URL of an HTTP step: its `url`, or its operation's server
    /// and path.
    fn url(&self, n: &Node) -> String {
        if let Some(u) = text(n, "url") {
            return u.to_string();
        }
        let op = flow::operation(self.doc, n);
        let base = op.and_then(|op| flow::server(self.doc, op)).unwrap_or("");
        let path = op.and_then(|op| text(op, "path")).unwrap_or("");
        format!("{}{path}", base.trim_end_matches('/'))
    }

    /// The body an HTTP step sends: its `body`, else the data for methods
    /// that carry one.
    fn sends_data(&self, n: &Node) -> bool {
        !n.props.contains_key("body") && matches!(flow::method(self.doc, n), "post" | "put" | "patch")
    }

    // ---- Serverless Workflow ----

    fn serverless(&self, source_name: &str) -> Value {
        let f = self.frame;
        let fp = |k: &str| f.props.get(k).and_then(Value::as_str);
        let mut meta = Map::new();
        meta.insert("rhizo-source".into(), source_name.into());
        meta.insert("rhizo-frame".into(), f.id.to_string().into());
        let mut schedule = None;
        if let Some(t) = self.trigger {
            meta.insert("rhizo-trigger".into(), t.id.to_string().into());
            match ty(t) {
                // Engines start workflows through their own API; the
                // deploy task in the brief says to expose it.
                TRIGGER_WEBHOOK => {
                    let method = text(t, "method").unwrap_or("post").to_uppercase();
                    meta.insert("rhizo-webhook".into(), format!("{method} {}", text(t, "path").unwrap_or("/")).into());
                }
                TRIGGER_SCHEDULE => schedule = Some(json!({ "cron": text(t, "cron").unwrap_or("") })),
                TRIGGER_EVENT => {
                    schedule = Some(json!({ "on": { "one": { "with": { "type": text(t, "event").unwrap_or("") } } } }))
                }
                _ => {}
            }
        }
        let mut document = Map::new();
        document.insert("dsl".into(), DSL.into());
        document.insert("namespace".into(), fp("namespace").unwrap_or("default").into());
        document.insert("name".into(), self.name.clone().into());
        document.insert("version".into(), fp("version").unwrap_or("0.1.0").into());
        document.insert("title".into(), self.title().into());
        if let Some(d) = fp("description") {
            document.insert("summary".into(), d.into());
        }
        document.insert("metadata".into(), meta.into());
        let mut sw = Map::new();
        sw.insert("document".into(), document.into());
        if let Some(s) = schedule {
            sw.insert("schedule".into(), s);
        }
        let tasks: Vec<Value> = self
            .order
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let mut t = self.task(n);
                t.insert("metadata".into(), json!({ "rhizo-node": n.id.as_str() }));
                if ty(n) != SWITCH {
                    let following = self.order.get(i + 1).map(|x| &x.id);
                    match self.next(n) {
                        Some(to) if Some(to) != following => {
                            t.insert("then".into(), to.as_str().into());
                        }
                        None if following.is_some() => {
                            t.insert("then".into(), "end".into());
                        }
                        _ => {}
                    }
                }
                json!({ n.id.as_str(): t })
            })
            .collect();
        sw.insert("do".into(), tasks.into());
        sw.into()
    }

    fn task(&self, n: &Node) -> Map<String, Value> {
        let mut t = Map::new();
        match ty(n) {
            HTTP_REQUEST => {
                let mut with = Map::new();
                with.insert("method".into(), flow::method(self.doc, n).into());
                with.insert("endpoint".into(), self.url(n).into());
                if let Some(h) = n.props.get("headers") {
                    with.insert("headers".into(), h.clone());
                }
                if let Some(b) = n.props.get("body") {
                    with.insert("body".into(), sw_value(b));
                } else if self.sends_data(n) {
                    with.insert("body".into(), "${ . }".into());
                }
                t.insert("call".into(), "http".into());
                t.insert("with".into(), with.into());
            }
            SWITCH => {
                let mut cases = Vec::new();
                let mut names: Vec<String> = vec!["default".into()];
                let mut fallback = None;
                for e in flow::next(self.doc, n) {
                    let label = e.label.as_deref().unwrap_or("");
                    if flow::is_default(label) {
                        fallback = Some(e.to.as_str());
                        continue;
                    }
                    let mut name = e.to.to_string();
                    for k in 2.. {
                        if !names.contains(&name) {
                            break;
                        }
                        name = format!("{}-{k}", e.to);
                    }
                    names.push(name.clone());
                    let when = expr::parse(label).map(|x| x.jq()).unwrap_or_else(|_| label.to_string());
                    cases.push(json!({ name: { "when": format!("${{ {when} }}"), "then": e.to.as_str() } }));
                }
                // No default edge: nothing matched, the workflow ends.
                cases.push(json!({ "default": { "then": fallback.unwrap_or("end") } }));
                t.insert("switch".into(), cases.into());
            }
            TRANSFORM => {
                t.insert("set".into(), sw_value(n.props.get("set").unwrap_or(&json!({}))));
            }
            DELAY => {
                t.insert("wait".into(), json!({ "seconds": n.props.get("seconds").cloned().unwrap_or(json!(1)) }));
            }
            CALL => {
                let target = text(n, "workflow").unwrap_or("");
                let g = self.doc.frames.iter().find(|g| g.id.as_str() == target);
                let gp = |k: &str| g.and_then(|g| g.props.get(k)).and_then(Value::as_str);
                t.insert(
                    "run".into(),
                    json!({ "workflow": {
                        "namespace": gp("namespace").unwrap_or("default"),
                        "name": dsl_name(target),
                        "version": gp("version").unwrap_or("0.1.0"),
                    } }),
                );
            }
            SCRIPT => {
                // DSL 1.0: script arguments are variables, the output is stdout.
                let code = script(n, "console.log(JSON.stringify(run(data)));\n");
                t.insert(
                    "run".into(),
                    json!({ "script": { "language": "js", "code": code, "arguments": { "data": "${ . }" } } }),
                );
                t.insert("output".into(), json!({ "as": "${ fromjson }" }));
            }
            _ => {}
        }
        t
    }

    // ---- Node-RED ----

    fn node_red(&self, source_name: &str, header: &str) -> Value {
        let frame = self.frame.id.as_str();
        let id = |key: &str| nr_id(source_name, frame, key);
        let tab = id("");
        let webhook = self.trigger.is_some_and(|t| ty(t) == TRIGGER_WEBHOOK);

        // Pass 1: the nodes, each step's entry (where wires in go) and exit.
        let mut nodes: Vec<Map<String, Value>> = Vec::new();
        let mut ends: HashMap<&NodeId, (String, usize)> = HashMap::new();
        let mut info: Vec<String> = Vec::new();
        // (body node, request node): a request with a `body` is two nodes.
        let mut chains: Vec<(usize, usize)> = Vec::new();
        let base = |nid: &str, kind: &str, name: &str| {
            let mut m = Map::new();
            m.insert("id".into(), nid.into());
            m.insert("type".into(), kind.into());
            m.insert("z".into(), tab.clone().into());
            m.insert("name".into(), name.into());
            m
        };
        for n in self.trigger.into_iter().chain(self.order.iter().copied()) {
            let nid = id(n.id.as_str());
            let mut m = base(&nid, "", &n.label);
            let mut entry = nid.clone();
            let mut body_at = None;
            match ty(n) {
                TRIGGER_WEBHOOK => {
                    m.insert("type".into(), "http in".into());
                    m.insert("url".into(), text(n, "path").unwrap_or("/").into());
                    m.insert("method".into(), text(n, "method").unwrap_or("post").into());
                    m.insert("upload".into(), false.into());
                    m.insert("skipBodyParsing".into(), false.into());
                    m.insert("swaggerDoc".into(), "".into());
                }
                TRIGGER_SCHEDULE => {
                    m.insert("type".into(), "inject".into());
                    m.insert("props".into(), json!([{ "p": "payload" }]));
                    m.insert("repeat".into(), "".into());
                    m.insert("crontab".into(), text(n, "cron").unwrap_or("").into());
                    m.insert("once".into(), false.into());
                    m.insert("onceDelay".into(), 0.1.into());
                    m.insert("topic".into(), "".into());
                    m.insert("payload".into(), "{}".into());
                    m.insert("payloadType".into(), "json".into());
                }
                HTTP_REQUEST => {
                    if let Some(b) = n.props.get("body") {
                        // Node-RED sends the payload: build it first.
                        let bid = id(&format!("{}#body", n.id));
                        let mut c = base(&bid, "change", &format!("{}: body", n.label));
                        c.insert("rules".into(), json!([set_payload(&jsonata_value(b))]));
                        change_defaults(&mut c);
                        nodes.push(c);
                        info.push(format!("Builds the request body of rhizo node `{}`.", n.id));
                        body_at = Some(nodes.len() - 1);
                        entry = bid;
                    }
                    let headers: Vec<Value> = n
                        .props
                        .get("headers")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                        .map(|(k, v)| {
                            json!({ "keyType": "other", "keyValue": k, "valueType": "other", "valueValue": v.as_str().unwrap_or("") })
                        })
                        .collect();
                    m.insert("type".into(), "http request".into());
                    m.insert("method".into(), flow::method(self.doc, n).to_uppercase().into());
                    m.insert("ret".into(), "obj".into());
                    m.insert("paytoqs".into(), "ignore".into());
                    m.insert("url".into(), self.url(n).into());
                    m.insert("tls".into(), "".into());
                    m.insert("persist".into(), false.into());
                    m.insert("proxy".into(), "".into());
                    m.insert("insecureHTTPParser".into(), false.into());
                    m.insert("authType".into(), "".into());
                    // A failed request stops the message, as it faults a
                    // Serverless Workflow; it does not go on as data.
                    m.insert("senderr".into(), true.into());
                    m.insert("headers".into(), headers.into());
                }
                SWITCH => {
                    let outs = flow::next(self.doc, n);
                    let mut rules: Vec<Value> = outs
                        .iter()
                        .filter(|e| !e.label.as_deref().is_some_and(flow::is_default))
                        .map(|e| {
                            let l = e.label.as_deref().unwrap_or("");
                            let v = expr::parse(l).map(|x| x.jsonata()).unwrap_or_else(|_| l.to_string());
                            json!({ "t": "jsonata_exp", "v": v, "vt": "jsonata" })
                        })
                        .collect();
                    let fallback = outs.iter().any(|e| e.label.as_deref().is_some_and(flow::is_default));
                    if fallback || webhook {
                        rules.push(json!({ "t": "else" }));
                    }
                    m.insert("type".into(), "switch".into());
                    m.insert("property".into(), "payload".into());
                    m.insert("propertyType".into(), "msg".into());
                    m.insert("outputs".into(), rules.len().into());
                    m.insert("rules".into(), rules.into());
                    m.insert("checkall".into(), "false".into());
                    m.insert("repair".into(), false.into());
                }
                TRANSFORM => {
                    m.insert("type".into(), "change".into());
                    let set = jsonata_value(n.props.get("set").unwrap_or(&json!({})));
                    m.insert("rules".into(), json!([set_payload(&set)]));
                    change_defaults(&mut m);
                }
                DELAY => {
                    let secs = n.props.get("seconds").and_then(Value::as_i64).unwrap_or(1);
                    m.insert("type".into(), "delay".into());
                    for (k, v) in [
                        ("pauseType", json!("delay")),
                        ("timeout", json!(secs.to_string())),
                        ("timeoutUnits", json!("seconds")),
                        ("rate", json!("1")),
                        ("nbRateUnits", json!("1")),
                        ("rateUnits", json!("second")),
                        ("randomFirst", json!("1")),
                        ("randomLast", json!("5")),
                        ("randomUnits", json!("seconds")),
                        ("drop", json!(false)),
                        ("allowrate", json!(false)),
                        ("outputs", json!(1)),
                    ] {
                        m.insert(k.into(), v);
                    }
                }
                SCRIPT => {
                    m.insert("type".into(), "function".into());
                    m.insert("func".into(), script(n, "msg.payload = run(msg.payload);\nreturn msg;\n").into());
                    m.insert("outputs".into(), 1.into());
                    m.insert("timeout".into(), 0.into());
                    m.insert("noerr".into(), 0.into());
                    m.insert("initialize".into(), "".into());
                    m.insert("finalize".into(), "".into());
                    m.insert("libs".into(), json!([]));
                }
                _ => {}
            }
            nodes.push(m);
            if let Some(b) = body_at {
                chains.push((b, nodes.len() - 1));
            }
            let mut i = format!("rhizo node `{}` ({}).", n.id, ty(n));
            if let Some(note) = &n.note {
                i += &format!("\n\n{note}");
            }
            info.push(i);
            ends.insert(&n.id, (entry, nodes.len() - 1));
        }
        let response = webhook.then(|| {
            let rid = id("#response");
            let mut r = base(&rid, "http response", "Reply");
            r.insert("statusCode".into(), "200".into());
            r.insert("headers".into(), json!({}));
            nodes.push(r);
            info.push("Answers the webhook with the data.".into());
            rid
        });
        // A failed step answers the webhook with 500 instead of leaving it hanging.
        let failed = webhook.then(|| {
            let cid = id("#catch");
            let mut c = base(&cid, "catch", "Step failed");
            c.insert("scope".into(), Value::Null);
            c.insert("uncaught".into(), false.into());
            nodes.push(c);
            info.push("Catches an error in any step of this workflow.".into());
            let fid = id("#failed");
            let mut r = base(&fid, "http response", "Reply: failed");
            r.insert("statusCode".into(), "500".into());
            r.insert("headers".into(), json!({}));
            nodes.push(r);
            info.push("Answers the webhook when a step failed.".into());
            (nodes.len() - 2, fid)
        });

        // Pass 2: wires, from the control edges.
        let mut wires: Vec<Vec<Vec<String>>> = vec![Vec::new(); nodes.len()];
        for n in self.trigger.into_iter().chain(self.order.iter().copied()) {
            let (_, exit) = &ends[&n.id];
            let entry_of = |to: &NodeId| ends.get(to).map(|(e, _)| e.clone());
            let outs = flow::next(self.doc, n);
            wires[*exit] = if ty(n) == SWITCH {
                let mut w: Vec<Vec<String>> = outs
                    .iter()
                    .filter(|e| !e.label.as_deref().is_some_and(flow::is_default))
                    .map(|e| entry_of(&e.to).into_iter().collect())
                    .collect();
                match outs.iter().find(|e| e.label.as_deref().is_some_and(flow::is_default)) {
                    Some(e) => w.push(entry_of(&e.to).into_iter().collect()),
                    None if webhook => w.push(response.clone().into_iter().collect()),
                    None => {}
                }
                w
            } else {
                match outs.first() {
                    Some(e) => vec![entry_of(&e.to).into_iter().collect()],
                    None => vec![response.clone().into_iter().collect()],
                }
            };
        }
        for (b, r) in chains {
            wires[b] = vec![vec![nodes[r]["id"].as_str().unwrap_or("").to_string()]];
        }
        if let Some((c, to)) = failed {
            wires[c] = vec![vec![to]];
        }

        // Layout: columns by longest path, rows in creation order. Wires only
        // go forward in creation order, so one pass does it.
        let index: HashMap<String, usize> =
            nodes.iter().enumerate().map(|(i, m)| (m["id"].as_str().unwrap_or("").to_string(), i)).collect();
        let mut rank = vec![0usize; nodes.len()];
        for i in 0..nodes.len() {
            for to in wires[i].iter().flatten() {
                if let Some(&j) = index.get(to) {
                    rank[j] = rank[j].max(rank[i] + 1);
                }
            }
        }
        let mut rows: HashMap<usize, usize> = HashMap::new();
        let mut out = vec![json!({
            "id": tab,
            "type": "tab",
            "label": self.title(),
            "disabled": false,
            "info": format!("{header}\n\nrhizo workflow `{frame}`."),
        })];
        for (i, mut m) in nodes.into_iter().enumerate() {
            let row = rows.entry(rank[i]).or_default();
            m.insert("info".into(), info[i].clone().into());
            m.insert("x".into(), (160 + 220 * rank[i]).into());
            m.insert("y".into(), (80 + 80 * *row).into());
            *row += 1;
            if m["type"] != "http response" {
                m.insert("wires".into(), json!(wires[i]));
            } else {
                m.insert("wires".into(), json!([]));
            }
            out.push(m.into());
        }
        out.into()
    }
}

fn set_payload(jsonata: &str) -> Value {
    json!({ "t": "set", "p": "payload", "pt": "msg", "to": jsonata, "tot": "jsonata" })
}

fn change_defaults(m: &mut Map<String, Value>) {
    for k in ["action", "property", "from", "to"] {
        m.insert(k.into(), "".into());
    }
    m.insert("reg".into(), false.into());
}

/// A Node-RED id: 16 hex digits, like the ones its editor makes, hashed
/// from the document, the workflow and the node.
fn nr_id(source: &str, frame: &str, key: &str) -> String {
    crate::sha256(format!("rhizo\0{source}\0{frame}\0{key}").as_bytes())[..16].to_string()
}

#[cfg(test)]
mod tests;
