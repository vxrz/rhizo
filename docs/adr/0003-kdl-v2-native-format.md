# 0003. KDL v2 as the native format

- Status: Accepted
- Date: 2026-10-18
- Decision log: D3

## Context and Problem Statement

rhizo needs a native text format that people write by hand and that tools can rewrite without destroying the author's layout. JSON and YAML must also work for scripts.

Which syntax should the native format use, and how do JSON and YAML relate to it?

## Considered Options

- KDL v2 with a format-preserving parser; JSON and YAML share one nested schema with string shorthands

## Decision Outcome

Chosen option: KDL v2 with a format-preserving parser; JSON and YAML share one nested schema with string shorthands, because the log records the choice. The README describes the result: each statement is one line, nesting means "child of", and labels double as ids.

### Consequences

- Good: Hand-written files stay line-oriented and readable.
- Good: The format-preserving parser lets tools edit a file and keep its formatting.
- Good: The JSON and YAML forms are lossless and covered by one JSON Schema (`rhizo schema`).
- Bad: The project depends on the KDL v2 specification and its parser.

The log does not list rejected formats.
