# 0004. Build the native editor on custom wgpu and vello

- Status: Accepted
- Date: 2026-10-18
- Decision log: D4

## Context and Problem Statement

rhizo plans a native editor. It comes after core and CLI (see [ADR 0005](0005-core-and-cli-first.md)). It needs a GUI and rendering stack.

Which stack should the native editor use?

## Considered Options

- Custom editor on wgpu and vello
- GPUI
- egui
- Tauri

## Decision Outcome

Chosen option: Custom editor on wgpu and vello, because the log names it as the choice over GPUI, egui and Tauri. It records no reasons against the others.

### Consequences

- Good: The editor controls its own rendering.
- Bad: A custom editor means building more UI infrastructure than a ready framework would need (an inference, not a reason from the log).

The log is thin here: it names the alternatives but gives no comparison.
