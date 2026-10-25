//! The `rhizo` binary end to end: exit codes and what reaches stderr.

use std::path::Path;
use std::process::{Command, Output};

fn rhizo(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rhizo")).args(args).output().unwrap()
}

fn playground(file: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/playground").join(file).display().to_string()
}

#[test]
fn playground_files_check_clean_except_the_errors_one() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/playground");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name == "README.md" {
            continue;
        }
        let out = rhizo(&["check", &path.display().to_string()]);
        assert_eq!(out.status.success(), name != "08-errors.rhizo", "{name}: {}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn check_reports_kit_errors_at_the_statement() {
    let src = std::fs::read_to_string(playground("11-data-model.rhizo")).unwrap();
    let broken = src.replace(r#"method="post""#, r#"method="send""#);
    let dir = std::env::temp_dir().join(format!("rhizo-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("broken.rhizo");
    std::fs::write(&file, broken).unwrap();
    let out = rhizo(&["check", &file.display().to_string()]);
    std::fs::remove_dir_all(&dir).unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("operation `place-order`, prop `method`"), "{err}");
    assert!(err.contains(r#"node "Place order" id=place-order"#), "points at the node: {err}");
}

#[test]
fn types_lists_kits_and_shows_a_type() {
    let out = rhizo(&["types"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("data.operation  node   method path"));
    let out = rhizo(&["types", "data.field"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("inside: data.entity, data.event") && text.contains(r#""pii""#), "{text}");
    assert!(!rhizo(&["types", "nope"]).status.success());
}

#[test]
fn compile_writes_checks_and_cleans_the_bundle() {
    let dir = std::env::temp_dir().join(format!("rhizo-compile-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = std::fs::read_to_string(playground("11-data-model.rhizo")).unwrap();
    let doc = dir.join("shop.rhizo");
    std::fs::write(&doc, &src).unwrap();
    let out = dir.join("build");
    let (doc_s, out_s) = (doc.display().to_string(), out.display().to_string());

    assert!(rhizo(&["compile", &doc_s, "-o", &out_s]).status.success());
    assert!(out.join("specs/shop-api.openapi.yaml").exists());
    assert!(rhizo(&["compile", &doc_s, "-o", &out_s, "--check"]).status.success());

    // Drop the API frame: its spec is stale and goes away.
    let without_api = src.split("frame \"Shop API\"").next().unwrap().to_string();
    std::fs::write(&doc, &without_api).unwrap();
    assert!(!rhizo(&["compile", &doc_s, "-o", &out_s, "--check"]).status.success());
    assert!(rhizo(&["compile", &doc_s, "-o", &out_s]).status.success());
    let spec = out.join("specs/shop-api.openapi.yaml");
    assert!(!spec.exists(), "stale generated file removed");

    // Unless someone edited it by hand: then it stays, with a warning.
    std::fs::write(&doc, &src).unwrap();
    assert!(rhizo(&["compile", &doc_s, "-o", &out_s]).status.success());
    std::fs::write(&spec, "edited\n").unwrap();
    std::fs::write(&doc, &without_api).unwrap();
    let run = rhizo(&["compile", &doc_s, "-o", &out_s]);
    assert!(String::from_utf8_lossy(&run.stderr).contains("edited by hand"));
    assert_eq!(std::fs::read_to_string(&spec).unwrap(), "edited\n");

    // A file rhizo never generated is never touched.
    std::fs::write(out.join("NOTES.md"), "mine\n").unwrap();
    assert!(rhizo(&["compile", &doc_s, "-o", &out_s]).status.success());
    assert_eq!(std::fs::read_to_string(out.join("NOTES.md")).unwrap(), "mine\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn starters_check_clean_and_the_data_one_compiles() {
    let dir = std::env::temp_dir().join(format!("rhizo-new-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for kind in ["mindmap", "flow", "canvas", "data", "sequence", "workflow", "infra", "intent", "learn"] {
        let out = rhizo(&["new", kind]);
        assert!(out.status.success());
        let file = dir.join(format!("{kind}.rhizo"));
        std::fs::write(&file, &out.stdout).unwrap();
        let check = rhizo(&["check", &file.display().to_string()]);
        assert!(check.status.success(), "{kind}: {}", String::from_utf8_lossy(&check.stderr));
        assert!(!String::from_utf8_lossy(&check.stderr).contains("warning"), "{kind}");
    }
    let file = dir.join("data.rhizo").display().to_string();
    let build = dir.join("build").display().to_string();
    assert!(rhizo(&["compile", &file, "-o", &build]).status.success());
    assert!(dir.join("build/specs/api.openapi.yaml").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Speaks JSON-RPC to `rhizo mcp` over stdio, like an agent host.
#[test]
fn mcp_serves_tools_over_stdio() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_rhizo"))
        .args(["mcp", &playground("11-data-model.rhizo")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut call = |msg: serde_json::Value| -> serde_json::Value {
        writeln!(stdin, "{msg}").unwrap();
        if msg.get("id").is_none() {
            return serde_json::Value::Null;
        }
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };

    // 2025-11-25: initialize handshake.
    let init = call(serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } } }));
    assert_eq!(init["result"]["serverInfo"]["name"], "rhizo");
    call(serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    let tools = call(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let names: Vec<&str> =
        tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for t in [
        "overview",
        "list_nodes",
        "get_node",
        "neighbors",
        "check",
        "types",
        "compile_preview",
        "propose_change",
        "authoring_guide",
        "validate",
        "brief",
        "render",
    ] {
        assert!(names.contains(&t), "{t} missing from {names:?}");
    }
    assert!(!names.contains(&"write_document"), "read-only unless --allow-write");
    let node = call(serde_json::json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "get_node", "arguments": { "id": "place-order" } } }));
    assert_eq!(node["result"]["structuredContent"]["props"]["method"], "post");
    let missing = call(serde_json::json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "get_node", "arguments": { "id": "nope" } } }));
    assert_eq!(missing["result"]["isError"], true);

    // 2026-07-28: stateless, version in request metadata.
    let meta = serde_json::json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "0" },
        "io.modelcontextprotocol/clientCapabilities": {} });
    let found = call(
        serde_json::json!({ "jsonrpc": "2.0", "id": 5, "method": "server/discover", "params": { "_meta": meta } }),
    );
    assert!(found["result"]["supportedVersions"].as_array().unwrap().contains(&"2026-07-28".into()));
    let o = call(serde_json::json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call",
        "params": { "_meta": meta, "name": "overview", "arguments": {} } }));
    assert_eq!(o["result"]["structuredContent"]["title"], "Shop data model");

    drop(stdin);
    assert!(child.wait().unwrap().success());
}

/// An agent creates a diagram from nothing through `rhizo mcp --allow-write`.
#[test]
fn mcp_agent_can_author_a_new_diagram() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;

    let dir = std::env::temp_dir().join(format!("rhizo-mcp-author-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("pipeline.rhizo");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rhizo"))
        .args(["mcp", "--allow-write", &file.display().to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut call = |id: u32, name: &str, args: serde_json::Value| -> serde_json::Value {
        let meta = serde_json::json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "0" },
            "io.modelcontextprotocol/clientCapabilities": {} });
        let msg = serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "_meta": meta, "name": name, "arguments": args } });
        writeln!(stdin, "{msg}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let src = "rhizo version=1\ntitle \"Ingest\"\nlayout \"layered\"\nnode \"Kafka\" id=kafka\nnode \"Warehouse\" id=wh\nedge kafka wh\n";
    let w = call(1, "write_document", serde_json::json!({ "source": src }));
    assert_ne!(w["result"]["isError"], true, "{w}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), src);
    let img = call(2, "render", serde_json::json!({}));
    assert_eq!(img["result"]["content"][0]["type"], "image");
    assert_eq!(img["result"]["content"][0]["mimeType"], "image/png");
    let brief = call(3, "brief", serde_json::json!({}));
    assert!(brief["result"]["content"][0]["text"].as_str().unwrap().contains("# Build brief: Ingest"));
    drop(stdin);
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn guide_prints() {
    let out = rhizo(&["guide"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("# Writing rhizo diagrams"));
}

#[test]
fn about_prints_two_lines_and_version_stays_one() {
    let out = rhizo(&["about"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].starts_with("Made with ❤️ in Erlangen, Germany · Metropolregion Nürnberg · Designed & built"));
    assert!(lines[1].contains("(MIT or Apache-2.0) · https://github.com/vxrz/rhizo · rhizo "));
    let version = rhizo(&["--version"]);
    assert_eq!(String::from_utf8_lossy(&version.stdout).lines().count(), 1);
}

#[test]
fn a_project_folder_checks_compiles_and_briefs_as_one() {
    let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop").display().to_string();
    let check = rhizo(&["check", &shop]);
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    assert!(String::from_utf8_lossy(&check.stderr).contains("(6 files,"));
    let dir = std::env::temp_dir().join(format!("rhizo-project-cli-{}", std::process::id()));
    let out = dir.display().to_string();
    assert!(rhizo(&["compile", &shop, "-o", &out]).status.success());
    // The API file's operations got the model file's schemas.
    let api = std::fs::read_to_string(dir.join("specs/shop-api.openapi.yaml")).unwrap();
    assert!(api.contains("$ref: '#/components/schemas/Order'"));
    assert!(dir.join("specs/schemas/order.schema.json").exists());
    assert!(dir.join("flows/node-red/order-intake.json").exists());
    // The infra file's function implements the API file's operations: they are its routes.
    let main = std::fs::read_to_string(dir.join("infra/prod/main.tf")).unwrap();
    assert!(main.contains("\"POST /orders\" = {") && main.contains("\"GET /orders\" = {"));
    let brief = String::from_utf8(rhizo(&["brief", &shop]).stdout).unwrap();
    assert!(brief.contains("Implement the flow \"Checkout\"") && brief.contains("Decide: Payments provider"));
    assert!(brief.contains("Implement the function \"Orders function\"") && brief.contains("(`T-prod-apply`)"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn compile_never_writes_through_a_symlink_out_of_the_bundle() {
    let dir = std::env::temp_dir().join(format!("rhizo-symlink-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (out, elsewhere) = (dir.join("build"), dir.join("elsewhere"));
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, out.join("specs")).unwrap();
    let doc = dir.join("m.rhizo");
    std::fs::write(&doc, "rhizo version=2\nuse data version=\"0.1\"\nnode \"A\" id=a type=data.entity\n").unwrap();
    let run = rhizo(&["compile", &doc.display().to_string(), "-o", &out.display().to_string()]);
    assert!(!run.status.success());
    assert!(String::from_utf8_lossy(&run.stderr).contains("refusing to touch"));
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0, "nothing written through the link");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Anything `check` accepts must compile (and brief) without crashing:
/// every example and fixture, plus shapes that once panicked.
#[test]
fn whatever_checks_also_compiles() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut inputs: Vec<std::path::PathBuf> = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rhizo") {
                out.push(p);
            }
        }
    }
    walk(&root.join("examples"), &mut inputs);
    walk(&root.join("crates/cli/tests/fixtures"), &mut inputs);
    let dir = std::env::temp_dir().join(format!("rhizo-checks-compile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A flow frame and a sequence frame holding only goals: nothing to build.
    for (name, src) in [
        (
            "only-goals-flow.rhizo",
            "rhizo version=2\nframe \"F\" type=rhizo.flow {\n    node \"G\" id=g type=rhizo.goal {\n        - \"Step\"\n    }\n}\n",
        ),
        (
            "only-goals-seq.rhizo",
            "rhizo version=2\nframe \"S\" layout=sequence {\n    node \"G\" id=g type=rhizo.goal\n}\n",
        ),
    ] {
        let p = dir.join(name);
        std::fs::write(&p, src).unwrap();
        inputs.push(p);
    }
    for input in &inputs {
        let path = input.display().to_string();
        if !rhizo(&["check", &path]).status.success() {
            continue; // e.g. the deliberate-errors example
        }
        let brief = rhizo(&["brief", &path]);
        assert!(brief.status.success(), "{path}: {}", String::from_utf8_lossy(&brief.stderr));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Author text that reaches generated code only ever lands in comments:
/// a line break in it must not start real HCL or JavaScript.
#[test]
fn author_text_cannot_inject_code() {
    let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop");
    let dir = std::env::temp_dir().join(format!("rhizo-inject-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("flows")).unwrap();
    for f in ["model.rhizo", "api.rhizo", "intent.rhizo", "flows/checkout.rhizo"] {
        std::fs::copy(shop.join(f), dir.join(f)).unwrap();
    }
    let infra = std::fs::read_to_string(shop.join("infra.rhizo")).unwrap().replace(
        r#"access="write" prefix="receipts/""#,
        r#"access="write" prefix="x\nstatement {\n actions = [\"*\"]\n resources = [\"*\"]\n}\n#""#,
    );
    std::fs::write(dir.join("infra.rhizo"), infra).unwrap();
    let flow = std::fs::read_to_string(shop.join("flows/order-intake.rhizo")).unwrap().replace(
        r#"node "Score fraud risk" id=score type=flow.script {"#,
        "node \"Score\\nrequire('child_process').execSync('touch /tmp/pwn');//\" id=score type=flow.script {\n        accept \"ok\\nglobal.process.exit(1)\"",
    );
    std::fs::write(dir.join("flows/order-intake.rhizo"), flow).unwrap();
    let out = dir.join("build");
    let run = rhizo(&["compile", &dir.display().to_string(), "-o", &out.display().to_string()]);
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let iam = std::fs::read_to_string(out.join("infra/prod/iam.tf")).unwrap();
    assert!(!iam.lines().any(|l| l.trim_start().starts_with("actions = [\"*\"]")), "{iam}");
    let red = std::fs::read_to_string(out.join("flows/node-red/order-intake.json")).unwrap();
    let nodes: serde_json::Value = serde_json::from_str(&red).unwrap();
    let func = nodes.as_array().unwrap().iter().find_map(|n| n["func"].as_str()).unwrap();
    for line in func.lines() {
        let l = line.trim_start();
        assert!(!l.starts_with("require(") && !l.starts_with("global."), "live code from author text: {line}");
    }
    // Ids with line breaks are refused outright.
    std::fs::write(dir.join("bad.rhizo"), "rhizo version=1\nnode \"A\" id=\"fn\\nresource\"\n").unwrap();
    let bad = rhizo(&["check", &dir.join("bad.rhizo").display().to_string()]);
    assert!(String::from_utf8_lossy(&bad.stderr).contains("control characters"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn new_project_writes_a_working_project() {
    let dir = std::env::temp_dir().join(format!("rhizo-new-project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.display().to_string();
    assert!(rhizo(&["new", "project", &d]).status.success());
    let check = rhizo(&["check", &d]);
    let err = String::from_utf8_lossy(&check.stderr);
    assert!(check.status.success() && err.contains("(4 files,") && !err.contains("warning"), "{err}");
    let brief = String::from_utf8(rhizo(&["brief", &d]).stdout).unwrap();
    assert!(brief.contains("Items can be managed online") && brief.contains("Implement the sequence"), "{brief}");
    assert!(!rhizo(&["new", "project", &d]).status.success(), "never writes into a non-empty folder");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn samples_list_and_copy() {
    let dir = std::env::temp_dir().join(format!("rhizo-samples-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let list = String::from_utf8(rhizo(&["samples"]).stdout).unwrap();
    assert!(list.contains("Infrastructure") && list.contains("shop") && list.contains("aws-infra"), "{list}");
    let shop = dir.join("shop").display().to_string();
    assert!(rhizo(&["samples", "copy", "shop", &shop]).status.success());
    let check = rhizo(&["check", &shop]);
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    assert!(!rhizo(&["samples", "copy", "shop", &shop]).status.success(), "never writes into a non-empty folder");
    let d = dir.display().to_string();
    assert!(rhizo(&["samples", "copy", "flowchart", &d]).status.success(), "into a folder");
    assert!(dir.join("flowchart.rhizo").exists());
    assert!(!rhizo(&["samples", "copy", "nope", &d]).status.success());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn proposals_list_show_apply() {
    let dir = std::env::temp_dir().join(format!("rhizo-proposals-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.display().to_string();
    assert!(rhizo(&["new", "project", &d]).status.success());
    let kits = rhizo_kit::Kits::builtin();
    rhizo_project::proposals::propose(
        &dir,
        "model.rhizo",
        "node \"name\" id=item-name type=data.field { props required=#true max=200 }",
        "node \"name\" id=item-name type=data.field { props required=#true max=200 }\n    node \"price\" id=item-price type=data.field { props type=\"number\" min=0 }",
        "Items need a price.",
        &kits,
    )
    .unwrap();
    let list = String::from_utf8(rhizo(&["proposals", &d]).stdout).unwrap();
    assert!(list.contains("open") && list.contains("Items need a price."), "{list}");
    assert!(String::from_utf8(rhizo(&["proposals", &d, "--show", "1"]).stdout).unwrap().contains("item-price"));
    assert!(rhizo(&["proposals", &d, "--apply", "1"]).status.success());
    assert!(std::fs::read_to_string(dir.join("model.rhizo")).unwrap().contains("item-price"));
    assert!(rhizo(&["check", &d]).status.success());
    assert!(!rhizo(&["proposals", &d, "--apply", "1"]).status.success(), "already applied");
    std::fs::remove_dir_all(&dir).unwrap();
}
