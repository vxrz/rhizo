# 0001. Use Rust only, no Zig

- Status: Accepted
- Date: 2026-10-18
- Decision log: D1

## Context and Problem Statement

rhizo needs text shaping, GPU vector rendering, PDF output, parsers and accessibility support. The language choice decides which libraries are available and how much glue code the project carries.

Which implementation language(s) should rhizo use?

## Considered Options

- Rust only
- Rust plus Zig

## Decision Outcome

Chosen option: Rust only, because Rust has the crates this project needs (text shaping, GPU vector rendering, PDF, parsers, accessibility). Zig would add FFI cost for no measured gain.

### Consequences

- Good: One language and one toolchain across all crates.
- Good: No FFI boundary between components.
- Bad: The project depends on the Rust ecosystem for every capability.

The log gives no further reasoning, and no benchmark is recorded.
