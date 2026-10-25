# Writing rhizo diagrams

rhizo diagrams are plain text files (`.rhizo`) in [KDL v2](https://kdl.dev). A person or an agent writes the text; rhizo checks it, draws it, converts it and, for typed diagrams, compiles it to specs and a build brief. Keep diagrams small and clear: short labels, details in notes.

## The basics

```kdl
rhizo version=1
title "Onboarding"

// A node is `-` (or `node`) with a quoted label. Nesting makes children.
- "Onboarding" {
    - "Account" {
        - "Sign up"
        - "Verify email" { note "Link expires after 24 hours." }
    }
    - "First project" shape=pill fill="#dbeafe"
}

// Edges connect nodes by id. Ids are slugs of labels unless set with id=.
edge sign-up first-project label="then" line=dashed
```

- Every file starts with `rhizo version=1`, or `rhizo version=2` when it uses typed features (below).
- One statement per line; `;` separates statements on one line. Comments start with `//`. Booleans are `#true` and `#false`.
- **Ids.** A node's id is its label slugged (`"Verify email"` → `verify-email`) unless you set `id=`. Set `id=` on any node you connect to or that carries a type, so renaming the label later doesn't break edges. Ids are unique across the document.
- **Edges.** `edge a b c` draws a→b and b→c with the same properties. Ends are ids (or labels). **A name that matches no node creates a new node**, so use exact ids. Properties: `label`, `arrow` (`end` `start` `both` `none`), `line` (`solid` `dashed` `dotted`), `stroke`, `class`.
- **Node properties.** `id`, `shape` (`rect` `rounded` `pill` `ellipse` `circle` `diamond` `hexagon` `text`), `fill`, `stroke`, `text` (text color), `line`, `class`, `tags`, `link`, `x`, `y`. Child statements: `note "…"` and nested nodes.
- **Styles.** `style "warn" fill="#fee2e2" stroke="#ef4444"` once, then `class=warn` on nodes or edges.

## Layouts and frames

Pick the layout that fits the content:

| Content | Layout |
|---|---|
| Ideas, brainstorming | `mindmap` (balanced, colored branches) |
| Hierarchies, org charts, outlines | `tree` with `direction` (`down` `right` `up` `left`) |
| Processes, pipelines, architectures, state machines | `layered` with `direction` |
| Networks, relationships, knowledge graphs | `force` |
| Collections, catalogs | `grid` (`columns=`) |
| Free canvas | `free` (nodes keep `x`/`y`) |

Set the document layout with `layout "layered"` and `direction "right"`, or split the diagram into frames, each with its own layout. Edges may cross frames.

```kdl
rhizo version=1
title "Ingest pipeline"

frame "Sources" layout=grid columns=1 {
    node "Postgres CDC" id=cdc
    node "Clickstream" id=clicks
}
frame "Pipeline" layout=layered direction=right {
    node "Kafka" id=kafka shape=pill
    node "Validate" id=validate
    node "Valid?" id=valid shape=diamond
    node "Warehouse" id=warehouse shape=rect
    node "Dead letters" id=dlq
    edge kafka validate valid
    edge valid warehouse label="yes"
    edge valid dlq label="no" line=dashed
}
edge cdc kafka
edge clicks kafka
```

### Nested frames

A frame can hold frames, each with its own layout, up to 16 deep: a system holding its containers holding their components, an environment holding a VPC holding its subnets. The parent lays out each nested frame as one box among its own nodes, and edges cross frame borders to reach what they connect, routed around the frames and boxes in their way. Write an edge in the innermost frame that holds both ends, or at the top level. A `sequence` frame cannot hold frames.

```kdl
rhizo version=1
title "Online shop"

frame "Online shop" id=shop layout=layered direction=down {
    node "Web shop" id=web
    node "Orders DB" id=db shape=hexagon

    frame "API" id=api layout=layered direction=right {
        node "Orders controller" id=ctl
        node "Checkout service" id=checkout
        edge ctl checkout
    }

    edge web ctl label="JSON over HTTPS"
    edge checkout db label="SQL"
}
node "Customer" shape=pill
edge customer web
```

## Typed diagrams

With `rhizo version=2`, nodes can carry a `type` from a kit, typed `props`, acceptance criteria and ports, and edges a `kind`. `rhizo check` validates props against the kit's JSON Schema; `rhizo compile` turns typed parts into specs.

- `use data version="0.1"` loads a kit. The built-in `rhizo` kit (goals, decisions) needs no `use`.
- `type=data.entity` on a node or a frame. Frame types (like `data.api`) go on `frame`.
- `props` holds typed values, JSON-in-KDL: scalars as `key=value`; a list of two or more as a child `key "a" "b"`; a list of one as `key { - "a" }`; objects as child blocks.
- `accept "…"` (repeatable) states a criterion that must hold when the thing is built.
- Edge `kind=data` (values flow), `kind=control` (order, triggers) or `kind=dependency` (needs to exist first). `from-port=` and `to-port=` pick ports.

```kdl
rhizo version=2
title "Tickets"
use data version="0.1"

node "Ticket" id=ticket type=data.entity {
    node "id" id=ticket-id type=data.field { props format="uuid" required=#true }
    node "title" id=ticket-title type=data.field { props required=#true max=120 }
    node "state" id=ticket-state type=data.field { props required=#true { enum "open" "closed" } }
}
frame "Tickets API" id=api type=data.api layout=layered direction=right {
    props version="1.0.0" auth="oauth2"
    node "Open ticket" id=open-ticket type=data.operation { props method="post" path="/tickets" }
}
edge ticket open-ticket kind=data label="body"
edge open-ticket ticket kind=data label="201"
```

## Infrastructure

`use infra version="0.1"` describes cloud infrastructure; `rhizo compile` turns each `infra.environment` frame into an OpenTofu root module (`infra/<env>/`) with pinned registry modules and secure defaults. Prefer the cloud-neutral types (`infra.network`, `infra.subnet`, `infra.bucket`, `infra.table`, `infra.database`, `infra.queue`, `infra.topic`, `infra.key`, `infra.function`, `infra.api`); the environment's `cloud` (`aws`, `azure` or `gcp`) decides what they become. With `cloud="azure"` or `cloud="gcp"` use only these neutral types (an `aws.*` node there is an error); on azure give a lowercase region such as `westeurope` and put remote state in `props { azure state-storage-account="…" }`; on gcp give a region such as `europe-west3` and put remote state in `props { gcp state-bucket="…" }` (the project is the `project_id` variable, never in the diagram). Settings only one cloud has go in a block named after it: `props storage-gb=50 { aws instance-class="db.t4g.small" }`. The `aws.*` types remain for AWS-only designs. Edges mean something by the types they join:

- HTTP API → function (`kind=control`): a route, from the edge prop `route="POST /orders"` or from the `data.operation`s the function implements (`edge fn op kind=dependency`).
- Function → bucket, table, queue, topic or database (`kind=data`): a least-privilege IAM grant; `access` is `read`, `write` or `read-write`, `prefix` narrows a bucket grant.
- Function or database → network or subnet tier (`kind=dependency`), or nesting inside an `infra.subnet`: where it runs. Databases must be in a network, never in a public tier.
- Queue → function (`kind=control`): the function consumes the queue. Topic → queue or function: a subscription.
- Bucket, table, queue, topic or database → key (`kind=dependency`): encrypted with that key. Stores marked `sensitive`, or holding a `data.entity` with a `pii="sensitive"` field, need one.

```kdl
rhizo version=2
title "Uploads"
use infra version="0.1"

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
```

## Flows

```kdl
rhizo version=2
title "Invoices"

frame "Invoice inbox" type=rhizo.flow layout=layered direction=right {
    props trigger="A new email in invoices@"
    node "Email arrives" id=email shape=pill
    node "Has PDF?" id=has-pdf shape=diamond
    node "Extract fields" id=extract { note "Vendor, amount, due date, IBAN" }
    node "Book in accounting" id=book
    node "Archive" id=archive
    edge email has-pdf
    edge has-pdf extract label="yes"
    edge has-pdf archive label="no"
    edge extract book
    edge book archive
}
```

## Flows that compile

With `use flow version="0.1"`, a flow becomes something that runs: `rhizo compile` writes each frame typed `flow.workflow` as a Serverless Workflow definition (`flows/serverless-workflow/<id>.sw.yaml`) and an importable Node-RED flow (`flows/node-red/<id>.json`).

```kdl
rhizo version=2
title "Refunds"
use flow version="0.1"

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
```

- **One trigger** per workflow: `flow.trigger.webhook` (`path`, `method`; the request body is the data, and the end of the flow answers the request), `flow.trigger.schedule` (`cron`) or `flow.trigger.event` (`event`, a CloudEvent type). Every step must be reachable from it, and flows cannot loop.
- **The data** is one JSON value passed along the edges. An HTTP request's response becomes the data; a transform replaces it with its `set` object; a switch and a delay pass it on.
- **Only a `flow.switch` branches.** Label each edge out of it with a condition or `default`; the first that holds is taken, and with no default the flow ends. Conditions are paths into the data (`total`, `customer.email`), literals, `==` `!=` `<` `<=` `>` `>=`, `and` `or` `not` and parentheses.
- **Templates** in `set` and `body`: `"{{ amount }}"` passes the value with its type, `"Order {{ id }}"` builds a string.
- **Calling an API you drew:** give a `flow.http-request` no `url` and draw a plain edge from it to a `data.operation` (`edge pay api:refund`); the method, path and the API's first `servers` entry are used.
- **`flow.script` is the gap.** Without `code` it compiles to a placeholder and becomes an `implement` task in the brief; put its `accept` lines on it. `code` is a JavaScript function body that gets `data` and returns the new data. Each workflow also gets a `human` task: deploy it.
- `targets` on the workflow picks what to compile to (`serverless-workflow`, `node-red`); a step a target cannot express (`flow.call` and `flow.trigger.event` in Node-RED) is an error, never an approximation.

## Projects: several diagrams that refer to each other

Keep one concern per file (a domain model, an API, each flow) in one folder. A file imports another and refers to its nodes as `alias:id`. The referenced thing keeps its type, props and details from where it is defined; here it is drawn dashed. Nothing has to be declared first: plain names still create local nodes.

```kdl
rhizo version=2
title "Shop API"
use data version="0.1"
import "model.rhizo" as=model

frame "Shop API" id=api type=data.api {
    node "Place order" id=place-order type=data.operation { props method="post" path="/orders" }
}
edge model:order place-order kind=data label="body"
```

- `import "shared/model.rhizo"` without `as=` uses the file name (`model`) as the alias.
- Place a referenced thing in your own frame with `node "Order" id=model:order` (a participant in a sequence, a step in a flow).
- `rhizo check`, `compile`, `brief` and `mcp` take a folder: the whole project is checked, compiled to one bundle and briefed as one.

## Saying what you are aiming for

Two built-in types tell an agent what done looks like and how to build it. They go into the build brief (`rhizo brief`), which turns everything nothing generates into ordered tasks.

```kdl
rhizo version=2
title "Status page"

node "Customers see outages within a minute" id=goal type=rhizo.goal {
    note "A public page shows each service as up, degraded or down."
    accept "An outage shows on the page within 60 seconds"
}
node "Static site on a CDN" id=hosting type=rhizo.decision {
    props status="accepted" decision="Serve the page as static files from a CDN"
}
- "Status page" {
    - "Health checks" { accept "Every service is probed every 15 seconds" }
    - "Page" { accept "Loads in under a second on 3G" }
}
edge health-checks page
edge goal page
```

- A top-level node with children is an **area**; each child becomes a task, everything below it the task's scope. Other top-level nodes are tasks on their own.
- A **flow** is built as one unit: a frame typed `rhizo.flow`, or a flat flowchart with a decision (a `diamond`, or a label ending in `?`). Its steps follow the edges; label the edges out of a decision with their conditions (`label="yes"`). Mark processes, automations, integrations and pipelines as flows; leave architectures (one component per node) unmarked.
- An edge between tasks orders them (the target comes after the source).
- A decision or goal linked by an edge applies to what it links to; an unlinked one applies to everything.
- Put the intent in notes and `accept` lines: they are what an agent builds against.

## Working with rhizo

1. Write or edit the file.
2. `rhizo check FILE`: fix every error (each has a line, a column and usually a fix). With MCP: `check` or `validate`.
3. Look at it: `rhizo render FILE -o out.png`, or the MCP `render` tool. The viewer (`rhizo-view FILE`) follows the file live.
4. For typed diagrams: `rhizo compile FILE -o build/` and `rhizo brief FILE`.

## Kits and their types

The kits built into this rhizo, generated from their schemas:
