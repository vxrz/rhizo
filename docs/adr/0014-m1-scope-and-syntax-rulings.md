# 0014. M1 scope and syntax rulings

- Status: Accepted
- Date: 2026-10-19
- Decision log: D18

## Context and Problem Statement

The design documents (`docs/design/profiles-and-compiler.md`, `docs/design/knowledge-graphs.md`) propose more than a first milestone can hold. M1 needs a fixed scope and fixed syntax.

What does milestone M1 contain, and which syntax questions are settled?

## Considered Options

- Phase 1 plus the core of Phase 1b; defer the rest

## Decision Outcome

Chosen option: Phase 1 plus the core of Phase 1b; defer the rest, because the log records this scope. Phase 1: `type`, `props` as strict JSON-in-KDL, ports, `accept`, edge `kind`, ports, props and id, and `use`. Phase 1b core: edge `type`, one id namespace for nodes and edges, CURIE ids with `base` and `prefix`, and stored `prov`. There is one format version (`rhizo version=2`). "Profile" is called "kit". Deferred: edge-to-edge endpoints and `role` (K1), value annotations, `graph`, `view` and `vocab` (K1/K2), fan-out syntax (K5). The typed edge shorthand is never added

### Consequences

- Good: The first milestone has a fixed scope and a stable file format.
- Good: One term, "kit", replaces "profile" in the design documents.
- Bad: Several features in the design documents wait for later milestones.

Some older text, such as the README, may still say "profile".
