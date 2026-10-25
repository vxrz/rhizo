# UI parity with the file format

Status: built, 2026-10-22. Everything a `.rhizo` file can say should be sayable in the viewer without touching text: the owner's test case was creating a `Production` environment and placing things in it, which the viewer could not do. This page lists every construct the format has and what in the viewer creates, edits and deletes it, before and after this change, and what is still text-only and why.

Rules it follows (see [product-map.md](product-map.md) §1–2):

- Every edit goes through the `Editor` (`crates/lang/src/edit.rs`) as one undoable, format-preserving text edit: comments, blank lines and the author's layout survive.
- Every action is a command of the one registry (`COMMANDS` in `crates/app/src/palette.rs`), so it shows in the menu bar, the ⌘K palette, the right-click menus and tooltips; the inspector's rows are the other way in.
- The table below is generated from `crates/app/src/parity.rs`. A test checks every keyword the parser reads (`rhizo_lang::kdl::{TOP_LEVEL, NODE_PROPS, …}`) against it: a construct added to the format later without a way in the viewer fails the test until it gets one, or is listed as text-only with the reason. A second test checks this page holds the generated table.

## Before

What the viewer could do before this change ("—": nothing; edits went through the text pane).

| Construct | Create | Edit | Delete |
|---|---|---|---|
| Format version | automatic | automatic | — |
| Document title, theme | — | — | — |
| Layout and direction of nodes outside frames | — | — | — |
| Kits (`use`) | automatic, when a type is set | — | — |
| Imports | Insert reference… | — | — |
| Frame | — | — | — |
| Frame label, id, type, layout, direction, colours, position | — | — | — |
| Frame props | — (the `Editor` had `set_frame_prop`, nothing called it) | — | — |
| Node | Tab, Enter, Add typed node…, Paste | drag (outlines, free frames) | Delete, Cut |
| Node label | with the node | F2, double-click | — |
| Node id | automatic (pinned as `id=new-idea` after typing the first label) | — | — |
| Node type | Change type…, Add typed node… | Change type… | Change type… |
| Node shape, colours | — (only Paste copied them) | — | — |
| Node note, link, tags, classes | — | — | — |
| Node position | drag in free frames, Align | the same | — |
| Node props | inspector | inspector | inspector |
| Acceptance criteria | — | — | — |
| Edge | Connect to…, ⌥-drag | — | Delete |
| Edge label | F2 | F2, double-click | F2 (empty) |
| Edge kind, line, arrowheads, colour, predicate | — | — | — |
| Edge props | — (the `Editor` had `set_edge_prop`, nothing called it) | — | — |
| Ports, provenance, styles, prefixes, base | — | — | — |

## After

| Construct | Written as | Create | Edit | Delete |
|---|---|---|---|---|
| Format version | `top.rhizo` | automatic: raised to 2 when a typed feature is set | automatic: raised to 2 when a typed feature is set | — |
| Document title | `top.title` | inspector: document › Title; “Edit document title” | inspector: document › Title; “Edit document title” | inspector: document › Title |
| Document theme | `top.theme` | inspector: document › Theme; “Document theme…” | inspector: document › Theme; “Document theme…” | inspector: document › Theme |
| Layout and direction of nodes outside frames | `top.layout` `top.direction` | inspector: document › Layout; inspector: document › Direction; “Set layout…”; “Set direction…” | inspector: document › Layout; inspector: document › Direction; “Set layout…”; “Set direction…” | inspector: document › Layout; inspector: document › Direction |
| Kits in use (`use`) | `top.use` | inspector: document › Kits; “Kits in use…”; automatic: added when a type of the kit is set | “Kits in use…” | inspector: document › Kits; “Kits in use…” |
| Imports of other files | `top.import` | “Insert reference…” (⌘R); drag a sidebar thing onto the canvas | — | “Remove unused imports”; automatic: offered when the last reference is deleted |
| Frame | `top.frame` `frame.frame` | “New frame”; “Group into frame” (⌘G) | drag nodes into and out of it | “Delete frame…” (⌫); “Ungroup frame” |
| Frame label | `frame.label` | “New frame”; “Rename frame” (F2) | “Rename frame” (F2); double-click the frame's label | — |
| Frame id | `frame.id` | automatic: pinned when a rename would change it (it names compile output) | — | — |
| Frame type (environment, API, workflow…) | `frame.type` | inspector: frame › Type; “Set frame type…” | inspector: frame › Type; “Set frame type…” | inspector: frame › Type; “Set frame type…” |
| Frame layout and direction | `frame.layout` `frame.direction` | inspector: frame › Layout; inspector: frame › Direction; “Set layout…”; “Set direction…” | inspector: frame › Layout; inspector: frame › Direction; “Set layout…”; “Set direction…” | inspector: frame › Layout; inspector: frame › Direction |
| Frame props | `frame.props` | inspector: frame › Props | inspector: frame › Props | inspector: frame › Props |
| Frame colours | `frame.fill` `frame.stroke` | inspector: frame › Fill; inspector: frame › Stroke; “Set fill colour…”; “Set stroke colour…” | inspector: frame › Fill; inspector: frame › Stroke; “Set fill colour…”; “Set stroke colour…” | inspector: frame › Fill; inspector: frame › Stroke |
| Frame position | `frame.x` `frame.y` | “New frame” | — | — |
| Node | `top.node` `top.-` `frame.node` `frame.-` `node.node` `node.-` | “Add child” (Tab); “Add sibling” (Enter); “Add typed node…”; “Paste” (⌘V); automatic: an edge to a new name | drag onto another node (outlines) or into a frame; “Group into frame” (⌘G) | “Delete” (⌫); “Cut” (⌘X) |
| Node label | `node.label` | “Add child” (Tab); “Add sibling” (Enter) | “Rename” (F2); double-click the node | — |
| Node id | `node.id` | automatic: from the label typed for a new node; pinned on typed nodes and on renames | — | — |
| Node type | `node.type` | inspector: node › Type; “Change type…”; “Add typed node…” | inspector: node › Type; “Change type…” | inspector: node › Type; “Change type…” |
| Node shape | `node.shape` | inspector: node › Shape; “Set shape…” | inspector: node › Shape; “Set shape…” | inspector: node › Shape |
| Node colours | `node.fill` `node.stroke` | inspector: node › Fill; inspector: node › Stroke; “Set fill colour…”; “Set stroke colour…” | inspector: node › Fill; inspector: node › Stroke; “Set fill colour…”; “Set stroke colour…” | inspector: node › Fill; inspector: node › Stroke |
| Node note, link, tags and classes | `node.note` `node.link` `node.tags` `node.class` | inspector: node › Note; inspector: node › Link; inspector: node › Tags; inspector: node › Class | inspector: node › Note; inspector: node › Link; inspector: node › Tags; inspector: node › Class | inspector: node › Note; inspector: node › Link; inspector: node › Tags; inspector: node › Class |
| Node position (free layouts) | `node.x` `node.y` | drag in a free frame; “Align left edges” | drag in a free frame; “Distribute horizontally” | automatic: dropped when the node moves to another frame |
| Node props | `node.props` | inspector: node › Props | inspector: node › Props | inspector: node › Props |
| Acceptance criteria | `node.accept` | inspector: node › Acceptance criteria; “Add acceptance criterion” | inspector: node › Acceptance criteria | inspector: node › Acceptance criteria |
| Edge | `top.edge` `frame.edge` | “Connect to…” (⌥ drag); ⌥-drag from node to node (any drag in flowcharts) | — | “Delete edge” (⌫) |
| Edge label | `edge.label` | inspector: edge › Label; “Edit edge label” (F2) | inspector: edge › Label; “Edit edge label” (F2); double-click the edge | inspector: edge › Label; “Edit edge label” (F2) |
| Edge kind | `edge.kind` | inspector: edge › Kind; “Set edge kind…” | inspector: edge › Kind; “Set edge kind…” | inspector: edge › Kind; “Set edge kind…” |
| Edge line style | `edge.line` | inspector: edge › Line; “Set line style…” | inspector: edge › Line; “Set line style…” | inspector: edge › Line; “Set line style…” |
| Edge arrowheads | `edge.arrow` | inspector: edge › Arrowheads; “Set arrowheads…” | inspector: edge › Arrowheads; “Set arrowheads…” | inspector: edge › Arrowheads; “Set arrowheads…” |
| Edge colour | `edge.stroke` | inspector: edge › Stroke | inspector: edge › Stroke | inspector: edge › Stroke |
| Edge predicate (`type`) | `edge.type` | inspector: edge › Predicate | inspector: edge › Predicate | inspector: edge › Predicate |
| Edge props (`access`, `prefix`, `route`, `cardinality`) | `edge.props` | inspector: edge › Props | inspector: edge › Props | inspector: edge › Props |

Text only (the text pane, ⌘\, edits them):

| Written as | Why |
|---|---|
| `top.base` | base IRI for linked-data ids: rare, and set once |
| `top.prefix` | CURIE prefixes for linked-data ids: rare, and set once |
| `top.style` | named styles (classes): a small stylesheet, clearer as text |
| `node.port` | ports refine a kit type's own; typed nodes get theirs from the kit |
| `node.prov` | provenance is written by importers and agents, and reviewed in proposals |
| `node.text` | text colour: chosen for contrast with the fill |
| `node.stroke-width` | fine typography and line weights |
| `node.font-size` | fine typography and line weights |
| `node.font-weight` | fine typography and line weights |
| `node.line` | an outline's line style: rare on nodes |
| `edge.id` | edge ids are for statements that point at edges (linked data) |
| `edge.class` | classes need a `style`, which is text-only |
| `edge.from-port` | ports come from the kit types at both ends |
| `edge.to-port` | ports come from the kit types at both ends |
| `edge.prov` | provenance is written by importers and agents |
| `edge.fill` | edges have no fill |
| `edge.text` | label colour: chosen for contrast |
| `edge.stroke-width` | fine line weights |
| `edge.font-size` | fine typography |
| `edge.font-weight` | fine typography |
| `edge.shape` | edges have no shape |
| `frame.columns` | grid columns: the grid picks them from the content |
| `frame.text` | label colour: chosen for contrast |
| `frame.stroke-width` | fine line weights |
| `frame.font-size` | fine typography |
| `frame.font-weight` | fine typography |
| `frame.line` | a frame's line style: rare |
| `frame.shape` | frames are always rounded boxes |

## How it works

- **Frames.** Right-click the canvas ▸ New frame makes an empty frame at that point (inside a selected frame, it nests), and its label is typed at once; the id follows the label, as if written by hand. ⌘G groups the selected nodes into a new frame: their statements move into a `frame` block where the first one was, with the edges among them. A click on a frame's label strip (or on its empty space) selects it; F2 or a double-click on the label renames it; ⌫ asks whether to keep what it holds (Ungroup) or delete it all. Dragging a node off every node and onto another frame moves it there (the frame lights up and the pointer says "Move into Production"); dropping it outside every frame moves it to the top level. Frames nest.
- **Inspector.** It shows what is selected: a node (type, look, props, acceptance criteria, references), a frame (type, layout, direction, colours, and its props as a form from its type's schema, so an environment shows `cloud` as a choice, `region`, `state-bucket`, `budget-monthly`, and the `azure` and `gcp` blocks as fields of their own), an edge (label, kind, line, arrowheads, colour, predicate, and the props the kits give that edge: `access` read/write/read-write and `prefix` from a function to a store, `route` from an API to a function, `cardinality` between entities), and, with nothing selected, the document (title, theme, kits, imports, and the layout of nodes outside frames). A typed frame offers "Add typed node…", which lists only the types that fit there (`TypeDef::fits`). The panel scrolls when it runs long.
- **Imports.** Deleting the last node that refers into an import offers to remove the `import` line; Diagram ▸ Remove Unused Imports does it any time.
- **Edge props** live in `rhizo_kit::EDGE_PROPS`, next to the rules that read them, as JSON Schemas like a type's props.

## What remains

- **Ids** are written by the viewer (from a new node's or frame's first label, pinned on renames and on typed nodes) but not edited: changing one breaks references from other files and generated code. Renaming refactors (rewriting edges and imports that use the id) would need project-wide edits.
- **Frame position** is written once, by New frame at a point; frames cannot be dragged yet, nor moved into other frames (nodes can).
- **Imports** cannot be renamed or pointed at another file in the viewer.
- **Text-only constructs**, listed in the table with their reasons: ports, provenance, named styles and classes on edges, prefixes and the base IRI, grid columns, and fine typography (text colour, font size and weight, stroke width).
- A path statement (`edge a b c`) is split into separate `edge` statements when one hop gets its own label, kind or props, as the label already did.
