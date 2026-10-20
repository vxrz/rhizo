use rhizo_core::{Arrow, Document, LayoutKind, LineStyle, NodeId, Shape, Theme};
use rhizo_layout::Heuristic;

use crate::{Format, export, import};

const SAMPLE: &str = r##"
title "Launch"
style "risk" fill="#fee2e2" stroke="#ef4444"
- "Launch" {
    - "Users" { - "Personas"; - "Jobs" }
    - "Pricing" class=risk note="Needs a decision" link="https://example.com/pricing"
}
frame "Release" layout=layered direction=down {
    node "Build" shape=pill
    node "Tests pass?" id=check shape=diamond
    edge build check "Ship"
    edge check build label="no" line=dashed
}
edge pricing check arrow=both
"##;

fn sample() -> Document {
    import(SAMPLE, Format::Rhizo).unwrap().0
}

fn out(doc: &Document, f: Format) -> String {
    export(doc, f, &Theme::light(), &Heuristic).unwrap()
}

fn id(s: &str) -> NodeId {
    NodeId::from(s)
}

#[test]
fn mermaid_flowchart_import() {
    let src = r#"
---
title: Checkout
---
flowchart LR
    %% comment
    A[Cart] --> B{Logged in?}
    B -->|yes| C([Pay])
    B -- no --> D(Login) -.-> B
    C & D --> E((Done)):::ok
    subgraph pay [Payment]
        direction TB
        P1["Card #quot;Visa#quot;"] ==> P2
    end
    classDef ok fill:#dcfce7,stroke:#16a34a
    style A fill:#eee
    linkStyle 0 stroke:#f00
    click C href "https://pay.example"
"#;
    let (doc, _) = import(src, Format::Mermaid).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Checkout"));
    assert_eq!(doc.nodes.len(), 7);
    assert_eq!(doc.nodes[&id("B")].style.shape, Some(Shape::Diamond));
    assert_eq!(doc.nodes[&id("C")].style.shape, Some(Shape::Pill));
    assert_eq!(doc.nodes[&id("C")].link.as_deref(), Some("https://pay.example"));
    assert_eq!(doc.nodes[&id("P1")].label, "Card \"Visa\"");
    assert_eq!(doc.nodes[&id("P1")].frame.as_str(), "pay");
    assert_eq!(doc.nodes[&id("E")].class, vec!["ok"]);
    assert_eq!(doc.edges.len(), 7);
    assert_eq!(doc.edges[1].label.as_deref(), Some("yes"));
    assert_eq!(doc.edges[2].label.as_deref(), Some("no"));
    assert_eq!(doc.edges[3].line(), LineStyle::Dashed);
    assert_eq!(doc.edges[0].style.stroke.unwrap().to_string(), "#ff0000");
    assert_eq!(doc.edges[6].style.stroke_width, Some(3.0));
}

#[test]
fn mermaid_mindmap_import() {
    let src = "mindmap\n  root((Product))\n    Users\n      Personas\n      ::icon(fa fa-user)\n    id2[Pricing]\n      Tiers\n";
    let (doc, _) = import(src, Format::Mermaid).unwrap();
    assert_eq!(doc.nodes.len(), 5);
    let root = doc.nodes.values().next().unwrap();
    assert_eq!(root.label, "Product");
    assert_eq!(root.children.len(), 2);
    assert_eq!(doc.frames[0].layout, LayoutKind::Mindmap);
}

#[test]
fn mermaid_state_import() {
    let src = "stateDiagram-v2\n    [*] --> Idle\n    Idle --> Running : start\n    Running --> Idle : stop\n    Running --> [*]\n";
    let (doc, _) = import(src, Format::Mermaid).unwrap();
    assert_eq!(doc.edges.len(), 4);
    assert_eq!(doc.edges[1].label.as_deref(), Some("start"));
}

#[test]
fn mermaid_roundtrip_keeps_structure() {
    let doc = sample();
    let mmd = out(&doc, Format::Mermaid);
    assert!(mmd.contains("flowchart LR"), "the first frame is a mindmap, so LR:\n{mmd}");
    assert!(mmd.contains("subgraph release[\"Release\"]"));
    assert!(mmd.contains("check{\"Tests pass?\"}"));
    let (back, _) = import(&mmd, Format::Mermaid).unwrap();
    assert_eq!(back.nodes.len(), doc.nodes.len());
    let nested: usize = doc.nodes.values().map(|n| n.children.len()).sum();
    assert_eq!(back.edges.len(), doc.edges.len() + nested);
    assert!(back.edges.iter().any(|e| e.arrow == Arrow::Both));
}

#[test]
fn mermaid_mindmap_export() {
    let (doc, _) = import("- \"Root\" {\n - \"A (1)\"\n - \"B\" { - \"C\" }\n}", Format::Rhizo).unwrap();
    let mmd = out(&doc, Format::Mermaid);
    assert!(mmd.starts_with("mindmap\n"));
    let (back, _) = import(&mmd, Format::Mermaid).unwrap();
    let labels: Vec<&str> = back.nodes.values().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, vec!["Root", "A (1)", "B", "C"]);
}

#[test]
fn outlines_roundtrip() {
    let (doc, _) = import("- \"Root\" {\n - \"A\" note=\"first\"\n - \"B\" { - \"C\" }\n}", Format::Rhizo).unwrap();
    for f in [Format::Opml, Format::FreeMind, Format::Markdown] {
        let text = out(&doc, f);
        let (back, _) = import(&text, f).unwrap_or_else(|e| panic!("{f:?}: {e:?}\n{text}"));
        let labels: Vec<&str> = back.nodes.values().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["Root", "A", "B", "C"], "{f:?}");
        let a = back.nodes.values().find(|n| n.label == "A").unwrap();
        assert_eq!(a.note.as_deref(), Some("first"), "{f:?}");
    }
}

#[test]
fn freemind_keeps_arrow_links() {
    let (doc, _) = import("- \"Root\" {\n - \"A\"\n - \"B\"\n}\nedge a b", Format::Rhizo).unwrap();
    let mm = out(&doc, Format::FreeMind);
    assert!(mm.contains("<arrowlink"));
    let (back, _) = import(&mm, Format::FreeMind).unwrap();
    assert_eq!(back.edges.len(), 1);
}

#[test]
fn canvas_roundtrip() {
    let doc = sample();
    let json = out(&doc, Format::JsonCanvas);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["nodes"].as_array().unwrap().iter().any(|n| n["type"] == "group"));
    let (back, _) = import(&json, Format::JsonCanvas).unwrap();
    assert_eq!(back.nodes.len(), doc.nodes.len());
    assert!(back.nodes.values().all(|n| n.at.is_some()));
}

#[test]
fn xml_exports_are_wellformed() {
    let doc = sample();
    for f in [Format::DrawIo, Format::GraphMl, Format::Opml, Format::FreeMind] {
        let text = out(&doc, f);
        roxmltree::Document::parse(&text).unwrap_or_else(|e| panic!("{f:?}: {e}\n{text}"));
    }
    let drawio = out(&doc, Format::DrawIo);
    assert!(drawio.contains("rhombus;"));
    assert!(drawio.contains("link=\"https://example.com/pricing\""));
}

#[test]
fn d2_and_dot() {
    let doc = sample();
    let d2 = out(&doc, Format::D2);
    assert!(d2.contains("release: \"Release\" {"));
    assert!(d2.contains("release.build -> release.check"));
    assert!(d2.contains("shape: diamond"));
    let dot = out(&doc, Format::Dot);
    assert!(dot.starts_with("digraph \"Launch\" {"));
    assert!(dot.contains("subgraph cluster_release"));
    assert!(dot.contains("dir=both"));
}

#[test]
fn format_names() {
    assert_eq!(Format::from_name("kdl"), Some(Format::Rhizo));
    assert_eq!(Format::from_path(std::path::Path::new("x.MMD")), Some(Format::Mermaid));
    assert_eq!(Format::from_name("obsidian"), Some(Format::JsonCanvas));
    assert!(!Format::D2.can_import());
}

const TYPED: &str = r#"rhizo version=2
node "Fn" id=fn type=aws.lambda.function { props memory-mb=512 }
node "Bucket" id=bucket type=aws.s3.bucket
edge fn bucket kind=data id=fn-write { props access="write" }
edge fn bucket
"#;

#[test]
fn typed_data_survives_graphml_and_drawio() {
    let (doc, _) = import(TYPED, Format::Rhizo).unwrap();
    let graphml = out(&doc, Format::GraphMl);
    roxmltree::Document::parse(&graphml).unwrap();
    assert!(graphml.contains(r#"<data key="type">aws.lambda.function</data>"#));
    assert!(graphml.contains(r#"<data key="props">{&quot;memory-mb&quot;:512}</data>"#));
    assert!(graphml.contains(r#"<data key="eid">fn-write</data>"#));
    assert!(graphml.contains(r#"<data key="ekind">data</data>"#));
    // The second, plain edge between the same nodes gets no typed data.
    assert_eq!(graphml.matches("<data key=\"ekind\">").count(), 1);

    let drawio = out(&doc, Format::DrawIo);
    roxmltree::Document::parse(&drawio).unwrap();
    assert!(drawio.contains(r#"rhizo-type="aws.s3.bucket""#));
    assert!(drawio.contains(r#"rhizo-id="fn-write" rhizo-kind="data""#));
}

#[test]
fn lossy_formats_warn_once() {
    let (doc, _) = import(TYPED, Format::Rhizo).unwrap();
    assert!(crate::dropped_features(&doc, Format::Mermaid).is_some());
    assert!(crate::dropped_features(&doc, Format::GraphMl).is_none());
    assert!(crate::dropped_features(&doc, Format::Svg).is_none());
    assert!(crate::dropped_features(&sample(), Format::Mermaid).is_none());
}
