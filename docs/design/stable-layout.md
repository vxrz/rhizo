# Stable layout in the viewer

| | |
|---|---|
| Status | Implemented |
| Date | 2026-10-22 |
| Code | [`crates/layout/src/stable.rs`](../../crates/layout/src/stable.rs), [`tidy.rs`](../../crates/layout/src/tidy.rs), [`layered.rs`](../../crates/layout/src/layered.rs), [`force.rs`](../../crates/layout/src/force.rs), the viewer's [`session.rs`](../../crates/app/src/viewer/session.rs) and [`workers.rs`](../../crates/app/src/viewer/workers.rs) |
| Measured by | [`crates/layout/src/stability_tests.rs`](../../crates/layout/src/stability_tests.rs) |

## Problem

The viewer laid the document out from scratch after every edit. Adding a child or an edge could move nodes the edit never touched: a mindmap recentred its root and moved branches to the other side, a layered graph reordered its layers, a force layout turned or mirrored. People lose their spatial memory of a diagram when that happens. Preserving it is what the graph drawing literature calls keeping the *mental map* (Misue, Eades, Lai & Sugiyama 1995). Experiments show that it helps people remember and find things in a changing graph (Archambault & Purchase 2013).

## Metric

The test takes every example (`examples/`, `examples/playground/`, `examples/shop/`) and applies six single edits to up to four nodes each: add a child, add a sibling, add an edge, rename to a longer label, delete a leaf, and re-parent. It then measures how far the nodes the edit did not touch move **on screen**. That is their move after the viewer's anchoring, with the edited node, any new node and a moved subtree left out. It reports the mean and the maximum distance and the share of nodes that moved more than 8 px.

There are three columns:

- **before:** the old viewer, a layout from scratch held at the old anchor (the selection, else the first node).
- **anchored:** the same layout from scratch, held at the new anchor (see below).
- **after:** the stable layout, held at the new anchor.

`cargo test -p rhizo-layout stability -- --nocapture` prints the table. The test asserts upper bounds on the "after" column.

| layout | edit | cases | before mean / max / >8 px | after mean / max / >8 px |
|---|---|---:|---|---|
| Tree | add child | 4 | 10.9 / 47 / 23% | 0.0 / 0 / 0% |
| Tree | add sibling | 4 | 22.6 / 98 / 68% | 14.0 / 112 / 12% |
| Tree | rename | 4 | 49.1 / 79 / 73% | 13.6 / 157 / 12% |
| Tree | delete leaf | 4 | 36.0 / 56 / 79% | 3.8 / 56 / 8% |
| Tree | re-parent | 3 | 308.6 / 579 / 100% | 24.2 / 183 / 26% |
| Mindmap | add child | 10 | 4.9 / 35 / 25% | 1.1 / 47 / 2% |
| Mindmap | add sibling | 10 | 5.8 / 35 / 31% | 5.4 / 47 / 11% |
| Mindmap | add edge | 10 | 655.4 / 2345 / 80% | 0.0 / 0 / 0% |
| Mindmap | rename | 10 | 58.8 / 93 / 100% | 19.2 / 175 / 32% |
| Mindmap | delete leaf | 9 | 23.0 / 170 / 83% | 0.5 / 24 / 2% |
| Mindmap | re-parent | 7 | 800.5 / 1245 / 100% | 18.3 / 223 / 17% |
| Layered | add child | 44 | 21.4 / 197 / 46% | 5.7 / 89 / 19% |
| Layered | add sibling | 48 | 7.5 / 120 / 18% | 3.2 / 146 / 9% |
| Layered | add edge | 43 | 63.9 / 587 / 67% | 7.8 / 260 / 5% |
| Layered | rename | 48 | 51.6 / 227 / 88% | 25.2 / 157 / 35% |
| Layered | delete leaf | 48 | 59.0 / 702 / 50% | 2.8 / 702 / 3% |
| Layered | re-parent | 42 | 199.5 / 719 / 76% | 0.9 / 69 / 1% |
| Force | add edge | 4 | 314.0 / 1030 / 92% | 38.5 / 130 / 92% |
| Force | rename | 4 | 366.7 / 1868 / 100% | 2.8 / 9 / 2% |
| Force | delete leaf | 4 | 49.7 / 202 / 79% | 1.3 / 5 / 0% |
| Grid | add child | 4 | 58.0 / 244 / 36% | 61.7 / 244 / 36% |
| Grid | rename | 4 | 52.9 / 78 / 94% | 24.4 / 156 / 39% |
| Free | rename | 4 | 38.7 / 62 / 100% | 0.0 / 0 / 0% |
| Sequence | rename | 4 | 65.8 / 72 / 100% | 26.5 / 127 / 33% |

Rows where nothing moves before or after are left out. Most of what still moves has to: siblings pushed aside by a new node, a subtree pushed outward by a wider label, a layer pushed down by a taller node. The 702 px in "layered, delete leaf" is a node that changes frames when the statement that declared it is deleted.

## Approach

The previous layout is session state of the viewer's file watcher, which runs off the UI thread. It is never stored in the file. `rhizo render`, export, compile and the MCP tools lay out from scratch, byte for byte as before, and the snapshot of every example stays identical. The viewer calls `rhizo_layout::stable::layout_from(doc, theme, measure, prev)`, and every engine takes the previous positions as a hint. The same previous layout and the same edit always give the same result.

**Anchoring** (`stable::anchor_shift`). After a relayout the viewer shifts the camera, not the diagram. When most nodes kept their place (as a stable layout does), the viewer holds those still. Otherwise it holds still the selected node. If the edit created, renamed or re-parented that node, its nearest unchanged ancestor is used instead, else the median move. Holding the edited node fixed when it moved itself would make everything else jump. Renaming a node grows it from one side, so its center is not a fixed point.

**Tree and mindmap** (`tidy::layout_kept`). Contour packing stays (it guarantees that no boxes overlap). Siblings keep their distance from the first of them that existed before, unless they must move apart. Gaps stay, and only what would overlap is pushed. A parent keeps its offset from that first child, clamped to its children's span, instead of recentring. An edit in one subtree therefore moves only the siblings after it, and only as far as they must move. A mindmap's branches stay on the side they were on, and a new branch joins the side of the branch before it. This is the incremental tree layout of Moen (1990) applied to the existing contour algorithm.

**Layered** (`layered.rs`), after dynamic Sugiyama layouts (North 1996, "Incremental layout in DynaDAG"; Branke 2001):

1. Cycle breaking keeps the previous directions. A new edge that closes a cycle is the one reversed, not an older one.
2. Each node keeps its previous layer where the edges allow. New nodes go below their predecessors, a backward edge pushes its target down, and empty layers close up.
3. Each layer starts in its previous order. A new node goes right after its siblings, and dummies sit on the line between their edge's ends. Adjacent swaps that remove crossings are always taken. A full barycenter reordering wins only when it removes at least two crossings and a quarter of them. This is hysteresis, so one crossing less does not reshuffle a layer.
4. Breadth positions are pulled towards the previous ones with a weight of 1000 against an edge's 1. Nodes that were there are all but pinned and move only when the spacing in their layer pushes them. New nodes settle between them.
5. Layers keep their previous depth unless the layer before them pushes them, by growing or by gaining a layer in between.

**Force** (`force::layout_from`). Stress majorization starts from the previous positions instead of a circle, so the drawing neither turns nor mirrors. New nodes start beside their neighbors. Each node is also pulled to where it was, which is anchored stress (Brandes, Indlekofer & Mader 2012), and the solver runs 30 iterations instead of up to 300. Components stay where they were, and new components go to the right.

**Grid** keeps its column count, so the grid does not reflow when the number of nodes passes a square. **Frames** stay where their nodes were and are pushed right only when the frame before them grows into them. An `auto` frame that would flip between mindmap and layered (the first link added to a mindmap, the last edge removed from a graph with nesting) keeps its engine.

**Tidy layout** (cmd+K "Tidy layout", `App::tidy_layout`) asks the watcher for one layout from scratch, the same one `render` gives. The selection stays where it is on screen and the rest eases to its new place.

**Agents.** An agent that edits the file over MCP reaches the viewer through the same watcher, so its edits get the same stable relayout.

## Limits

- A stable layout drifts from the one from scratch. Parents end up off-center, gaps stay where nodes were deleted, and new edges in a layered or force layout pull their ends less than a fresh layout would. "Tidy layout" resets all of it. The export then looks different from the window until the person tidies.
- Insertions into a grid shift every later cell, because a grid places nodes by their order. Sequence diagrams are only anchored.
- A node renamed by id in the text pane (not its label) counts as a new node and loses its place.
- Nested frames grow around their center, so the content of a frame that grows on one side moves half the growth.

## Sources

- K. Misue, P. Eades, W. Lai, K. Sugiyama. Layout adjustment and the mental map. *Journal of Visual Languages and Computing* 6(2), 1995.
- S. Moen. Drawing dynamic trees. *IEEE Software* 7(4), 1990.
- S. C. North. Incremental layout in DynaDAG. *Graph Drawing (GD '95)*, LNCS 1027, 1996.
- J. Branke. Dynamic graph drawing. In *Drawing Graphs: Methods and Models*, LNCS 2025, 2001.
- U. Brandes, N. Indlekofer, M. Mader. Visualization methods for longitudinal social networks and stochastic actor-oriented modeling. *Social Networks* 34(3), 2012.
- D. Archambault, H. C. Purchase. The "map" in the mental map: experimental results in dynamic graph drawing. *International Journal of Human-Computer Studies* 71(11), 2013.
- E. R. Gansner, Y. Koren, S. North. Graph drawing by stress majorization. *Graph Drawing (GD 2004)*, LNCS 3383, 2005.
