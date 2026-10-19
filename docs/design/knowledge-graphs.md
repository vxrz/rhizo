# Knowledge graphs, views and reasoning profiles

| | |
|---|---|
| Status | Draft, for discussion |
| Date | 2026-10-18 |
| Extends | [profiles-and-compiler.md](profiles-and-compiler.md) (typed nodes, props, ports, edge kinds, profiles, compiler, MCP, overlays) |
| Scope | `rhizo-core` model, `rhizo-lang` syntax, `rhizo-layout`, new crates `rhizo-kg` (graph store, views, RDF/property-graph interop) and the profiles `kg`, `argue`, `law`, `dx` |
| Touches today's code | [`crates/core/src/model.rs`](../../crates/core/src/model.rs), [`crates/lang/src/kdl.rs`](../../crates/lang/src/kdl.rs), [`crates/layout/src/lib.rs`](../../crates/layout/src/lib.rs), [`crates/interop/src/canvas.rs`](../../crates/interop/src/canvas.rs) (GraphML) |

## Summary

The profiles document turns a diagram into a specification. This addendum turns a diagram into a **window onto a knowledge graph**.

A knowledge graph is too large to draw whole, so the drawing and the graph come apart:

- The **graph** is the set of nodes, typed edges and statements about them. It lives in the document, in a text sidecar next to it, or in an external store.
- A **view** is a saved, text-first **query plus layout** over that graph. A view is what gets rendered, edited and diffed.
- A plain diagram is the degenerate case: a document that is its own graph and its own single view. It keeps working byte for byte.

On top of that sit three additions:

1. **Core model additions** a knowledge graph needs beyond the profiles document's Phase 1: IRIs and CURIEs as ids, open edge *types* (predicates), edges as first-class objects that other statements can be about, mandatory **provenance**, n-ary relations, and language-tagged and datatyped literals.
2. A **`kg` profile** for interop with the standards that already exist: RDF 1.2 (Turtle, TriG, N-Quads, JSON-LD), SHACL, SKOS and OWL import on the semantic-web side, and openCypher, ISO GQL and GraphML on the property-graph side.
3. **Reasoning-graph profiles** that agents query over MCP. These are a domain-neutral argument profile (`argue`) with a legal layer (`law`), and a medical differential-diagnosis profile (`dx`). Their evaluators are deterministic and their results are overlays, never edits to the source. **Every inferred edge carries provenance and a resolvable citation, or it is rejected.**

These profiles provide **decision support, not decisions**. §6 sets the boundaries and says where the regulatory picture is unclear.

The first step is again small. **Phase 1b** (§8) adds CURIE ids, edge types, edge identity and stored provenance to the core model, still without semantics. It belongs next to Phase 1, before the native editor, because it changes what an edge *is* in the editor's data model.

---

## 1. Principles and non-goals

**Principles** (these add to the profiles document's §1)

1. **Diagrams stay first-class.** A document with no `graph`, `view`, `prefix` or edge `type` behaves exactly as today. Knowledge-graph features are opt-in, behind `rhizo version=3` (§3.10).
2. **Graph and view are separate objects.** The graph says what is true according to whom. The view says what to look at and where to put it. Neither is derived from the other.
3. **Text first, diffable always.** Every committed artifact is canonical text with a stable order: documents, views and sidecar graphs. Binary indexes are caches derived from that text and are never committed.
4. **Standards over invention.** Interop targets are published specs: RDF 1.2, JSON-LD 1.1, SHACL, SKOS, OWL 2, ISO GQL, openCypher, GraphML, PROV-O, AIF, ECLI, ELI, Akoma Ntoso. Where a spec is still a draft (RDF 1.2, SHACL 1.2), support sits behind a feature flag and the doc says so.
5. **Provenance is part of the data, not a comment.** Who asserted an edge, from which source, by which method and with what confidence is structured, validated and exported.
6. **Computed is not authored.** Evaluator results (posteriors, acceptability labels, rankings) are overlays (profiles doc §8). They are recomputed from the source and never written back into it.
7. **Agents propose, humans accept.** The MCP server never writes the source (profiles doc §7.1, rule 2). For reasoning profiles, an agent's addition stays `proposed` until a named human accepts it.
8. **No licensed content ships with rhizo.** Vocabularies with license terms (SNOMED CT, LOINC, ICD-11, HPO, UMLS sources) are fetched by the user under their own license into a local cache. rhizo ships identifier patterns and resolvers only.

**Non-goals**

- **A triple store at web scale.** The sidecar targets project-sized graphs, roughly up to a million statements (§2.3). Bigger graphs stay in an external store, and rhizo views them through snapshots.
- **OWL reasoning.** rhizo imports OWL classes and properties as types (§4.6) and computes subclass closure for type checks. Description-logic reasoning is left to dedicated reasoners.
- **An LLM inside rhizo.** Extraction, summarization and answer writing happen in agents outside rhizo. They arrive as proposals with provenance. This is the profiles doc's "no LLM in the compiler", applied to graphs.
- **A clinical or legal product.** `dx` ships for education, research and knowledge curation (§6.2). `law` is a tool for people who are allowed to give legal advice, not a replacement for them (§6.3).

---

## 2. Graph and view

### 2.1 Three objects

| Object | What it is | Where it is written | Committed? |
|---|---|---|---|
| **Graph** | Nodes, typed edges, props, provenance. No positions, no styles. | In the document, in a sidecar directory, or in an external store | Yes (document, sidecar). External stores are pinned through snapshots. |
| **View** | A query over one or more graphs, plus layout, pins, collapsed clusters and styling | `view` blocks in a `.rhizo` document | Yes |
| **Document** | Today's `.rhizo` file. It may declare graphs and views, and may contain authored nodes, frames and styles as today. | `.rhizo` / `.kdl` / JSON / YAML | Yes |

The rules that keep plain diagramming intact:

- A document with no `graph` statement is **its own graph**, and with no `view` it is **its own single view**: everything drawn, laid out by today's engines. This is every existing file.
- Authored nodes and edges in a document always belong to the graph named `self`. A view can query `self` together with sidecar and external graphs, so a hand-drawn diagram can sit next to, and link into, a large graph.
- A **frame can embed a view**: `frame "PE work-up" view=pe-workup`. One document can then mix a hand-drawn flowchart, a mindmap and a queried neighborhood of a 200,000-node graph on one canvas.

### 2.2 Where the graph lives

| Tier | Declared as | Good for | Limits | Determinism |
|---|---|---|---|---|
| **In-document** | nodes and edges in the `.rhizo` file (graph `self`) | diagrams, argument maps, a single case, small vocabularies: up to a few thousand nodes | Rendering the whole document is still the default. Large documents get slow to edit as text. | Same as today |
| **Sidecar** | `graph main path="kg/"` | project knowledge graphs, case collections, vocabulary subsets: target up to about 10⁶ statements (unmeasured) | Parse time. A derived index cache (§2.3) keeps queries fast. | Same as today: text in git |
| **External** | `graph umls endpoint="sparql+https://…"`, `endpoint="bolt://…"`, or a local Oxigraph store | large reference graphs (UMLS, SNOMED CT, institutional graphs) | Not diffable. Network. Licensing. Read-only from rhizo. | Each view over an external graph stores a **snapshot** of its result (§2.6) |

The recommendation is **sidecar by default** for anything that outgrows a document. It keeps git as the audit log (§6.1) and makes graph changes reviewable as pull requests. External stores are for data that is too big, too licensed or owned by somebody else.

### 2.3 The sidecar

```
kg/
  graph.kdl                 # manifest: prefixes, profiles, vocab packs, shard count
  HP/03.kg.kdl              # one shard: entities whose CURIE prefix is HP, hash bucket 03
  MONDO/11.kg.kdl
  _local/00.kg.kdl          # ids without a prefix
.rhizo/cache/main.idx       # derived index (git-ignored), keyed by the shards' sha256
```

- **Shard files use the same KDL syntax as documents**, restricted to `node` and `edge` statements with `type`, `props`, `prov` and `role` (§3). There are no frames, styles or positions, because those belong to views.
- **Canonical order makes diffs small.** Each shard holds entities sorted by id. Each node is followed by its outgoing edges sorted by `(type, to, id)`, the way Turtle groups triples by subject. `rhizo fmt` enforces the order. Adding an edge therefore touches one shard and shows up as a few added lines.
- **Sharding is stable.** The shard is the CURIE prefix plus `xxh3(id) mod N`, where N is declared in `graph.kdl`. Changing N is an explicit `rhizo kg reshard` commit, so files never move because a shard grew.
- **Alternative formats.** `format=turtle` and `format=nquads` sidecars are supported for teams that already keep RDF in git. rhizo then writes canonical Turtle 1.2 with sorted subjects. Edge ids and provenance are kept through reifiers (§4.2). KDL stays the default because it carries spans for diagnostics, `prov` blocks and JiK props without a mapping layer.
- **The derived index** (`rhizo-kg` crate) holds interned ids (`u32`), CSR adjacency in both directions per edge type, a label and synonym index for lexical search, and per-type id lists. It is rebuilt when the shards' combined hash changes, and it is never committed. An optional in-process SPARQL engine (Oxigraph, §4.3) can be loaded from the same shards when a view or an agent asks for SPARQL.

### 2.4 Views

```kdl
rhizo version=3
use kg version="0.1"
use dx version="0.1"

graph main path="kg/"

view "Case 17: dyspnea work-up" id=case17-workup layout=layered direction=right {
    query {
        seed case-17
        expand depth=2 via="dx.observed dx.considers dx.lr.finding dx.lr.disorder dx.tests" direction=both
        where type="dx.* kg.source"
        always where="props.red-flag == true"   // red flags are never cut by `limit`
        limit 120
    }
    cluster by=type
    pin case-17 x=0 y=0
    collapse src-*                               // sources as a single "12 sources" node
    style type=dx.hypothesis shape=rounded
    members count=31 digest="sha256:5f1c…"       // written by `rhizo fmt --refresh-views`
}
```

A view block has:

| Part | Meaning |
|---|---|
| `query { … }` | Which nodes and edges are in the view (§2.5). |
| `layout=`, `direction=`, layout options | Any rhizo layout. `stress` and `force` are new (§7). |
| `pin <id> x= y=` | Fixed positions. Dragging a node in the editor writes a pin. Nothing else stores coordinates, so most view diffs are query diffs. |
| `cluster by=type\|community\|prop:<key>\|hierarchy:<edge-type>` | Grouping, drawn as hulls or collapsed meta-nodes (§7.4) |
| `collapse <id-or-glob>` | Collapsed groups |
| `style …` | View-local style rules by type, tag or class, slotted into the existing cascade after type defaults |
| `members count= digest=` | The view's last result. It is written by rhizo and used for drift detection (§2.6). |

### 2.5 The view query language

Views need a query that is small, deterministic, easy to diff and portable to the stores rhizo talks to. SPARQL and GQL are too big to be the only option, and neither runs over a property graph *and* RDF. So views get a tiny built-in language, `rhizo-query`, with SPARQL and GQL as escape hatches.

| Statement | Meaning |
|---|---|
| `seed <id>…` / `seed where type=… prop.k=v` | Start set |
| `expand depth=N via="<edge types>" direction=out\|in\|both max-degree=K` | Breadth-first expansion. A node with more than K neighbors is expanded only into a collapsed "+N more" stub. |
| `path from=<id> to=<id> via=… max-length=N k=K` | The k shortest paths (explanation paths, §5.2) |
| `where type="<glob>…" tag=… prop.<key><op><value>` | Filter, applied after expansion |
| `include <id>…` / `exclude <id>…` | Manual additions and removals. The editor's "show" and "hide" actions write these. |
| `always where=…` | Members that are never dropped by `limit` (red flags, the case node) |
| `limit N order="distance degree id"` | Deterministic truncation |
| `query lang=sparql #"…"#` / `query lang=gql #"…"#` | Escape hatch. The query must return node and edge ids. It is only allowed against graphs whose engine supports that language. |

- **Semantics are deterministic.** Expansion visits edge types in the order listed, then neighbors in id order. Truncation sorts by the declared order with id as the final tie-breaker.
- **Compilation.** `rhizo-query` compiles to:
  - SPARQL 1.1 for SPARQL endpoints and the in-process Oxigraph engine. Bounded depth becomes a `UNION` of fixed-length paths, because SPARQL 1.1 property paths have no `{m,n}` bound.
  - openCypher variable-length patterns for Neo4j and Memgraph.
  - Direct index walks for sidecars.
- **Why not SPARQL only.** Most views are "this node, two hops, these edge types". That should be one line an author can read in a diff, and it must also work on property-graph stores and plain documents.

### 2.6 Determinism, drift and snapshots

- **Rendering a view is deterministic** given the graph contents, the view text and the rhizo version. Layouts are seeded from the view id, and pins are honored (§7.2).
- **Drift.** The graph can change while the view text stays the same. `members count= digest=` records the last result, a sha256 over the sorted member node and edge ids. `rhizo check` reports `info[kg.view.drift]` when the current result differs, and `rhizo fmt --refresh-views` rewrites the line. A pull request that changes the graph therefore also shows *which views changed*, even though nobody edited them.
- **Snapshots for external graphs.** A view over an external store records `snapshot "kg/snapshots/case17-workup.nq"`, an N-Quads dump of exactly the statements in its result. Rendering and CI read the snapshot and never the network. `rhizo kg refresh case17-workup` re-runs the query and rewrites the snapshot, and that change is reviewed like any other. A snapshot of licensed content is redistribution, so the vocab license gates in §4.8 apply to it.

### 2.7 Graph diffs

`rhizo diff a b` on graphs and views prints a **semantic diff**: added and removed nodes and edges, and changed props and provenance, keyed by id rather than by line. `rhizo render --view v --diff main..HEAD` draws the diff as an overlay: added in green, removed as ghosts, changed with a badge. This is how a reviewer sees what an agent's accepted proposals changed in a medical or legal graph.

---

## 3. Core model additions

### 3.1 What the profiles document's Phase 1 already provides

Phase 1 (profiles doc §2) adds `type`, `props` (JSON, validated by profiles), `ports` and `accept` to nodes, and adds an optional `id`, `kind` (`data` / `control` / `dependency`), ports and `props` to edges. A knowledge graph needs more:

| Need | Why Phase 1 is not enough | Section |
|---|---|---|
| Global ids | Node ids are document-local slugs. Graphs merge data from many sources and must round-trip to RDF IRIs. | §3.2 |
| Open edge predicates | `kind` is a closed set of three, with compiler semantics. A KG has thousands of predicates. | §3.3 |
| Edges as first-class objects | Edge ids are optional, and nothing can point *at* an edge. Provenance, evidence and attacks on inferences need that. | §3.4 |
| Provenance | Free text in `note` cannot be validated or exported | §3.5 |
| N-ary relations | "LR of finding F for disease D in population P is 2.1, per source S" relates four things | §3.6 |
| Literals | JSON has no language tags or datatypes. RDF needs both. | §3.7 |
| Knowledge vs case data | Patient findings must never mix with general medical knowledge | §3.8 |

### 3.2 IRIs, CURIEs and prefixes

```kdl
rhizo version=3
base "https://kg.example.org/cardio/"                      // local ids expand against this
prefix HP    "http://purl.obolibrary.org/obo/HP_"
prefix MONDO "http://purl.obolibrary.org/obo/MONDO_"
prefix skos  "http://www.w3.org/2004/02/skos/core#"

node "Tachycardia" id=HP:0001649 type=kg.concept
node "Pulmonary embolism" id=MONDO:0005279 type=kg.concept
node "Surgery within the last 4 weeks" id=rf-recent-surgery type=kg.concept   // local id → base IRI
```

- **Ids may be CURIEs** (`PREFIX:local`) whose prefix is declared in the document or in the sidecar manifest. KDL v2 identifier strings may contain `:` (checked against the KDL 2.0 spec, 2026-10-18), so `id=HP:0001649` needs no quotes.
- **Local ids** (today's slugs) expand against `base`. Without `base`, they expand against the document's URL when exported, with a warning that the IRIs are not stable.
- **Full IRIs** are written as strings: `id="https://example.org/x"`. `rhizo fmt` compacts them to a CURIE when a declared prefix matches.
- **Prefixes follow the OBO and Bioregistry conventions** for biomedical ontologies, where `HP:0001945` ↔ `http://purl.obolibrary.org/obo/HP_0001945` (obofoundry.org ID policy, bioregistry.io). The `kg` profile ships a default prefix map taken from Bioregistry, pinned per profile version.
- **TypeName gets a second form.** Profiles doc §2.2 defines dotted, lowercase type names (`aws.s3.bucket`). A type can now also be a CURIE (`type=MONDO:0005279`, `type=schema:Person`) when it comes from an imported ontology (§4.6). Dotted names stay for profile-defined types, and both forms live in one namespace table.
- **Stable ids** (profiles doc §2.7) are stricter here. Every node and edge in a sidecar has an explicit id, and `rhizo fmt` never derives ids from labels in `kg` shards.

### 3.3 Edge types

Edges get a **`type`**, the same field nodes have. It names the predicate (the relationship type, in property-graph terms):

```kdl
edge rf-recent-surgery rf-immobilization type=skos:related
edge r-fear c-damage type=arg.inference.conclusion          // a role edge, see §3.6
```

- `type` is open and namespaced, and profiles declare it like node types: label, `kind` default, domain and range types, inverse, characteristics (`symmetric`, `transitive`, `functional`), and a props schema.
- **`kind` stays.** It is the compiler-semantics family from the profiles doc. An edge type declares its default kind (`skos:broader` → no kind; `infra`-style data edges → `data`), so authors rarely write both. An explicit `kind` that contradicts the type's declaration is an error.
- Untyped edges remain plain diagram edges. An edge with only a `label` is still a drawing.

Profile declaration (extends profiles doc §3.3):

```kdl
edge-type dx.lr.finding {
    label "finding of"
    domain dx.lr
    range kg.concept
    functional #true          // one finding per LR node
}
edge-type skos:broader {
    label "broader"
    domain kg.concept
    range kg.concept
    inverse skos:narrower
    hierarchy #true           // views may lower it to `parent` for tree and mindmap layouts (§4.5)
}
```

### 3.4 Edge identity and statements about edges

**Every edge in a `kg` graph has an id.** When the author gives none, `rhizo fmt` writes a deterministic one: `e-` followed by the first 12 hex digits of the sha256 of `(from, type, to, n)`, where `n` disambiguates parallel edges in document order. Node ids and edge ids share **one namespace**, so any statement can refer to an edge.

There are four established ways to say something about a relationship:

| Approach | Status (2026-10-18) | Verdict |
|---|---|---|
| Classic RDF reification (`rdf:Statement`, `rdf:subject` …) | RDF 1.0/1.1 vocabulary | Verbose, four triples per edge, no link to the asserted triple. Export fallback only. |
| N-ary / qualified-relation pattern (the relationship becomes a node) | W3C Working Group Note (2006) | Right for genuinely n-ary facts (§3.6). Too heavy for every edge. |
| RDF-star quoted triples (`<< s p o >>` as a term) | CG report (2021). Its `<< >>` syntax was redefined by RDF 1.2. | Do not target. |
| **RDF 1.2 triple terms plus reifiers** (`rdf:reifies`, `<< s p o ~ r >>`, `{| … |}` annotations) | RDF 1.2 Concepts is a Candidate Recommendation (2026-04-07). Turtle, TriG and N-Triples 1.2 are Working Drafts (2026-10-07). | **Export target**, behind a flag until Recommendation |

**The rhizo model is a property graph with edge identity**, which is the most natural form for authors. An edge is an object with an id, a type, endpoints, props and provenance. Mapping it to RDF 1.2 is direct: the edge id becomes the **reifier**. Its props and provenance become triples about the reifier, and the asserted triple is emitted as well (§4.2). Property-graph targets keep edge props natively.

**Edges may target edges.** An edge whose `to` (or `from`) is an edge id is a statement about that edge, for example "this study supports *that* LR", or "this argument undercuts *that* inference". Core resolves endpoints against both namespaces:

```rust
pub enum Endpoint { Node(NodeId), Edge(EdgeId) }
```

The KDL syntax is unchanged (`edge src-ceriani-2010 e-lr-wells-pe type=kg.supports`). Layouts draw an edge-to-edge connection to the target edge's label anchor. When a structure needs more than a single edge-to-edge link, profiles should model the relationship as an n-ary node instead (§3.6). The argument profile does this, following AIF (§5.5). See open question K3.

### 3.5 Provenance

Provenance is a structured block on any node or edge:

```kdl
node id=lr-wells-pe type=dx.lr {                 // roles and props elided, see §5.4.1
    prov method=extracted by="agent:claude@session-7f3a" at="2026-10-18" confidence=0.8 {
        source src-ceriani-2010 locator="Table 3" quote="…"
        derived-from e-12 e-13
        review status=accepted by="human:dr-lehmann" at="2026-10-19" note="Checked against full text"
    }
}
```

| Field | Values | Meaning |
|---|---|---|
| `method` | `asserted` · `imported` · `extracted` · `inferred` · `computed` | `asserted`: a person wrote it. `imported`: from a vocab pack or external graph. `extracted`: an agent read a source. `inferred`: derived from other statements. `computed`: produced by an evaluator. |
| `by` | `human:<id>` · `agent:<model>@<run>` · `rule:<rule-id>` · `import:<graph>` | The responsible party. Agent ids identify a run, never a person. |
| `at` | ISO 8601 date or date-time | When the statement was made. It is data written by the author or the MCP server, never read from the compiler's clock (profiles doc §4.2). |
| `source` (child, repeatable) | id of a `kg.source` node, `locator=`, `quote=` | The citation. `locator` is a pinpoint: page, table, paragraph, article. |
| `derived-from` | node and edge ids | For `inferred` and `computed`: the statements this was derived from |
| `confidence` | number in [0, 1], or `scheme:value` (for example `grade:moderate`) | rhizo attaches **no calculus** to confidence. Profiles define what it means (K5). |
| `review` | `status=proposed\|accepted\|rejected`, `by=`, `at=`, `note=` | Human review state (§6.1) |

Rules in the `kg` profile, which reasoning profiles make stricter:

- `kg.prov.required` (error): `method` ∈ {`extracted`, `inferred`} requires at least one `source` **and**, for `inferred`, `derived-from`. In `dx`, `argue` and `law` this rule **cannot be suppressed with `allow`**.
- `kg.source.resolvable` (error): every `kg.source` has at least one identifier (`doi`, `pmid`, `isbn`, `ecli`, `eli`, `celex`, `akn`, or `url` together with `content-sha256`). `rhizo kg verify-sources` resolves the identifiers (opt-in, network) and records an overlay, which catches fabricated citations (§6.1).
- `kg.review.agent` (warning; error in reasoning profiles): statements `by="agent:…"` without `review status=accepted` are drawn as proposed (dashed, muted) and excluded from evaluators unless the evaluator is run with `--include-proposed`.

Export maps `prov` to **W3C PROV-O** (`prov:wasAttributedTo`, `prov:wasDerivedFrom`, `prov:generatedAtTime`, `prov:hadPrimarySource`) on the edge's reifier.

### 3.6 N-ary relations

A profile can declare a node type as a **relation node** with named roles:

```kdl
// in the dx profile
type dx.lr nary=#true {
    label "Likelihood ratio"
    role finding  type=kg.concept required=#true
    role disorder type=kg.concept required=#true
    role population type=dx.population
    schema "schemas/dx.lr.json"        // lr-pos, lr-neg, ci, ...
}
```

```kdl
// in a document or sidecar
node id=lr-hr-pe type=dx.lr {
    role finding HP:0001649
    role disorder MONDO:0005279
    role population pop-ed-adults
    props lr-pos=1.8 lr-neg=0.7          // illustrative numbers
    prov method=asserted by="human:reviewer-a" { source src-demo locator="Table 2" }
}
```

- `role <name> <id>` is sugar for an edge of type `<node-type>.<role>` from the relation node to the target. The edges are ordinary edges, so queries, exports and validation need nothing special.
- Relation nodes are drawn small (a dot or diamond glyph with role labels on the edges), which is the usual hyperedge drawing.
- Export follows the W3C n-ary pattern in RDF and becomes a node with role relationships in property graphs. Both are lossless.

### 3.7 Literals: multi-valued, language-tagged, datatyped

Props stay JSON (profiles doc §2.3). Two additions make them RDF-capable:

```kdl
node "Chest pain" id=HP:0100749 type=kg.concept {
    props {
        pref-label (@en)"Chest pain" (@de)"Brustschmerz"     // two language-tagged values
        alt-label (@en)"Thoracic pain"
        first-seen (xsd:date)"2026-10-12"
        weight-kg (xsd:decimal)"81.5"
    }
}
```

- **KDL value annotations** carry the tag. An annotation starting with `@` is a BCP 47 language tag, and an annotation containing `:` is a datatype CURIE. Value annotations are legal KDL v2 (§3.7 of the spec), and `@` is allowed in identifiers (checked 2026-10-18). JiK 4.0.0 defines only the node annotations `(array)` and `(object)` and says nothing about value annotations, so this extension does not clash with it. A JiK-only tool would read the plain strings, which loses the tag but keeps the data.
- **The JSON and YAML form uses JSON-LD value objects**, so there is nothing new to learn: `{"@value": "Brustschmerz", "@language": "de"}`, `{"@value": "2026-10-12", "@type": "xsd:date"}`.
- **Multi-valued props are arrays.** A schema may mark one as a set (`"x-rhizo": {"set": true}`). Author order is then kept in the text, while comparison, diffing and export treat it as unordered.
- **Display labels.** The node's positional label remains the drawn label. A document or view may set `lang="de"`, and then the label is taken from `pref-label` with that tag, falling back to the positional label.

### 3.8 Knowledge and case data

Reasoning graphs mix two kinds of statements that must never be confused:

- **Knowledge**: general and sourced. "Tachycardia has LR+ 1.8 for PE in adult ED patients (source S)." "Art. 82(1) GDPR gives a right to compensation for material or non-material damage."
- **Case data**: about one patient, one matter. "Case 17 has a heart rate of 118." "The claimant received a breach notification on 3 March."

A graph declaration carries a role, `graph knowledge path="kg/knowledge/" role=knowledge` or `role=case`, and a document's own graph `self` defaults to `case` when a reasoning profile is in use. Profiles check the boundary:

- `dx`: findings live only in case graphs. LRs, prevalences and test accuracy live only in knowledge graphs. Rule `dx.layer` (error).
- Exports put each graph in its own **named graph** (TriG and N-Quads), so the separation survives interop.
- Case graphs are where personal data lives, which matters for §6.1.

### 3.9 Model sketch

```rust
// crates/core/src/model.rs (additions on top of profiles doc §2.1)

pub struct Document {
    // ...
    pub base: Option<Iri>,
    pub prefixes: IndexMap<String, Iri>,        // author order
    pub graphs: Vec<GraphSource>,               // `graph` statements (§2.2)
    pub views: Vec<View>,                       // `view` blocks (§2.4)
    pub vocabs: Vec<VocabRef>,                  // `vocab` statements (§4.8)
}

pub struct Edge {
    // ... Phase 1 fields: id, kind, from_port, to_port, props ...
    pub ty: Option<TypeName>,                   // the predicate
    pub prov: Option<Prov>,
    // `from`/`to` stay strings in the AST; lowering resolves them to Endpoint.
}

pub struct Node {
    // ... Phase 1 fields ...
    pub prov: Option<Prov>,
    pub roles: Vec<(RoleName, NodeId)>,          // sugar for role edges; kept for round-tripping
}

pub struct Prov {
    pub method: ProvMethod,                     // asserted | imported | extracted | inferred | computed
    pub by: Option<String>,
    pub at: Option<String>,                     // ISO 8601, validated, never from a clock
    pub sources: Vec<Citation>,                 // { source: NodeId, locator, quote }
    pub derived_from: Vec<String>,              // node or edge ids
    pub confidence: Option<Confidence>,         // Number(f64) | Scheme(String)
    pub review: Option<Review>,                 // { status, by, at, note }
}
```

Core's structural checks add the following. Prefixes are declared before use. CURIEs expand to valid IRIs. Node and edge ids are unique across both namespaces. Endpoints resolve. `prov.at` parses as ISO 8601. Literal annotations are a valid language tag or a declared datatype prefix. Everything else (which provenance is required, domains, ranges) belongs to profiles, as in the profiles doc.

### 3.10 Syntax summary and versioning

| Where | Syntax | Meaning |
|---|---|---|
| top level | `rhizo version=3` | Required when any feature below is used |
| top level | `base "<iri>"`, `prefix P "<iri>"` | IRI expansion (§3.2) |
| top level | `graph <name> path=… \| endpoint=… role=knowledge\|case format=kdl\|turtle\|nquads` | Graph sources (§2.2) |
| top level | `vocab <prefix> version=… source=… sha256=… license=…` | Vocabulary packs (§4.8) |
| top level | `view "<title>" id=… layout=… { query {…} pin … cluster … collapse … members … }` | Views (§2.4) |
| frame | `view=<view-id>` | Embed a view in a frame |
| node / edge | `id=PREFIX:local` | CURIE ids |
| edge | `type=<predicate>` | Edge type (§3.3) |
| edge | `from`/`to` may name an edge id | Statements about edges (§3.4) |
| node / edge block | `prov … { source …; derived-from …; review … }` | Provenance (§3.5) |
| node block | `role <name> <id>` | N-ary roles (§3.6) |
| props values | `(@de)"…"`, `(xsd:date)"…"` | Language tags, datatypes (§3.7) |

Why `version=3` and not 2: the features arrive after Phase 1's `version=2`. A version-2 build reading these files should say "this file needs a newer rhizo", not "unknown statement `prefix`". The JSON and YAML forms carry `"rhizo": 3`, and the profiles doc §2.10 guarantees carry over unchanged: plain files are byte-identical, and old builds fail loudly. If Phase 1 and Phase 1b ship in the same release, both can collapse into version 2 (K1).

---

## 4. The `kg` profile

### 4.1 Vocabulary

| Type | Notes |
|---|---|
| `kg.concept` | A concept from a vocabulary or ontology. Props `pref-label`, `alt-label`, `definition`, `notation`, `deprecated`. Maps to `skos:Concept`, or to the imported OWL class when the id is a class IRI. |
| `kg.entity` | A thing in the world (a person, an organization, a document instance) |
| `kg.scheme` | A concept scheme. It can be a frame, so a whole SKOS scheme draws as a mindmap (§4.5). |
| `kg.source` | A citable source. Props `doi`, `pmid`, `isbn`, `ecli`, `eli`, `celex`, `akn`, `url`, `content-sha256`, `title`, `issued`, `kind` (`study`, `guideline`, `judgment`, `legislation`, `textbook`, `illustrative`). `kind=illustrative` taints every result derived from it (§5.4). |
| `kg.summary` | An agent-written summary of a set of nodes. Always `prov method=extracted` with `derived-from` (§5.2). |
| edge types | `kg.supports`, `kg.contradicts`, `kg.cites`, `kg.same-as` (→ `owl:sameAs`), plus the SKOS relations (§4.5) |

### 4.2 Mapping rhizo ↔ RDF ↔ property graph

| rhizo | RDF 1.2 | Property graph (openCypher / GQL) |
|---|---|---|
| node id (CURIE / IRI) | IRI | node with an `iri` property |
| node `type` | `rdf:type` | label (`:Concept`, or the CURIE escaped as a label) |
| node label | `rdfs:label`, or `skos:prefLabel` in SKOS schemes | `name` property |
| scalar prop | literal triple. The property IRI comes from the schema's `x-rhizo.iri`, or `base` + key. | property |
| nested prop (object) | `rdf:JSON` literal (the datatype JSON-LD 1.1 defines) or a blank node, per schema | JSON string property (Neo4j properties cannot hold maps) |
| language-tagged values | `"Brustschmerz"@de` | `pref_label` list plus `pref_label_lang` list, or a `pref_label_de` property (configurable, lossy) |
| edge with `type` | triple `s p o` | relationship of that type |
| edge id + props + prov | `<< s p o ~ :e1 >>` with props and PROV-O on `:e1`. The asserted triple is emitted too. | relationship properties: `id`, props, `prov_*` |
| edge targeting an edge | the reifier `:e1` as subject or object | not native: the target edge is materialized as a `:Statement` node (lossy, warned) |
| n-ary node | node with role predicates (W3C n-ary pattern) | node with role relationships |
| graph role / name | named graph (TriG, N-Quads) | a `graph` property, or one database per graph |
| frames, styles, pins, views | not exported, or exported to a separate `rhizo:` named graph with `--with-views` | dropped |

**RDF 1.1 fallback.** RDF 1.2 is not a Recommendation yet. `--rdf=1.1` exports edge props through standard reification (`rdf:Statement`), and `--rdf=1.2-basic` drops triple terms. Each mode warns once if it loses information. The default stays `1.1` until RDF 1.2 Concepts and Turtle 1.2 reach Recommendation (K4).

### 4.3 RDF formats and libraries

| Format | Direction | Library (2026-10-18) |
|---|---|---|
| Turtle, N-Triples, N-Quads, TriG (1.1, and 1.2 behind `rdf-12`) | read + write | `oxttl` 0.2.4 and `oxrdfio` 0.2.6 (MIT OR Apache-2.0). Both have an `rdf-12` feature. |
| RDF/XML | read (for OWL imports) | `oxrdfio` |
| JSON-LD 1.1 | read + write | `oxrdfio` (JSON-LD enabled by default since Oxigraph 0.5.5), or the `json-ld` crate 0.21.4 (MIT/Apache-2.0, 1.1 only) |
| SPARQL 1.1 (1.2 behind `sparql-12`) | query, in-process | `oxigraph` 0.5.11 / `spargebra` 0.4.7 (MIT OR Apache-2.0) |

- The Oxigraph crates are pure Rust. Whether the in-memory store builds for `wasm32-unknown-unknown` without its RocksDB backend still has to be checked (K15). It is behind a `sparql` feature of `rhizo-kg`, so the WASM promise of the core crates holds either way.
- `rio_turtle` is unmaintained and points to `oxttl`. It is not used.
- JSON-LD 1.2 is a deliverable of the rechartered JSON-LD WG (2026-01-06 to 2028-01-31), but no draft is published. A June 2026 list post suggests it will not include triple terms. rhizo's JSON-LD export therefore uses classic reification for edge props until that changes.

### 4.4 SHACL

SHACL validates the **RDF projection** of a graph. It complements JSON Schema, which validates one node's props:

| Layer (profiles doc §3.4) | Tool | Checks |
|---|---|---|
| 2. Schema | JSON Schema 2020-12 | the props of one node or edge |
| 4. Rules | built-in Rust rules, Rego, and **SHACL shapes** | graph shape: cardinality of relationships, value types of targets, required provenance, closed shapes |

- **Engine:** the `shacl` crate from the rudof project (0.3.27, 2026-10-17, MIT OR Apache-2.0). It supersedes the older `shacl_validation` and `srdf` crates. Its conformance to the SHACL 1.0 test suite has not been checked for this doc.
- **Diagnostics keep spans.** The exporter keeps a side table from RDF terms to source spans. `sh:focusNode` maps to the node or edge span, `sh:resultPath` maps to the prop's JSON Pointer, and `sh:resultSeverity` maps to the diagnostic severity. A SHACL violation then underlines the right `props` value, exactly like a JSON Schema error.
- **Where shapes come from:**
  - the profile (the `kg` profile ships shapes for `kg.source` and `prov`);
  - the user (`shapes "shapes/*.ttl"` in `graph.kdl`);
  - **generated for export**: `rhizo kg shapes --profile dx` emits SHACL derived from the profile's type and edge-type declarations, so people who receive exported RDF can validate it without rhizo.
- **Target version.** SHACL 1.0 (Recommendation, 2017-07-20). SHACL 1.2 is in progress (Core WD 2026-10-14; Rules FPWD 2026-09-17; UI FPWD 2026-05-26; the compact syntax is only an editor's draft). It adds `sh:reifierShape`, which is what validating provenance on reified edges in pure SHACL would need. Until 1.2 is stable, rhizo's own `kg.prov.*` rules check provenance.

### 4.5 SKOS

SKOS (Recommendation, 2009-08-18, still current) is the natural format for controlled vocabularies, and it is where knowledge graphs meet mindmaps:

- `kg.scheme` and `kg.concept` map to `skos:ConceptScheme` and `skos:Concept`. The edge types `skos:broader`, `skos:narrower`, `skos:related`, `skos:exactMatch`, `skos:closeMatch`, `skos:broadMatch` and `skos:narrowMatch` are declared by the `kg` profile.
- **Hierarchy lowering.** A view with `cluster by=hierarchy:skos:broader layout=mindmap` lowers `broader` edges to `parent` links for that view, so a vocabulary renders with today's mindmap and tidy-tree layouts. A concept with several broader concepts gets one tree parent (the first in id order) and its other parents as cross-links, the way FreeMind import handles cross links today.
- **Import and export.** `rhizo convert scheme.ttl -o scheme.rhizo` and back. A Markdown or OPML outline can become a SKOS scheme in one step (`rhizo convert outline.md --as skos`), which turns rhizo's existing outline importers into a vocabulary-authoring tool.
- **Mappings and licenses.** `skos:exactMatch` crosswalks between vocabularies are the obvious use, but **the ICD-11 license requires a separate written agreement with WHO for mappings, crosswalks and translations** (ICD-11 license §1.2.4, checked 2026-10-18). rhizo will not ship ICD-11 crosswalks. Rule `kg.vocab.mapping-license` warns when a document maps *from or to* a vocabulary whose pack declares `mappings=restricted`. Whether a user's private, unpublished mapping falls under that clause is unclear (K8).

### 4.6 OWL import

`rhizo kg import-ontology mondo.owl --prefix MONDO` turns an OWL 2 ontology (Recommendation, second edition 2012-12-11) into types:

| OWL | rhizo |
|---|---|
| `owl:Class` with an IRI | node type `PREFIX:local` |
| `rdfs:subClassOf` a named class | type `extends` (the subclass closure is used for type checks, e.g. "range `MONDO:0000001` accepts any disease") |
| `owl:ObjectProperty` | edge type. `rdfs:domain`/`rdfs:range` become domain and range. |
| `owl:DatatypeProperty` | prop entry in a generated JSON Schema, with the XSD range mapped to a JSON Schema type and format |
| `rdfs:label`, `skos:prefLabel`, `skos:altLabel`, `IAO:0000115` (definition) | type labels and descriptions (multilingual) |
| `owl:deprecated true`, `IAO:0100001` (term replaced by) | a deprecation warning with a fix-it to the replacement |
| restrictions, class expressions, other axioms | kept as annotations in the import report and not enforced |

- **Domain and range are hints, not constraints.** Under OWL semantics, `rdfs:domain` *infers* the subject's class instead of rejecting a mismatch. rhizo reads them in a closed-world way as **warnings**, and the docs say so plainly. That is useful for authors, but it is not OWL entailment.
- **Large ontologies are not expanded into the profile manifest.** SNOMED CT has several hundred thousand concepts. An imported ontology becomes an **ontology-backed type set**: types are looked up in the derived index on demand, and the manifest records only the source, version and digest.
- **Parsing.** OWL is read through its RDF serializations (RDF/XML, Turtle) with `oxrdfio`. OWL/XML and Functional Syntax would need `horned-owl` 4.0.0, which is **LGPL-3.0**. That license sits badly with a MIT OR Apache-2.0 binary that links statically, so those syntaxes are left out. Users convert them with ROBOT or another external tool first.

### 4.7 Property-graph exports

| Target | Form | Notes |
|---|---|---|
| **openCypher** (Neo4j, Memgraph) | `rhizo convert kg/ -o out/ --to cypher` writes `nodes-*.csv` and `rels-*.csv` with headers for `neo4j-admin database import` (`:ID`, `:LABEL`, `:START_ID`, `:END_ID`, `:TYPE`), plus `load.cypher` (`LOAD CSV` with `MERGE`) for incremental loads | openCypher's latest release tag is `2024.3` (2026-03-20, Apache-2.0). Neo4j says Cypher is evolving toward GQL. |
| **ISO GQL** (ISO/IEC 39075:2024, published 2024-04-12) | `--to gql`: `INSERT` statements with explicit node and edge ids | **Experimental.** Neo4j's Cypher 25 manual lists GQL conformance with unsupported mandatory features. Memgraph offers GQL through a separate federated engine (MemGQL, v0.1.0 2026-03-29) that translates to Cypher, not natively. A Technical Corrigendum (2026) is listed by national bodies, with a conflicting status on ISO's own page. A revised edition is in development. rhizo emits GQL once one engine runs the output in CI (K11). |
| **GraphML** | Today export-only, with layout ([`canvas.rs`](../../crates/interop/src/canvas.rs)). It gains **import**, plus `<key>`s for `type`, `iri`, `props` (JSON), `prov` (JSON) and edge `id`. | yEd, Gephi, Cytoscape and networkx read it. It is the lowest-friction way into network-analysis tools. |
| **SQL/PGQ** (ISO/IEC 9075-16:2023) | none planned | It reads property graphs out of SQL tables. Anyone who needs it can use the CSVs. |

### 4.8 Vocabulary packs and licenses

```kdl
// kg/graph.kdl
vocab HP    version="2026-09-01" source="https://github.com/obophenotype/human-phenotype-ontology/releases/download/v2026-09-01/hp.owl" sha256="…" license=hpo
vocab MONDO version="2026-10-06" source="https://github.com/monarch-initiative/mondo/releases/download/v2026-10-06/mondo.owl" sha256="…" license=cc-by-4.0
vocab LOINC version="2.83" source="file:~/vocab/Loinc_2.83.zip" license=loinc redistribute=#false
vocab SCTID version="20261001" source="file:~/vocab/SnomedCT_InternationalRF2_PRODUCTION_20261001.zip" license=snomed-affiliate redistribute=#false
```

- **rhizo downloads nothing implicitly.** `rhizo vocab fetch` downloads public packs from their declared `source` on request. Licensed packs are always `file:` sources the user obtained. Each pack is pinned by digest in `rhizo.lock` and cached outside the repository (`~/.cache/rhizo/vocab/`).
- **License metadata travels with the data.** Each pack's license id selects a built-in policy: `attribution` text, `redistribute`, `modify`, `mappings`. The policy drives three gates:
  1. **Export and snapshot gate.** Exporting or snapshotting content from a pack with `redistribute=#false` fails unless `--i-have-rights=<pack>` is passed. Ids alone are always allowed, while labels and relationships are not.
  2. **Attribution.** Exports and rendered views append the required attribution (for example HPO's "uses the Human Phenotype Ontology (version …)").
  3. **No modification.** Imported vocab content is read-only. Local additions go in the user's own namespace and link to the vocabulary, and never edit it. This matters for HPO, whose license forbids altering its content or logical relationships.
- The per-vocabulary terms are summarized in §5.3.2 and Appendix A. **These summaries are not legal advice, and the license texts govern.**

---

## 5. Reasoning graphs for agents

### 5.1 What the reasoning profiles share

`dx`, `argue` and `law` follow one pattern, and a shared internal module (`reason`) implements it:

| Concern | Rule |
|---|---|
| **Authored vs computed** | The source holds what people and accepted agents asserted: findings, LRs, premises, weights, proof standards. **Evaluators** compute posteriors, rankings and acceptability. They are deterministic Rust functions in the profile, and their results are **overlays** (`"source": "evaluator:dx.bayes@0.1.0"`) that are never written into the source. |
| **Provenance** | `kg.prov.required` is unsuppressible. Every `extracted` or `inferred` edge has a resolvable `kg.source` with a pinpoint `locator`, and evaluator outputs carry `derived-from` automatically. |
| **Review** | Agent contributions are `proposed` until a named human accepts them. Evaluators ignore proposed statements by default, and views draw them dashed. |
| **Explanation** | Every computed value comes with its **explanation paths**: the ordered list of statements that produced it, each with its sources. No number appears without the ability to ask "why". |
| **Taint** | A result that depends on a `kg.source kind=illustrative`, on a proposed statement (`--include-proposed`), or on a deprecated vocabulary term carries that label in the overlay. Renderers print it on the view. |
| **Determinism** | Same graph, same profile lock, same result, byte for byte, like the compiler (profiles doc §4.2) |

### 5.2 MCP tools for graphs

These extend `rhizo mcp` (profiles doc §7.1). Results use MCP **structured tool output** (`structuredContent` with an `outputSchema`), which the spec has had since 2025-06-18 and kept in 2026-07-28. Sources are returned as `resource_link` items (`rhizo://source/{id}`). MCP has no dedicated citation field, so rhizo defines one in its output schemas: every edge in every result carries `prov` and `citations`.

| Tool | Purpose |
|---|---|
| `kg_search(text, types?, vocab?, limit?)` | **Entity linking.** Lexical search (BM25 over labels and synonyms, all languages) across the graph index and the user's vocab packs. Returns ids with scores and the matched label. It is deterministic. Embedding search is the agent's business (K10). |
| `kg_neighborhood(ids, depth, via?, direction?, limit?)` | k-hop subgraph, with the same semantics as `rhizo-query`'s `expand` (§2.5) |
| `kg_rank(seeds, via?, k)` | Personalized PageRank from the seeds (fixed iteration count, so deterministic). Ranks the neighborhood when it is too big to return whole. |
| `kg_paths(from, to, via?, max_length, k)` | **Explanation paths**: the k shortest paths, each edge with `prov` and citations |
| `kg_query(query, lang)` | Read-only `rhizo-query`, SPARQL (Oxigraph over the sidecar), or GQL/Cypher (external engines only) |
| `kg_get(id)` / `kg_cite(id)` | A node or edge with props and provenance / its sources with identifiers and locators |
| `kg_communities(graph, resolution?)` | Leiden communities (fixed seed), member lists, and any **accepted** `kg.summary` nodes for them |
| `kg_explain(id)` | The evaluator's explanation of a computed value: contributions in order, each with citations |
| `kg_propose(statements, rationale)` | Proposes nodes and edges. They are validated (schema, SHACL, provenance rules) **before** they are stored in `.rhizo/proposals/`. **A proposal without sources is rejected with the rule id**, not stored. |
| `kg_check_answer(claims)` | The agent submits its answer as claims, each with the edge ids it relies on. The server checks that every cited edge exists, is accepted, has resolvable sources and that the path connects the claim's entities. It returns per-claim `supported` / `unsupported` / `relies-on-proposed`. |
| `dx_rank(case)`, `dx_next_tests(case)` | §5.3 |
| `arg_status(claim \| case)`, `arg_whatif(case, changes)` | §5.5 |

**GraphRAG-style retrieval without an LLM in rhizo.** Microsoft's GraphRAG (v3.3.0, 2026-10-08, MIT) builds a graph with LLM extraction, clusters it hierarchically with Leiden and has an LLM write community summaries that global, local and DRIFT search then use. rhizo keeps the retrieval structure and drops the parts that would put unreviewed model output into the knowledge base:

1. **Link.** The agent maps the mentions in a question to ids with `kg_search`.
2. **Expand.** `kg_neighborhood` with typed predicates gives the local context. `kg_rank` keeps the k most relevant nodes when it would be too large.
3. **Summarize (optional).** `kg_communities` gives structure for global questions. Community summaries exist only as `kg.summary` nodes an agent proposed and a human accepted. rhizo never writes them.
4. **Answer with paths.** The agent writes its answer outside rhizo, cites edge ids, and runs `kg_check_answer` before showing it. Hosts can make a failed check block the answer.

Extraction (an agent reading a paper and proposing LR nodes) is the same loop in reverse: `kg_propose` with `prov method=extracted`, a `source` and a `locator`, and a human accepts or rejects it in the editor or with `rhizo proposals`.

### 5.3 The `dx` profile: differential diagnosis

#### 5.3.1 Types

| Type | Layer | Props / roles | Notes |
|---|---|---|---|
| `dx.case` (frame) | case | `setting`, `population`, `exclusive` | **No direct identifiers.** Rule `dx.case.identifiers` (error) rejects props such as `name`, `dob`, `mrn`, `address`. |
| `dx.finding` | case | `code` (HP, LOINC, SNOMED CT), `state` (`present` / `absent` / `value`), `value`, `unit` (UCUM) | Uncoded findings are allowed and warned about. They never match an LR. |
| `dx.hypothesis` | case | `code` (MONDO, ORPHA, ICD-11, SNOMED CT), `red-flag`, `thresholds { test; treat }` | Thresholds are authored inputs with provenance. **rhizo ships no clinical thresholds.** |
| `dx.test` | case | `code` (LOINC where one exists), `risk`, `cost`, `turnaround`, `available` | |
| `dx.population` | knowledge | `setting`, age range | What an LR or prevalence applies to |
| `dx.prevalence` (n-ary) | knowledge | roles `disorder`, `population`; prop `p` | The pretest probability source |
| `dx.lr` (n-ary) | knowledge | roles `finding`, `disorder`, `population`, optionally `replaces` (a finding set, see below); props `lr-pos`, `lr-neg`, `ci` | One sourced likelihood ratio |
| `dx.test-accuracy` (n-ary) | knowledge | roles `test`, `disorder`, `population`; props `sensitivity`, `specificity` **or** `lr-pos`, `lr-neg` | Converted to LRs by the evaluator |
| edges | case | `dx.observed` (case → finding), `dx.considers` (case → hypothesis), `dx.tests` (test → hypothesis), `dx.ruled-out` (case → hypothesis, requires review) | `dx.supports` and `dx.weakens` (finding → hypothesis) are **computed** and appear only in overlays |

#### 5.3.2 Vocabularies and their licenses (checked 2026-10-18)

| Vocabulary | Used for | Release | License | rhizo's handling |
|---|---|---|---|---|
| **HPO** (Human Phenotype Ontology) | findings, phenotypes | v2026-09-01 | Custom HPO license, free to use. It requires citation and display of the version, and **forbids altering the content or logical relationships**. Commercial use is not addressed. It covers the annotation files too. | public pack, read-only, attribution added to exports |
| **Mondo** | diseases | v2026-10-06 | CC BY 4.0 | public pack |
| **Orphanet ORDO** | rare diseases | 4.9 (2026-07-01) | CC BY 4.0, changes must be indicated | public pack |
| **ICD-11 MMS** (WHO) | diagnoses, classification | 2026-01 (announced 2026-02-16) | CC BY-ND 3.0 IGO. Code, title and URI must be kept. **Mappings, crosswalks and translations need a written agreement with WHO.** It may not be used to build a different standard. The ICD-API has separate royalty-free terms. | public pack. `mappings=restricted`. Codes are always displayed with their WHO title and URI, as the license requires. |
| **ICD-10-CM** (US) | US billing diagnoses | FY2027 (effective 2026-10-01) | Usually treated as public domain (CDC). **NCHS's own statement was not verified.** | pack, treated as restricted until verified |
| **SNOMED CT** | findings, disorders, procedures | International Edition 2026-10-01 | **Affiliate License.** Free in member countries through the National Release Centre or MLDS. Fees may apply elsewhere, and apps and browsers need permission. The Global Patient Set is CC BY-ND 4.0 per the current page (older guides said CC BY 4.0), and contains ids and terms only, with no hierarchy. | user-supplied `file:` pack. `redistribute=#false`. |
| **LOINC** | lab tests, observations | 2.83 (2026-08-19) | Free under Regenstrief terms. Field names and content must not be changed, and a notice is required in products. **The full current text was not readable (HTTP 403).** | user-supplied pack. `redistribute=#false` until verified. |
| **UMLS / RxNorm** | crosswalks, drugs | UMLS 2026AA (2026-05-04) | UMLS: a free individual license, with per-source restrictions and no redistribution outside your own applications. RxNorm names and RXCUIs are public domain, but the full release needs the UMLS license. | external graph or user pack only |
| **MeSH** | literature indexing | 2026 | Free. Requires acknowledging NLM, stating the version and not implying endorsement. | public pack |

#### 5.3.3 The evaluator (`dx.bayes`)

- **Odds-form Bayes.** For each hypothesis H in the case:
  - posterior odds = prior odds × Π LRᵢ, where prior odds = p/(1−p) from the matching `dx.prevalence`;
  - LRᵢ is `lr-pos` for a present finding and `lr-neg` for an absent one;
  - only LRs whose population matches the case's population are used.

  This is the standard evidence-based-medicine rule (post-test odds = pre-test odds × LR; Deeks & Altman, BMJ 2004;329:168).
- **Missing evidence is reported, never imputed.** A finding with no matching LR is listed under `no-evidence` for that hypothesis. It does not silently count as LR = 1.
- **The independence assumption is stated every time.** Multiplying LRs assumes the findings are conditionally independent given H. They often are not (tachycardia and dyspnea, or the items of a clinical score). The evaluator always emits `dx.independence` (warning) when it multiplies more than one LR. The mitigation is set-level LRs: a `dx.lr` with a `replaces` role over a finding set, for example a validated score, is used *instead of* its component findings.
- **Uncertainty.** When LRs carry `ci`, the evaluator propagates the bounds and reports a posterior range, not just a point value.
- **No normalization unless declared.** Hypotheses in a differential are rarely mutually exclusive and exhaustive, so each is evaluated against "not H". `exclusive=#true` on the case switches to normalized multi-hypothesis posteriors.
- **Next-best test (`dx_next_tests`)** uses the threshold approach (Pauker & Kassirer, NEJM 1980):
  - For each available test and each hypothesis it has accuracy data for, the evaluator computes the posterior after a positive and after a negative result.
  - It checks whether either result **crosses the hypothesis's test or treatment threshold**.
  - Tests are ranked by: red-flag hypotheses they can resolve, then the number of hypotheses where some result crosses a threshold, then expected entropy reduction, then `risk` and `cost`.
  - The output is a ranked list with the reason for each rank. It is a ranking of information value under the stated inputs, **not an order**.
- **Red flags.**
  - A `red-flag=#true` hypothesis is always in every view of its case (`always`, §2.5), whatever its rank.
  - Rule `dx.red-flag.unaddressed` (error) fires while a red-flag hypothesis has neither a linked `dx.test` nor a `dx.ruled-out` edge with an accepted human review.
  - It cannot be suppressed with `allow`.

#### 5.3.4 Querying over MCP

An agent working a case:

1. `kg_search("pleuritic chest pain")` → `HP:0033771`. Coding the findings is the agent's job, and the human checks it in the view.
2. `kg_propose` adds `dx.finding` nodes to the case graph (`method=extracted`, `source` = the note the finding came from, `review status=proposed`).
3. After human acceptance, `dx_rank(case-17)` and `dx_next_tests(case-17)`.
4. `kg_explain(h-pe)` returns the contributions and their sources (§5.4.4). The agent's summary cites those edge ids and passes `kg_check_answer`.

### 5.4 Worked example: a differential for acute dyspnea

> **Every number in this example is illustrative.** The LRs, prevalences and thresholds are made up to show the mechanics. They come from a `kg.source` with `kind=illustrative`, so every result is tainted and labelled. The vocabulary codes are real (checked 2026-10-18). Real use needs sourced numbers for a matching population (§6.2).

A 54-year-old presents to the emergency department with dyspnea and pleuritic chest pain, nine days after knee surgery.

#### 5.4.1 Knowledge (sidecar excerpt)

```kdl
// kg/knowledge/_local/00.kg.kdl (excerpt, canonical order)
node "Illustrative numbers (not clinical data)" id=src-demo type=kg.source {
    props kind=illustrative url="https://rhizo.dev/examples/dx-illustrative.html" content-sha256="…"
}
node "Adults in the ED with suspected PE" id=pop-ed type=dx.population {
    props setting=emergency age-min=18
}
node "Surgery within the last 4 weeks" id=rf-recent-surgery type=kg.concept   // local concept: HPO is never edited
node "CT pulmonary angiography" id=t-ctpa-concept type=kg.concept
node "Wells score: PE likely" id=wells-score-high type=kg.concept

node id=prev-pe type=dx.prevalence {
    role disorder MONDO:0005279
    role population pop-ed
    props p=0.15
    prov method=asserted by="human:reviewer-a" at="2026-10-18" { source src-demo }
}
node id=lr-pleur-pe type=dx.lr {
    role finding HP:0033771
    role disorder MONDO:0005279
    role population pop-ed
    props lr-pos=1.2
    prov method=asserted by="human:reviewer-a" at="2026-10-18" { source src-demo locator="row 1" }
}
node id=lr-tachy-pe type=dx.lr {
    role finding HP:0001649
    role disorder MONDO:0005279
    role population pop-ed
    props lr-pos=1.8
    prov method=asserted by="human:reviewer-a" at="2026-10-18" { source src-demo locator="row 2" }
}
node id=lr-surgery-pe type=dx.lr {
    role finding rf-recent-surgery
    role disorder MONDO:0005279
    role population pop-ed
    props lr-pos=2.0
    prov method=asserted by="human:reviewer-a" at="2026-10-18" { source src-demo locator="row 3" }
}
node id=acc-ctpa-pe type=dx.test-accuracy {
    role test t-ctpa-concept
    role disorder MONDO:0005279
    role population pop-ed
    props lr-pos=20 lr-neg=0.04
    prov method=asserted by="human:reviewer-a" at="2026-10-18" { source src-demo locator="row 9" }
}
// … the same for pneumonia (MONDO:0005249), pneumothorax (MONDO:0002076),
//   acute coronary syndrome (MONDO:0005542), D-dimer, hs-troponin I and chest X-ray
```

An agent proposing a real, sourced LR looks like this. It stays out of every computation until a human accepts it:

```kdl
node "Ceriani et al., J Thromb Haemost 2010;8(5):957–70" id=src-ceriani-2010 type=kg.source {
    props kind=study pmid="20149072"
}
node id=lr-wells-pe type=dx.lr {
    role finding wells-score-high          // a set-level LR (score), see §5.3.3
    role replaces HP:0001649 HP:0002105 rf-recent-surgery
    role disorder MONDO:0005279
    role population pop-ed
    props { lr-pos "<from the paper>" }     // a proposal carries the extracted number; none is invented here
    prov method=extracted by="agent:claude@run-7f3a" at="2026-10-18" {
        source src-ceriani-2010 locator="Table 3"
        review status=proposed
    }
}
```

#### 5.4.2 The case

```kdl
rhizo version=3
title "Case 17: dyspnea after knee surgery"
use kg version="0.1"
use dx version="0.1"

prefix HP    "http://purl.obolibrary.org/obo/HP_"
prefix MONDO "http://purl.obolibrary.org/obo/MONDO_"
prefix LOINC "https://loinc.org/"                       // IRI form to be confirmed, see Appendix A

graph knowledge path="kg/knowledge/" role=knowledge

frame "Case 17" id=case-17 type=dx.case layout=layered direction=right {
    props setting=emergency population=pop-ed

    node "Pleuritic chest pain" id=f-pleur type=dx.finding { props code=HP:0033771 state=present }
    node "Heart rate 118/min" id=f-tachy type=dx.finding {
        props code=HP:0001649 state=present value=118 unit="/min"
    }
    node "No fever (37.1 °C)" id=f-fever type=dx.finding { props code=HP:0001945 state=absent }
    node "Knee surgery 9 days ago" id=f-surgery type=dx.finding { props code=rf-recent-surgery state=present }

    node "Pulmonary embolism" id=h-pe type=dx.hypothesis {
        props code=MONDO:0005279 icd11="BB00" red-flag=#true {
            thresholds test=0.05 treat=0.60
        }
        prov method=asserted by="human:dr-lehmann" at="2026-10-18" { source src-demo locator="thresholds" }
    }
    node "Pneumonia" id=h-pneu type=dx.hypothesis {
        props code=MONDO:0005249 icd11="CA40" { thresholds test=0.05 treat=0.50 }
    }
    node "Pneumothorax" id=h-ptx type=dx.hypothesis {
        props code=MONDO:0002076 icd11="CB21" red-flag=#true { thresholds test=0.01 treat=0.50 }
    }
    node "Acute coronary syndrome" id=h-acs type=dx.hypothesis {
        // ICD-11 has no code for ACS itself: it is an inclusion under "Acute ischaemic heart disease"
        props code=MONDO:0005542 red-flag=#true { thresholds test=0.01 treat=0.50 }
    }

    node "D-dimer" id=t-ddimer type=dx.test { props code=LOINC:48065-7 risk=low }
    node "hs-Troponin I" id=t-trop type=dx.test { props code=LOINC:89579-7 risk=low }
    node "CT pulmonary angiography" id=t-ctpa type=dx.test { props concept=t-ctpa-concept risk=moderate }
    node "Chest X-ray" id=t-cxr type=dx.test { props risk=low }

    edge case-17 f-pleur type=dx.observed
    edge case-17 f-tachy type=dx.observed
    edge case-17 f-fever type=dx.observed
    edge case-17 f-surgery type=dx.observed
    edge case-17 h-pe type=dx.considers
    edge case-17 h-pneu type=dx.considers
    edge case-17 h-ptx type=dx.considers
    edge case-17 h-acs type=dx.considers
}

view "Case 17 work-up" id=case17-workup layout=layered direction=right {
    query { seed case-17; expand depth=1 via="dx.observed dx.considers"; include t-ddimer t-trop t-ctpa t-cxr }
    style type=dx.hypothesis shape=rounded
}
```

Today `edge a b c` is a chain (a→b→c), so a fan-out from the case node takes one `edge` per target, as above. A shorter fan-out syntax is open question K12.

#### 5.4.3 What `rhizo check` says

```
error[dx.red-flag.unaddressed]: red-flag hypothesis `h-acs` has no linked test and no reviewed rule-out
  ┌─ case17.rhizo:37:5
  = help: link a test (`edge t-trop h-acs type=dx.tests`) or record `dx.ruled-out` with a review
error[dx.red-flag.unaddressed]: red-flag hypothesis `h-ptx` has no linked test and no reviewed rule-out
error[dx.red-flag.unaddressed]: red-flag hypothesis `h-pe` has no linked test and no reviewed rule-out
warning[dx.independence]: `h-pe` combines 3 LRs as conditionally independent (f-pleur, f-tachy, f-surgery)
  = help: prefer a set-level LR (e.g. a validated score) with `replaces`
info[dx.no-evidence]: no LR for `f-fever` (HP:0001945, absent) → `h-pe` in population `pop-ed`
note[reason.taint]: all results for `case-17` derive from `src-demo` (kind=illustrative)
```

The three red-flag errors go away once the clinician links tests (`dx.tests` edges) or records reviewed rule-outs. They are errors because "the system was never asked about ACS" is exactly the failure a differential tool exists to prevent.

#### 5.4.4 Evaluator output (overlay)

```json
{
  "rhizo-overlay": 1,
  "source": "evaluator:dx.bayes@0.1.0",
  "inputs": { "graph-digest": "sha256:…", "profile-lock": "sha256:…", "include-proposed": false },
  "taint": ["illustrative:src-demo"],
  "entries": {
    "h-pe": {
      "status": "warning",
      "attrs": {
        "prior": 0.15, "posterior": 0.433, "zone": "test",
        "contributions": [
          { "finding": "f-surgery", "lr": 2.0, "via": "lr-surgery-pe", "citations": [{ "source": "src-demo", "locator": "row 3" }] },
          { "finding": "f-tachy",   "lr": 1.8, "via": "lr-tachy-pe",   "citations": [{ "source": "src-demo", "locator": "row 2" }] },
          { "finding": "f-pleur",   "lr": 1.2, "via": "lr-pleur-pe",   "citations": [{ "source": "src-demo", "locator": "row 1" }] }
        ],
        "no-evidence": ["f-fever"],
        "assumptions": ["conditional-independence"]
      }
    },
    "h-pneu": { "attrs": { "prior": 0.20, "posterior": 0.196, "zone": "test" } },
    "h-ptx":  { "attrs": { "prior": 0.03, "posterior": 0.053, "zone": "test", "red-flag": true } },
    "h-acs":  { "attrs": { "prior": 0.10, "posterior": 0.038, "zone": "test", "red-flag": true } }
  }
}
```

Arithmetic for `h-pe`: prior odds 0.15/0.85 = 0.176, × 1.2 × 1.8 × 2.0 = 0.762, which is a probability of 0.762/1.762 = 0.433. The other hypotheses use illustrative LRs from the elided part of the knowledge excerpt.

`dx_next_tests(case-17)` returns:

| Rank | Test | Hypothesis | After positive | After negative | Why this rank |
|---|---|---|---|---|---|
| 1 | CT pulmonary angiography | PE (red flag) | 0.94: above treat (0.60) | 0.03: below test (0.05) | Resolves a red flag **either way** |
| 2 | hs-Troponin I | ACS (red flag) | 0.24: still test zone | 0.004: below test (0.01) | Can rule out a red flag |
| 3 | Chest X-ray | Pneumothorax (red flag), pneumonia | 0.36 / 0.42 | 0.006 / 0.09 | Can rule out a red flag; low risk |
| 4 | D-dimer | PE | 0.56: still test zone | 0.07: still test zone | **Cannot change any decision at this pretest probability** |

The D-dimer row is the point of the example. A test that is often the first instinct carries no decision value once the pretest probability is this high, and the tool can show why with numbers and sources. (Using the D-dimer to rule out PE only at low pretest probability is the textbook teaching, and that is what these illustrative numbers reproduce.)

The rendered view is an ordinary layered diagram: findings on the left, hypotheses in the middle with posterior badges and red-flag outlines, tests on the right. Computed `dx.supports` edges are drawn with a width proportional to |ln LR|, and an "ILLUSTRATIVE" banner sits across the frame.

### 5.5 The `argue` and `law` profiles: legal case analysis

`argue` is domain-neutral: it models argument graphs for ADRs, design reviews, policy debates and research synthesis. `law` adds legal sources, identifiers and legal proof standards. Splitting them keeps the argument machinery usable outside law, and keeps legal-specific rules out of everyone else's way.

#### 5.5.1 The argument model (AIF-compatible)

The model follows the **Argument Interchange Format** (Chesñevar et al., Knowledge Engineering Review 21(4), 2006). Information nodes (I-nodes) hold claims. Scheme-application nodes (S-nodes) hold inferences (RA), conflicts (CA) and preferences (PA). Because inferences are nodes, attacking an inference (an *undercutter*) needs no edge-to-edge link.

| Type | AIF | Props / roles |
|---|---|---|
| `arg.claim` | I-node | `status` (`stated` / `questioned` / `accepted` / `rejected`), `proof-standard`, `burden` (party) |
| `arg.inference` | RA-node (n-ary) | roles `premise` (1..n), `exception` (0..n), `conclusion` (1), or `undercuts` (an inference); props `scheme`, `direction` (`pro` / `con`), `weight` in [0, 1] |
| `arg.conflict` | CA-node | Generated on export from `direction=con` and `undercuts`. It can also be authored directly. |
| `arg.preference` | PA-node | roles `preferred`, `over` |

Schemes are a small open list: `from-rule`, `from-precedent`, `from-evidence`, `from-expert-opinion`, `from-analogy`, `distinguish`, `from-consequences`. Profiles may add more, which follows Walton's argumentation schemes in spirit without copying any catalog's text.

**Mapping to familiar frames.** These are views and labels over the same graph, not separate models:

| Toulmin (1958) | rhizo |
|---|---|
| Claim | the `arg.claim` an inference concludes |
| Data / grounds | the inference's `premise` claims |
| Warrant | the inference's `scheme` plus the rule it applies (a premise claim citing the rule) |
| Backing | the `law.*` source the warrant cites (`prov source`, with a pinpoint) |
| Qualifier | `weight`, and the issue's `proof-standard` |
| Rebuttal | a `direction=con` inference, or an `undercuts` inference |

| IRAC | rhizo |
|---|---|
| Issue | a top-level `arg.claim` with `proof-standard` and `burden` (`law.issue` is an alias that requires both) |
| Rule | `law.act` / `law.decision` sources plus the `from-rule` and `from-precedent` inferences that use them |
| Application | the inference chain from fact claims to the issue |
| Conclusion | the issue's **computed** label (overlay), never an authored field |

A built-in view template, `rhizo view new --template irac <issue>`, lays this out top-down: issue at the top, rules on the left, facts at the bottom, attacks in red.

#### 5.5.2 Evaluation (`arg.carneades`, `arg.grounded`)

- **Carneades-style proof standards** (Gordon, Prakken & Walton, Artificial Intelligence 171, 2007). Each issue declares a standard, and an inference is *applicable* when its premises are acceptable and its exceptions are not. The standards named in the literature are:
  - `scintilla`: at least one applicable pro argument;
  - `preponderance`: the strongest applicable pro outweighs the strongest applicable con;
  - `clear-and-convincing`: preponderance, plus the pro weight is above α and exceeds the con weight by β;
  - `beyond-reasonable-doubt`: clear-and-convincing, plus the con weight is below γ;
  - `dialectical-validity`: an applicable pro and no applicable con.

  α, β and γ are profile settings with documented defaults. The exact formulations differ between Carneades papers, so the implementation must pin one and cite it (K13).
- **Burden of proof** sits on the claim (`burden=defendant`). It decides which side's claim has to meet the standard. This is how "the controller bears the burden of proving the security measures were appropriate" (C-340/21) becomes structural instead of a footnote.
- **Grounded semantics** (Dung's abstract argumentation) is offered as `arg.grounded` for pure attack graphs without weights.
- **Output** is an overlay with `in` / `out` / `undecided` per claim and per inference, plus a justification tree. `arg_whatif(case, changes)` re-evaluates with hypothetical changes ("what if the court finds the attacker never read the data?") **without** touching the source. That is the question a lawyer actually asks.
- **Weights are judgments, not facts.** They are authored, sourced where possible, reviewed, and shown on the view. The evaluator makes their consequences explicit. It does not make them right.

#### 5.5.3 Legal sources and identifiers

| Type | Identifier props | Validation |
|---|---|---|
| `law.act` (a `kg.source`) | `eli` (European Legislation Identifier), `celex`, `akn` (Akoma Ntoso FRBR IRI), national ids | ELI: a URI under a declared ELI base (EU: `http://data.europa.eu/eli/{type}/{year}/{number}/oj`). CELEX: sector, year, type, number (`32016R0679`). AKN: `/akn/{country}/{docType}/…/{date}/{number}` with expression `/{lang}` (Naming Convention v1.0, OASIS Standard 2019-02-21). |
| `law.decision` (a `kg.source`) | `ecli`, `court`, `date`, `case-number`, `celex` | ECLI: `ECLI:{country}:{court}:{year}:{ordinal}`, with the ordinal up to 25 alphanumerics or dots (Council conclusions 2011/C 127/01, revised 2019/C 360/01) |
| `law.document` | `akn` | Akoma Ntoso 1.0 (OASIS Standard 2018-08-29) import: `<article eId="…">` becomes `law.provision` nodes with the `eId` as locator |
| `law.provision` | parent act + `locator` (`Art. 82(1)`) + `in-force-on` | Point-in-time: a citation to a provision names the version date it relies on (K14) |
| `law.matter` (frame) | `forum`, `jurisdiction`, `language` | One matter. Views and evaluators are scoped to it. |
| `law.evidence` | `kind` (document, witness, expert), `exhibit` | A piece of evidence. Inferences from it support fact claims, so a `questioned` fact without one is warned about. |

- **Pinpoints are mandatory.** Rule `law.cite.pinpoint` (error): every `from-rule` and `from-precedent` inference cites its source with a `locator`, such as an article and paragraph for legislation, or a paragraph or operative-part point for judgments.
- **Citations must resolve.** `rhizo kg verify-sources` resolves ECLIs through the e-Justice portal's ECLI resolver (`/ecli/<ECLI>`), and ELI and CELEX through EUR-Lex. It records `ok` or `failed` per source as an overlay. A citation an agent made up fails here, before it reaches a brief. (Courts have sanctioned lawyers for filing AI-fabricated citations, *Mata v. Avianca*, S.D.N.Y. 2023, which is the reason this gate exists.)
- **Outside the EU.** There is no free universal case identifier in the US. `law.decision` then stores the reporter citation string, and the `law` profile's US pack can resolve it through CourtListener's citation-lookup API. That API is a membership or commercial service, and its data license was not verifiable. The Caselaw Access Project's data has been free of access restrictions since March 2024, though a formal CC0 statement was not verified. Citation strings follow the CC0 Indigo Book rather than the copyrighted Bluebook text. See K15.
- **Exports.** AIF (JSON in AIFdb's form, and RDF with the AIF ontology), RDF with the ELI ontology (v1.3, 2020-07-17, EUPL 1.1) for legislation, and LegalRuleML (OASIS Standard 2021-08-30) for a later phase, if ever. No ISO or W3C standard for argument graphs exists. The W3C Argument Representation Community Group closed in April 2023 without one.

#### 5.5.4 Querying over MCP

- `arg_status(c-82)` returns the label of the main issue, and for each sub-issue its label, standard, burden, and the applicable pro and con inferences with their citations.
- `kg_paths(f-exfil, c-82)` returns the chains from a fact to the conclusion. Each step is an inference with its scheme and pinpoint citation, which is the "explanation path" an agent quotes.
- `arg_whatif` lets an agent test the brittleness of a conclusion and report it honestly ("the damage finding depends on distinguishing C-687/21; if that fails, the margin narrows to 0.7 vs 0.5").
- `kg_propose` is how an agent adds an argument it found. A `from-precedent` proposal without an ECLI and a pinpoint is rejected, and one whose ECLI does not resolve is flagged.

### 5.6 Worked example: a GDPR damages claim

> **Hypothetical facts, real law.** The parties and facts are invented. The legislation and judgments are real, and their identifiers and the operative-part points cited below were checked on EUR-Lex on 2026-10-18. Weights are the analyst's judgment and are shown as such. This is an illustration of the tool, **not legal analysis of any real matter**.

Müller, a customer of ShopCo, sues for €1,000 in non-material damages under Art. 82(1) GDPR. Attackers exfiltrated ShopCo's customer database through a publicly known, unpatched vulnerability. Müller fears misuse of the data, but no misuse has been shown. ShopCo points to its ISO/IEC 27001 certification and denies any damage.

#### 5.6.1 The graph

```kdl
rhizo version=3
title "Müller v. ShopCo: Art. 82 GDPR"
use kg version="0.1"
use argue version="0.1"
use law version="0.1"

// Sources (in practice these live in a shared `law` sidecar and are reused across matters)
node "GDPR" id=gdpr type=law.act {
    props eli="http://data.europa.eu/eli/reg/2016/679/oj" celex="32016R0679"
}
node "C-300/21 Österreichische Post" id=cjeu-c300-21 type=law.decision {
    props ecli="ECLI:EU:C:2023:370" court="CJEU" date="2023-05-04" case-number="C-300/21"
}
node "C-340/21 Natsionalna agentsia za prihodite" id=cjeu-c340-21 type=law.decision {
    props ecli="ECLI:EU:C:2023:986" court="CJEU" date="2023-12-14" case-number="C-340/21"
}
node "C-687/21 MediaMarktSaturn" id=cjeu-c687-21 type=law.decision {
    props ecli="ECLI:EU:C:2024:72" court="CJEU" date="2024-01-25" case-number="C-687/21"
}
node "BGH VI ZR 10/24" id=bgh-vi-zr-10-24 type=law.decision {
    props ecli="ECLI:DE:BGH:2024:181124UVIZR10.24.0" court="BGH" date="2024-11-18"
}

frame "Müller v. ShopCo" id=matter type=law.matter layout=layered direction=up {
    // Issues and claims
    node "Müller is entitled to compensation under Art. 82(1) GDPR" id=c-82 type=law.issue {
        props proof-standard=preponderance burden=claimant
    }
    node "ShopCo infringed Art. 32 GDPR" id=c-infr type=arg.claim
    node "ShopCo's security measures were appropriate" id=c-appropriate type=arg.claim {
        props proof-standard=preponderance burden=defendant
    }
    node "Müller suffered non-material damage" id=c-damage type=arg.claim {
        props proof-standard=preponderance burden=claimant
    }
    node "The damage was caused by the infringement" id=c-causal type=arg.claim

    // Facts
    node "Attackers exfiltrated the database incl. Müller's data" id=f-exfil type=arg.claim { props status=accepted }
    node "The vulnerability was public and unpatched for 5 months" id=f-unpatched type=arg.claim { props status=questioned }
    node "ShopCo held an ISO/IEC 27001 certificate" id=f-iso type=arg.claim { props status=accepted }
    node "Müller fears misuse of the data" id=f-fear type=arg.claim { props status=questioned }
    node "Fear is a trivial, everyday annoyance" id=f-trivial type=arg.claim { props status=stated }
    node "No misuse of Müller's data has been shown" id=f-no-misuse type=arg.claim { props status=accepted }

    // Inferences (RA-nodes)
    node id=r-82 type=arg.inference {
        role premise c-infr c-damage c-causal
        role conclusion c-82
        props scheme=from-rule weight=0.9
        prov method=asserted by="human:lawyer-k" { source gdpr locator="Art. 82(1)" }
    }
    node id=r-infr type=arg.inference {
        role premise f-exfil
        role exception c-appropriate              // burden on ShopCo: C-340/21, operative part pt 3
        role conclusion c-infr
        props scheme=from-rule weight=0.8
        prov method=asserted by="human:lawyer-k" {
            source gdpr locator="Art. 5(2), 24, 32"
            source cjeu-c340-21 locator="operative part, pt 3"
        }
    }
    node id=r-appr-iso type=arg.inference {
        role premise f-iso
        role conclusion c-appropriate
        props scheme=from-evidence weight=0.4
        prov method=asserted by="human:lawyer-k" { source cjeu-c340-21 locator="operative part, pt 2" }
    }
    node id=r-appr-unpatched type=arg.inference {
        role premise f-unpatched
        role conclusion c-appropriate
        props scheme=from-evidence direction=con weight=0.6
        prov method=asserted by="human:lawyer-k" { source cjeu-c340-21 locator="operative part, pt 1–2" }
    }
    node id=r-fear type=arg.inference {
        role premise f-fear
        role conclusion c-damage
        props scheme=from-precedent weight=0.7
        prov method=asserted by="human:lawyer-k" { source cjeu-c340-21 locator="operative part, pt 6" }
    }
    node id=r-loss-of-control type=arg.inference {
        role premise f-exfil
        role conclusion c-damage
        props scheme=from-precedent weight=0.5     // persuasive national authority for a German forum
        prov method=asserted by="human:lawyer-k" { source bgh-vi-zr-10-24 locator="headnote" }
    }
    node id=r-no-misuse type=arg.inference {
        role premise f-no-misuse
        role conclusion c-damage
        props scheme=from-precedent direction=con weight=0.5
        prov method=asserted by="human:lawyer-k" { source cjeu-c687-21 locator="operative part, pt 5" }
    }
    node id=u-distinguish type=arg.inference {
        role premise f-exfil
        role undercuts r-no-misuse                // C-687/21: recipient never became aware of the data
        props scheme=distinguish weight=0.7
        prov method=asserted by="human:lawyer-k" { source cjeu-c687-21 locator="operative part, pt 5; para. 68" }
    }
    node id=r-trivial type=arg.inference {
        role premise f-trivial
        role conclusion c-damage
        props scheme=from-consequences direction=con weight=0.3
        prov method=asserted by="human:lawyer-k"
    }
    node id=u-no-threshold type=arg.inference {
        role premise f-trivial
        role undercuts r-trivial                  // no "degree of seriousness" threshold
        props scheme=from-precedent weight=0.9
        prov method=asserted by="human:lawyer-k" { source cjeu-c300-21 locator="operative part, pt 2" }
    }
    node id=r-causal type=arg.inference {
        role premise f-exfil
        role conclusion c-causal
        props scheme=from-evidence weight=0.6
        prov method=asserted by="human:lawyer-k"
    }
}
```

`rhizo check` before evaluation:

```
ok[law.cite.pinpoint]: every from-rule and from-precedent inference cites a locator
warning[arg.unsupported-fact]: `f-unpatched` is questioned and has no evidence inference
  = help: add an inference from a `law.evidence` node (e.g. the forensic report) or mark it accepted
warning[arg.unsupported-fact]: `f-fear` is questioned and has no evidence inference
info[kg.sources]: 5 sources resolved (ECLI ×4 via e-Justice, ELI ×1 via EUR-Lex), see overlay `verify-sources`
```

#### 5.6.2 Evaluation

Assume evidence nodes are added for the two questioned facts (a forensic report and Müller's statement, weights 0.7 and 0.6). The facts are then acceptable, and `arg.carneades` yields:

| Claim | Standard, burden | Applicable pro | Applicable con | Label |
|---|---|---|---|---|
| `c-appropriate` | preponderance, ShopCo | 0.4 (ISO certificate) | 0.6 (unpatched vulnerability) | **out**: ShopCo does not carry its burden |
| `c-infr` | preponderance | 0.8 (exfiltration, exception `c-appropriate` is out) | none | **in** |
| `c-damage` | preponderance, Müller | 0.7 (fear, C-340/21), 0.5 (loss of control, BGH) | `r-no-misuse` undercut by the distinction; `r-trivial` undercut by C-300/21 | **in** (0.7 vs none) |
| `c-causal` | preponderance | 0.6 | none | **in** |
| `c-82` | preponderance, Müller | 0.9 | none | **in** |

`arg_whatif(matter, {"u-distinguish": "rejected"})` shows that if the court does not accept the distinction from C-687/21, `c-damage` stays **in**, but on 0.7 against 0.5. The agent's report must say that. If `f-fear` also fails for lack of evidence, the claim rests on the BGH line alone (0.5 vs 0.5), which **fails** preponderance. That brittleness is what the tool exists to surface.

The IRAC view renders the issue at the top, three sub-issues below, inference glyphs between them, ECLI-labelled source nodes at the side, and the two undercutters as red edges into the inferences they attack. Every edge carries its pinpoint on hover, and in the SVG `<title>` for accessibility.

---

## 6. Safety, regulation and audit

### 6.1 Decision support, not decisions

What the design commits to:

1. **Outputs are framed as computed under stated assumptions.** Overlays carry `assumptions`, `taint` and `inputs`. Renderers print the assumptions on the view, and neither the evaluator nor the MCP tools ever phrase a result as "the diagnosis" or "the outcome".
2. **The basis is always reviewable.** Every number and label has explanation paths down to cited sources with pinpoints (§5.1). This is also what FDA's fourth CDS criterion asks for (§6.2).
3. **Humans accept knowledge.** Agent statements are `proposed` until a named person accepts them. Evaluators ignore proposals by default, and red-flag rules cannot be suppressed.
4. **Audit trail.**
   - Sources and sidecars are in git, so every change has an author, a time and a diff.
   - Reviews are recorded in `prov.review` and in commit trailers (`Rhizo-Review: accepted lr-wells-pe`).
   - For reasoning profiles, `.rhizo/proposals/` is **committed** rather than git-ignored, including rejected proposals. This answers the profiles doc's Q12 for this case.
   - Evaluator overlays record the input digests and evaluator version, and `rhizo kg audit <id>` reconstructs who asserted what, from which source, and which results depended on it.
5. **Data protection.**
   - Case graphs hold health data, a special category under GDPR Art. 9, or privileged legal material.
   - rhizo is local-first: `rhizo mcp` uses stdio by default, `--http` binds to localhost unless told otherwise, and there is no telemetry.
   - `dx.case.identifiers` rejects direct identifiers, and exports and snapshots of case graphs warn.
   - Whether an MCP-connected *agent* sends case data to a model provider is the deployer's decision, and the docs must say so plainly.
6. **Known failure modes are named in the product.**

   | Failure mode | Mitigation |
   |---|---|
   | Automation bias | Rankings, not answers; the assumptions banner |
   | Correlated findings | `dx.independence`; set-level LRs |
   | Wrong population | Population matching; `no-evidence` instead of guessing |
   | Stale vocabularies | Deprecation warnings |
   | Fabricated citations | `verify-sources`; `kg_check_answer` |
   | Extraction errors | Review |

### 6.2 Medical: regulatory notes

**I am not a regulatory expert, and this section is not a regulatory determination.** It records what was checked on 2026-10-18 and where the uncertainty lies. Before any `dx` release that is not marked experimental, qualified regulatory counsel should review it (exit criterion of phase K6).

- **EU MDR (2017/745), Annex VIII Rule 11.** "Software intended to provide information which is used to take decisions with diagnosis or therapeutic purposes is classified as class IIa." It is IIb or III when such decisions could cause serious deterioration or death. MDCG 2019-11 (Rev. 1, June 2025) is the qualification and classification guidance.
  - **Qualification depends on the manufacturer's intended purpose.** Software for general purposes is not a medical device even when used in healthcare (MDR recital 19), and that covers rhizo core, `kg` and `argue`.
  - A `dx` profile that ranks diagnoses and recommends tests *for individual patients* would very likely be medical device software, at least class IIa, **if placed on the market with that purpose**.
  - **Decision:** `dx` ships with a stated intended purpose of education, research and knowledge curation, "not for the diagnosis or treatment of individual patients". It carries an in-product banner and has no "clinical mode".
  - **Uncertain:** whether such a statement holds when the functionality is plainly diagnostic, since regulators look at labelling, claims and features together. Also uncertain: how "making available … free of charge" applies to a community open-source project with no legal manufacturer. Anyone deploying `dx` for patient care becomes a manufacturer or an in-house health institution under Art. 5(5), with all that entails.
- **EU AI Act (2024/1689), as amended by the Digital Omnibus on AI (Regulation (EU) 2026/1744, 8 July 2026, OJ 24 July 2026).**
  - An AI system that is, or is a safety component of, a device needing notified-body assessment is high-risk under Art. 6(1) and Annex I. Per the Omnibus's recital 40, those obligations apply from **2 August 2028**.
  - Annex III systems are high-risk from **2 December 2027**.
  - The recital was read directly. The amended article text was not, because the page was truncated.
  - **Uncertain:** whether a deterministic Bayesian evaluator over authored LRs is an "AI system" under Art. 3(1) at all (K16). An LLM agent using it through MCP is part of the deployer's system either way.
- **US FDA: Clinical Decision Support Software guidance**, current version of **29 January 2026**. It supersedes a 6 January 2026 revision, which in turn superseded the 28 September 2022 final guidance.
  - Software meeting all four criteria of FD&C Act §520(o)(1)(E) is not a device:
    1. it does not process images, signals or patterns;
    2. it displays or analyzes medical information;
    3. it supports or makes recommendations to a healthcare professional;
    4. the professional can independently review the basis.
  - The 2026 version adds enforcement discretion where only one recommendation is clinically appropriate. Time-criticality and automation bias remain factors under criterion 4.
  - `dx` takes coded findings only (criterion 1), addresses professionals (criterion 3) and exposes its full basis (criterion 4). **But the worked example (ED work-up for PE) is exactly the kind of time-critical setting where criterion 4 is weakest.** That is one more reason the example is educational.

### 6.3 Legal: constraints

- **Not legal advice.** `argue` and `law` are tools for lawyers, law students, researchers and courts' staff, used under their own professional rules.
- **Germany.** A *Rechtsdienstleistung* is any activity in concrete third-party matters that requires a legal examination of the individual case (§ 2(1) RDG). The BGH held that a document generator for contracts is not such a service (I ZR 113/20, 9 September 2021, *Smartlaw*). Local software for professionals is clearly outside the RDG. **A hosted service that applies `law` to consumers' individual cases may not be**, and needs its own assessment.
- **UK.** The Legal Services Act 2007 s.12 reserves six activities, including rights of audience and the conduct of litigation. Analysis tools are not among them, but services built on them may touch them.
- **US.** Unauthorized-practice rules are set state by state and were not surveyed. ABA Formal Opinion 512 (29 July 2024) tells lawyers using generative AI to attend to competence, confidentiality, communication, supervision, candor and fees. rhizo's citation verification and audit trail support the candor and supervision duties, but they do not discharge them.
- **EU AI Act, Annex III point 8(a).** AI systems "intended to be used by a judicial authority or on their behalf to assist a judicial authority in researching and interpreting facts and the law" are high-risk from 2 December 2027 (Omnibus). rhizo is not intended for that use. A court or ADR body deploying it with an AI agent should treat the combination as potentially in scope.
- **Confidentiality and privilege** follow §6.1: local-first, no telemetry, and explicit warnings on export.

---

## 7. Layout and rendering

### 7.1 Which layouts for which views

| View | Layout | Status |
|---|---|---|
| SKOS schemes, issue trees, taxonomies | `mindmap` / `tree` over a hierarchy edge type (§4.5) | **existing engines** |
| Reasoning graphs (findings → hypotheses → tests; facts → inferences → issues) | `layered` (Sugiyama) | **existing engine.** It fits because reasoning graphs are nearly DAGs. |
| Exploratory neighborhoods, communities, "what is around X" | **`stress`** (new, the default for KG views) | Phase K2 |
| Interactive exploration in the editor | **`force`** (new) | Phase K7 |

**Stress** (stress majorization: Gansner, Koren & North, GD 2004) is the default for general graphs. It approximates graph-theoretic distances, is deterministic given its initialization, and is what Graphviz's `neato` and ELK's `org.eclipse.elk.stress` use.

- For views beyond a few thousand nodes, the **sparse stress model** (Ortmann, Klimenta & Brandes, JGAA 21(5), 2017) aggregates distant terms through pivots and replaces the O(n²) all-pairs version.
- Initialization uses **pivot MDS** (Brandes & Pich, GD 2006), which is deterministic when pivots are chosen by max-min distance starting from the smallest id.

**Force-directed** layout (Fruchterman–Reingold 1991; a ForceAtlas2-style degree-weighted repulsion, Jacomy et al. 2014, with Barnes–Hut approximation) serves the interactive "let it settle" feel in the editor.

- The `forceatlas2` crate is **AGPL-3.0**, which is incompatible with rhizo's license policy, and `fdg` is unpublished or stale. So rhizo implements it in `rhizo-layout`.
- That fits roadmap item 4 ("Force-directed layout"). This addendum orders stress first and force second.

### 7.2 Determinism

Batch layouts (`rhizo render --view`) are seeded from the view id and run fixed iteration counts with no time-based stopping, so the same input gives the same output. Floating-point results can differ between architectures (for example through fused multiply-add). The plan:

- avoid FMA-sensitive formulations in the kernels;
- round output coordinates to 1/100 px before writing;
- compare golden SVGs on both x86_64 and aarch64 in CI.

If exact cross-architecture equality proves too costly, golden tests fall back to per-architecture files (K17).

### 7.3 Neighborhood expansion

The editor's exploration actions are **edits to the view text**, so exploration is saved, undoable and diffable:

| Action | Writes |
|---|---|
| Expand node X (by edge type) | `expand from=X depth=1 via="…"` in the view's `query` |
| Hide node X | `exclude X` |
| Show a search result | `include X` |
| Drag | `pin X x= y=` |
| Collapse a cluster | `collapse …` |

**Incremental layout keeps the mental map.** New nodes start at the barycenter of their already-placed neighbors, plus a small deterministic offset. Stress then runs with existing nodes either fixed or heavily weighted toward their old positions, and pins are hard constraints. Prior art: ELK stress's `fixed` option, yFiles' "from sketch" incremental modes, and stress majorization under separation constraints (IPSep-CoLa, Dwyer, Koren & Marriott, IEEE TVCG 2006). Neo4j Explore (formerly Bloom) and Kumu's "focus" are the UX references for expansion by relationship type and step-by-step focus.

### 7.4 Clustering

- `cluster by=community` runs **Leiden** (Traag, Waltman & van Eck, Sci. Rep. 9:5233, 2019) with a fixed seed.
- Candidate crates: `graphrs` 0.12.0 (MIT, Louvain and Leiden) and `leiden-rs` 0.8.1 (MIT OR Apache-2.0). Microsoft's `network_partitions` (the Rust core of graspologic-native, MIT) is not on crates.io.
- None has been evaluated for correctness. GraphRAG pins graspologic-native below 1.3 because a minor release changed Leiden's results, so the chosen crate must be pinned and covered by golden tests (K18).
- `cluster by=type`, `prop:<key>` and `hierarchy:<edge-type>` are cheap alternatives.
- **Drawing.**
  - Expanded clusters are drawn as **hulls** behind their members. This does not need compound layout (profiles doc Q3); the layout only adds a weak attraction within each cluster.
  - Collapsed clusters are meta-nodes with a count, and their edges are aggregated with a multiplicity badge.

### 7.5 Rendering knowledge-graph semantics

- **Edge types** get default looks from profiles (profiles doc §3.3), and **review state** has a look of its own: proposed is dashed and muted, rejected is hidden unless asked for, and accepted is normal.
- **Provenance badges** show the number of sources and a warning glyph for unresolved ones. Computed edges use width for magnitude (|ln LR|, argument weight).
- **Overlays** (posterior, acceptability label) render as badges, as in profiles doc §8. Taint renders as a frame banner.
- **Labels use level of detail.** Below a zoom threshold, labels give way to colored glyphs. Label collisions are resolved with an R-tree (`rstar` 0.13.0).
- **Static output.** Views export to SVG, PNG and PDF like any diagram. Above about 2,000 nodes, `rhizo render` warns that a static image is probably the wrong medium and suggests clustering.

### 7.6 Large graphs in the native editor

The editor (roadmap item 3: wgpu + vello) is not built yet, so this section sets **targets to verify with a spike**, not measured numbers:

| Target | Approach |
|---|---|
| 60 fps pan and zoom with 10,000 visible nodes and 30,000 edges | Viewport culling with `rstar`. Per-node vello scene fragments are encoded once and re-appended with transforms. Below a zoom threshold, nodes and edges are drawn as instanced points and lines in a separate wgpu pipeline instead of vello paths. |
| Views over 100,000-node graphs | Level of detail through clustering: far zoom shows collapsed communities, and expanding loads members. |
| Responsive layout | Layout runs on a worker thread and streams positions every few iterations. The UI interpolates between them. Batch determinism is unaffected because batch mode does not stream. |
| Hit testing and nearest neighbor | `rstar` for rectangles, `kiddo` 6.3.0 for points |
| Text | `parley` 0.12.0 with a glyph cache. Only labels above the LOD threshold are shaped. |
| Memory | `u32` interned ids, CSR adjacency and struct-of-arrays positions from the derived index (§2.3) |

Versions checked 2026-10-18: `wgpu` 30.0.1, `vello` 0.11.0, `parley` 0.12.0, `rstar` 0.13.0, `kiddo` 6.3.0, `petgraph` 0.8.3. The reference points for scale are claims, not measurements:

- Sigma.js v4 (WebGL) claims smooth performance at tens of thousands of nodes and edges.
- cosmos.gl (GPU simulation and rendering, an OpenJS Foundation incubation project since 2025-05) claims over a million.
- Gephi rewrote its OpenGL engine in 0.11.1 (2026-05).

Whether vello's compute pipeline holds 10,000 nodes without the instanced fallback is the first question the spike answers.

---

## 8. Phasing

This slots into the profiles doc's §9. The "K" phases can run alongside the profiles doc's phases 3–9 once their prerequisites land.

| Phase | Depends on | Delivers |
|---|---|---|
| **1b: graph-ready core** (**before the native editor**, with or right after Phase 1) | Phase 1 | `rhizo version=3`. `base` and `prefix`. CURIE ids and CURIE type names. Edge `type`. One id namespace for nodes and edges, with edge-to-edge endpoints. `prov` and `role` stored. Value annotations for language and datatype in props. `graph`, `view` and `vocab` statements parsed and stored (no engine yet). Round-trip and golden tests as in profiles doc §2.10. **No semantics.** |
| 2 (profiles doc), extended | 1b | The profile manifest also declares `edge-type`s (domain, range, inverse, characteristics, default kind) and `nary` types with roles |
| **K1: `kg` interop** | 2 | `kg` profile vocabulary. Turtle, N-Triples, N-Quads, TriG and JSON-LD import and export (RDF 1.2 behind `rdf-12`). SKOS import, export and hierarchy lowering. GraphML import and richer export. openCypher CSV export. Provenance rules. |
| **K2: graph store and views** | K1 | `rhizo-kg` sidecar with canonical shards, derived index, `rhizo-query`, views, drift digests, `rhizo render --view`, `rhizo diff` for graphs, **stress and sparse stress layout**, Leiden clustering, hulls |
| **K3: validation and vocabularies** | K2 | SHACL (rudof `shacl`). OWL import as ontology-backed type sets. Vocab packs with license gates and attribution. `rhizo kg verify-sources`. |
| **K4: MCP for graphs** | K2, profiles doc phase 5 | The §5.2 tools, proposals with mandatory provenance, `kg_check_answer`, committed proposal log |
| **K5: `argue` + `law`** | K4 | Argument model, Carneades and grounded evaluators, `arg_whatif`, AIF export, ECLI/ELI/CELEX/AKN validation and resolution, IRAC view template |
| **K6: `dx`** (experimental) | K4 | Types, `dx.bayes`, next-best test, red-flag rules, illustrative knowledge pack. **Exit criterion for leaving experimental:** an external regulatory review of intended purpose and labelling (§6.2). |
| **K7: scale** | K2, roadmap item 3 | External graphs (SPARQL endpoints, Bolt) with snapshots. Editor large-graph mode (§7.6) and `force` layout. **A rendering spike runs as soon as the editor's canvas exists**, not at the end. |
| K8: GQL export | K1 | When at least one engine can run rhizo's GQL output in CI (K11) |

Why **argue/law before dx**:

- The legal identifiers are free and open (ECLI, ELI, CELEX), while the medical vocabularies are license-encumbered.
- Argument graphs are useful well beyond law: ADRs, design reviews, research synthesis.
- Analysis tools for lawyers carry far less regulatory weight than diagnostic software (§6).

`dx` benefits from everything K5 shakes out: provenance, review, explanation paths and evaluator overlays.

Why **1b before the editor**: it changes what an edge is. Edges become objects with identity that other statements point at, positions move from nodes to views (pins), and the editor gets two first-class objects (graph and view) instead of one. Retrofitting that into an editor's operation log and text↔canvas sync is far more expensive than one more model PR now.

---

## 9. Open questions

1. **K1: Format version.** `version=3` for knowledge-graph features, or fold them into `version=2` if Phases 1 and 1b ship in one release?
2. **K2: Default sidecar format.** KDL shards (spans, `prov` blocks, JiK props) or canonical Turtle 1.2 (familiar to RDF users, readable by every RDF tool)? The proposal is KDL by default with Turtle supported. Is the mapping layer worth keeping two first-class forms?
3. **K3: Edge-to-edge endpoints.** Keep them in core (needed for RDF 1.2 reifiers as subjects), or require n-ary nodes for every statement about an edge? Edge-to-edge links complicate layout and editing.
4. **K4: When to switch the default RDF export to 1.2.** At Recommendation of RDF 1.2 Concepts and Turtle 1.2, or earlier, given that Oxigraph already supports it behind a flag?
5. **K5: Confidence semantics.** A bare number, or always a named scheme (GRADE for clinical evidence, a profile-defined scale for legal weights)? A bare number invites false precision.
6. **K6: Who curates knowledge packs?** An LR pack is clinical content with liability attached. Should rhizo ship only the illustrative pack and leave real packs to institutions, or host a community pack with an editorial board?
7. **K7: Where `dx` lives.** In-tree, or as a separately governed plugin profile (profiles doc phase 9) with its own maintainers, intended-purpose statement and regulatory owner?
8. **K8: ICD-11 mappings.** Does a user's private, unpublished `skos:exactMatch` from ICD-11 to another vocabulary fall under the license's clause on mappings? A WHO answer is needed before `kg.vocab.mapping-license` is anything but a warning.
9. **K9: HPO "no alteration".** Local concepts in the user's own namespace that link to HPO terms seem compatible with the license. The HPO team should confirm, especially for local `skos:broader` links that point *into* HPO.
10. **K10: Vector search.** `kg_search` is lexical and deterministic. Should rhizo also maintain an optional embedding index (an external model, a non-deterministic input) or leave embeddings to agents?
11. **K11: GQL.** Which engine runs rhizo's GQL output in CI? Neo4j covers "most" mandatory features. Memgraph's GQL is a translating front end. Is openCypher enough for the foreseeable future?
12. **K12: Edge fan-out syntax.** `edge a b c` is a chain today. Reasoning graphs need one-to-many constantly (`case → findings`). Options: `edge a { - b; - c }`, `edge a -> b c`, or relying on `role` and n-ary nodes.
13. **K13: Which Carneades.** The proof-standard formulations differ between the 2007 paper and later Carneades work (the last software release, 4.3, was in 2017). Pin the 2007 definitions, or follow later formalizations such as van Gijzel & Prakken's ASPIC+ reconstruction?
14. **K14: Point-in-time law.** How should provision versions be modelled: ELI's point-in-time URIs, `in-force-on` props, or Akoma Ntoso expressions? This matters for every citation to amended legislation.
15. **K15: US citations.** Is CourtListener's API (membership or commercial, with an unverified data license) acceptable as the default resolver, or should the US pack resolve only against Caselaw Access Project bulk data?
16. **K16: Is `dx.bayes` an "AI system"** under AI Act Art. 3(1)? The answer changes the compliance story but not the design.
17. **K17: Cross-architecture float determinism.** Exact equality, or per-architecture goldens?
18. **K18: Leiden implementation.** Adopt `graphrs` or `leiden-rs` after a correctness review, or port `network_partitions`?
19. **K19: Views in their own files.** Should views live in `*.view.kdl` files that several documents and people can share, the way dashboards are shared?

---

## Appendix A: specs, vocabularies and tools, as of 2026-10-18

Each entry was checked on 2026-10-18 against official pages, registries or release feeds unless marked **unverified**. Versions drift. Profiles pin exact versions, and this table records what the design was written against.

| Spec / vocabulary / tool | State on 2026-10-18 | Relevance and caveats |
|---|---|---|
| RDF 1.2 Concepts | Candidate Recommendation Snapshot 2026-04-07; Semantics CR Draft 2026-09-24 | Triple terms (object position only), `rdf:reifies`, Full vs Basic conformance |
| Turtle / N-Triples / TriG 1.2; N-Quads 1.2 | Working Drafts 2026-10-07; N-Quads WD 2026-10-10 | `<<( s p o )>>`, `<< s p o ~ r >>`, `{| … |}`. The grammar may still change. |
| SPARQL 1.2 Query / Update | WD 2026-10-11 / 2026-10-12 | `VERSION`, `TRIPLE()`, `isTRIPLE` |
| RDF-star CG report | Final CG report (2021); `<< >>` redefined by RDF 1.2 | not targeted. An explicit "superseded" notice was not found. |
| JSON-LD | 1.1 Recommendation 2020-07-16. WG rechartered 2026-01-06 to 2028-01-31; no 1.2 draft published. YAML-LD WD 2026-10-05. | Triple terms likely not before "1.3" (mailing-list post, secondary) |
| SHACL | 1.0 Recommendation 2017-07-20. 1.2: Core WD 2026-10-14, SPARQL WD 2026-10-11, Node Expressions WD 2026-07-21, Rules FPWD 2026-09-17, Profiling FPWD 2026-07-02, UI FPWD 2026-05-26, Compact Syntax editor's draft only | 1.2 adds `sh:reifierShape` |
| SKOS | Recommendation 2009-08-18 | current |
| OWL 2 | Recommendation (second edition) 2012-12-11 | current |
| PROV-O | W3C Recommendation (2013), from memory and not re-checked | `prov` export |
| N-ary relations pattern | W3C WG Note (2006), from memory | n-ary export |
| ISO GQL | ISO/IEC 39075:2024, published 2024-04-12. Technical Corrigendum 1:2026 per UNE/DIN, but **status conflicts** with ISO's committee page. Revised edition in development. | experimental export |
| SQL/PGQ | ISO/IEC 9075-16:2023; Cor 1:2026 per UNE | not targeted |
| openCypher | release `2024.3` (2026-03-20), Apache-2.0 | Neo4j/Memgraph export |
| Neo4j GQL | Cypher 25 manual lists conformance; some mandatory features unsupported; no certification | |
| Memgraph GQL | via MemGQL / Memgraph Zero (v0.1.0 2026-03-29), which translates to Cypher/SQL. Full conformance **unverified**. | |
| PG-Schema | SIGMOD 2023 research proposal, not a standard. Adoption into GQL **unverified**. | |
| `oxigraph` / `oxrdf` / `oxttl` / `oxrdfio` / `spargebra` | 0.5.11 / 0.3.4 / 0.2.4 / 0.2.6 / 0.4.7, MIT OR Apache-2.0. `rdf-12` and `sparql-12` features. | Tracks moving drafts. WASM build of the store **unverified**. |
| `sophia` | 0.10.0, Apache-2.0 / CeCILL-B. RDF 1.2 support **unverified**. | alternative, not chosen |
| `rio_turtle` | 0.8.6, unmaintained ("use oxttl") | not used |
| `horned-owl` | 4.0.0, **LGPL-3.0** | not used (§4.6) |
| rudof (`shacl`, `rudof_rdf`, `rudof_lib`) | 0.3.27 (2026-10-17), MIT OR Apache-2.0. `shacl_validation`/`srdf` superseded. | SHACL engine. Test-suite conformance **unverified**. |
| `json-ld` crate | 0.21.4, MIT/Apache-2.0, JSON-LD 1.1 only | alternative JSON-LD path |
| `petgraph` | 0.8.3, MIT OR Apache-2.0 | algorithms |
| Microsoft GraphRAG | v3.3.0 (2026-10-08), MIT. Leiden via graspologic-native pinned `<1.3`. | prior art (§5.2) |
| LazyGraphRAG | Microsoft Research (2024). OSS availability **unverified**. | prior art |
| LightRAG | 1.5.7 (2026-09-02), MIT | prior art |
| MCP | 2026-07-28 spec keeps `structuredContent`, `outputSchema` and `resource_link` (added 2025-06-18). No citation field. | §5.2 |
| HPO | v2026-09-01. Custom license: attribution and version display, **no alteration** of content or relationships; commercial use not addressed. | `HP:` → `http://purl.obolibrary.org/obo/HP_` |
| Mondo | v2026-10-06, CC BY 4.0 | `MONDO:` |
| Orphanet ORDO | 4.9 (2026-07-01), CC BY 4.0 | |
| ICD-11 MMS | 2026-01 (WHO announcement 2026-02-16), CC BY-ND 3.0 IGO. Mappings, crosswalks and translations need a WHO agreement. Separate ICD-API terms. | IRI base `http://id.who.int/icd/…` from memory, **unverified** |
| ICD-10-CM | FY2027 (effective 2026-10-01). Public-domain status **unverified** from NCHS itself. | |
| SNOMED CT | International Edition 2026-10-01. Affiliate License; free in member countries; fees elsewhere. GPS is CC BY-ND 4.0 per the current page (older guides: CC BY 4.0). | `http://snomed.info/sct` from memory, **unverified** |
| LOINC | 2.83 (2026-08-19). Free under Regenstrief terms; no changes to content; notice required. Full text **unverified** (HTTP 403). | System `http://loinc.org` from memory. The CURIE expansion used in §5.4 is **unverified**. |
| UMLS / RxNorm | UMLS 2026AA (2026-05-04), individual license. RxNorm names/RXCUIs public domain; full release needs UMLS. | |
| MeSH | 2026. Free; acknowledgement, version and no-endorsement terms. | |
| OBO / Bioregistry | `IDSPACE:LOCAL` ↔ `http://purl.obolibrary.org/obo/IDSPACE_LOCAL` | prefix map |
| HL7 FHIR | R5 5.0.0 (2023-03-26) current. R6 not published (ballot 5 and "R6 Stable" snapshot 2026-07-17). | not targeted yet. A FHIR bridge for `dx` findings is a natural later step. |
| CDS Hooks / CQL | CDS Hooks 2.0 (2.0.1 2025-03-12), 3.0.0 normative ballot 2025-12. CQL 1.5.3 normative, 2.0.0 trial use 2026-07-29. | prior art and possible integration |
| HPO / MONDO / ICD-11 / LOINC codes in §5.4 | HP:0100749, HP:0033771, HP:0001649, HP:0002094, HP:0001945, HP:0002105, HP:0012418; MONDO:0005279, 0005249, 0002076, 0005904, 0005542; ICD-11 BB00, CA40, CB21, BB20, BA41; LOINC 48065-7, 89579-7, 59408-5 | checked in OLS4, the ICD-11 browser and loinc.org. ICD-11 has no own code for ACS, and MONDO has no "acute pericarditis". |
| Likelihood ratios | Deeks & Altman, BMJ 2004;329:168–169 | evaluator basis |
| Threshold approach | Pauker & Kassirer, NEJM 1980, from memory | next-best test |
| Ceriani et al. 2010 (PMID 20149072), Stein et al. 2004 (PMID 15096330) | confirmed on PubMed. **No numbers taken from them.** | proposal example only |
| EU MDR Rule 11; MDCG 2019-11 | Rule 11 text checked; MDCG 2019-11 Rev. 1 June 2025 | §6.2 |
| EU AI Act + Digital Omnibus | Regulation (EU) 2026/1744 of 8 July 2026, OJ 2026-07-24. Recital 40: Annex III from 2027-12-02, Annex I from 2028-08-02. The amended article text was not read. | §6.2, §6.3 |
| FDA CDS guidance | 2026-01-29 (supersedes 2026-01-06, which superseded 2022-09-28) | §6.2 |
| ECLI | Council conclusions 2011/C 127/01; revised 2019/C 360/01 (ECLI 2.0, minimum metadata, Dublin Core). Resolver on the e-Justice portal. | No official ECLI ontology found. |
| ELI | Council conclusions 2012/C 325/02 and 2017/C 441/05. Ontology v1.3 (2020-07-17, EUPL 1.1); a v1.4 is claimed by a third party only. | legislation only |
| CELEX | EUR-Lex scheme (e.g. `32016R0679`, case law sector 6 such as `62021CJ0340`) | |
| Akoma Ntoso | v1.0 OASIS Standard 2018-08-29; Naming Convention v1.0 OASIS Standard 2019-02-21. TC activity **unverified**. | import |
| LegalRuleML | Core v1.0 OASIS Standard 2021-08-30 | possible later export |
| AIF | Chesñevar et al. 2006. The AIF spec PDF was unreachable, so its version is **unverified**. AIFdb and xAIF from ARG-tech. W3C Argument Representation CG closed 2023-04-18. | export |
| Carneades | Gordon, Prakken & Walton, AI 171 (2007). Proof-standard names confirmed. Software 4.3 (2017-07-12), dormant. | evaluator |
| ASPIC+ / LKIF Core | Modgil & Prakken AI 195 (2013); LKIF Core OWL (ESTRELLA, 2007) | prior art |
| CJEU C-300/21, C-340/21, C-687/21, C-741/21 | ECLI:EU:C:2023:370 (2023-05-04), ECLI:EU:C:2023:986 (2023-12-14), ECLI:EU:C:2024:72 (2024-01-25), ECLI:EU:C:2024:288 (2024-04-11). Operative parts of 300/21, 340/21 and 687/21 read on EUR-Lex. | §5.6 |
| BGH VI ZR 10/24 | ECLI:DE:BGH:2024:181124UVIZR10.24.0 (2024-11-18), via rechtsinformationen.bund.de (test phase) | §5.6 |
| GDPR | ELI `http://data.europa.eu/eli/reg/2016/679/oj`, CELEX 32016R0679 | §5.6 |
| RDG § 2(1); BGH I ZR 113/20; LSA 2007 s.12; ABA Op. 512 | checked | §6.3 |
| *Mata v. Avianca* (S.D.N.Y. 2023) | from memory, not re-checked | §5.5.3 |
| Caselaw Access Project / CourtListener / Indigo Book | CAP restrictions ended March 2024 (CC0 label **unverified**). CourtListener API is a membership or commercial service (data license **unverified**). Indigo Book CC0. | US pack |
| KDL 2.0 identifiers and value annotations | `:` and `@` allowed in identifiers; value annotations allowed (spec §3.7) | §3.2, §3.7 |
| JiK 4.0.0 | defines only `(array)` and `(object)`; silent on value annotations | §3.7 |
| Stress / sparse stress / pivot MDS | Gansner, Koren & North GD 2004; Ortmann, Klimenta & Brandes JGAA 2017 (CC BY 4.0); Brandes & Pich GD 2006 | §7 |
| ELK stress / Graphviz neato, sfdp | `org.eclipse.elk.stress` exists (elkjs 0.12.0, EPL-2.0); neato uses stress majorization; sfdp is multilevel force-directed (Hu 2005) | prior art |
| ForceAtlas2 / Fruchterman–Reingold | PLoS ONE 2014; Software P&E 1991 | §7 |
| Leiden | Traag, Waltman & van Eck, Sci. Rep. 2019 | §7.4 |
| IPSep-CoLa / WebCola | IEEE TVCG 2006 / WebCola 3.4.0 (2019), maintenance only | §7.3 |
| Rust crates for layout and rendering | `wgpu` 30.0.1, `vello` 0.11.0, `parley` 0.12.0, `rstar` 0.13.0, `kiddo` 6.3.0, `graphrs` 0.12.0 (MIT), `leiden-rs` 0.8.1, `forceatlas2` 0.8.0 (**AGPL-3.0**), `fdg-sim` 0.9.1 (stale) | §7 |
| Sigma.js / cosmos.gl / Cytoscape.js / Gephi | 4.0.3 (MIT) / 3.5.1 (MIT, OpenJS incubation) / 3.34.4 (MIT) / 0.11.3 (GPL-3.0) | scale references (claims) |

## Appendix B: prior art

- **Microsoft GraphRAG, LightRAG**: LLM-built graphs for retrieval. rhizo takes the retrieval structure (link, expand, communities, paths) and replaces unreviewed LLM writes with proposals that carry provenance.
- **Neo4j Explore (Bloom), Kumu, Gephi, Cytoscape**: graph exploration tools. rhizo's difference is that an exploration *is* a text view in git.
- **Protégé, VocBench, Skosmos**: ontology and vocabulary editors. rhizo is not trying to replace them. It draws and reviews what they produce, and authors small vocabularies as mindmaps.
- **Nanopublications**: assertion, provenance and publication info as named graphs per statement. rhizo's `prov` per edge and named graphs per role (§3.8) follow the same idea, and a nanopub export is a plausible later addition.
- **OVA / AIFdb (ARG-tech), Rationale, Kialo**: argument mapping. rhizo adds proof standards, legal identifiers and agent access.
- **Isabel, DXplain, VisualDx**: established diagnostic decision-support products (named from general knowledge, not re-checked). They are curated by clinical editorial teams under medical-device regimes, which is the bar §6.2 refers to.
