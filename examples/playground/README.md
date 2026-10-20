# Playground

Files to try rhizo on. Copy the folder first if you want to keep these as they are:

```bash
cp -R examples/playground /tmp/play && cd /tmp/play
```

The commands below assume the CLI is installed (`cargo install --path crates/cli`) and the viewer runs from the repo (`cargo run --release -p rhizo-app -- FILE`).

## The files

| File | Shows |
|---|---|
| `01-mindmap.rhizo` | Mindmap with branch colors, a note, a link and a class |
| `02-flowchart.rhizo` | Layered flowchart: decision, edge labels, a back edge, nodes created by edges |
| `03-canvas.rhizo` | Free canvas: nodes keep their `x`/`y` |
| `04-network.rhizo` | Force layout for a network |
| `05-org-chart.rhizo` | Tidy tree, top down |
| `06-many-frames.rhizo` | Several frames with different layouts in one document, edges across frames, the `paper` theme |
| `07-typed-infra.rhizo` | Format version 2: typed nodes, props, ports, acceptance criteria |
| `08-errors.rhizo` | Deliberate mistakes, to see the diagnostics |
| `09-checkout.mmd` | A Mermaid flowchart to import |
| `10-outline.md` | A Markdown outline that becomes a mindmap |

## In the viewer

```bash
cargo run --release -p rhizo-app -- 01-mindmap.rhizo
```

Open the same file in your editor next to it: changes go both ways.

| Do | How |
|---|---|
| Pan, zoom, fit | scroll; cmd/ctrl+scroll or pinch; `F` or double-click empty space; `0` for 100% |
| Select, move around | click; arrow keys |
| Add a child, a sibling | `Tab`, `Enter`, then type the label and `Enter` (Esc cancels) |
| Rename, delete | `F2` or double-click; Backspace |
| Re-parent (mindmaps, trees) | drag onto the middle of another node; onto its top or bottom quarter to place it before or after |
| Move (free canvas) | drag; watch `x=` and `y=` change in the file |
| Connect | alt+drag from one node to another; in flowcharts any drag connects |
| Everything else | cmd+K |
| Undo, redo | cmd+Z, shift+cmd+Z |

Try `--theme dark` or `--theme paper` on any file.

## On the command line

```bash
rhizo render 01-mindmap.rhizo -o mindmap.png          # also .svg and .pdf
rhizo render 06-many-frames.rhizo -o frames.pdf --theme dark
rhizo check 08-errors.rhizo                           # fix one mistake, run again: more show up
rhizo convert 09-checkout.mmd -o checkout.rhizo        # Mermaid → rhizo
rhizo convert 10-outline.md -o outline.png             # Markdown outline → mindmap image
rhizo convert 02-flowchart.rhizo -o flow.mmd           # → Mermaid; also .d2 .dot .drawio .canvas .opml .mm .graphml .json .yaml
rhizo convert 07-typed-infra.rhizo -o infra.graphml    # typed data survives in GraphML and draw.io
rhizo fmt 01-mindmap.rhizo                             # canonical formatting
rhizo watch 04-network.rhizo -o network.svg            # re-render on every save
rhizo formats                                          # everything rhizo reads and writes
```

Open a `.drawio` result in diagrams.net, a `.canvas` in Obsidian, a `.mm` in Freeplane or XMind, a `.d2` with `d2`.
