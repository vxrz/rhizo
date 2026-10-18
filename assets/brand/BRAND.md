# rhizo brand

A rhizome is a root network with no fixed hierarchy: any point can connect to any other. The mark is a seed sending out shoots in the product's branch colors, drawn with the same curves rhizo uses for mindmap edges, plus one dotted cross-link between two branches. That link is what turns the tree into a graph. Don't drop it.

The mark is itself a rhizo document:

```kdl
- "seed" {
    - "indigo" { - "amber" }
    - "sky"
    - "emerald" { - "pink" }
}
edge sky pink line=dotted arrow=none
```

## Files

| File | Use |
|---|---|
| `icon.svg`, `rhizo.icns`, `png/icon-*.png` | App icon (macOS grid squircle), 16–1024 px |
| `favicon.svg`, `png/favicon-32.png`, `png/apple-touch-icon.png` | Web; the SVG favicon follows the browser's color scheme |
| `logo-light.svg`, `logo-dark.svg` | Mark + wordmark, for light and dark backgrounds |
| `mark-light.svg`, `mark-dark.svg`, `mark-mono.svg` | Mark alone; mono uses `currentColor` |
| `wordmark-light.svg`, `wordmark-dark.svg` | Wordmark alone |
| `social-card.svg`, `png/social-card.png` | 1280×640 repository social preview |
| `brand-sheet.html` | Visual guide (built from the template) |

`icon.svg` is hand-tuned. Everything else is generated, so change shapes or colors in `generate.py` and run:

```bash
python3 assets/brand/generate.py
```

To re-rasterize PNGs with rhizo's own renderer:

```bash
cargo run -p rhizo-render --example rasterize -- assets/brand/icon.svg assets/brand/png/icon-512.png 512
```

## Rules

- Clear space: at least the seed's diameter on every side.
- Minimum size: mark 16 px (use the favicon below 32 px), lockup 96 px wide.
- Use the color, light or one-color mark as provided. Don't recolor branches, add effects, or rotate the mark.
- The wordmark is a monoline drawing (x-height 60, stroke 14, round caps), and the tittle on the i is a node. Never set "rhizo" in a font as a logo. In text the name is always lowercase.

## Color

The branch colors are the product theme palette (`crates/core/src/theme.rs`), so rhizo's own mindmaps are on-brand by default.

| Name | Light | Dark | Role |
|---|---|---|---|
| Ink | `#14131f` | | surfaces, text on paper |
| Paper | `#f8f8fc` | | light surfaces, the seed |
| Indigo | `#6366f1` | `#818cf8` | primary accent, the seed's core |
| Sky | `#0ea5e9` | `#38bdf8` | branch |
| Emerald | `#10b981` | `#34d399` | branch |
| Amber | `#f59e0b` | `#fbbf24` | sprout |
| Pink | `#ec4899` | `#f472b6` | sprout |

## Type

| Role | Face | Where |
|---|---|---|
| Display | Unbounded | headlines, social cards, sparingly |
| Text | Figtree | docs, website, marketing |
| Code | JetBrains Mono | snippets, file names |
| Product UI | Inter, then the platform sans | inside diagrams, for legibility at 12–17 px |

All four are under the SIL Open Font License.

## Voice

Plain, precise, a little alive. Write like a good engineer explaining their tool: short sentences, real examples, numbers instead of adjectives ("renders 5,000 nodes in 0.4 s", not "blazing fast").

Taglines: **Think in graphs. Ship in any format.** · Diagrams that grow roots. · Write it. See it. Export it anywhere.

Words we use: graph, node, frame, export, plain text. Words we avoid: synergy, revolutionary, seamless, AI-powered.
