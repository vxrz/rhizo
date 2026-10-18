# Typed nodes, profiles and the spec compiler

| | |
|---|---|
| Status | Draft, for discussion |
| Date | 2026-10-18 |
| Scope | `rhizo-core` model, `rhizo-lang` syntax, new crates `rhizo-profile`, `rhizo-compile`, `rhizo-mcp` |
| Touches today's code | [`crates/core/src/model.rs`](../../crates/core/src/model.rs), [`crates/lang/src/ast.rs`](../../crates/lang/src/ast.rs), [`crates/lang/src/kdl.rs`](../../crates/lang/src/kdl.rs), [`crates/lang/src/lower.rs`](../../crates/lang/src/lower.rs), [`crates/lang/src/write.rs`](../../crates/lang/src/write.rs) |

## Summary

rhizo draws diagrams. This document proposes how a diagram can also be a **specification**: nodes get a *type* and validated *properties*, edges get a *kind*, and a **profile** supplies the vocabulary, the rules and a **compiler** that turns a typed diagram into a deterministic bundle of open-format artifacts. The bundle holds OpenAPI, JSON Schema, OpenTofu, workflow files, ADRs, a security baseline and a task graph with acceptance criteria. Coding agents read the bundle and the model through an MCP server. They fill only the gaps the compiler cannot fill, and their work counts as done only when the verification gates pass. Runtime state (OpenTofu state, workflow executions, agent progress) flows back onto the same nodes as *overlays*, which turns the diagram into a live view.

rhizo does not become a runtime. It emits files that existing tools run: OpenTofu, Kestra, n8n, Node-RED, any CNCF Serverless Workflow engine, and whatever app stack the ADRs choose.

Plain diagramming stays first-class. A document with no `type` anywhere parses, lowers, lays out, renders and round-trips exactly as it does today, byte for byte.

The first step is small: add `type`, `props`, `ports` and edge `kind` to the core model and the three syntaxes. Store them, round-trip them and give them no meaning yet. That step should land **before the native editor** (roadmap item 3), so the editor's data model, undo stack and text↔canvas sync are built on the final node shape.

---

## 1. Principles and non-goals

**Principles**

1. **One model, many views.** Each typed node is an ordinary rhizo node with a few extra fields, so it can sit in a mindmap, a flowchart and an architecture diagram at once. Layout and rendering never depend on profiles.
2. **Open specifications over bespoke formats.** Every output uses a published, versioned spec (JSON Schema 2020-12, OpenAPI 3.1, AsyncAPI 3, OpenTofu HCL, Serverless Workflow 1.0, MADR, …) or the runtime's own documented import format (Kestra YAML, n8n and Node-RED JSON). Proprietary formats are never reverse-engineered. Mendix shapes the UX ideas (typed canvases, data views bound to entities, microflows) but nothing is read from or written to its formats.
3. **The compiler does everything it can.** Whatever can be derived deterministically is generated, never left for an agent: schemas, API contracts, infrastructure, IAM policies, workflow wiring, tests and checklists. Agents fill *declared* gaps such as business logic and UI components, inside a declared write scope.
4. **Deterministic.** The same document, profile lockfile and rhizo version always produce byte-identical output. There is no clock, randomness, network or LLM inside the compiler.
5. **Verification decides "done".** A task is done when its gates pass (`tofu validate`, `tflint`, `checkov`/`trivy`, `tofu test`, generated contract tests, and an opt-in `infracost` budget check). An agent saying so is not enough.
6. **Traceable both ways.** Every generated file, resource, task and test carries the node id it came from. Every runtime object can be mapped back to a node.

**Non-goals**

- Executing workflows, provisioning infrastructure or hosting apps. rhizo never runs `tofu apply`. Apply belongs to CI or a human.
- Hiding the generated code. The generated HCL, YAML and JSON are meant to be read, reviewed and committed.
- Round-tripping arbitrary hand-written HCL or n8n workflows back into diagrams. Import is limited to *state* (overlays, §8) and to formats rhizo already imports for plain diagrams.
- An LLM in the loop of `rhizo compile`. Agents sit outside the compiler and talk to it through files and MCP.

---

## 2. Typed nodes in the core model

### 2.1 Model changes

Today a `Node` has `id, label, frame, parent, class, style, at, note, link, tags`, and an `Edge` has `from, to, label, arrow, class, style`. The proposal adds fields. None are removed and none change meaning.

```rust
// crates/core/src/model.rs (proposed additions)

id_type!(TypeName);  // namespaced: `aws.s3.bucket`, `flow.http-request`
id_type!(PortName);  // `in`, `out`, `invoke`, `true`, `false`, ...
id_type!(EdgeId);

/// Typed properties. Validated against the type's JSON Schema by a profile,
/// never by core. Key order is preserved (it is the author's order).
pub type Props = IndexMap<String, serde_json::Value>;

pub struct Node {
    // ... existing fields unchanged ...
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<TypeName>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub props: Props,
    /// Ports declared on this node. Types usually declare their ports in the
    /// profile; these add ports or refine one (e.g. give it a schema).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
    /// Authored acceptance criteria, in order. Stored only; see §4.6.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accept: Vec<String>,
}

pub struct Frame {
    // ... existing fields unchanged ...
    /// Frames can be typed too: a workflow, an environment, a screen.
    pub ty: Option<TypeName>,
    pub props: Props,
}

pub struct Port {
    pub name: PortName,
    pub dir: PortDir,                 // in | out
    pub kind: EdgeKind,               // which edges may attach
    pub schema: Option<String>,       // schema reference, e.g. `data:order` (§3.8)
    pub label: Option<String>,
}

#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    /// Values flow along the edge (a function writes to a bucket, a form
    /// submits an order, a task's output feeds the next task's input).
    Data,
    /// Ordering or triggering: "then", "on true", "navigates to", "invokes".
    Control,
    /// "Needs to exist first" / "is configured by". No value flows.
    Dependency,
}

pub struct Edge {
    // ... existing fields unchanged ...
    pub id: Option<EdgeId>,           // only needed when something must point at an edge
    pub kind: Option<EdgeKind>,       // None = a plain diagram edge
    pub from_port: Option<PortName>,
    pub to_port: Option<PortName>,
    pub props: Props,
}

pub struct Document {
    // ... existing fields unchanged ...
    /// Profiles the document uses (`use infra version="0.1"`). Resolution and
    /// version matching happen in `rhizo-profile`, not here.
    pub profiles: Vec<ProfileRef>,
}
```

`serde_json` moves from a dev-dependency of `rhizo-core` to a normal one, with the `preserve_order` feature so nested objects keep the author's key order too (without it, `serde_json::Map` is a `BTreeMap`). It is pure Rust and builds for `wasm32-unknown-unknown`, so the "no native deps" promise holds.

What core checks in `Document::validate()` (structure only, no profile knowledge):

- `TypeName` syntax: two or more dot-separated segments, each `[a-z][a-z0-9-]*`. The first segment is the namespace.
- Port names are unique per node. Edge ids are unique per document.
- `from_port`/`to_port` refer to a declared port, but only when the node is **untyped**. A typed node's ports come from its profile, which core cannot see, so that check moves to profile validation (§3.4).

Nothing else. Prop values, port compatibility and containment rules are profile concerns.

### 2.2 Type names

- **Namespaced and dotted**, most general part first: `aws.s3.bucket`, `aws.lambda.function`, `flow.http-request`, `flow.trigger.webhook`, `data.entity`, `ui.form`.
- **A profile owns one or more namespaces** (`infra` owns `aws`, `gcp`, `azure`, `infra`; `flow` owns `flow`; …). Loading two profiles that claim the same namespace is an error.
- The `rhizo` namespace is reserved for a small vocabulary that is always available: `rhizo.decision` (an ADR), `rhizo.baseline` (the security baseline), `rhizo.requirement` and `rhizo.actor`. See §4.5.
- **An unknown type is a warning, not an error**, unless `rhizo check --strict` is used. A sketch with `type=aws.dynamodb.table` must still render while the profile lacks that type.
- Versioning belongs to the profile (`use infra version="0.1"`), not to individual type names. Breaking changes to one type ship as a new profile major version with a migration (§3.6).

### 2.3 Props

`props` is a free JSON object on the node. Its shape is defined by the type's JSON Schema (2020-12). Props sit in a **separate map, not flattened** into the node like style properties. There are three reasons:

1. **No collisions.** `label`, `shape`, `link` and `x` are already node keys, and a type could reasonably define a prop with any of those names.
2. **One thing to validate.** The schema validates exactly `props`, and an error's JSON Pointer (`/props/lifecycle/0/expire-after-days`) maps one-to-one to a source span.
3. **A loud failure today.** Today's JSON loader *silently ignores* unknown node keys (verified: `{"label":"A","type":"aws.s3.bucket"}` passes `rhizo check` with `type` dropped). A nested `props` object makes the change visible, and §2.10 makes it fail loudly on old builds.

Prop keys are kebab-case like the rest of rhizo. A compiler maps them to whatever its target uses (`memory-mb` → `memory_size`).

Defaults: JSON Schema `default` values are **not** written back into the document. The compiler applies them during normalization (§4.1), so documents stay minimal and a profile upgrade can improve a default.

### 2.4 Ports

Ports tell an edge *which* input or output it connects to. Without them, "this function writes to that bucket" and "that bucket's events invoke this function" would be the same edge. Ports are optional everywhere:

- Most ports are **declared by the type** in its profile. `aws.lambda.function` declares `in invoke (control)`, `out calls (data)` and `out logs (data)`. `flow.branch` declares `in in (control)`, `out true (control)` and `out false (control)`.
- When an edge names no port, the profile picks the **default port** for that edge kind and direction. If none exists or the choice is ambiguous, the result is a diagnostic with a list of choices.
- A node can **declare extra ports or refine one**, for example to give the webhook's output a schema. Untyped nodes can declare ports too, for people who just want to draw port-accurate block diagrams.
- `schema` on a port names the shape of the values that pass through it. Data edges between ports whose schemas are both known are checked for compatibility (§3.4).

Rendering (phase 2): ports are drawn as small anchors on the node border on the side the layout direction implies (inputs toward the incoming edges, outputs toward the outgoing ones). Phase 1 stores ports but does not draw them.

### 2.5 Edge kinds

| Kind | Meaning | Default look (phase 2) | Examples |
|---|---|---|---|
| *(none)* | Plain diagram edge, no semantics | today's look | every existing document |
| `data` | Values flow from → to | solid | function → bucket (`access=write`), form → entity, task output → task input |
| `control` | Ordering or triggering | solid, filled arrow | branch `true` → task, API route → function, button → screen |
| `dependency` | "Needs first" / "configured by"; nothing flows | dashed, open arrow | function → VPC (runs in it), service → secret |

What each kind means depends on the profile. In `infra`, a `data` edge from compute to storage becomes a least-privilege IAM statement plus, for network services, a security group rule. In `flow` a `control` edge is sequencing, while in `ui` it is navigation. The kind tells the compiler which family of semantics to apply, and the edge's `props` (such as `access=read`) parameterize it.

When an edge names ports but no kind, its kind is **inferred** from the ports, and a mismatch between the two is an error.

### 2.6 Containment

Containment reuses `parent`, the existing nesting relation. Nested `-` lines already mean "child of", so a subnet nested in a VPC, or a field nested in an entity, needs no new concept. Types declare which parents they accept (§3.3).

One caveat is about drawing. Today nesting is rendered as a *tree* (mindmap or tidy tree). Architecture diagrams want nested **boxes** (compound nodes), which is a layout feature rhizo does not have yet (open question Q3). Until then, profiles use **typed frames** for the big containers (an environment, a workflow, a screen) and **props** for placement (`subnet=private`). That covers the worked examples below without compound layout.

### 2.7 Stable ids

Traceability (§7) hangs everything on node ids, but today ids are often *derived* from labels (`"Orders DB"` → `orders-db`), and `raise()` drops ids that equal the slug of the label. Renaming a label would then silently change the id and break every trace link, task and OpenTofu address. Rules for typed nodes:

1. **Typed nodes always keep an explicit id.** `raise()`/`rhizo fmt` write the id of every typed node even when it equals the slug, so the first `fmt` pins it.
2. `rhizo check` warns when a typed node has no explicit id in the source.
3. The compiler keeps the previous id set in the bundle manifest. When a node with the same type and props shows up under a new id, the compiler emits an OpenTofu `moved {}` block and a diagnostic, so a rename doesn't destroy and recreate the resource.

### 2.8 KDL syntax

New statements and properties (everything else is unchanged):

| Where | Syntax | Meaning |
|---|---|---|
| top level | `rhizo version=2` | The file uses typed features. Required when any of the following appear. |
| top level | `use infra version="0.1"` | Use a profile (§3.6). Repeatable. |
| node / frame | `type=aws.s3.bucket` | The type. KDL v2 bare identifiers allow dots, so no quotes are needed. |
| node / frame / edge block | `props k=v … { … }` | The props object (mapping below). |
| node block | `port out "events" kind=data schema=data:s3-event` | Declare or refine a port. |
| node block | `accept "…"` | One acceptance criterion. Repeatable. |
| edge | `kind=data` / `kind=control` / `kind=dependency` | The edge kind. |
| edge | `from-port=true to-port=invoke` | Ports at each end. |
| edge | `id=orders-write` | Optional edge id. |
| edge block | `edge a b { props access=write }` | Edge props. Today's parser rejects any block on `edge` (`no_children`), so this is new. |

**KDL → JSON mapping for `props`.** The `props` node is a **JSON-in-KDL (JiK) 4.0.0 object node**. JiK is the official KDL microsyntax for JSON values, and its 4.0.0 version uses KDL v2 syntax. Adopting it exactly means rhizo invents nothing, and the mapping is someone else's tested spec.

```kdl
props versioning=#true storage-gb=20 {   // properties on `props` → scalar keys
    owner "team-orders"                   // child with one argument → scalar
    cors-origins "https://a.example" "https://b.example"   // several arguments → array
    tags { cost-center "cc-42"; env "prod" }               // child with a block → object
    lifecycle {                                            // `-` children → array items
        - prefix="tmp/" expire-after-days=7                //   item with properties → object
        - prefix="logs/" expire-after-days=90
    }
    (array)azs "eu-central-1a"            // (array) annotation: a one-element array
    kms-key #null                         // explicit null
}
```

- These are exactly JiK's rules:
  - Properties and named children are object keys.
  - A node with one argument is a scalar, and a node with several arguments or with `-` children is an array.
  - `(array)` and `(object)` annotations disambiguate the one-element and empty cases.
- One rhizo convenience sits on top. When the type's schema is known, a single argument where the schema expects an array is accepted as a one-element array, so authors rarely need `(array)`. `rhizo fmt` writes the canonical JiK form.
- Every prop value keeps its source span, held in a side table keyed by JSON Pointer that is never serialized. A schema error at `/props/lifecycle/1/expire-after-days` then underlines exactly that `90`.

Prior forms were considered and rejected:

- **KDL type annotations**, `(aws.s3.bucket)node "Uploads"`. They are legal KDL v2, but annotations are meant for *value* types, are easy to miss when reading, and have no JSON/YAML counterpart. `type=` keeps parity across the three syntaxes.
- **Props inline on the node**, `node "Uploads" type=aws.s3.bucket versioning=#true`. This collides with node and style keys and makes the parser's unknown-property diagnostics useless.

A complete typed node:

```kdl
rhizo version=2
use infra version="0.1"

node "Receipts" id=receipts type=aws.s3.bucket {
    props versioning=#true {
        lifecycle { - prefix="tmp/" expire-after-days=7 }
    }
    port out "created" kind=data schema=data:s3-object-created
    accept "Objects are encrypted at rest and the bucket rejects non-TLS requests"
    note "Customer receipts as PDF. Retention is a legal requirement."
}

edge orders-fn receipts kind=data { props access=write prefix="receipts/" }
```

### 2.9 JSON and YAML

The JSON and YAML forms use the same schema (`rhizo schema` regenerates it). A typed document carries a top-level `"rhizo": 2`, mirroring the KDL header:

```json
{
  "rhizo": 2,
  "use": [{ "profile": "infra", "version": "0.1" }],
  "nodes": [
    {
      "label": "Receipts",
      "id": "receipts",
      "type": "aws.s3.bucket",
      "props": {
        "versioning": true,
        "lifecycle": [{ "prefix": "tmp/", "expire-after-days": 7 }]
      },
      "ports": [{ "name": "created", "dir": "out", "kind": "data", "schema": "data:s3-object-created" }],
      "accept": ["Objects are encrypted at rest and the bucket rejects non-TLS requests"],
      "note": "Customer receipts as PDF. Retention is a legal requirement."
    }
  ],
  "edges": [
    { "from": "orders-fn", "to": "receipts", "kind": "data",
      "props": { "access": "write", "prefix": "receipts/" } }
  ]
}
```

```yaml
rhizo: 2
use: [{ profile: infra, version: "0.1" }]
nodes:
  - label: Receipts
    id: receipts
    type: aws.s3.bucket
    props:
      versioning: true
      lifecycle: [{ prefix: tmp/, expire-after-days: 7 }]
edges:
  - { from: orders-fn, to: receipts, kind: data, props: { access: write } }
```

The edge string shorthand (`"a -> b"`) stays and always means an untyped edge. A typed shorthand such as `"a.out -> b.in : data"` is tempting but is left as open question Q5.

### 2.10 Plain diagrams are unaffected

These are guarantees, each backed by a test in phase 1:

1. **Same bytes.** For every file in `examples/` and every interop fixture, `rhizo fmt`, `rhizo render` (SVG) and every exporter produce identical output before and after the change. That is checked by golden files.
2. **No new required syntax.** `rhizo version=2` and `"rhizo": 2` are needed only when a typed feature is used. Writing `type=` in a version-1 file is an error, and its help text says "add `rhizo version=2`".
3. **Old builds fail loudly on new files instead of dropping data.** In KDL, old builds already reject `type=`, `props` and `kind=` as unknown properties (verified). In JSON/YAML, the top-level `File` has `deny_unknown_fields`, so `"rhizo": 2` makes an old build reject the file (verified: `unknown field 'rhizo'`). Without that marker, old builds would silently drop `type`, `props` and `kind` from nodes and edges. That is why the writer always emits the marker when a typed feature is present.
4. **No layout or rendering effect** until a profile is loaded (phase 2). Even then, profile styling slots into the existing cascade (*theme → layout defaults → **type defaults** → classes → inline*), so inline style always wins.
5. Phase 1 also fixes the silent-drop behavior for future versions. Node and edge objects in JSON/YAML reject unknown keys with a "did you mean" diagnostic, matching the KDL parser. serde's `deny_unknown_fields` doesn't compose with the flattened `Style`, so this takes a small custom visitor or a captured `#[serde(flatten)] extra: Map` that is checked after deserializing.

**Interop.** Formats that have a place for custom data keep type, props and kind: GraphML as `<data>` keys (props as a JSON string) and draw.io as attributes on the cell's `<object>` wrapper. Everything else (Mermaid, D2, DOT, OPML, FreeMind, Markdown, JSON Canvas) drops them, and `rhizo convert` prints one warning per conversion, not one per node.

---

## 3. Profiles

### 3.1 What a profile is

A profile bundles:

| Part | Contents |
|---|---|
| **Vocabulary** | Node and frame types: a props JSON Schema, ports, allowed parents, label template, default shape, icon and style |
| **Rules** | Graph-level validation beyond what a schema can say, each rule with an id, severity, help text and baseline mapping |
| **Compilers** | One or more targets (`opentofu`; `kestra`, `n8n`, `node-red`, `serverless-workflow`; …) that turn the validated model into bundle files, tasks and tests |
| **Catalog** | Pinned external things the compiler references, such as registry modules with exact versions, provider versions and runtime plugin versions |
| **Overlay importers** | Optional. Turn runtime output (`tofu show -json`, an execution API response) into overlays (§8) |

### 3.2 Package layout

```
infra/
  profile.kdl                 # manifest, types, rules index
  schemas/aws.s3.bucket.json  # one JSON Schema 2020-12 document per type
  schemas/...
  rules/*.rego                # declarative rules (§3.4)
  catalog.kdl                 # pinned modules/providers
  icons/*.svg
  compiler.wasm               # plugin profiles only (§3.5)
```

```kdl
// profile.kdl
profile infra version="0.1.0" {
    description "Cloud infrastructure, compiled to OpenTofu"
    requires rhizo=">=0.4"
    namespaces aws infra
    targets opentofu
}

type infra.environment frame=#true {
    label "Environment"
    schema "schemas/infra.environment.json"   // provider, region, state backend, budget
}

type aws.s3.bucket {
    label "S3 bucket"
    schema "schemas/aws.s3.bucket.json"
    shape rect
    icon "icons/aws-s3.svg"
    parents infra.environment                 // allowed containers (node parent or typed frame)
    port in "write" kind=data default=#true
    port out "read" kind=data
    port out "created" kind=data schema=data:s3-object-created
    realize module="terraform-aws-modules/s3-bucket/aws"
}
```

Schemas are plain JSON Schema files, so profile authors can use ordinary JSON Schema tooling and editors. rhizo-specific annotations live under an `x-rhizo` keyword. JSON Schema 2020-12 treats unknown keywords as annotations, so a standard validator ignores them.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://rhizo.dev/profiles/infra/0.1/aws.s3.bucket.json",
  "title": "S3 bucket",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "versioning":  { "type": "boolean", "default": true },
    "public":      { "type": "boolean", "default": false,
                     "x-rhizo": { "rule": "infra.s3.no-public", "explain": "Public buckets need an `allow` with a reason." } },
    "kms-key":     { "type": ["string", "null"], "default": null,
                     "description": "Node id of an aws.kms.key, or null for SSE-S3" },
    "lifecycle": {
      "type": "array",
      "items": {
        "type": "object", "additionalProperties": false,
        "required": ["expire-after-days"],
        "properties": {
          "prefix": { "type": "string", "default": "" },
          "expire-after-days": { "type": "integer", "minimum": 1 }
        }
      }
    }
  }
}
```

### 3.3 What a type definition can say

- `schema`: the props schema.
- `ports`: name, direction, kind, optional schema, default flag.
- `parents`: which types may contain this one, through a node `parent` or a typed frame. `parents none` means it must be top-level.
- `label`: a template for when the author gave no label, e.g. `"{props.method} {props.path}"`.
- `shape`, `icon`, `style`: default rendering, slotted into the style cascade.
- `realize`: an opaque hint for the profile's own compiler (module source, runtime task type). Validation ignores it.

### 3.4 Validation in layers

| Layer | Who | Examples | Where diagnostics point |
|---|---|---|---|
| 1. Structure | `rhizo-core` | duplicate ids, bad type-name syntax, unknown frame | node span |
| 2. Schema | `rhizo-profile` | props vs JSON Schema; port exists; edge kind matches port kind; `out`→`in`; containment allowed | exact prop span (JSON Pointer → span) |
| 3. Compatibility | `rhizo-profile` | data edge from port schema A to port schema B: is A assignable to B? (structural check on the two JSON Schemas) | edge span |
| 4. Rules | each profile | "an RDS instance must not be reachable from a public subnet", "a flow needs exactly one trigger", "a form that writes PII needs an authenticated screen" | node or edge span, plus `help` and baseline id |

**Schema validation.** It uses the [`jsonschema`](https://crates.io/crates/jsonschema) crate (0.59 as of 2026-10). The crate supports 2020-12 and is pure Rust, but it is still pre-1.0 with a few known failures against the official test suite (see Bowtie). Profiles should therefore stick to mainstream keywords. JSON Schema 2020-12 is still the latest published dialect: the announced stable "v1" release has not shipped.

**Rules.** Built-in profiles write their rules in Rust: that's fast, they're tested with the profile, and they get spans for free. For plugin profiles and for user-local policy (house rules such as "every bucket has an `owner` tag"), the proposal is **Rego evaluated in-process with [regorus](https://github.com/microsoft/regorus)** against the model's JSON. Rego is already the lingua franca of IaC policy (OPA, Conftest), so teams can reuse their skills and often their policies. regorus (0.13, pre-1.0) is "mostly compliant" with OPA. A few builtins are missing (JWT, `net.cidr*`, GraphQL), but the gaps don't matter for model-level rules, and it builds for WASM and `no_std`. The alternative is CEL via the `cel` crate (0.15): simpler and side-effect free, but less known in this space. See open question Q7.

Every rule has:

```kdl
rule infra.rds.private severity=error {
    message "RDS instances must not be publicly accessible"
    help "remove `publicly-accessible` or add: allow infra.rds.private reason=\"...\""
    maps-to "cis-aws-foundations" "checkov:CKV_AWS_17"   // illustrative ids, see §4.5
}
```

**Suppression is explicit and visible.** A node writes `allow infra.s3.no-public reason="Public website assets"`. The compiler copies every `allow` into `constraints/exceptions.md` in the bundle, so reviewers see each waived rule and its justification in one place.

### 3.5 Built-in and plugin profiles

Both kinds implement one interface. Built-ins implement it in Rust inside the `rhizo` binary. Plugins implement it as **WebAssembly components** described by WIT.

```rust
// crates/profile/src/lib.rs
pub trait Profile: Send + Sync {
    fn manifest(&self) -> &Manifest;                       // types, ports, rules, targets
    fn validate(&self, doc: &Document, cx: &Cx) -> Vec<Diagnostic>;
    fn compile(&self, doc: &Document, target: &str, cx: &Cx, out: &mut Bundle) -> Result<(), Vec<Diagnostic>>;
    fn import_overlay(&self, source: &str, input: &[u8], cx: &Cx) -> Result<Overlay, Vec<Diagnostic>> { … }
}
```

```wit
// wit/profile.wit
package rhizo:profile@0.1.0;

interface types {
    enum severity { error, warning, info }
    record diagnostic {
        severity: severity,
        code: string,              // rule id
        message: string,
        node: option<string>,      // node id
        pointer: option<string>,   // JSON Pointer into the node, e.g. /props/lifecycle/0
        help: option<string>,
    }
    record file {
        path: string,              // bundle-relative, forward slashes
        contents: list<u8>,
        nodes: list<string>,       // node ids this file derives from (trace)
        owner: ownership,          // §4.4
    }
    enum ownership { generated, scaffold }
}

world profile {
    use types.{diagnostic, file};
    /// profile.kdl and schemas, lowered to JSON
    export manifest: func() -> string;
    /// `model` is the normalized document as JSON (the `rhizo schema --model` shape)
    export validate: func(model: string) -> list<diagnostic>;
    export compile: func(model: string, target: string, options: string) -> result<list<file>, list<diagnostic>>;
    export import-overlay: func(source: string, input: list<u8>) -> result<string, list<diagnostic>>;
}
```

**The world imports nothing.** There is no filesystem, network, clock or randomness, so a plugin is **deterministic and sandboxed by construction**. It cannot leak your model, phone home or produce timestamps. The host enforces fuel (instruction budget) and memory limits.

**Platform status (2026-10).**
- WASI 0.3 (native async) was ratified in June 2026, and 0.2 is still supported. Component Model 1.0 has no date yet.
- wasmtime is at 49 and `wit-bindgen` at 0.63. Both are pre-1.0 in their interfaces, so the WIT package is versioned (`@0.1.0`) and the host may support two versions side by side.
- A world with **no WASI imports** sidesteps most of this churn, and it also runs in the browser: `jco transpile` turns a component into an ES module. jco's WASI 0.3 shim is Node-only today, which doesn't matter when nothing imports WASI.
- Extism was considered and rejected. It is mature, but it uses core modules plus its own ABI rather than the Component Model, so plugin interfaces would not be described in WIT.

**Distribution.** Plugins are published as OCI artifacts in the CNCF TAG Runtime *Wasm OCI artifact* layout (`application/vnd.wasm.config.v0+json` with one `application/wasm` layer), the layout `wkg`, containerd and others already use. They are pinned by digest in `rhizo.lock` and verified with cosign/Sigstore signatures when present. `wkg` itself has no signing, so signature verification is rhizo's job.

**Why the four starting profiles are built in.** They need fast iteration while the WIT interface is still moving, and they define what "good" looks like for plugin authors. Moving one built-in to a component (phase 9) proves the interface is sufficient.

### 3.6 Resolution, versions, lockfile

- `use infra version="0.1"` accepts semver ranges in Cargo's caret syntax. The resolved exact version and the digest of the profile's content are written to `rhizo.lock` next to the document.
- **The lockfile is part of determinism.** The catalog (module versions, provider versions) belongs to the profile version, so upgrading the profile is the one place where outputs change for reasons outside the document. `rhizo compile --locked` (CI) refuses to update the lockfile.
- Profile major versions may ship a `migrate` function (props renames, type renames) that `rhizo fmt --migrate` applies to the source. `migrate` is a rewrite of the source, not a compatibility shim.

### 3.7 The four starting profiles

#### `infra` → OpenTofu

| Type | Realized as (catalog) | Key props | Notes |
|---|---|---|---|
| `infra.environment` (frame) | root module + backend + provider | `provider`, `region`, `state-bucket`, `budget-monthly` | One environment per OpenTofu root module |
| `aws.vpc` | `terraform-aws-modules/vpc/aws` | `cidr`, `azs`, `nat` (`none`/`single`/`per-az`) | Subnets are module inputs, not separate nodes (yet, see Q3) |
| `aws.lambda.function` | `terraform-aws-modules/lambda/aws` | `runtime`, `memory-mb`, `timeout-s`, `handler`, `package` | Code is a gap (§4.4) |
| `aws.apigw.http` | `terraform-aws-modules/apigateway-v2/aws` | `cors-origins`; routes come from control edges | |
| `aws.rds.postgres` | `terraform-aws-modules/rds/aws` | `engine-version`, `instance-class`, `storage-gb`, `multi-az` | Encrypted, private and deletion-protected by default |
| `aws.s3.bucket` | `terraform-aws-modules/s3-bucket/aws` | `versioning`, `lifecycle`, `kms-key`, `public` | Public access blocked, TLS-only policy |
| `aws.sqs.queue` | `terraform-aws-modules/sqs/aws` | `fifo`, `retention-s`, `dlq` | |
| `aws.kms.key` | `terraform-aws-modules/kms/aws` | `rotation` | |

**Edge semantics in `infra`.**
- A `data` edge from compute to storage or a queue becomes a least-privilege IAM statement whose actions come from `access` (`read`/`write`/`read-write`) and whose resource is narrowed by `prefix` when given.
- A `data` edge to a network service becomes a security group ingress rule on the service's port, from the source's security group.
- A `control` edge from an API to a function becomes a route plus the invoke permission.
- A `dependency` edge from compute to a VPC places the compute in its private subnets. Any other `dependency` edge becomes `depends_on`, but only when no implicit reference exists already.

**Why registry modules.** Well-maintained community modules encode years of edge cases, and reviewers already know them. That makes them the safest thing an agent-free compiler can emit.

Modules are resolved through the OpenTofu Registry, which indexes terraform-aws-modules, terraform-google-modules and Azure Verified Modules. The registry is an **index, not a vetting body**. Submissions are screened only for exclusion categories such as malware and leaked secrets, and there is **no "verified" tier**. So "vetted" is a property of **rhizo's catalog**, and the catalog needs a written admission policy (Q9). The catalog pins **exact** versions, never ranges. Phase 4 starts with AWS only. GCP and Azure follow the same pattern (Q9).

#### `flow` → Kestra, n8n, Node-RED, Serverless Workflow

A typed frame `flow.workflow` is one workflow. Its node types:

| Type | Ports | Kestra | n8n | Node-RED | Serverless Workflow 1.0 |
|---|---|---|---|---|---|
| `flow.trigger.webhook` | out `body` | Webhook trigger | Webhook node | `http in` (+ `http response`) | workflow `input` (started by the engine's own HTTP API) |
| `flow.trigger.schedule` | out `fired` | Schedule trigger | Schedule Trigger | `inject` (repeat) | `schedule` |
| `flow.http-request` | in, out `response` | HTTP Request task | HTTP Request node | `http request` | `call: http` |
| `flow.branch` | in, out `true`/`false` | `If` | `IF` | `switch` | `switch` |
| `flow.db.query` | in `params`, out `rows` | JDBC PostgreSQL Query | Postgres node | (contrib node; capability gap) | custom function (capability gap) |
| `flow.emit` | in | target-specific publish task | target-specific node | target-specific node | `emit` (CloudEvent) |
| `flow.log` | in | `Log` | (no-op / Code) | `debug` | `set` + engine log |
| `flow.subflow` | in, out | `Subflow` | Execute Workflow | `link call` | `run: workflow` |

Verified type strings (2026-10):
- **Kestra 2.0:** `io.kestra.plugin.core.trigger.Webhook`, `…core.trigger.Schedule`, `…core.http.Request`, `…core.flow.If`/`Switch`, `…core.log.Log`, `…core.flow.Subflow`, `io.kestra.plugin.jdbc.postgresql.Query`.
- **n8n 2.x:** `n8n-nodes-base.webhook`, `…httpRequest`, `…if`, `…postgres`. Every n8n node also carries a `typeVersion`, which the catalog pins (for example `if` 2.4, `httpRequest` 4.5).
- **Node-RED:** Postgres is not a core node, so it needs a contrib package. The catalog would have to pin and vet one, which makes it a capability gap for now.

**Naming.** The CNCF project now calls the spec the **Open Workflow Specification**. The repo moved to `open-workflow-specification/specification`, though released schemas still use `serverlessworkflow.io` ids. The DSL is the same 1.0 line (latest release 1.0.1, 2026-07-31). The project is still CNCF **Sandbox**. This doc keeps the familiar "Serverless Workflow" name, and the target id is `serverless-workflow`.

**Capability matrix.** Each target declares which types and features it supports. Compiling a workflow that uses an unsupported type for a chosen target is an **error on that node, naming the target**. It is never a silent approximation. `targets="kestra serverless-workflow"` on the workflow frame picks the targets.

**Portable expressions.** The runtimes use different expression languages: Kestra uses Pebble templates, n8n JavaScript expressions, Node-RED message properties and JSONata, and Serverless Workflow jq. rhizo defines a deliberately tiny portable layer and compiles it to each:

- *References*: `<node-id>.<port>[.<path>]`, e.g. `received.body.total`.
- *Predicates* (for `flow.branch`): comparisons, `and`/`or`/`not`, literals and references. No function calls.
- *Templates*: `"{{ received.body.id }}"` inside string props. A string that is exactly one `{{ ref }}` passes the referenced value with its type (number stays number), which is how `params` bind values.
- *Escape hatch*: `expr.kestra="…"`, `expr.jq="…"`. Such a node is portable only to targets that have that dialect, and the capability matrix enforces it.

**Structured vs wired targets.** n8n and Node-RED are wired graphs, so any rhizo control graph maps directly. Kestra nests branches (`If` with `then`/`else`). Serverless Workflow can jump with `then: <task>` flow directives. For Kestra the compiler must recover structured regions: a branch whose arms re-join at a single node. Anything else gets a diagnostic suggesting a `flow.subflow`.

#### `data` → JSON Schema, OpenAPI, AsyncAPI

| Type | Notes |
|---|---|
| `data.entity` | Children of type `data.field` are its properties. Nesting is the natural mindmap way to sketch a data model. |
| `data.field` | `type`, `format`, `required`, `pii` (`none`/`personal`/`sensitive`), `enum`, `min`/`max` |
| `data.enum` | Values as children |
| `data.api` (frame) | One OpenAPI document: `title`, `version`, `servers`, `auth` (`oauth2`/`api-key`/`none`) |
| `data.operation` | `method`, `path`; data edges from entities become request/response bodies |
| `data.event` / `data.channel` | AsyncAPI messages and channels |

- Outputs are JSON Schema 2020-12 per entity, **OpenAPI 3.1** by default and AsyncAPI 3.
  - OpenAPI 3.2.0 shipped in September 2025, and 3.2.1 followed in September 2026. Linters lag behind: Spectral's 3.2 support is unclear. So 3.1, already aligned with JSON Schema 2020-12, is the default, and `api-version="3.2"` on a `data.api` frame opts in.
  - AsyncAPI 3.1.0 (January 2026) is a non-breaking minor of 3.0, and the profile emits `asyncapi: 3.1.0`.
- **Arazzo** 1.1 (OpenAPI Initiative) describes sequences of API calls. A `flow.workflow` made only of `flow.http-request` nodes against `data.operation`s can also be exported as an Arazzo document for API-journey docs and tests. That export is later work (Q15).
- Relations between entities are `data` edges with `cardinality` (`one`/`many`) and become `$ref`s.
- `pii` is the one cross-profile property that matters most. It drives logging redaction acceptance criteria, encryption requirements in `infra` (a `sensitive` field stored in a bucket requires a KMS key, as a rule) and authentication requirements in `ui`.

#### `ui` → a stack-neutral screen spec

| Type | Notes |
|---|---|
| `ui.app` (frame) | Navigation root, auth requirement, design tokens |
| `ui.screen` | Route, title, `auth` (`public`/`signed-in`/role) |
| `ui.form` | Binds to a `data.entity` (data edge); submits to a `data.operation` or to a workflow's trigger node (control edge; edges connect nodes, not frames) |
| `ui.list` / `ui.table` | Binds to a collection operation; columns as children |
| `ui.detail` | Shows one entity |
| `ui.action` | Button or link; a control edge to a screen (navigation) or an operation |

There is no widely adopted, open, stack-neutral screen-spec standard. The candidates as of 2026-10:

| Candidate | Status | Why it doesn't fit as the main output |
|---|---|---|
| JSON Forms UI Schema | Active (3.8, MIT); defined by a library, with no standards body | Forms only, no screens or navigation. Used for forms (below). |
| A2UI (Google-led, Apache-2.0) | v0.9.x, 1.0 RC open | Built for agents *generating* UI at runtime; still pre-1.0 |
| MCP Apps (`io.modelcontextprotocol/ui`) | Final MCP extension | HTML resources shown in an MCP host, not an app spec |
| Open UI (W3C CG) | Active | Standardizes native HTML controls; not a UI description format |

So `ui` emits:

- **Its own small, versioned, JSON Schema-described screen spec** (`ui/screens/<id>.json`): screens, regions, components, bindings and navigation. It's meant as a contract an agent implements in whatever stack the ADRs choose, not as a renderer format.
- **JSON Forms UI schemas** for `ui.form`, next to the JSON Schema from `data`. JSON Forms is an open, documented format with existing renderers, so a team can render forms without writing them.
- **Design tokens** in the W3C Design Tokens Community Group format. Its 2025.10 release is the first stable one. It is a Community Group report, not a W3C Recommendation.
- **Acceptance criteria per screen**: navigation paths, required fields, error states and a WCAG 2.2 AA baseline. WCAG 2.2 is a W3C Recommendation, while WCAG 3.0 is still a Working Draft. They are written as Given/When/Then so they compile into end-to-end tests for whatever stack is chosen.

Mendix is the UX reference here: pages built from data views bound to entities, with actions calling microflows. rhizo makes the same ideas explicit as typed nodes and edges, and emits open specs rather than a proprietary model.

### 3.8 Cross-profile references

A whole system is one document, or several documents that import each other (Q6), with the profiles side by side. References cross profiles in two ways:

- **Edges.** A `ui.form` data edge to a `data.entity`. A `flow.http-request` control edge to a `data.operation`, which gets the operation's URL, method and schema. An `aws.lambda.function` dependency edge to a `data.operation`, which makes the function the implementation of that operation.
- **Schema references in ports and props.** `schema=data:order` means "the JSON Schema the `data` profile generates for the node with id `order`".

The compiler runs one **link pass** after per-profile validation (§4.1). Every cross-profile reference must resolve, and the link pass checks that the referenced node has a type the referring profile accepts. Most of the system's value comes from here. The webhook's input schema, the API contract, the Lambda's IAM policy and the form's fields all come from the same `order` node, so they cannot drift apart.

---

## 4. The spec compiler

### 4.1 Pipeline

```
parse ─► lower ─► resolve profiles (rhizo.lock)
      ─► validate: core → schema → compatibility → rules      (errors stop here)
      ─► normalize: apply schema defaults, infer edge kinds and default ports,
                    pin derived ids, expand label templates
      ─► per-profile IR
      ─► link: resolve cross-profile references
      ─► emit: each target writes files (generated or scaffold)
      ─► derive: constraints, task graph, tests, verification plan
      ─► write bundle + manifest.json + trace.json
```

`rhizo compile shop.rhizo -o build/` runs the whole pipeline. `rhizo check` stops after *validate*, and `rhizo plan` stops after *derive* and prints what would change against the existing bundle.

### 4.2 The determinism contract

**Same document, same `rhizo.lock` and same rhizo version give the same bytes.** Enforced by:

- **Ordering.** Iteration follows document order (the `IndexMap`s already preserve it). Any set is sorted by id. Nothing iterates a `HashMap` into output.
- **No ambient inputs.** Nothing is taken from the environment, the clock, randomness, the hostname or the network. Variable inputs (account ids, domain names, secrets) become **declared variables without defaults** and are never guessed.
- **Canonical formatting.** HCL is emitted in `tofu fmt` canonical form (CI checks this with `tofu fmt -check`). YAML and JSON use one canonical writer with sorted keys only where order has no meaning.
- **Stable derived names.** OpenTofu addresses come from node ids (`orders-db` → `module.orders_db`). Kestra task ids are node ids. n8n node ids are UUIDv5 values of `(document id, node id)`.
- **A test in CI.** Every golden example is compiled twice in different working directories and environments and compared byte for byte.

`manifest.json` records the inputs (document hash, lockfile hash, rhizo version) and every output file with its sha256, owner and node ids. It doubles as provenance: a later phase can wrap it as an in-toto/SLSA provenance statement.

### 4.3 Bundle layout

```
build/
  manifest.json                     # inputs, outputs, hashes, owners, node ids
  trace.json                        # file → ranges → node ids (§7.2)
  AGENTS.md                         # how to work in this bundle; points at MCP and TASKS.md
  specs/
    schemas/order.schema.json       # data
    openapi.yaml                    # data (OpenAPI 3.1)
    asyncapi.yaml                   # data (AsyncAPI 3)
  infra/prod/                       # infra, one dir per environment
    versions.tf providers.tf main.tf iam.tf network.tf
    variables.tf outputs.tf moved.tf
    encryption.tofu                 # OpenTofu-only syntax lives in .tofu files
    tests/prod.tftest.hcl
    .tflint.hcl .checkov.yaml
  flows/
    kestra/shop.orders/order-intake.yaml
    serverless-workflow/order-intake.sw.yaml
    n8n/order-intake.json
    node-red/flows.json
  ui/
    screens/*.json  forms/*.uischema.json  tokens.json  navigation.json
  app/                              # scaffolds (written once, then owned by agents/humans)
    orders-fn/src/handler.ts
  constraints/
    adr/0001-adr-backend.md         # MADR, from rhizo.decision nodes
    security-baseline.md            # from rhizo.baseline + profile rules
    exceptions.md                   # every `allow` with its reason
    stack.json                      # machine-readable stack constraints
  tasks/
    tasks.json                      # the task graph (§4.6)
    TASKS.md                        # the same, for agents without MCP
  tests/
    contract/                       # Schemathesis config per API
    acceptance/*.feature            # Given/When/Then from accept + profile criteria
  verify.json                       # the verification plan (§4.7)
```

**`.tf` vs `.tofu`.** OpenTofu (since 1.8) also reads `.tofu` files, but the tools around it disagree:
- TFLint reads only `.tf`/`.tf.json`. Its maintainers have said there are no plans to support OpenTofu, and `.tofu` support was closed as "not planned" in July 2026.
- Checkov reads only `.tf`, `.tf.json` and `.hcl`.
- Trivy reads `.tofu`.

The rule is therefore **everything in `.tf`, except OpenTofu-only syntax, which goes in `.tofu` files** whose base name matches no `.tf` file. Today that is just the state `encryption` block. OpenTofu merges the extra `terraform {}` block, while TFLint and Checkov never see a construct they would reject or misread. The compiler enforces this split, and the gate set accounts for it: the encryption block is validated only by `tofu validate`, and an integrity rule in `rhizo check` requires it to be present whenever an environment has a remote backend.

### 4.4 Ownership: generated, scaffold, agent-owned

| Owner | Written by | Regenerated | May agents edit? | How it's enforced |
|---|---|---|---|---|
| `generated` | compiler | every compile | **No.** To change one, change the model (MCP `propose_change`, §7). | Header `@generated by rhizo … do not edit`; `rhizo verify` compares hashes with `manifest.json` |
| `scaffold` | compiler, **once** | never overwritten (a new scaffold appears as `*.rhizo-new` with a diagnostic) | Yes, within the task's write scope | The manifest records its origin. Verify checks the exported signatures still match the contract. |
| agent/human | agents, humans | — | Yes, within the write scope | `rhizo verify` flags writes outside every task's scope |

This draws the line between "compiler output" and "gap". It works like generated protobuf stubs next to hand-written implementations. Expected gaps by profile:

| Profile | Gaps left for agents | Gaps left for humans |
|---|---|---|
| infra | normally none; only `custom` escape-hatch nodes | variable values (account, domains), `tofu apply` |
| data | none | — |
| flow | `flow.script` bodies; target features outside the capability matrix | secrets in the runtime |
| ui | the components implementing each screen in the chosen stack | visual design review |
| app (implied by compute nodes) | handler bodies, DB migrations, domain logic | product review |

### 4.5 Constraints: stack ADRs and the security baseline

Constraints are typed nodes from the reserved `rhizo` namespace, so they live in the diagram, are linked to the nodes they govern and get traced like everything else:

```kdl
frame "Decisions" layout=grid {
    node "Backend runtime" id=adr-backend type=rhizo.decision {
        props status=accepted decision="TypeScript on Node.js 22, AWS Lambda" {
            drivers "Team skills" "Cold-start budget under 300 ms"
            options "TypeScript/Node.js 22" "Python 3.13" "Go 1.25"
        }
    }
    node "Security baseline" id=baseline type=rhizo.baseline {
        props asvs-level=2 cis="aws-foundations" ssdf=#true
    }
}
edge adr-backend orders-fn kind=dependency
```

- `rhizo.decision` nodes become **MADR 4.0** files (`constraints/adr/NNNN-<id>.md`) with context taken from the linked nodes. Each task lists the ADRs that apply to its nodes, so an agent cannot "not know" the stack.
- `rhizo.baseline` selects the **security baseline**:
  - **OWASP ASVS 5.0** (May 2025, about 350 requirements in 17 chapters, CC BY-SA 4.0) at a chosen level, for application code. The compiler turns relevant ASVS requirements into acceptance criteria on the nodes they apply to: authentication for screens marked `signed-in`, input validation for every operation with a body, logging rules for every `pii` field. ASVS 5.0 renumbered everything relative to 4.0, so mapping tables must reference 5.0 ids explicitly. The OWASP Top 10:2025 categories can tag findings, but they are an awareness list, not a verification standard.
  - **CIS Benchmarks** for cloud configuration (current: AWS Foundations v7.1.0, Azure Foundations v6.0.0, GCP Foundations v5.0.0). Two caveats shape the design:
    - **CIS licensing limits redistributing benchmark content and mappings.** Profiles therefore ship only rule→benchmark *ids*, never benchmark text.
    - **Neither scanner gives an authoritative CIS report for IaC.** Trivy's `--compliance` reports cover images and Kubernetes, not `trivy config`. Checkov's CIS-mapped reporting appears to live in the commercial Prisma Cloud platform. rhizo's own rules, plus the scanners' individual checks, are what get enforced. "CIS-aligned" in `security-baseline.md` means *these listed controls are checked*, never *compliant*.
  - **NIST SSDF** practices as process-level criteria: dependency pinning, signed artifacts, code review. The final version is SP 800-218 v1.1; Rev. 1 (SSDF 1.2) is still an initial public draft. **SLSA 1.2** levels give the provenance target for the bundle itself (§4.2).
- `security-baseline.md` lists every requirement, where it applies (node ids) and how it is verified (the rule id, the scanner check or the generated test). Requirements that can be verified only by review are marked **manual**, so the baseline never overclaims.

Mapping ids such as `CKV_AWS_17` or an ASVS requirement number are illustrative in this document. The real mapping tables live in the profiles, are versioned with them and need their own review (Q8).

### 4.6 Task graph and acceptance criteria

The compiler derives one task per gap (§4.4) plus one per human step. Tasks are **derived, not authored**: they are deterministic functions of the model, so recompiling after a model change updates them, and their ids are stable (`T-<node-id>-<kind>`).

```json
{
  "rhizo-tasks": 1,
  "source": { "document": "shop.rhizo", "sha256": "…" },
  "tasks": [
    {
      "id": "T-orders-fn-implement",
      "title": "Implement the Orders function (POST /orders)",
      "kind": "implement",
      "nodes": ["orders-fn", "create-order", "order"],
      "depends-on": [],
      "inputs": [
        "specs/openapi.yaml#/paths/~1orders/post",
        "specs/schemas/order.schema.json",
        "infra/prod/outputs.tf"
      ],
      "write-scope": ["app/orders-fn/**"],
      "constraints": ["constraints/adr/0001-adr-backend.md", "baseline:asvs:input-validation", "baseline:asvs:logging"],
      "acceptance": [
        { "id": "AC-1", "source": "authored", "text": "p95 latency under 300 ms at 50 rps",
          "verify": { "kind": "manual", "note": "load test in staging" } },
        { "id": "AC-2", "source": "data", "text": "Rejects a body that does not match order.schema.json with 400",
          "verify": { "kind": "test", "ref": "tests/contract/orders-api.toml" } },
        { "id": "AC-3", "source": "baseline", "text": "Never logs fields marked pii=sensitive (customer.email)",
          "verify": { "kind": "test", "ref": "tests/acceptance/orders-fn.feature:12" } }
      ],
      "verify": ["contract-orders-api", "unit-orders-fn"],
      "status": "todo"
    },
    {
      "id": "T-prod-apply", "title": "Apply the prod environment", "kind": "human",
      "nodes": ["prod"], "depends-on": ["T-orders-fn-implement"],
      "verify": ["infra-prod-validate", "infra-prod-scan", "infra-prod-cost"]
    }
  ]
}
```

- **Acceptance criteria come from three sources.** They are authored (`accept "…"` on a node), derived by a profile ("every operation with a body rejects invalid bodies with 400") or derived from the baseline. Each records its `source`, so reviewers can tell the author's intent apart from boilerplate.
- **Every criterion names how it's verified**: a generated test, a scanner check, a rule or `manual`. Criteria without automatic verification are allowed but are always reported as unverified.
- **`depends-on` follows `dependency` edges** between gap nodes, plus fixed orderings (scaffold before implement, implement before human apply).

### 4.7 Verification

`verify.json` lists the gates, each with a command (as argv, never a shell string), the working directory, the node ids it covers and the expected result. `rhizo verify` runs them and stores the results as an overlay (§8), and the MCP server reports them per node.

| Gate | Command | Covers |
|---|---|---|
| integrity | built in: hashes of generated files vs `manifest.json`; writes vs write scopes | all |
| model | `rhizo check --strict` | all |
| infra format | `tofu fmt -check -recursive infra/` | infra |
| infra validate | `tofu init -backend=false` then `tofu validate` | infra |
| infra lint | `tflint --recursive` with `tflint-ruleset-aws` | infra, `.tf` files only (TFLint follows Terraform's language and ignores `.tofu`) |
| infra scan | `checkov -d infra/prod` and `trivy config infra/prod` | infra. Trivy also reads `.tofu`; both can scan plan JSON |
| infra tests | `tofu test` (`mock_provider`, plan only, no credentials) | infra |
| infra cost | infracost, compared with `budget-monthly` | infra. **Optional**, see below |
| API lint | Spectral or Redocly CLI on `openapi.yaml` | data |
| contract | Schemathesis against a running service | data + app |
| acceptance | the stack's test runner on `tests/acceptance/` | app, ui |
| flows | Serverless Workflow JSON Schema (2020-12, from the spec repo); `kestra flow validate` or `POST /api/v1/{tenant}/flows/validate` | flow |

Tool versions for the gates are pinned in the profile catalog. `rhizo verify --print-tools` shows them. Gates whose tool isn't installed are reported as **skipped**, never as passed.

**Cost gate caveat.** Infracost has pivoted.
- The new v2 CLI (`infracost scan`) requires an account token, downloads plugins at runtime and doesn't document OpenTofu or plan-JSON input.
- The legacy v0.10 CLI still has `infracost breakdown --path`, which accepts directories and plan JSON, but its support lifetime is undocumented.

The cost gate is therefore **opt-in and network-dependent**, never part of the default gate set. Its output is an overlay (`monthly-cost`), not a determinism input.

**Supply-chain hygiene for the gates themselves.** Verification tools are an attack surface. In March 2026, Trivy releases, `trivy-action` and `setup-trivy` tags, and Checkmarx's KICS action were hijacked to steal CI secrets (GHSA-69fq-xp46-6x23 / CVE-2026-33634). The generated CI snippet therefore:
- pins every action by full commit SHA and every image by digest;
- verifies tool releases with cosign where the vendor signs them;
- runs the gates in a job with **no cloud credentials**, which works because the plan-only tests use mock providers.

---

## 5. Worked example: diagram → OpenTofu bundle

An HTTP API backed by a Lambda function, a private Postgres database and a receipts bucket.

### 5.1 The diagram

```kdl
rhizo version=2
title "Shop API — production"
use infra version="0.1"

frame "Production" id=prod type=infra.environment layout=layered direction=right {
    props provider=aws region=eu-central-1 state-bucket=shop-tofu-state budget-monthly=300

    node "Network" id=net type=aws.vpc {
        props cidr="10.20.0.0/16" azs=3 nat=single
    }
    node "HTTP API" id=api type=aws.apigw.http {
        props cors-origins="https://shop.example.com"
    }
    node "Orders function" id=orders-fn type=aws.lambda.function {
        props runtime="nodejs22.x" memory-mb=512 timeout-s=10 handler="index.handler"
        accept "p95 latency under 300 ms at 50 rps"
    }
    node "Orders DB" id=orders-db type=aws.rds.postgres {
        props engine-version="17" instance-class="db.t4g.small" storage-gb=20 multi-az=#true
    }
    node "Receipts" id=receipts type=aws.s3.bucket {
        props versioning=#true {
            lifecycle { - prefix="tmp/" expire-after-days=7 }
        }
    }

    edge api orders-fn kind=control { props route="POST /orders" }
    edge orders-fn orders-db kind=data { props access=read-write }
    edge orders-fn receipts kind=data { props access=write prefix="receipts/" }
    edge orders-fn net kind=dependency label="private subnets"
    edge orders-db net kind=dependency
}
```

Rendered, it is an ordinary layered flowchart (five boxes, five edges). Once the `infra` profile is loaded (phase 2), the boxes get AWS shapes and icons and the dependency edges are drawn dashed.

### 5.2 What `rhizo check` says before compiling

```
warning[infra.lambda.no-dlq]: async failures of `orders-fn` are dropped
  ┌─ shop.rhizo:15:5
  │ node "Orders function" id=orders-fn type=aws.lambda.function {
  = help: add `dead-letter=<sqs node id>` or `allow infra.lambda.no-dlq reason="sync only"`

info[infra.rds.iam-auth]: `orders-fn` → `orders-db` uses IAM database authentication;
  a task was added to create the `orders_fn` database role (T-orders-db-role)
```

### 5.3 Generated files (excerpts)

`infra/prod/versions.tf`:

```hcl
# @generated by rhizo 0.4.0 from shop.rhizo (frame prod). Do not edit; change the diagram.
terraform {
  # 1.10: S3 native locking. Mock providers (1.8) and state encryption (1.7) are older.
  required_version = ">= 1.10.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "= 6.69.1" # pinned by profile infra@0.1.0 (latest 6.x on 2026-10-15)
    }
  }

  backend "s3" {
    bucket       = "shop-tofu-state"
    key          = "shop/prod.tfstate"
    region       = "eu-central-1"
    use_lockfile = true # S3-native locking, no DynamoDB table
  }
}
```

`infra/prod/encryption.tofu`. This is OpenTofu-only syntax, so it lives in a `.tofu` file that TFLint and Checkov never read (§4.3):

```hcl
# @generated by rhizo 0.4.0 from shop.rhizo (frame prod). Do not edit; change the diagram.
terraform {
  encryption {
    key_provider "aws_kms" "state" {
      kms_key_id = var.state_kms_key_id
      region     = "eu-central-1"
      key_spec   = "AES_256"
    }
    method "aes_gcm" "state" {
      keys = key_provider.aws_kms.state
    }
    state { method = method.aes_gcm.state }
    plan  { method = method.aes_gcm.state }
  }
}
```

`infra/prod/providers.tf`. Every resource is tagged with the node it came from, which is what live import (§8) and cloud consoles map back on:

```hcl
# @generated by rhizo 0.4.0 from shop.rhizo (frame prod). Do not edit; change the diagram.
provider "aws" {
  region = "eu-central-1"

  default_tags {
    tags = {
      "rhizo:document"    = "shop.rhizo"
      "rhizo:environment" = "prod"
      "managed-by"        = "opentofu"
    }
  }
}
```

`infra/prod/main.tf`. Module versions are the latest on the OpenTofu registry as of 2026-10. The s3-bucket and rds input names below were checked against those modules' `variables.tf`. The vpc, lambda and apigateway-v2 inputs and outputs follow their documented interfaces from memory, and building the catalog must re-check them against the pinned versions. The rds module defaults `deletion_protection` to `false`, which is why the baseline sets it explicitly.

```hcl
# @generated by rhizo 0.4.0 from shop.rhizo (frame prod). Do not edit; change the diagram.

# rhizo:node=net
module "net" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "6.7.3"

  name             = "prod-net"
  cidr             = "10.20.0.0/16"
  azs              = ["eu-central-1a", "eu-central-1b", "eu-central-1c"]
  private_subnets  = ["10.20.0.0/20", "10.20.16.0/20", "10.20.32.0/20"]
  public_subnets   = ["10.20.48.0/24", "10.20.49.0/24", "10.20.50.0/24"]
  database_subnets = ["10.20.64.0/24", "10.20.65.0/24", "10.20.66.0/24"]

  enable_nat_gateway           = true
  single_nat_gateway           = true # props nat=single
  create_database_subnet_group = true

  tags = { "rhizo:node" = "net" }
}

# rhizo:node=orders-db
module "orders_db" {
  source  = "terraform-aws-modules/rds/aws"
  version = "7.2.2"

  identifier           = "prod-orders-db"
  engine               = "postgres"
  engine_version       = "17"
  family               = "postgres17"
  major_engine_version = "17"
  instance_class       = "db.t4g.small"
  allocated_storage    = 20
  multi_az             = true

  db_name  = "orders"
  username = "orders_admin"
  manage_master_user_password         = true # secret in Secrets Manager, never in state as plain text
  iam_database_authentication_enabled = true # from data edge orders-fn → orders-db

  db_subnet_group_name   = module.net.database_subnet_group_name
  vpc_security_group_ids = [aws_security_group.orders_db.id]
  publicly_accessible    = false # baseline, rule infra.rds.private
  storage_encrypted      = true  # baseline
  deletion_protection    = true  # baseline
  backup_retention_period = 7    # baseline default

  tags = { "rhizo:node" = "orders-db" }
}

# rhizo:node=receipts
module "receipts" {
  source  = "terraform-aws-modules/s3-bucket/aws"
  version = "5.17.0"

  bucket_prefix = "prod-receipts-"
  versioning    = { enabled = true }

  block_public_acls       = true # baseline, rule infra.s3.no-public
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true

  attach_deny_insecure_transport_policy = true # baseline: TLS only

  server_side_encryption_configuration = {
    rule = { apply_server_side_encryption_by_default = { sse_algorithm = "aws:kms" } }
  }

  lifecycle_rule = [{
    id         = "tmp"
    enabled    = true
    filter     = { prefix = "tmp/" }
    expiration = { days = 7 }
  }]

  tags = { "rhizo:node" = "receipts" }
}

# rhizo:node=orders-fn
module "orders_fn" {
  source  = "terraform-aws-modules/lambda/aws"
  version = "8.12.0"

  function_name = "prod-orders-fn"
  runtime       = "nodejs22.x"
  handler       = "index.handler"
  memory_size   = 512
  timeout       = 10

  create_package         = false
  local_existing_package = var.orders_fn_package # built by the app task, never by the compiler

  vpc_subnet_ids         = module.net.private_subnets # dependency edge orders-fn → net
  vpc_security_group_ids = [aws_security_group.orders_fn.id]
  attach_network_policy  = true

  tracing_mode          = "Active"
  attach_tracing_policy = true
  cloudwatch_logs_retention_in_days = 30

  attach_policy_json = true
  policy_json        = data.aws_iam_policy_document.orders_fn.json # iam.tf

  allowed_triggers = {
    api = { # control edge api → orders-fn
      service    = "apigateway"
      source_arn = "${module.api.api_execution_arn}/*/*"
    }
  }

  tags = { "rhizo:node" = "orders-fn" }
}

# rhizo:node=api
module "api" {
  source  = "terraform-aws-modules/apigateway-v2/aws"
  version = "6.1.1"

  name          = "prod-api"
  protocol_type = "HTTP"
  create_domain_name = false # no domain prop given

  cors_configuration = {
    allow_origins = ["https://shop.example.com"]
    allow_methods = ["POST"]
    allow_headers = ["content-type", "authorization"]
  }

  routes = {
    "POST /orders" = { # control edge api → orders-fn, props route
      integration = {
        uri                    = module.orders_fn.lambda_function_arn
        payload_format_version = "2.0"
      }
    }
  }

  tags = { "rhizo:node" = "api" }
}
```

`infra/prod/iam.tf`, derived entirely from the two `data` edges out of `orders-fn`:

```hcl
# @generated by rhizo 0.4.0 from shop.rhizo (frame prod). Do not edit; change the diagram.
data "aws_iam_policy_document" "orders_fn" {
  # rhizo:edge=orders-fn->receipts access=write prefix=receipts/
  statement {
    sid       = "ReceiptsWrite"
    actions   = ["s3:PutObject"]
    resources = ["${module.receipts.s3_bucket_arn}/receipts/*"]
  }

  # rhizo:edge=orders-fn->orders-db access=read-write (IAM DB auth, role orders_fn)
  statement {
    sid       = "OrdersDbConnect"
    actions   = ["rds-db:connect"]
    resources = ["arn:aws:rds-db:eu-central-1:${data.aws_caller_identity.current.account_id}:dbuser:${module.orders_db.db_instance_resource_id}/orders_fn"]
  }
}

data "aws_caller_identity" "current" {}
```

`infra/prod/network.tf`, derived from the same `orders-fn → orders-db` data edge:

```hcl
# @generated by rhizo 0.4.0 from shop.rhizo (frame prod). Do not edit; change the diagram.
resource "aws_security_group" "orders_fn" {
  name_prefix = "prod-orders-fn-"
  vpc_id      = module.net.vpc_id
  tags        = { "rhizo:node" = "orders-fn" }
}

resource "aws_security_group" "orders_db" {
  name_prefix = "prod-orders-db-"
  vpc_id      = module.net.vpc_id
  tags        = { "rhizo:node" = "orders-db" }
}

# rhizo:edge=orders-fn->orders-db
resource "aws_vpc_security_group_ingress_rule" "orders_db_from_orders_fn" {
  security_group_id            = aws_security_group.orders_db.id
  referenced_security_group_id = aws_security_group.orders_fn.id
  ip_protocol                  = "tcp"
  from_port                    = 5432
  to_port                      = 5432
}

# rhizo:edge=orders-fn->orders-db
resource "aws_vpc_security_group_egress_rule" "orders_fn_to_orders_db" {
  security_group_id            = aws_security_group.orders_fn.id
  referenced_security_group_id = aws_security_group.orders_db.id
  ip_protocol                  = "tcp"
  from_port                    = 5432
  to_port                      = 5432
}
```

`infra/prod/tests/prod.tftest.hcl` is a generated plan-only test that runs without cloud credentials:

```hcl
# @generated by rhizo 0.4.0 from shop.rhizo (frame prod). Do not edit; change the diagram.
mock_provider "aws" {}

variables {
  state_kms_key_id  = "arn:aws:kms:eu-central-1:000000000000:key/test"
  orders_fn_package = "tests/fixtures/empty.zip"
}

run "db_only_reachable_from_orders_fn" {
  command = plan
  assert {
    condition     = aws_vpc_security_group_ingress_rule.orders_db_from_orders_fn.from_port == 5432
    error_message = "orders-db must accept 5432 from orders-fn only (edge orders-fn->orders-db)"
  }
}
```

### 5.4 What the compiler could not do, as tasks

```
T-orders-fn-scaffold   generated   app/orders-fn/src/handler.ts (types from openapi.yaml)
T-orders-fn-implement  agent       write-scope app/orders-fn/**   AC-1 (manual), AC-2, AC-3
T-orders-db-role       agent       write-scope app/orders-fn/migrations/**
                                   "create role orders_fn with login; grant rds_iam; grant … on orders.*"
T-prod-vars            human       set state_kms_key_id, orders_fn_package in CI variables
T-prod-apply           human       depends on all above; gates infra-prod-*
```

### 5.5 Verification run

```
$ rhizo verify build/
integrity          ok        23 generated files unchanged, no writes outside scopes
model              ok
infra-prod-fmt     ok        tofu fmt -check
infra-prod-validate ok       tofu validate
infra-prod-lint    ok        tflint (ruleset aws)
infra-prod-scan    ok        checkov: 0 failed · trivy config: 0 HIGH/CRITICAL
infra-prod-test    ok        tofu test: 1 passed
infra-prod-cost    skipped   opt-in gate (needs network + infracost token), see §4.7
contract-orders-api skipped  service not running (start with `npm run dev` in app/orders-fn)
```

The results land as an overlay. The diagram shows each node with a green or grey badge, and the MCP server reports `orders-fn` as "contract tests pending".

---

## 6. Worked example: a flow to Kestra and Serverless Workflow

Order intake: a webhook receives an order, large orders are sent for approval, and then the order is stored and logged.

### 6.1 The diagram

```kdl
rhizo version=2
use flow version="0.1"
use data version="0.1"

frame "Order intake" id=order-intake type=flow.workflow layout=layered direction=down {
    props namespace="shop.orders" targets="kestra serverless-workflow" sw-catalog="https://git.example.com/shop/sw-functions"

    node "Order received" id=received type=flow.trigger.webhook {
        port out "body" kind=data schema=data:order       // refine the type's `body` port
    }
    node "Large order?" id=large type=flow.branch {
        props when="received.body.total > 1000"           // portable predicate
    }
    node "Ask for approval" id=approve type=flow.http-request {
        props method=POST url="https://approvals.example.com/requests" {
            body { order-id "{{ received.body.id }}"; total "{{ received.body.total }}" }
        }
        accept "Approval requests are retried 3 times with backoff"
    }
    node "Save order" id=save type=flow.db.query {
        props connection=orders-db statement="insert into orders (id, total) values (:id, :total)" {
            params { id "{{ received.body.id }}"; total "{{ received.body.total }}" }
        }
    }
    node "Log" id=done type=flow.log { props message="Stored order {{ received.body.id }}" }

    edge received large kind=control
    edge large approve kind=control from-port=true
    edge large save    kind=control from-port=false
    edge approve save  kind=control
    edge save done     kind=control
}
```

Notes:

- `flow.db.query` takes **named parameters**, never string interpolation into SQL. Rule `flow.sql.no-interpolation` rejects `{{ }}` inside `statement`, and the compiler maps `params` to the target's parameter binding. Where a target cannot bind parameters, the node is a capability gap for that target, never a silent string concatenation. Kestra's PostgreSQL `Query` binds `:name` placeholders from a `parameters` map as prepared-statement parameters.
- `connection=orders-db` is a cross-profile reference to the `aws.rds.postgres` node from §5. The link pass checks that it exists, and the compiler emits secret *references* (`secret('…')`), never values.

### 6.2 Kestra output

`flows/kestra/shop.orders/order-intake.yaml`:

```yaml
# @generated by rhizo 0.4.0 from orders.rhizo (frame order-intake). Do not edit; change the diagram.
id: order-intake
namespace: shop.orders
labels:
  rhizo-document: orders.rhizo
  rhizo-frame: order-intake

tasks:
  - id: large                       # rhizo:node=large
    type: io.kestra.plugin.core.flow.If
    condition: "{{ trigger.body.total > 1000 }}"
    then:
      - id: approve                 # rhizo:node=approve
        type: io.kestra.plugin.core.http.Request
        uri: https://approvals.example.com/requests
        method: POST
        contentType: application/json
        body: "{{ {'order-id': trigger.body.id, 'total': trigger.body.total} | json }}"   # filter name: verify
        retry:
          type: exponential
          maxAttempts: 3
          interval: PT2S
          maxInterval: PT30S

  - id: save                        # rhizo:node=save
    type: io.kestra.plugin.jdbc.postgresql.Query
    url: "{{ secret('ORDERS_DB_JDBC_URL') }}"
    username: "{{ secret('ORDERS_DB_USER') }}"
    password: "{{ secret('ORDERS_DB_PASSWORD') }}"
    sql: insert into orders (id, total) values (:id, :total)
    parameters:                     # bound, never interpolated
      id: "{{ trigger.body.id }}"
      total: "{{ trigger.body.total }}"

  - id: done                        # rhizo:node=done
    type: io.kestra.plugin.core.log.Log
    message: "Stored order {{ trigger.body.id }}"

triggers:
  - id: received                    # rhizo:node=received
    type: io.kestra.plugin.core.trigger.Webhook
    key: "{{ secret('ORDER_INTAKE_WEBHOOK_KEY') }}"   # value: letters and digits only
```

This targets Kestra 2.0. 2.0 broke some 1.x flows (for example `ForEach` became `Loop`, trigger `conditions` became `when:`, and `json()` became `fromJson()`), so the target version is part of the catalog. Two details in this excerpt are unverified and must be checked against the 2.0 plugin schemas when the target is built: the Pebble `json` filter used to build `body`, and the `retry` property names. Kestra publishes per-type JSON Schemas (`https://api.kestra.io/v1/plugins/schemas/{type}`), so the emitter's golden tests can validate every task they generate against them.

How the compiler got there:

- The webhook node became a **trigger**, not a task, because Kestra models triggers separately. `received.body.x` therefore compiles to `trigger.body.x`.
- The `large` branch's `true` arm (`approve`) re-joins at `save`, which is also the `false` target. That makes it a structured region: `If` with only `then`, followed by `save`. An arm that never re-joined would be a diagnostic (§3.7).
- The authored `accept` "retried 3 times" turned into a `retry` block, because the profile knows that criterion pattern. Otherwise it would have remained a manual criterion.

### 6.3 Serverless Workflow 1.0 output

`flows/serverless-workflow/order-intake.sw.yaml`:

```yaml
# @generated by rhizo 0.4.0 from orders.rhizo (frame order-intake). Do not edit; change the diagram.
document:
  dsl: '1.0.1'
  namespace: shop-orders
  name: order-intake
  version: '0.1.0'
  metadata:
    rhizo-document: orders.rhizo
    rhizo-frame: order-intake
input:
  schema:
    format: json
    document:                       # inlined from specs/schemas/order.schema.json (node `order`)
      type: object
      required: [id, total]
      properties:
        id: { type: string }
        total: { type: number, minimum: 0 }
use:
  catalogs:
    shop:                           # from frame prop sw-catalog
      endpoint:
        uri: https://git.example.com/shop/sw-functions
do:
  - large:
      metadata: { rhizo-node: large }
      switch:
        - large-order:
            when: ${ .total > 1000 }
            then: approve
        - default:
            then: save
  - approve:
      metadata: { rhizo-node: approve }
      call: http
      with:
        method: post
        endpoint: https://approvals.example.com/requests
        body:
          order-id: ${ .id }
          total: ${ .total }
      output:
        as: ${ $input }             # keep the order as the data for the next task
      then: save
  - save:
      metadata: { rhizo-node: save }
      call: sql-query:1.0.0@shop    # capability gap: SQL is not in the core DSL
      with:
        connection: orders-db
        statement: insert into orders (id, total) values (:id, :total)
        params:
          id: ${ .id }
          total: ${ .total }
  - done:
      metadata: { rhizo-node: done }
      set:
        message: ${ "Stored order " + (.id | tostring) }
```

What differs from the Kestra output, and why:

- **No trigger.** The webhook trigger has no Serverless Workflow equivalent, because engines start workflows through their own APIs. It becomes the workflow's **input schema**, inlined so the file stands alone, and a diagnostic notes that HTTP exposure is the engine's job.
- **The SQL step is a catalog call.** `flow.db.query` has no core DSL task. When the workflow frame sets `sw-catalog=…`, the SW target emits a call to the catalog function `sql-query:1.0.0` (the DSL's `{name}:{version}@{catalog}` form), and the task graph gets `T-save-sw-function`: "publish `sql-query` in your function catalog". Without `sw-catalog`, compiling for this target fails on node `save`. This is the capability matrix at work.
- **Names.** Task names are node ids, and each task also carries `metadata.rhizo-node`, since every SW task accepts a `metadata` map. The namespace `shop.orders` became `shop-orders`, because DSL names are restricted to lowercase letters, digits and dashes. That restriction comes from the 1.0 schema's name pattern as I remember it, so it should be checked when the emitter is built.
- **Expressions.** They are wrapped in `${ }` because strict mode, the default, requires it. The predicate `received.body.total > 1000` compiles to the jq form `.total > 1000`, since the input *is* the webhook body.
- **Version and runtimes.** The output targets DSL 1.0.1. Engines that list 1.0 support include Quarkus Flow, Lemline, Synapse and the Java SDK's reference implementation. SonataFlow's docs still describe spec 0.8. No engine's conformance has been checked for this doc.
- **No runtime imports it.** None of Kestra, n8n or Node-RED imports Serverless Workflow natively. The SW export is for SW engines and as a portable, reviewable artifact, not a bridge between the other three.

### 6.4 n8n and Node-RED

These follow the same plan with wired targets, so there is no structured-region recovery. Their exports are complete workflow JSON with `connections` (n8n) or `wires` (Node-RED) generated from control edges. Node identity is deterministic: n8n gets a UUIDv5 of `(document, node id)` and Node-RED gets the node id hash. n8n keeps importer-chosen node ids and generates one only when `id` is missing. Node-RED treats `id` as an opaque string, though only the 16-hex form its editor generates is documented, which is why rhizo hashes to that form. The rhizo node id is also written into each runtime node's free-text field (n8n `notes`, Node-RED `info`, which is Markdown), so the runtime's own UI shows where a node came from. n8n nodes carry a `typeVersion`, which the catalog pins per node type, because parameter shapes change between type versions.

n8n's license matters to users: it is distributed under the *Sustainable Use License* (fair-code), not an OSI-approved license, and `.ee.` files are under a separate enterprise license. rhizo only *emits* n8n's documented import format, so the license affects people who run n8n, not rhizo.

---

## 7. MCP server and traceability

### 7.1 `rhizo mcp`

`rhizo mcp shop.rhizo` starts a Model Context Protocol server (stdio by default, Streamable HTTP with `--http`) over a document, its bundle and its overlays. It is built on the official Rust SDK, [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) (3.5 as of 2026-10), in a new `rhizo-mcp` crate.

**Spec version.** The current MCP revision is **2026-07-28**, and it is a breaking one:
- The protocol is stateless. There is no `initialize` handshake or session id; servers implement `server/discover`.
- `resources/subscribe` became `subscriptions/listen`.
- Tasks moved into an official extension.
- Server-initiated requests use the multi-round-trip `input_required` pattern.

`rmcp` 3.5 implements 2026-07-28 and stays compatible with 2025-11-25, so `rhizo-mcp` gets both from the SDK. The design below relies only on tools, resources, prompts and subscriptions, the parts that survived the revision. MCP is now governed by the Linux Foundation's Agentic AI Foundation (AAIF), which also hosts AGENTS.md.

**Tools**

| Tool | Purpose |
|---|---|
| `list_nodes(type?, profile?, tag?, status?)` | Find nodes: "all `aws.*` nodes", "all failing nodes" |
| `get_node(id)` | Label, type, normalized props (with defaults), ports, note, link, acceptance criteria, generated files and ranges, current overlay status |
| `neighbors(id, kind?, direction?, depth?)` | Edges and adjacent nodes, filtered by edge kind: "what does `orders-fn` write to?" |
| `get_tasks(status?, ready?)` / `get_task(id)` | The task graph. `ready=true` returns tasks whose dependencies are done. |
| `acceptance_criteria(node_id \| task_id)` | Criteria with source and verification method |
| `report_progress(id, status, evidence)` | `status`: `in-progress`/`blocked`/`done`; `evidence`: commit, files touched, gate results. `id` is a node or task id. |
| `trace(path, line?)` | File position → node ids (and the reverse with `node:`) |
| `verify(task_id)` | Runs that task's gates through `rhizo verify` and returns the results. Off unless the server is started with `--allow-verify`, because it executes tools. |
| `propose_change(patch, rationale)` | Proposes a **model** change, e.g. "add a DLQ to `orders-fn`". It is stored as a pending proposal for a human to accept in the editor or with `rhizo proposals`, never applied silently. |

**Resources.** `rhizo://node/{id}`, `rhizo://task/{id}`, `rhizo://bundle/manifest`, `rhizo://bundle/file/{path}` and `rhizo://adr/{n}`. Agents can listen for changes (`subscriptions/listen` in 2026-07-28, `resources/subscribe` for older clients), so an agent learns when a recompile changes a node it is working on.

**Prompts.** `implement-task(task_id)` returns a ready-made instruction: the task, its criteria, constraints, write scope and the gates that define "done".

**Rules the server enforces:**

1. `report_progress(status=done)` is accepted only when the task's gates have passed: they ran during this call or appear in the latest verify overlay. Otherwise the status is stored as `claimed`, and the diagram shows that differently from `verified`.
2. The server never writes the `.rhizo` source. Proposals are stored in `.rhizo/proposals/`, and only a human action applies them.
3. The server is read-only toward the bundle. Agents write files with their own tools, inside write scopes that `rhizo verify` checks.

Agents without MCP get the same information from `AGENTS.md` (the plain-Markdown convention, now an AAIF project), `tasks/TASKS.md` and `rhizo trace` on the command line.

**Optional: the diagram inside the agent's host.** MCP Apps (`io.modelcontextprotocol/ui`) is a final MCP extension that lets a server offer `ui://` HTML resources, which hosts render in a sandboxed iframe. `rhizo mcp` can serve the live diagram (SVG plus overlays) that way, so a human watching an agent sees node status change in place. The feature is cheap because the renderer already emits standalone SVG.

### 7.2 Traceability

Ids flow in both directions:

| Artifact | How it points at a node |
|---|---|
| Any generated file | Header comment + `manifest.json` entry with node ids |
| Ranges in generated files | `trace.json`: `{ "path": "infra/prod/main.tf", "ranges": [{ "lines": [3, 21], "nodes": ["net"] }] }` plus `# rhizo:node=` comments for humans |
| OpenTofu addresses | Derived from ids (`module.orders_db`); `manifest.json` maps address → node |
| Cloud resources | `rhizo:node` tag on every resource that supports tags; `default_tags` for document and environment |
| OpenAPI / AsyncAPI | `x-rhizo-node` specification extension on operations, schemas, channels |
| JSON Schema | `x-rhizo-node` annotation keyword (ignored by validators) |
| Kestra | task id = node id; flow labels for document and frame |
| n8n / Node-RED | deterministic node ids + id in the free-text field |
| Scaffolds | Header comment with node id and task id |
| Agent-written code | Optional `rhizo:node=<id>` comments; **commit trailers** `Rhizo-Node: orders-fn` and `Rhizo-Task: T-orders-fn-implement` (standard `git interpret-trailers` format) |
| Tests | Test names/tags carry acceptance-criterion ids (`AC-2@orders-fn`) |

`rhizo trace app/orders-fn/src/handler.ts:40` answers "why does this code exist?" using `trace.json`, scaffold headers and `git log` trailers. `rhizo trace --node orders-fn` lists every file, resource, task, test and commit for a node, which is also what the editor shows in a node's side panel.

---

## 8. Live diagrams: overlays

An **overlay** is per-node data layered onto a document *without changing it*. Progress, verification results, infrastructure state, plan diffs, workflow executions and cost all use the same structure:

```json
{
  "rhizo-overlay": 1,
  "source": "opentofu-state",
  "captured-from": "infra/prod",
  "entries": {
    "orders-db": {
      "status": "ok",
      "attrs": { "address": "module.orders_db", "engine_version": "17.4", "multi_az": true },
      "link": "https://console.aws.amazon.com/rds/home?region=eu-central-1#database:id=prod-orders-db"
    },
    "receipts": { "status": "drifted", "attrs": { "address": "module.receipts" },
                  "message": "versioning disabled outside OpenTofu" }
  }
}
```

- **Status** is one small fixed set: `unknown`, `planned`, `pending`, `running`, `ok`, `warning`, `failed`, `drifted`, `claimed`, `verified`. Themes define how each is drawn (badge and stroke), so overlays work in every renderer, including static SVG/PNG/PDF: `rhizo render shop.rhizo --overlay state.json`.
- **Overlays are inputs, not part of the source.** They are never written into `.rhizo` files, so the source stays reviewable and deterministic. Overlays are stored under `.rhizo/overlays/` (git-ignored by default).

### 8.1 OpenTofu

| Command | Overlay |
|---|---|
| `tofu show -json` (state) | one entry per node: `ok`, resource attributes (allow-listed), console link |
| `tofu show -json plan.out` | `resource_changes[].change.actions` → `planned` with create/update/delete/replace, so a PR review sees the plan *on the diagram* |
| `tofu plan -refresh-only -json` | `drifted` where the real resource differs |
| infracost JSON output (opt-in, see §4.7) | `attrs.monthly-cost` per node |

`rhizo import-overlay opentofu --state infra/prod` runs `tofu show -json` and maps each `address` to a node through `manifest.json`. For resources inside modules, it uses the module address prefix. Resources that map to no node are listed as **unmanaged** on the environment frame.

**Secrets.** State JSON contains sensitive values. The importer keeps only the attributes the profile allow-lists per type (ids, ARNs, engine versions, counts), drops anything listed in `sensitive_values`, and never puts raw values into SVG text or tooltips. With state encryption enabled, `tofu show` decrypts using the configured key provider, and rhizo never handles the key.

### 8.2 Workflows

| Runtime | Source | Mapping |
|---|---|---|
| Kestra | `GET /api/v1/{tenant}/executions/{id}` (`state`, `taskRunList[]`), or the SSE stream `…/executions/{id}/follow` | `taskRunList[].taskId` = node id; states map to the status set. In 2.0, task outputs are fetched separately (`/api/v1/{tenant}/outputs/tasks/…`) and only when allow-listed. |
| n8n | `GET /api/v1/executions/{id}?includeData=true` (`X-N8N-API-KEY`); polling only, with no public streaming endpoint | Execution `status` is documented. Per-node run data sits under `data`, keyed by node name and mapped to node ids through the export's table, but its shape is **not part of the documented schema**. The importer must be tolerant and pinned to n8n versions. |
| Node-RED | The editor's `/comms` WebSocket (`status/<nodeId>` messages with `{text, fill, shape}`). This is an **internal, undocumented interface**. The supported alternative is a generated `status`/`catch`/`complete` node trio that forwards events to rhizo over HTTP. | node id |
| Serverless Workflow | Engine-specific APIs. The spec also describes lifecycle CloudEvents for workflow and task status, but whether engines emit them consistently is unverified. | task name / `metadata.rhizo-node` = node id |

`rhizo watch orders.rhizo --overlay kestra://localhost:8080/shop.orders/order-intake` polls (or follows, where the API streams) and re-renders on change. The editor uses the same overlay stream. Polling intervals and credentials are client-side configuration and never part of the document.

### 8.3 Agent progress and verification

`report_progress` and `rhizo verify` write overlays with sources `agent-progress` and `verify`. Every source therefore lands on the same canvas: a node can show "plan: update · verify: 6/7 · agent: claimed".

---

## 9. Phased plan

Each phase ends with something usable on its own. Phases 2–9 can run alongside the native editor. **Phase 1 should land before it.**

### Phase 1: typed core model (before the native editor)

*Goal:* the final node and edge shape, stored and round-tripped, with no semantics yet.

- `rhizo-core`: `TypeName`, `PortName`, `EdgeId`, `Props`, `Port`, `PortDir`, `EdgeKind`; new fields on `Node`, `Frame`, `Edge`, `Document` (§2.1); structural checks in `validate()`.
- `rhizo-lang`:
  - AST fields; KDL parser support for `rhizo version=2`, `use`, `type=`, `props` blocks (with the KDL ↔ JSON mapping and per-value spans), `port`, `accept`, `kind=`, `from-port=`/`to-port=`, `id=` and edge blocks.
  - Writer support for all of them, with typed nodes keeping explicit ids. `FORMAT_VERSION` becomes 2 (version 1 files are still read).
  - The top-level `"rhizo": 2` marker in JSON/YAML, and unknown keys on JSON/YAML nodes and edges become errors.
- `rhizo schema` regenerated.
- Interop: GraphML and draw.io carry type, props and kind. Other formats warn once.
- Tests:
  - Golden files proving plain documents are byte-identical (§2.10).
  - Round-trip of typed documents through KDL → JSON → YAML → KDL.
  - Span tests for prop diagnostics.
  - Old-build behavior documented.
- **Not in phase 1:** profiles, validation of props, rendering changes, the compiler.

*Why before the editor:* the editor's document operations (insert node, set prop, connect port), its undo log and its text↔canvas sync all key off node fields. Adding `props` and ports later would mean migrating the editor's operation set and its sync logic. Doing it now costs about one focused PR.

### Phase 2: profile system and the `data` vocabulary

- New crate `rhizo-profile`: manifest format, JSON Schema validation (layers 2–3), Rust rule API, `rhizo.lock`, `rhizo check` with profiles, `rhizo types`.
- The built-in `data` profile, vocabulary only, which is the smallest of the four.
- Rendering: type defaults in the style cascade, icons, edge-kind looks, port anchors.

### Phase 3: compiler skeleton and the `data` compiler

- New crate `rhizo-compile`: pipeline, bundle writer, `manifest.json`, `trace.json`, ownership, determinism tests.
- `data` → JSON Schema, OpenAPI 3.1, AsyncAPI 3. API lint and contract test configs.
- `rhizo compile`, `rhizo plan`, `rhizo trace`.

### Phase 4: `infra` (AWS subset) and `rhizo verify`

- The types from §3.7, the module catalog, IAM and security group derivation from edges, `moved` blocks on id renames, generated `tofu test`.
- `rhizo verify` with the gates from §4.7. CI runs the §5 example through every gate.

### Phase 5: constraints, task graph, MCP

- `rhizo.decision`/`rhizo.baseline`, MADR output, baseline mapping tables (with review), `tasks.json`/`TASKS.md`/`AGENTS.md`.
- `rhizo-mcp` with the tools from §7.1 and the proposal workflow.

### Phase 6: `flow`

- Kestra and Serverless Workflow first (structured), then n8n and Node-RED (wired).
- Portable expressions, capability matrix, structured-region recovery.

### Phase 7: overlays and live diagrams

- The overlay model and rendering. Importers for `tofu show -json` (state and plan), infracost and the Kestra executions API, then n8n and Node-RED.
- `rhizo watch --overlay`; the editor consumes the same stream.

### Phase 8: `ui`

- Screen spec schema, JSON Forms UI schema output, DTCG tokens, Given/When/Then criteria with the WCAG 2.2 AA baseline.

### Phase 9: WASM plugin profiles

- `wit/profile.wit`, a wasmtime-based host (native builds), OCI distribution and signature checks.
- Re-implement one built-in (probably `data`) as a component to prove the interface. Publish a plugin SDK crate.

---

## 10. Open questions

1. **Q1 — `type` on frames vs a node.** Typed frames are the natural boundary for workflows and environments, but frames can't nest today. Is "environment contains workflows" ever needed in one document, or does that go across documents (Q6)?
2. **Q2 — Props shorthand beyond JiK.** JiK 4.0.0 covers the mapping exactly. Should rhizo also accept the schema-directed single-argument → array convenience (§2.8), or stay strictly JiK so any JiK tool reads `props` blocks unchanged?
3. **Q3 — Compound layout.** Architecture diagrams want nested boxes (VPC ⊃ subnet ⊃ instance). This is a layout feature (compound Sugiyama, or grouped grid) independent of profiles, but `infra` gets much better with it. Should `aws.subnet` wait for it?
4. **Q4 — Abstract types.** Should there be provider-neutral types (`infra.database` realized as RDS, Cloud SQL or Azure Database by environment), or always concrete ones? Abstract types are attractive for multi-cloud, but they hide important differences. Proposal: concrete first, abstract later as a profile feature, not a core one.
5. **Q5 — Typed edge shorthand** in JSON/YAML (`"api.out -> fn.invoke : control"`). Is it worth a second grammar?
6. **Q6 — Multi-document systems.** `import "data.rhizo" as data` for cross-document references, with node ids qualified as `data:order`. How does the lockfile scope work across documents?
7. **Q7 — Rule language for plugins and house rules:** Rego (regorus) or CEL? Rego fits the existing IaC policy ecosystem. CEL is smaller and easier to sandbox and teach.
8. **Q8 — Baseline mapping ownership.** Who maintains and reviews the ASVS/CIS/SSDF mapping tables? They need security review on every profile release. An open, separately versioned mapping file would invite outside contributions.
9. **Q9 — Which clouds and modules.** Phase 4 is AWS only. For Azure, Azure Verified Modules (Microsoft's official module library, MIT) are the obvious source. AVM publishes no OpenTofu support statement, though the modules resolve from the OpenTofu registry, so the catalog would need its own compatibility testing. For GCP, `terraform-google-modules`. Each catalog needs a policy on which modules qualify (maintenance activity, license, signed releases).
10. **Q10 — Scaffold updates.** When the contract changes (a new required field), scaffolds are already agent-owned. Is a `*.rhizo-new` file plus a failing contract test enough, or should rhizo generate a patch?
11. **Q11 — Secrets in flows.** The compiler emits secret *names*. Should the infra profile also create the secret containers (Secrets Manager entries) that flows reference, linking the two profiles further?
12. **Q12 — Where overlays live.** Is a git-ignored `.rhizo/overlays/` enough, or should verification overlays be committed for audit trails, and in what format?
13. **Q13 — Licensing of icons.** Cloud vendors' architecture icon sets are proprietary. AWS permits their use in architecture diagrams, but its terms for modifying the icons or using them in a product UI ship with the download and haven't been checked. Azure's icons may be used only in diagrams, training and documentation, may not be altered, and may not represent your own product. Built-in profiles should ship neutral icons (Lucide ISC, Tabler MIT, Kubernetes community icons Apache-2.0/CC-BY-4.0) and let users point at a vendor set they downloaded themselves.
14. **Q14 — Name.** "Profile" overlaps with UML profiles, which is apt (UML profiles extend a base metamodel with stereotypes) but might mislead people who expect UML. Alternatives: "kit", "vocabulary", "pack".
15. **Q15 — Arazzo export.** A workflow made only of API calls against `data.operation`s is also an Arazzo 1.1 document. Is that export worth adding for API-journey docs and agent tests, or is it scope creep?
16. **Q16 — Kestra version span.** Kestra 2.0 broke 1.x flows, and 1.3.x is still patched. Should the `kestra` target emit for one major (the catalog's) or offer `kestra-version=1|2`?
17. **Q17 — Node-RED live status.** The `/comms` WebSocket is internal. Is a generated status-forwarding subflow acceptable as an addition to user flows, or should live status for Node-RED wait for a public API?

---

## Appendix A: specs and tools referenced, as of 2026-10-18

Each entry was checked against release pages, registries or source code on 2026-10-18. **Unverified** marks claims that could not be confirmed and must be re-checked before implementation. Versions will drift: the profile catalogs pin exact versions, and this table only records the state the design was written against.

| Spec / tool | State on 2026-10-18 | Relevance and caveats |
|---|---|---|
| KDL 2.0 | Final (2024-12-21); Rust `kdl` 6.7 | rhizo's format |
| JiK (JSON-in-KDL) | 4.0.0, uses KDL v2 syntax; listed on kdl.dev with no formal status label | `props` mapping (§2.8) |
| JSON Schema | 2020-12 is still the latest; the stable "v1" is announced but not published | props schemas, data profile |
| `jsonschema` crate | 0.59, MIT, pre-1.0, a few test-suite failures | schema validation |
| OpenAPI | 3.2.0 (2025-09-19), 3.2.1 (2026-09-10), 3.1.2 | Default 3.1; 3.2 opt-in |
| OpenAPI Overlay | 1.2.0 (2026-09-22) | not used yet |
| AsyncAPI | 3.1.0 (2026-01-31) | data profile events |
| Arazzo | 1.1.0 (2026-05) | possible export (Q15) |
| OpenTofu | 1.13.1 (2026-10-01), MPL-2.0, CNCF Sandbox | `.tofu` files (1.8+), `use_lockfile` S3 locking (1.10+), state encryption (1.7+), `mock_provider` in `tofu test` (1.8+), `moved` blocks, `check` blocks, `-exclude`, `tofu show -json`: all confirmed |
| OpenTofu Registry | Indexes terraform-aws-modules, terraform-google-modules and AVM | **no verified tier**; vetting is rhizo's catalog policy |
| terraform-aws-modules | vpc 6.7.3, rds 7.2.2, s3-bucket 5.17.0, lambda 8.12.0, apigateway-v2 6.1.1, sqs 5.2.2, security-group 6.0.0 | Inputs used for s3-bucket/rds checked; others from memory (§5.3) |
| AWS provider | 6.69.1 (2026-10-15); `default_tags` still supported | |
| Azure Verified Modules | Official Microsoft library, MIT | no OpenTofu support statement |
| TFLint / ruleset-aws | 0.64.0 / 0.49.0, MPL-2.0 | **No OpenTofu support planned**, ignores `.tofu` (§4.3) |
| Checkov | 3.3.29, Apache-2.0 | Scans `.tf` and plan JSON, not `.tofu`. OpenTofu plan JSON expected to work (same format) but **unverified**. |
| Trivy | 0.76.0, Apache-2.0; tfsec merged into it | Reads `.tofu`. **March 2026 supply-chain compromise** (CVE-2026-33634): pin by SHA/digest, verify with cosign. |
| KICS | 2.2.0, Apache-2.0, lists OpenTofu | Its GitHub Action was compromised in the same March 2026 campaign |
| Conftest / OPA | 0.71.1 / 1.21.2, Apache-2.0; OPA CNCF graduated | plan-JSON policy, Rego |
| regorus | 0.13.0, pre-1.0, "mostly compliant" with OPA | in-process Rego (§3.4) |
| `cel` crate | 0.15.0, MIT | CEL alternative; conformance level **unverified** |
| Infracost | v2 CLI (`infracost scan`, account token); legacy v0.10.46 has `breakdown` | **Pivoted.** OpenTofu/plan JSON in v2 undocumented; v0.10 lifetime undocumented. Cost gate is opt-in. |
| terraform-docs | 0.24.0, MIT, recognizes `.tofu` | optional README generation |
| CIS Benchmarks | AWS Foundations 7.1.0, Azure 6.0.0, GCP 5.0.0 | Release dates **unverified**; licensing limits redistributing mappings; no authoritative IaC CIS report from Trivy/Checkov OSS |
| OWASP ASVS | 5.0.0 (2025-05-30), CC BY-SA 4.0 | app baseline |
| OWASP Top 10 | 2025 edition published (exact final date **unverified**) | tagging only |
| NIST SSDF | SP 800-218 v1.1 final; Rev. 1 (SSDF 1.2) initial public draft (2025-12) | process criteria |
| SLSA | 1.2 (2025-11) | bundle provenance |
| OpenSSF Scorecard | 5.6.0 | plugin and catalog admission (Q9) |
| MADR | 4.0.0 (2024-09) | ADR output |
| WCAG | 2.2 Recommendation; 3.0 Working Draft | ui baseline |
| Design Tokens (DTCG) | 2025.10, first stable; Community Group report | ui tokens |
| JSON Forms | 3.8.0, MIT | form UI schemas |
| A2UI | v0.9.x, 1.0 RC, Apache-2.0 | considered, not adopted (§3.7) |
| Kestra | 2.0.6 (2.0 LTS, 2026-09); 1.3.x patched; Apache-2.0 core | Task types, `If`, `Request`, JDBC `parameters`, templated webhook `key`, `flow validate`, executions API and SSE `follow` confirmed. 2.0 has breaking changes. |
| n8n | 2.43.7; Sustainable Use License (fair-code, not OSI) | Node JSON shape, type strings, `notes`, importer-chosen ids, executions API confirmed; per-node run-data shape **undocumented** |
| Node-RED | 5.0.8 (5.0 requires Node 22.9+); 4.1.x maintained; Apache-2.0 | Flow JSON, `info`, admin API (`Node-RED-Deployment-Type`) confirmed; `/comms` status is **internal**; OpenJS membership not re-checked |
| Serverless Workflow (now "Open Workflow Specification") | DSL 1.0.1 (2026-07-31), CNCF Sandbox, Apache-2.0 | DSL syntax used in §6.3 confirmed except the name pattern (**unverified**); lifecycle events **unverified**; no native import in Kestra/n8n/Node-RED |
| MCP | Spec 2026-07-28 (stateless, breaking); governed by AAIF (Linux Foundation) | §7.1 |
| `rmcp` | 3.5.1, Apache-2.0; implements 2026-07-28 and 2025-11-25 | MCP server |
| MCP Apps | Final extension `io.modelcontextprotocol/ui` | optional live view in agent hosts |
| AGENTS.md | AAIF project, plain Markdown | bundle `AGENTS.md` |
| WASI / Component Model | WASI 0.3 ratified 2026-06, 0.3.1 2026-08; 0.2 supported; CM 1.0 undated | plugin host (§3.5) |
| wasmtime / wit-bindgen / jco | 49.0.3 / 0.63.0 / 1.39.0 | jco's WASI 0.3 shim is Node-only; irrelevant for an import-free world |
| Extism | 1.41.0, BSD-3, core modules + own ABI | considered, rejected (no Component Model) |
| Wasm OCI artifact layout / wkg / cosign | TAG Runtime layout (`vnd.wasm.config.v0`); wkg 0.16.1 (no signing); cosign 3.1.3 | plugin distribution |
| Schemathesis | 4.32.0, MIT | contract tests |
| Spectral / Redocly CLI | 6.17.0 (OpenAPI 3.2 support **unclear**) / 2.64.1 | API lint |
| Git trailers | `git interpret-trailers` | `Rhizo-Node:` / `Rhizo-Task:` |

## Appendix B: prior art

- **Structurizr / C4 model** and **LikeC4**: architecture as code, rendering many views from one model. Both are maintained: Structurizr is at v2026.09, and its vNext `server` command needs a license. LikeC4 is at 1.60, MIT. Neither has a compiler to infrastructure.
- **AWS Infrastructure Composer** (formerly Application Composer): visual design → CloudFormation/SAM. It is AWS-only and proprietary. AWS is **retiring the standalone console**: it takes no new customers after 2026-10-29 and ends on 2026-12-07, while the VS Code integration stays.
- **Mendix / low-code platforms**: typed visual models compiled to running apps on a proprietary runtime. rhizo borrows the UX and leaves the runtime out.
- **Node-RED, n8n and Kestra editors**: visual workflow editors, each tied to its own runtime. rhizo targets all of them from one model.
- **Pluralith, Inframap, Rover**: diagrams *from* Terraform/OpenTofu, the opposite direction. rhizo's live overlays cover the useful part (state on the diagram) without generating layouts from state.
