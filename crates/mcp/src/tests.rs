use serde_json::json;

use super::*;

const EXAMPLE: &str = include_str!("../../../examples/playground/11-data-model.rhizo");

/// A workspace over a private copy of `src`.
fn ws(name: &str, src: &str) -> (Workspace, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rhizo-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("shop.rhizo");
    std::fs::write(&file, src).unwrap();
    (Workspace::new(file), dir)
}

#[test]
fn explores_the_document() {
    let (w, dir) = ws("explore", EXAMPLE);
    let o = w.overview().unwrap();
    assert_eq!(o["title"], "Shop data model");
    assert_eq!(o["types"]["data.field"], 7);
    assert_eq!(
        o["frames"][1],
        json!({ "id": "shop-api", "label": "Shop API", "type": "data.api", "layout": "layered", "nodes": 2 })
    );

    let fields = w.list_nodes(Some("data.field"), None, None, Some("ID")).unwrap();
    assert_eq!(fields["total"], 2, "{fields}");
    assert_eq!(w.list_nodes(Some("data"), None, None, None).unwrap()["total"], 14 - 1, "all but the mailer");

    let order = w.get_node("order").unwrap();
    assert_eq!(order["kit_type"]["label"], "Entity");
    assert_eq!(order["children"].as_array().unwrap().len(), 4);
    assert_eq!(
        order["outgoing"][0],
        json!({ "from": "order", "to": "customer", "label": "placed by", "kind": "data", "props": { "cardinality": "one" } })
    );
    assert_eq!(order["ports"][0]["port"]["name"], "ref");
    assert!(w.get_node("Order").unwrap_err().contains("did you mean `order`"));

    let n = w.neighbors("order", Some("data"), Some("out"), Some(2)).unwrap();
    let ids: Vec<&str> = n["nodes"].as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["customer", "place-order", "order-placed"]);
    let both = w.neighbors("orders-placed", None, None, Some(1)).unwrap();
    assert_eq!(both["edges"].as_array().unwrap().len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_types_and_preview() {
    let (w, dir) = ws("check", &EXAMPLE.replace(r#"method="post""#, r#"method="send""#));
    let c = w.check().unwrap();
    assert_eq!(c["ok"], false);
    let d = &c["diagnostics"][0];
    assert!(d["message"].as_str().unwrap().contains("prop `method`"), "{c}");
    assert_eq!(d["line"], 26);
    assert!(w.compile_preview(None).unwrap_err().starts_with("cannot compile"));

    std::fs::write(&w.path, EXAMPLE).unwrap(); // fixed on disk: the next call sees it
    assert_eq!(w.check().unwrap()["ok"], true);
    let files = w.compile_preview(None).unwrap();
    assert_eq!(files["files"].as_array().unwrap().len(), 7);
    let api = w.compile_preview(Some("specs/shop-api.openapi.yaml")).unwrap();
    assert!(api["contents"].as_str().unwrap().contains("operationId: placeOrder"));
    assert!(!dir.join("build").exists(), "preview writes nothing");

    assert_eq!(w.types(Some("data.operation")).unwrap()["parents"], json!(["data.api"]));
    assert_eq!(w.types(None).unwrap()["kits"][0]["kit"], "data");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn proposals_never_touch_the_document() {
    let (w, dir) = ws("propose", EXAMPLE);
    let old = r#"node "Mailer" id=mailer"#;
    let a = w.propose_change(None, old, r#"node "Mailer service" id=mailer"#, "Clearer name.").unwrap();
    assert_eq!(a["proposal"], 1);
    assert!(a["how"].as_str().unwrap().contains("--apply 1"));
    let md = std::fs::read_to_string(dir.join(".rhizo/proposals/0001.md")).unwrap();
    assert!(md.contains("Clearer name.") && md.contains("Mailer service"));
    assert_eq!(std::fs::read_to_string(&w.path).unwrap(), EXAMPLE, "never applied by the server");
    // Checked when proposed: broken results and missing reasons are refused.
    assert!(w.propose_change(None, r#"method="post""#, r#"method="send""#, "x").unwrap_err().contains("errors"));
    assert!(w.propose_change(None, old, "x", " ").is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn writes_are_validated_and_minimal() {
    let (w, dir) = ws("write", EXAMPLE);
    // A broken document is refused and nothing changes.
    let err = w.write_document("rhizo version=1\nedge\n", None).unwrap_err();
    assert!(err.starts_with("not written: the text has errors"), "{err}");
    assert_eq!(std::fs::read_to_string(&w.path).unwrap(), EXAMPLE);

    // A targeted edit keeps everything else byte for byte.
    let r = w.edit_document(r#"node "Mailer" id=mailer"#, r#"node "Mailer service" id=mailer"#, None).unwrap();
    assert_eq!(r["written"], w.path.display().to_string());
    let now = std::fs::read_to_string(&w.path).unwrap();
    assert_eq!(now, EXAMPLE.replace(r#"node "Mailer" id=mailer"#, r#"node "Mailer service" id=mailer"#));
    assert!(w.edit_document("id=", "x", None).unwrap_err().contains("occurs"));
    assert!(w.edit_document("nowhere to be found", "x", None).unwrap_err().contains("does not occur"));
    let bad = w.edit_document(r#"method="post""#, r#"method="send""#, None).unwrap_err();
    assert!(bad.contains("prop `method`"), "kit rules apply to writes too: {bad}");

    // An agent can start from nothing.
    let fresh = Workspace::new(dir.join("new.rhizo"));
    fresh.write_document("rhizo version=1\n- \"Idea\" {\n    - \"Why\"\n}\n", None).unwrap();
    assert_eq!(fresh.overview().unwrap()["nodes"], 2);
    assert!(!dir.join("new.rhizo.tmp").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn guide_validate_brief_and_render() {
    let (w, dir) = ws("render", EXAMPLE);
    assert!(w.guide().contains("## Kits and their types"));
    let v = w.validate("rhizo version=2\nnode \"X\" id=x type=data.entity\n");
    assert_eq!(v["ok"], false, "data types need `use data`: {v}");
    assert_eq!(w.validate("rhizo version=1\n- \"Fine\"\n")["ok"], true);
    assert!(w.brief(false).unwrap().starts_with("# Build brief: Shop data model"));
    assert!(w.brief(true).unwrap().contains("\"rhizo-tasks\": 1"));
    match w.render(false, None).unwrap() {
        Rendered::Svg(svg) => assert!(svg.starts_with("<svg")),
        Rendered::Png(_) => panic!("asked for svg"),
    }
    match w.render(true, None).unwrap() {
        Rendered::Png(png) => assert_eq!(&png[1..4], b"PNG"),
        Rendered::Svg(_) => panic!("asked for png"),
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lists_and_serves_samples() {
    let (w, _) = ws("samples", "rhizo version=1\n- \"A\"\n");
    let list = w.samples(None).unwrap();
    let ids: Vec<&str> = list["samples"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"shop") && ids.contains(&"aws-infra"), "{ids:?}");
    let shop = w.samples(Some("shop")).unwrap();
    assert_eq!(shop["files"].as_array().unwrap().len(), 6);
    assert!(shop["files"][0]["text"].as_str().unwrap().starts_with("rhizo version="));
    assert!(w.samples(Some("nope")).is_err());
}

#[test]
fn serves_a_project_folder() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop");
    let dir = std::env::temp_dir().join(format!("rhizo-mcp-project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for rel in ["model.rhizo", "api.rhizo", "intent.rhizo", "flows/checkout.rhizo"] {
        let to = dir.join(rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(src.join(rel), to).unwrap();
    }
    let w = Workspace::new(dir.clone());
    let o = w.overview().unwrap();
    assert_eq!(o["files"].as_array().unwrap().len(), 4);
    let ops = w.list_nodes(Some("data.operation"), None, None, None).unwrap();
    assert_eq!(ops["nodes"][0]["file"], "api.rhizo");
    let order = w.get_node("order").unwrap();
    assert_eq!(order["file"], "model.rhizo");
    assert!(order["outgoing"].as_array().unwrap().iter().any(|e| e["to"] == "place-order"), "references are resolved");
    assert_eq!(w.check().unwrap()["ok"], true);
    let u = w.usages("order", None).unwrap();
    assert_eq!(u["file"], "model.rhizo");
    assert_eq!(u["files"], json!(["api.rhizo"]));
    assert_eq!(
        u["usages"][0],
        json!({ "file": "api.rhizo", "kind": "reference", "id": "model:order", "label": "Order", "line": null })
    );
    assert_eq!(u["usages"][1]["kind"], "edge");
    assert_eq!(u["usages"][1]["line"], 17);
    assert_eq!(w.usages("place-order", Some("api.rhizo")).unwrap()["files"], json!(["flows/checkout.rhizo"]));
    assert!(w.usages("nope", None).unwrap_err().contains("no node"));
    // Served one file: its folder is searched for files that use it.
    let one = Workspace::new(dir.join("model.rhizo"));
    assert_eq!(one.usages("order", None).unwrap()["files"], json!(["api.rhizo"]));
    assert!(w.render(true, None).unwrap_err().contains("say which `file`"));
    assert!(matches!(w.render(true, Some("flows/checkout.rhizo")).unwrap(), Rendered::Png(_)));
    assert!(w.brief(false).unwrap().contains("Implement the flow \"Checkout\""));

    // Writing needs a file inside the folder; new files are created.
    assert!(w.write_document("rhizo version=1\n- \"X\"\n", None).is_err());
    assert!(w.write_document("rhizo version=1\n- \"X\"\n", Some("../escape.rhizo")).is_err());
    let flow = "rhizo version=2\nimport \"../model.rhizo\" as=model\nlayout \"layered\"\nedge \"Refund\" model:order\n";
    w.write_document(flow, Some("flows/refund.rhizo")).unwrap();
    assert_eq!(w.overview().unwrap()["files"].as_array().unwrap().len(), 5);
    let check = w.check().unwrap();
    assert_eq!(check["ok"], true, "{check}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn adopt_refuses_without_writing() {
    let (w, dir) = ws("adopt", EXAMPLE);
    let err = w.adopt_nodes(&["mailer".into(), "nope".into()], false, &[], None).unwrap_err();
    assert!(err.starts_with("refusing to adopt: nothing was written"), "{err}");
    assert!(err.contains("no node `nope`") && err.contains("`mailer` is not in an imported environment"), "{err}");
    assert_eq!(std::fs::read_to_string(&w.path).unwrap(), EXAMPLE);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn var_files_stay_tfvars_inside_the_folder() {
    let (w, dir) = ws("vars", EXAMPLE);
    for bad in ["/etc/hosts", "../x.tfvars", "secrets.env", ".hidden/x.tfvars"] {
        let err = w.adopt_nodes(&["mailer".into()], false, &[bad.into()], None).unwrap_err();
        assert!(err.contains("give a .tfvars file inside the served folder"), "{bad}: {err}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hosts", dir.join("link.tfvars")).unwrap();
        let err = w.adopt_nodes(&["mailer".into()], false, &["link.tfvars".into()], None).unwrap_err();
        assert!(err.contains("is a symlink"), "{err}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// An agent can never allow OpenTofu providers: only `rhizo mcp
/// --allow-provider` (the person, at start) can, so `adopt_nodes` takes no
/// such argument and ignores one an agent sends.
#[test]
fn agents_cannot_allow_providers() {
    let schema = serde_json::to_value(schemars::schema_for!(AdoptArgs)).unwrap();
    assert!(schema["properties"].get("allow_providers").is_none(), "{schema}");
    let args: AdoptArgs =
        serde_json::from_value(json!({ "nodes": ["x"], "allow_providers": ["evil.example/x/y"] })).unwrap();
    assert_eq!(args.nodes, ["x"]);
    let w = Workspace::new(std::env::temp_dir());
    assert!(w.allow_providers.is_empty(), "nothing is allowed unless the person starts the server so");
}
