# Product map: how rhizo's features fit together

Status: draft, 2026-10-22. The map every new feature is placed on before it is built, so the CLI, the viewer, the MCP server and the web embed stay one product rather than a pile of tools.

## 1. Rule: every capability has three faces

Anything rhizo can do is reachable the same way from each surface:

| Face | What it is | Example (adopt) |
|---|---|---|
| **CLI** | a `rhizo` subcommand, scriptable, deterministic output, used by CI | `rhizo adopt FILE NODE --dry-run` |
| **Viewer** | a command in the one command registry, so it appears in the menu bar, the ⌘K palette, the context menus that fit, and tooltips with its shortcut | Diagram ▸ Adopt node… |
| **MCP** | a tool for agents (write tools only behind `--allow-write`) | `adopt_nodes` |

The **library is the feature**. CLI, viewer and MCP are thin shells over the same function, with the same options and the same report text. A feature that exists in only one face is unfinished.

## 2. Six building blocks

Every feature below is one (or a few) of these. Each block has a registry in code, like the existing `RULES` and `TARGETS`.

| Block | Question it answers | Registry | Exists today |
|---|---|---|---|
| **Kit** | What things exist in this domain, how do they look, what rules hold? | `Kits` (data: `kit.kdl` + JSON Schema) + `RULES` | data, flow, infra, rhizo |
| **Target** | What does the diagram become? | `TARGETS` (`Target` trait) | OpenAPI, AsyncAPI, JSON Schema, Serverless Workflow, Node-RED, OpenTofu (aws, azure, gcp), build brief |
| **Source** | What existing thing becomes a diagram? | **`SOURCES`** (new, mirrors `TARGETS`) | `import tofu` (+ fold/adopt); interop formats (Mermaid, draw.io, …) |
| **Overlay** | What live or derived facts sit on top of a diagram? | **`OVERLAYS`** (new) | problems banner, references (ad hoc) |
| **Check** | Is it right? | kit rules, audits, L1/L1-adopt round trips | yes |
| **Agent link** | How do agents create, read and build from diagrams? | MCP tools + build brief + guide | yes |

### 2.1 Sources (new registry)

`rhizo import <source> <input> -o file.rhizo`. A source:
- produces typed nodes in the right kit (generic nodes where it cannot type something, never dropping anything);
- **round-trips where a target exists**: import → compile must give back the input, checked like L1 (verbatim plus fold, as for OpenTofu);
- reports coverage (share typed) and what it could not read;
- can be re-run: deterministic ids from the source, so a re-import is a small diff.

Viewer: File ▸ Import ▸ (one entry per source), also by dropping a file or folder on the window.

### 2.2 Overlays (new registry)

An overlay maps node and edge ids to **badges, a colour scale, a value and a tooltip**, plus a legend. It is computed off the UI thread from data outside the diagram and never edits the file.
- CLI: `rhizo render file.rhizo --overlay cost -o out.svg`, and `rhizo overlay <name> file.rhizo --json` for the raw values.
- Viewer: View ▸ Overlays ▸ (toggle each), with a legend in the corner. Hover tooltips show the value.
- MCP: `overlay` tool, so an agent can ask "what is expensive / drifting / failing".

### 2.3 Commands (being built in the UX pass)

One registry: id, label, shortcut, enabled-when, action. The menu bar, palette, context menus and tooltips read it. The CLI and MCP map onto the same ids where an action exists there (`compile`, `check`, `adopt`, …), so documentation can list each command once with its three faces.

## 3. Feature catalogue, placed on the map

Status: ✓ built · ◐ partly · ○ idea.

### Diagrams and editing
| Feature | Blocks | Status |
|---|---|---|
| Stable layout (no jumping on edits), "Tidy layout" | viewer + layout | ◐ in progress |
| Native menu bar, context menus, tooltips, multi-select, real text fields, focus ring | commands | ◐ in progress |
| **Visual diff** `rhizo diff a b` (added green, removed red, changed amber), viewer "Compare with…" (a file, a git revision), Forgejo PR comment with the rendered diff | overlay `diff` + CLI + CI | ○ |
| **Time travel**: a slider over the file's git history in the viewer; `rhizo render --rev` | overlay `diff` + git | ○ |
| **Presentation mode**: step through a sequence or flow (one message or step at a time, narrated from labels and the brief); export as slides (PDF) or an animated SVG | viewer + target `slides` | ○ |
| **Comments and review**: threads on nodes stored next to the file (`.rhizo/comments/`), resolved state, an agent can answer them | kit `rhizo` + MCP | ○ |
| **Templates gallery**: starters per domain, including the domain kits below | `rhizo new` + welcome screen | ◐ (8 starters) |

### Build from diagrams
| Feature | Blocks | Status |
|---|---|---|
| Compile per kit (APIs, flows, infra on 3 clouds) | targets | ✓ |
| Build brief for agents | target `brief` | ✓ |
| **Agent progress**: tasks from the brief shown on their nodes (todo, doing, done, blocked); agents report through MCP `task_status` | overlay `tasks` + MCP | ○ |
| **Ask panel** in the viewer: "add a refund flow", "why is this red?", answered by an agent through the same MCP tools, changes arrive as proposals to accept | viewer + MCP + proposals | ○ |
| **Simulation**: "play" a flow with sample input (the flow kit's jq/JSONata expressions are evaluated), highlight the path taken; a sequence plays its messages | overlay `run` + flow kit | ○ |
| **Architecture docs**: arc42 or C4 documentation from a project (diagrams + brief text + decisions) | target `docs` | ○ |

### Read what exists (sources)
| Source | Kit | Round trip via | Status |
|---|---|---|---|
| OpenTofu/Terraform (fold, adopt) | infra | OpenTofu target, L1 | ✓ |
| docker-compose, Kubernetes manifests | infra (`infra.service`, `infra.container` types) | compose/k8s target (later) | ○ |
| OpenAPI, AsyncAPI | data | data target | ○ |
| SQL schema (Postgres `pg_dump --schema-only`) | data (ER) | SQL DDL target | ○ |
| Node-RED flows | flow | Node-RED target | ○ |
| OpenTelemetry traces | sequence | — (compared against the designed sequence: conformance overlay) | ○ |
| Code structure (Go packages, Cargo workspace, TS imports) | C4 components | — | ○ |

### Live facts (overlays)
| Overlay | Data from | Status |
|---|---|---|
| Problems, references | rhizo itself | ✓ (ad hoc; moves into the registry) |
| **Drift** | live `tofu plan` (L2) against real state | ○ |
| **Cost** | provider price lists (Infracost-style estimate per node, monthly), budget on the environment | ○ |
| **Health and traffic** | Prometheus/VictoriaMetrics/CloudWatch queries named on nodes | ○ |
| **Tasks** | agent progress (above) | ○ |
| **Diff** | another revision | ○ |
| **Threats** | threat-model kit rules (below) | ○ |

### Checks
| Feature | Blocks | Status |
|---|---|---|
| Kit rules, imported-code audit | checks | ✓ |
| **Threat-modelling kit**: trust boundaries as frames, STRIDE prompts per data edge crossing a boundary, findings as an overlay and open questions in the brief | kit `threat` + rules + overlay | ○ |
| **Policy packs**: an organisation's rules as a kit without types (e.g. "every bucket uses a customer-managed key", "no public database"), shared across projects; NIS2/ISO 27001 evidence lists from them | kit (rules only) | ○ |

### Reach
| Feature | Blocks | Status |
|---|---|---|
| **Web viewer** (the WASM build the core crates already pass): read-only, pan/zoom, overlays, for docs sites, Forgejo and Obsidian | surface | ○ |
| **Obsidian plugin**: renders `.rhizo` files and ` ```rhizo ` blocks with the web viewer; decision nodes linked to Decisions notes | web viewer | ○ |
| Packaging: signed and notarized macOS app, Homebrew tap, Linux AppImage, nightly CI with the OpenTofu suites | release | ○ |

## 4. Definition of done for any new feature

1. It is placed in this map: which block, which registry.
2. The library function exists and is tested; output is deterministic where it is a file.
3. All three faces: CLI subcommand or flag, viewer command in the registry (menu, palette, context menu where it fits), MCP tool.
4. Round trip or check where the block requires it (sources: import → compile; targets: validated by the real tool, as `tofu validate` and the Serverless Workflow schema are today).
5. The agent guide (`crates/kit/guide.md`) and the brief know about it, if agents can use it.
6. An example in `examples/`, the baseline snapshot updated on purpose, README and design note updated.

## 5. Order

1. Commit what exists (the consolidated commit plan).
2. Finish the UX pass (stable layout, commands and menus, selection, text fields, focus).
3. Registries for **Sources** and **Overlays**, by moving the existing import and problems/references code into them. Then **diff** as the first new overlay (it is small and makes every later change reviewable).
4. **Web viewer** and the Obsidian plugin (reach), and packaging.
5. **Drift and cost** overlays (L2), then **agent progress** and the **ask panel**.
6. **Domain kits** on the same blocks (for example industrial protocols and pipelines, backend schemas, static sites, home automation), once the core registries are stable.
7. Presentation mode, simulation, threat kit and policy packs as the core grows.
