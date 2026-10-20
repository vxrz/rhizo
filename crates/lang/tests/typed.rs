//! Format version 2: typed nodes, props (JiK), ports, provenance, CURIEs.

use pretty_assertions::assert_eq;
use rhizo_core::{Confidence, Document, EdgeKind, NodeId, PortDir, ProvMethod, ReviewStatus};
use rhizo_lang::{Severity, Syntax, load, to_json, to_kdl, to_yaml};
use serde_json::json;

const TYPED: &str = r##"rhizo version=2
title "Shop"
use infra version="0.1"
base "https://kg.example.org/shop/"
prefix HP "http://purl.obolibrary.org/obo/HP_"

frame "Prod" type=infra.environment layout=layered {
    props region="eu-central-1"

    node "Receipts" id=receipts type=aws.s3.bucket {
        props versioning=#true storage-gb=20 {
            owner "team-orders"
            cors-origins "https://a.example" "https://b.example"
            tags { cost-center "cc-42"; env "prod" }
            lifecycle {
                - prefix="tmp/" expire-after-days=7
                - prefix="logs/" expire-after-days=90
            }
            (array)azs "eu-central-1a"
            (object)extra
            kms-key #null
        }
        port out "created" kind=data schema="data:s3-object-created"
        accept "Objects are encrypted at rest"
        accept "Non-TLS requests are rejected"
        prov method=extracted by="agent:claude@run-1" at="2026-10-19" confidence=0.8 {
            source src-1 locator="Table 3" quote="…"
            derived-from e-12
            review status=accepted by="human:arvid" at="2026-10-19T10:00"
        }
    }
    node "Orders fn" id=orders-fn type=aws.lambda.function
    node "Tachycardia" id=HP:0001649

    edge orders-fn receipts kind=data id=orders-write {
        props access="write" prefix="receipts/"
    }
    edge HP:0001649 receipts type=HP:0000001
}
"##;

fn ok(src: &str, syntax: Syntax) -> Document {
    let loaded = load(src, syntax);
    assert!(!loaded.has_errors(), "{:#?}", loaded.diagnostics);
    loaded.document
}

#[test]
fn typed_document_loads() {
    let d = ok(TYPED, Syntax::Kdl);
    assert_eq!(d.kits[0].kit, "infra");
    assert_eq!(d.prefixes["HP"], "http://purl.obolibrary.org/obo/HP_");
    assert_eq!(d.frames[0].ty.as_ref().unwrap().as_str(), "infra.environment");
    assert_eq!(d.frames[0].props["region"], json!("eu-central-1"));

    let r = &d.nodes[&NodeId::from("receipts")];
    assert_eq!(r.ty.as_ref().unwrap().as_str(), "aws.s3.bucket");
    assert_eq!(
        serde_json::to_value(&r.props).unwrap(),
        json!({
            "versioning": true,
            "storage-gb": 20,
            "owner": "team-orders",
            "cors-origins": ["https://a.example", "https://b.example"],
            "tags": { "cost-center": "cc-42", "env": "prod" },
            "lifecycle": [
                { "prefix": "tmp/", "expire-after-days": 7 },
                { "prefix": "logs/", "expire-after-days": 90 }
            ],
            "azs": ["eu-central-1a"],
            "extra": {},
            "kms-key": null
        })
    );
    assert_eq!(r.ports[0].dir, PortDir::Out);
    assert_eq!(r.ports[0].kind, Some(EdgeKind::Data));
    assert_eq!(r.accept.len(), 2);
    let prov = r.prov.as_ref().unwrap();
    assert_eq!(prov.method, ProvMethod::Extracted);
    assert_eq!(prov.confidence, Some(Confidence::Number(0.8)));
    assert_eq!(prov.sources[0].locator.as_deref(), Some("Table 3"));
    assert_eq!(prov.review.as_ref().unwrap().status, ReviewStatus::Accepted);

    let e = &d.edges[0];
    assert_eq!(e.kind, Some(EdgeKind::Data));
    assert_eq!(e.id.as_ref().unwrap().as_str(), "orders-write");
    assert_eq!(e.props["access"], json!("write"));
    assert_eq!(d.edges[1].ty.as_ref().unwrap().as_str(), "HP:0000001");
}

#[test]
fn typed_roundtrips_through_every_syntax() {
    let d = ok(TYPED, Syntax::Kdl);
    let kdl = to_kdl(&d);
    assert!(kdl.starts_with("rhizo version=2\n"));
    let d2 = ok(&kdl, Syntax::Kdl);
    assert_eq!(d2, d);
    assert_eq!(to_kdl(&d2), kdl, "fmt is idempotent");

    let json = to_json(&d);
    assert!(json.contains("\"rhizo\": 2"));
    assert_eq!(ok(&json, Syntax::Json), d);
    let yaml = to_yaml(&d);
    let d3 = ok(&yaml, Syntax::Yaml);
    assert_eq!(d3, d);
    assert_eq!(to_kdl(&d3), kdl);
}

#[test]
fn version_gate() {
    let loaded = load("node \"A\" type=aws.s3.bucket id=a", Syntax::Kdl);
    let err = loaded.diagnostics.iter().find(|d| d.is_error()).expect("error");
    assert!(err.message.contains("version 2"));
    assert!(err.help.as_deref().unwrap().contains("rhizo version=2"));
    assert!(err.span.is_some());

    let loaded = load(r#"{"nodes":[{"label":"A","type":"aws.s3.bucket"}]}"#, Syntax::Json);
    assert!(loaded.diagnostics.iter().any(|d| d.is_error() && d.message.contains("version 2")));

    let loaded = load("rhizo version=3\n- \"A\"", Syntax::Kdl);
    assert!(loaded.diagnostics[0].message.contains("version 3"));
}

#[test]
fn unknown_json_keys_are_errors() {
    let loaded =
        load(r#"{"nodes":[{"label":"A","lable":"B"}],"edges":[{"from":"a","to":"a","kindd":"data"}]}"#, Syntax::Json);
    let msgs: Vec<_> = loaded.diagnostics.iter().map(|d| (d.message.clone(), d.help.clone())).collect();
    assert!(
        msgs.iter().any(|(m, h)| m.contains("`lable`") && h.as_deref() == Some("did you mean `label`?")),
        "{msgs:?}"
    );
    assert!(msgs.iter().any(|(m, _)| m.contains("`kindd`")));
}

#[test]
fn plain_documents_stay_version_1() {
    let d = ok("- \"A\" { - \"B\" }\nedge a b", Syntax::Kdl);
    assert!(!d.is_typed());
    assert!(to_kdl(&d).starts_with("rhizo version=1\n"));
    assert!(!to_json(&d).contains("\"rhizo\""));
}

#[test]
fn typed_nodes_get_pinned_ids() {
    let loaded = load("rhizo version=2\nnode \"Orders DB\" type=aws.rds.postgres", Syntax::Kdl);
    let w = loaded.diagnostics.iter().find(|d| d.severity == Severity::Warning).expect("warning");
    assert!(w.message.contains("no explicit id"));
    assert!(to_kdl(&loaded.document).contains("\"Orders DB\" id=orders-db type=aws.rds.postgres"));
}

#[test]
fn jik_and_structure_errors() {
    let cases = [
        ("rhizo version=2\nnode \"A\" { props { x } }", "no value"),
        ("rhizo version=2\nnode \"A\" { props { x 1 k=2 } }", "not both"),
        ("rhizo version=2\nnode \"A\" { props { (list)x 1 } }", "unknown annotation"),
        ("rhizo version=2\nnode \"A\" { props { - 1 } }", "belong in an array"),
        ("rhizo version=2\nnode \"A\" { port sideways \"p\" }", "port direction"),
        ("rhizo version=2\nnode \"A\" { prov by=\"x\" }", "needs `method=`"),
        ("rhizo version=2\nedge a b c id=e1", "cannot share one id"),
        ("rhizo version=2\nnode \"A\" id=XX:1", "prefix `XX`"),
        ("rhizo version=2\nnode \"A\" id=a type=Bucket", "invalid type name"),
        ("rhizo version=2\nnode \"A\" id=a\nedge a a id=a", "more than once"),
        ("rhizo version=2\nnode \"A\" { prov method=asserted at=\"monday\" }", "ISO 8601"),
    ];
    for (src, want) in cases {
        let loaded = load(src, Syntax::Kdl);
        assert!(
            loaded.diagnostics.iter().any(|d| d.is_error() && d.message.contains(want)),
            "{src:?} should fail with {want:?}, got {:#?}",
            loaded.diagnostics
        );
    }
}
