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

const SEQUENCE_MMD: &str = r#"
sequenceDiagram
    title Login flow
    autonumber
    actor U as User
    participant B as Browser
    participant S as Server
    U->>B: Open /login
    activate B
    B->>+S: POST /session
    Note right of S: rate limited
    alt valid credentials
        S->>DB: SELECT user; hash
        DB-->>S: row
        S-->>-B: 200 + cookie
    else invalid
        S--xB: 401
    end
    loop every 5 min
        B-)S: heartbeat
    end
    B<<->>S: sync
    deactivate B
    B->>B: render dashboard
    S-->B
"#;

#[test]
fn mermaid_sequence_import() {
    let (doc, warnings) = import(SEQUENCE_MMD, Format::Mermaid).unwrap();
    assert_eq!(doc.title.as_deref(), Some("Login flow"));
    assert_eq!(doc.frames[0].layout, LayoutKind::Sequence);
    let ids: Vec<&str> = doc.nodes.keys().map(|k| k.as_str()).collect();
    assert_eq!(ids, ["U", "B", "S", "DB"]);
    assert_eq!(doc.nodes[&id("U")].label, "User");
    assert_eq!(doc.nodes[&id("U")].style.shape, Some(Shape::Pill));
    assert_eq!(doc.nodes[&id("S")].note.as_deref(), Some("rate limited"));
    assert_eq!(doc.edges.len(), 10);
    assert_eq!(doc.edges[1].to.as_str(), "S", "activation markers are not part of the name");
    assert_eq!(doc.edges[2].label.as_deref(), Some("SELECT user; hash"));
    assert_eq!(doc.edges[3].line(), LineStyle::Dashed);
    assert_eq!(doc.edges[5].arrow, Arrow::End);
    assert_eq!(doc.edges[7].arrow, Arrow::Both);
    assert_eq!((doc.edges[8].from.as_str(), doc.edges[8].to.as_str()), ("B", "B"));
    assert_eq!(
        (doc.edges[9].arrow, doc.edges[9].line(), doc.edges[9].label.as_deref()),
        (Arrow::None, LineStyle::Dashed, None)
    );
    let skipped: Vec<&str> = warnings.iter().map(|w| w.message.split('`').nth(1).unwrap()).collect();
    assert_eq!(skipped, ["autonumber", "activate", "alt", "loop", "deactivate"]);
}

#[test]
fn mermaid_sequence_roundtrip() {
    let (doc, _) = import(SEQUENCE_MMD, Format::Mermaid).unwrap();
    let mmd = out(&doc, Format::Mermaid);
    assert!(mmd.contains("sequenceDiagram\n    actor U as User\n"), "{mmd}");
    assert!(mmd.contains("S->>DB: SELECT user#59; hash"), "{mmd}");
    let (back, warnings) = import(&mmd, Format::Mermaid).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let shape = |d: &Document| {
        let nodes: Vec<_> = d.nodes.values().map(|n| (&n.id, &n.label, n.style.shape)).collect();
        let edges: Vec<_> = d.edges.iter().map(|e| (&e.from, &e.to, &e.label, e.arrow, e.line())).collect();
        format!("{nodes:?} {edges:?}")
    };
    assert_eq!(shape(&back), shape(&doc));
    assert_eq!(out(&back, Format::Mermaid), mmd);
}

#[test]
fn d2_sequence_export() {
    let (doc, _) = import(SEQUENCE_MMD, Format::Mermaid).unwrap();
    let d2 = out(&doc, Format::D2);
    assert!(d2.contains("shape: sequence_diagram\n"), "{d2}");
    assert!(d2.contains("U -> B: \"Open /login\""), "{d2}");
}

#[test]
fn lifelines_are_not_exported_as_edges() {
    let (doc, _) = import(SEQUENCE_MMD, Format::Mermaid).unwrap();
    for f in [Format::JsonCanvas, Format::DrawIo, Format::GraphMl] {
        let text = out(&doc, f);
        assert!(
            !text.contains("edge-11") && !text.contains("\"e11\"") && !text.contains("e-11"),
            "{f:?}: lifelines exported"
        );
    }
}

const NESTED: &str = r#"
frame "Context" layout=layered {
    node "Customer"
    frame "Shop" layout=layered {
        node "Web"
        frame "API" layout=layered direction=right {
            edge "Controller" "Service"
        }
    }
    edge customer web
}
node "Loose"
edge service loose
"#;

#[test]
fn nested_frames_export_without_losing_nodes() {
    let doc = import(NESTED, Format::Rhizo).unwrap().0;
    for f in [
        Format::Mermaid,
        Format::D2,
        Format::Dot,
        Format::Opml,
        Format::FreeMind,
        Format::Markdown,
        Format::JsonCanvas,
        Format::DrawIo,
        Format::GraphMl,
    ] {
        let text = out(&doc, f);
        for n in doc.nodes.values() {
            assert!(text.contains(&n.label), "{} lost `{}`", f.name(), n.label);
        }
        assert_eq!(crate::dropped_features(&doc, f).is_some(), !crate::keeps_nested_frames(f), "{}", f.name());
    }
    // Subgraphs and containers nest.
    let mmd = out(&doc, Format::Mermaid);
    assert!(mmd.contains("    subgraph context[\"Context\"]\n        customer(\"Customer\")\n        subgraph shop"));
    assert!(mmd.contains("            subgraph api[\"API\"]\n"));
    let d2 = out(&doc, Format::D2);
    assert!(d2.contains("context.shop.api.service -> loose"), "{d2}");
    assert!(out(&doc, Format::Dot).contains("        subgraph cluster_shop {"));
    let drawio = out(&doc, Format::DrawIo);
    roxmltree::Document::parse(&drawio).unwrap();
    assert!(drawio.contains(r#"<mxCell id="f-api" "#) && drawio.contains(r#"parent="f-shop">"#));

    // Mermaid reads its own nesting back.
    let back = import(&mmd, Format::Mermaid).unwrap().0;
    let parent = |id: &str| back.frame(&id.into()).and_then(|f| f.parent.clone()).map(|p| p.to_string());
    assert_eq!(parent("api").as_deref(), Some("shop"));
    assert_eq!(parent("shop").as_deref(), Some("context"));
    assert_eq!(back.nodes.len(), doc.nodes.len());
}
