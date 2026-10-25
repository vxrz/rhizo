# 0009. Edges to unknown names create nodes

- Status: Accepted
- Date: 2026-10-18
- Decision log: D9

## Context and Problem Statement

In the native format an `edge` statement refers to nodes by id, by label or by the slug of a label. Authors can mistype a name.

What should happen when an edge names a node that does not exist?

## Considered Options

- Create the node
- Report an error (implied by the decision, not discussed in the log)

## Decision Outcome

Chosen option: Create the node, and warn on a near miss of an existing name, never error, because the log states this rule. The README documents it as part of the `edge` statement.

### Consequences

- Good: Short files: an edge can introduce its own nodes.
- Good: A likely typo gets a warning instead of passing silently.
- Bad: A typo that is not close to an existing name creates a new node with no warning.

The log gives the rule but no further reasons.
