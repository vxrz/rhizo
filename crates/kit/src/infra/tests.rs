use crate::Kits;

fn run(src: &str) -> Vec<String> {
    let loaded = rhizo_lang::load(src, rhizo_lang::Syntax::Kdl);
    assert!(!loaded.has_errors(), "{:?}", loaded.diagnostics);
    crate::check(&loaded.document, &Kits::builtin())
        .into_iter()
        .map(|f| format!("{:?}: {}", f.diagnostic.severity, f.diagnostic.message))
        .collect()
}

const PLAYGROUND: &str = include_str!("../../../../examples/playground/07-typed-infra.rhizo");

/// A small environment that passes every rule; tests break one thing each.
const ENV: &str = r#"rhizo version=2
use infra version="0.1"
frame "Prod" id=prod type=infra.environment {
    props region="eu-central-1" state-bucket="tofu-state"
    node "Net" id=net type=aws.vpc { props cidr="10.20.0.0/16" }
    node "Fn" id=fn type=aws.lambda.function { props timeout-s=20 }
    node "DB" id=db type=aws.rds.postgres
    node "Jobs" id=jobs type=aws.sqs.queue { props visibility-timeout-s=120 }
    node "Files" id=files type=aws.s3.bucket
    node "Key" id=key type=aws.kms.key
    edge fn net kind=dependency
    edge db net kind=dependency
    edge fn db kind=data
    edge jobs fn kind=control
    edge fn files kind=data { props access="write" }
    edge files key kind=dependency
}
"#;

#[test]
fn clean_examples_pass() {
    assert_eq!(run(PLAYGROUND), Vec::<String>::new());
    assert_eq!(run(ENV), Vec::<String>::new());
}

#[test]
fn kit_loads() {
    let kits = Kits::builtin();
    let infra = kits.get("infra").unwrap();
    assert_eq!(infra.namespaces, ["infra", "aws"]);
    assert_eq!(infra.types.len(), 23);
    assert!(infra.types["infra.environment"].frame);
    assert_eq!(infra.types["aws.subnet"].parents, ["aws.vpc"]);
    assert_eq!(infra.types["infra.subnet"].parents, ["infra.network", "aws.vpc"]);
}

#[test]
fn props_and_parents_are_checked() {
    let src = ENV
        .replace(r#"cidr="10.20.0.0/16""#, r#"cidr="10.20.0.0/8" nat="always""#)
        .replace(r#"props timeout-s=20"#, r#"props timeout-s=2000 memroy=1"#)
        .replace(r#"region="eu-central-1""#, r#"region="Frankfurt""#);
    let found = run(&src);
    assert!(found.iter().any(|m| m.contains("vpc `net`, prop `cidr`")), "{found:#?}");
    assert!(found.iter().any(|m| m.contains("prop `nat`: \"always\" is not one of")), "{found:#?}");
    assert!(found.iter().any(|m| m.contains("prop `timeout-s`")), "{found:#?}");
    assert!(found.iter().any(|m| m.contains("unknown prop `memroy`")), "{found:#?}");
    assert!(found.iter().any(|m| m.contains("frame `prod`, prop `region`")), "{found:#?}");

    // A subnet tier belongs in a VPC; buckets do not go in subnets.
    let src = ENV.replace(
        r#"node "Files" id=files type=aws.s3.bucket"#,
        r#"node "Tier" id=tier type=aws.subnet { props tier="private"; node "B" id=b type=aws.s3.bucket; }
    node "Files" id=files type=aws.s3.bucket"#,
    );
    let found = run(&src);
    assert!(found.iter().any(|m| m.contains("subnet tier `tier` belongs inside a aws.vpc")), "{found:#?}");
    assert!(found.iter().any(|m| m.contains("s3 bucket `b` belongs inside a infra.environment")), "{found:#?}");
}

#[test]
fn rds_must_be_private_and_in_a_vpc() {
    let found = run(&ENV.replace("    edge db net kind=dependency\n", ""));
    assert!(found.iter().any(|m| m.starts_with("Error: [infra.rds.placement] database `db` is not in a VPC")));
    // Without a VPC of its own, the function cannot reach the database either.
    let found = run(&ENV.replace("    edge fn net kind=dependency\n", ""));
    assert_eq!(found, ["Error: [infra.lambda.vpc] `fn` connects to database `db` but does not run in its VPC"]);

    let public = ENV
        .replace(
            r#"node "Net" id=net type=aws.vpc { props cidr="10.20.0.0/16" }"#,
            r#"node "Net" id=net type=aws.vpc {
        props cidr="10.20.0.0/16"
        node "Public" id=public type=aws.subnet { props tier="public" }
        node "Public 2" id=public-2 type=aws.subnet { props tier="public" }
    }"#,
        )
        .replace("edge db net kind=dependency", "edge db public kind=dependency");
    let found = run(&public);
    assert_eq!(
        found,
        [
            "Error: [infra.subnet.tier] VPC `net` has more than one public subnet tier",
            "Error: [infra.rds.private] database `db` is in a public subnet, reachable from the internet",
        ]
    );
}

#[test]
fn sensitive_data_needs_a_kms_key() {
    let src = ENV.replace(r#"type=aws.sqs.queue { props "#, r#"type=aws.sqs.queue { props sensitive=#true "#);
    assert_eq!(
        run(&src),
        ["Error: [infra.kms.sensitive] `jobs` holds sensitive data but has no customer-managed KMS key"]
    );
    // Sensitive because the data kit says so: an entity with a pii=sensitive field is stored there.
    let src = ENV.replace("use infra version=\"0.1\"", "use infra version=\"0.1\"\nuse data version=\"0.1\"")
        + r#"node "Card" id=card type=data.entity {
    node "number" id=card-number type=data.field { props pii="sensitive" }
}
edge card db kind=data
"#;
    assert_eq!(
        run(&src),
        ["Error: [infra.kms.sensitive] `db` holds sensitive data but has no customer-managed KMS key"]
    );
}

#[test]
fn edge_rules() {
    let src = ENV
        .replace("visibility-timeout-s=120", "visibility-timeout-s=5")
        .replace(r#"access="write""#, r#"access="all""#)
        .replace("edge files key kind=dependency", "edge files key kind=dependency\n    edge key files")
        .replace(r#"state-bucket="tofu-state""#, "");
    assert_eq!(
        run(&src),
        [
            "Warning: [infra.state.remote] environment `prod` keeps its state on the machine that runs tofu",
            "Error: [infra.sqs.visibility] queue `jobs` hides a message for 5 s, less than the 20 s timeout of `fn`",
            "Error: [infra.edge.access] `fn` → `files`: access \"all\" is not \"read\", \"write\" or \"read-write\"",
            "Warning: [infra.edge.meaning] aws.kms.key → aws.s3.bucket has no meaning in the infra kit and is not compiled",
        ]
    );
    // A function in a VPC without NAT cannot call S3.
    let src = ENV.replace(r#"cidr="10.20.0.0/16""#, r#"cidr="10.20.0.0/16" nat="none""#);
    assert_eq!(
        run(&src),
        ["Warning: [infra.lambda.egress] `fn` cannot reach the AWS services it uses: VPC `net` has nat=\"none\""]
    );
    let src = ENV.replace(r#"cidr="10.20.0.0/16""#, r#"cidr="10.20.1.0/16""#);
    assert_eq!(run(&src), ["Error: [infra.vpc.cidr] `net`: 10.20.1.0/16 has host bits set"]);
}

#[test]
fn routes_come_from_edges_or_operations() {
    let src = ENV.replace("use infra version=\"0.1\"", "use infra version=\"0.1\"\nuse data version=\"0.1\"")
        + r#"frame "API spec" id=spec type=data.api {
    node "Place" id=place type=data.operation { props method="post" path="/orders" }
}
"#;
    let src = src.replace(
        "    edge jobs fn kind=control\n",
        "    edge jobs fn kind=control\n    node \"API\" id=api type=aws.apigateway.http\n    edge api fn kind=control\n    edge api fn kind=control { props route=\"POST /orders\" }\n    edge fn place kind=dependency\n",
    );
    assert_eq!(run(&src), ["Error: [infra.api.route] API `api` has route `POST /orders` twice"]);
    let src = src.replace(r#"route="POST /orders""#, r#"route="orders""#);
    assert_eq!(run(&src), ["Error: [infra.api.route] route `orders` is not `METHOD /path` or `$default`"]);
}

#[test]
fn edges_may_not_cross_environments() {
    let src = ENV.to_string()
        + r#"frame "Dev" id=dev type=infra.environment {
    props region="eu-central-1" state-bucket="tofu-state"
    node "Dev fn" id=dev-fn type=aws.lambda.function
}
edge dev-fn files kind=data
"#;
    assert_eq!(run(&src), ["Error: [infra.edge.environment] `dev-fn` → `files` crosses environments"]);
}

const IMPORTED: &str = r#"rhizo version=2
use infra version="0.1"
frame "prod" id=prod type=infra.environment {
    props {
        origin dir="prod"
        blocks {
            - type="variable" file="main.tf" {
                (array)labels "env"
                body type="string"
            }
        }
    }
    node "logs" id=aws_s3_bucket.logs type=infra.resource {
        props tf-type="aws_s3_bucket" name="logs" {
            body bucket="\"logs\"" {
                "dynamic \"rule\"" {
                    - for_each="var.rules"
                }
            }
            origin address="aws_s3_bucket.logs" shape="resource" file="main.tf"
        }
    }
    node "net" id=module.net type=infra.module { props source="terraform-aws-modules/vpc/aws" name="net" }
    edge aws_s3_bucket.logs module.net kind=dependency
}
"#;

#[test]
fn imported_environments_check_clean() {
    // No region, no state bucket, edges between generic nodes: all fine.
    assert_eq!(run(IMPORTED), Vec::<String>::new());
}

#[test]
fn imported_environments_hold_generic_nodes_only() {
    let found = run(&IMPORTED.replace("    edge aws", "    node \"Q\" id=q type=aws.sqs.queue\n    edge aws"));
    assert!(found.iter().any(|f| f.starts_with("Error: [infra.import.kit-type] `q` is a aws.sqs.queue")), "{found:?}");
}

/// An imported environment with nodes folded from their code.
const FOLDED: &str = r#"rhizo version=2
use infra version="0.1"
frame "Prod" id=prod type=infra.environment {
    props cloud="aws" {
        origin dir="prod"
    }
    node "logs" id=aws_s3_bucket.logs type=infra.bucket {
        props tf-type="aws_s3_bucket" name="logs" versioning=#false {
            body bucket="\"logs\""
            origin address="aws_s3_bucket.logs" shape="resource" file="main.tf"
        }
    }
    node "q" id=module.q type=infra.queue {
        props source="terraform-aws-modules/sqs/aws" name="q" dead-letter=#false ordered=#false lock-timeout-s=10 {
            body visibility_timeout_seconds="10"
            origin address="module.q" shape="module"
        }
    }
    node "f" id=module.f type=infra.function {
        props source="terraform-aws-modules/lambda/aws" name="f" memory-mb=128 timeout-s=30 {
            aws architecture="x86_64"
            body timeout="30" event_source_mapping="{ q = { event_source_arn = module.q.queue_arn } }"
            origin address="module.f" shape="module"
        }
    }
    edge module.q module.f kind=control
}
"#;

#[test]
fn rules_on_imported_code_are_an_audit() {
    let found = run(FOLDED);
    assert!(found.iter().all(|f| f.starts_with("Warning: audit: ")), "{found:?}");
    assert!(found.iter().any(|f| f.contains("[infra.audit.versioning] `aws_s3_bucket.logs`")), "{found:?}");
    assert!(found.iter().any(|f| f.contains("[infra.sqs.visibility] queue `module.q`")), "{found:?}");
}

#[test]
fn folded_props_are_read_only() {
    let found = run(&FOLDED.replace("versioning=#false", "versioning=#true"));
    assert!(
        found.iter().any(|f| f.starts_with("Error: [infra.import.derived] `aws_s3_bucket.logs`: prop `versioning` is read-only: its code says false, the diagram says true")),
        "{found:?}"
    );
    let found = run(&FOLDED.replace("architecture=\"x86_64\"", "architecture=\"arm64\""));
    assert!(found.iter().any(|f| f.contains("prop `aws.architecture` is read-only")), "{found:?}");
    let found = run(&FOLDED.replace(" lock-timeout-s=10", ""));
    assert!(
        found
            .iter()
            .any(|f| f.contains("prop `lock-timeout-s` is read-only: its code says 10, the diagram leaves it out")),
        "{found:?}"
    );
    // Code that does not fold to the type written.
    let found = run(&FOLDED.replace("id=module.q type=infra.queue", "id=module.q type=infra.topic"));
    assert!(
        found.iter().any(|f| f.contains(
            "[infra.import.derived] `module.q` is an infra.topic, but its code is code that folds to infra.queue"
        )),
        "{found:?}"
    );
}

/// [`FOLDED`] with the queue and the function adopted: rhizo's code now.
fn adopted() -> String {
    FOLDED
        .replace(
            "props source=\"terraform-aws-modules/sqs/aws\" name=\"q\" dead-letter=#false ordered=#false lock-timeout-s=10 {\n            body visibility_timeout_seconds=\"10\"\n            origin address=\"module.q\" shape=\"module\"",
            "props dead-letter=#false ordered=#false lock-timeout-s=10 {\n            origin address=\"module.q\" shape=\"rhizo\"",
        )
        .replace(
            "props source=\"terraform-aws-modules/lambda/aws\" name=\"f\" memory-mb=128 timeout-s=30 {\n            aws architecture=\"x86_64\"\n            body timeout=\"30\" event_source_mapping=\"{ q = { event_source_arn = module.q.queue_arn } }\"\n            origin address=\"module.f\" shape=\"module\"",
            "props memory-mb=128 timeout-s=30 {\n            aws architecture=\"x86_64\"\n            origin address=\"module.f\" shape=\"rhizo\"",
        )
}

#[test]
fn adopted_props_are_editable_and_checked_in_full() {
    let src = adopted();
    assert!(src.contains("shape=\"rhizo\"") && !src.contains("terraform-aws-modules/sqs"), "{src}");
    // Editing a prop of an adopted node is no longer read-only.
    let found = run(&src.replace("memory-mb=128", "memory-mb=1024"));
    assert!(!found.iter().any(|f| f.contains("[infra.import.")), "{found:?}");
    // Rule findings on adopted nodes are full findings: rhizo writes their code.
    assert!(
        found.iter().any(|f| f.starts_with("Error: [infra.sqs.visibility] queue `module.q` hides a message for 10 s")),
        "{found:?}"
    );
    // The bucket is still imported code: audited.
    assert!(found.iter().any(|f| f.starts_with("Warning: audit: [infra.audit.versioning]")), "{found:?}");
    assert_eq!(
        run(&src.replace("lock-timeout-s=10", "lock-timeout-s=60")).iter().filter(|f| f.starts_with("Error")).count(),
        0
    );
}

#[test]
fn adopted_code_refers_to_rhizo_code_only() {
    // The function's code refers to the queue as `module.q`: the queue must be rhizo's code too.
    let src = adopted().replace(
        "node \"q\" id=module.q type=infra.queue {\n        props dead-letter=#false ordered=#false lock-timeout-s=10 {\n            origin address=\"module.q\" shape=\"rhizo\"",
        "node \"q\" id=aws_sqs_queue.q type=infra.queue {\n        props tf-type=\"aws_sqs_queue\" name=\"q\" dead-letter=#false ordered=#false lock-timeout-s=60 {\n            body visibility_timeout_seconds=\"60\"\n            origin address=\"aws_sqs_queue.q\" shape=\"resource\"",
    )
    .replace("edge module.q module.f", "edge aws_sqs_queue.q module.f");
    let found = run(&src);
    assert!(
        found.iter().any(|f| f.starts_with(
            "Error: [infra.adopt.edge] `aws_sqs_queue.q` → `module.f`: rhizo's code for adopted `module.f` refers to `aws_sqs_queue.q`"
        )),
        "{found:?}"
    );
}

#[test]
fn verbatim_satellites_of_adopted_nodes_are_checked() {
    let src = r#"rhizo version=2
use infra version="0.1"
frame "Prod" id=prod type=infra.environment {
    props cloud="aws" {
        origin dir="prod"
    }
    node "b" id=aws_s3_bucket.b type=infra.bucket {
        props versioning=#true {
            origin address="aws_s3_bucket.b" shape="rhizo"
        }
    }
    node "b" id=aws_s3_bucket_lifecycle_configuration.b type=infra.resource {
        props tf-type="aws_s3_bucket_lifecycle_configuration" name="b" {
            body bucket="module.b.s3_bucket_id"
            origin address="aws_s3_bucket_lifecycle_configuration.b" shape="resource"
        }
    }
}
"#;
    assert_eq!(run(src), Vec::<String>::new());
    let found = run(&src.replace(
        "            origin address=\"aws_s3_bucket.b\"",
        "            lifecycle {\n                - expire-after-days=7\n            }\n            origin address=\"aws_s3_bucket.b\"",
    ));
    assert!(
        found.iter().any(|f| f.starts_with("Error: [infra.adopt.satellite] `aws_s3_bucket_lifecycle_configuration.b` configures adopted `aws_s3_bucket.b`")),
        "{found:?}"
    );
}

#[test]
fn kit_types_in_imported_environments_need_their_code() {
    let found =
        run(&FOLDED
            .replace("            origin address=\"aws_s3_bucket.logs\" shape=\"resource\" file=\"main.tf\"\n", ""));
    assert!(
        found.iter().any(|f| f.starts_with("Error: [infra.import.kit-type] `aws_s3_bucket.logs` is a infra.bucket")),
        "{found:?}"
    );
    // The code props are checked like a generic node's (shared definitions in imported.json).
    let found = run(&FOLDED.replace("file=\"main.tf\"", "file=\"../main.tf\""));
    assert!(found.iter().any(|f| f.starts_with("Error: bucket `aws_s3_bucket.logs`, prop `origin.file`")), "{found:?}");
    // Designed environments do not carry code.
    let found =
        run(&ENV
            .replace("id=files type=aws.s3.bucket", "id=files type=infra.bucket { props tf-type=\"aws_s3_bucket\" }"));
    assert!(found.iter().any(|f| f.starts_with("Error: [infra.import.code] `files` has `tf-type`")), "{found:?}");
}

#[test]
fn generic_bodies_take_hcl_keys_only() {
    let found = run(&IMPORTED.replace("body bucket=", "body \"bucket = 1\\nresource\"="));
    assert!(found.iter().any(|f| f.contains("body")), "{found:?}");
    let found = run(&IMPORTED.replace("tf-type=\"aws_s3_bucket\"", "tf-type=\"aws s3\""));
    assert!(found.iter().any(|f| f.contains("tf-type")), "{found:?}");
}

/// An azure environment that passes every rule; tests break one thing each.
const AZURE: &str = r#"rhizo version=2
use infra version="0.1"
frame "Prod" id=prod type=infra.environment {
    props cloud="azure" region="westeurope" {
        azure state-storage-account="tofustate"
    }
    node "Net" id=net type=infra.network {
        props cidr="10.20.0.0/16" flow-logs=#false
        node "Data" id=data type=infra.subnet { props tier="database" }
    }
    node "Fn" id=fn type=infra.function { props runtime="python3.12" timeout-s=20 }
    node "DB" id=db type=infra.database
    node "Jobs" id=jobs type=infra.queue { props lock-timeout-s=120 }
    node "Files" id=files type=infra.bucket { props { azure access-tier="Cool" } }
    node "Key" id=key type=infra.key
    edge fn net kind=dependency
    edge db data kind=dependency
    edge fn db kind=data
    edge jobs fn kind=control
    edge fn files kind=data { props access="write" }
    edge files key kind=dependency
    edge jobs key kind=dependency
}
"#;

#[test]
fn azure_environments_check_clean() {
    assert_eq!(run(AZURE), Vec::<String>::new());
}

#[test]
fn aws_types_are_refused_on_azure() {
    let found =
        run(&AZURE.replace("id=jobs type=infra.queue { props lock-timeout-s=120 }", "id=jobs type=aws.sqs.queue"));
    assert!(
        found
            .iter()
            .any(|f| f.starts_with("Error: [infra.cloud.type] `jobs` is an aws.sqs.queue")
                && f.contains("targets azure")),
        "{found:?}"
    );
    // The AWS region pattern is for aws only; azure regions are lowercase words.
    let found = run(&AZURE.replace("region=\"westeurope\"", "region=\"West Europe\""));
    assert!(found.iter().any(|f| f.contains("region")), "{found:?}");
    let found = run(&AZURE.replace(
        "    props cloud=\"azure\" region=\"westeurope\" {\n        azure state-storage-account=\"tofustate\"\n    }\n",
        "    props cloud=\"azure\" region=\"westeurope\"\n",
    ));
    assert!(found.iter().any(|f| f.starts_with("Warning: [infra.state.remote]")), "{found:?}");
}

#[test]
fn azure_gaps_are_findings() {
    let errors = [
        ("lock-timeout-s=120", "lock-timeout-s=600", "[infra.azure.lock] queue `jobs`"),
        ("runtime=\"python3.12\"", "runtime=\"ruby3.3\"", "[infra.azure.runtime] function `fn`"),
        ("timeout-s=20 }", "timeout-s=20 memory-mb=8192 }", "[infra.azure.memory] function `fn`"),
        ("id=db type=infra.database", "id=db type=infra.database { props backup-days=3 }", "[infra.azure.backup]"),
        (
            "id=db type=infra.database",
            "id=db type=infra.database { props { azure sku-name=\"B_Standard_B1ms\" } }",
            "[infra.azure.sku]",
        ),
        ("edge fn net kind=dependency", "edge fn data kind=dependency", "[infra.azure.subnet]"),
        (
            "    edge jobs key kind=dependency\n",
            "    edge jobs key kind=dependency\n    node \"Key 2\" id=key2 type=infra.key\n    node \"T\" id=t type=infra.topic\n    edge t key2 kind=dependency\n",
            "[infra.azure.key]",
        ),
        (
            "    edge jobs fn kind=control\n",
            "    edge jobs fn kind=control\n    node \"API\" id=api type=infra.api\n    edge api fn kind=control { props route=\"ANY /{proxy+}\" }\n",
            "[infra.azure.route]",
        ),
    ];
    for (from, to, expected) in errors {
        let src = AZURE.replace(from, to);
        assert_ne!(src, AZURE, "{from}");
        let found = run(&src);
        assert!(found.iter().any(|f| f.starts_with("Error: ") && f.contains(expected)), "{expected}: {found:?}");
    }
    let warnings = [
        ("flow-logs=#false", "flow-logs=#true", "[infra.azure.flow-logs]"),
        ("timeout-s=20 }", "timeout-s=20 handler=\"app.main\" }", "[infra.azure.handler]"),
        ("lock-timeout-s=120", "lock-timeout-s=120 dead-letter=#false", "[infra.azure.dead-letter]"),
        (
            "id=db type=infra.database",
            "id=db type=infra.database { props engine-version=\"16.4\" }",
            "[infra.azure.minor-version]",
        ),
        (
            "    node \"Key\" id=key",
            "    node \"T\" id=t type=infra.table { props partition-key=\"id\" sort-key=\"at\" ttl-attribute=\"exp\" }\n    node \"Key\" id=key",
            "[infra.azure.ttl]",
        ),
        (
            "props cloud=\"azure\" region=\"westeurope\"",
            "props cloud=\"azure\" region=\"westeurope\" budget-monthly=50",
            "[infra.azure.budget]",
        ),
        (
            "timeout-s=20 }",
            "timeout-s=20 { aws architecture=\"x86_64\" } }",
            "[infra.cloud.setting] `fn`: its aws settings apply only on aws",
        ),
    ];
    for (from, to, expected) in warnings {
        let src = AZURE.replace(from, to);
        assert_ne!(src, AZURE, "{from}");
        let found = run(&src);
        assert!(found.iter().any(|f| f.starts_with("Warning: ") && f.contains(expected)), "{expected}: {found:?}");
        assert!(!found.iter().any(|f| f.starts_with("Error")), "{expected}: {found:?}");
    }
}

#[test]
fn azure_settings_on_aws_are_a_warning() {
    let src = AZURE.replace(
        "props cloud=\"azure\" region=\"westeurope\" {\n        azure state-storage-account=\"tofustate\"\n    }",
        "props region=\"eu-central-1\" state-bucket=\"tofu-state\"",
    );
    let found = run(&src);
    assert!(
        found.iter().any(|f| f.contains("[infra.cloud.setting] `files`: its azure settings apply only on azure")),
        "{found:?}"
    );
}

#[test]
fn flex_runtimes_and_memory() {
    use super::azure::{flex_memory, flex_runtime};
    assert_eq!(flex_runtime("nodejs22.x"), Some(("node", "22".into())));
    assert_eq!(flex_runtime("python3.12"), Some(("python", "3.12".into())));
    assert_eq!(flex_runtime("java21"), Some(("java", "21".into())));
    assert_eq!(flex_runtime("dotnet8"), Some(("dotnet-isolated", "8.0".into())));
    assert_eq!(flex_runtime("ruby3.3"), None);
    assert_eq!(flex_runtime("provided.al2023"), None);
    let node = |props: &str| {
        let src = format!(
            "rhizo version=2\nuse infra version=\"0.1\"\nnode \"F\" id=f type=infra.function {{ props {props} }}\n"
        );
        let loaded = rhizo_lang::load(&src, rhizo_lang::Syntax::Kdl);
        loaded.document.nodes.values().next().unwrap().clone()
    };
    assert_eq!(flex_memory(&node("memory-mb=128")), Some(512));
    assert_eq!(flex_memory(&node("memory-mb=1024")), Some(2048));
    assert_eq!(flex_memory(&node("memory-mb=8192")), None);
    assert_eq!(flex_memory(&node("memory-mb=8192 { azure instance-memory-mb=4096 }")), Some(4096));
}

/// A gcp environment that passes every rule; tests break one thing each.
const GCP: &str = r#"rhizo version=2
use infra version="0.1"
frame "Prod" id=prod type=infra.environment {
    props cloud="gcp" region="europe-west3" {
        gcp state-bucket="tofu-state"
    }
    node "Net" id=net type=infra.network {
        props cidr="10.20.0.0/16" nat="none"
        node "Data" id=data type=infra.subnet { props tier="database" }
    }
    node "Fn" id=fn type=infra.function { props runtime="python3.12" timeout-s=20 }
    node "DB" id=db type=infra.database { props { gcp tier="db-custom-2-7680" } }
    node "Jobs" id=jobs type=infra.queue { props lock-timeout-s=120 }
    node "Files" id=files type=infra.bucket { props { gcp storage-class="COLDLINE" } }
    node "Key" id=key type=infra.key
    node "API" id=api type=infra.api { props { gcp region="europe-west1" } }
    edge fn net kind=dependency
    edge db data kind=dependency
    edge fn db kind=data
    edge jobs fn kind=control
    edge api fn kind=control { props route="GET /files/{name}" }
    edge fn files kind=data { props access="write" }
    edge files key kind=dependency
    edge jobs key kind=dependency
}
"#;

#[test]
fn gcp_environments_check_clean() {
    // nat="none" is fine on gcp: Private Google Access reaches Google APIs.
    assert_eq!(run(GCP), Vec::<String>::new());
}

#[test]
fn gcp_regions_state_and_aws_types() {
    let found = run(&GCP.replace("region=\"europe-west3\"", "region=\"eu-central-1\""));
    assert!(found.iter().any(|f| f.contains("region")), "{found:?}");
    let found =
        run(&GCP.replace("id=jobs type=infra.queue { props lock-timeout-s=120 }", "id=jobs type=aws.sqs.queue"));
    assert!(
        found.iter().any(|f| f.starts_with("Error: [infra.cloud.type] `jobs` is an aws.sqs.queue") && f.contains("targets gcp")),
        "{found:?}"
    );
    let found = run(&GCP.replace(
        "    props cloud=\"gcp\" region=\"europe-west3\" {\n        gcp state-bucket=\"tofu-state\"\n    }\n",
        "    props cloud=\"gcp\" region=\"europe-west3\"\n",
    ));
    assert!(found.iter().any(|f| f.starts_with("Warning: [infra.state.remote]")), "{found:?}");
    let found = run(&GCP.replace("gcp tier=\"db-custom-2-7680\"", "gcp tier=\"large\""));
    assert!(found.iter().any(|f| f.contains("tier")), "{found:?}");
}

#[test]
fn gcp_gaps_are_findings() {
    let errors = [
        ("lock-timeout-s=120", "lock-timeout-s=900", "[infra.gcp.ack-deadline] queue `jobs`"),
        ("lock-timeout-s=120", "lock-timeout-s=120 max-deliveries=3", "[infra.gcp.deliveries] queue `jobs`"),
        ("lock-timeout-s=120", "lock-timeout-s=120 retention-s=120", "[infra.gcp.retention] queue `jobs`"),
        ("runtime=\"python3.12\"", "runtime=\"provided.al2023\"", "[infra.gcp.runtime] function `fn`"),
        ("edge fn net kind=dependency", "edge fn data kind=dependency", "[infra.gcp.tier] function `fn`"),
        (
            "{ gcp region=\"europe-west1\" }",
            "",
            "[infra.gcp.gateway-region] API `api`: API Gateway has no gateways in europe-west3",
        ),
        ("route=\"GET /files/{name}\"", "route=\"ANY /{proxy+}\"", "[infra.gcp.route]"),
        (
            "    edge fn db kind=data\n",
            "    edge fn db kind=data\n    edge fn jobs kind=data { props access=\"read\" }\n",
            "[infra.gcp.trigger] queue `jobs`",
        ),
    ];
    for (from, to, expected) in errors {
        let src = GCP.replace(from, to);
        assert_ne!(src, GCP, "{from}");
        let found = run(&src);
        assert!(found.iter().any(|f| f.starts_with("Error: ") && f.contains(expected)), "{expected}: {found:?}");
    }
    let partition_only = run(&GCP.replace(
        "    node \"Key\" id=key",
        "    node \"T\" id=t type=infra.table { props partition-key=\"id\" }\n    node \"Key\" id=key",
    ));
    assert!(
        !partition_only.iter().any(|f| f.contains("[infra.gcp.keys]")),
        "a partition key is the document id: {partition_only:?}"
    );
    let warnings = [
        (
            "    node \"Key\" id=key",
            "    node \"T\" id=t type=infra.table { props partition-key=\"id\" sort-key=\"at\" ttl-attribute=\"exp\" }\n    node \"Key\" id=key",
            "[infra.gcp.keys] table `t`",
        ),
        (
            "    node \"Key\" id=key",
            "    node \"T\" id=t type=infra.table { props partition-key=\"id\" ttl-attribute=\"exp\" }\n    node \"Key\" id=key",
            "[infra.gcp.ttl] table `t`",
        ),
        (
            "props { gcp tier=\"db-custom-2-7680\" }",
            "props engine-version=\"16.4\" { gcp tier=\"db-custom-2-7680\" }",
            "[infra.gcp.minor-version]",
        ),
        (
            "props { gcp tier=\"db-custom-2-7680\" }",
            "props backup-days=14 { gcp tier=\"db-custom-2-7680\" }",
            "[infra.gcp.pitr]",
        ),
        (
            "props { gcp region=\"europe-west1\" }",
            "props { cors-origins { - \"*\" }; gcp region=\"europe-west1\"; }",
            "[infra.gcp.cors]",
        ),
        (
            "    edge fn db kind=data\n",
            "    edge fn db kind=data\n    node \"G\" id=g type=infra.function { props timeout-s=900 }\n    edge api g kind=control { props route=\"POST /g\" }\n",
            "[infra.gcp.wait] function `g`",
        ),
        (
            "    edge fn db kind=data\n",
            "    edge fn db kind=data\n    node \"T\" id=t type=infra.topic\n    edge t jobs kind=data\n",
            "[infra.gcp.forward] `t` → `jobs`",
        ),
        (
            "props cloud=\"gcp\" region=\"europe-west3\"",
            "props cloud=\"gcp\" region=\"europe-west3\" budget-monthly=50",
            "[infra.gcp.budget]",
        ),
        (
            "timeout-s=20 }",
            "timeout-s=20 { azure instance-memory-mb=2048 } }",
            "[infra.cloud.setting] `fn`: its azure settings apply only on azure",
        ),
    ];
    for (from, to, expected) in warnings {
        let src = GCP.replace(from, to);
        assert_ne!(src, GCP, "{from}");
        let found = run(&src);
        assert!(found.iter().any(|f| f.starts_with("Warning: ") && f.contains(expected)), "{expected}: {found:?}");
        assert!(!found.iter().any(|f| f.starts_with("Error")), "{expected}: {found:?}");
    }
}

#[test]
fn gcp_runtimes_and_memory() {
    use super::gcp::{memory, runtime};
    assert_eq!(runtime("nodejs22.x").as_deref(), Some("nodejs22"));
    assert_eq!(runtime("python3.12").as_deref(), Some("python312"));
    assert_eq!(runtime("ruby3.3").as_deref(), Some("ruby33"));
    assert_eq!(runtime("java21").as_deref(), Some("java21"));
    assert_eq!(runtime("dotnet8").as_deref(), Some("dotnet8"));
    assert_eq!(runtime("provided.al2023"), None);
    let node = |props: &str| {
        let src = format!(
            "rhizo version=2\nuse infra version=\"0.1\"\nnode \"F\" id=f type=infra.function {{ props {props} }}\n"
        );
        let loaded = rhizo_lang::load(&src, rhizo_lang::Syntax::Kdl);
        loaded.document.nodes.values().next().unwrap().clone()
    };
    assert_eq!(memory(&node("memory-mb=128")), 128);
    assert_eq!(memory(&node("memory-mb=1000")), 1024);
    assert_eq!(memory(&node("memory-mb=10240")), 16384);
}
