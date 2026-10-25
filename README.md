<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/logo-dark.svg">
    <img src="assets/brand/logo-light.svg" alt="rhizo" width="260">
  </picture>
</h1>

**Think in graphs. Ship in any format.** Diagrams and mindmaps as plain text, rendered fast and exported to the formats other tools actually read.

<p>
  <img src="docs/mindmap.png" alt="A mindmap rendered by rhizo" width="720">
</p>

rhizo has one document model and many ways in and out. Write a `.rhizo` file by hand, generate JSON or YAML from a script, or import Mermaid, OPML, FreeMind or an Obsidian canvas. Then render SVG/PNG/PDF, or export to Mermaid, D2, Graphviz, draw.io, GraphML and more.

```kdl
title "Release pipeline"

frame "Release pipeline" layout=layered direction=down {
    node "Merge to main" shape=pill
    node "All green?" id=green shape=diamond
    edge "Merge to main" "Build" "Tests" green
    edge green "Publish" label="yes"
    edge green "Open incident" label="no" line=dashed
}
```

<img src="docs/flow.png" alt="A flowchart rendered by rhizo" width="320">

## Install and use

**Prebuilt:** each [release](https://github.com/vxrz/rhizo/releases) has an archive per platform (macOS arm64 and Intel, Linux x86_64 and arm64, Windows x86_64) with `rhizo` and the viewer `rhizo-view`. Check it before you unpack it:

```bash
sha256sum -c SHA256SUMS.txt --ignore-missing          # macOS: shasum -a 256 -c SHA256SUMS.txt --ignore-missing
gh attestation verify rhizo-<version>-<target>.tar.gz --repo vxrz/rhizo   # built by this repository's release workflow
```

On macOS, an archive that is not yet notarized needs `xattr -d com.apple.quarantine rhizo rhizo-view` once. On Linux, `rhizo-view` needs a Vulkan driver and libxkbcommon (Wayland or X11).

**From source** (Rust stable):

```bash
cargo install --path crates/cli   # rhizo
cargo install --path crates/app   # rhizo-view
```

```bash
rhizo new mindmap > idea.rhizo     # starter (mindmap, flow, canvas, data, sequence, workflow, infra, intent, learn)
rhizo new learn > learn.rhizo      # a hands-on tutorial: open it in rhizo-view and do its ten steps
rhizo samples                      # the built-in examples; `rhizo samples copy shop ./shop` copies one
rhizo new project shop             # a starter project: intent, model, API, a sequence, referring to each other
rhizo render idea.rhizo            # → idea.svg
rhizo render idea.rhizo -o idea.pdf --theme paper
rhizo convert flow.mmd -o flow.rhizo
rhizo convert idea.rhizo -o idea.drawio
rhizo watch idea.rhizo -o idea.png # re-render on save
rhizo check *.rhizo                # diagnostics with source spans
rhizo fmt idea.rhizo               # canonical formatting
rhizo schema > rhizo.schema.json   # JSON Schema for the JSON/YAML forms
rhizo compile shop/ -o build/      # typed diagrams → JSON Schema, OpenAPI, AsyncAPI, OpenTofu + build brief
rhizo brief shop/                  # what an agent needs to build it: goal, questions, tasks
rhizo import tofu infra/prod -o prod.rhizo  # existing OpenTofu → a diagram, proven equal offline (L1)
rhizo adopt prod.rhizo --all       # imported nodes → rhizo's own code (with moved blocks), props editable
rhizo adopt prod.rhizo aws_sqs_queue.jobs --dry-run   # the same checks and report, nothing written
rhizo guide                        # how to write rhizo, for people and language models
rhizo mcp shop/ --allow-write      # let an AI agent read, draw and edit the diagrams
rhizo proposals shop --apply 1     # apply a change an agent proposed (checked first)
```

A native viewer follows the file as you edit it, in any editor:

```bash
cargo run --release -p rhizo-app -- idea.rhizo   # scroll pans, cmd+scroll/pinch zooms, F fits, 0 = 100%
```

New to it? The welcome screen's **Learn rhizo** card (also Help ▸ Learn rhizo, or `rhizo new learn`) saves a tutorial diagram where you choose and walks you through ten steps on it: a child (Tab), a sibling (Enter), renaming (F2), connecting (alt+drag), grouping a selection into a frame (cmd+G), typing a node in the inspector (cmd+I), an Environment frame with a cloud and region, compiling, the text pane (`cmd+\`) and cmd+K. A card in the corner ticks each step off as you do it, read from the file and from what you did, so the file is yours to keep. **Samples** (welcome screen, File ▸ Open Sample…) shows the repo's examples, built in, with a thumbnail each: mindmaps and notes, flows and sequences, a data model, infrastructure on AWS, Azure and Google Cloud, the shop project and two files to import; opening one saves a copy first. The status bar suggests what the keys do for the selection ("Tab child · Enter sibling · F2 rename · cmd+I inspect" for a node, "cmd+G group" for several, the frame and edge commands for those), taken from the command registry; View ▸ Status-Bar Hints hides them for the session. cmd+K understands what you mean as well as what things are called: "aws", "cloud" or "environment" find Set frame type… and New frame, "png" finds Export…, "terraform" or "openapi" Compile specs…, with the word it matched shown faintly; typing a kit type ("bucket") offers Add typed node: Bucket, and typing a label jumps to that node. An empty document says how to start (double-click or Tab adds a node) and links to the tutorial and the samples; an empty frame says what goes in it.

It handles like a Mac app. On macOS the menu bar (File, Edit, View, Insert, Format, Diagram, Window, Help) lists every command with its shortcut; elsewhere cmd+K (ctrl+K) lists the same commands, and cmd+/ shows every shortcut. Right-click (or ctrl+click on macOS) a node, an edge, the canvas, a sidebar row or the text pane for what can be done there, and rest the pointer on a control for a tooltip with its shortcut. Shift+click adds nodes to the selection, cmd+A selects all, and dragging on empty canvas draws a selection box (two-finger scroll, space+drag or the middle button pan); a selection moves together in free layouts, is deleted, cut, copied and pasted (as rhizo text, edges between the nodes included) as one undo step, can be aligned and distributed, and the inspector sets a prop or type on all of it, showing "mixed" where the nodes differ. Every text field (inspector, label, palette) edits like the text pane: alt+arrows by word, cmd+arrows to the ends, shift selects, double and triple click, cut, copy, paste and undo before it is written. One part has the keyboard at a time and shows an accent ring; Tab and shift+Tab walk the inspector's fields and Esc steps back to the canvas.

The drawing holds still while you edit. After a change, yours or an agent's over MCP, only what has to make room moves, and the view stays on what you edited. cmd+K "Tidy layout" lays everything out from scratch, the way `render` and exports always do ([design](docs/design/stable-layout.md)).

Select a node and press cmd+I for the inspector: its type and props as a form built from the kit's schema (an enum is a list to pick from, a boolean a switch, text and numbers are checked as you type), "Set type…" for a plain node, and where a reference is defined or which files use the node. cmd+K "Add typed node…" adds a node of any kit type; Enter next to an operation in an API adds another operation. Every change lands in the file as a minimal text edit, with undo. Everything the file can say can be made in the window, without touching text ([table](docs/design/ui-parity.md)). Right-click the canvas ▸ New frame makes a frame where you clicked (inside a selected frame, it nests) and you type its name; cmd+G groups the selected nodes into a new frame, with the edges among them. Click a frame's label to select it: the inspector shows its type ("Set frame type…" lists Environment, API, Workflow…), layout, direction, colours and props from the type's schema, so an environment offers `cloud` as a choice, `region`, `state-bucket`, the `azure` and `gcp` settings and the budget, and "Add typed node…" lists only what fits in it. Drag a node onto another frame to move it there, or outside every frame to take it out; F2 renames a frame, Backspace asks whether to keep what it holds (Ungroup) or delete it all. Select an edge for its label, kind, line, arrowheads, colour and the props its kits give it (`access` read, write or read-write from a function to a store; `prefix`, `route`, `cardinality`). A node's inspector also sets its shape and colours (swatches), note, link, tags and acceptance criteria; with nothing selected it shows the document: title, theme, kits and the layout of loose nodes. Deleting the last reference into an imported file offers to remove the unused `import`. On a node of imported infrastructure code the inspector says whether it can switch to rhizo's own code, and why not when it cannot; its "Adopt…" button (or cmd+K "Adopt node…", and "Adopt all that switch without changes") runs adopt as a dry run in the background, with OpenTofu's progress in the status bar ("Cancel adopt" stops it), then shows the report: the moves, the changes, the creates and the live-plan rule. Nothing is written until you press Adopt (or cmd+Enter); the file then changes as one edit cmd+Z undoes, and the node's props are editable.

## The format

rhizo files are [KDL v2](https://kdl.dev). Every statement is one line, nesting means "child of", and labels double as ids.

| Statement | Meaning |
|---|---|
| `- "Label" { ... }` | A node; nested `-` lines are its children. `node "Label"` is the same thing. |
| `edge a b c label="x"` | Edges a→b and b→c. Refer to nodes by id, by label, or by the slug of a label. A name that matches nothing creates a node. |
| `frame "Title" layout=... { ... }` | A region with its own layout. A document can mix a mindmap, a flowchart and a free canvas. Frames nest: a frame inside a frame is laid out as one box of its parent (system ⊃ containers ⊃ components), see `examples/c4-system.rhizo`. |
| `style "name" fill="#..."` | A named style. Apply it with `class=name`. |
| `title`, `theme`, `layout`, `direction` | Document settings. `layout` and `direction` apply to nodes outside frames. |

**Layouts.** `mindmap` (balanced, branch colored), `tree` (tidy, org-chart style), `layered` (flowcharts and DAGs), `force` (networks: graph distance becomes drawing distance), `grid`, `free` (nodes keep `x`/`y`), `sequence` (nodes are participants left to right, edges are messages top to bottom, both in document order; a self edge is a self call, `line=dashed` reads as a reply), and `auto`: layered if there are edges, mindmap if there is nesting, otherwise grid. `auto` never picks `sequence`.

**Node properties.** `id`, `class`, `shape` (`rect` `rounded` `pill` `ellipse` `circle` `diamond` `hexagon` `text`), `fill`, `stroke`, `text`, `stroke-width`, `font-size`, `font-weight`, `line` (`solid` `dashed` `dotted`), `x`, `y`, `note`, `link`, `tags`.

**Edge properties.** `label`, `arrow` (`end` `start` `both` `none`), `line`, `class`, `stroke`, `stroke-width`.

**Themes.** `light`, `dark`, `paper`. Styles cascade: theme → layout defaults (root, branch colors) → classes → inline. When you set a fill without a text color, rhizo picks a readable text color for you.

**Typed documents (format version 2).** Nodes, frames and edges can carry a namespaced `type`, a `props` object, ports, acceptance criteria and provenance. Kits give types their meaning. Plain documents are unaffected and stay version 1.

```kdl
rhizo version=2
use infra version="0.1"

node "Receipts" id=receipts type=aws.s3.bucket {
    props versioning=#true {
        lifecycle { - prefix="tmp/" expire-after-days=7 }
    }
    port out "created" kind=data
    accept "Objects are encrypted at rest"
}
edge orders-fn receipts kind=data { props access="write" }
```

`props` blocks follow [JSON-in-KDL](https://github.com/kdl-org/kdl/blob/main/JSON-IN-KDL.md) exactly. Ids can be CURIEs (`id=HP:0001649`) once their `prefix` is declared, and node and edge ids share one namespace. GraphML and draw.io exports keep types and props; other formats drop them with one warning. Design: `docs/design/profiles-and-compiler.md`.

**Kits.** A kit is a typed vocabulary: for each type, a JSON Schema 2020-12 for its props, its ports, and where it may sit. `rhizo check` validates a document against the kits it `use`s and points at the statement at fault:

```
× operation `place-order`, prop `method`: "send" is not one of "get", "put", "post", …
× a control edge cannot use the data port of `customer`   help: use kind=data
× field `loose` belongs inside a data.entity or a data.event, but is not inside a typed node or frame
```

The built-in `data` kit covers entities and fields, enums, APIs and operations, events and channels; `infra` covers cloud infrastructure (below). `rhizo types` lists kits and types; `rhizo types data.field` shows one type's ports and props schema. A kit is a `kit.kdl` manifest plus one schema file per type (`crates/kit/kits/data/`). Typed nodes get their kit's default shape in drawings and exports, and draw what they mean through the type's `look` block in `kit.kdl`: an operation shows its method badge and path, an entity is a card with one row per field, a decision shows its status (proposed dashed, rejected struck), a VPC contains its subnets.

**Compiling.** `rhizo compile shop.rhizo -o build/` turns a checked document into a spec bundle:

```
build/
  manifest.json                        inputs, every file with its sha256 and source node ids
  specs/schemas/order.schema.json      JSON Schema 2020-12, one per entity and enum
  specs/shop-api.openapi.yaml          OpenAPI 3.1, one per data.api frame
  specs/asyncapi.yaml                  AsyncAPI 3, events and channels
```

Fields become properties (`required`, `format`, `min`/`max`, `pii` as `x-rhizo-pii`), data edges between entities become `$ref`s, an edge from an entity to an operation is its request body and one back is a response (label it `201` or `404` to set the status). Operations get RFC 9457 problem-details error responses unless you model your own. Output is deterministic: same document, same bytes. Recompiling removes files it generated before and no longer produces, keeps any you edited by hand, and never touches files it did not write; `--check` fails when the bundle is out of date (for CI). The generated specs pass Redocly and the AsyncAPI CLI.

Start one with `rhizo new data`.

**Workflows.** The built-in `flow` kit turns a drawn automation into something that runs. A frame typed `flow.workflow` holds one trigger (`flow.trigger.webhook`, `.schedule` or `.event`) and the steps it leads to (`flow.http-request`, `flow.switch`, `flow.transform`, `flow.delay`, `flow.call`, `flow.script`), joined by edges. A switch's edges carry their conditions (`label="total > 1000"`), and props can hold templates (`"Order {{ id }}"`); both are a small portable expression language compiled to jq and JSONata. `rhizo compile` writes each workflow twice:

```
build/
  flows/serverless-workflow/order-intake.sw.yaml   Serverless Workflow DSL 1.0.1 (checked against the official schema in the tests)
  flows/node-red/order-intake.json                 a Node-RED flow to import; ids are stable, so a re-import updates it in place
```

Rules catch a missing or second trigger, unreachable steps, loops, branches without conditions, and steps a chosen target cannot express (`targets`): an error, never an approximation. A `flow.http-request` with an edge to a `data.operation` calls it with that operation's method, path and server. What the runtimes cannot do is left as work: a `flow.script` without `code` compiles to a placeholder and an `implement` task in the brief, and each workflow gets a `human` task to deploy it. See `examples/shop/flows/order-intake.rhizo`.

**Infrastructure.** The `infra` kit describes cloud infrastructure: an `infra.environment` frame (cloud, region, remote state, budget) holding networks and subnet tiers, functions, HTTP APIs, Postgres databases, tables, buckets, queues, topics and keys. These cloud-neutral types (`infra.network`, `infra.function`, …) compile for the environment's `cloud`: `aws`, `azure` or `gcp`. On `aws` they become VPCs, Lambda, API Gateway, RDS, DynamoDB, S3, SQS, SNS and KMS, exactly like the `aws.*` types, which remain for AWS-only designs. On `azure` they become pinned Azure Verified Modules: a resource group, a VNet with delegated subnets and NAT, Function Apps on Flex Consumption (one managed identity each), API Management, PostgreSQL Flexible Server with Entra ID auth, Cosmos DB, Storage, Service Bus and Key Vault; edges become role assignments scoped to one resource, and whatever has no faithful Azure mapping is a finding on the node (`examples/playground/12-azure-infra.rhizo`). On `gcp` they become pinned terraform-google-modules in an existing project (its APIs enabled, no project created): a custom-mode VPC with Private Google Access, flow logs and Cloud NAT, Cloud Run functions with Direct VPC egress (one service account each), API Gateway, Cloud SQL for PostgreSQL on private IP with IAM database auth, Firestore, Cloud Storage, Pub/Sub and Cloud KMS; edges become IAM bindings on the one resource, and gaps are findings again (`examples/playground/13-gcp-infra.rhizo`). Settings only one cloud has go in a block named after it (`props { aws instance-class="db.t4g.small" }`, `azure sku-name="GP_Standard_D2ds_v5"`, `gcp tier="db-custom-1-3840"`). Each environment compiles to an OpenTofu root module in `infra/<env>/`: `versions.tf` (OpenTofu and providers pinned exactly, S3 backend with native locking), `encryption.tofu` (state and plan encryption, OpenTofu-only syntax kept out of `.tf` so TFLint and Checkov stay happy), `main.tf` (terraform-aws-modules registry modules pinned to exact versions), `iam.tf`, `network.tf`, `variables.tf` and `outputs.tf`. Edges are the wiring: API → function is a route (or one per `data.operation` the function implements), function → store a least-privilege IAM statement, function → database also a security group rule and IAM database auth, queue → function an event source, store → KMS key its encryption. Defaults are the secure ones (no public buckets, private encrypted deletion-protected databases, TLS-only, flow logs), and rules catch what a schema cannot: a database outside a VPC or in a public subnet, a function that cannot reach its database, sensitive data without a customer-managed key, a queue that hides messages for less than its consumer's timeout. Account ids and package paths become variables without defaults, never guesses. Function code is the one gap: it becomes an `implement` task, and applying the environment a human task after it. The output passes `tofu fmt -check` and `tofu validate`. See `examples/shop/infra.rhizo`.

**Importing OpenTofu.** `rhizo import tofu DIR -o FILE.rhizo` turns an existing root module (AWS, Azure or Google Cloud) into an imported `infra.environment`, and refuses anything it would lose. Every resource, data source and module call is a node that keeps its HCL verbatim, so the diagram compiles back to the same code, and the import proves it: OpenTofu plans the original and the compiled code offline, with every provider mocked, and both plans must be equal (L1). Code rhizo recognizes is folded into the neutral types: S3, Storage accounts and Cloud Storage buckets become `infra.bucket`, Lambda, Function Apps and Cloud Run functions `infra.function`, and so on for queues, topics, keys, tables, Postgres, networks, subnets and APIs, both as raw resources and as the modules rhizo itself emits. A folded node shows the props its literal arguments state (`versioning`, `memory-mb`, `ordered`, …); satellites such as `aws_s3_bucket_versioning` fold into their bucket's props. Those props are read-only (`[infra.import.derived]`: edit the HCL, or the code, and re-import). References become typed edges, grants carry `access` where an IAM statement, role assignment or binding says it, subnets and placed functions nest in their network, and the environment's `cloud` comes from its providers. Kit rules then audit the existing code as warnings ("audit: a bucket without versioning", "a publicly accessible database"), never blocking the round trip. The report gives the semantic coverage: the share of resources that folded.

**Adopting imported code.** `rhizo adopt prod.rhizo module.receipts aws_sqs_queue.jobs` (or `--all`) switches folded nodes to rhizo's own code, so their props become editable and the kit's rules apply to them in full. The node keeps where it came from in `origin` (its address, and the satellites folded into it), and the compiler writes its code into `rhizo.tf` with a `moved` block for each address that changes: a raw `aws_s3_bucket` with its versioning, encryption, public access block, ownership and policy resources becomes the pinned `s3-bucket` module, a raw `google_storage_bucket`, `google_pubsub_topic`, `google_kms_crypto_key` (with its key ring) or `azurerm_key_vault_key` (with its vault) becomes the module rhizo calls for it, and code already in rhizo's shape (its own module calls, on AWS, Azure and Google Cloud) stays where it is. Names and tags of what exists are kept, and references to the old addresses are rewritten. Nothing is written unless L1-adopt passes: OpenTofu plans the code before and after offline, every original resource must be accounted for (where it was, or where a `moved` block takes it), and the report lists each attribute that changes and each resource only the new code creates. `--all` adopts only what switches without a change; `--dry-run` reports without writing; `--var-file` passes variable values; `--against OLD.rhizo` compares a later edit with the imported original. OpenTofu runs every provider the code requires, even mocked, so importing and adopting install only official providers (HashiCorp's aws, azurerm, google, google-beta and utility providers, and `azure/azapi`, `azure/modtm`); code that requires another, in the root module or a module it calls, is refused by name before anything is downloaded. `--allow-provider NAMESPACE/NAME` (repeatable, on `import tofu`, `verify-roundtrip` and `adopt`) allows one for that run only, if you trust it. Whether a change replaces a real resource only a live `tofu plan` shows: the environment's apply task says so.

**Projects.** Several diagrams in one folder can refer to each other: a domain model, the APIs that use it, flows and sequences that move it around. A file imports another (`import "model.rhizo" as=model`) and refers to its nodes as `model:order`, in edges or placed in its own frames, where they are drawn dashed. Nothing has to be declared first; declared things carry their type and details wherever they are used. `rhizo check`, `compile`, `brief` and `mcp` take the folder and treat it as one model. See `examples/shop/`.

**From diagram to build.** Every diagram compiles to a build brief (`AGENTS.md` and `tasks/tasks.json` in the bundle, or `rhizo brief`): the goal, the decisions to follow, the open questions the diagram leaves (no stack chosen, a decision still proposed, branches without conditions, tasks without acceptance criteria), the whole system in words, and the tasks in dependency order with their scope, connections, input specs and acceptance criteria. What a kit compiles (schemas, APIs) is generated; everything else becomes a task an agent can implement. A flow (a frame typed `rhizo.flow`, or a flowchart with a decision) is built as one unit. Two built-in types state intent: `rhizo.goal` (what done looks like) and `rhizo.decision` (stack and constraints).

**Agents (MCP).** `rhizo mcp shop/` serves a diagram or a project over the Model Context Protocol (stdio; protocol revisions 2025-11-25 and 2026-07-28). Agents get `overview`, `list_nodes`, `get_node`, `neighbors`, `check`, `types`, `authoring_guide`, `validate`, `render` (a PNG they can look at), `brief`, `compile_preview` and `propose_change`. With `--allow-write` they can also create and change diagrams (`write_document`, `edit_document`; validated first, never written with errors) and adopt imported infrastructure (`adopt_nodes`, as `rhizo adopt`; extra OpenTofu providers only if you start the server with `--allow-provider`, never by the agent) while `rhizo-view` shows the result live, so you can describe an API, a pipeline or an infrastructure and watch it appear. Without it, the server only reads, and proposals land in `.rhizo/proposals/` for you to apply. With Claude Code:

```bash
claude mcp add rhizo -- rhizo mcp ~/diagrams/shop --allow-write
```

Other hosts: `{ "command": "rhizo", "args": ["mcp", "/path/to/shop", "--allow-write"] }`.

The same document in YAML:

```yaml
title: Plan
nodes:
  - label: Plan
    children: [Research, Build, { label: Ship, shape: pill }]
edges:
  - research -> build
```

## Formats

| Format | Read | Write | Notes |
|---|:-:|:-:|---|
| rhizo (`.rhizo`, `.kdl`) | ✓ | ✓ | native, lossless |
| JSON / YAML | ✓ | ✓ | same schema, lossless; `rhizo schema` |
| Mermaid (`.mmd`) | ✓ | ✓ | flowchart/graph, mindmap and sequenceDiagram; stateDiagram import |
| D2 (`.d2`) | | ✓ | sequence frames as `sequence_diagram` |
| Graphviz (`.dot`) | | ✓ | |
| OPML | ✓ | ✓ | outlines |
| FreeMind / Freeplane (`.mm`) | ✓ | ✓ | includes cross links; XMind can import `.mm` |
| Markdown | ✓ | ✓ | headings and lists, markmap style |
| JSON Canvas (`.canvas`) | ✓ | ✓ | Obsidian; with positions |
| draw.io (`.drawio`) | | ✓ | diagrams.net and Confluence; with positions |
| GraphML | | ✓ | yEd, Gephi, Cytoscape, networkx |
| SVG / PNG / PDF | | ✓ | semantic, accessible SVG; PDF with real, selectable text |

## Architecture

```
crates/
  core/     document model, styles, themes, colors        (no native deps)
  lang/     KDL parser and writer, JSON/YAML, diagnostics  (no native deps)
  layout/   tidy trees, mindmaps, layered graphs, grid     (no native deps)
  render/   SVG writer; PNG via resvg; PDF via krilla
  interop/  every other format
  kit/      kits: typed vocabularies, props schemas, validation
  compile/  spec compiler: typed document → JSON Schema, OpenAPI, AsyncAPI, OpenTofu bundle
  mcp/      Model Context Protocol server for agents (rmcp)
  project/  several files that reference each other, merged into one model
  cli/      the `rhizo` binary
```

Everything above `render` builds for `wasm32-unknown-unknown`, which CI checks, so a web viewer and editor can share the exact same engine.

Decisions and their reasons are recorded in [`docs/adr/`](docs/adr/README.md).

**Layout engines.**
- Trees use non-layered tidy-tree packing with contours, so small subtrees tuck in beside large ones instead of being stacked as bounding boxes. Iterative and O(n · contour).
- Graphs use Sugiyama-style layering: DFS cycle breaking, longest-path layering, Fenwick-tree crossing counts, barycenter sweeps with transposition, and coordinate assignment by exact weighted isotonic regression, which keeps long edges straight.

**Speed.** On an M-series Mac, release build, including loading system fonts, `rhizo render` takes about 0.4 s for a 5,000-node mindmap and 0.6 s for a 1,500-node flowchart.

## Roadmap

1. ~~Core model, text format, layouts, SVG/PNG/PDF, CLI~~
2. ~~Interop: Mermaid, D2, DOT, OPML, FreeMind, Markdown, JSON Canvas, draw.io, GraphML~~
3. Native editor (wgpu + vello): canvas and text side by side, live sync both ways
4. Force-directed layout, network simplex layering, edge-label-aware routing
5. Vega-Lite chart nodes, WASM plugins for custom formats and layouts, web viewer

## License

MIT ([LICENSE-MIT](LICENSE-MIT)) or Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option. A few files from third parties keep their own licences; see [NOTICE](NOTICE).

---

<p align="center"><sub>
Made with ❤️ in Erlangen, Germany · Metropolregion Nürnberg <img src="assets/brand/franconian-rake.svg" alt="Franconian rake" height="14"> · Designed &amp; built by uebelacker.io with AI<br>
© 2026 uebelacker.io · Open source (MIT or Apache-2.0) · <a href="https://github.com/vxrz/rhizo">GitHub</a> · rhizo 0.1.0 · <a href="#readme">About rhizo</a>
</sub></p>
