---
title: Release pipeline
---

## Release pipeline

- Merge to main
- Build artifacts
- Unit tests
- Integration tests
- All green?
- Publish crates
- Notify team
- Open incident

## Connections

- Merge to main → Build artifacts
- Build artifacts → Unit tests
- Build artifacts → Integration tests
- Unit tests → All green?
- Integration tests → All green?
- All green? → Publish crates (yes)
- All green? → Open incident (no)
- Publish crates → Notify team
- Open incident → Build artifacts (retry)
