use std::path::PathBuf;

use rhizo_core::{LineStyle, NodeId};

use super::*;

const MODEL: &str = r#"rhizo version=2
title "Domain model"
use data version="0.1"
node "Customer" id=customer type=data.entity {
    node "email" id=customer-email type=data.field { props format="email" required=#true }
}
node "Order" id=order type=data.entity {
    node "total" id=order-total type=data.field { props type="number" required=#true }
}
edge order customer kind=data label="placed by"
node "Notes" id=notes
"#;

const API: &str = r#"rhizo version=2
title "Shop API"
use data version="0.1"
import "model.rhizo" as=model
frame "Shop API" id=api type=data.api {
    props version="1.0.0"
    node "Place order" id=place type=data.operation { props method="post" path="/orders" }
}
edge model:order place kind=data
edge place model:order kind=data label="201"
node "Notes" id=notes
"#;

const FLOW: &str = r#"rhizo version=2
import "../model.rhizo"
node "The order" id=model:order
- "Checkout" {
    - "Pay"
}
edge pay model:order
"#;

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rhizo-project-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, src) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, src).unwrap();
    }
    dir
}

fn id(s: &str) -> NodeId {
    NodeId(s.to_string())
}

#[test]
fn a_folder_merges_into_one_model() {
    let dir = project("merge", &[("model.rhizo", MODEL), ("api.rhizo", API), ("flows/checkout.rhizo", FLOW)]);
    let p = Project::load_dir(&dir).unwrap();
    let rels: Vec<&str> = p.files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(rels, ["api.rhizo", "flows/checkout.rhizo", "model.rhizo"]);
    let all: Vec<Vec<String>> =
        p.check(&Kits::builtin()).iter().map(|d| d.iter().map(|x| x.message.clone()).collect()).collect();
    assert!(all.iter().all(Vec::is_empty), "{all:#?}");

    let m = &p.merged;
    // References are gone; edges point at the real entity.
    assert!(!m.nodes.keys().any(|k| k.as_str().contains(':')), "{:?}", m.nodes.keys().collect::<Vec<_>>());
    assert!(m.edges.iter().any(|e| e.from == id("order") && e.to == id("place")));
    assert!(m.edges.iter().any(|e| e.from == id("pay") && e.to == id("order")));
    // Two files define `notes`: both get qualified ids.
    assert!(m.nodes.contains_key(&id("api.notes")) && m.nodes.contains_key(&id("model.notes")));
    assert_eq!(p.origin(&id("model.notes")), Some((2, &id("notes"))));
    // Each file's loose nodes become a frame named after it.
    assert!(m.frames.iter().any(|f| f.label.as_deref() == Some("Domain model") && !f.implicit));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn references_draw_as_their_target() {
    let dir = project("display", &[("model.rhizo", MODEL), ("flows/checkout.rhizo", FLOW)]);
    let p = Project::load_file(&dir.join("flows/checkout.rhizo"));
    assert_eq!(p.files.len(), 2, "imports are followed");
    let d = p.display(0);
    let placed = &d.nodes[&id("model:order")];
    assert_eq!(placed.label, "The order", "an explicit label wins");
    assert_eq!(placed.ty.as_ref().unwrap().as_str(), "data.entity");
    assert_eq!(placed.style.line, Some(LineStyle::Dashed));
    assert_eq!(p.target_of(0, &id("model:order")), Some((1, &id("order"))));
    assert_eq!(list(&dir), ["flows/checkout.rhizo", "model.rhizo"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn broken_references_are_reported_where_written() {
    let api = API
        .replace("edge model:order place kind=data", "edge model:ordr place kind=data")
        .replace("edge place model:order kind=data label=\"201\"", "edge place model:customer kind=control");
    let dir = project(
        "broken",
        &[
            ("model.rhizo", MODEL),
            ("api.rhizo", &api),
            ("other.rhizo", "rhizo version=2\nimport \"gone.rhizo\" as=gone\n"),
        ],
    );
    let p = Project::load_dir(&dir).unwrap();
    let diags = p.check(&Kits::builtin());
    let api_msgs: Vec<String> = diags[0].iter().map(|d| d.message.clone()).collect();
    assert!(api_msgs.contains(&"`model:ordr`: model.rhizo has no node `ordr`".to_string()), "{api_msgs:#?}");
    let help = diags[0].iter().find(|d| d.message.contains("ordr")).unwrap().help.clone();
    assert_eq!(help.as_deref(), Some("did you mean `model:order`?"));
    // A cross-file edge is checked with the target's type: entities only take data edges.
    let kind = diags[0].iter().find(|d| d.message.contains("control edge")).expect("kind checked across files");
    let (line, _) = kind.line_col(&p.files[0].src).unwrap();
    assert_eq!(line, 10, "points at the edge in api.rhizo");
    assert_eq!(diags[2][0].message, "import `gone`: cannot read `gone.rhizo`");
    assert!(diags[2][0].span.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_folder_is_named_after_itself_however_it_is_written() {
    let dir = project("named", &[("model.rhizo", MODEL), ("api.rhizo", API)]);
    std::fs::create_dir(dir.join("sub")).unwrap();
    let name = dir.file_name().unwrap().to_string_lossy().to_string();
    assert_eq!(Project::load_dir(&dir).unwrap().name(), name);
    assert_eq!(Project::load_dir(&dir.join("sub").join("..")).unwrap().name(), name);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_single_file_is_its_own_project() {
    let dir = project("single", &[("model.rhizo", MODEL)]);
    let p = Project::load_file(&dir.join("model.rhizo"));
    assert!(p.is_single());
    assert_eq!(p.merged, p.files[0].doc);
    assert_eq!(p.name(), "model.rhizo");
    assert_eq!(p.sources(), MODEL);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn paths_normalize() {
    assert_eq!(normalize("api.rhizo", "model.rhizo"), "model.rhizo");
    assert_eq!(normalize("flows/a.rhizo", "../model.rhizo"), "model.rhizo");
    assert_eq!(normalize("a/b/c.rhizo", "./d/../e.rhizo"), "a/b/e.rhizo");
    assert_eq!(normalize("a.rhizo", "../outside.rhizo"), "../outside.rhizo");
}

#[test]
fn imports_stay_rhizo_files_and_inside_a_served_folder() {
    let dir = project(
        "confined",
        &[("inner/a.rhizo", "rhizo version=2\nimport \"../../outside.rhizo\" as=o\nimport \"../notes.txt\" as=n\n")],
    );
    std::fs::write(dir.parent().unwrap().join("outside.rhizo"), "rhizo version=1\n- \"Secret\"\n").unwrap();
    let p = Project::load_dir(&dir.join("inner")).unwrap();
    assert_eq!(p.files.len(), 1, "nothing outside was read");
    let msgs: Vec<String> = p.check(&Kits::builtin())[0].iter().map(|d| d.message.clone()).collect();
    assert!(msgs.contains(&"import `o`: `../../outside.rhizo` is outside the project folder".to_string()), "{msgs:?}");
    assert!(msgs.contains(&"import `n`: `../notes.txt` is not a .rhizo or .kdl file".to_string()), "{msgs:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_served_folder_never_reads_through_links_out_of_it() {
    let dir = project(
        "linked",
        &[("a.rhizo", "rhizo version=1\n- \"Mine\"\n"), ("sub/b.rhizo", "rhizo version=1\n- \"B\"\n")],
    );
    let secret = dir.with_extension("secret.rhizo");
    std::fs::write(&secret, "rhizo version=1\n- \"Secret\"\n").unwrap();
    std::os::unix::fs::symlink(&secret, dir.join("leak.rhizo")).unwrap();
    std::os::unix::fs::symlink(dir.join("sub/b.rhizo"), dir.join("also-b.rhizo")).unwrap();
    let p = Project::load_dir(&dir).unwrap();
    assert!(p.files.iter().all(|f| !f.src.contains("Secret")), "the linked file was read");
    let leak = p.files.iter().find(|f| f.rel == "leak.rhizo").unwrap();
    assert!(leak.load[0].message.contains("outside the project folder"), "{:?}", leak.load);
    assert!(p.files.iter().any(|f| f.rel == "also-b.rhizo" && f.src.contains("\"B\"")), "a link inside still reads");
    assert!(super::importable("C:/x.rhizo", true).is_err());
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_file(secret).unwrap();
}

#[cfg(unix)]
#[test]
fn atomic_writes_ignore_planted_links_and_keep_links_and_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = project("atomic", &[]);
    std::fs::create_dir_all(&dir).unwrap();
    let victim = dir.join("victim.txt");
    std::fs::write(&victim, "precious").unwrap();
    let doc = dir.join("doc.rhizo");
    std::fs::write(&doc, "old").unwrap();
    std::fs::set_permissions(&doc, std::fs::Permissions::from_mode(0o600)).unwrap();
    // Links planted at old-style temp names are never followed.
    std::os::unix::fs::symlink(&victim, dir.join("doc.rhizo.tmp")).unwrap();
    std::os::unix::fs::symlink(&victim, dir.join(".doc.rhizo.rhizo-tmp")).unwrap();
    write_atomic(&doc, b"new").unwrap();
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
    assert_eq!(std::fs::read_to_string(&doc).unwrap(), "new");
    assert_eq!(std::fs::metadata(&doc).unwrap().permissions().mode() & 0o777, 0o600);
    // A document that is a link stays a link; its target gets the text.
    let real = dir.join("real.rhizo");
    std::fs::write(&real, "a").unwrap();
    let link = dir.join("link.rhizo");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    write_atomic(&link, b"b").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "b");
    assert!(read_source(Path::new("/dev/zero")).is_err(), "devices are not diagrams");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn merged_ids_never_collide() {
    let dir = project(
        "clash",
        &[
            ("a.rhizo", "rhizo version=1\nnode \"X from a\" id=x\n"),
            ("b.rhizo", "rhizo version=1\nnode \"X from b\" id=x\n"),
            ("c.rhizo", "rhizo version=1\nnode \"Sneaky\" id=\"a.x\"\n"),
        ],
    );
    let p = Project::load_dir(&dir).unwrap();
    let labels: Vec<&str> = p.merged.nodes.values().map(|n| n.label.as_str()).collect();
    assert_eq!(p.merged.nodes.len(), 3, "{labels:?}");
    for l in ["X from a", "X from b", "Sneaky"] {
        assert!(labels.contains(&l), "{l} lost: {labels:?}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn usages_list_references_and_their_edges_in_other_files() {
    let dir = project("usages", &[("model.rhizo", MODEL), ("api.rhizo", API), ("flows/checkout.rhizo", FLOW)]);
    let p = Project::load_dir(&dir).unwrap();
    let u = p.usages("model.rhizo", &id("order"));
    let rows: Vec<(&str, &Used, &str)> =
        u.iter().map(|u| (u.file.as_str(), &u.node_or_edge, u.label.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("api.rhizo", &Used::Node(id("model:order")), "Order"),
            ("api.rhizo", &Used::Edge(0), "Order → Place order"),
            ("api.rhizo", &Used::Edge(1), "Place order → Order “201”"),
            ("flows/checkout.rhizo", &Used::Node(id("model:order")), "The order"),
            ("flows/checkout.rhizo", &Used::Edge(0), "Pay → The order"),
        ]
    );
    // A reference made only by edges has no statement; one declared as a node has.
    assert_eq!(u[0].span, None);
    let (at, _) = u[3].span.unwrap();
    assert!(p.files[1].src[at..].starts_with("node \"The order\""));
    assert!(u[1].span.is_some());
    assert!(p.usages("model.rhizo", &id("customer")).is_empty(), "nobody refers to customers");
    assert!(p.usages("model.rhizo", &id("nope")).is_empty());
    assert!(p.usages("gone.rhizo", &id("order")).is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_root_holds_imports_and_importers() {
    let dir = project(
        "root",
        &[
            ("shop/model.rhizo", MODEL),
            ("shop/flows/checkout.rhizo", FLOW),
            ("shop/top.rhizo", "rhizo version=2\nimport \"deep/a/b.rhizo\" as=b\nedge b:x \"Y\"\n"),
            ("shop/deep/a/b.rhizo", "rhizo version=1\nnode \"X\" id=x\n"),
            ("shop/lone/alone.rhizo", "rhizo version=1\n- \"Alone\"\n"),
        ],
    );
    let shop = dir.join("shop");
    let root = |rel: &str, home: &Path| root_within(&shop.join(rel), Some(home));
    assert_eq!(root("flows/checkout.rhizo", &dir), shop, "up to what it imports");
    assert_eq!(root("model.rhizo", &dir), shop, "its importers are in its own folder");
    assert_eq!(root("deep/a/b.rhizo", &dir), shop, "up to the file that imports it");
    assert_eq!(root("lone/alone.rhizo", &dir), shop.join("lone"), "nothing related: its own folder");
    assert_eq!(root("deep/a/b.rhizo", &shop.join("deep")), shop.join("deep/a"), "never above home");
    assert_eq!(root_of(&shop.join("lone/alone.rhizo")), shop.join("lone"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn import_paths_are_relative_to_the_importing_file() {
    assert_eq!(import_path("flows/checkout.rhizo", "model.rhizo"), "../model.rhizo");
    assert_eq!(import_path("api.rhizo", "model.rhizo"), "model.rhizo");
    assert_eq!(import_path("a/b/c.rhizo", "a/d/e.rhizo"), "../d/e.rhizo");
    assert_eq!(import_path("x.rhizo", "flows/y.rhizo"), "flows/y.rhizo");
    assert_eq!(import_path("a/b/c.rhizo", "z.rhizo"), "../../z.rhizo");
    let lines = "rhizo version=2\n  import \"../m.rhizo\" as=m\nimport \"x.kdl\"\nimported \"no\"\n";
    assert_eq!(import_paths(lines).collect::<Vec<_>>(), ["../m.rhizo", "x.kdl"]);
}

#[test]
fn open_reads_folders_files_with_imports_and_other_formats() {
    let dir = project(
        "open",
        &[("model.rhizo", MODEL), ("flows/checkout.rhizo", FLOW), ("notes.md", "# Plan\n- Why\n- How\n")],
    );
    assert_eq!(Project::open(&dir).unwrap().files.len(), 2, "a folder: its rhizo files");
    let flow = Project::open(&dir.join("flows/checkout.rhizo")).unwrap();
    let rels: Vec<&str> = flow.files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(rels, ["checkout.rhizo", "../model.rhizo"], "a file and what it imports");

    let notes = Project::open(&dir.join("notes.md")).unwrap();
    assert!(notes.is_single());
    assert_eq!(notes.files[0].format, Format::Markdown);
    assert!(notes.merged.nodes.values().any(|n| n.label == "How"));
    assert!(notes.check(&Kits::builtin())[0].is_empty());
    assert_eq!(notes.name(), "notes.md");
    assert!(Project::open(&dir.join("a.svg")).err().unwrap().contains("rhizo cannot read"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn text_in_hand_wins_over_the_file_on_disk() {
    let dir = project("text", &[("model.rhizo", MODEL), ("flows/checkout.rhizo", FLOW)]);
    let edited = FLOW.replace("\"The order\"", "\"Edited\"");
    let p = Project::with_text(&dir.join("flows/checkout.rhizo"), edited.clone(), Format::Rhizo);
    assert_eq!(p.files[0].src, edited);
    assert_eq!(p.files.len(), 2, "imports still come from disk");
    assert_eq!(p.display(0).nodes[&id("model:order")].label, "Edited");

    // Text that is no file follows no imports and so cannot miss them.
    let t = Project::open_text("flow.rhizo", &edited, Format::Rhizo);
    assert_eq!(t.files.len(), 1);
    assert_eq!(t.merged, t.files[0].doc);
    assert!(!t.check(&Kits::builtin())[0].iter().any(Diagnostic::is_error));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn drawable_is_display_plus_kit_looks() {
    let dir = project("drawable", &[("model.rhizo", MODEL), ("api.rhizo", API)]);
    let p = Project::load_file(&dir.join("api.rhizo"));
    let d = p.drawable(0, &Kits::builtin());
    let mut looks = p.display(0);
    Kits::builtin().apply_style_defaults(&mut looks);
    assert_eq!(d, looks);
    assert_eq!(d.nodes[&id("model:order")].label, "Order");
    assert!(d.decor.get(&id("model:order")).is_some_and(|x| x.card), "an entity draws as a card");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn kit_problems_in_json_and_yaml_point_at_the_node() {
    let kdl = "rhizo version=2\nuse data version=\"0.1\"\nframe \"API\" id=api type=data.api {\n    node \"Op\" id=op type=data.operation { props method=\"send\" path=\"/x\" }\n}\n";
    let doc = rhizo_lang::load(kdl, rhizo_lang::Syntax::Kdl).document;
    for (format, src) in [(Format::Json, rhizo_lang::to_json(&doc)), (Format::Yaml, rhizo_lang::to_yaml(&doc))] {
        let p = Project::open_text("api", &src, format);
        let diags = p.check(&Kits::builtin()).swap_remove(0);
        let d = diags.iter().find(|d| d.message.contains("prop `method`")).expect("the kit finding");
        let (at, len) = d.span.unwrap_or_else(|| panic!("{format:?}: no span"));
        assert!(src[at..at + len].eq_ignore_ascii_case("op"), "{format:?}: {}", &src[at..at + len]);
    }
}
