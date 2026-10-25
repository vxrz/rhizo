# Architecture decision records

ADRs use the [MADR 4.0](https://adr.github.io/madr/) layout. One file records one accepted decision. The number follows the order of the decision log, and each file names its log entry (`Decision log: Dn`).

The Obsidian notebook is the working notebook. This folder is the technical truth. Drafts, options and open questions live in the notebook. Once a decision is accepted, it is recorded here, and the repo wins if the two disagree. This rule is itself decision D16 ([ADR 0012](0012-docs-notebook-and-repo-truth.md)).

| No. | Title | Status | Date | Log |
|---|---|---|---|---|
| 0001 | [Use Rust only, no Zig](0001-rust-only-no-zig.md) | Accepted | 2026-10-18 | D1 |
| 0002 | [One document model, every format is an import or export](0002-one-document-model.md) | Accepted | 2026-10-18 | D2 |
| 0003 | [KDL v2 as the native format](0003-kdl-v2-native-format.md) | Accepted | 2026-10-18 | D3 |
| 0004 | [Build the native editor on custom wgpu and vello](0004-native-editor-on-wgpu-and-vello.md) | Accepted | 2026-10-18 | D4 |
| 0005 | [Build core and CLI first, editor second](0005-core-and-cli-first.md) | Accepted | 2026-10-18 | D5 |
| 0006 | [Target macOS, Linux, Windows and the web](0006-targets-macos-linux-windows-web.md) | Accepted | 2026-10-18 | D6 |
| 0007 | [Name the project rhizo](0007-name-rhizo.md) | Accepted | 2026-10-18 | D7 |
| 0008 | [Treat Mermaid and D2 as interop dialects, not renderers](0008-mermaid-and-d2-are-interop-dialects.md) | Accepted | 2026-10-18 | D8 |
| 0009 | [Edges to unknown names create nodes](0009-edges-to-unknown-names-create-nodes.md) | Accepted | 2026-10-18 | D9 |
| 0010 | [Brand: rhizome mark, monoline wordmark, product palette](0010-brand-rhizome-mark.md) | Accepted | 2026-10-18 | D14 |
| 0011 | [Host on Peerlayer Forgejo; sign every commit; branches and PRs only](0011-forgejo-hosting-and-signed-commits.md) | Accepted | 2026-10-19 | D15 |
| 0012 | [Obsidian is the working notebook; repo docs are technical truth](0012-docs-notebook-and-repo-truth.md) | Accepted | 2026-10-19 | D16 |
| 0013 | [Merge mode: fast-forward only](0013-fast-forward-only-merges.md) | Accepted | 2026-10-19 | D17 |
| 0014 | [M1 scope and syntax rulings](0014-m1-scope-and-syntax-rulings.md) | Accepted | 2026-10-19 | D18 |
| 0015 | [Dragging nodes (option A)](0015-dragging-nodes.md) | Accepted | 2026-10-20 | D19 |
| 0016 | [Public mirror and releases on GitHub](0016-public-mirror-on-github.md) | Accepted | 2026-10-25 | — |

Proposed decisions have no ADR yet. They get one when they are accepted.

| Log | Topic | Status |
|---|---|---|
| D10 | License split (libraries and CLI MIT OR Apache-2.0; future server parts AGPL-3.0-or-later; packs per pack) | proposed, no ADR yet |
| D11 | Executable diagrams (typed nodes and kits compile to build specs; see [profiles-and-compiler.md](../design/profiles-and-compiler.md)) | proposed, no ADR yet |
| D12 | Typed, graph-ready core (Phase 1 and 1b) before the native editor | proposed, no ADR yet |
| D13 | Knowledge graphs (a diagram is a view onto a graph; see [knowledge-graphs.md](../design/knowledge-graphs.md)) | proposed, no ADR yet |

D12 is partly covered by the scope in [ADR 0014](0014-m1-scope-and-syntax-rulings.md) (D18).
