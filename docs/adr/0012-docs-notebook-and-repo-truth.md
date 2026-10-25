# 0012. Obsidian is the working notebook; repo docs are technical truth

- Status: Accepted
- Date: 2026-10-19
- Decision log: D16

## Context and Problem Statement

Notes, plans and decisions start in an Obsidian folder. The repository must hold what the code depends on.

Where do project documents live, and which copy is authoritative?

## Considered Options

- The Obsidian folder is the working notebook; repo `docs/` is technical truth

## Decision Outcome

Chosen option: The option listed above, because the log records the rule without further reasons. The log also says each accepted entry gets an ADR in the repo, after which the log becomes an index.

### Consequences

- Good: Technical facts are versioned with the code and reviewed in pull requests.
- Good: The notebook stays free for drafts.
- Bad: Two places to look. Each accepted decision must be copied into the repo as an ADR.

These ADRs are the first set written under this rule; see [the index](README.md).
