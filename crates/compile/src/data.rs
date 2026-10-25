//! The `data` kit compiler.
//!
//! - Each `data.entity` becomes `specs/schemas/<id>.schema.json`: its
//!   `data.field` children are properties, data edges to other entities are
//!   properties holding a `$ref` (an array of them for `cardinality="many"`).
//! - Each `data.enum` becomes `specs/schemas/<id>.schema.json` with its child
//!   labels as values. A data edge from an enum to a field types the field.
//! - Each `data.api` frame becomes `specs/<frame-id>.openapi.yaml`. Its
//!   `data.operation`s are the paths; a data edge from an entity to an
//!   operation is the request body, one from an operation to an entity a
//!   response (an edge label like `201` is the status code).
//! - `data.event`s and `data.channel`s become `specs/asyncapi.yaml`
//!   (AsyncAPI 3): an event's payload is the entity with a data edge into
//!   it, or else its own `data.field` children.

use std::collections::{BTreeSet, HashMap};

use indexmap::IndexMap;
use rhizo_core::{Document, Edge, EdgeKind, Frame, Node, NodeId};
use rhizo_kit::data::{API, CHANNEL, ENTITY, ENUM, EVENT, FIELD, OPERATION};
use serde_json::{Map, Value, json};

use rhizo_kit::{Finding, Subject};
use rhizo_lang::Diagnostic;

use crate::brief::{Criterion, Task};
use crate::target::{Cx, Generated, Target};
use crate::{Files, OutFile, file_stem, json};

const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Trace annotation: the node an object comes from (design doc §7.2).
/// Validators ignore unknown keywords; OpenAPI and AsyncAPI allow `x-` keys.
const NODE: &str = "x-rhizo-node";

/// Writes the data specs into `files`. Returns the clashes: two ids that
/// make the same file name (`a b` and `a-b`), which must stop the compile.
fn emit(doc: &Document, header: &str, files: &mut Files) -> Vec<String> {
    let mut clashes = Vec::new();
    let mut put = |files: &mut Files, path: String, f: OutFile| {
        if let Some(old) = files.insert(path.clone(), f) {
            clashes.push(format!("`{path}` comes from two nodes ({} and another); rename one", old.nodes.join(", ")));
        }
    };
    let m = Model::new(doc);
    for &n in m.entities.iter().chain(&m.enums) {
        let mut schema = Map::new();
        schema.insert("$schema".into(), DIALECT.into());
        schema.insert("$comment".into(), header.into());
        schema.extend(m.schema(n, &Refs::Files));
        let mut nodes = vec![n.id.to_string()];
        nodes.extend(m.fields(n).map(|f| f.id.to_string()));
        let path = format!("specs/schemas/{}.schema.json", file_stem(n.id.as_str()));
        put(files, path, OutFile { contents: json(&schema.into()), nodes });
    }
    for f in doc.frames.iter().filter(|f| f.ty.as_ref().is_some_and(|t| t.as_str() == API)) {
        let (api, nodes) = m.openapi(f);
        let yaml = serde_yaml_ng::to_string(&api).expect("JSON values serialize as YAML");
        let path = format!("specs/{}.openapi.yaml", file_stem(f.id.as_str()));
        put(files, path, OutFile { contents: format!("# {header}\n{yaml}"), nodes });
    }
    if let Some((api, nodes)) = m.asyncapi() {
        let yaml = serde_yaml_ng::to_string(&api).expect("JSON values serialize as YAML");
        put(files, "specs/asyncapi.yaml".into(), OutFile { contents: format!("# {header}\n{yaml}"), nodes });
    }
    clashes
}

/// Types whose nodes the compiler covers completely: nothing is left to
/// build by hand. (`data.operation` gets a spec *and* an implement task.)
fn generated_only(ty: &str) -> bool {
    matches!(ty, ENTITY | FIELD | ENUM | API | EVENT | CHANNEL)
}

/// JSON Schema, OpenAPI and AsyncAPI under `specs/`.
pub(crate) struct Data;

impl Target for Data {
    fn kit(&self) -> &'static str {
        rhizo_kit::data::KIT
    }

    fn dir(&self) -> &'static str {
        "specs"
    }

    fn generated(&self) -> Generated {
        Generated::Spec
    }

    fn emit(&self, cx: &Cx, files: &mut Files) -> Result<(), Vec<Finding>> {
        let clashes = emit(cx.doc, cx.header, files);
        if clashes.is_empty() {
            return Ok(());
        }
        let diag = |m: String| Finding { subject: Subject::Document, diagnostic: Diagnostic::error(m, None) };
        Err(clashes.into_iter().map(diag).collect())
    }

    fn covers(&self, doc: &Document, n: &Node) -> bool {
        // Enum values are generated with their enum.
        let value = n.parent.as_ref().and_then(|p| doc.nodes.get(p)).is_some_and(|p| is(p, ENUM));
        value || n.ty.as_ref().is_some_and(|t| generated_only(t.as_str()))
    }

    fn implement(&self, n: &Node) -> Option<String> {
        is(n, OPERATION).then(|| {
            let p = |k: &str| text(&n.props, k).unwrap_or("");
            format!("Implement {} {} ({})", p("method").to_uppercase(), p("path"), n.label)
        })
    }

    fn criteria(&self, doc: &Document, _files: &Files, t: &Task, inputs: &[String]) -> Vec<Criterion> {
        if is(t.node, OPERATION) && t.flow.is_none() {
            return operation_criteria(doc, inputs, t.node, false);
        }
        Vec::new()
    }

    fn questions(&self, doc: &Document, decided: &dyn Fn(&[&str]) -> bool) -> Vec<String> {
        let mut q = Vec::new();
        for f in &doc.frames {
            let oauth = f.ty.as_ref().is_some_and(|t| t.as_str() == API) && text(&f.props, "auth") == Some("oauth2");
            if oauth && !decided(&["identity", "oauth", "auth0", "keycloak", "cognito", "entra", "okta"]) {
                q.push(format!(
                    "Which identity provider issues the OAuth 2.0 tokens for \"{}\", and which scopes does each operation need?",
                    crate::brief::frame_name(doc, f)
                ));
            }
        }
        q
    }
}

/// What an implemented operation must do, given its task's `inputs`;
/// `named` says which operation (for a task with several).
pub(crate) fn operation_criteria(doc: &Document, inputs: &[String], op: &Node, named: bool) -> Vec<Criterion> {
    let spec = inputs.iter().find(|f| f.ends_with(".openapi.yaml"));
    let spec = spec.map_or("the OpenAPI spec".to_string(), |s| format!("`{s}`"));
    let p = |k: &str| text(&op.props, k).unwrap_or("");
    let which = if named { format!("{} {}: ", p("method").to_uppercase(), p("path")) } else { String::new() };
    let mut out = vec![Criterion::new(
        "data",
        format!("{which}Request and response bodies, status codes and parameters match {spec}"),
    )];
    let has_body = doc.edges.iter().any(|e| e.to == op.id && doc.nodes.get(&e.from).is_some_and(|n| is(n, ENTITY)));
    if has_body {
        out.push(Criterion::new(
            "data",
            format!("{which}Rejects a request body that does not match its schema with a 4XX response"),
        ));
    }
    out
}

fn is(n: &Node, t: &str) -> bool {
    n.ty.as_ref().is_some_and(|x| x.as_str() == t)
}

fn text<'a>(props: &'a rhizo_core::Props, key: &str) -> Option<&'a str> {
    props.get(key).and_then(Value::as_str)
}

/// A plain data edge (no kind counts too: the kit already rejected others).
fn data_edge(e: &Edge) -> bool {
    e.kind.is_none_or(|k| k == EdgeKind::Data)
}

enum Refs {
    /// `customer.schema.json`, next to the referring file.
    Files,
    /// `#/components/schemas/Customer`.
    Components,
}

struct Model<'a> {
    doc: &'a Document,
    entities: Vec<&'a Node>,
    enums: Vec<&'a Node>,
    /// Component name per entity and enum.
    names: HashMap<&'a NodeId, String>,
}

impl<'a> Model<'a> {
    fn new(doc: &'a Document) -> Self {
        let entities: Vec<&Node> = doc.nodes.values().filter(|n| is(n, ENTITY)).collect();
        let enums: Vec<&Node> = doc.nodes.values().filter(|n| is(n, ENUM)).collect();
        let mut names = HashMap::new();
        let mut taken = BTreeSet::new();
        for n in entities.iter().chain(&enums) {
            let mut name = pascal(&n.label);
            if name.is_empty() || !taken.insert(name.clone()) {
                name = pascal(n.id.as_str());
                taken.insert(name.clone());
            }
            names.insert(&n.id, name);
        }
        Model { doc, entities, enums, names }
    }

    /// `Problem`, unless an entity or enum already has that name.
    fn problem_name(&self) -> String {
        ["Problem", "ProblemDetails", "RfcProblem"]
            .into_iter()
            .find(|n| !self.names.values().any(|v| v == n))
            .unwrap_or("RhizoProblem")
            .to_string()
    }

    fn node(&self, id: &NodeId) -> Option<&'a Node> {
        self.doc.nodes.get(id)
    }

    fn fields(&self, entity: &'a Node) -> impl Iterator<Item = &'a Node> + 'a {
        let doc = self.doc;
        entity.children.iter().filter_map(move |c| doc.nodes.get(c)).filter(|c| is(c, FIELD))
    }

    fn reference(&self, target: &Node, refs: &Refs) -> Value {
        match refs {
            Refs::Files => json!({ "$ref": format!("{}.schema.json", file_stem(target.id.as_str())) }),
            Refs::Components => json!({ "$ref": format!("#/components/schemas/{}", self.names[&target.id]) }),
        }
    }

    /// The schema of an entity or enum, without `$schema`/`$comment`.
    fn schema(&self, n: &Node, refs: &Refs) -> Map<String, Value> {
        let mut s = Map::new();
        s.insert("title".into(), n.label.clone().into());
        s.insert(NODE.into(), n.id.to_string().into());
        if let Some(d) = text(&n.props, "description") {
            s.insert("description".into(), d.into());
        }
        if is(n, ENUM) {
            s.insert("type".into(), "string".into());
            let values: Vec<Value> =
                n.children.iter().filter_map(|c| self.node(c)).map(|c| c.label.clone().into()).collect();
            s.insert("enum".into(), values.into());
            return s;
        }
        s.insert("type".into(), "object".into());
        let mut props = Map::new();
        let mut required = Vec::new();
        for f in self.fields(n) {
            let name = field_name(&f.label);
            if f.props.get("required") == Some(&Value::Bool(true)) {
                required.push(Value::from(name.clone()));
            }
            props.insert(name, self.field(f, refs));
        }
        // Relations: data edges to other entities. A field of the same name wins.
        for e in self.doc.edges.iter().filter(|e| e.from == n.id && data_edge(e)) {
            let Some(target) = self.node(&e.to).filter(|t| is(t, ENTITY)) else { continue };
            let mut value = self.reference(target, refs);
            if text(&e.props, "cardinality") == Some("many") {
                value = json!({ "type": "array", "items": value });
            }
            if let (Some(label), Some(obj)) = (&e.label, value.as_object_mut()) {
                obj.insert("description".into(), label.clone().into());
            }
            props.entry(e.to.as_str().replace('-', "_")).or_insert(value);
        }
        s.insert("properties".into(), props.into());
        if !required.is_empty() {
            s.insert("required".into(), required.into());
        }
        s.insert("additionalProperties".into(), false.into());
        s
    }

    fn field(&self, f: &Node, refs: &Refs) -> Value {
        let mut s = Map::new();
        // A data edge from an enum gives the field its type.
        let typed_by = self
            .doc
            .edges
            .iter()
            .filter(|e| e.to == f.id && data_edge(e))
            .find_map(|e| self.node(&e.from).filter(|n| is(n, ENUM)));
        if let Some(en) = typed_by {
            s.extend(self.reference(en, refs).as_object().cloned().unwrap_or_default());
        } else {
            let t = text(&f.props, "type").unwrap_or("string");
            s.insert("type".into(), t.into());
            if let Some(format) = text(&f.props, "format") {
                s.insert("format".into(), format.into());
            }
            if let Some(values) = f.props.get("enum") {
                s.insert("enum".into(), values.clone());
            }
            let (lo, hi) = match t {
                "string" => ("minLength", "maxLength"),
                "array" => ("minItems", "maxItems"),
                "object" => ("minProperties", "maxProperties"),
                _ => ("minimum", "maximum"),
            };
            for (key, word) in [("min", lo), ("max", hi)] {
                if let Some(v) = f.props.get(key) {
                    s.insert(word.into(), v.clone());
                }
            }
        }
        if let Some(d) = text(&f.props, "description") {
            s.insert("description".into(), d.into());
        }
        s.insert(NODE.into(), f.id.to_string().into());
        match text(&f.props, "pii") {
            None | Some("none") => {}
            Some(p) => {
                s.insert("x-rhizo-pii".into(), p.into());
            }
        }
        s.into()
    }

    /// One OpenAPI document for a `data.api` frame, and the node ids it uses.
    fn openapi(&self, frame: &Frame) -> (Value, Vec<String>) {
        let props = &frame.props;
        let version = if text(props, "api-version") == Some("3.2") { "3.2.0" } else { "3.1.0" };
        let mut info = Map::new();
        info.insert("title".into(), frame.label.clone().unwrap_or_else(|| frame.id.to_string()).into());
        info.insert("version".into(), text(props, "version").unwrap_or("0.1.0").into());
        if let Some(d) = text(props, "description") {
            info.insert("description".into(), d.into());
        }
        if let Some(l) = text(props, "license") {
            info.insert("license".into(), json!({ "name": l, "identifier": l }));
        }
        let problems = text(props, "errors") != Some("none");
        let mut api = Map::new();
        api.insert("openapi".into(), version.into());
        api.insert("info".into(), info.into());
        if let Some(servers) = props.get("servers").and_then(Value::as_array) {
            let list: Vec<Value> = servers.iter().map(|u| json!({ "url": u })).collect();
            api.insert("servers".into(), list.into());
        }

        let mut nodes = Vec::new();
        let mut used: Vec<&Node> = Vec::new();
        let mut paths: IndexMap<String, Map<String, Value>> = IndexMap::new();
        let ops = self.doc.nodes.values().filter(|n| n.frame == frame.id && is(n, OPERATION));
        for op in ops {
            nodes.push(op.id.to_string());
            let method = text(&op.props, "method").unwrap_or("get");
            let path = text(&op.props, "path").unwrap_or("/");
            let o = self.operation(op, method, path, problems, &mut used);
            paths.entry(path.to_string()).or_default().insert(method.into(), o);
        }
        let paths: Map<String, Value> = paths.into_iter().map(|(p, m)| (p, m.into())).collect();
        api.insert("paths".into(), paths.into());

        // Components: entities and enums the operations reach, in document order.
        let mut schemas = self.components(&used, &mut nodes);
        if problems {
            schemas.insert(self.problem_name(), problem_schema());
        }
        let mut components = Map::new();
        if !schemas.is_empty() {
            components.insert("schemas".into(), schemas.into());
        }
        let scheme = match text(props, "auth") {
            Some("oauth2") => Some((
                "oauth2",
                json!({ "type": "http", "scheme": "bearer", "description": "OAuth 2.0 bearer access token" }),
            )),
            Some("api-key") => Some(("apiKey", json!({ "type": "apiKey", "in": "header", "name": "X-API-Key" }))),
            _ => None,
        };
        if let Some((name, s)) = &scheme {
            components.insert("securitySchemes".into(), json!({ *name: s }));
        }
        if !components.is_empty() {
            api.insert("components".into(), components.into());
        }
        if let Some((name, _)) = scheme {
            api.insert("security".into(), json!([{ name: [] }]));
        }
        (api.into(), nodes)
    }

    /// Schemas of the entities and enums reachable from `seeds` (through
    /// relations and enum-typed fields), keyed by component name.
    fn components(&self, seeds: &[&'a Node], nodes: &mut Vec<String>) -> Map<String, Value> {
        let mut reach: BTreeSet<&NodeId> = seeds.iter().map(|n| &n.id).collect();
        let holder = |n: &Node| is(n, ENTITY) || is(n, EVENT);
        loop {
            let before = reach.len();
            for e in self.doc.edges.iter().filter(|e| data_edge(e)) {
                let (Some(from), Some(to)) = (self.node(&e.from), self.node(&e.to)) else { continue };
                if reach.contains(&e.from) && holder(from) && is(to, ENTITY) {
                    reach.insert(&e.to);
                }
                let field_of_reached = is(to, FIELD) && to.parent.as_ref().is_some_and(|p| reach.contains(p));
                if field_of_reached && is(from, ENUM) {
                    reach.insert(&e.from);
                }
            }
            if reach.len() == before {
                break;
            }
        }
        let mut schemas = Map::new();
        for &n in self.entities.iter().chain(&self.enums).filter(|n| reach.contains(&n.id)) {
            nodes.push(n.id.to_string());
            schemas.insert(self.names[&n.id].clone(), self.schema(n, &Refs::Components).into());
        }
        schemas
    }

    /// One AsyncAPI 3 document for all `data.event`s and `data.channel`s:
    /// an edge event → channel is a `send` operation, channel → any other
    /// node a `receive` operation by that node.
    fn asyncapi(&self) -> Option<(Value, Vec<String>)> {
        let events: Vec<&Node> = self.doc.nodes.values().filter(|n| is(n, EVENT)).collect();
        let channels: Vec<&Node> = self.doc.nodes.values().filter(|n| is(n, CHANNEL)).collect();
        if events.is_empty() && channels.is_empty() {
            return None;
        }
        let mut nodes: Vec<String> = Vec::new();
        let mut used: Vec<&Node> = events.clone();
        let msg_name = |n: &Node| {
            let p = pascal(&n.label);
            if p.is_empty() { pascal(n.id.as_str()) } else { p }
        };

        let mut messages = Map::new();
        for &ev in &events {
            nodes.push(ev.id.to_string());
            let payload_entity = self
                .doc
                .edges
                .iter()
                .filter(|e| e.to == ev.id && data_edge(e))
                .find_map(|e| self.node(&e.from).filter(|n| is(n, ENTITY)));
            let payload = match payload_entity {
                Some(en) => {
                    used.push(en);
                    self.reference(en, &Refs::Components)
                }
                None => self.schema(ev, &Refs::Components).into(),
            };
            let mut m = Map::new();
            m.insert("name".into(), ev.id.to_string().into());
            m.insert(NODE.into(), ev.id.to_string().into());
            m.insert("title".into(), ev.label.clone().into());
            if let Some(d) = text(&ev.props, "description") {
                m.insert("description".into(), d.into());
            }
            m.insert("contentType".into(), text(&ev.props, "content-type").unwrap_or("application/json").into());
            m.insert("payload".into(), payload);
            messages.insert(msg_name(ev), m.into());
        }

        let mut chans = Map::new();
        let mut ops = Map::new();
        for &ch in &channels {
            nodes.push(ch.id.to_string());
            let cid = camel(ch.id.as_str());
            let sent: Vec<&Node> = self
                .doc
                .edges
                .iter()
                .filter(|e| e.to == ch.id)
                .filter_map(|e| self.node(&e.from).filter(|n| is(n, EVENT)))
                .collect();
            let mut c = Map::new();
            c.insert("address".into(), text(&ch.props, "address").unwrap_or(ch.id.as_str()).into());
            c.insert(NODE.into(), ch.id.to_string().into());
            if let Some(d) = text(&ch.props, "description") {
                c.insert("description".into(), d.into());
            }
            let msgs: Map<String, Value> = sent
                .iter()
                .map(|ev| (msg_name(ev), json!({ "$ref": format!("#/components/messages/{}", msg_name(ev)) })))
                .collect();
            let refs: Vec<Value> =
                msgs.keys().map(|k| json!({ "$ref": format!("#/channels/{cid}/messages/{k}") })).collect();
            if !msgs.is_empty() {
                c.insert("messages".into(), msgs.into());
            }
            chans.insert(cid.clone(), c.into());
            for ev in &sent {
                let id = camel(&format!("send-{}-to-{}", ev.id, ch.id));
                let r = json!({ "$ref": format!("#/channels/{cid}/messages/{}", msg_name(ev)) });
                ops.insert(
                    id,
                    json!({ "action": "send", "channel": { "$ref": format!("#/channels/{cid}") }, "messages": [r] }),
                );
            }
            for e in self.doc.edges.iter().filter(|e| e.from == ch.id) {
                let Some(to) = self.node(&e.to).filter(|n| !is(n, EVENT)) else { continue };
                let id = camel(&format!("{}-receives-{}", to.id, ch.id));
                let mut op = json!({ "action": "receive", "channel": { "$ref": format!("#/channels/{cid}") } });
                if !refs.is_empty() {
                    op["messages"] = refs.clone().into();
                }
                ops.insert(id, op);
            }
        }

        let mut api = Map::new();
        api.insert("asyncapi".into(), "3.1.0".into());
        let title = self.doc.title.clone().unwrap_or_else(|| "Events".into());
        api.insert("info".into(), json!({ "title": title, "version": "0.1.0" }));
        if !chans.is_empty() {
            api.insert("channels".into(), chans.into());
        }
        if !ops.is_empty() {
            api.insert("operations".into(), ops.into());
        }
        let schemas = self.components(&used, &mut nodes);
        let mut components = Map::new();
        if !messages.is_empty() {
            components.insert("messages".into(), messages.into());
        }
        if !schemas.is_empty() {
            components.insert("schemas".into(), schemas.into());
        }
        if !components.is_empty() {
            api.insert("components".into(), components.into());
        }
        Some((api.into(), nodes))
    }

    fn operation(&self, op: &'a Node, method: &str, path: &str, problems: bool, used: &mut Vec<&'a Node>) -> Value {
        let mut o = Map::new();
        o.insert("operationId".into(), camel(op.id.as_str()).into());
        o.insert(NODE.into(), op.id.to_string().into());
        o.insert("summary".into(), text(&op.props, "summary").unwrap_or(&op.label).into());
        if let Some(d) = text(&op.props, "description") {
            o.insert("description".into(), d.into());
        }
        if op.props.get("deprecated") == Some(&Value::Bool(true)) {
            o.insert("deprecated".into(), true.into());
        }
        let params: Vec<Value> = path_params(path)
            .into_iter()
            .map(|p| json!({ "name": p, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        if !params.is_empty() {
            o.insert("parameters".into(), params.into());
        }

        let body = |e: &Edge, n: &'a Node, used: &mut Vec<&'a Node>| {
            used.push(n);
            let r = self.reference(n, &Refs::Components);
            if text(&e.props, "cardinality") == Some("many") { json!({ "type": "array", "items": r }) } else { r }
        };
        let requests: Vec<Value> = self
            .doc
            .edges
            .iter()
            .filter(|e| e.to == op.id && data_edge(e))
            .filter_map(|e| Some((e, self.node(&e.from).filter(|n| is(n, ENTITY) || is(n, ENUM))?)))
            .map(|(e, n)| body(e, n, used))
            .collect();
        if !requests.is_empty() {
            let schema = if requests.len() == 1 { requests[0].clone() } else { json!({ "oneOf": requests }) };
            o.insert(
                "requestBody".into(),
                json!({ "required": true, "content": { "application/json": { "schema": schema } } }),
            );
        }

        let mut responses = Map::new();
        for e in self.doc.edges.iter().filter(|e| e.from == op.id && data_edge(e)) {
            let Some(n) = self.node(&e.to).filter(|n| is(n, ENTITY) || is(n, ENUM)) else { continue };
            let label = e.label.as_deref().unwrap_or("");
            let code = if is_status(label) { label.to_string() } else { default_status(method).to_string() };
            let description =
                if is_status(label) || label.is_empty() { reason(&code).to_string() } else { label.into() };
            let schema = body(e, n, used);
            responses.entry(code).or_insert_with(
                || json!({ "description": description, "content": { "application/json": { "schema": schema } } }),
            );
        }
        if !responses.keys().any(|c| !c.starts_with('4') && c != "default") {
            let code = default_status(method);
            responses.insert(code.into(), json!({ "description": reason(code) }));
        }
        if problems && !responses.keys().any(|c| c.starts_with('4') || c == "default") {
            responses.insert(
                "4XX".into(),
                json!({
                    "description": "Client error",
                    "content": { "application/problem+json": { "schema": { "$ref": format!("#/components/schemas/{}", self.problem_name()) } } }
                }),
            );
        }
        o.insert("responses".into(), responses.into());
        o.into()
    }
}

/// RFC 9457 problem details.
fn problem_schema() -> Value {
    json!({
        "title": "Problem",
        "description": "An error, as RFC 9457 problem details",
        "type": "object",
        "properties": {
            "type": { "type": "string", "format": "uri-reference", "default": "about:blank" },
            "title": { "type": "string" },
            "status": { "type": "integer", "minimum": 100, "maximum": 599 },
            "detail": { "type": "string" },
            "instance": { "type": "string", "format": "uri-reference" }
        }
    })
}

fn is_status(s: &str) -> bool {
    s == "default"
        || (s.len() == 3 && s.starts_with(['1', '2', '3', '4', '5']) && s.bytes().all(|b| b.is_ascii_digit()))
}

fn default_status(method: &str) -> &'static str {
    match method {
        "post" => "201",
        "delete" => "204",
        _ => "200",
    }
}

fn reason(code: &str) -> &'static str {
    match code {
        "200" => "OK",
        "201" => "Created",
        "202" => "Accepted",
        "204" => "No Content",
        "400" => "Bad Request",
        "401" => "Unauthorized",
        "403" => "Forbidden",
        "404" => "Not Found",
        "409" => "Conflict",
        "422" => "Unprocessable Content",
        "default" => "Unexpected response",
        _ => "Response",
    }
}

/// `{id}` segments of a path template, in order.
fn path_params(path: &str) -> Vec<&str> {
    path.split('{').skip(1).filter_map(|s| s.split_once('}').map(|(p, _)| p)).filter(|p| !p.is_empty()).collect()
}

/// A property name from a field label: as written, runs of whitespace as `_`.
fn field_name(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join("_")
}

/// `order item` / `order-item` → `OrderItem`. Keeps only ASCII alphanumerics,
/// which OpenAPI component names and most code generators accept.
fn pascal(s: &str) -> String {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

/// `list-orders` → `listOrders`.
fn camel(s: &str) -> String {
    let p = pascal(s);
    let mut c = p.chars();
    c.next().map(|f| f.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}
