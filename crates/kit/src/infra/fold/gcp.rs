//! Google Cloud recognizers: raw `google_*` resources (including the ones
//! rhizo's Google compiler writes) and the terraform-google-modules it emits.
//! Defaults for absent arguments are the provider's or the module's.

use serde_json::{Value, json};

use super::{
    API_N, Arg, BUCKET_N, Block, DATABASE_N, Env, FUNCTION_N, KEY_N, NETWORK_N, Out, QUEUE_N, SUBNET_N, Sat, TABLE_N,
    TOPIC_N, at, boolean, expiry, has, integer, is, items, seconds, string, string_map,
};

pub(super) fn resource(env: &Env, b: &Block, o: &mut Out) -> Option<&'static str> {
    let t = &b.tree;
    Some(match b.tf_type? {
        "google_storage_bucket" => {
            o.read(t, "versioning.enabled", "versioning", Some(json!(false)), boolean);
            o.read(t, "storage_class", "gcp.storage-class", None, string);
            lifecycle(t, "lifecycle_rule", o);
            BUCKET_N
        }
        "google_pubsub_topic" => TOPIC_N,
        "google_pubsub_subscription" => {
            subscription(t, o);
            o.read(t, "dead_letter_policy.max_delivery_attempts", "max-deliveries", None, integer);
            match has(t, "dead_letter_policy") {
                Some(d) => o.set("dead-letter", json!(d)),
                None => o.unread("dead-letter"),
            }
            QUEUE_N
        }
        "google_kms_crypto_key" => {
            o.read(t, "destroy_scheduled_duration", "deletion-window-days", None, days);
            KEY_N
        }
        "google_firestore_database" => {
            let pitr = is("POINT_IN_TIME_RECOVERY_ENABLED");
            o.read(t, "point_in_time_recovery_enablement", "point-in-time-recovery", Some(json!(false)), pitr);
            let protect = is("DELETE_PROTECTION_ENABLED");
            o.read(t, "delete_protection_state", "deletion-protection", Some(json!(false)), protect);
            // A field with a TTL policy is the TTL attribute.
            let ttl: Vec<_> = env
                .satellites(b, "google_firestore_field")
                .into_iter()
                .filter(|f| f.tree.get("ttl_config").is_some())
                .collect();
            o.satellites.extend(ttl.iter().map(|f| f.address.clone()));
            match ttl.as_slice() {
                [f] => o.read(&f.tree, "field", "ttl-attribute", None, string),
                [] => {}
                _ => o.unread("ttl-attribute"),
            }
            TABLE_N
        }
        "google_sql_database_instance" => {
            postgres_version(t, o)?;
            o.read(t, "settings.tier", "gcp.tier", None, string);
            o.read(t, "settings.disk_size", "storage-gb", None, integer);
            o.read(t, "settings.availability_type", "high-availability", Some(json!(false)), is("REGIONAL"));
            o.read(
                t,
                "settings.backup_configuration.backup_retention_settings.retained_backups",
                "backup-days",
                None,
                integer,
            );
            o.read(t, "settings.deletion_protection_enabled", "deletion-protection", Some(json!(false)), boolean);
            match env.satellite(b, "google_sql_database", o) {
                Sat::One(d) => o.read(&d.tree, "name", "database", None, string),
                Sat::Many => o.unread("database"),
                Sat::None => {}
            }
            public_ip(t, "settings.ip_configuration.ipv4_enabled", o);
            DATABASE_N
        }
        "google_compute_network" => NETWORK_N,
        "google_compute_subnetwork" => SUBNET_N,
        "google_cloudfunctions2_function" => {
            o.read(t, "build_config.runtime", "runtime", None, runtime);
            o.read(t, "build_config.entry_point", "handler", None, string);
            o.read(t, "service_config.available_memory", "memory-mb", Some(json!(256)), memory);
            o.read(t, "service_config.timeout_seconds", "timeout-s", Some(json!(60)), integer);
            o.read(t, "service_config.max_instance_count", "gcp.max-instance-count", None, integer);
            o.read(t, "service_config.environment_variables", "environment", None, string_map);
            FUNCTION_N
        }
        "google_api_gateway_gateway" => {
            o.read(t, "region", "gcp.region", None, string);
            API_N
        }
        _ => return None,
    })
}

/// terraform-google-modules, as rhizo's Google compiler emits them.
pub(super) fn module(source: &str, b: &Block, o: &mut Out) -> Option<&'static str> {
    let t = &b.tree;
    Some(match source.strip_prefix("terraform-google-modules/")? {
        "cloud-storage/google//modules/simple_bucket" => {
            o.read(t, "versioning", "versioning", Some(json!(true)), boolean);
            o.read(t, "storage_class", "gcp.storage-class", None, string);
            lifecycle(t, "lifecycle_rules", o);
            BUCKET_N
        }
        "pubsub/google" => {
            let (Some(push), Some(pull)) = (items(t, "push_subscriptions"), items(t, "pull_subscriptions")) else {
                return Some(TOPIC_N);
            };
            let subs: Vec<&Value> = push.into_iter().chain(pull).collect();
            // rhizo's queue: a topic with one subscription of the same name.
            let named = |s: &Value| matches!((at(s, "name"), at(t, "topic")), (Arg::Lit(a), Arg::Lit(b)) if a == b);
            if let [s] = subs.as_slice().iter().filter(|s| named(s)).copied().collect::<Vec<_>>().as_slice()
                && subs.len() == 1
            {
                subscription(s, o);
                match has(s, "dead_letter_topic") {
                    Some(d) => o.set("dead-letter", json!(d)),
                    None => o.unread("dead-letter"),
                }
                o.read(s, "max_delivery_attempts", "max-deliveries", None, integer);
                return Some(QUEUE_N);
            }
            // A topic is ordered when every subscription delivers in order.
            let ordered: Vec<Option<bool>> = subs
                .iter()
                .map(|s| match at(s, "enable_message_ordering") {
                    Arg::Absent | Arg::Lit(Value::Null) => Some(false),
                    Arg::Lit(v) => v.as_bool(),
                    Arg::Expr => None,
                })
                .collect();
            if ordered.contains(&None) {
                o.unread("ordered");
            } else if !ordered.is_empty() {
                o.set("ordered", json!(ordered.iter().all(|x| *x == Some(true))));
            }
            TOPIC_N
        }
        "kms/google" => {
            let keys = items(t, "keys")?;
            let [_] = keys.as_slice() else { return None };
            o.read(t, "key_destroy_scheduled_duration", "deletion-window-days", None, days);
            KEY_N
        }
        "sql-db/google//modules/postgresql" => {
            postgres_version(t, o)?;
            o.read(t, "tier", "gcp.tier", None, string);
            o.read(t, "disk_size", "storage-gb", Some(json!(10)), integer);
            o.read(t, "availability_type", "high-availability", Some(json!(false)), is("REGIONAL"));
            o.read(t, "backup_configuration.retained_backups", "backup-days", None, integer);
            o.read(t, "deletion_protection_enabled", "deletion-protection", Some(json!(false)), boolean);
            o.read(t, "db_name", "database", None, string);
            public_ip(t, "ip_configuration.ipv4_enabled", o);
            DATABASE_N
        }
        "network/google" => {
            // rhizo's own: the private subnet is the first /len+4 of the network's range.
            let private: Vec<&Value> = items(t, "subnets")
                .unwrap_or_default()
                .into_iter()
                .filter(|s| matches!(at(s, "subnet_name"), Arg::Lit(Value::String(n)) if n.ends_with("-private")))
                .collect();
            if let [p] = private.as_slice()
                && let Arg::Lit(Value::String(ip)) = at(p, "subnet_ip")
                && let Some((base, len)) = crate::infra::parse_cidr(ip).filter(|(_, l)| *l >= 4)
            {
                o.set("cidr", json!(format!("{}/{}", std::net::Ipv4Addr::from(base), len - 4)));
            }
            NETWORK_N
        }
        _ => return None,
    })
}

fn subscription(t: &Value, o: &mut Out) {
    o.read(t, "ack_deadline_seconds", "lock-timeout-s", None, integer);
    o.read(t, "message_retention_duration", "retention-s", None, |v| seconds(v).map(Value::from));
    o.read(t, "enable_message_ordering", "ordered", Some(json!(false)), boolean);
}

fn postgres_version(t: &Value, o: &mut Out) -> Option<()> {
    let Arg::Lit(Value::String(v)) = at(t, "database_version") else { return None };
    o.set("engine", json!("postgres"));
    o.set("engine-version", json!(v.strip_prefix("POSTGRES_")?.replace('_', ".")));
    Some(())
}

fn public_ip(t: &Value, path: &str, o: &mut Out) {
    if matches!(at(t, path), Arg::Lit(Value::Bool(true))) {
        o.audit.push(("infra.audit.public", "it has a public IP address: reachable from the internet".into()));
    }
}

/// `"604800s"` → 7 (days).
fn days(v: &Value) -> Option<Value> {
    let s = seconds(v)?;
    (s % 86400 == 0).then(|| json!(s / 86400))
}

/// `"256Mi"`, `"1Gi"`, `"512M"` → megabytes.
fn memory(v: &Value) -> Option<Value> {
    let s = v.as_str()?;
    let split = s.find(|c: char| !c.is_ascii_digit())?;
    let (n, unit) = s.split_at(split);
    let n: i64 = n.parse().ok()?;
    Some(json!(match unit {
        "Mi" | "M" => n,
        "Gi" | "G" => n * 1024,
        _ => return None,
    }))
}

/// `nodejs22` → `nodejs22.x`, `python312` → `python3.12`, `ruby33` → `ruby3.3`.
fn runtime(v: &Value) -> Option<Value> {
    let s = v.as_str()?;
    let digits = s.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let name = &s[..s.len() - digits.len()];
    Some(json!(match name {
        "nodejs" => format!("nodejs{digits}.x"),
        "python" | "ruby" if digits.len() >= 2 => format!("{name}{}.{}", &digits[..1], &digits[1..]),
        "java" | "dotnet" => s.to_string(),
        _ => return None,
    }))
}

/// Lifecycle rules (`lifecycle_rule` blocks, or the module's `lifecycle_rules`
/// list): a Delete rule on age, optionally by prefix, becomes an entry.
fn lifecycle(t: &Value, key: &str, o: &mut Out) {
    let Some(rules) = items(t, key) else {
        o.unread("lifecycle");
        return;
    };
    let mut out = Vec::new();
    for r in rules {
        match at(r, "action.type") {
            Arg::Lit(Value::String(a)) if a == "Delete" => {}
            Arg::Expr => return o.unread("lifecycle"),
            _ => continue,
        }
        let Some(cond) = r.get("condition").map(|c| match c {
            Value::Array(a) if a.len() == 1 => &a[0],
            c => c,
        }) else {
            continue;
        };
        // Only age and prefix: rules on noncurrent versions or state are not neutral.
        if cond.as_object().is_none_or(|m| m.keys().any(|k| !matches!(k.as_str(), "age" | "matches_prefix"))) {
            continue;
        }
        let days = match at(cond, "age") {
            Arg::Lit(d) => d.as_i64(),
            _ => None,
        };
        let prefix = match at(cond, "matches_prefix") {
            Arg::Absent => Some(String::new()),
            Arg::Lit(Value::String(p)) if !p.contains(',') => Some(p.clone()),
            Arg::Lit(Value::Array(p)) if p.len() == 1 => p[0].as_str().map(str::to_string),
            _ => None,
        };
        let (Some(days), Some(prefix)) = (days, prefix) else { return o.unread("lifecycle") };
        out.push(expiry(&prefix, days));
    }
    if !out.is_empty() {
        o.set("lifecycle", Value::Array(out));
    }
}
