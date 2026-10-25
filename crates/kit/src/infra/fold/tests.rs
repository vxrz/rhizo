use hcl_edit::Span;
use hcl_edit::structure::{Body, Structure};
use serde_json::{Map, Value, json};

use super::*;

/// Imported nodes as `rhizo import` writes them: each block's arguments as
/// their HCL text, nested blocks as arrays of bodies.
fn nodes(hcl: &str) -> Vec<Props> {
    fn body(b: &Body, text: &str) -> Map<String, Value> {
        let mut out = Map::new();
        for s in b.iter() {
            match s {
                Structure::Attribute(a) => {
                    out.insert(a.key.to_string(), json!(text[a.value.span().unwrap()].to_string()));
                }
                Structure::Block(b) => {
                    let key = std::iter::once(b.ident.to_string())
                        .chain(b.labels.iter().map(|l| format!("\"{}\"", l.as_str())))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let v = out.entry(key).or_insert_with(|| json!([]));
                    v.as_array_mut().unwrap().push(Value::Object(body(&b.body, text)));
                }
            }
        }
        out
    }
    let parsed = hcl_edit::parser::parse_body(hcl).unwrap();
    parsed
        .iter()
        .filter_map(Structure::as_block)
        .map(|b| {
            let labels: Vec<&str> = b.labels.iter().map(|l| l.as_str()).collect();
            let mut p = Props::new();
            let mut inner = body(&b.body, hcl);
            let address = match (b.ident.as_str(), labels.as_slice()) {
                ("module", [n]) => {
                    for k in ["source", "version"] {
                        if let Some(v) = inner.remove(k) {
                            p.insert(k.into(), json!(v.as_str().unwrap().trim_matches('"')));
                        }
                    }
                    p.insert("name".into(), json!(n));
                    format!("module.{n}")
                }
                (kind, [t, n]) => {
                    p.insert("tf-type".into(), json!(t));
                    if kind == "data" {
                        p.insert("mode".into(), json!("data"));
                    }
                    p.insert("name".into(), json!(n));
                    if kind == "data" { format!("data.{t}.{n}") } else { format!("{t}.{n}") }
                }
                _ => unreachable!(),
            };
            p.insert("body".into(), Value::Object(inner));
            p.insert("origin".into(), json!({ "address": address, "shape": "resource" }));
            p
        })
        .collect()
}

type Folds = Vec<(String, Option<Fold>)>;
type Edges = Vec<(String, String, Option<Link>, Option<&'static str>)>;

/// Folds every block; returns (address, fold) pairs, the env's edges and
/// each block's parent.
fn fold_all(hcl: &str) -> (Folds, Edges, Vec<Option<String>>) {
    let props = nodes(hcl);
    let env = Env::new(props.iter());
    let folds = env.folds();
    let addr = |i: usize| env.blocks[i].address.clone();
    let edges = env.edges(&folds).into_iter().map(|e| (addr(e.from), addr(e.to), e.link, e.access)).collect();
    let parents = (0..folds.len()).map(|i| env.parent(i, &folds).map(addr)).collect();
    ((0..folds.len()).map(|i| (addr(i), folds[i].clone())).collect(), edges, parents)
}

fn get<'f>(folds: &'f [(String, Option<Fold>)], a: &str) -> &'f Fold {
    folds.iter().find(|(x, _)| x == a).and_then(|(_, f)| f.as_ref()).unwrap_or_else(|| panic!("{a} is not folded"))
}

fn props(f: &Fold) -> Value {
    Value::Object(f.props.clone().into_iter().collect())
}

#[test]
fn aws_bucket_with_satellites() {
    let (folds, edges, _) = fold_all(
        r#"
resource "aws_kms_key" "k" {
  description             = "data"
  deletion_window_in_days = 7
}
resource "aws_s3_bucket" "b" {
  bucket = "x-${var.env}"
}
resource "aws_s3_bucket_versioning" "b" {
  bucket = aws_s3_bucket.b.id
  versioning_configuration {
    status = "Enabled"
  }
}
resource "aws_s3_bucket_lifecycle_configuration" "b" {
  bucket = aws_s3_bucket.b.id
  rule {
    id     = "tmp"
    status = "Enabled"
    filter {
      prefix = "tmp/"
    }
    expiration {
      days = 7
    }
  }
  rule {
    id     = "mpu"
    status = "Enabled"
    filter {}
    abort_incomplete_multipart_upload {
      days_after_initiation = 7
    }
  }
}
resource "aws_s3_bucket_server_side_encryption_configuration" "b" {
  bucket = aws_s3_bucket.b.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm     = "aws:kms"
      kms_master_key_id = aws_kms_key.k.arn
    }
  }
}
"#,
    );
    let b = get(&folds, "aws_s3_bucket.b");
    assert_eq!(b.ty, "infra.bucket");
    assert_eq!(props(b), json!({ "lifecycle": [{ "prefix": "tmp/", "expire-after-days": 7 }], "versioning": true }));
    assert_eq!(b.satellites.len(), 3);
    assert!(b.audit.is_empty() && b.unread.is_empty());
    assert_eq!(props(get(&folds, "aws_kms_key.k")), json!({ "deletion-window-days": 7, "description": "data" }));
    // The encryption satellite joins the bucket and the key.
    assert!(edges.contains(&("aws_s3_bucket.b".into(), "aws_kms_key.k".into(), Some(Link::Encrypt), None)));
    assert!(folds.iter().any(|(a, f)| a == "aws_s3_bucket_versioning.b" && f.is_none()));
}

#[test]
fn expressions_derive_nothing() {
    let (folds, _, _) = fold_all(
        r#"
resource "aws_s3_bucket" "b" {}
resource "aws_s3_bucket_lifecycle_configuration" "b" {
  bucket = aws_s3_bucket.b.id
  rule {
    status = "Enabled"
    expiration {
      days = var.days
    }
  }
}
resource "aws_sqs_queue" "q" {
  fifo_queue                 = var.fifo
  visibility_timeout_seconds = 60
  redrive_policy             = var.dlq ? jsonencode({ maxReceiveCount = 5 }) : null
}
"#,
    );
    let b = get(&folds, "aws_s3_bucket.b");
    assert_eq!(props(b), json!({ "versioning": false }));
    assert_eq!(b.unread, ["lifecycle"]);
    assert_eq!(b.audit[0].0, "infra.audit.versioning");
    let q = get(&folds, "aws_sqs_queue.q");
    assert_eq!(props(q), json!({ "lock-timeout-s": 60 }));
    assert_eq!(q.unread, ["dead-letter", "max-deliveries", "ordered"]);
}

#[test]
fn aws_modules_and_out_of_range_values() {
    let (folds, edges, parents) = fold_all(
        r#"
module "net" {
  source             = "terraform-aws-modules/vpc/aws"
  cidr               = "10.20.0.0/16"
  azs                = ["a", "b", "c"]
  enable_nat_gateway = true
  single_nat_gateway = true
}
module "db" {
  source               = "terraform-aws-modules/rds/aws"
  engine               = "postgres"
  engine_version       = "17"
  instance_class       = "db.t4g.small"
  allocated_storage    = 5
  db_subnet_group_name = module.net.database_subnet_group_name
  publicly_accessible  = true
}
module "mysql" {
  source = "terraform-aws-modules/rds/aws"
  engine = "mysql"
}
module "fn" {
  source         = "terraform-aws-modules/lambda/aws"
  runtime        = "nodejs22.x"
  vpc_subnet_ids = module.net.private_subnets
  policy_statements = {
    db = {
      actions   = ["rds-db:connect"]
      resources = [module.db.db_instance_arn]
    }
  }
  event_source_mapping = {
    q = { event_source_arn = module.q.queue_arn }
  }
}
module "q" {
  source      = "terraform-aws-modules/sqs/aws"
  create_dlq  = true
  redrive_policy = {
    maxReceiveCount = 3
  }
}
"#,
    );
    assert_eq!(
        props(get(&folds, "module.net")),
        json!({ "cidr": "10.20.0.0/16", "flow-logs": false, "nat": "single", "zones": 3 })
    );
    let db = get(&folds, "module.db");
    // 5 GB is below the kit's minimum: not derived.
    assert_eq!(
        props(db),
        json!({ "aws": { "instance-class": "db.t4g.small" }, "deletion-protection": false, "engine": "postgres", "engine-version": "17", "high-availability": false })
    );
    assert_eq!(db.unread, ["storage-gb"]);
    assert_eq!(db.audit[0].0, "infra.audit.public");
    assert!(folds.iter().any(|(a, f)| a == "module.mysql" && f.is_none()));
    let q = get(&folds, "module.q");
    assert_eq!(props(q), json!({ "dead-letter": true, "max-deliveries": 3, "ordered": false }));
    assert_eq!(get(&folds, "module.fn").props["memory-mb"], json!(128));
    // Placement nests; an event source mapping triggers; IAM grants with access.
    assert_eq!(parents[1], Some("module.net".into()));
    assert_eq!(parents[3], Some("module.net".into()));
    assert!(edges.contains(&("module.q".into(), "module.fn".into(), Some(Link::Trigger), None)));
    assert!(edges.contains(&("module.fn".into(), "module.db".into(), Some(Link::Grant), Some("read-write"))));
}

#[test]
fn aws_grants_through_a_role() {
    let (_, edges, parents) = fold_all(
        r#"
resource "aws_vpc" "v" {
  cidr_block = "10.0.0.0/16"
}
resource "aws_subnet" "a" {
  vpc_id                  = aws_vpc.v.id
  map_public_ip_on_launch = true
}
resource "aws_subnet" "b" {
  vpc_id = aws_vpc.v.id
}
resource "aws_s3_bucket" "b" {}
resource "aws_sns_topic" "t" {}
resource "aws_iam_role" "r" {}
resource "aws_iam_role_policy" "p" {
  role = aws_iam_role.r.id
  policy = jsonencode({
    Statement = [
      { Effect = "Allow", Action = ["s3:GetObject"], Resource = "${aws_s3_bucket.b.arn}/*" },
      { Effect = "Allow", Action = "sns:Publish", Resource = aws_sns_topic.t.arn },
    ]
  })
}
resource "aws_lambda_function" "f" {
  role = aws_iam_role.r.arn
  vpc_config {
    subnet_ids = [aws_subnet.a.id, aws_subnet.b.id]
  }
}
"#,
    );
    assert!(edges.contains(&(
        "aws_lambda_function.f".into(),
        "aws_s3_bucket.b".into(),
        Some(Link::Grant),
        Some("read")
    )));
    assert!(edges.contains(&(
        "aws_lambda_function.f".into(),
        "aws_sns_topic.t".into(),
        Some(Link::Grant),
        Some("write")
    )));
    // Two subnets of one network: the function sits in the network.
    assert_eq!(parents[1], Some("aws_vpc.v".into()));
    assert_eq!(parents[7], Some("aws_vpc.v".into()));
}

#[test]
fn azure_resources_and_modules() {
    let (folds, edges, parents) = fold_all(
        r#"
resource "azurerm_virtual_network" "n" {
  address_space = ["10.20.0.0/16"]
}
resource "azurerm_subnet" "s" {
  virtual_network_name = azurerm_virtual_network.n.name
}
resource "azurerm_storage_account" "st" {
  access_tier = "Cool"
  blob_properties {
    versioning_enabled = true
  }
}
resource "azurerm_storage_management_policy" "p" {
  storage_account_id = azurerm_storage_account.st.id
  rule {
    name    = "tmp"
    enabled = true
    filters {
      prefix_match = ["data/tmp/"]
      blob_types   = ["blockBlob"]
    }
    actions {
      base_blob {
        delete_after_days_since_modification_greater_than = 30
      }
    }
  }
}
resource "azurerm_linux_function_app" "f" {
  virtual_network_subnet_id = azurerm_subnet.s.id
  site_config {
    application_stack {
      python_version = "3.12"
    }
  }
}
resource "azurerm_role_assignment" "r" {
  scope                = azurerm_storage_account.st.id
  role_definition_name = "Storage Blob Data Reader"
  principal_id         = azurerm_linux_function_app.f.identity[0].principal_id
}
module "bus" {
  source = "Azure/avm-res-servicebus-namespace/azurerm"
  queues = {
    jobs = {
      requires_session   = true
      default_message_ttl = "P4D"
      lock_duration      = "PT2M"
      max_delivery_count = 3
    }
  }
}
module "both" {
  source = "Azure/avm-res-servicebus-namespace/azurerm"
  queues = { a = {} }
  topics = { b = {} }
}
module "pg" {
  source             = "Azure/avm-res-dbforpostgresql-flexibleserver/azurerm"
  server_version     = "17"
  storage_mb         = 32768
  high_availability  = null
  databases          = { d = { name = "reports" } }
}
"#,
    );
    assert_eq!(props(get(&folds, "azurerm_virtual_network.n")), json!({ "cidr": "10.20.0.0/16" }));
    assert_eq!(
        props(get(&folds, "azurerm_storage_account.st")),
        json!({ "azure": { "access-tier": "Cool" }, "lifecycle": [{ "prefix": "tmp/", "expire-after-days": 30 }], "versioning": true })
    );
    assert_eq!(get(&folds, "azurerm_linux_function_app.f").props["runtime"], json!("python3.12"));
    assert_eq!(
        props(get(&folds, "module.bus")),
        json!({ "dead-letter": false, "lock-timeout-s": 120, "max-deliveries": 3, "ordered": true, "retention-s": 345600 })
    );
    assert!(folds.iter().any(|(a, f)| a == "module.both" && f.is_none()));
    assert_eq!(
        props(get(&folds, "module.pg")),
        json!({ "database": "reports", "deletion-protection": false, "engine": "postgres", "engine-version": "17", "high-availability": false, "storage-gb": 32 })
    );
    assert_eq!(parents[1], Some("azurerm_virtual_network.n".into()));
    assert_eq!(parents[4], Some("azurerm_subnet.s".into()));
    assert!(edges.contains(&(
        "azurerm_linux_function_app.f".into(),
        "azurerm_storage_account.st".into(),
        Some(Link::Grant),
        Some("read")
    )));
}

#[test]
fn gcp_resources_and_modules() {
    let (folds, edges, _) = fold_all(
        r#"
module "sa" {
  source = "terraform-google-modules/service-accounts/google"
}
resource "google_cloudfunctions2_function" "f" {
  build_config {
    runtime     = "python312"
    entry_point = "handle"
  }
  service_config {
    available_memory      = "1Gi"
    timeout_seconds       = 30
    service_account_email = module.sa.service_accounts_map["f"].email
  }
}
module "jobs" {
  source = "terraform-google-modules/pubsub/google"
  topic  = "jobs"
  push_subscriptions = [{
    name                    = "jobs"
    push_endpoint           = google_cloudfunctions2_function.f.service_config[0].uri
    ack_deadline_seconds    = 120
    enable_message_ordering = true
    dead_letter_topic       = module.dl.id
    max_delivery_attempts   = 5
  }]
}
module "alerts" {
  source = "terraform-google-modules/pubsub/google"
  topic  = "alerts"
  push_subscriptions = [{ name = "a", enable_message_ordering = true }, { name = "b", enable_message_ordering = true }]
}
resource "google_pubsub_topic_iam_member" "pub" {
  topic  = module.alerts.id
  role   = "roles/pubsub.publisher"
  member = module.sa.service_accounts_map["f"].member
}
resource "google_pubsub_topic_iam_member" "other" {
  topic  = module.jobs.id
  role   = "roles/pubsub.publisher"
  member = module.sa.service_accounts_map["g"].member
}
resource "google_firestore_database" "t" {
  point_in_time_recovery_enablement = "POINT_IN_TIME_RECOVERY_ENABLED"
}
resource "google_firestore_field" "ttl" {
  database = google_firestore_database.t.name
  field    = "expires_at"
  ttl_config {}
}
resource "google_sql_database_instance" "db" {
  database_version = "POSTGRES_16"
  settings {
    tier              = "db-custom-1-3840"
    availability_type = "REGIONAL"
  }
}
"#,
    );
    assert_eq!(
        props(get(&folds, "google_cloudfunctions2_function.f")),
        json!({ "handler": "handle", "memory-mb": 1024, "runtime": "python3.12", "timeout-s": 30 })
    );
    assert_eq!(
        props(get(&folds, "module.jobs")),
        json!({ "dead-letter": true, "lock-timeout-s": 120, "max-deliveries": 5, "ordered": true })
    );
    assert_eq!(get(&folds, "module.jobs").ty, "infra.queue");
    assert_eq!(props(get(&folds, "module.alerts")), json!({ "ordered": true }));
    assert_eq!(
        props(get(&folds, "google_firestore_database.t")),
        json!({ "deletion-protection": false, "point-in-time-recovery": true, "ttl-attribute": "expires_at" })
    );
    assert_eq!(
        props(get(&folds, "google_sql_database_instance.db")),
        json!({ "deletion-protection": false, "engine": "postgres", "engine-version": "16", "gcp": { "tier": "db-custom-1-3840" }, "high-availability": true })
    );
    // The push subscription triggers the function; the binding for its own
    // service account grants it, the one for another account does not.
    assert!(edges.contains(&(
        "module.jobs".into(),
        "google_cloudfunctions2_function.f".into(),
        Some(Link::Trigger),
        None
    )));
    assert!(edges.contains(&(
        "google_cloudfunctions2_function.f".into(),
        "module.alerts".into(),
        Some(Link::Grant),
        Some("write")
    )));
    assert!(!edges.iter().any(|(a, b, l, _)| a.ends_with(".f") && b == "module.jobs" && *l == Some(Link::Grant)));
}

#[test]
fn values_read_as_written() {
    assert_eq!(iso_seconds(&json!("PT345600S")), Some(345600));
    assert_eq!(iso_seconds(&json!("P14D")), Some(1209600));
    assert_eq!(iso_seconds(&json!("PT1M30S")), Some(90));
    assert_eq!(iso_seconds(&json!("P1Y")), None);
    assert_eq!(classify(&["sqs:ReceiveMessage".into(), "sqs:DeleteMessage".into()]), (true, false));
    assert_eq!(
        classify(&["Microsoft.Storage/storageAccounts/blobServices/containers/blobs/add/action".into()]),
        (false, true)
    );
    assert_eq!(classify(&["s3:*".into()]), (true, true));
    let t = tree(&json!({ "a": "{ x = 1, y = var.z }", "b": "\"lit\"", "c": "\"${var.x}\"" }));
    assert_eq!(at(&t, "a.x"), Arg::Lit(&json!(1)));
    assert_eq!(at(&t, "a.y"), Arg::Expr);
    assert_eq!(at(&t, "a"), Arg::Expr);
    assert_eq!(at(&t, "b"), Arg::Lit(&json!("lit")));
    assert_eq!(at(&t, "c"), Arg::Expr);
    assert_eq!(at(&t, "d"), Arg::Absent);
}
