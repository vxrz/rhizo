# 0008. Treat Mermaid and D2 as interop dialects, not renderers

- Status: Accepted
- Date: 2026-10-18
- Decision log: D8

## Context and Problem Statement

Mermaid and D2 are popular diagram languages. rhizo imports Mermaid and exports Mermaid and D2.

Should rhizo use Mermaid or D2 as a rendering or layout backend?

## Considered Options

- Use them as renderers (implied by the decision, not discussed in the log)
- Treat them as interop dialects; rhizo lays out natively

## Decision Outcome

Chosen option: Treat them as interop dialects; rhizo lays out natively, because the log states the rule. Layout lives in `crates/layout` and rendering in `crates/render`.

### Consequences

- Good: One layout engine gives the same result for every input format.
- Good: No dependency on external renderers.
- Bad: rhizo must maintain its own layout algorithms (tidy trees, mindmaps, layered graphs, grid).

The log gives the rule but no further reasons.
