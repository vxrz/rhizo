use super::*;

fn run(src: &str) -> Vec<String> {
    let loaded = rhizo_lang::load(src, rhizo_lang::Syntax::Kdl);
    assert!(!loaded.has_errors(), "{:?}", loaded.diagnostics);
    check(&loaded.document, &Kits::builtin())
        .into_iter()
        .map(|f| {
            let d = f.diagnostic;
            match d.help {
                Some(h) => format!("{:?}: {} ({h})", d.severity, d.message),
                None => format!("{:?}: {}", d.severity, d.message),
            }
        })
        .collect()
}

const MODEL: &str = r#"rhizo version=2
use data version="0.1"
node "Customer" id=customer type=data.entity {
    node "email" type=data.field { props type="string" format="email" required=#true pii="personal" }
    node "name" type=data.field
}
node "Order" id=order type=data.entity {
    node "total" type=data.field { props type="number" min=0 }
}
node "Status" id=status type=data.enum {
    - "open"
    - "paid"
}
edge order customer kind=data { props cardinality="one" }
frame "Shop API" type=data.api {
    props version="1.0.0" auth="oauth2"
    node "List orders" id=list-orders type=data.operation { props method="get" path="/orders" }
    edge list-orders order kind=data
}
edge customer list-orders
"#;

#[test]
fn a_clean_model_passes() {
    assert_eq!(run(MODEL), Vec::<String>::new());
}

#[test]
fn builtin_kit_loads() {
    let kits = Kits::builtin();
    let data = kits.get("data").unwrap();
    assert_eq!(data.types.len(), 7);
    assert!(data.types["data.api"].frame);
    assert_eq!(data.types["data.field"].parents, ["data.entity", "data.event"]);
    assert!(data.types["data.field"].prop_names().contains(&"pii"));
}

#[test]
fn props_are_checked_against_the_schema() {
    let src = MODEL
        .replace(r#"pii="personal""#, r#"pii="secret" requird=#true"#)
        .replace(r#"min=0"#, r#"min="zero""#)
        .replace(r#"path="/orders""#, r#"path="orders""#);
    assert_eq!(
        run(&src),
        [
            "Error: field `email`: unknown prop `requird` (did you mean `required`?)",
            r#"Error: field `email`, prop `pii`: "secret" is not one of "none", "personal" or "sensitive""#,
            r#"Error: field `total`, prop `min`: "zero" is not of type "number""#,
            r#"Error: operation `list-orders`, prop `path`: "orders" does not match "^/""#,
        ]
    );
}

#[test]
fn required_props_and_frame_props() {
    let src = MODEL
        .replace(r#"props method="get" path="/orders""#, r#"props method="get""#)
        .replace("auth=\"oauth2\"", "auth=\"basic\"");
    let got = run(&src);
    assert_eq!(got.len(), 2, "{got:#?}");
    assert!(got[0].starts_with("Error: frame `shop-api`, prop `auth`"), "{got:#?}");
    assert_eq!(got[1], r#"Error: operation `list-orders`: "path" is a required property"#);
}

#[test]
fn unknown_types_and_missing_use() {
    let src = MODEL.replace("type=data.enum", "type=data.enumm");
    assert_eq!(run(&src), ["Error: the `data` kit has no type `data.enumm` (did you mean `data.enum`?)"]);
    let src = MODEL.replace("use data version=\"0.1\"\n", "");
    assert_eq!(
        run(&src),
        [
            "Error: `data.api` comes from the `data` kit, which this document does not use (add: use data version=\"0.1\")"
        ]
    );
}

#[test]
fn kits_and_versions() {
    let src = MODEL.replace("version=\"0.1\"", "version=\"0.2\"");
    assert_eq!(
        run(&src)[0],
        "Error: this document needs data 0.2, but rhizo has data 0.1.0 (use data version=\"0.1\")"
    );
    let src = "rhizo version=2\nuse ui version=\"0.1\"\nnode \"A\" type=ui.screen\n";
    assert_eq!(
        run(src),
        ["Warning: kit `ui` is not available, so its types are not checked (available kits: data, flow, infra, rhizo)"]
    );
}

#[test]
fn containment() {
    let src = MODEL.replace(
        "edge customer list-orders",
        "node \"loose\" type=data.field\nnode \"Op\" id=op type=data.operation { props method=\"get\" path=\"/\" }",
    );
    assert_eq!(
        run(&src),
        [
            "Error: field `loose` belongs inside a data.entity or a data.event, but is not inside a typed node or frame",
            "Error: operation `op` belongs inside a data.api, but is not inside a typed node or frame",
        ]
    );
    // A frame type on a node, and a node type on a frame.
    let src = MODEL
        .replace("type=data.enum", "type=data.api")
        .replace("frame \"Shop API\" type=data.api", "frame \"Shop API\" type=data.entity");
    let got = run(&src);
    assert!(got.iter().any(|m| m.starts_with("Error: `data.api` is a frame type")), "{got:#?}");
    assert!(got.iter().any(|m| m == "Error: `data.entity` is a node type, not a frame type"), "{got:#?}");
}

#[test]
fn ports_and_edge_kinds() {
    let src = MODEL.replace("edge customer list-orders", "edge customer list-orders kind=control");
    assert_eq!(run(&src), ["Error: a control edge cannot use the data port of `customer` (use kind=data)"]);
    let src = MODEL.replace("edge customer list-orders", "edge customer list-orders to-port=reqest");
    assert_eq!(run(&src), ["Error: `list-orders` has no port `reqest` (did you mean `request`?)"]);
    let src = MODEL.replace("edge customer list-orders", "edge customer list-orders to-port=response");
    assert_eq!(run(&src), ["Error: port `response` of `list-orders` is an out port; an edge ends at an in port"]);
}

#[test]
fn data_rules() {
    let src =
        MODEL.replace(r#"cardinality="one""#, r#"cardinality="few""#).replace("    - \"open\"\n    - \"paid\"\n", "");
    assert_eq!(
        run(&src),
        [
            r#"Error: cardinality "few" between entities: use "one" or "many""#,
            "Warning: enum `status` has no values (add one child node per value)",
        ]
    );
}

#[test]
fn style_defaults_fill_only_unset_shapes() {
    let src = MODEL.replace(
        r#"node "Order" id=order type=data.entity"#,
        r#"node "Order" id=order type=data.entity shape=ellipse"#,
    );
    let mut doc = rhizo_lang::load(&src, rhizo_lang::Syntax::Kdl).document;
    Kits::builtin().apply_style_defaults(&mut doc);
    let shape = |id: &str| doc.nodes[&rhizo_core::NodeId(id.into())].style.shape;
    assert_eq!(shape("customer"), Some(rhizo_core::Shape::Rect));
    assert_eq!(shape("order"), Some(rhizo_core::Shape::Ellipse), "the document wins");
    assert_eq!(shape("status"), Some(rhizo_core::Shape::Hexagon));
    assert_eq!(shape("email"), None, "fields have no default");
}

#[test]
fn the_rhizo_kit_needs_no_use() {
    let src = r#"rhizo version=2
node "Checkout works" id=goal type=rhizo.goal { props status="active" due="2026-12-01" }
node "Use Postgres" id=pg type=rhizo.decision { props status="accepted" decision="Postgres 17 for all state" }
node "Bad" id=bad type=rhizo.decision { props status="maybe" }
node "Typo" id=typo type=rhizo.goall
"#;
    assert_eq!(
        run(src),
        [
            r#"Error: decision `bad`, prop `status`: "maybe" is not one of "proposed", "accepted", "rejected", "deprecated" or "superseded""#,
            "Error: the `rhizo` kit has no type `rhizo.goall` (did you mean `rhizo.goal`?)",
        ]
    );
}

const FLOW: &str = r#"rhizo version=2
use flow version="0.1"
use data version="0.1"
frame "Shop API" id=api type=data.api {
    props { servers { - "https://api.example.com" } }
    node "Place order" id=place-order type=data.operation { props method="post" path="/orders" }
}
frame "Intake" id=intake type=flow.workflow {
    props namespace="shop"
    node "Received" id=received type=flow.trigger.webhook { props path="/orders" }
    node "Large?" id=large type=flow.switch
    node "Approve" id=approve type=flow.http-request {
        props method="post" url="https://approvals.example.com" { body { id "{{ id }}" } }
    }
    node "Place" id=place type=flow.http-request
    node "Wait" id=wait type=flow.delay { props seconds=5 }
    node "Done" id=done type=flow.transform { props { set { ok "Order {{ id }} placed" } } }
    edge received large
    edge large approve label="total > 1000 and not express"
    edge large place label="default"
    edge approve place
    edge place wait done
}
edge place place-order
"#;

#[test]
fn a_clean_workflow_passes() {
    assert_eq!(run(FLOW), Vec::<String>::new());
    let kits = Kits::builtin();
    let flow = kits.get("flow").unwrap();
    assert!(flow.types["flow.workflow"].frame);
    assert_eq!(flow.types["flow.switch"].parents, ["flow.workflow"]);
    assert_eq!(flow.targets, ["serverless-workflow", "node-red"]);
}

#[test]
fn flow_steps_live_in_a_workflow() {
    let src = "rhizo version=2\nuse flow version=\"0.1\"\nnode \"Wait\" id=wait type=flow.delay { props seconds=0 }\n";
    assert_eq!(
        run(src),
        [
            "Error: delay `wait`, prop `seconds`: 0 is less than the minimum of 1",
            "Error: delay `wait` belongs inside a flow.workflow, but is not inside a typed node or frame",
        ]
    );
}

#[test]
fn flow_graph_rules() {
    // A second trigger, an unreachable step, a loop, an unlabelled branch,
    // a bad condition and fan-out without a switch.
    let src = FLOW
        .replace(
            r#"    edge received large"#,
            "    node \"Tick\" id=tick type=flow.trigger.schedule { props cron=\"0 7 * * *\" }\n    node \"Lost\" id=lost type=flow.delay { props seconds=1 }\n    edge received large",
        )
        .replace(r#"label="default""#, "")
        .replace("total > 1000 and not express", "total = 1000")
        .replace("edge place wait done", "edge place wait done\n    edge done place\n    edge wait large");
    assert_eq!(
        run(&src),
        [
            "Error: workflow `intake` has a second trigger `tick` (the first is `received`) (one trigger per workflow: split it into two workflows)",
            "Error: condition `total = 1000` on the edge `large` → `approve`: `=` compares nothing: use `==`",
            "Error: the edge `large` → `place` leaves a switch without a condition (label it with a condition, like label=\"total > 1000\", or label=\"default\")",
            "Error: `wait` leads to 2 steps (done, large); only a flow.switch branches (put a flow.switch after it and label its edges with conditions)",
            "Error: step `lost` is never reached from trigger `received` (connect it with an edge from an earlier step, or remove it)",
            "Error: the edge `done` → `place` closes a loop; workflows cannot loop yet",
            "Error: the edge `wait` → `large` closes a loop; workflows cannot loop yet",
        ]
    );
}

#[test]
fn flow_step_rules() {
    let src = FLOW
        .replace(r#"props path="/orders""#, r#"props path="/orders" method="get""#)
        .replace(r#"url="https://approvals.example.com""#, r#"method="get" url="https://approvals.example.com""#)
        .replace(r#"props method="post" method="get""#, r#"props method="get""#)
        .replace(r#"id "{{ id }}""#, r#"id "{{ id ""#)
        .replace(
            r#"node "Place" id=place type=flow.http-request"#,
            r#"node "Place" id=place type=flow.http-request { props method="put" }"#,
        )
        .replace(r#"props { set { ok "Order {{ id }} placed" } }"#, r#"props { set { ok "{{ id + 1 }}" } }"#)
        .replace(
            r#"edge received large"#,
            "node \"Other\" id=other type=flow.call { props workflow=\"intake\" }\n    edge received large",
        )
        .replace("edge place wait done", "edge place wait done other")
        .replace(r#"props { servers { - "https://api.example.com" } }"#, "");
    assert_eq!(
        run(&src),
        [
            "Error: `approve` sends a body with a GET request (use another method, or drop `body`)",
            "Error: `approve`: template \"{{ id \": a `{{` is not closed: add `}}`",
            "Error: `place` calls operation `place-order`, but its API lists no `servers` (give the data.api frame a `servers` list)",
            "Error: `done`: template \"{{ id + 1 }}\": unexpected `+`",
            "Error: flow.call `other` cannot be compiled to node-red: Node-RED has no core node that runs another flow and waits for its result (leave node-red out of the workflow's `targets`)",
            "Error: `other` calls `intake`, which is not another workflow (no other workflow here)",
        ]
    );
}

#[test]
fn flow_targets_and_cross_workflow_edges() {
    let src = r#"rhizo version=2
use flow version="0.1"
frame "A" id=a type=flow.workflow {
    node "On event" id=on-event type=flow.trigger.event { props event="com.example.placed" }
    node "Wait" id=wait-a type=flow.delay { props seconds=1 }
    edge on-event wait-a
}
frame "B" id=b type=flow.workflow {
    props { targets { - "serverless-workflow" } }
    node "On event" id=on-b type=flow.trigger.event { props event="com.example.placed" }
    node "Run A" id=run-a type=flow.call { props workflow="a" }
    edge on-b run-a
}
frame "C" id=c type=flow.workflow {
    node "Step" id=step type=flow.delay { props seconds=1 }
}
edge run-a wait-a
"#;
    assert_eq!(
        run(src),
        [
            "Error: flow.trigger.event `on-event` cannot be compiled to node-red: Node-RED has no core node that receives CloudEvents (leave node-red out of the workflow's `targets`)",
            "Error: workflow `c` has no trigger (add a flow.trigger.webhook, flow.trigger.schedule or flow.trigger.event)",
            "Error: `run-a` and `wait-a` are steps of different workflows (run the other workflow with a flow.call step)",
        ]
    );
}
