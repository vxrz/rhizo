# 0015. Dragging nodes (option A)

- Status: Accepted
- Date: 2026-10-20
- Decision log: D19

## Context and Problem Statement

The editor lets users drag nodes. What a drag means depends on the frame's layout: some layouts store positions, others compute them.

What does dragging a node do in each kind of frame?

## Considered Options

- Option A, described below; the log does not describe the other options

## Decision Outcome

Chosen option: Option A, because the log records it: in free frames a drag moves the node and pins `x` and `y`; in mindmaps and trees, dropping onto the middle of a node re-parents, and dropping on its top or bottom quarter reorders as a sibling; layered frames keep their layout positions and nodes there can only be selected

### Consequences

- Good: Drag behavior follows how each layout works.
- Bad: Layered frames cannot be arranged by hand.

The log is thin here: it does not record the other options or reasons.
