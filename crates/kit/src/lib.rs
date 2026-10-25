//! Kits: typed vocabularies for rhizo documents.
//!
//! A kit owns one or more type namespaces (`data.*`) and says, for each type,
//! which props it takes (a JSON Schema 2020-12 document), which ports it has,
//! and where it may sit. [`check`] validates a document against the kits it
//! `use`s; core only checks structure. Built-in kits are embedded in the
//! binary as plain `kit.kdl` + `schemas/*.json`, the same layout a kit on disk
//! will have.

mod check;
pub mod data;
pub mod expr;
pub mod flow;
mod guide;
pub mod infra;
pub mod look;
mod manifest;
pub mod rhizo;

pub use check::{Finding, RULES, Rules, Subject, check, locate};
pub use manifest::ManifestError;

use indexmap::IndexMap;
use rhizo_core::{Document, EdgeKind, Port, PortDir, Shape};

#[derive(Debug)]
pub struct Kit {
    pub name: String,
    pub version: semver::Version,
    pub description: Option<String>,
    /// First segments of the type names this kit owns.
    pub namespaces: Vec<String>,
    /// Compiler targets the kit declares (`targets` in `kit.kdl`). Which
    /// compilers run is decided by `rhizo-compile`'s registry, not by this.
    pub targets: Vec<String>,
    pub types: IndexMap<String, TypeDef>,
}

#[derive(Debug)]
pub struct TypeDef {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    /// A frame type (an API, an environment) rather than a node type.
    pub frame: bool,
    /// Types allowed as the nearest typed container. Empty: anywhere.
    pub parents: Vec<String>,
    pub ports: Vec<KitPort>,
    /// Default shape, below classes and inline styles in the cascade.
    pub shape: Option<Shape>,
    /// What the type shows when drawn: badge, subtitle, card rows.
    pub look: Option<look::Look>,
    pub schema: serde_json::Value,
    validator: jsonschema::Validator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KitPort {
    pub port: Port,
    /// Used by edges that name no port on this end.
    pub default: bool,
}

impl TypeDef {
    /// The port an edge end uses: the named one, else the default for `dir`.
    pub fn port(&self, name: Option<&str>, dir: PortDir) -> Option<&KitPort> {
        match name {
            Some(n) => self.ports.iter().find(|p| p.port.name.as_str() == n),
            None => self.ports.iter().find(|p| p.default && p.port.dir == dir),
        }
    }

    /// Prop keys the schema declares, in schema order.
    pub fn prop_names(&self) -> Vec<&str> {
        self.schema
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// True when a node of this type may sit in `container`: the type of
    /// its nearest typed parent node or frame ([`container_of`]).
    pub fn fits(&self, container: Option<&str>) -> bool {
        !self.frame && (self.parents.is_empty() || container.is_some_and(|c| self.parents.iter().any(|p| p == c)))
    }

    /// What the schema finds wrong with prop `key` of `props`, if anything.
    /// Problems of the whole object (a required prop missing) are not its.
    pub fn prop_problem(&self, props: &rhizo_core::Props, key: &str) -> Option<String> {
        self.path_problem(props, &[key])
    }

    /// [`Self::prop_problem`] for a prop inside an object prop: `["azure", "resource-group"]`.
    pub fn path_problem(&self, props: &rhizo_core::Props, path: &[&str]) -> Option<String> {
        let instance = serde_json::Value::Object(props.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
        let at: String = path.iter().map(|k| format!("/{}", k.replace('~', "~0").replace('/', "~1"))).collect();
        self.validator
            .iter_errors(&instance)
            .find(|e| {
                let p = e.instance_path().as_str();
                p == at || p.strip_prefix(at.as_str()).is_some_and(|rest| rest.starts_with('/'))
            })
            .map(|e| e.to_string())
    }

    fn closed(&self) -> bool {
        self.schema.get("additionalProperties") == Some(&serde_json::Value::Bool(false))
    }
}

impl Kit {
    pub fn owns(&self, ty: &str) -> bool {
        let ns = ty.split('.').next().unwrap_or(ty);
        self.namespaces.iter().any(|n| n == ns)
    }
}

/// The kits rhizo knows about.
#[derive(Debug)]
pub struct Kits {
    kits: Vec<Kit>,
}

impl Kits {
    /// Kits built into this binary.
    pub fn builtin() -> Kits {
        let data = Kit::parse(include_str!("../kits/data/kit.kdl"), |path| {
            Some(match path {
                "schemas/data.entity.json" => include_str!("../kits/data/schemas/data.entity.json"),
                "schemas/data.field.json" => include_str!("../kits/data/schemas/data.field.json"),
                "schemas/data.enum.json" => include_str!("../kits/data/schemas/data.enum.json"),
                "schemas/data.api.json" => include_str!("../kits/data/schemas/data.api.json"),
                "schemas/data.operation.json" => include_str!("../kits/data/schemas/data.operation.json"),
                "schemas/data.event.json" => include_str!("../kits/data/schemas/data.event.json"),
                "schemas/data.channel.json" => include_str!("../kits/data/schemas/data.channel.json"),
                _ => return None,
            })
        })
        .expect("the built-in data kit is valid");
        let flow = Kit::parse(include_str!("../kits/flow/kit.kdl"), |path| {
            Some(match path {
                "schemas/flow.workflow.json" => include_str!("../kits/flow/schemas/flow.workflow.json"),
                "schemas/flow.trigger.webhook.json" => include_str!("../kits/flow/schemas/flow.trigger.webhook.json"),
                "schemas/flow.trigger.schedule.json" => include_str!("../kits/flow/schemas/flow.trigger.schedule.json"),
                "schemas/flow.trigger.event.json" => include_str!("../kits/flow/schemas/flow.trigger.event.json"),
                "schemas/flow.http-request.json" => include_str!("../kits/flow/schemas/flow.http-request.json"),
                "schemas/flow.switch.json" => include_str!("../kits/flow/schemas/flow.switch.json"),
                "schemas/flow.transform.json" => include_str!("../kits/flow/schemas/flow.transform.json"),
                "schemas/flow.delay.json" => include_str!("../kits/flow/schemas/flow.delay.json"),
                "schemas/flow.call.json" => include_str!("../kits/flow/schemas/flow.call.json"),
                "schemas/flow.script.json" => include_str!("../kits/flow/schemas/flow.script.json"),
                _ => return None,
            })
        })
        .expect("the built-in flow kit is valid");
        let rhizo = Kit::parse(include_str!("../kits/rhizo/kit.kdl"), |path| {
            Some(match path {
                "schemas/rhizo.goal.json" => include_str!("../kits/rhizo/schemas/rhizo.goal.json"),
                "schemas/rhizo.decision.json" => include_str!("../kits/rhizo/schemas/rhizo.decision.json"),
                "schemas/rhizo.flow.json" => include_str!("../kits/rhizo/schemas/rhizo.flow.json"),
                _ => return None,
            })
        })
        .expect("the built-in rhizo kit is valid");
        let infra = Kit::parse(include_str!("../kits/infra/kit.kdl"), |path| {
            Some(match path {
                "schemas/infra.environment.json" => include_str!("../kits/infra/schemas/infra.environment.json"),
                "schemas/aws.vpc.json" => include_str!("../kits/infra/schemas/aws.vpc.json"),
                "schemas/aws.subnet.json" => include_str!("../kits/infra/schemas/aws.subnet.json"),
                "schemas/aws.s3.bucket.json" => include_str!("../kits/infra/schemas/aws.s3.bucket.json"),
                "schemas/aws.dynamodb.table.json" => include_str!("../kits/infra/schemas/aws.dynamodb.table.json"),
                "schemas/aws.rds.postgres.json" => include_str!("../kits/infra/schemas/aws.rds.postgres.json"),
                "schemas/aws.sqs.queue.json" => include_str!("../kits/infra/schemas/aws.sqs.queue.json"),
                "schemas/aws.sns.topic.json" => include_str!("../kits/infra/schemas/aws.sns.topic.json"),
                "schemas/aws.kms.key.json" => include_str!("../kits/infra/schemas/aws.kms.key.json"),
                "schemas/aws.lambda.function.json" => include_str!("../kits/infra/schemas/aws.lambda.function.json"),
                "schemas/aws.apigateway.http.json" => include_str!("../kits/infra/schemas/aws.apigateway.http.json"),
                "schemas/infra.network.json" => include_str!("../kits/infra/schemas/infra.network.json"),
                "schemas/infra.subnet.json" => include_str!("../kits/infra/schemas/infra.subnet.json"),
                "schemas/infra.bucket.json" => include_str!("../kits/infra/schemas/infra.bucket.json"),
                "schemas/infra.table.json" => include_str!("../kits/infra/schemas/infra.table.json"),
                "schemas/infra.database.json" => include_str!("../kits/infra/schemas/infra.database.json"),
                "schemas/infra.queue.json" => include_str!("../kits/infra/schemas/infra.queue.json"),
                "schemas/infra.topic.json" => include_str!("../kits/infra/schemas/infra.topic.json"),
                "schemas/infra.key.json" => include_str!("../kits/infra/schemas/infra.key.json"),
                "schemas/infra.function.json" => include_str!("../kits/infra/schemas/infra.function.json"),
                "schemas/infra.api.json" => include_str!("../kits/infra/schemas/infra.api.json"),
                "schemas/infra.resource.json" => include_str!("../kits/infra/schemas/infra.resource.json"),
                "schemas/imported.json" => include_str!("../kits/infra/schemas/imported.json"),
                "schemas/infra.module.json" => include_str!("../kits/infra/schemas/infra.module.json"),
                _ => return None,
            })
        })
        .expect("the built-in infra kit is valid");
        Kits { kits: vec![data, flow, infra, rhizo] }
    }

    pub fn get(&self, name: &str) -> Option<&Kit> {
        self.kits.iter().find(|k| k.name == name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Kit> {
        self.kits.iter()
    }

    /// The kit owning `ty`'s namespace, and the type if the kit defines it.
    pub fn owner(&self, ty: &str) -> Option<(&Kit, Option<&TypeDef>)> {
        let kit = self.kits.iter().find(|k| k.owns(ty))?;
        Some((kit, kit.types.get(ty)))
    }

    pub fn find_type(&self, ty: &str) -> Option<&TypeDef> {
        self.kits.iter().find_map(|k| k.types.get(ty))
    }

    /// The kits `doc` uses: those it names, plus the always-on `rhizo` kit.
    pub fn used_by(&self, doc: &Document) -> Vec<&Kit> {
        let mut used: Vec<&Kit> = doc.kits.iter().filter_map(|r| self.get(&r.kit)).collect();
        if let Some(k) = self.get(IMPLICIT).filter(|k| !used.iter().any(|u| u.name == k.name)) {
            used.push(k);
        }
        used
    }

    /// Gives typed nodes their kit's default look where the document sets
    /// none. For drawing and exports only: never write the result back.
    pub fn apply_style_defaults(&self, doc: &mut Document) {
        let used = self.used_by(doc);
        for n in doc.nodes.values_mut() {
            let Some(ty) = &n.ty else { continue };
            let def = used.iter().find_map(|k| k.types.get(ty.as_str()));
            if let (None, Some(shape)) = (n.style.shape, def.and_then(|d| d.shape)) {
                n.style.shape = Some(shape);
            }
        }
        doc.decor = look::decorate(doc, |n| used.iter().find_map(|k| k.types.get(n.ty.as_ref()?.as_str())));
    }
}

/// The type of the nearest typed parent node of `id`, else of its frame:
/// what [`TypeDef::parents`] is checked against.
pub fn container_of<'a>(doc: &'a Document, id: &rhizo_core::NodeId) -> Option<&'a str> {
    let n = doc.nodes.get(id)?;
    doc.ancestors(id)
        .filter_map(|a| doc.nodes.get(a)?.ty.as_ref())
        .next()
        .or_else(|| doc.frame(&n.frame)?.ty.as_ref())
        .map(|t| t.as_str())
}

/// A prop an edge takes between nodes of certain types: the kits' rules
/// and compilers read it (`access` on a function's grant to a store, …).
/// Kits declare props for node and frame types in their schemas; edge
/// props live here, next to the rules that read them.
pub struct EdgeProp {
    pub key: &'static str,
    /// Types of the edge's source and target, any of them (AWS names; the
    /// neutral `infra.*` types count as theirs).
    pub from: &'static [&'static str],
    pub to: &'static [&'static str],
    /// The prop's JSON Schema.
    pub schema: &'static str,
}

const FUNCTIONS: &[&str] = &[infra::FUNCTION];
const STORES: &[&str] = &[infra::BUCKET, infra::TABLE, infra::QUEUE, infra::TOPIC, infra::POSTGRES];

pub const EDGE_PROPS: &[EdgeProp] = &[
    EdgeProp {
        key: "access",
        from: FUNCTIONS,
        to: STORES,
        schema: r#"{ "enum": ["read", "write", "read-write"], "description": "What the function may do with the store (default read-write; write for queues and topics)" }"#,
    },
    EdgeProp {
        key: "prefix",
        from: FUNCTIONS,
        to: &[infra::BUCKET],
        schema: r#"{ "type": "string", "description": "Only objects under this key prefix" }"#,
    },
    EdgeProp {
        key: "route",
        from: &[infra::HTTP_API],
        to: FUNCTIONS,
        schema: r#"{ "type": "string", "description": "Route key, e.g. `GET /orders` (default: one per operation the function implements)" }"#,
    },
    EdgeProp {
        key: "cardinality",
        from: &[data::ENTITY],
        to: &[data::ENTITY],
        schema: r#"{ "enum": ["one", "many"], "description": "How many of the target one source refers to" }"#,
    },
];

/// The schema of the props an edge `from` → `to` (their types) takes, as
/// an object schema like a type's; `None` when no kit gives it any.
pub fn edge_schema(from: Option<&str>, to: Option<&str>) -> Option<serde_json::Value> {
    let aws = |t: &str| infra::NEUTRAL.iter().find(|(n, _)| *n == t).map_or(t, |(_, a)| *a).to_string();
    let (from, to) = (aws(from?), aws(to?));
    let props: serde_json::Map<String, serde_json::Value> = EDGE_PROPS
        .iter()
        .filter(|p| p.from.contains(&from.as_str()) && p.to.contains(&to.as_str()))
        .map(|p| (p.key.to_string(), serde_json::from_str(p.schema).expect("edge prop schemas are JSON")))
        .collect();
    (!props.is_empty()).then(|| serde_json::json!({ "type": "object", "properties": props }))
}

/// The kit every document uses without saying so.
pub const IMPLICIT: &str = rhizo::KIT;

pub(crate) fn kind_name(k: Option<EdgeKind>) -> &'static str {
    k.map_or("plain", EdgeKind::name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_constants_name_builtin_types() {
        let kits = Kits::builtin();
        let consts: &[(&str, &[&str])] = &[
            (
                data::KIT,
                &[data::ENTITY, data::FIELD, data::ENUM, data::API, data::OPERATION, data::EVENT, data::CHANNEL],
            ),
            (
                flow::KIT,
                &[
                    flow::WORKFLOW,
                    flow::TRIGGER_WEBHOOK,
                    flow::TRIGGER_SCHEDULE,
                    flow::TRIGGER_EVENT,
                    flow::HTTP_REQUEST,
                    flow::SWITCH,
                    flow::TRANSFORM,
                    flow::DELAY,
                    flow::CALL,
                    flow::SCRIPT,
                ],
            ),
            (
                infra::KIT,
                &[
                    infra::ENVIRONMENT,
                    infra::VPC,
                    infra::SUBNET,
                    infra::BUCKET,
                    infra::TABLE,
                    infra::POSTGRES,
                    infra::QUEUE,
                    infra::TOPIC,
                    infra::KEY,
                    infra::FUNCTION,
                    infra::HTTP_API,
                    infra::RESOURCE,
                    infra::MODULE,
                ],
            ),
            (rhizo::KIT, &[rhizo::GOAL, rhizo::DECISION, rhizo::FLOW]),
        ];
        for (kit, types) in consts {
            let k = kits.get(kit).unwrap_or_else(|| panic!("no built-in kit `{kit}`"));
            for t in *types {
                assert!(k.types.contains_key(*t), "kit `{kit}` has no type `{t}`");
            }
            // Every type has its constant (neutral infra types are in `infra::NEUTRAL`).
            let neutral: Vec<&str> =
                if *kit == infra::KIT { infra::NEUTRAL.iter().map(|(n, _)| *n).collect() } else { Vec::new() };
            for t in &neutral {
                assert!(k.types.contains_key(*t), "kit `{kit}` has no type `{t}`");
            }
            assert_eq!(k.types.len(), types.len() + neutral.len(), "kit `{kit}` has types without a constant");
        }
        assert_eq!(kits.iter().count(), consts.len());
    }

    #[test]
    fn edge_props_follow_the_types_they_join() {
        let s = edge_schema(Some("infra.function"), Some("infra.bucket")).unwrap();
        let keys: Vec<&String> = s["properties"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["access", "prefix"]);
        assert_eq!(s["properties"]["access"]["enum"][1], "write");
        let api = edge_schema(Some(infra::HTTP_API), Some(infra::FUNCTION)).unwrap();
        assert!(api["properties"].get("route").is_some());
        assert!(edge_schema(Some("infra.bucket"), Some("infra.function")).is_none(), "no props the other way");
        assert!(edge_schema(None, Some("infra.bucket")).is_none());
        // Every type named is a built-in one.
        let kits = Kits::builtin();
        for p in EDGE_PROPS {
            for t in p.from.iter().chain(p.to) {
                assert!(kits.find_type(t).is_some(), "{t}");
            }
        }
    }

    #[test]
    fn rules_belong_to_builtin_kits() {
        let kits = Kits::builtin();
        for r in RULES {
            assert!(kits.get(r.kit()).is_some(), "rules for unknown kit `{}`", r.kit());
        }
        let mut names: Vec<&str> = RULES.iter().map(|r| r.kit()).collect();
        names.dedup();
        assert_eq!(names.len(), RULES.len(), "one Rules per kit");
    }

    #[test]
    fn types_fit_their_containers_and_props_report_their_own_problems() {
        let kits = Kits::builtin();
        let op = kits.find_type(data::OPERATION).unwrap();
        assert!(op.fits(Some(data::API)));
        assert!(!op.fits(None) && !op.fits(Some(data::ENTITY)));
        assert!(kits.find_type(rhizo::GOAL).unwrap().fits(None), "no parents: anywhere");
        assert!(!kits.find_type(data::API).unwrap().fits(None), "frame types are not for nodes");
        let mut props = rhizo_core::Props::new();
        props.insert("method".into(), serde_json::json!("fetch"));
        props.insert("path".into(), serde_json::json!("orders"));
        assert!(op.prop_problem(&props, "method").is_some_and(|m| m.contains("fetch")));
        assert!(op.prop_problem(&props, "path").is_some_and(|m| m.contains("^/")));
        assert_eq!(op.prop_problem(&props, "summary"), None);
        props.insert("path".into(), serde_json::json!("/orders"));
        assert_eq!(op.prop_problem(&props, "path"), None);

        let src = "rhizo version=2\nuse data version=\"0.1\"\nframe \"A\" type=data.api {\n    node \"E\" type=data.entity { node \"f\" }\n    node \"g\"\n}\n";
        let doc = rhizo_lang::lower(&rhizo_lang::parse(src, rhizo_lang::Syntax::Kdl).0).0;
        assert_eq!(container_of(&doc, &"f".into()), Some(data::ENTITY));
        assert_eq!(container_of(&doc, &"g".into()), Some(data::API));
        assert_eq!(container_of(&doc, &"nope".into()), None);
    }
}
