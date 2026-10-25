//! Adopting imported nodes (docs/design/iac-import.md, "Adopt"): a folded
//! node switches from its verbatim code to rhizo's own shape, so its props
//! become editable. Its `origin` then says `shape "rhizo"`, keeps the
//! address it came from and the satellites folded into it (`also`), which
//! the compiler moves into rhizo's code, and the arguments rhizo carries
//! over unchanged (`keep`: names, tags, descriptions).
//!
//! The tables here say how raw resources map into the modules rhizo emits:
//! where each one lives inside the module, which satellites the module
//! manages, and which module output replaces each attribute other code
//! reads. Addresses were read from the pinned module sources
//! (terraform-aws-modules s3-bucket 5.17.0, sqs 5.2.2, sns 7.2.0, kms 4.2.2,
//! dynamodb-table 5.5.2; terraform-google-modules cloud-storage
//! simple_bucket 12.4.1, pubsub 8.8.1, kms 4.2.1; Azure Verified Module
//! avm-res-keyvault-vault 0.12.0), not from memory.

use rhizo_core::{Node, Props};
use serde_json::Value;

/// Whether a node was adopted: rhizo generates its code from its props.
pub fn adopted(props: &Props) -> bool {
    props.get("origin").and_then(|o| o.get("shape")).and_then(Value::as_str) == Some("rhizo")
}

/// The OpenTofu name an imported node goes by: `aws_s3_bucket.receipts` and
/// `module.receipts` → `receipts`.
pub fn label(address: &str) -> &str {
    address.rsplit('.').next().unwrap_or(address)
}

/// The name rhizo's code for an imported node goes by: the adopted node's
/// own (`origin.name`), else the name of the code it came from.
pub fn code_name(props: &Props) -> Option<&str> {
    let o = props.get("origin")?;
    o.get("name").and_then(Value::as_str).or_else(|| o.get("address").and_then(Value::as_str).map(label))
}

/// The addresses an adopted node moves into its code: where it came from,
/// then its satellites.
pub fn moves(props: &Props) -> Vec<String> {
    let o = props.get("origin");
    let address = o.and_then(|o| o.get("address")).and_then(Value::as_str);
    let also = o.and_then(|o| o.get("also")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
    address.into_iter().chain(also).map(str::to_string).collect()
}

/// Arguments an adopted node carries from its original code: (key, HCL).
pub fn keep(props: &Props) -> Vec<(&str, &str)> {
    let keep = props.get("origin").and_then(|o| o.get("keep")).and_then(Value::as_object);
    keep.into_iter().flatten().filter_map(|(k, v)| Some((k.as_str(), v.as_str()?))).collect()
}

/// Arguments of the original code that adopting keeps: they name or tag
/// what exists, and rhizo would write them differently. Per cloud.
pub fn keep_keys(cloud: &str, neutral: &str) -> &'static [&'static str] {
    match (cloud, neutral) {
        // A key's description is a prop.
        ("aws", "infra.key") => &["aliases", "tags"],
        ("aws", _) => &["bucket", "bucket_prefix", "name", "identifier", "function_name", "tags"],
        ("azure", _) => &["name", "tags"],
        // A raw Firestore database is rhizo's own resource: where it is stays too.
        ("gcp", "infra.table") => &["name", "labels", "project", "location_id"],
        ("gcp", _) => &["name", "labels"],
        _ => &[],
    }
}

/// The argument rhizo writes a node's label into: adopting makes its
/// value the node's label.
pub fn label_arg(cloud: &str, neutral: &str) -> Option<&'static str> {
    match (cloud, neutral) {
        ("aws" | "gcp", "infra.function") | ("aws", "infra.api") => Some("description"),
        ("azure", "infra.api") => Some("publisher_name"),
        _ => None,
    }
}

/// The resource rhizo's code for a neutral type is, where it is not a
/// module call (Google Cloud functions, Firestore tables and API gateways).
pub fn rhizo_resource(cloud: &str, neutral: &str) -> Option<&'static str> {
    match (cloud, neutral) {
        ("gcp", "infra.function") => Some("google_cloudfunctions2_function"),
        ("gcp", "infra.table") => Some("google_firestore_database"),
        ("gcp", "infra.api") => Some("google_api_gateway_gateway"),
        _ => None,
    }
}

/// Whether rhizo's code can refer to a node of an imported environment of
/// `cloud` by its name: it is adopted, or folded from a module call (rhizo
/// refers to `module.<name>`) or from the resource rhizo's own code for its
/// type is.
pub fn rhizo_shaped(n: &Node, cloud: &str) -> bool {
    let neutral = super::written_ty(n);
    if adopted(&n.props) {
        return true;
    }
    if !n.props.contains_key("origin") || !super::NEUTRAL.iter().any(|(t, _)| *t == neutral) {
        return false;
    }
    match n.props.get("tf-type").and_then(Value::as_str) {
        None => n
            .props
            .get("origin")
            .and_then(|o| o.get("address"))
            .and_then(Value::as_str)
            .is_some_and(|a| a.starts_with("module.")),
        Some(t) => rhizo_resource(cloud, neutral) == Some(t),
    }
}

/// Arguments that say the same thing two ways: a kept one replaces the
/// other in rhizo's code.
pub const ALTERNATIVES: [(&str, &str); 1] = [("bucket", "bucket_prefix")];

/// A satellite resource a module manages.
pub struct Satellite {
    pub ty: &'static str,
    /// Its address inside the module.
    pub at: &'static str,
    /// Whether rhizo's code for the node creates it.
    pub manages: fn(&Node) -> bool,
    /// For a resource the main one sits in (a key's vault or key ring),
    /// not one folded into it: the main resource's argument naming it.
    pub via: Option<&'static str>,
    /// Attribute other code reads → the module output with its value.
    pub outputs: &'static [(&'static str, &'static str)],
}

/// An argument adopting carries from the original code into rhizo's,
/// where rhizo writes it under another name or place.
pub struct Keep {
    /// The satellite type it is read from; `None` for the main resource.
    pub of: Option<&'static str>,
    pub arg: &'static str,
    /// Where rhizo's module call takes it: an input, or a path into one
    /// (`keys.key.name`).
    pub to: &'static str,
    /// rhizo's input is a list of it (`keys = [<name>]`).
    pub list: bool,
}

/// A raw resource rhizo's shape takes over: a module call at
/// `module.<name>` with the resource inside.
pub struct Raw {
    pub cloud: &'static str,
    pub ty: &'static str,
    pub neutral: &'static str,
    /// Its address inside the module.
    pub at: &'static str,
    pub satellites: &'static [Satellite],
    /// Attribute other code reads → the module output with its value.
    pub outputs: &'static [(&'static str, &'static str)],
    /// What adopting keeps (names, places, labels), where rhizo's inputs
    /// are named otherwise; empty: [`keep_keys`] under their own names.
    pub keep: &'static [Keep],
}

fn always(_: &Node) -> bool {
    true
}

fn versioned(n: &Node) -> bool {
    super::flag(n, "versioning", true)
}

fn lifecycle(n: &Node) -> bool {
    n.props.get("lifecycle").and_then(Value::as_array).is_some_and(|a| !a.is_empty())
}

/// The raw resources rhizo can adopt.
pub const RAW: [Raw; 9] = [
    Raw {
        cloud: "aws",
        ty: "aws_s3_bucket",
        neutral: "infra.bucket",
        at: "aws_s3_bucket.this[0]",
        satellites: &[
            Satellite {
                ty: "aws_s3_bucket_versioning",
                at: "aws_s3_bucket_versioning.this[0]",
                manages: versioned,
                via: None,
                outputs: &[],
            },
            Satellite {
                ty: "aws_s3_bucket_public_access_block",
                at: "aws_s3_bucket_public_access_block.this[0]",
                manages: always,
                via: None,
                outputs: &[],
            },
            Satellite {
                ty: "aws_s3_bucket_ownership_controls",
                at: "aws_s3_bucket_ownership_controls.this[0]",
                manages: always,
                via: None,
                outputs: &[],
            },
            Satellite {
                ty: "aws_s3_bucket_server_side_encryption_configuration",
                at: "aws_s3_bucket_server_side_encryption_configuration.this[0]",
                manages: always,
                via: None,
                outputs: &[],
            },
            Satellite {
                ty: "aws_s3_bucket_policy",
                at: "aws_s3_bucket_policy.this[0]",
                manages: always,
                via: None,
                outputs: &[],
            },
            Satellite {
                ty: "aws_s3_bucket_lifecycle_configuration",
                at: "aws_s3_bucket_lifecycle_configuration.this[0]",
                manages: lifecycle,
                via: None,
                outputs: &[],
            },
        ],
        outputs: &[
            ("id", "s3_bucket_id"),
            ("bucket", "s3_bucket_id"),
            ("arn", "s3_bucket_arn"),
            ("bucket_domain_name", "s3_bucket_bucket_domain_name"),
            ("bucket_regional_domain_name", "s3_bucket_bucket_regional_domain_name"),
            ("hosted_zone_id", "s3_bucket_hosted_zone_id"),
            ("bucket_region", "s3_bucket_region"),
            ("tags", "s3_bucket_tags"),
        ],
        keep: &[],
    },
    Raw {
        cloud: "aws",
        ty: "aws_sqs_queue",
        neutral: "infra.queue",
        at: "aws_sqs_queue.this[0]",
        satellites: &[],
        outputs: &[("id", "queue_url"), ("url", "queue_url"), ("arn", "queue_arn"), ("name", "queue_name")],
        keep: &[],
    },
    Raw {
        cloud: "aws",
        ty: "aws_sns_topic",
        neutral: "infra.topic",
        at: "aws_sns_topic.this[0]",
        satellites: &[],
        outputs: &[("arn", "topic_arn"), ("id", "topic_id"), ("name", "topic_name"), ("owner", "topic_owner")],
        keep: &[],
    },
    Raw {
        cloud: "aws",
        ty: "aws_kms_key",
        neutral: "infra.key",
        at: "aws_kms_key.this[0]",
        satellites: &[],
        outputs: &[("arn", "key_arn"), ("key_id", "key_id"), ("id", "key_id"), ("policy", "key_policy")],
        keep: &[],
    },
    Raw {
        cloud: "aws",
        ty: "aws_dynamodb_table",
        neutral: "infra.table",
        at: "aws_dynamodb_table.this[0]",
        satellites: &[],
        outputs: &[
            ("id", "dynamodb_table_id"),
            ("name", "dynamodb_table_id"),
            ("arn", "dynamodb_table_arn"),
            ("stream_arn", "dynamodb_table_stream_arn"),
            ("stream_label", "dynamodb_table_stream_label"),
        ],
        keep: &[],
    },
    // Google Cloud: rhizo's code names the project as `project_id`.
    Raw {
        cloud: "gcp",
        ty: "google_storage_bucket",
        neutral: "infra.bucket",
        at: "google_storage_bucket.bucket",
        satellites: &[],
        outputs: &[
            ("name", "name"),
            ("url", "url"),
            ("id", "bucket.id"),
            ("self_link", "bucket.self_link"),
            ("location", "bucket.location"),
            ("project", "bucket.project"),
        ],
        keep: &[
            Keep { of: None, arg: "name", to: "name", list: false },
            Keep { of: None, arg: "location", to: "location", list: false },
            Keep { of: None, arg: "project", to: "project_id", list: false },
            Keep { of: None, arg: "labels", to: "labels", list: false },
        ],
    },
    Raw {
        cloud: "gcp",
        ty: "google_pubsub_topic",
        neutral: "infra.topic",
        at: "google_pubsub_topic.topic[0]",
        satellites: &[],
        outputs: &[("id", "id"), ("name", "topic")],
        keep: &[
            Keep { of: None, arg: "name", to: "topic", list: false },
            Keep { of: None, arg: "project", to: "project_id", list: false },
            Keep { of: None, arg: "labels", to: "topic_labels", list: false },
        ],
    },
    Raw {
        cloud: "gcp",
        ty: "google_kms_crypto_key",
        neutral: "infra.key",
        at: "google_kms_crypto_key.key[0]",
        satellites: &[Satellite {
            ty: "google_kms_key_ring",
            at: "google_kms_key_ring.key_ring",
            manages: always,
            via: Some("key_ring"),
            outputs: &[("id", "keyring"), ("name", "keyring_name")],
        }],
        // The module's `keys` output is a map by key name; its list has the one key.
        outputs: &[("id", "key_id_list[0]")],
        keep: &[
            Keep { of: Some("google_kms_key_ring"), arg: "name", to: "keyring", list: false },
            Keep { of: Some("google_kms_key_ring"), arg: "location", to: "location", list: false },
            Keep { of: Some("google_kms_key_ring"), arg: "project", to: "project_id", list: false },
            Keep { of: None, arg: "name", to: "keys", list: true },
            Keep { of: None, arg: "labels", to: "labels", list: false },
        ],
    },
    // Azure: the key vault module (azurerm inside, so the moves stay within one type).
    Raw {
        cloud: "azure",
        ty: "azurerm_key_vault_key",
        neutral: "infra.key",
        at: "module.keys[\"key\"].azurerm_key_vault_key.this",
        satellites: &[Satellite {
            ty: "azurerm_key_vault",
            at: "azurerm_key_vault.this",
            manages: always,
            via: Some("key_vault_id"),
            outputs: &[("id", "resource_id"), ("name", "name"), ("vault_uri", "uri")],
        }],
        outputs: &[
            ("id", "keys[\"key\"].id"),
            ("versionless_id", "keys[\"key\"].versionless_id"),
            ("resource_id", "keys[\"key\"].resource_id"),
            ("resource_versionless_id", "keys[\"key\"].resource_versionless_id"),
            ("name", "keys[\"key\"].name"),
            ("public_key_pem", "keys[\"key\"].public_key_pem"),
            ("public_key_openssh", "keys[\"key\"].public_key_openssh"),
            ("n", "keys[\"key\"].n"),
            ("e", "keys[\"key\"].e"),
        ],
        keep: &[
            Keep { of: Some("azurerm_key_vault"), arg: "name", to: "name", list: false },
            Keep { of: Some("azurerm_key_vault"), arg: "location", to: "location", list: false },
            Keep { of: Some("azurerm_key_vault"), arg: "resource_group_name", to: "resource_group_name", list: false },
            Keep { of: Some("azurerm_key_vault"), arg: "tenant_id", to: "tenant_id", list: false },
            Keep { of: Some("azurerm_key_vault"), arg: "tags", to: "tags", list: false },
            Keep { of: None, arg: "name", to: "keys.key.name", list: false },
        ],
    },
];

/// The table entry for a raw resource type.
pub fn raw(ty: &str) -> Option<&'static Raw> {
    RAW.iter().find(|r| r.ty == ty)
}

/// The table entry for a satellite type: what reading it means once it is
/// inside rhizo's code.
pub fn satellite(ty: &str) -> Option<&'static Satellite> {
    RAW.iter().flat_map(|r| r.satellites).find(|s| s.ty == ty)
}

/// Where a resource of type `ty` (a main resource or a satellite) lives
/// inside the module rhizo adopts it into.
pub fn inside(ty: &str) -> Option<&'static str> {
    RAW.iter()
        .find_map(|r| if r.ty == ty { Some(r.at) } else { r.satellites.iter().find(|s| s.ty == ty).map(|s| s.at) })
}

/// The resource type of an address: `aws_s3_bucket.receipts` →
/// `aws_s3_bucket`; `None` for module calls and data sources.
pub fn resource_type(address: &str) -> Option<&str> {
    let (ty, _) = address.split_once('.')?;
    (ty != "module" && ty != "data" && ty != "ephemeral").then_some(ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_answer_by_type() {
        assert_eq!(inside("aws_s3_bucket"), Some("aws_s3_bucket.this[0]"));
        assert_eq!(inside("aws_s3_bucket_policy"), Some("aws_s3_bucket_policy.this[0]"));
        assert_eq!(inside("aws_lambda_function"), None);
        assert_eq!(inside("google_kms_key_ring"), Some("google_kms_key_ring.key_ring"));
        assert_eq!(inside("azurerm_key_vault_key"), Some("module.keys[\"key\"].azurerm_key_vault_key.this"));
        assert_eq!(satellite("azurerm_key_vault").and_then(|s| s.via), Some("key_vault_id"));
        assert!(satellite("aws_s3_bucket_policy").is_some_and(|s| s.via.is_none() && s.outputs.is_empty()));
        // Every raw entry keeps its name (or has rhizo's own name rules for it).
        for r in RAW.iter().filter(|r| r.cloud != "aws") {
            assert!(r.keep.iter().any(|k| k.of.is_none() && k.arg == "name"), "{}", r.ty);
        }
        assert_eq!(label("module.orders_db"), "orders_db");
        assert_eq!(resource_type("aws_s3_bucket_versioning.receipts"), Some("aws_s3_bucket_versioning"));
        assert_eq!(resource_type("module.x"), None);
        let mut n = Node::new("b", "b", rhizo_core::FrameId::from("f"));
        assert!(versioned(&n) && !lifecycle(&n));
        n.props.insert("versioning".into(), Value::Bool(false));
        assert!(!versioned(&n));
    }
}
