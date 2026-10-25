//! The import corpus (`tests/corpus/`): every entry imports with L1 equal,
//! compiles, and imports again to the same diagram, with the semantic
//! coverage its README states. rhizo's own output for each cloud comes back
//! as the neutral types it was compiled from (the full circle). Needs `tofu`
//! and the OpenTofu registry (about 3 minutes), so the tests that plan are
//! opt-in: `cargo test -p rhizo-iac -- --ignored`. Without `tofu` they are
//! skipped.

use std::path::{Path, PathBuf};
use std::process::Command;

use rhizo_core::Document;
use rhizo_iac::{Error, Options, import_tofu};
use rhizo_kit::infra::{self as k, NEUTRAL};
use serde_json::Value;

fn corpus() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut dirs: Vec<PathBuf> =
        std::fs::read_dir(root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs
}

/// A fresh folder for this test binary.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn options(verify: bool) -> Options {
    // One provider download for the whole corpus.
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("tofu-plugins");
    std::fs::create_dir_all(&cache).unwrap();
    Options { verify, plugin_cache: Some(cache), ..Options::default() }
}

fn tofu_installed() -> bool {
    Command::new("tofu").arg("version").output().is_ok_and(|o| o.status.success())
}

/// Writes the root module the diagram compiles to into `out/<environment>`.
fn compile_into(kdl: &str, out: &Path) -> PathBuf {
    let (doc, _) = rhizo_lang::load(kdl, rhizo_lang::Syntax::Kdl).into_result().unwrap();
    let bundle = rhizo_compile::compile(&doc, &rhizo_kit::Kits::builtin(), "corpus.rhizo", kdl).unwrap();
    write_infra(&bundle, out)
}

/// Writes a bundle's root module into `out/<environment>` and returns it.
fn write_infra(bundle: &rhizo_compile::Bundle, out: &Path) -> PathBuf {
    let mut root = None;
    for (path, f) in &bundle.files {
        let Some(rel) = path.strip_prefix("infra/") else { continue };
        let target = out.join(rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, &f.contents).unwrap();
        root = Some(target.parent().unwrap().to_path_buf());
    }
    root.expect("the diagram compiles to a root module")
}

#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn corpus_round_trips() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let mut failed = Vec::new();
    for dir in corpus() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let first = match import_tofu(&dir, &options(true)) {
            Ok(i) => i,
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                continue;
            }
        };
        assert!(first.report.contains("L1 equal"), "{}", first.report);
        let coverage = first.report.lines().find(|l| l.starts_with("semantic coverage: ")).unwrap_or_default();
        let want = expected_coverage(&name);
        if coverage != want {
            failed.push(format!("{name}: {coverage:?}, the corpus README says {want:?}"));
            continue;
        }
        let compiled = compile_into(&first.kdl, &scratch(&format!("again-{name}")));
        // Variable values are inputs, not part of the diagram.
        for f in std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()) {
            if f.extension().is_some_and(|x| x == "tfvars") {
                std::fs::copy(&f, compiled.join(f.file_name().unwrap())).unwrap();
            }
        }
        let second = import_tofu(&compiled, &options(false)).unwrap_or_else(|e| panic!("{name}, again: {e}"));
        if second.kdl != first.kdl {
            failed.push(format!("{name}: importing the compiled code gives another diagram"));
            continue;
        }
        println!("{name}: imported, compiled, L1 equal, idempotent, {coverage}");
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The coverage line the corpus README's table states for an entry:
/// `| \`name\` | folded | satellites | resources and module calls | coverage |`.
fn expected_coverage(name: &str) -> String {
    let readme = include_str!("corpus/README.md");
    let row = readme
        .lines()
        .find(|l| l.starts_with(&format!("| `{name}` |")) && l.ends_with("% |"))
        .unwrap_or_else(|| panic!("the corpus README has no coverage row for {name}"));
    let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
    format!(
        "semantic coverage: {} ({} folded and {} satellites of {} resources and module calls)",
        cells[4], cells[1], cells[2], cells[3]
    )
}

/// The full circle: every node rhizo compiled from `source` comes back from
/// its imported output as the same neutral type (an `aws.*` type counts as
/// its neutral one), with the props the source wrote as literals. Returns
/// what did not come back, as `<id>: …` or `<id>.<prop>: …`, and prints the
/// recovered share.
fn full_circle(cloud: &str, source: &Document, imported: &str) -> Vec<String> {
    let (doc, _) = rhizo_lang::load(imported, rhizo_lang::Syntax::Kdl).into_result().unwrap();
    let mut gaps = Vec::new();
    let (mut nodes, mut typed, mut props, mut same) = (0, 0, 0, 0);
    for n in source.nodes.values().filter(|n| k::environment(source, n).is_some()) {
        let written = k::written_ty(n);
        let Some(want) = NEUTRAL.iter().find(|(neutral, aws)| *neutral == written || *aws == written).map(|(x, _)| *x)
        else {
            continue;
        };
        nodes += 1;
        // Emitted at the address the node's id names: `orders-db` → `orders_db`.
        let name = n.id.as_str().replace('-', "_");
        let named: Vec<_> =
            doc.nodes.values().filter(|m| m.props.get("name") == Some(&Value::from(name.clone()))).collect();
        let Some(m) = named.iter().find(|m| k::written_ty(m) == want) else {
            let found: Vec<&str> = named.iter().map(|m| k::written_ty(m)).collect();
            gaps.push(format!("{}: no {want} named {name} (found {found:?})", n.id));
            continue;
        };
        typed += 1;
        for (key, v) in n.props.iter().filter(|(key, _)| *key != "sensitive") {
            let pairs: Vec<(String, Option<&Value>, Option<&Value>)> = match v {
                // A cloud block: each setting.
                Value::Object(block) if k::CLOUDS.contains(&key.as_str()) => block
                    .iter()
                    .map(|(s, v)| (format!("{key}.{s}"), Some(v), m.props.get(key).and_then(|b| b.get(s))))
                    .collect(),
                // An aws.* prop by its AWS name: the neutral node's `aws` block or renamed prop.
                _ if written.starts_with("aws.") => vec![(key.clone(), Some(v), k::prop(m, key))],
                _ => vec![(key.clone(), Some(v), m.props.get(key))],
            };
            for (what, want, got) in pairs {
                props += 1;
                if want == got {
                    same += 1;
                } else {
                    gaps.push(format!(
                        "{}.{what}: {} in the source, {} imported",
                        n.id,
                        want.map_or("-".into(), |v| v.to_string()),
                        got.map_or("-".into(), |v| v.to_string())
                    ));
                }
            }
        }
    }
    println!(
        "{cloud}: full circle recovered {typed} of {nodes} nodes as their neutral type and {same} of {props} literal props"
    );
    for g in &gaps {
        println!("  gap: {g}");
    }
    gaps
}

/// The gaps `full_circle` must find: only these, each with its reason.
fn assert_gaps(found: &[String], expected: &[(&str, &str)]) {
    let ids: Vec<&str> = found.iter().map(|g| g.split(':').next().unwrap()).collect();
    let want: Vec<&str> = expected.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, want, "{found:#?}");
}

/// rhizo's own OpenTofu output round-trips. Its pinned modules read data
/// sources (`aws_caller_identity`, `aws_availability_zones`), which only
/// mocked providers can plan offline.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn rhizo_output_round_trips() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop");
    let project = rhizo_project::Project::open(&shop).unwrap();
    let bundle = project.compile(&rhizo_kit::Kits::builtin()).unwrap_or_else(|f| panic!("{} findings", f.len()));
    let root = write_infra(&bundle, &scratch("shop"));
    std::fs::write(
        root.join("prod.auto.tfvars"),
        "aws_account_id = \"123456789012\"\nstate_kms_key_id = \"arn:aws:kms:eu-central-1:123456789012:key/00000000-0000-0000-0000-000000000000\"\nbudget_alert_emails = [\"ops@example.com\"]\norders_fn_package = \"orders.zip\"\nemail_fn_package = \"email.zip\"\n",
    )
    .unwrap();
    let imported = import_tofu(&root, &options(true)).unwrap_or_else(|e| panic!("{e}"));
    assert!(imported.report.contains("L1 equal"), "{}", imported.report);
    let gaps = full_circle("aws", &project.merged, &imported.kdl);
    assert_gaps(&gaps, AWS_GAPS);
}

/// What rhizo's AWS output does not give back, and why.
const AWS_GAPS: &[(&str, &str)] = &[
    ("app-subnets", "subnet tiers are inputs of the vpc module, not resources of their own"),
    ("data-subnets", "subnet tiers are inputs of the vpc module, not resources of their own"),
];

/// rhizo's Azure output round-trips: Azure Verified Modules, most built on
/// azapi, plan offline once each azapi resource gets an ID of its ARM type.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn azure_output_round_trips() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let src = include_str!("../../../examples/playground/12-azure-infra.rhizo");
    let root = compile_into(src, &scratch("azure"));
    std::fs::write(
        root.join("staging.auto.tfvars"),
        "subscription_id = \"00000000-0000-0000-0000-000000000000\"\ntenant_id = \"00000000-0000-0000-0000-000000000000\"\nbudget_alert_emails = [\"ops@example.com\"]\nreports_db_admin_object_id = \"00000000-0000-0000-0000-000000000000\"\nreports_db_admin_name = \"db-admins\"\napi_publisher_email = \"api@example.com\"\n",
    )
    .unwrap();
    let imported = import_tofu(&root, &options(true)).unwrap_or_else(|e| panic!("{e}"));
    assert!(imported.report.contains("L1 equal"), "{}", imported.report);
    let (source, _) = rhizo_lang::load(src, rhizo_lang::Syntax::Kdl).into_result().unwrap();
    assert_gaps(&full_circle("azure", &source, &imported.kdl), AZURE_GAPS);
}

/// What rhizo's Azure output does not give back, and why.
const AZURE_GAPS: &[(&str, &str)] = &[
    ("net.nat", "the NAT gateway is a module of its own (avm-res-network-natgateway)"),
    ("net.flow-logs", "flow logs are not compiled on azure, so the code does not say"),
    ("private", "subnet tiers are inputs of the virtual network module"),
    ("worker.timeout-s", "the timeout is functionTimeout in the app's host.json, not infrastructure"),
    ("worker.environment", "the app settings add what its edges need, by reference"),
    ("data-tier", "subnet tiers are inputs of the virtual network module"),
    ("jobs", "queues and topics share one Service Bus namespace module, which is no one type"),
    ("alerts", "queues and topics share one Service Bus namespace module, which is no one type"),
    ("hook.memory-mb", "Flex Consumption sizes come back as azure.instance-memory-mb (2048)"),
];

/// rhizo's Google Cloud output round-trips: terraform-google-modules plan
/// offline once compute self links, zones and an API Gateway API's managed
/// service have valid mocked shapes.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn gcp_output_round_trips() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let src = include_str!("../../../examples/playground/13-gcp-infra.rhizo");
    let root = compile_into(src, &scratch("gcp"));
    std::fs::write(
        root.join("staging.auto.tfvars"),
        "project_id = \"shop-staging-123\"\nstate_kms_key = \"projects/shop-staging-123/locations/europe-west3/keyRings/tofu/cryptoKeys/state\"\nbilling_account_id = \"012345-6789AB-CDEF01\"\nbudget_alert_emails = [\"ops@example.com\"]\nworker_package = \"worker.zip\"\nhook_package = \"hook.zip\"\nreports_db_admin_group = \"db-admins@example.com\"\n",
    )
    .unwrap();
    let imported = import_tofu(&root, &options(true)).unwrap_or_else(|e| panic!("{e}"));
    assert!(imported.report.contains("L1 equal"), "{}", imported.report);
    let (source, _) = rhizo_lang::load(src, rhizo_lang::Syntax::Kdl).into_result().unwrap();
    assert_gaps(&full_circle("gcp", &source, &imported.kdl), GCP_GAPS);
}

/// What rhizo's Google Cloud output does not give back, and why.
const GCP_GAPS: &[(&str, &str)] = &[
    ("net.nat", "Cloud NAT is a module of its own (cloud-router)"),
    ("private", "subnets are inputs of the network module"),
    ("worker.handler", "the entry point is the handler's last part (main.handle → handle)"),
    ("worker.environment", "the environment adds what its edges need, by reference"),
    ("data-tier", "the database tier is the private services access range, a module of its own"),
    ("carts.partition-key", "Firestore finds documents by id: the code has no partition key"),
    ("key.description", "Cloud KMS keys have no description"),
    ("api.cors-origins", "API Gateway has no CORS policy; the functions answer preflights"),
];

/// Code that reads a file the diagram cannot carry fails L1, naming it.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn files_the_code_reads_fail_l1() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let dir = scratch("reads-a-file");
    std::fs::write(dir.join("policy.json"), "{\"Version\": \"2012-10-17\", \"Statement\": []}").unwrap();
    std::fs::write(
        dir.join("main.tf"),
        "terraform {\n  required_providers {\n    aws = {\n      source  = \"hashicorp/aws\"\n      version = \"= 6.69.1\"\n    }\n  }\n}\n\nresource \"aws_iam_policy\" \"p\" {\n  name   = \"p\"\n  policy = file(\"${path.module}/policy.json\")\n}\n",
    )
    .unwrap();
    match import_tofu(&dir, &options(true)) {
        Err(Error::Tofu { step, output }) => {
            assert!(step.contains("compiled bundle"), "{step}");
            assert!(output.contains("policy.json"), "{output}");
        }
        other => panic!("expected L1 to fail, got {other:?}"),
    }
}

/// What would be lost is refused before OpenTofu runs, naming it.
#[test]
fn refuses_what_it_would_lose() {
    let cases = [
        (
            "override",
            "main.tf",
            "resource \"aws_sqs_queue\" \"q\" {}\n",
            Some(("override.tf", "resource \"aws_sqs_queue\" \"q\" {\n  name = \"x\"\n}\n")),
            "override.tf: override files",
        ),
        ("json", "main.tf", "", Some(("extra.tf.json", "{}")), "extra.tf.json: JSON configuration"),
        (
            "local-module",
            "main.tf",
            "module \"net\" {\n  source = \"./modules/net\"\n}\n",
            None,
            "module \"net\": local module source `./modules/net`",
        ),
        ("attribute", "main.tf", "name = \"x\"\n", None, "main.tf: argument `name` outside any block"),
        ("syntax", "main.tf", "resource \"aws_sqs_queue\" \"q\" {\n", None, "main.tf:"),
    ];
    for (name, file, text, extra, expected) in cases {
        let dir = scratch(&format!("refuse-{name}"));
        std::fs::write(dir.join(file), text).unwrap();
        if let Some((f, t)) = extra {
            std::fs::write(dir.join(f), t).unwrap();
        }
        match import_tofu(&dir, &Options { tofu: "/nonexistent/tofu".into(), ..options(true) }) {
            Err(Error::Refused(items)) => {
                assert!(items.iter().any(|i| i.contains(expected)), "{name}: {items:?}")
            }
            other => panic!("{name}: expected a refusal, got {other:?}"),
        }
    }
}

/// A small imported environment for the adopt refusals: nothing here
/// reaches OpenTofu.
const IMPORTED: &str = r#"rhizo version=2
use infra version="0.1"
frame "prod" id=prod type=infra.environment {
    props cloud="aws" {
        origin dir="prod"
        blocks {
            - type="output" file="main.tf" {
                (array)labels "v"
                body value="aws_s3_bucket_versioning.logs.id"
            }
        }
    }
    node "logs" id=aws_s3_bucket.logs type=infra.bucket {
        props tf-type="aws_s3_bucket" name="logs" versioning=#true {
            body bucket="\"logs\""
            origin address="aws_s3_bucket.logs" shape="resource" file="main.tf"
        }
    }
    node "logs" id=aws_s3_bucket_versioning.logs type=infra.resource {
        props tf-type="aws_s3_bucket_versioning" name="logs" {
            body bucket="aws_s3_bucket.logs.id" {
                versioning_configuration {
                    - status="\"Enabled\""
                }
            }
            origin address="aws_s3_bucket_versioning.logs" shape="resource" file="main.tf"
        }
    }
    node "many" id=aws_sqs_queue.many type=infra.queue {
        props tf-type="aws_sqs_queue" name="many" ordered=#false dead-letter=#false {
            body count="2"
            origin address="aws_sqs_queue.many" shape="resource" file="main.tf"
        }
    }
    node "f" id=aws_lambda_function.f type=infra.function {
        props tf-type="aws_lambda_function" name="f" memory-mb=128 timeout-s=3 {
            aws architecture="x86_64"
            body role="\"arn:aws:iam::123456789012:role/f\""
            origin address="aws_lambda_function.f" shape="resource" file="main.tf"
        }
    }
    node "old" id=module.old type=infra.queue {
        props source="terraform-aws-modules/sqs/aws" version="4.0.0" name="old" ordered=#false dead-letter=#false {
            origin address="module.old" shape="module" file="main.tf"
        }
    }
    edge aws_s3_bucket_versioning.logs aws_s3_bucket.logs kind=dependency
}
"#;

/// Raw Google Cloud code for the refusals: nothing here reaches OpenTofu.
const IMPORTED_GCP: &str = r#"rhizo version=2
use infra version="0.1"
frame "g" id=g type=infra.environment {
    props cloud="gcp" {
        origin dir="g"
        blocks {
            - type="provider" file="main.tf" {
                (array)labels "google"
                body project="\"acme\""
            }
        }
        lock-file "provider \"registry.opentofu.org/hashicorp/google\" {\n  version = \"7.46.1\"\n}\n"
    }
    node "acl" id=google_storage_bucket.acl type=infra.bucket {
        props tf-type="google_storage_bucket" name="acl" versioning=#false {
            body name="\"acl\"" location="\"EU\""
            origin address="google_storage_bucket.acl" shape="resource" file="main.tf"
        }
    }
    node "open" id=google_storage_bucket.open type=infra.bucket {
        props tf-type="google_storage_bucket" name="open" versioning=#false {
            body name="\"open\"" location="\"EU\"" uniform_bucket_level_access="true"
            origin address="google_storage_bucket.open" shape="resource" file="main.tf"
        }
    }
    node "everyone" id=google_storage_bucket_iam_member.everyone type=infra.resource {
        props tf-type="google_storage_bucket_iam_member" name="everyone" {
            body bucket="google_storage_bucket.open.name" role="\"roles/storage.objectViewer\"" member="\"allUsers\""
            origin address="google_storage_bucket_iam_member.everyone" shape="resource" file="main.tf"
        }
    }
    node "sub" id=google_pubsub_subscription.sub type=infra.queue {
        props tf-type="google_pubsub_subscription" name="sub" dead-letter=#false ordered=#false {
            body name="\"sub\"" topic="\"projects/acme/topics/t\""
            origin address="google_pubsub_subscription.sub" shape="resource" file="main.tf"
        }
    }
    node "t" id=google_pubsub_topic.t type=infra.topic {
        props tf-type="google_pubsub_topic" name="t" {
            body name="\"t\""
            origin address="google_pubsub_topic.t" shape="resource" file="main.tf"
        }
    }
    edge google_storage_bucket_iam_member.everyone google_storage_bucket.open kind=dependency
}
"#;

/// Raw Azure code for the refusals: nothing here reaches OpenTofu.
const IMPORTED_AZURE: &str = r#"rhizo version=2
use infra version="0.1"
frame "a" id=a type=infra.environment {
    props cloud="azure" {
        origin dir="a"
    }
    node "v" id=azurerm_key_vault.v type=infra.resource {
        props tf-type="azurerm_key_vault" name="v" {
            body name="\"v\"" location="\"westeurope\"" resource_group_name="\"rg\"" tenant_id="\"t\"" sku_name="\"premium\"" rbac_authorization_enabled="true"
            origin address="azurerm_key_vault.v" shape="resource" file="main.tf"
        }
    }
    node "k" id=azurerm_key_vault_key.k type=infra.key {
        props tf-type="azurerm_key_vault_key" name="k" {
            body name="\"k\"" key_vault_id="azurerm_key_vault.v.id" key_type="\"RSA\""
            origin address="azurerm_key_vault_key.k" shape="resource" file="main.tf"
        }
    }
    node "p" id=azurerm_key_vault_access_policy.p type=infra.resource {
        props tf-type="azurerm_key_vault_access_policy" name="p" {
            body key_vault_id="azurerm_key_vault.v.id" tenant_id="\"t\"" object_id="\"o\""
            origin address="azurerm_key_vault_access_policy.p" shape="resource" file="main.tf"
        }
    }
    node "st" id=azurerm_storage_account.st type=infra.bucket {
        props tf-type="azurerm_storage_account" name="st" {
            body name="\"st\""
            origin address="azurerm_storage_account.st" shape="resource" file="main.tf"
        }
    }
    edge azurerm_key_vault_key.k azurerm_key_vault.v kind=dependency
    edge azurerm_key_vault_access_policy.p azurerm_key_vault.v kind=dependency
}
"#;

/// Raw Azure and Google Cloud code: refused by name where rhizo's code would
/// lose a grant unseen, move across types OpenTofu cannot plan, or run on
/// another provider version.
#[test]
fn raw_azure_and_gcp_refuse_by_name() {
    let opts = Options { tofu: "/nonexistent/tofu".into(), ..options(true) };
    let refused = |kdl: &str, id: &str| match rhizo_iac::adopt(kdl, &[id.to_string()], false, &opts, &[]) {
        Err(Error::NotAdopted(why)) => why.join("\n"),
        // Past the checks: L1-adopt needs OpenTofu.
        Err(Error::Io(e)) if e.contains("/nonexistent/tofu") => "would run L1-adopt".into(),
        other => panic!("{id}: expected a refusal, got {other:?}"),
    };
    let g = IMPORTED_GCP;
    assert!(refused(g, "google_storage_bucket.acl").contains("ends every ACL grant"));
    assert!(
        refused(g, "google_storage_bucket.open")
            .contains("ends the public grant of `google_storage_bucket_iam_member.everyone`")
    );
    assert!(refused(g, "google_pubsub_subscription.sub").contains("rhizo's code for a queue is a topic of its own"));
    assert_eq!(refused(g, "google_pubsub_topic.t"), "would run L1-adopt");
    let old = g.replace("version = \\\"7.46.1\\\"", "version = \\\"6.49.0\\\"");
    assert!(
        refused(&old, "google_pubsub_topic.t")
            .contains("rhizo's code is pinned to hashicorp/google 7.46.1, and this environment locks google 6.49.0")
    );
    let no_project = g.replace("body project=\"\\\"acme\\\"\"", "body region=\"\\\"europe-west3\\\"\"");
    assert!(
        refused(&no_project, "google_pubsub_topic.t")
            .contains("neither its code nor the environment's google provider configuration does")
    );
    let a = IMPORTED_AZURE;
    assert!(refused(a, "azurerm_storage_account.st").contains("a move across types"));
    assert!(refused(a, "azurerm_key_vault_key.k").contains("has access policy `azurerm_key_vault_access_policy.p`"));
    let no_rbac = a.replace(" rbac_authorization_enabled=\"true\"", "");
    assert!(refused(&no_rbac, "azurerm_key_vault_key.k").contains("authorizes with access policies"));
}

/// Adopting refuses what it cannot do faithfully, by name, and writes nothing.
#[test]
fn adopt_refuses_by_name() {
    let opts = Options { tofu: "/nonexistent/tofu".into(), ..options(true) };
    let refused = |ids: &[&str]| match rhizo_iac::adopt(
        IMPORTED,
        &ids.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        false,
        &opts,
        &[],
    ) {
        Err(Error::NotAdopted(why)) => why.join("\n"),
        other => panic!("{ids:?}: expected a refusal, got {other:?}"),
    };
    assert!(refused(&["nope"]).contains("no node `nope`"));
    assert!(
        refused(&["aws_s3_bucket_versioning.logs"]).contains("only nodes folded into a cloud-neutral infra.* type")
    );
    // Rewriting what refers to the bucket's satellite would need an output it has no name for.
    assert!(refused(&["aws_s3_bucket.logs"]).contains("output \"v\" reads `aws_s3_bucket_versioning.logs.id`"));
    assert!(refused(&["aws_sqs_queue.many"]).contains("cannot adopt `aws_sqs_queue.many` yet: it uses `count`"));
    assert!(
        refused(&["aws_lambda_function.f"])
            .contains("cannot adopt `aws_lambda_function.f` from shape resources (`aws_lambda_function`) yet")
    );
    assert!(
        refused(&["module.old"]).contains(
            "cannot adopt `module.old` from shape module terraform-aws-modules/sqs/aws@4.0.0 yet: rhizo's code for it is terraform-aws-modules/sqs/aws@5.2.2"
        )
    );
    // --all adopts nothing here, and says why for each.
    match rhizo_iac::adopt(&IMPORTED.replace("aws_s3_bucket_versioning.logs.id", "var.x"), &[], true, &opts, &[]) {
        // The bucket can be adopted: L1-adopt needs OpenTofu.
        Err(Error::Io(e)) => assert!(e.contains("/nonexistent/tofu"), "{e}"),
        other => panic!("{other:?}"),
    }
    let only_refused = IMPORTED
        .replace("aws_s3_bucket.logs type=infra.bucket", "aws_s3_bucket.logs type=infra.resource")
        .replace("name=\"logs\" versioning=#true", "name=\"logs\"");
    match rhizo_iac::adopt(&only_refused, &[], true, &opts, &[]) {
        Err(Error::NotAdopted(why)) => {
            assert_eq!(why[0], "nothing to adopt");
            assert_eq!(why.len(), 4, "{why:#?}");
        }
        other => panic!("{other:?}"),
    }
}

// --- Adopt: switching shapes ----------------------------------------------

/// Writes `vars` as a `.tfvars` file in `dir` and returns it.
fn var_file(dir: &Path, vars: &str) -> PathBuf {
    let f = dir.join("inputs.tfvars");
    std::fs::write(&f, vars).unwrap();
    f
}

/// Compiles an adopted diagram and runs `tofu init` and `tofu validate` on it.
fn validate(kdl: &str, out: &Path) {
    let root = compile_into(kdl, out);
    assert!(
        root.join(rhizo_compile::ADOPTED_FILE).exists(),
        "no {} in {}",
        rhizo_compile::ADOPTED_FILE,
        root.display()
    );
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("tofu-plugins");
    let run = |args: &[&str]| {
        Command::new("tofu").args(args).current_dir(&root).env("TF_PLUGIN_CACHE_DIR", &cache).output().unwrap()
    };
    let init = run(&["init", "-backend=false", "-input=false", "-no-color"]);
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));
    let v = run(&["validate", "-no-color"]);
    assert!(v.status.success(), "{}{}", String::from_utf8_lossy(&v.stderr), String::from_utf8_lossy(&v.stdout));
    let _ = std::fs::remove_dir_all(root.join(".terraform"));
}

/// The ids `rhizo adopt` adopted and named as not adopted, from its report.
fn adopted_ids(report: &str) -> (Vec<&str>, Vec<&str>) {
    let ids = |prefix: &str| -> Vec<&str> {
        report
            .lines()
            .filter_map(|l| l.strip_prefix(prefix))
            .map(|l| l.split([' ', '`']).find(|w| !w.is_empty()).unwrap())
            .collect()
    };
    (ids("  adopt "), ids("  not adopted: "))
}

/// The full circle, adopted: rhizo's own output, imported, then `adopt
/// --all`. Its code was rhizo's shape, so L1-adopt finds every instance
/// where it was and no attribute that changes. Then one prop is edited and
/// L1-adopt lists exactly that change.
fn adopt_full_circle(
    cloud: &str,
    root: &Path,
    vars: &str,
    adopted: &[&str],
    left: &[&str],
    edit: (&str, &str, Value),
    change: &str,
) {
    std::fs::write(root.join("inputs.auto.tfvars"), vars).unwrap();
    let imported = import_tofu(root, &options(false)).unwrap_or_else(|e| panic!("{e}"));
    let inputs = [var_file(&scratch(&format!("{cloud}-vars")), vars)];
    let a = rhizo_iac::adopt(&imported.kdl, &[], true, &options(true), &inputs).unwrap_or_else(|e| panic!("{e}"));
    println!("{cloud}:\n{}", a.report);
    assert_eq!(adopted_ids(&a.report), (adopted.to_vec(), left.to_vec()), "{}", a.report);
    assert!(a.report.contains("every original instance is accounted for: 0 moved"), "{}", a.report);
    assert!(a.report.contains("  attribute differences: none\n  creates: none\n"), "{}", a.report);
    validate(&a.kdl, &scratch(&format!("{cloud}-adopted")));
    // One prop changes: exactly that.
    let (id, key, value) = edit;
    let mut ed = rhizo_lang::edit::Editor::open(a.kdl.clone());
    ed.set_prop(&rhizo_core::NodeId::from(id), key, &value).unwrap();
    let report = rhizo_iac::verify_adopted(&imported.kdl, ed.source(), &options(true), &inputs)
        .unwrap_or_else(|e| panic!("{e}"));
    println!("{cloud}, {id} {key}={value}:\n{report}");
    let changes: Vec<&str> = report.lines().filter(|l| l.starts_with("  adopting changes ")).collect();
    assert_eq!(changes, [change], "{report}");
    assert!(report.contains("  creates: none\n"), "{report}");
}

#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn rhizo_output_adopts() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop");
    let project = rhizo_project::Project::open(&shop).unwrap();
    let bundle = project.compile(&rhizo_kit::Kits::builtin()).unwrap_or_else(|f| panic!("{} findings", f.len()));
    let root = write_infra(&bundle, &scratch("shop-adopt"));
    adopt_full_circle(
        "aws",
        &root,
        "aws_account_id = \"123456789012\"\nstate_kms_key_id = \"arn:aws:kms:eu-central-1:123456789012:key/00000000-0000-0000-0000-000000000000\"\nbudget_alert_emails = [\"ops@example.com\"]\norders_fn_package = \"orders.zip\"\nemail_fn_package = \"email.zip\"\n",
        &[
            "module.net",
            "module.orders_db",
            "module.orders_fn",
            "module.http_api",
            "module.order_events",
            "module.email_queue",
            "module.email_fn",
            "module.receipts",
            "module.data_key",
        ],
        &[],
        ("module.orders_fn", "memory-mb", Value::from(1024)),
        "  adopting changes module.orders_fn.aws_lambda_function.this[0]: memory_size: 512 → 1024",
    );
}

#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn azure_output_adopts() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let src = include_str!("../../../examples/playground/12-azure-infra.rhizo");
    let root = compile_into(src, &scratch("azure-adopt"));
    adopt_full_circle(
        "azure",
        &root,
        "subscription_id = \"00000000-0000-0000-0000-000000000000\"\ntenant_id = \"00000000-0000-0000-0000-000000000000\"\nbudget_alert_emails = [\"ops@example.com\"]\nreports_db_admin_object_id = \"00000000-0000-0000-0000-000000000000\"\nreports_db_admin_name = \"db-admins\"\napi_publisher_email = \"api@example.com\"\n",
        &[
            "module.net",
            "module.worker",
            "module.reports_db",
            "module.hook",
            "module.carts",
            "module.key",
            "module.docs",
            "module.api",
        ],
        // A function's host storage folds to a bucket, but is not rhizo's code for one.
        &["module.hook_storage", "module.worker_storage"],
        ("module.docs", "azure", serde_json::json!({ "access-tier": "Hot" })),
        "  adopting changes module.docs.azapi_resource.this: body.properties.accessTier: \"Cool\" → \"Hot\"",
    );
}

#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn gcp_output_adopts() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let src = include_str!("../../../examples/playground/13-gcp-infra.rhizo");
    let root = compile_into(src, &scratch("gcp-adopt"));
    adopt_full_circle(
        "gcp",
        &root,
        "project_id = \"shop-staging-123\"\nstate_kms_key = \"projects/shop-staging-123/locations/europe-west3/keyRings/tofu/cryptoKeys/state\"\nbilling_account_id = \"012345-6789AB-CDEF01\"\nbudget_alert_emails = [\"ops@example.com\"]\nworker_package = \"worker.zip\"\nhook_package = \"hook.zip\"\nreports_db_admin_group = \"db-admins@example.com\"\n",
        &[
            "module.net",
            "module.reports_db",
            "google_firestore_database.carts",
            "module.jobs",
            "module.key",
            "module.docs",
            "google_api_gateway_gateway.api",
        ],
        &[
            "module.alerts",
            "module.jobs_dead_letter",
            "google_cloudfunctions2_function.hook",
            "google_cloudfunctions2_function.worker",
            "module.function_sources",
        ],
        ("module.docs", "gcp", serde_json::json!({ "storage-class": "COLDLINE" })),
        "  adopting changes module.docs.google_storage_bucket.bucket: storage_class: \"NEARLINE\" → \"COLDLINE\"",
    );
}

/// A raw S3 bucket with its satellites becomes the pinned module: L1-adopt
/// accounts for each instance through `moved` and lists what changes.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn raw_bucket_adopts() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/s3-bucket");
    let imported = import_tofu(&dir, &options(false)).unwrap_or_else(|e| panic!("{e}"));
    let a = rhizo_iac::adopt(&imported.kdl, &["aws_s3_bucket.receipts".into()], false, &options(true), &[])
        .unwrap_or_else(|e| panic!("{e}"));
    println!("{}", a.report);
    let r = &a.report;
    assert!(r.contains("every original instance is accounted for: 6 moved, 1 at their own address"), "{r}");
    for ty in [
        "",
        "_versioning",
        "_public_access_block",
        "_ownership_controls",
        "_server_side_encryption_configuration",
        "_policy",
    ] {
        let line = format!("  moves aws_s3_bucket{ty}.receipts -> module.receipts.aws_s3_bucket{ty}.this[0]\n");
        assert!(r.contains(&line), "{line}{r}");
    }
    // The lifecycle rules expire after `var.tmp_expiry_days`: not a prop, so they stay verbatim.
    assert!(r.contains("satellites kept verbatim (rhizo's code does not manage them with these props): aws_s3_bucket_lifecycle_configuration.receipts"), "{r}");
    assert!(r.contains("kept from its code (names, places and tags of what exists): bucket_prefix, tags"), "{r}");
    // What rhizo's shape does differently, honestly listed.
    for change in [
        "module.receipts.aws_s3_bucket_policy.this[0] (from aws_s3_bucket_policy.receipts): policy:",
        "module.receipts.aws_s3_bucket_public_access_block.this[0] (from aws_s3_bucket_public_access_block.receipts): skip_destroy: null → true",
        "module.receipts.aws_s3_bucket_server_side_encryption_configuration.this[0] (from aws_s3_bucket_server_side_encryption_configuration.receipts): rule[0].bucket_key_enabled: true → false",
    ] {
        assert!(r.contains(&format!("  adopting changes {change}")), "{change}\n{r}");
    }
    assert!(r.contains("  creates: none\n"), "{r}");
    // What referred to the bucket refers to the module now.
    assert!(a.kdl.contains("value=\"module.receipts.s3_bucket_arn\""), "{}", a.kdl);
    assert!(a.kdl.contains("bucket=\"module.receipts.s3_bucket_id\""), "{}", a.kdl);
    assert!(!a.kdl.contains("id=aws_s3_bucket_policy.receipts"), "{}", a.kdl);
    validate(&a.kdl, &scratch("raw-bucket-adopted"));
}

/// Raw queues, topics, keys and tables become the pinned modules; two of
/// one name get distinct module names.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn raw_resources_adopt() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/adopt/raw-aws");
    let imported = import_tofu(&dir, &options(false)).unwrap_or_else(|e| panic!("{e}"));
    let ids: Vec<String> =
        ["aws_kms_key.events", "aws_sns_topic.events", "aws_sqs_queue.jobs", "aws_dynamodb_table.carts"]
            .map(String::from)
            .to_vec();
    let a = rhizo_iac::adopt(&imported.kdl, &ids, false, &options(true), &[]).unwrap_or_else(|e| panic!("{e}"));
    println!("{}", a.report);
    let r = &a.report;
    assert!(r.contains("every original instance is accounted for: 4 moved, 0 at their own address"), "{r}");
    for m in [
        "aws_kms_key.events -> module.events.aws_kms_key.this[0]",
        "aws_sns_topic.events -> module.events_topic.aws_sns_topic.this[0]",
        "aws_sqs_queue.jobs -> module.jobs.aws_sqs_queue.this[0]",
        "aws_dynamodb_table.carts -> module.carts.aws_dynamodb_table.this[0]",
    ] {
        assert!(r.contains(&format!("  moves {m}\n")), "{m}\n{r}");
    }
    assert!(a.kdl.contains("name=\"events_topic\""), "{}", a.kdl);
    assert!(r.contains("  adopting creates module.events.aws_kms_alias.this[\"raw-aws-events\"]"), "{r}");
    validate(&a.kdl, &scratch("raw-resources-adopted"));
}

/// A background adopt (the viewer's) reports each step and stops when told:
/// its `tofu` is killed and nothing comes back but `Cancelled`.
#[test]
#[ignore = "needs tofu and network for providers"]
fn adopt_reports_progress_and_cancels() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/adopt/raw-aws");
    let imported = import_tofu(&dir, &options(false)).unwrap_or_else(|e| panic!("{e}"));
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let opts = Options { cancel: Some(cancel.clone()), progress: Some(tx), ..options(true) };
    let run =
        std::thread::spawn(move || rhizo_iac::adopt(&imported.kdl, &["aws_sqs_queue.jobs".into()], false, &opts, &[]));
    let (mut steps, mut at) = (Vec::new(), None);
    for s in rx.iter() {
        if s == "planning the original code" {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            at = Some(std::time::Instant::now());
        }
        steps.push(s);
    }
    assert!(matches!(run.join().unwrap(), Err(Error::Cancelled)));
    assert!(at.unwrap().elapsed() < std::time::Duration::from_secs(5), "stops at once, not after the plan");
    assert_eq!(steps, ["compiling the adopted code", "planning the original code"]);
}

/// Raw Google Cloud resources: a bucket, a topic, a key with its key ring
/// and a Firestore database become rhizo's code (`rhizo_` locals from the
/// provider configuration name the project); a subscription and the topic
/// it reads are refused by name.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn raw_gcp_resources_adopt() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/adopt/raw-gcp");
    let imported = import_tofu(&dir, &options(false)).unwrap_or_else(|e| panic!("{e}"));
    let refused = |id: &str| match rhizo_iac::adopt(&imported.kdl, &[id.into()], false, &options(true), &[]) {
        Err(Error::NotAdopted(why)) => why.join("\n"),
        other => panic!("{id}: expected a refusal, got {other:?}"),
    };
    assert!(
        refused("google_pubsub_subscription.jobs_worker").contains("rhizo's code for a queue is a topic of its own")
    );
    assert!(refused("google_pubsub_topic.jobs").contains("[infra.adopt.edge]"));
    let ids: Vec<String> = [
        "google_storage_bucket.assets",
        "google_pubsub_topic.events",
        "google_kms_crypto_key.data",
        "google_firestore_database.app",
    ]
    .map(String::from)
    .to_vec();
    let a = rhizo_iac::adopt(&imported.kdl, &ids, false, &options(true), &[]).unwrap_or_else(|e| panic!("{e}"));
    println!("{}", a.report);
    let r = &a.report;
    assert!(r.contains("every original instance is accounted for: 4 moved, 6 at their own address"), "{r}");
    for m in [
        "google_kms_crypto_key.data -> module.data.google_kms_crypto_key.key[0]",
        "google_kms_key_ring.shop -> module.data.google_kms_key_ring.key_ring",
        "google_pubsub_topic.events -> module.events.google_pubsub_topic.topic[0]",
        "google_storage_bucket.assets -> module.assets.google_storage_bucket.bucket",
    ] {
        assert!(r.contains(&format!("  moves {m}\n")), "{m}\n{r}");
    }
    assert!(r.contains("kept from its code (names, places and tags of what exists): keyring, location, keys"), "{r}");
    // Names, places and labels are kept: no change lists them.
    for kept in ["name:", "topic:", "location:", "labels: {\"team\"", "protection_level"] {
        assert!(!r.lines().any(|l| l.starts_with("  adopting changes") && l.contains(kept)), "{kept}\n{r}");
    }
    assert!(r.contains("  creates: none\n"), "{r}");
    let root = compile_into(&a.kdl, &scratch("raw-gcp-adopted"));
    let rhizo_tf = std::fs::read_to_string(root.join(rhizo_compile::ADOPTED_FILE)).unwrap();
    assert!(rhizo_tf.contains("locals {\n  rhizo_project = var.project\n}"), "{rhizo_tf}");
    let main = std::fs::read_to_string(root.join("main.tf")).unwrap();
    assert!(main.contains("crypto_key_id = module.data.key_id_list[0]"), "{main}");
    assert!(main.contains("bucket = module.assets.name"), "{main}");
    validate(&a.kdl, &scratch("raw-gcp-validated"));
}

/// A raw Azure Key Vault key and its vault become the key vault module
/// (azurerm inside: moves within one type); a storage account, whose module
/// is built on azapi, is refused by name.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn raw_azure_resources_adopt() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/adopt/raw-azure");
    let imported = import_tofu(&dir, &options(false)).unwrap_or_else(|e| panic!("{e}"));
    match rhizo_iac::adopt(&imported.kdl, &["azurerm_storage_account.assets".into()], false, &options(true), &[]) {
        Err(Error::NotAdopted(why)) => assert!(why[0].contains("a move across types"), "{why:?}"),
        other => panic!("{other:?}"),
    }
    let a = rhizo_iac::adopt(&imported.kdl, &["azurerm_key_vault_key.data".into()], false, &options(true), &[])
        .unwrap_or_else(|e| panic!("{e}"));
    println!("{}", a.report);
    let r = &a.report;
    assert!(r.contains("every original instance is accounted for: 2 moved, 2 at their own address"), "{r}");
    assert!(r.contains("  moves azurerm_key_vault.shop -> module.data.azurerm_key_vault.this\n"), "{r}");
    assert!(
        r.contains(
            "  moves azurerm_key_vault_key.data -> module.data.module.keys[\"key\"].azurerm_key_vault_key.this\n"
        ),
        "{r}"
    );
    assert!(r.contains("  adopting creates module.data.azurerm_role_assignment.this[\"deployer\"]\n"), "{r}");
    for kept in ["name:", "location:", "tenant_id:", "tags:", "key_type:", "key_size:"] {
        assert!(!r.lines().any(|l| l.starts_with("  adopting changes") && l.contains(kept)), "{kept}\n{r}");
    }
    let root = compile_into(&a.kdl, &scratch("raw-azure-adopted"));
    let rhizo_tf = std::fs::read_to_string(root.join(rhizo_compile::ADOPTED_FILE)).unwrap();
    assert!(rhizo_tf.contains("data \"azurerm_client_config\" \"rhizo_current\" {}"), "{rhizo_tf}");
    validate(&a.kdl, &scratch("raw-azure-validated"));
}

/// Code that requires a provider outside rhizo's allow-list, in the root
/// module or only in a registry module, is refused by name before OpenTofu
/// downloads that provider; allowed for the run, it imports.
#[test]
#[ignore = "needs tofu and the registry; run with --ignored"]
fn providers_outside_the_allow_list_are_refused_before_download() {
    if !tofu_installed() {
        eprintln!("skipped: tofu is not installed");
        return;
    }
    for (name, at) in [("root-github", "the root module (main.tf)"), ("module-github", "module.team (versions.tf")] {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/providers").join(name);
        let cache = scratch(&format!("providers-{name}"));
        let opts = Options { plugin_cache: Some(cache.clone()), ..Options::default() };
        let github = cache.join("registry.opentofu.org/integrations/github");
        match import_tofu(&dir, &opts) {
            Err(e @ Error::NotAllowed(_)) => {
                let text = e.to_string();
                assert!(text.contains(&format!("integrations/github, required by {at}")), "{name}: {text}");
                assert!(text.contains("`--allow-provider integrations/github`"), "{text}");
            }
            other => panic!("{name}: not refused: {other:?}"),
        }
        assert!(!github.exists(), "{name}: the refused provider was downloaded");
        let opts = Options { allow_providers: vec!["integrations/github".into()], ..opts };
        let imported = import_tofu(&dir, &opts).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(imported.report.contains("L1 equal"), "{}", imported.report);
        assert!(github.exists(), "{name}: allowed, it is installed");
    }
}
