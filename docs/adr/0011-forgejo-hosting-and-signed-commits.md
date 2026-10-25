# 0011. Host on Peerlayer Forgejo; sign every commit; branches and PRs only

- Status: Accepted
- Date: 2026-10-19
- Decision log: D15

## Context and Problem Statement

The repository needs a host and a rule for who commits what. AI assistants work on the code.

Where is the repository hosted, and how do changes reach `main`?

## Considered Options

- Peerlayer Forgejo; the owner signs every commit with a YubiKey; the assistant stages changes and hands over the commit message; branches and pull requests only, nothing straight to `main`

## Decision Outcome

Chosen option: The option listed above, because the log records the rule without further reasons. CI runs on Woodpecker (`.woodpecker/ci.yaml`) for pushes to `main` and for pull requests.

### Consequences

- Good: Every commit on `main` has a human signature.
- Good: Review happens in pull requests.
- Bad: The owner makes every commit by hand.

See [ADR 0013](0013-fast-forward-only-merges.md) for the merge mode that keeps this property.
