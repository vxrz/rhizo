# 0002. One document model, every format is an import or export

- Status: Accepted
- Date: 2026-10-18
- Decision log: D2

## Context and Problem Statement

rhizo reads and writes many formats: its own, JSON, YAML, Mermaid, OPML, FreeMind, Markdown, JSON Canvas, draw.io, GraphML, D2, Graphviz and more (see the README). Each format needs a defined relationship to the rest of the tool.

How should rhizo relate its formats to its internal data?

## Considered Options

- One document model; every format is an import or export of it
- A separate model per format (implied by the decision, not discussed in the log)

## Decision Outcome

Chosen option: One document model; every format is an import or export of it, because the log states this rule without further reasons. The README describes the result: one model with many ways in and out.

### Consequences

- Good: Layout and rendering work on one model, whatever the source format.
- Good: A new format needs one importer or exporter, not a path to every other format.
- Bad: A format that cannot express part of the model loses it on export. The README notes that some exports drop typed data with a warning.

The log is thin here.
