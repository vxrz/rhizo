//! The owner's scenario, end to end, through what the viewer's commands and
//! inspector run (`rhizo_app::ops` and the `Editor`), with no typing in the
//! text: an empty file, a frame "Production" typed as an environment on
//! aws, a bucket and a function in it, the function writing to the bucket,
//! then compiled. The file must be what a person writes by hand, and check
//! clean.

use rhizo_app::inspector::{Input, Subject};
use rhizo_app::ops::{self, Owner, Spot};
use rhizo_app::palette::{self, Action, Selection};
use rhizo_core::FrameId;
use rhizo_kit::Kits;
use rhizo_lang::Editor;
use serde_json::json;

const BY_HAND: &str = r#"rhizo version=2
use infra version="0.1"
frame "Production" type=infra.environment {
    props cloud="aws" region="eu-central-1"
    node "Bucket" id=bucket type=infra.bucket
    node "Function" id=function type=infra.function
    edge function bucket {
        props access="write"
    }
}
"#;

/// The registry offers `action` for `selection`, as the menus and palette do.
fn offered(action: Action, selection: Selection) {
    assert!(
        palette::COMMANDS.iter().any(|c| c.action == action && c.needs.met_by(selection)),
        "{action:?} is not offered for {selection:?}"
    );
}

#[test]
fn the_owner_builds_a_production_environment_without_touching_text() {
    let kits = Kits::builtin();
    // 1. An empty file.
    let mut e = Editor::open("");

    // 2. Right-click the canvas ▸ New frame (here from the palette: no point), then type its label.
    offered(Action::NewFrame, Selection::Nothing);
    let made = e.add_frame(None, "New frame", None).unwrap();
    let prod = ops::name_new_frame(&mut e, &made, "Production").unwrap();
    assert_eq!(prod, FrameId::from("production"));

    // 3. Set frame type… lists the frame types; Environment is infra.environment.
    offered(Action::FrameTypeMenu, Selection::Frame);
    let types = ops::frame_types(&kits);
    let env = types.iter().find(|t| t.1 == "Environment").unwrap();
    assert_eq!(env.0, "infra.environment");
    ops::set_frame_type(&mut e, &kits, &prod, Some(env.0)).unwrap();

    // 4. The inspector shows the environment's props: cloud is a choice, region a text field.
    let m = ops::frame_model(e.document(), &kits, &prod, None).unwrap();
    assert_eq!(m.subject, Subject::Frame);
    let cloud = m.fields.iter().find(|f| f.key == "cloud").unwrap();
    assert!(matches!(&cloud.input, Input::Choice(v) if v.contains(&json!("aws"))));
    assert_eq!(m.fields.iter().find(|f| f.key == "region").unwrap().input, Input::Text);
    assert!(m.add_typed, "a typed frame offers Add typed node…");
    let owner = Owner::Frame(prod.clone());
    ops::set_prop(&mut e, &owner, None, "cloud", Some(json!("aws"))).unwrap();
    ops::set_prop(&mut e, &owner, None, "region", Input::Text.parse("eu-central-1").unwrap()).unwrap();

    // 5. Add typed node… inside it lists what fits an environment; Bucket and Function, labels typed.
    let fits = |label: &str| {
        kits.iter()
            .flat_map(|k| k.types.values())
            .find(|d| d.label == label && d.fits(ops::container_of_frame(e.document(), &prod)))
            .map(|d| d.name.clone())
            .unwrap_or_else(|| panic!("{label} does not fit an environment"))
    };
    let (bucket_ty, function_ty) = (fits("Bucket"), fits("Function"));
    let new = ops::add_typed(&mut e, &kits, &Spot::In(prod.clone()), &bucket_ty).unwrap();
    let bucket = e.rename_new(&new, "Bucket").unwrap();
    let new = ops::add_typed(&mut e, &kits, &Spot::In(prod.clone()), &function_ty).unwrap();
    let function = e.rename_new(&new, "Function").unwrap();

    // 6. Connect the function to the bucket; the edge's inspector offers access, set to write.
    offered(Action::Connect, Selection::Node);
    e.connect(&function, &bucket).unwrap();
    let k = e.document().edges.iter().position(|x| x.from == function && x.to == bucket).unwrap();
    let m = ops::edge_model(e.document(), k, None).unwrap();
    let access = m.fields.iter().find(|f| f.key == "access").unwrap();
    assert!(matches!(&access.input, Input::Choice(v) if v.contains(&json!("write"))));
    ops::set_prop(&mut e, &Owner::Edge(k), None, "access", Some(json!("write"))).unwrap();

    // The file is what a person writes by hand.
    assert_eq!(e.source(), BY_HAND);
    assert!(e.diagnostics().is_empty(), "{:#?}", e.diagnostics());

    // 7. Compile (Diagram ▸ Compile specs…), as the viewer does: the file on disk, as a project.
    let dir = std::env::temp_dir().join(format!("rhizo-scenario-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("prod.rhizo");
    std::fs::write(&path, e.source()).unwrap();
    let p = rhizo_project::Project::open(&path).unwrap();
    // `rhizo check`: no errors.
    let problems: Vec<_> = p.check(&kits).into_iter().flatten().filter(|d| d.is_error()).collect();
    assert!(problems.is_empty(), "{problems:#?}");
    let bundle = p
        .compile(&kits)
        .unwrap_or_else(|f| panic!("{:#?}", f.iter().map(|f| &f.diagnostic.message).collect::<Vec<_>>()));
    let files: Vec<&String> = bundle.files.keys().collect();
    assert!(files.iter().any(|f| f.starts_with("infra/production/")), "{files:?}");
    let iam =
        bundle.files.iter().find(|(k, _)| k.ends_with("iam.tf")).map(|(_, f)| f.contents.clone()).unwrap_or_default();
    assert!(iam.contains("s3:PutObject") && !iam.contains("s3:GetObject"), "access=write grants writing only:\n{iam}");
    std::fs::remove_dir_all(&dir).ok();
}
