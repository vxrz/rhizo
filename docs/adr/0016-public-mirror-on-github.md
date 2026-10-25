# 0016. Public mirror and releases on GitHub

- Status: Accepted
- Date: 2026-10-25
- Refines: [ADR 0011](0011-forgejo-hosting-and-signed-commits.md)

## Context and Problem Statement

rhizo is developed on the owner's self-hosted Forgejo (ADR 0011), which is not reachable from the internet. To publish it, others need a public place to read the code, report issues and download release builds.

Where does rhizo become public, and where are releases built?

## Considered Options

- Move development to GitHub
- Keep Forgejo as the primary home and push-mirror to GitHub
- Publish release archives only, without the source history

## Decision Outcome

Chosen option: keep Forgejo as the primary home and push-mirror it to `github.com/vxrz/rhizo`, because development, signing and review stay where they are, while GitHub gives public access, CI on three operating systems and release builds.

- Forgejo pushes every branch and tag to GitHub; nothing is pushed to GitHub by hand.
- GitHub Actions runs CI and builds releases from version tags (`docs/RELEASING.md`): checksums, CycloneDX SBOMs and build provenance attestations.
- The repository starts private and becomes public after CI and a test release succeeded and a security review was clean.
- History was rewritten once before the first push to remove an internal host name; commits stay signed by the owner.

### Consequences

- Good: one source of truth, public visibility, reproducible release builds with provenance.
- Bad: the push mirror overwrites branches created on GitHub, so Dependabot's update branches are notices to apply on Forgejo; issues and pull requests on GitHub need to be carried over by hand.
