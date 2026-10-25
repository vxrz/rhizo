//! The viewer engine on projects and kits: no GPU needed.

use std::path::{Path, PathBuf};

use rhizo_app::Engine;
use rhizo_core::{LineStyle, NodeId, Shape};

fn shop(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop").join(file)
}

fn id(s: &str) -> NodeId {
    NodeId(s.to_string())
}

#[test]
fn references_draw_as_their_targets_and_know_where_they_point() {
    let built = Engine::default().build_file(&shop("api.rhizo"), None).expect("builds");
    assert!(built.warnings.is_none(), "{:?}", built.warnings.map(|w| w.message));
    let order = built.diagram.nodes.iter().find(|n| n.id.as_str() == "model:order").expect("drawn");
    assert_eq!(order.label, "Order");
    assert_eq!(order.line, LineStyle::Dashed);
    let (file, target) = &built.references[&id("model:order")];
    assert!(file.ends_with("model.rhizo"));
    assert_eq!(target, &id("order"));
    assert_eq!(built.deps.len(), 1);
    assert!(built.deps[0].ends_with("model.rhizo"));
}

#[test]
fn kit_problems_show_but_the_diagram_still_draws() {
    let dir = std::env::temp_dir().join(format!("rhizo-app-kits-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("api.rhizo");
    std::fs::write(
        &file,
        "rhizo version=2\nuse data version=\"0.1\"\nframe \"API\" type=data.api {\n    node \"Op\" id=op type=data.operation { props method=\"send\" path=\"/x\" }\n}\n",
    )
    .unwrap();
    let built = Engine::default().build_file(&file, None).expect("still draws");
    let p = built.warnings.expect("a problem");
    assert!(p.is_error());
    assert!(p.message.contains("prop `method`"), "{}", p.message);
    assert_eq!(p.at.map(|a| a.0), Some(4));
    // Kit default shapes apply: operations are pills.
    let op = built.diagram.nodes.iter().find(|n| n.id.as_str() == "op").unwrap();
    assert_eq!(op.shape, Shape::Pill);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_broken_reference_is_a_problem_and_still_draws() {
    let dir = std::env::temp_dir().join(format!("rhizo-app-refs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(shop("model.rhizo"), dir.join("model.rhizo")).unwrap();
    let file = dir.join("flow.rhizo");
    std::fs::write(&file, "rhizo version=2\nimport \"model.rhizo\" as=model\nedge \"Pay\" model:ordr\n").unwrap();
    let built = Engine::default().build_file(&file, None).expect("draws");
    let p = built.warnings.expect("the broken reference");
    assert_eq!(p.message, "`model:ordr`: model.rhizo has no node `ordr`");
    assert_eq!(p.help.as_deref(), Some("did you mean `model:order`?"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn exports_match_the_window_kit_looks_and_resolved_references() {
    let svg = |file: &str| {
        let built = Engine::default().build_file(&shop(file), None).expect("builds");
        String::from_utf8(rhizo_app::files::export(&built, rhizo_interop::Format::Svg, None).unwrap()).unwrap()
    };
    assert!(svg("model.rhizo").contains("rz-card"), "entities draw as cards, as in the window");
    let api = svg("api.rhizo");
    assert!(api.contains(">Order<") && !api.contains(">model:order<"), "references show their target's label");
    // JSON and YAML keep the file as written.
    let built = Engine::default().build_file(&shop("api.rhizo"), None).unwrap();
    let json = String::from_utf8(rhizo_app::files::export(&built, rhizo_interop::Format::Json, None).unwrap()).unwrap();
    assert!(json.contains("model:order") && !json.contains("\"decor\""), "{json}");
}
