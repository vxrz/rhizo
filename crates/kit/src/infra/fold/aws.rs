//! AWS recognizers: raw `aws_*` resources and the terraform-aws-modules rhizo
//! emits. Defaults for absent arguments are the provider's or the module's.

use serde_json::{Value, json};

use super::{
    API_N, Arg, BUCKET_N, Block, DATABASE_N, Env, FUNCTION_N, KEY_N, NETWORK_N, Out, QUEUE_N, SUBNET_N, Sat, TABLE_N,
    TOPIC_N, at, boolean, expiry, has, integer, items, string, string_map, strings,
};

pub(super) fn resource(env: &Env, b: &Block, o: &mut Out) -> Option<&'static str> {
    let t = &b.tree;
    Some(match b.tf_type? {
        "aws_s3_bucket" => {
            match env.satellite(b, "aws_s3_bucket_versioning", o) {
                Sat::One(s) => {
                    o.read(&s.tree, "versioning_configuration.status", "versioning", Some(json!(false)), |v| {
                        Some(json!(v.as_str()? == "Enabled"))
                    })
                }
                Sat::Many => o.unread("versioning"),
                Sat::None => o.read(t, "versioning.enabled", "versioning", Some(json!(false)), boolean),
            }
            match env.satellite(b, "aws_s3_bucket_lifecycle_configuration", o) {
                Sat::One(s) => lifecycle(&s.tree, "rule", o),
                Sat::Many => o.unread("lifecycle"),
                Sat::None => {}
            }
            for s in [
                "aws_s3_bucket_server_side_encryption_configuration",
                "aws_s3_bucket_public_access_block",
                "aws_s3_bucket_policy",
                "aws_s3_bucket_ownership_controls",
            ] {
                env.satellite(b, s, o);
            }
            BUCKET_N
        }
        "aws_sqs_queue" => {
            queue(t, "fifo_queue", o);
            match has(t, "redrive_policy") {
                Some(d) => o.set("dead-letter", json!(d)),
                None => o.unread("dead-letter"),
            }
            o.read(t, "redrive_policy.maxReceiveCount", "max-deliveries", None, integer);
            QUEUE_N
        }
        "aws_sns_topic" => {
            o.read(t, "fifo_topic", "ordered", Some(json!(false)), boolean);
            TOPIC_N
        }
        "aws_kms_key" => {
            key(t, o);
            KEY_N
        }
        "aws_lambda_function" => {
            function(t, "environment.variables", o);
            FUNCTION_N
        }
        "aws_dynamodb_table" => {
            o.read(t, "hash_key", "partition-key", None, string);
            o.read(t, "range_key", "sort-key", None, string);
            key_types(t, "attribute", o);
            match (at(t, "ttl.enabled"), at(t, "ttl.attribute_name")) {
                (Arg::Lit(Value::Bool(true)), Arg::Lit(Value::String(a))) => o.set("ttl-attribute", json!(a)),
                (Arg::Expr, _) | (_, Arg::Expr) => o.unread("ttl-attribute"),
                _ => {}
            }
            o.read(t, "point_in_time_recovery.enabled", "point-in-time-recovery", Some(json!(false)), boolean);
            o.read(t, "deletion_protection_enabled", "deletion-protection", Some(json!(false)), boolean);
            TABLE_N
        }
        "aws_db_instance" => {
            database(t, o)?;
            DATABASE_N
        }
        "aws_vpc" => {
            o.read(t, "cidr_block", "cidr", None, string);
            let logs = env.satellite(b, "aws_flow_log", o);
            o.set("flow-logs", json!(!matches!(logs, Sat::None)));
            NETWORK_N
        }
        "aws_subnet" => {
            if matches!(at(t, "map_public_ip_on_launch"), Arg::Lit(Value::Bool(true))) {
                o.set("tier", json!("public"));
            }
            SUBNET_N
        }
        "aws_apigatewayv2_api" => {
            if !matches!(at(t, "protocol_type"), Arg::Lit(Value::String(p)) if p == "HTTP") {
                return None;
            }
            o.read(t, "cors_configuration.allow_origins", "cors-origins", None, strings);
            API_N
        }
        _ => return None,
    })
}

/// terraform-aws-modules, as rhizo's AWS compiler emits them.
pub(super) fn module(source: &str, b: &Block, o: &mut Out) -> Option<&'static str> {
    let t = &b.tree;
    let name = source.strip_prefix("terraform-aws-modules/")?.strip_suffix("/aws")?;
    Some(match name {
        "s3-bucket" => {
            o.read(t, "versioning.enabled", "versioning", Some(json!(false)), boolean);
            lifecycle(t, "lifecycle_rule", o);
            BUCKET_N
        }
        "sqs" => {
            queue(t, "fifo_queue", o);
            o.read(t, "create_dlq", "dead-letter", Some(json!(false)), boolean);
            o.read(t, "redrive_policy.maxReceiveCount", "max-deliveries", None, integer);
            QUEUE_N
        }
        "sns" => {
            o.read(t, "fifo_topic", "ordered", Some(json!(false)), boolean);
            TOPIC_N
        }
        "kms" => {
            key(t, o);
            KEY_N
        }
        "lambda" => {
            function(t, "environment_variables", o);
            o.read(t, "cloudwatch_logs_retention_in_days", "aws.log-retention-days", None, integer);
            FUNCTION_N
        }
        "dynamodb-table" => {
            o.read(t, "hash_key", "partition-key", None, string);
            o.read(t, "range_key", "sort-key", None, string);
            key_types(t, "attributes", o);
            match (at(t, "ttl_enabled"), at(t, "ttl_attribute_name")) {
                (Arg::Lit(Value::Bool(true)), Arg::Lit(Value::String(a))) => o.set("ttl-attribute", json!(a)),
                (Arg::Expr, _) | (_, Arg::Expr) => o.unread("ttl-attribute"),
                _ => {}
            }
            o.read(t, "point_in_time_recovery_enabled", "point-in-time-recovery", Some(json!(false)), boolean);
            o.read(t, "deletion_protection_enabled", "deletion-protection", Some(json!(false)), boolean);
            TABLE_N
        }
        "rds" => {
            database(t, o)?;
            DATABASE_N
        }
        "vpc" => {
            o.read(t, "cidr", "cidr", None, string);
            match at(t, "azs") {
                Arg::Lit(v) => o.set("zones", json!(v.as_array().map_or(0, Vec::len))),
                // rhizo's own: the first N zones of the region.
                Arg::Expr => {
                    match b.props.get("body").and_then(|x| x.get("azs")).and_then(Value::as_str).and_then(first_zones) {
                        Some(n) => o.set("zones", json!(n)),
                        None => o.unread("zones"),
                    }
                }
                Arg::Absent => {}
            }
            let flag = |k: &str| match at(t, k) {
                Arg::Absent | Arg::Lit(Value::Null) => Some(false),
                Arg::Lit(v) => boolean(v).and_then(|b| b.as_bool()),
                Arg::Expr => None,
            };
            match (flag("enable_nat_gateway"), flag("single_nat_gateway"), flag("one_nat_gateway_per_az")) {
                (Some(false), _, _) => o.set("nat", json!("none")),
                (Some(true), Some(true), _) => o.set("nat", json!("single")),
                // One per zone, or one per subnet: at least one per zone.
                (Some(true), Some(false), Some(_)) => o.set("nat", json!("per-az")),
                _ => o.unread("nat"),
            }
            o.read(t, "enable_flow_log", "flow-logs", Some(json!(false)), boolean);
            NETWORK_N
        }
        "apigateway-v2" => {
            match at(t, "protocol_type") {
                Arg::Absent => {}
                Arg::Lit(Value::String(p)) if p == "HTTP" => {}
                _ => return None,
            }
            o.read(t, "cors_configuration.allow_origins", "cors-origins", None, strings);
            API_N
        }
        _ => return None,
    })
}

/// `slice(<zones>, 0, 3)` → 3.
fn first_zones(text: &str) -> Option<i64> {
    let hcl_edit::expr::Expression::FuncCall(f) = hcl_edit::parser::parse_expr(text).ok()? else { return None };
    let args: Vec<_> = f.args.iter().collect();
    match (f.name.name.as_str(), args.as_slice()) {
        ("slice", [_, from, to]) if super::lit(from) == json!(0) => super::lit(to).as_i64(),
        _ => None,
    }
}

fn queue(t: &Value, fifo: &str, o: &mut Out) {
    o.read(t, fifo, "ordered", Some(json!(false)), boolean);
    o.read(t, "message_retention_seconds", "retention-s", None, integer);
    o.read(t, "visibility_timeout_seconds", "lock-timeout-s", None, integer);
}

fn key(t: &Value, o: &mut Out) {
    o.read(t, "description", "description", None, string);
    o.read(t, "deletion_window_in_days", "deletion-window-days", None, integer);
}

fn function(t: &Value, env: &str, o: &mut Out) {
    o.read(t, "runtime", "runtime", None, string);
    o.read(t, "handler", "handler", None, string);
    o.read(t, "memory_size", "memory-mb", Some(json!(128)), integer);
    o.read(t, "timeout", "timeout-s", Some(json!(3)), integer);
    o.read(t, "architectures", "aws.architecture", Some(json!("x86_64")), |v| match v.as_array()?.as_slice() {
        [a] => string(a),
        _ => None,
    });
    o.read(t, env, "environment", None, string_map);
}

/// Postgres only: another engine is not an `infra.database`.
fn database(t: &Value, o: &mut Out) -> Option<()> {
    match at(t, "engine") {
        Arg::Lit(Value::String(e)) if e == "postgres" => o.set("engine", json!("postgres")),
        _ => return None,
    }
    o.read(t, "engine_version", "engine-version", None, string);
    o.read(t, "allocated_storage", "storage-gb", None, integer);
    o.read(t, "multi_az", "high-availability", Some(json!(false)), boolean);
    o.read(t, "backup_retention_period", "backup-days", None, integer);
    o.read(t, "deletion_protection", "deletion-protection", Some(json!(false)), boolean);
    o.read(t, "db_name", "database", None, string);
    o.read(t, "instance_class", "aws.instance-class", None, string);
    if matches!(at(t, "publicly_accessible"), Arg::Lit(Value::Bool(true))) {
        o.audit.push(("infra.audit.public", "it is publicly accessible: reachable from the internet".into()));
    }
    Some(())
}

/// Attribute types of the partition and sort keys.
fn key_types(t: &Value, attrs: &str, o: &mut Out) {
    let Some(list) = items(t, attrs) else {
        o.unread("partition-key-type");
        return;
    };
    for (key, prop) in [("partition-key", "partition-key-type"), ("sort-key", "sort-key-type")] {
        let Some(name) = o.props.get(key).and_then(Value::as_str).map(str::to_string) else { continue };
        if let Some(a) = list.iter().find(|a| a.get("name").and_then(Value::as_str) == Some(name.as_str())) {
            o.read(a, "type", prop, None, string);
        }
    }
}

/// S3 lifecycle rules (`rule` blocks of the satellite, or the module's
/// `lifecycle_rule` list): each enabled rule that expires objects after a
/// literal number of days becomes an entry; other rules are not neutral.
fn lifecycle(t: &Value, key: &str, o: &mut Out) {
    let Some(rules) = items(t, key) else {
        o.unread("lifecycle");
        return;
    };
    let mut out = Vec::new();
    for r in rules {
        let enabled = match (at(r, "status"), at(r, "enabled")) {
            (Arg::Lit(Value::String(s)), _) => s == "Enabled",
            (_, Arg::Lit(Value::Bool(b))) => *b,
            (Arg::Absent, Arg::Absent) => true,
            _ => return o.unread("lifecycle"),
        };
        let days = match at(r, "expiration.days") {
            Arg::Absent => continue,
            Arg::Lit(v) => integer(v),
            Arg::Expr => None,
        };
        let prefix = match (at(r, "filter.prefix"), at(r, "prefix")) {
            (Arg::Lit(Value::String(p)), _) | (Arg::Absent, Arg::Lit(Value::String(p))) => Some(p.as_str()),
            (Arg::Absent, Arg::Absent) => Some(""),
            _ => None,
        };
        let (Some(days), Some(prefix)) = (days.and_then(|d| d.as_i64()), prefix) else {
            return o.unread("lifecycle");
        };
        if enabled {
            out.push(expiry(prefix, days));
        }
    }
    if !out.is_empty() {
        o.set("lifecycle", Value::Array(out));
    }
}
