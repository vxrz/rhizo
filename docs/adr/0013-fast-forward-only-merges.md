# 0013. Merge mode: fast-forward only

- Status: Accepted
- Date: 2026-10-19
- Decision log: D17

## Context and Problem Statement

Under [ADR 0011](0011-forgejo-hosting-and-signed-commits.md) every commit is signed with the owner's YubiKey. Forgejo also offers squash, rebase and merge-commit buttons.

Which merge style should Forgejo allow on `main`?

## Considered Options

- Fast-forward only; the owner rebases and re-signs locally
- Squash, rebase or merge commit on the server

## Decision Outcome

Chosen option: Fast-forward only, because the server-side styles create commits signed by the server key or not at all. `main` would then no longer carry only YubiKey-signed commits.

### Consequences

- Good: `main` carries only YubiKey-signed commits.
- Good: History is linear.
- Bad: The owner rebases and re-signs locally before each merge.

Set on Forgejo on 2026-10-20 as the only allowed style. `main` is protected: no direct pushes, signed commits required.
