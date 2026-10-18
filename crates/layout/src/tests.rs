use rhizo_core::{LayoutKind, Theme};
use rhizo_lang::{Syntax, load};

use crate::{Diagram, EdgeKind, Heuristic, layout};

fn diagram(src: &str) -> Diagram {
    let doc = load(src, Syntax::Kdl).into_result().expect("valid source").0;
    layout(&doc, &Theme::light(), &Heuristic)
}

fn assert_no_overlap(d: &Diagram) {
    for (i, a) in d.nodes.iter().enumerate() {
        for b in &d.nodes[i + 1..] {
            assert!(
                !a.rect.inflate(-0.5, -0.5).intersects(&b.rect),
                "`{}` overlaps `{}`: {:?} {:?}",
                a.id,
                b.id,
                a.rect,
                b.rect
            );
        }
    }
    for (i, a) in d.frames.iter().enumerate() {
        for b in &d.frames[i + 1..] {
            assert!(!a.rect.intersects(&b.rect), "frames `{}` and `{}` overlap", a.id, b.id);
        }
    }
}

fn node<'a>(d: &'a Diagram, id: &str) -> &'a crate::NodeBox {
    d.nodes.iter().find(|n| n.id.as_str() == id).unwrap()
}

#[test]
fn mindmap_grows_both_ways() {
    let d = diagram(
        r#"
        - "Root" {
            - "North" { - "a"; - "b"; - "c" }
            - "East" { - "d"; - "e" }
            - "South" { - "f"; - "g"; - "h" }
            - "West" { - "i" }
        }
        "#,
    );
    assert_eq!(d.frames[0].layout, LayoutKind::Mindmap);
    assert_no_overlap(&d);
    let root = node(&d, "root").rect.center().x;
    assert!(node(&d, "north").rect.center().x > root);
    assert!(node(&d, "west").rect.center().x < root);
    assert_eq!(d.edges.iter().filter(|e| e.kind == EdgeKind::Branch).count(), 13);
    // Branch colors differ per top-level child.
    assert_ne!(node(&d, "north").stroke, node(&d, "east").stroke);
    assert_eq!(node(&d, "a").branch, Some(0));
}

#[test]
fn tree_down_is_tidy() {
    let d = diagram(
        r#"
        layout "tree"
        - "CEO" {
            - "CTO" { - "Eng 1"; - "Eng 2"; - "Eng 3" }
            - "CFO"
            - "COO" { - "Ops" }
        }
        "#,
    );
    assert_no_overlap(&d);
    let ceo = node(&d, "ceo");
    let cto = node(&d, "cto");
    assert!(cto.rect.y > ceo.rect.bottom());
    assert_eq!(node(&d, "eng-1").rect.y, node(&d, "eng-3").rect.y);
}

#[test]
fn layered_flow() {
    let d = diagram(
        r#"
        frame "Release" layout=layered {
            node "Build" shape=pill
            node "Test" shape=diamond
            node "Ship"
            edge build test ship
            edge test build label="fail" line=dashed
        }
        "#,
    );
    assert_no_overlap(&d);
    let (b, t, s) = (node(&d, "build"), node(&d, "test"), node(&d, "ship"));
    assert!(b.rect.bottom() < t.rect.y && t.rect.bottom() < s.rect.y);
    let back = d.edges.iter().find(|e| e.label.is_some()).unwrap();
    assert_eq!(back.from.as_str(), "test");
    assert!(back.path.end().y <= b.rect.bottom() + 1.0, "back edge enters build from below");
}

#[test]
fn frames_sit_side_by_side_and_link() {
    let d = diagram(
        r#"
        frame "Ideas" { - "Idea" { - "Detail" } }
        frame "Plan" layout=layered { edge "Do" "Check" }
        edge detail do
        "#,
    );
    assert_eq!(d.frames.len(), 2);
    assert!(d.frames.iter().all(|f| f.visible));
    assert_no_overlap(&d);
    assert!(d.frames[0].rect.right() < d.frames[1].rect.x);
    let cross = d.edges.iter().find(|e| e.from.as_str() == "detail").unwrap();
    assert!(d.frames[0].rect.contains(cross.path.start));
    assert!(d.frames[1].rect.inflate(1.0, 1.0).contains(cross.path.end()));
    // Everything is inside the bounds.
    for n in &d.nodes {
        assert!(
            d.bounds.contains(Point::new(n.rect.x, n.rect.y))
                && d.bounds.contains(Point::new(n.rect.right(), n.rect.bottom()))
        );
    }
}

use rhizo_core::Point;

#[test]
fn grid_and_free() {
    let d = diagram(r#"- "a"; - "b"; - "c"; - "d"; - "e""#);
    assert_eq!(d.frames[0].layout, LayoutKind::Grid);
    assert_no_overlap(&d);

    let d = diagram(r#"layout "free"; - "a" x=10 y=20; - "b" x=300 y=20; edge a b"#);
    let (a, b) = (node(&d, "a"), node(&d, "b"));
    assert_eq!(b.rect.x - a.rect.x, 290.0);
}

#[test]
fn styles_cascade() {
    let d = diagram(
        r##"
        style "hot" fill="#dc2626"
        layout "layered"
        node "A" class=hot
        node "B" class=hot text="#000000"
        edge a b
        "##,
    );
    let (a, b) = (node(&d, "a"), node(&d, "b"));
    assert_eq!(a.fill.to_string(), "#dc2626");
    // Readable text is picked automatically on a dark fill.
    assert_eq!(a.text.to_string(), "#ffffff");
    assert_eq!(b.text.to_string(), "#000000");
}

#[test]
fn big_mindmap_is_fast() {
    let mut src = String::from("- \"Root\" {\n");
    for i in 0..40 {
        src.push_str(&format!("  - \"Topic {i}\" {{\n"));
        for j in 0..25 {
            src.push_str(&format!("    - \"Item {i}.{j}\"\n"));
        }
        src.push_str("  }\n");
    }
    src.push_str("}\n");
    let t = std::time::Instant::now();
    let d = diagram(&src);
    assert_eq!(d.nodes.len(), 1041);
    assert!(t.elapsed().as_millis() < 1500, "took {:?}", t.elapsed());
    assert_no_overlap(&d);
}
