//! Typed documents draw what they mean: the kit's look reaches the SVG as
//! badges, subtitles, cards and rows, with stable classes.

use std::path::Path;

use rhizo_core::Theme;
use rhizo_interop::Format;
use rhizo_kit::Kits;
use rhizo_layout::Heuristic;

fn svg(example: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(example);
    let src = std::fs::read_to_string(path).unwrap();
    let (mut doc, _) = rhizo_interop::import(&src, Format::Rhizo).expect("example parses");
    Kits::builtin().apply_style_defaults(&mut doc);
    let d = rhizo_layout::layout(&doc, &Theme::light(), &Heuristic);
    rhizo_render::to_svg(&d, &Default::default())
}

#[test]
fn operations_show_method_and_path() {
    let s = svg("shop/api.rhizo");
    assert!(s.contains(r#"class="rz-node rz-shape-pill rz-depth-0 rz-type-data-operation" id="node-place-order""#));
    assert!(s.contains(r#"<g class="rz-badge">"#) && s.contains(">POST</text>") && s.contains(">GET</text>"));
    assert!(s.contains(r#"<text class="rz-subtitle""#) && s.contains(">/orders</text>"));
}

#[test]
fn entities_are_cards_with_field_rows() {
    let s = svg("shop/model.rhizo");
    assert!(s.contains("rz-type-data-entity rz-card"));
    assert!(s.contains(r#"<path class="rz-card-header""#));
    assert!(s.contains(r#"rz-type-data-field rz-row" id="node-customer-email""#));
    assert!(s.contains(r#"<g class="rz-marker">"#) && s.contains(">PII</text>"));
    assert!(s.contains(r#"<text class="rz-detail""#) && s.contains(">uuid</text>"));
    // Fields are rows, not boxes on branches.
    assert!(!s.contains("rz-branch"));
}

#[test]
fn decisions_show_their_status() {
    let s = svg("shop/intent.rhizo");
    assert!(s.contains("rz-type-rhizo-goal"));
    assert!(s.contains(">ACCEPTED</text>"));
    assert!(s.contains(r#"rz-type-rhizo-decision rz-state-draft" id="node-payments""#));
}

#[test]
fn vpcs_contain_their_subnets() {
    let s = svg("shop/infra.rhizo");
    assert!(s.contains(r#"rz-type-aws-vpc rz-container" id="node-net""#));
    assert!(s.contains(">LAMBDA</text>"));
}

#[test]
fn typed_svg_is_deterministic() {
    assert_eq!(svg("shop/infra.rhizo"), svg("shop/infra.rhizo"));
    assert_eq!(svg("playground/11-data-model.rhizo"), svg("playground/11-data-model.rhizo"));
}
