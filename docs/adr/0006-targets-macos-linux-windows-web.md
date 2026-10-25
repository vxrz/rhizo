# 0006. Target macOS, Linux, Windows and the web

- Status: Accepted
- Date: 2026-10-18
- Decision log: D6

## Context and Problem Statement

rhizo should run on desktop systems and in the browser, so that a web viewer and editor can share the same engine.

Which platforms must rhizo support, and what does that imply for the crates?

## Considered Options

- macOS, Linux, Windows and Web (WASM), with the core crates kept WASM-clean

## Decision Outcome

Chosen option: macOS, Linux, Windows and Web (WASM), because the log fixes these targets. `core`, `lang`, `layout` and `interop` stay free of native dependencies so that they build for WASM.

### Consequences

- Good: A web viewer and editor can share the exact same engine.
- Good: CI enforces it: `.woodpecker/ci.yaml` runs `cargo check --target wasm32-unknown-unknown` on `rhizo-core`, `rhizo-lang`, `rhizo-layout` and `rhizo-interop`.
- Bad: Native dependencies (GPU, PDF, raster) cannot enter the four WASM-clean crates.
