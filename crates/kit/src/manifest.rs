//! Reading `kit.kdl`:
//!
//! ```kdl
//! kit data version="0.1.0" { description "…"; namespaces data; targets openapi }
//! type data.operation {
//!     label "Operation"
//!     schema "schemas/data.operation.json"
//!     parents data.api
//!     port in "request" kind=data default=#true
//! }
//! ```

use indexmap::IndexMap;
use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use rhizo_core::{DecorState, EdgeKind, Port, PortDir, PortName, Shape};

use crate::look::{Look, Rows, Tag};
use crate::{Kit, KitPort, TypeDef};

#[derive(Debug)]
pub struct ManifestError(pub String);

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ManifestError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ManifestError> {
    Err(ManifestError(msg.into()))
}

impl Kit {
    /// Parses a manifest. `file` returns the text of a path the manifest
    /// refers to (schemas), relative to the kit's folder.
    pub fn parse(src: &str, file: impl Fn(&str) -> Option<&str>) -> Result<Kit, ManifestError> {
        let doc = KdlDocument::parse(src).map_err(|e| ManifestError(format!("kit.kdl: {e}")))?;
        let mut kit: Option<Kit> = None;
        for node in doc.nodes() {
            match node.name().value() {
                "kit" => {
                    let name = arg(node, 0).ok_or_else(|| ManifestError("`kit` needs a name".into()))?;
                    let version = prop(node, "version").ok_or_else(|| ManifestError("`kit` needs a version".into()))?;
                    let version = semver::Version::parse(&version)
                        .map_err(|e| ManifestError(format!("kit version `{version}`: {e}")))?;
                    let mut k = Kit {
                        name,
                        version,
                        description: None,
                        namespaces: Vec::new(),
                        targets: Vec::new(),
                        types: IndexMap::new(),
                    };
                    for child in children(node) {
                        match child.name().value() {
                            "description" => k.description = arg(child, 0),
                            "namespaces" => k.namespaces = args(child),
                            "targets" => k.targets = args(child),
                            other => return err(format!("unknown statement `{other}` in `kit`")),
                        }
                    }
                    kit = Some(k);
                }
                "type" => {
                    let Some(k) = kit.as_mut() else { return err("`type` before `kit`") };
                    let t = type_def(node, &file)?;
                    if !k.owns(&t.name) {
                        return err(format!("type `{}` is outside the kit's namespaces", t.name));
                    }
                    k.types.insert(t.name.clone(), t);
                }
                other => return err(format!("unknown statement `{other}` in kit.kdl")),
            }
        }
        kit.ok_or_else(|| ManifestError("kit.kdl has no `kit` statement".into()))
    }
}

fn type_def(node: &KdlNode, file: &impl Fn(&str) -> Option<&str>) -> Result<TypeDef, ManifestError> {
    let name = arg(node, 0).ok_or_else(|| ManifestError("`type` needs a name".into()))?;
    let frame = node.get("frame").and_then(KdlValue::as_bool).unwrap_or(false);
    let (mut label, mut description, mut parents, mut ports) = (None, None, Vec::new(), Vec::new());
    let mut shape = None;
    let mut look = None;
    let mut schema = serde_json::json!({ "type": "object", "additionalProperties": false });
    let mut shared = Siblings::default();
    for child in children(node) {
        match child.name().value() {
            "label" => label = arg(child, 0),
            "description" => description = arg(child, 0),
            "parents" => parents = args(child),
            "shape" => {
                let s = arg(child, 0).unwrap_or_default();
                shape = Some(Shape::parse(&s).ok_or_else(|| ManifestError(format!("{name}: unknown shape `{s}`")))?);
            }
            "schema" => {
                let path = arg(child, 0).unwrap_or_default();
                let text = file(&path).ok_or_else(|| ManifestError(format!("{name}: no file `{path}`")))?;
                schema = serde_json::from_str(text).map_err(|e| ManifestError(format!("{path}: {e}")))?;
                shared = siblings(&schema, &path, file)?;
            }
            "port" => ports.push(port(child).map_err(|e| ManifestError(format!("{name}: {}", e.0)))?),
            "look" => look = Some(look_block(child).map_err(|e| ManifestError(format!("{name}: look: {}", e.0)))?),
            other => return err(format!("{name}: unknown statement `{other}`")),
        }
    }
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .with_retriever(shared)
        .build(&schema)
        .map_err(|e| ManifestError(format!("{name}: schema: {e}")))?;
    Ok(TypeDef {
        label: label.unwrap_or_else(|| name.clone()),
        name,
        description,
        frame,
        parents,
        ports,
        shape,
        look,
        schema,
        validator,
    })
}

/// Schemas of the same kit that a schema refers to (`"$ref":
/// "imported.json#/$defs/body"`), by their URI. Nothing else is fetched.
#[derive(Default)]
struct Siblings(std::collections::HashMap<String, serde_json::Value>);

impl jsonschema::Retrieve for Siblings {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        self.0.get(uri.as_str()).cloned().ok_or_else(|| format!("{uri} is not a schema of this kit").into())
    }
}

/// The sibling schemas `schema` (read from `path`) refers to, read with `file`.
fn siblings(
    schema: &serde_json::Value,
    path: &str,
    file: &impl Fn(&str) -> Option<&str>,
) -> Result<Siblings, ManifestError> {
    fn refs<'v>(v: &'v serde_json::Value, out: &mut Vec<&'v str>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(r) = m.get("$ref").and_then(|r| r.as_str()) {
                    out.push(r);
                }
                m.values().for_each(|x| refs(x, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| refs(x, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    refs(schema, &mut found);
    let base = |p: &str| p.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
    let id = schema.get("$id").and_then(|i| i.as_str()).unwrap_or_default();
    let mut out = Siblings::default();
    for sibling in found.into_iter().filter_map(|r| r.split('#').next()).filter(|f| !f.is_empty() && !f.contains(':')) {
        let at = format!("{}/{sibling}", base(path)).trim_start_matches('/').to_string();
        let text = file(&at).ok_or_else(|| ManifestError(format!("{path}: no file `{at}`")))?;
        let value = serde_json::from_str(text).map_err(|e| ManifestError(format!("{at}: {e}")))?;
        out.0.insert(format!("{}/{sibling}", base(id)), value);
    }
    Ok(out)
}

fn look_block(node: &KdlNode) -> Result<Look, ManifestError> {
    let mut look = Look::default();
    for child in children(node) {
        match child.name().value() {
            "badge" => look.badge = Some(tag(child)?),
            "subtitle" => look.subtitle = Some(template(child)?),
            "state" => {
                let t = tag(child)?;
                if let Some(bad) = t.values.values().find(|v| DecorState::parse(v).is_none()) {
                    return err(format!("unknown state `{bad}` (normal, draft, void)"));
                }
                look.state = Some(t);
            }
            "tone" => look.tone = Some(template(child)?),
            "card" => look.card = true,
            "container" => look.container = true,
            "rows" => {
                let mut rows = Rows { of: template(child)?, ..Rows::default() };
                for r in children(child) {
                    match r.name().value() {
                        "detail" => rows.detail = Some(template(r)?),
                        "strong" => rows.strong = Some(template(r)?),
                        "marker" => rows.marker = Some(tag(r)?),
                        other => return err(format!("unknown statement `{other}` in `rows`")),
                    }
                }
                look.rows = Some(rows);
            }
            other => return err(format!("unknown statement `{other}`")),
        }
    }
    Ok(look)
}

fn template(node: &KdlNode) -> Result<String, ManifestError> {
    arg(node, 0).ok_or_else(|| ManifestError(format!("`{}` needs a text", node.name().value())))
}

/// `badge "{method}" get=green tone=grey text=PII`: everything but `tone`
/// and `text` maps a value to a tone (or a state).
fn tag(node: &KdlNode) -> Result<Tag, ManifestError> {
    let mut t = Tag { template: template(node)?, ..Tag::default() };
    for e in node.entries() {
        let (Some(key), Some(v)) = (e.name(), e.value().as_string()) else { continue };
        match key.value() {
            "tone" => t.tone = Some(v.to_string()),
            "text" => t.text = Some(v.to_string()),
            k => {
                t.values.insert(k.to_string(), v.to_string());
            }
        }
    }
    Ok(t)
}

fn port(node: &KdlNode) -> Result<KitPort, ManifestError> {
    let dir = arg(node, 0).as_deref().and_then(PortDir::parse);
    let (Some(dir), Some(name)) = (dir, arg(node, 1)) else { return err("a port needs `in`/`out` and a name") };
    let kind = match prop(node, "kind") {
        Some(k) => Some(EdgeKind::parse(&k).ok_or_else(|| ManifestError(format!("port kind `{k}`")))?),
        None => None,
    };
    Ok(KitPort {
        port: Port { name: PortName(name), dir, kind, schema: prop(node, "schema"), label: prop(node, "label") },
        default: node.get("default").and_then(KdlValue::as_bool).unwrap_or(false),
    })
}

fn children(node: &KdlNode) -> impl Iterator<Item = &KdlNode> {
    node.children().into_iter().flat_map(|d| d.nodes())
}

fn positional(node: &KdlNode) -> impl Iterator<Item = &KdlEntry> {
    node.entries().iter().filter(|e| e.name().is_none())
}

fn arg(node: &KdlNode, i: usize) -> Option<String> {
    positional(node).nth(i).and_then(|e| e.value().as_string()).map(str::to_string)
}

fn args(node: &KdlNode) -> Vec<String> {
    positional(node).filter_map(|e| e.value().as_string()).map(str::to_string).collect()
}

fn prop(node: &KdlNode, key: &str) -> Option<String> {
    node.get(key).and_then(KdlValue::as_string).map(str::to_string)
}
