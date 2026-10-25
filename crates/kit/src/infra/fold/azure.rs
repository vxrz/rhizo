//! Azure recognizers: raw `azurerm_*` resources and the Azure Verified
//! Modules rhizo emits. Defaults for absent arguments are the provider's or
//! the module's.

use serde_json::{Value, json};

use super::{
    API_N, Arg, BUCKET_N, Block, DATABASE_N, Env, FUNCTION_N, KEY_N, NETWORK_N, Out, QUEUE_N, SUBNET_N, Sat, TABLE_N,
    TOPIC_N, at, boolean, expiry, integer, iso_seconds, items, string, string_map,
};

pub(super) fn resource(env: &Env, b: &Block, o: &mut Out) -> Option<&'static str> {
    let t = &b.tree;
    Some(match b.tf_type? {
        "azurerm_storage_account" => {
            storage(t, o);
            match env.satellite(b, "azurerm_storage_management_policy", o) {
                Sat::One(s) => lifecycle(&s.tree, "rule", o),
                Sat::Many => o.unread("lifecycle"),
                Sat::None => {}
            }
            for s in ["azurerm_storage_container", "azurerm_storage_account_customer_managed_key"] {
                env.satellite(b, s, o);
            }
            BUCKET_N
        }
        "azurerm_servicebus_queue" => {
            queue(t, o);
            QUEUE_N
        }
        "azurerm_servicebus_topic" => {
            o.read(t, "support_ordering", "ordered", Some(json!(false)), boolean);
            TOPIC_N
        }
        "azurerm_key_vault_key" => {
            o.read(t, "tags.description", "description", None, string);
            // The vault it is in holds its soft-delete window.
            let vault = b.refs.iter().find(|(k, _)| k == "key_vault_id").and_then(|(_, r)| super::address(r));
            if let Some(v) = vault.and_then(|a| env.get(&a)).filter(|v| v.managed("azurerm_key_vault")) {
                o.read(&v.tree, "soft_delete_retention_days", "deletion-window-days", Some(json!(90)), integer);
            }
            KEY_N
        }
        "azurerm_cosmosdb_account" => {
            o.read(t, "backup.type", "point-in-time-recovery", Some(json!(false)), |v| {
                Some(json!(v.as_str()? == "Continuous"))
            });
            env.satellite(b, "azurerm_cosmosdb_sql_database", o);
            match env.satellite(b, "azurerm_cosmosdb_sql_container", o) {
                Sat::One(c) => {
                    o.read(&c.tree, "partition_key_paths", "partition-key", None, path_key);
                    o.read(&c.tree, "partition_key_path", "partition-key", None, |v| path_key(&json!([v])));
                }
                Sat::Many => o.unread("partition-key"),
                Sat::None => {}
            }
            TABLE_N
        }
        "azurerm_postgresql_flexible_server" => {
            o.set("engine", json!("postgres"));
            o.read(t, "version", "engine-version", None, string);
            o.read(t, "storage_mb", "storage-gb", None, |v| Some(json!(v.as_i64()? / 1024)));
            o.read(t, "high_availability.mode", "high-availability", Some(json!(false)), |_| Some(json!(true)));
            o.read(t, "backup_retention_days", "backup-days", None, integer);
            o.read(t, "sku_name", "azure.sku-name", None, string);
            match env.satellite(b, "azurerm_postgresql_flexible_server_database", o) {
                Sat::One(d) => o.read(&d.tree, "name", "database", None, string),
                Sat::Many => o.unread("database"),
                Sat::None => {}
            }
            public(t, o);
            DATABASE_N
        }
        "azurerm_virtual_network" => {
            o.read(t, "address_space", "cidr", None, single);
            NETWORK_N
        }
        "azurerm_subnet" => {
            let delegated = at(t, "delegation.service_delegation.name");
            if matches!(delegated, Arg::Lit(Value::String(d)) if d == "Microsoft.DBforPostgreSQL/flexibleServers") {
                o.set("tier", json!("database"));
            }
            SUBNET_N
        }
        "azurerm_function_app_flex_consumption" => {
            runtime(at(t, "runtime_name"), at(t, "runtime_version"), o);
            o.read(t, "instance_memory_in_mb", "azure.instance-memory-mb", None, integer);
            o.read(t, "app_settings", "environment", None, string_map);
            FUNCTION_N
        }
        "azurerm_linux_function_app" => {
            let stack = ["node", "python", "java", "dotnet"]
                .into_iter()
                .find(|s| at(t, &format!("site_config.application_stack.{s}_version")) != Arg::Absent);
            match stack {
                Some(s) => runtime(
                    Arg::Lit(&Value::String(s.into())),
                    at(t, &format!("site_config.application_stack.{s}_version")),
                    o,
                ),
                None if at(t, "site_config") == Arg::Expr => o.unread("runtime"),
                None => {}
            }
            o.read(t, "app_settings", "environment", None, string_map);
            FUNCTION_N
        }
        "azurerm_api_management" => API_N,
        _ => return None,
    })
}

/// Azure Verified Modules, as rhizo's Azure compiler emits them.
pub(super) fn module(source: &str, b: &Block, o: &mut Out) -> Option<&'static str> {
    let t = &b.tree;
    let name = source.strip_prefix("azure/avm-res-")?.strip_suffix("/azurerm")?;
    Some(match name {
        "storage-storageaccount" => {
            storage(t, o);
            lifecycle(t, "storage_management_policy_rule", o);
            BUCKET_N
        }
        "servicebus-namespace" => {
            let (Some(queues), Some(topics)) = (items(t, "queues"), items(t, "topics")) else { return None };
            match (queues.as_slice(), topics.as_slice()) {
                ([q], []) => {
                    queue(q, o);
                    QUEUE_N
                }
                ([], [topic]) => {
                    o.read(topic, "support_ordering", "ordered", Some(json!(false)), boolean);
                    TOPIC_N
                }
                // A namespace of several queues and topics is one module call: no one type.
                _ => return None,
            }
        }
        "keyvault-vault" => {
            let keys = items(t, "keys")?;
            let [k] = keys.as_slice() else { return None };
            o.read(k, "tags.description", "description", None, string);
            o.read(t, "soft_delete_retention_days", "deletion-window-days", Some(json!(90)), integer);
            KEY_N
        }
        "documentdb-databaseaccount" => {
            let dbs = items(t, "sql_databases")?;
            let [db] = dbs.as_slice() else { return None };
            match items(db, "containers").as_deref() {
                Some([c]) => o.read(c, "partition_key_paths", "partition-key", None, path_key),
                Some([]) => {}
                _ => o.unread("partition-key"),
            }
            // The module backs up continuously unless told otherwise.
            o.read(t, "backup.type", "point-in-time-recovery", Some(json!(true)), |v| {
                Some(json!(v.as_str()? == "Continuous"))
            });
            lock(t, o);
            TABLE_N
        }
        "dbforpostgresql-flexibleserver" => {
            o.set("engine", json!("postgres"));
            o.read(t, "server_version", "engine-version", None, string);
            o.read(t, "storage_mb", "storage-gb", None, |v| Some(json!(v.as_i64()? / 1024)));
            // The module is zone-redundant unless high_availability is null.
            match at(t, "high_availability") {
                Arg::Absent => o.set("high-availability", json!(true)),
                Arg::Lit(Value::Null) => o.set("high-availability", json!(false)),
                Arg::Lit(_) => o.set("high-availability", json!(true)),
                Arg::Expr => o.unread("high-availability"),
            }
            o.read(t, "backup_retention_days", "backup-days", None, integer);
            o.read(t, "sku_name", "azure.sku-name", None, string);
            match items(t, "databases").as_deref() {
                Some([d]) => o.read(d, "name", "database", None, string),
                Some(_) => {}
                None => o.unread("database"),
            }
            lock(t, o);
            public(t, o);
            DATABASE_N
        }
        "network-virtualnetwork" => {
            o.read(t, "address_space", "cidr", None, single);
            NETWORK_N
        }
        "web-site" => {
            if !matches!(at(t, "kind"), Arg::Lit(Value::String(k)) if k == "functionapp") {
                return None;
            }
            runtime(at(t, "fc1_runtime_name"), at(t, "fc1_runtime_version"), o);
            o.read(t, "instance_memory_in_mb", "azure.instance-memory-mb", None, integer);
            o.read(t, "app_settings", "environment", None, string_map);
            FUNCTION_N
        }
        "apimanagement-service" => {
            // rhizo's CORS: an API policy with `<allowed-origins>`.
            match items(t, "apis").as_deref() {
                Some([api]) => match at(api, "policy.xml_content") {
                    Arg::Lit(Value::String(xml)) => {
                        let origins = cors_origins(xml);
                        if !origins.is_empty() {
                            o.set("cors-origins", json!(origins));
                        }
                    }
                    Arg::Expr => o.unread("cors-origins"),
                    _ => {}
                },
                Some(_) => {}
                None => o.unread("cors-origins"),
            }
            API_N
        }
        _ => return None,
    })
}

/// The origins of an API Management `<cors>` policy.
fn cors_origins(xml: &str) -> Vec<String> {
    let Some(list) = xml.split_once("<allowed-origins>").and_then(|(_, r)| r.split_once("</allowed-origins>")) else {
        return Vec::new();
    };
    list.0.split("<origin>").skip(1).filter_map(|o| o.split_once("</origin>")).map(|(o, _)| o.to_string()).collect()
}

fn storage(t: &Value, o: &mut Out) {
    o.read(t, "blob_properties.versioning_enabled", "versioning", Some(json!(false)), boolean);
    o.read(t, "access_tier", "azure.access-tier", None, string);
}

fn queue(t: &Value, o: &mut Out) {
    o.read(t, "requires_session", "ordered", Some(json!(false)), boolean);
    o.read(t, "default_message_ttl", "retention-s", None, |v| iso_seconds(v).map(Value::from));
    o.read(t, "lock_duration", "lock-timeout-s", None, |v| iso_seconds(v).map(Value::from));
    o.read(t, "max_delivery_count", "max-deliveries", None, integer);
    o.read(t, "dead_lettering_on_message_expiration", "dead-letter", Some(json!(false)), boolean);
}

/// A `CanNotDelete` lock protects against deletion.
fn lock(t: &Value, o: &mut Out) {
    o.read(t, "lock.kind", "deletion-protection", Some(json!(false)), |v| Some(json!(v.as_str()? == "CanNotDelete")));
}

fn public(t: &Value, o: &mut Out) {
    if matches!(at(t, "public_network_access_enabled"), Arg::Lit(Value::Bool(true))) {
        o.audit.push(("infra.audit.public", "public network access is enabled: reachable from the internet".into()));
    }
}

/// `["10.20.0.0/16"]` → `10.20.0.0/16`.
fn single(v: &Value) -> Option<Value> {
    match v.as_array()?.as_slice() {
        [one] => string(one),
        _ => None,
    }
}

/// `["/cart_id"]` → `cart_id`.
fn path_key(v: &Value) -> Option<Value> {
    match v.as_array()?.as_slice() {
        [p] => Some(json!(p.as_str()?.strip_prefix('/')?)),
        _ => None,
    }
}

/// A Functions runtime (`node` `22`) as the neutral one (`nodejs22.x`).
fn runtime(name: Arg, version: Arg, o: &mut Out) {
    let (Arg::Lit(Value::String(n)), Arg::Lit(v)) = (&name, &version) else {
        if name != Arg::Absent || version != Arg::Absent {
            o.unread("runtime");
        }
        return;
    };
    let Some(Value::String(v)) = string(v) else { return o.unread("runtime") };
    let major = v.split('.').next().unwrap_or(&v);
    let neutral = match n.as_str() {
        "node" => format!("nodejs{major}.x"),
        "python" => format!("python{v}"),
        "java" => format!("java{major}"),
        "dotnet" | "dotnet-isolated" => format!("dotnet{major}"),
        _ => return o.unread("runtime"),
    };
    o.set("runtime", json!(neutral));
}

/// Management policy rules (`rule` blocks of the satellite, or the module's
/// `storage_management_policy_rule` map): a rule that deletes base blobs
/// after a literal number of days becomes an entry. Prefixes start with the
/// container (`data/tmp/`); the neutral prefix is the rest.
fn lifecycle(t: &Value, key: &str, o: &mut Out) {
    let Some(rules) = items(t, key) else {
        o.unread("lifecycle");
        return;
    };
    let mut out = Vec::new();
    for r in rules {
        let days =
            match (at(r, "actions.base_blob.delete_after_days_since_modification_greater_than"), at(r, "enabled")) {
                (Arg::Absent, _) | (_, Arg::Lit(Value::Bool(false))) => continue,
                (Arg::Lit(d), Arg::Absent | Arg::Lit(Value::Bool(true))) => d.as_i64(),
                _ => None,
            };
        let prefix = match at(r, "filters.prefix_match") {
            Arg::Absent => Some(String::new()),
            Arg::Lit(Value::Array(p)) if p.len() == 1 => {
                p[0].as_str().map(|p| p.split_once('/').map_or(String::new(), |(_, rest)| rest.to_string()))
            }
            _ => None,
        };
        let (Some(days), Some(prefix)) = (days, prefix) else { return o.unread("lifecycle") };
        out.push(expiry(&prefix, days));
    }
    if !out.is_empty() {
        o.set("lifecycle", Value::Array(out));
    }
}
