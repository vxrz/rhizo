# 0005. Build core and CLI first, editor second

- Status: Accepted
- Date: 2026-10-18
- Decision log: D5

## Context and Problem Statement

rhizo needs a document model, layout, rendering and interop before an editor can be useful.

In what order should rhizo's parts be built?

## Considered Options

- Core and CLI first, editor second

## Decision Outcome

Chosen option: Core and CLI first, editor second, because the log records the order. `docs/design/profiles-and-compiler.md` adds that the model should be final before the editor, so that its data model, undo stack and text/canvas sync are built on the final node shape.

### Consequences

- Good: The editor starts on a tested model.
- Good: The CLI is usable before the editor exists.
- Bad: There is no graphical editor in the early milestones.

See [ADR 0014](0014-m1-scope-and-syntax-rulings.md) for what the first milestone contains.
