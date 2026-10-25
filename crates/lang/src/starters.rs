//! Starter documents for new files, one per kind of diagram.

/// The main starters, shown as tiles on the viewer's welcome screen: kind
/// (for [`starter`]) and display name.
pub const STARTERS: &[(&str, &str)] =
    &[("mindmap", "Mindmap"), ("flow", "Flowchart"), ("canvas", "Canvas"), ("data", "Data model")];

/// More starters, for `rhizo new` and the template list.
pub const MORE: &[(&str, &str)] = &[
    ("sequence", "Sequence diagram"),
    ("workflow", "Automation (compiles to Node-RED and Serverless Workflow)"),
    ("infra", "AWS infrastructure (compiles to OpenTofu)"),
    ("intent", "Goal and decisions"),
    ("learn", "Learn rhizo: a hands-on tutorial in ten steps"),
];

/// A starter project: a folder of files that refer to each other
/// (path, contents). `rhizo new project DIR` writes it.
pub const PROJECT: &[(&str, &str)] =
    &[("intent.rhizo", P_INTENT), ("model.rhizo", P_MODEL), ("api.rhizo", P_API), ("flows/create-item.rhizo", P_FLOW)];

/// The starter document for `kind` (see [`STARTERS`]; `map`, `flowchart`
/// and `free` are accepted too).
pub fn starter(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "mindmap" | "map" => MINDMAP,
        "flow" | "flowchart" => FLOW,
        "canvas" | "free" => CANVAS,
        "data" | "api" => DATA,
        "sequence" | "seq" => SEQUENCE,
        "workflow" | "automation" => WORKFLOW,
        "infra" | "aws" => INFRA,
        "intent" | "goal" => INTENT,
        "learn" | "tutorial" => LEARN,
        _ => return None,
    })
}

const SEQUENCE: &str = r##"rhizo version=1
title "Sign in"
layout "sequence"

// Participants left to right, messages top to bottom, in the order written.
// A dashed line reads as a reply; an edge to itself is a self call.
node "User" id=user shape=pill
node "Web app" id=web
node "Auth service" id=auth
node "User store" id=store
edge user web label="Enter email and password"
edge web auth label="POST /sessions"
edge auth store label="Find user"
edge store auth label="user" line=dashed
edge auth auth label="Check the password hash"
edge auth web label="201 Created, session token" line=dashed
edge web user label="Show the dashboard" line=dashed
"##;

const WORKFLOW: &str = r##"rhizo version=2
title "Refunds"
use flow version="0.1"

// Compiles to a Node-RED flow and a Serverless Workflow (`rhizo compile`).
// A script without `code` becomes a task for an agent in the build brief.
frame "Refund request" id=refunds type=flow.workflow layout=layered direction=right {
    props namespace="shop"
    node "Request received" id=received type=flow.trigger.webhook { props method="post" path="/refunds" }
    node "Small?" id=small type=flow.switch
    node "Pay out" id=pay type=flow.http-request {
        props method="post" url="https://payments.example.com/refunds" {
            body { order "{{ order-id }}"; amount "{{ amount }}" }
        }
    }
    node "Check the order" id=check type=flow.script {
        note "Look the order up; refuse it if it is older than 90 days."
        accept "Refuses orders older than 90 days"
    }
    node "Answer" id=answer type=flow.transform { props { set { status "queued"; order "{{ order-id }}" } } }
    edge received small
    edge small pay label="amount <= 50"
    edge small check label="default"
    edge check answer
}
"##;

const INFRA: &str = r##"rhizo version=2
title "Uploads"
use infra version="0.1"

// Compiles to OpenTofu in infra/prod/ (`rhizo compile`) for the cloud set on
// the environment. Edges grant access: the function may write under
// incoming/ in the bucket, nothing else.
frame "Production" id=prod type=infra.environment {
    props cloud="aws" region="eu-central-1" state-bucket="acme-tofu-state"
    node "Uploads API" id=api type=infra.api
    node "Upload function" id=upload-fn type=infra.function { props runtime="nodejs22.x" timeout-s=10 }
    node "Uploads" id=uploads type=infra.bucket { props sensitive=#true }
    node "Uploads key" id=uploads-key type=infra.key
    edge api upload-fn kind=control { props route="PUT /uploads/{name}" }
    edge upload-fn uploads kind=data { props access="write" prefix="incoming/" }
    edge uploads uploads-key kind=dependency
}
"##;

const INTENT: &str = P_INTENT;

/// The tutorial: its steps are nodes, and the viewer ticks them off as
/// they are done (`rhizo_app::learn`, which knows it by its title).
const LEARN: &str = r##"rhizo version=2
title "Learn rhizo"

// A hands-on tutorial in ten steps. Do what each step says, right here on
// the canvas; the card in the corner ticks a step off once it is done.
// This file is yours to keep: every edit is saved as you go.
// (Cmd is Ctrl on Linux and Windows.)
frame "Ten steps" id=steps layout=layered direction=down {
    node "1. Click “My idea”, press Tab, type a name, Enter: a child" id=step-1
    node "2. Press Enter on a node: a sibling beside it" id=step-2
    node "3. Select a node and press F2 (or double-click it) to rename it" id=step-3
    node "4. Hold Alt and drag from “Apples” to “Pears”: an edge" id=step-4
    node "5. Shift+click both fruits (or drag a box round them), then Cmd+G: a frame" id=step-5
    node "6. Cmd+I opens the inspector: give a node a type there" id=step-6
    node "7. Right-click the canvas > New frame, Set frame type… Environment, then a cloud and a region in the inspector" id=step-7
    node "8. Diagram > Compile specs… writes what the diagram becomes" id=step-8
    node "9. Cmd+\\ shows the text: the same diagram, as you could type it" id=step-9
    node "10. Cmd+K finds every command: type “environment” or “export”" id=step-10
    edge step-1 step-2
    edge step-2 step-3
    edge step-3 step-4
    edge step-4 step-5
    edge step-5 step-6
    edge step-6 step-7
    edge step-7 step-8
    edge step-8 step-9
    edge step-9 step-10
}

frame "Try it here" id=try layout=mindmap {
    - "My idea" id=idea
}

frame "Fruit" id=fruit layout=grid columns=2 {
    node "Apples" id=apples shape=pill
    node "Pears" id=pears shape=pill
}
"##;

const P_INTENT: &str = r##"rhizo version=2
title "Intent"

// What done looks like, and what to build it with. The build brief
// (`rhizo brief`) hands both to whoever builds it, person or agent.
node "Items can be managed online" id=goal type=rhizo.goal {
    note "Describe the end state in a sentence or two."
    accept "One check that proves it works"
}
node "Stack" id=stack type=rhizo.decision {
    props status="proposed" decision="Name the language, runtime and hosting"
}
"##;

const P_MODEL: &str = r##"rhizo version=2
title "Domain model"
use data version="0.1"

// The things the other files refer to as model:item.
node "Item" id=item type=data.entity {
    node "id" id=item-id type=data.field { props format="uuid" required=#true }
    node "name" id=item-name type=data.field { props required=#true max=200 }
}
"##;

const P_API: &str = r##"rhizo version=2
title "API"
use data version="0.1"
import "model.rhizo" as=model

frame "API" id=api type=data.api layout=layered direction=right {
    props version="0.1.0"
    node "List items" id=list-items type=data.operation { props method="get" path="/items" }
    node "Create item" id=create-item type=data.operation { props method="post" path="/items" }
}

edge list-items model:item kind=data label="200" { props cardinality="many" }
edge model:item create-item kind=data label="body"
"##;

const P_FLOW: &str = r##"rhizo version=2
title "Create an item"
import "../api.rhizo" as=api

frame "Create an item" layout=sequence {
    node "User" id=user shape=pill
    node "Web app" id=web
    node "Create item" id=api:create-item
    edge user web label="Fill in the form"
    edge web api:create-item label="POST /items"
    edge api:create-item web label="201 Created" line=dashed
    edge web user label="Show the item" line=dashed
}
"##;

const MINDMAP: &str = r##"rhizo version=1
title "My idea"

// `-` adds a node; nesting makes children. Ids come from labels.
- "My idea" {
    - "Why" {
        - "Problem"
        - "Who has it"
    }
    - "How" {
        - "First step"
        - "Risks" fill="#fee2e2"
    }
    - "Next"
}
"##;

const FLOW: &str = r##"rhizo version=1
title "Process"
layout "layered"
direction "down"

node "Start" shape=pill
node "Ready?" id=ready shape=diamond
node "Do the thing"
node "Done" shape=pill

// Refer to nodes by id or by label. Unknown names create new nodes.
edge start ready
edge ready "Do the thing" label="yes"
edge ready start label="no" line=dashed
edge "Do the thing" done
"##;

const CANVAS: &str = r##"rhizo version=1
title "Board"
layout "free"

node "Idea" x=0 y=0
node "Related" x=260 y=40
node "Question?" x=120 y=180 shape=ellipse
edge idea related
edge idea "Question?" line=dotted arrow=none
"##;

const DATA: &str = r##"rhizo version=2
title "My API"
use data version="0.1"

// Typed nodes: `rhizo check` validates their props, `rhizo compile` turns
// them into JSON Schema and OpenAPI. `rhizo types data` lists the types.
frame "Model" layout=tree direction=right {
    node "Item" id=item type=data.entity {
        node "id" id=item-id type=data.field { props format="uuid" required=#true }
        node "name" id=item-name type=data.field { props required=#true max=200 }
    }
}

frame "API" id=api type=data.api layout=layered direction=right {
    props version="0.1.0"
    node "List items" id=list-items type=data.operation { props method="get" path="/items" }
    node "Create item" id=create-item type=data.operation { props method="post" path="/items" }
}

edge list-items item kind=data label="200" { props cardinality="many" }
edge item create-item kind=data
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Syntax, load};

    #[test]
    fn every_starter_loads_cleanly() {
        for (kind, _) in STARTERS.iter().chain(MORE) {
            let loaded = load(starter(kind).unwrap(), Syntax::Kdl);
            assert!(loaded.diagnostics.is_empty(), "{kind}: {:?}", loaded.diagnostics);
            assert!(!loaded.document.nodes.is_empty(), "{kind}");
        }
        assert_eq!(starter("map"), starter("mindmap"));
        assert!(starter("nope").is_none());
    }
}
