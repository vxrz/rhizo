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

use rhizo_core::{Point, Shape};

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

#[test]
fn force_layout_frame() {
    let d = diagram(
        r#"
        frame "Team" layout=force {
            edge "Ada" "Grace" "Linus" "Ada"
            edge "Grace" "Margaret"
            node "Alone"
        }
        "#,
    );
    assert_eq!(d.frames[0].layout, LayoutKind::Force);
    assert_no_overlap(&d);
    assert_eq!(d.edges.len(), 4);
    // Edges start and end on node outlines, not centers.
    let e = &d.edges[0];
    let a = node(&d, "ada").rect;
    assert!(!a.inflate(-1.0, -1.0).contains(e.path.start));
}

/// No edge label covers a node or another label.
fn assert_labels_clear(d: &Diagram) {
    let labels: Vec<_> = d.edges.iter().filter_map(|e| Some((e, e.label.as_ref()?.rect()))).collect();
    for (i, (e, l)) in labels.iter().enumerate() {
        for n in &d.nodes {
            assert!(!l.inflate(-0.5, -0.5).intersects(&n.rect), "label of {}->{} covers `{}`", e.from, e.to, n.id);
        }
        for (f, m) in &labels[i + 1..] {
            assert!(
                !l.inflate(-0.5, -0.5).intersects(m),
                "labels of {}->{} and {}->{} overlap",
                e.from,
                e.to,
                f.from,
                f.to
            );
        }
    }
}

#[test]
fn labels_keep_clear_of_nodes_and_each_other() {
    let docs = [
        // Layered, with a labeled self loop beside a neighbor.
        r#"
        layout "layered"
        edge "Build" "Test" label="run the suite"
        edge "Test" "Test" label="flaky, retry"
        edge "Build" "Lint" label="check"
        edge "Lint" "Ship" label="clean"
        edge "Test" "Ship" label="green"
        edge "Ship" "Build" label="next release"
        "#,
        // The same, sideways.
        r#"
        layout "layered"
        direction "right"
        edge "Build" "Test" label="run the suite"
        edge "Test" "Test" label="flaky, retry"
        edge "Build" "Lint" label="check"
        edge "Lint" "Ship" label="clean"
        "#,
        // Mindmap and tree cross links whose midpoints fall on other nodes.
        r#"
        frame "Map" layout=mindmap {
            - "Root" {
                - "Alpha" { - "One"; - "Two"; - "Three" }
                - "Beta" { - "Four"; - "Five" }
            }
            edge one five label="depends on"
            edge two four label="blocks"
            edge four four label="loops back"
        }
        frame "Org" layout=tree {
            - "Lead" { - "Left"; - "Middle"; - "Right" }
            edge left right label="pairs with"
        }
        "#,
        // Grid neighbors, and labeled edges between frames.
        r#"
        frame "Grid" layout=grid columns=3 {
            node "A"; node "B"; node "C"; node "D"
            edge a b label="hands over to"
            edge a d label="reviews"
        }
        frame "Flow" layout=layered { edge "X" "Y" label="then" }
        edge c x label="feeds"
        edge d y label="also feeds"
        "#,
    ];
    for src in docs {
        let d = diagram(src);
        assert_no_overlap(&d);
        assert_labels_clear(&d);
    }
}

/// No edge the layered engine leaves alone (links in other layouts, edges
/// between frames) passes through a node other than its ends, or through a
/// frame neither end is in.
fn assert_routes_clear(d: &Diagram) {
    let layered: Vec<_> = d.frames.iter().filter(|f| f.layout == LayoutKind::Layered).map(|f| &f.id).collect();
    for e in d.edges.iter().filter(|e| e.kind == EdgeKind::Link && e.from != e.to) {
        let (a, b) = (node(d, e.from.as_str()), node(d, e.to.as_str()));
        if a.frame == b.frame && layered.contains(&&a.frame) {
            continue;
        }
        let pts = e.path.flatten(24);
        let blocks = d.nodes.iter().filter(|n| n.id != a.id && n.id != b.id).map(|n| (n.id.as_str(), n.rect)).chain(
            d.frames
                .iter()
                .filter(|f| f.visible && f.id != a.frame && f.id != b.frame)
                .map(|f| (f.id.as_str(), f.rect)),
        );
        for (id, r) in blocks {
            assert!(
                !pts.windows(2).any(|w| crate::route::crosses(w[0], w[1], &r)),
                "{} -> {} crosses `{id}`",
                e.from,
                e.to
            );
        }
    }
}

const ROUTING_DOCS: [&str; 5] = [
    // Edges between frames, some skipping a frame in between.
    r#"
    frame "One" layout=layered { edge "A1" "A2" "A3" }
    frame "Two" layout=mindmap { - "Hub" { - "x"; - "y"; - "z"; - "w"; - "v" } }
    frame "Three" layout=grid columns=2 { node "C1"; node "C2"; node "C3"; node "C4" }
    frame "Four" layout=layered direction=right { edge "D1" "D2" }
    edge a1 c1 label="skip"
    edge a3 c4
    edge a2 d2 label="far"
    edge c3 a2 line=dashed
    edge hub c2
    edge a1 z
    "#,
    // A tree beside an API, linked across fields (the data model example).
    r#"
    frame "Model" layout=tree direction=right {
        node "Customer" { node "id" id=cid; node "email" { node "asd" }; node "name" }
        node "Order" { node "id" id=oid; node "total"; node "status"; node "placed at" }
        edge order customer label="placed by"
    }
    frame "Shop API" layout=layered direction=right {
        node "List orders" id=list
        node "Place order" id=place
    }
    edge list order label="200"
    edge order place label="body"
    "#,
    // Mindmap cross links over branches and siblings, one of them doubled.
    r#"
    frame "Map" layout=mindmap {
        - "Product" {
            - "Research" { - "Interviews"; - "Surveys"; - "Analytics" }
            - "Design" { - "Wireframes"; - "Prototype"; - "Visual" }
            - "Build" { - "Backend"; - "Frontend"; - "Mobile" }
            - "Launch" { - "Press"; - "Pricing"; - "Docs" }
        }
        edge interviews prototype label="informs"
        edge surveys pricing label="sets"
        edge visual press label="assets"
        edge analytics mobile
        edge analytics mobile line=dashed
    }
    "#,
    // Grid links across rows, columns and diagonals.
    r#"
    frame "Teams" layout=grid columns=4 {
        node "A"; node "B"; node "C"; node "D"
        node "E"; node "F"; node "G"; node "H"
        node "I"; node "J"; node "K"; node "L"
        edge a d label="row"
        edge a k label="diagonal"
        edge e h
        edge b j
        edge i d
        edge c l
        edge c l
    }
    "#,
    // Free placement with a blocker on every straight line.
    r#"
    layout "free"
    node "Left" x=0 y=100
    node "Blocker" x=200 y=95
    node "Right" x=420 y=100
    node "Top" x=210 y=0
    node "Bottom" x=210 y=220
    node "Wide box with a long label here" id=wide x=120 y=320
    node "Under" x=10 y=420
    node "Over" x=420 y=250
    edge left right label="through"
    edge top bottom
    edge left right
    edge right left
    edge under over label="diag"
    "#,
];

#[test]
fn routes_go_around_nodes_and_frames() {
    for src in ROUTING_DOCS {
        let d = diagram(src);
        assert_no_overlap(&d);
        assert_routes_clear(&d);
        assert_labels_clear(&d);
        // Ends stay on their nodes' outlines.
        let on_outline = |n: &crate::NodeBox, p: Point| {
            let boxy = matches!(n.shape, Shape::Rect | Shape::Rounded);
            n.rect.inflate(1.0, 1.0).contains(p) && !(boxy && n.rect.inflate(-1.0, -1.0).contains(p))
        };
        for e in d.edges.iter().filter(|e| e.kind == EdgeKind::Link) {
            let (a, b) = (node(&d, e.from.as_str()), node(&d, e.to.as_str()));
            assert!(on_outline(a, e.path.start) && on_outline(b, e.path.end()), "{} -> {} is off its ends", a.id, b.id);
        }
    }
}

#[test]
fn parallel_edges_keep_apart() {
    for src in [ROUTING_DOCS[3], ROUTING_DOCS[4], ROUTING_DOCS[2]] {
        let d = diagram(src);
        let links: Vec<_> = d.edges.iter().filter(|e| e.kind == EdgeKind::Link).collect();
        for (i, e) in links.iter().enumerate() {
            for f in &links[i + 1..] {
                let same = (e.from == f.from && e.to == f.to) || (e.from == f.to && e.to == f.from);
                if same {
                    let gap = (1..10).map(|k| e.path.at(k as f32 / 10.0).distance(f.path.at(k as f32 / 10.0)));
                    assert!(gap.fold(0.0, f32::max) > 6.0, "{} and {} coincide", e.from, e.to);
                }
            }
        }
    }
}

#[test]
fn self_loops_fan_out() {
    let docs = [
        r#"
        layout "layered"
        edge "Idle" "Running" "Done"
        edge "Running" "Running" label="tick"
        edge "Running" "Running" label="retry"
        edge "Running" "Running" label="poll"
        edge "Running" "Running"
        "#,
        r#"
        layout "layered"
        direction "right"
        edge "Idle" "Running" "Done"
        edge "Running" "Running" label="tick"
        edge "Running" "Running" label="retry"
        edge "Running" "Running" label="poll"
        "#,
        r#"
        layout "mindmap"
        - "Root" { - "Prototype"; - "Other" }
        edge prototype prototype label="iterate"
        edge prototype prototype label="test"
        edge prototype prototype label="review"
        "#,
    ];
    for src in docs {
        let d = diagram(src);
        assert_no_overlap(&d);
        assert_labels_clear(&d);
        let loops: Vec<_> = d.edges.iter().filter(|e| e.from == e.to).collect();
        assert!(loops.len() >= 3);
        for (i, a) in loops.iter().enumerate() {
            for b in &loops[i + 1..] {
                let apart = a.path.at(0.5).distance(b.path.at(0.5));
                assert!(apart > 12.0, "two loops on `{}` meet: {apart}", a.from);
            }
        }
    }
}

#[test]
fn routing_is_deterministic() {
    let all = ROUTING_DOCS.concat();
    for src in ROUTING_DOCS.iter().copied().chain([all.as_str()]) {
        let (a, b) = (diagram(src), diagram(src));
        let paths = |d: &Diagram| {
            format!("{:?}", d.edges.iter().map(|e| (&e.path, e.label.as_ref().map(|l| l.center))).collect::<Vec<_>>())
        };
        assert_eq!(paths(&a), paths(&b));
    }
}

const CHECKOUT: &str = r#"
    title "Checkout"
    layout "sequence"
    node "Customer" id=customer
    node "Web app" id=web
    node "Orders API" id=api
    node "Payments" id=pay
    edge customer web label="Place order"
    edge web api label="POST /orders"
    edge api pay label="Charge card"
    edge pay api label="charged" line=dashed
    edge api api label="Save order"
    edge api web label="201 Created" line=dashed
    edge web customer label="Confirmation" line=dashed
"#;

/// Eight participants, thirty messages, long labels and self calls.
fn long_sequence() -> String {
    let names = ["User", "App", "Gateway", "Auth", "Orders", "Inventory", "Payments provider", "Notifications"];
    let mut src = String::from("layout \"sequence\"\n");
    for n in names {
        src += &format!("node \"{n}\"\n");
    }
    for k in 0..30usize {
        let (a, b) = (k * 5 % 8, if k % 7 == 3 { k * 5 % 8 } else { (k * 3 + 1) % 8 });
        let label = "a long message label that has to wrap ".repeat(k % 3 + 1);
        src += &format!("edge \"{}\" \"{}\" label=\"{label}{k}\"\n", names[a], names[b]);
    }
    src
}

fn messages(d: &Diagram) -> Vec<&crate::EdgeBox> {
    d.edges.iter().filter(|e| e.kind == EdgeKind::Link).collect()
}

fn lifeline_x(d: &Diagram, id: &rhizo_core::NodeId) -> f32 {
    d.edges.iter().find(|e| e.kind == EdgeKind::Lifeline && &e.from == id).unwrap().path.start.x
}

#[test]
fn sequence_messages_run_top_to_bottom() {
    for src in [CHECKOUT.to_string(), long_sequence()] {
        let d = diagram(&src);
        assert_eq!(d.frames[0].layout, LayoutKind::Sequence);
        assert_no_overlap(&d);
        assert_labels_clear(&d);
        let doc = load(&src, Syntax::Kdl).into_result().unwrap().0;
        let msgs = messages(&d);
        assert_eq!(msgs.len(), doc.edges.len());
        let mut last = f32::MIN;
        for (m, e) in msgs.iter().zip(&doc.edges) {
            assert_eq!((&m.from, &m.to), (&e.from, &e.to), "messages keep document order");
            let top = m.path.bounds().y;
            assert!(top > last, "message {}->{} is not below the one before", m.from, m.to);
            last = m.path.bounds().bottom();
            assert!((m.path.start.x - lifeline_x(&d, &m.from)).abs() < 0.01);
            assert!((m.path.end().x - lifeline_x(&d, &m.to)).abs() < 0.01);
            if let Some(l) = &m.label {
                assert!(l.rect().bottom() <= m.path.bounds().y, "label of {}->{} sits above its arrow", m.from, m.to);
            }
        }
        // Participants in document order, left to right, on one row.
        let xs: Vec<f32> = d.nodes.iter().map(|n| n.rect.center().x).collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]));
        assert!(d.nodes.iter().all(|n| n.rect.y == d.nodes[0].rect.y));
    }
}

#[test]
fn sequence_lifelines_span_all_messages() {
    let d = diagram(CHECKOUT);
    let bottom = messages(&d).iter().map(|m| m.path.bounds().bottom()).fold(f32::MIN, f32::max);
    let lifelines: Vec<_> = d.edges.iter().filter(|e| e.kind == EdgeKind::Lifeline).collect();
    assert_eq!(lifelines.len(), d.nodes.len());
    for (l, n) in lifelines.iter().zip(&d.nodes) {
        assert_eq!((&l.from, &l.to), (&n.id, &n.id));
        assert!(l.arrow == rhizo_core::Arrow::None && l.label.is_none());
        assert!((l.path.start.y - n.rect.bottom()).abs() < 0.01 && (l.path.start.x - n.rect.center().x).abs() < 0.01);
        assert!(l.path.end().y > bottom);
    }
}

#[test]
fn sequence_self_calls_loop_right() {
    let d = diagram(CHECKOUT);
    let call = messages(&d).into_iter().find(|m| m.from == m.to).unwrap();
    let x = lifeline_x(&d, &call.from);
    let b = call.path.bounds();
    assert!((call.path.start.x - x).abs() < 0.01 && (call.path.end().x - x).abs() < 0.01);
    assert!(call.path.end().y > call.path.start.y + 10.0);
    assert!(b.x >= x - 0.01 && b.right() > x + 20.0);
    // The label clears the next lifeline.
    assert!(call.label.as_ref().unwrap().rect().right() < lifeline_x(&d, &"pay".into()));
}

#[test]
fn sequence_is_deterministic_and_never_auto() {
    let src = long_sequence();
    let (a, b) = (diagram(&src), diagram(&src));
    let dump = |d: &Diagram| {
        format!("{:?}", d.edges.iter().map(|e| (&e.path, e.label.as_ref().map(|l| l.center))).collect::<Vec<_>>())
    };
    assert_eq!(dump(&a), dump(&b));
    let auto = diagram(&CHECKOUT.replace("layout \"sequence\"", ""));
    assert_eq!(auto.frames[0].layout, LayoutKind::Layered);
}

#[test]
fn sequence_frame_beside_other_frames() {
    let d = diagram(
        r#"
        frame "Flow" layout=layered { edge "Start" "Done" }
        frame "Talk" layout=sequence {
            node "Client"; node "Server"
            edge client server label="ask"
            edge server client label="answer" line=dashed
        }
        edge done client label="then"
        edge server start
        "#,
    );
    assert_no_overlap(&d);
    assert_labels_clear(&d);
    // Edges between frames stay out of the message area.
    let msgs = messages(&d);
    let inside = |e: &&crate::EdgeBox| d.nodes.iter().find(|n| n.id == e.from).unwrap().frame.as_str() == "talk";
    let area = msgs
        .iter()
        .filter(|e| inside(e) && d.nodes.iter().find(|n| n.id == e.to).unwrap().frame.as_str() == "talk")
        .map(|e| e.path.bounds())
        .reduce(|a, b| a.union(&b))
        .unwrap();
    for e in msgs.iter().filter(|e| {
        e.to.as_str() == "client" && e.from.as_str() == "done"
            || e.from.as_str() == "server" && e.to.as_str() == "start"
    }) {
        for p in e.path.flatten(32) {
            assert!(!area.inflate(-1.0, -1.0).contains(p), "{}->{} crosses the messages", e.from, e.to);
        }
    }
}

/// `src` laid out with `decor` set the way a kit would set it.
fn decorated(src: &str, decor: &[(&str, rhizo_core::Decor)]) -> Diagram {
    let mut doc = load(src, Syntax::Kdl).into_result().expect("valid source").0;
    for (id, d) in decor {
        doc.decor.insert(rhizo_core::NodeId::from(*id), d.clone());
    }
    layout(&doc, &Theme::light(), &Heuristic)
}

fn card() -> rhizo_core::Decor {
    rhizo_core::Decor { card: true, tone: "blue".into(), ..Default::default() }
}

fn row(detail: &str, strong: bool) -> rhizo_core::Decor {
    let row = rhizo_core::DecorRow { detail: Some(detail.into()), strong, marker: None };
    rhizo_core::Decor { row: Some(row), ..Default::default() }
}

const ER: &str = r#"
    frame "Model" layout=layered direction=right {
        node "Order" {
            node "id" id=order-id
            node "total"
            node "status" id=order-status
        }
        node "Status" { - "open"; - "paid" }
        edge status order-status
        edge order customer label="placed by"
        node "Customer" { node "email" }
    }
"#;

fn er() -> Diagram {
    decorated(
        ER,
        &[
            ("order", card()),
            ("order-id", row("uuid", true)),
            ("total", row("number", true)),
            ("order-status", row("string", false)),
            ("status", card()),
            ("open", row("", false)),
            ("paid", row("", false)),
            ("customer", card()),
            ("email", row("email", true)),
        ],
    )
}

#[test]
fn entities_are_cards_with_one_row_per_field() {
    let d = er();
    let order = node(&d, "order");
    let rows: Vec<_> = ["order-id", "total", "order-status"].map(|id| node(&d, id).rect).into();
    // Rows fill the card's width, stack under the title band without gaps,
    // and end just above its bottom.
    for (i, r) in rows.iter().enumerate() {
        assert_eq!((r.x, r.width), (order.rect.x, order.rect.width));
        if i > 0 {
            assert_eq!(r.y, rows[i - 1].bottom());
        }
    }
    assert!(rows[0].y > order.rect.y + 20.0, "a title band comes first");
    assert!((order.rect.bottom() - rows[2].bottom() - 6.0).abs() < 0.01);
    // Rows are drawn inside the card, not as boxes joined by branches.
    let t = node(&d, "total").typed.as_deref().unwrap();
    assert!(t.row && t.host.as_ref().is_some_and(|h| h.as_str() == "order"));
    assert!(node(&d, "order").typed.as_deref().unwrap().card);
    assert!(d.edges.iter().all(|e| e.kind != EdgeKind::Branch), "no branches into rows");
    // The required field's name is bold; its type sits on the right.
    let ink = node(&d, "order-id").ink();
    assert!(ink.iter().any(|i| matches!(i, crate::Ink::Text { text, weight: 600, .. } if text == "id")));
    assert!(ink.iter().any(|i| matches!(i, crate::Ink::Text { text, end: true, .. } if text == "uuid")));
    // Cards, the items the engine placed, do not overlap.
    let cards: Vec<_> = d.nodes.iter().filter(|n| n.typed.as_ref().is_none_or(|t| t.host.is_none())).collect();
    for (i, a) in cards.iter().enumerate() {
        for b in &cards[i + 1..] {
            assert!(!a.rect.intersects(&b.rect), "{} overlaps {}", a.id, b.id);
        }
    }
}

#[test]
fn relations_attach_to_the_card_and_field_refs_to_the_row() {
    let d = er();
    let (order, status_row) = (node(&d, "order").rect, node(&d, "order-status").rect);
    // Entity to entity: an ordinary link to the card's outline.
    let rel = d.edges.iter().find(|e| e.to.as_str() == "customer").unwrap();
    assert_eq!(rel.from.as_str(), "order");
    let end = rel.path.start;
    assert!(order.inflate(1.0, 1.0).contains(end) && !order.inflate(-1.0, -1.0).contains(end));
    // Enum to field: leaves a side of the card level with the row.
    let r = d.edges.iter().find(|e| e.to.as_str() == "order-status").unwrap();
    assert_eq!(r.kind, EdgeKind::Link);
    let q = r.path.end();
    assert!((q.y - status_row.center().y).abs() < 0.01);
    assert!((q.x - order.x).abs() < 0.01 || (q.x - order.right()).abs() < 0.01);
}

#[test]
fn containers_hold_their_children() {
    let decor = rhizo_core::Decor { container: true, ..Default::default() };
    let d = decorated(
        r#"
        frame "Prod" layout=layered direction=right {
            node "VPC" { node "Subnet" { node "DB" } }
            node "Function"
            edge function db
        }
        "#,
        &[("vpc", decor.clone()), ("subnet", decor)],
    );
    let (vpc, subnet, db) = (node(&d, "vpc").rect, node(&d, "subnet").rect, node(&d, "db").rect);
    let inside = |inner: crate::Rect, outer: crate::Rect| {
        inner.x > outer.x && inner.y > outer.y && inner.right() < outer.right() && inner.bottom() < outer.bottom()
    };
    assert!(inside(subnet, vpc) && inside(db, subnet));
    assert!(d.edges.iter().all(|e| e.kind != EdgeKind::Branch));
    // The deepest box is drawn last, so it is on top and hit first.
    let order: Vec<&str> = d.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(order.iter().position(|&i| i == "db") > order.iter().position(|&i| i == "subnet"));
    // The edge reaches the database itself.
    let e = d.edges.iter().find(|e| e.to.as_str() == "db").unwrap();
    assert!(db.inflate(1.0, 1.0).contains(e.path.end()));
}

#[test]
fn badges_and_subtitles_widen_the_node() {
    let src = r#"node "Place order" id=op"#;
    let plain = diagram(src);
    let badge = rhizo_core::Badge { text: "POST".into(), tone: "blue".into() };
    let decor = rhizo_core::Decor { badge: Some(badge), subtitle: Some("/orders".into()), ..Default::default() };
    let d = decorated(src, &[("op", decor)]);
    let (a, b) = (node(&plain, "op"), node(&d, "op"));
    assert!(b.rect.width > a.rect.width && b.rect.height > a.rect.height);
    let ink = b.ink();
    assert!(ink.iter().any(|i| matches!(i, crate::Ink::Tag { text, .. } if text == "POST")));
    assert!(ink.iter().any(|i| matches!(i, crate::Ink::Text { text, .. } if text == "/orders")));
    // Everything stays inside the box.
    for i in ink {
        if let crate::Ink::Tag { rect, .. } = i {
            assert!(rect.x >= 0.0 && rect.right() <= b.rect.width && rect.bottom() <= b.rect.height);
        }
    }
}

#[test]
fn nesting_outside_mindmaps_draws_unlike_an_edge() {
    let d = diagram(
        r#"
        frame "Flow" layout=layered {
            node "Service" { node "API" }
            edge api queue
        }
        frame "Org" layout=tree { node "CEO" { node "CTO" } }
        "#,
    );
    let link = d.edges.iter().find(|e| e.kind == EdgeKind::Link).unwrap();
    let branches: Vec<_> = d.edges.iter().filter(|e| e.kind == EdgeKind::Branch).collect();
    assert_eq!(branches.len(), 2);
    for b in branches {
        assert_eq!(b.arrow, rhizo_core::Arrow::None, "{} → {}", b.from, b.to);
        assert_ne!(b.color, link.color);
        assert!(b.width < link.width);
    }
}

#[test]
fn typed_layout_is_deterministic() {
    let a = er();
    let b = er();
    assert_eq!(format!("{:?}", a), format!("{:?}", b));
}

const NESTED: &str = r#"
    frame "Context" layout=layered direction=down {
        node "Customer"
        node "Payments"
        frame "Shop" layout=layered direction=down {
            node "Web"
            node "DB"
            frame "API" layout=layered direction=right {
                edge "Controller" "Service" "Repo"
            }
            frame "Jobs" layout=grid { node "Mailer"; node "Cleanup" }
            edge web controller
            edge repo db
        }
        edge customer web
        edge service payments
    }
    frame "Other" layout=grid { node "Lonely" }
    edge mailer lonely
"#;

/// The drawn frames holding `id`'s frame, itself included.
fn holding<'a>(d: &'a Diagram, frame: &rhizo_core::FrameId) -> Vec<&'a crate::FrameBox> {
    let mut out = Vec::new();
    let mut cur = d.frames.iter().find(|f| &f.id == frame);
    while let Some(f) = cur {
        out.push(f);
        cur = f.parent.as_ref().and_then(|p| d.frames.iter().find(|g| &g.id == p));
    }
    out
}

#[test]
fn nested_frames_contain_their_contents_with_padding() {
    let d = diagram(NESTED);
    let frame = |id: &str| d.frames.iter().find(|f| f.id.as_str() == id).unwrap();
    let within = |inner: crate::Rect, outer: crate::Rect, pad: f32| {
        inner.x >= outer.x + pad
            && inner.y >= outer.y + pad
            && inner.right() <= outer.right() - pad
            && inner.bottom() <= outer.bottom() - pad
    };
    assert_eq!((frame("context").depth, frame("shop").depth, frame("api").depth), (0, 1, 2));
    // Names first used by an edge in a nested frame live in that frame.
    assert_eq!(node(&d, "controller").frame.as_str(), "api");
    assert_eq!(node(&d, "service").frame.as_str(), "api");
    assert_eq!(frame("api").parent.as_ref().map(|p| p.as_str()), Some("shop"));
    // Parents come before the frames inside them, so they are drawn underneath.
    let pos = |id: &str| d.frames.iter().position(|f| f.id.as_str() == id).unwrap();
    assert!(pos("context") < pos("shop") && pos("shop") < pos("api") && pos("shop") < pos("jobs"));
    for f in d.frames.iter().filter(|f| f.parent.is_some()) {
        let p = frame(f.parent.as_ref().unwrap().as_str());
        assert!(within(f.rect, p.rect, 20.0), "`{}` is not inside `{}` with padding", f.id, p.id);
    }
    for n in &d.nodes {
        assert!(within(n.rect, frame(n.frame.as_str()).rect, 20.0), "`{}` sticks out of its frame", n.id);
        // No node overlaps a frame it is not in.
        let mine = holding(&d, &n.frame);
        for f in d.frames.iter().filter(|f| !mine.iter().any(|m| m.id == f.id)) {
            assert!(!n.rect.intersects(&f.rect), "`{}` overlaps frame `{}`", n.id, f.id);
        }
    }
    // Sibling frames keep apart.
    for (i, a) in d.frames.iter().enumerate() {
        for b in d.frames[i + 1..].iter().filter(|b| b.parent == a.parent) {
            assert!(!a.rect.intersects(&b.rect), "frames `{}` and `{}` overlap", a.id, b.id);
        }
    }
    assert_no_overlap_nodes(&d);
    // The parent's layered layout ranks its boxes by the edges into nested
    // frames: the customer above the shop, the shop above the payments.
    assert!(node(&d, "customer").rect.bottom() < frame("shop").rect.y);
    assert!(frame("shop").rect.bottom() < node(&d, "payments").rect.y);
}

fn assert_no_overlap_nodes(d: &Diagram) {
    for (i, a) in d.nodes.iter().enumerate() {
        for b in &d.nodes[i + 1..] {
            assert!(!a.rect.inflate(-0.5, -0.5).intersects(&b.rect), "`{}` overlaps `{}`", a.id, b.id);
        }
    }
}

#[test]
fn edges_between_nested_frames_avoid_unrelated_boxes() {
    let d = diagram(NESTED);
    for e in d.edges.iter().filter(|e| e.kind == EdgeKind::Link) {
        let (a, b) = (node(&d, e.from.as_str()), node(&d, e.to.as_str()));
        if a.frame == b.frame {
            continue;
        }
        let (fa, fb) = (holding(&d, &a.frame), holding(&d, &b.frame));
        let pts = e.path.flatten(24);
        let others = d.nodes.iter().filter(|n| n.id != a.id && n.id != b.id).map(|n| (n.id.as_str(), n.rect));
        let frames =
            d.frames.iter().filter(|f| !fa.iter().chain(&fb).any(|h| h.id == f.id)).map(|f| (f.id.as_str(), f.rect));
        for (id, r) in others.chain(frames) {
            assert!(
                !pts.windows(2).any(|w| crate::route::crosses(w[0], w[1], &r)),
                "{} -> {} crosses `{id}`",
                a.id,
                b.id
            );
        }
    }
}

#[test]
fn nested_layout_is_deterministic() {
    let a = diagram(NESTED);
    let b = diagram(NESTED);
    assert_eq!(format!("{:?}", a), format!("{:?}", b));
}

#[test]
fn broken_nesting_still_lays_out() {
    // Built by hand: a cycle and a frame inside a sequence. Both end up at the top level.
    let mut doc = load(
        "frame \"A\" { node \"x\" }\nframe \"B\" { node \"y\" }\nframe \"S\" layout=sequence { edge p q }",
        Syntax::Kdl,
    )
    .into_result()
    .unwrap()
    .0;
    doc.frame_mut(&"a".into()).unwrap().parent = Some("b".into());
    doc.frame_mut(&"b".into()).unwrap().parent = Some("a".into());
    let d = layout(&doc, &Theme::light(), &Heuristic);
    assert_eq!(d.nodes.len(), 4);
    doc.frame_mut(&"a".into()).unwrap().parent = Some("s".into());
    doc.frame_mut(&"b".into()).unwrap().parent = None;
    let d = layout(&doc, &Theme::light(), &Heuristic);
    assert_eq!(d.nodes.len(), 4);
    assert!(d.frames.iter().all(|f| f.parent.is_none()));
}

#[test]
fn edges_into_containers_go_around_sibling_containers() {
    let decor = rhizo_core::Decor { container: true, ..Default::default() };
    let d = decorated(
        r#"
        frame "Prod" layout=layered direction=down {
            node "VPC" { node "Data" { node "DB"; node "Cache"; node "Queue" }; node "App" }
            node "Function"
            edge function app
            edge function db
        }
        "#,
        &[("vpc", decor.clone()), ("app", decor.clone()), ("data", decor)],
    );
    for (to, avoid) in [("app", ["data", "cache"]), ("db", ["app", "queue"])] {
        let e = d.edges.iter().find(|e| e.to.as_str() == to).unwrap();
        let pts = e.path.flatten(24);
        for id in avoid {
            let r = node(&d, id).rect;
            assert!(!pts.windows(2).any(|w| crate::route::crosses(w[0], w[1], &r)), "function -> {to} crosses {id}");
        }
    }
}
