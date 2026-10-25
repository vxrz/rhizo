# Security policy

Thank you for helping keep rhizo and its users safe.

## Supported versions

rhizo is pre-1.0. Security fixes are made on `main` and released as a new
version; only the **latest release** receives them. Please update before
reporting, and check whether the issue still exists on `main`.

| Version              | Supported          |
| -------------------- | ------------------ |
| latest release (0.x) | ✅                 |
| older releases       | ❌ – please update |

This covers the `rhizo` command-line tool (including `rhizo mcp`), the
`rhizo-view` viewer, the library crates in this repository and the release
archives published on GitHub.

## Reporting a vulnerability

**Please do not open a public issue, discussion or pull request for a security
problem.**

Report it privately via GitHub: **Security → Advisories → Report a
vulnerability** in this repository (GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)).
Only the maintainers can see the report. If you cannot use GitHub, open an
issue that only asks for a private contact – without any details.

Please include, as far as you can:

- the affected version or commit, the platform, and how rhizo ran (CLI command,
  `rhizo mcp` with or without `--allow-write`, the viewer)
- the impact you see (e.g. writing or reading files outside the project, code
  execution from a crafted diagram or import, a crash or hang on untrusted input)
- steps to reproduce or a proof of concept, ideally a minimal input file
- whether the issue is already public or known to others

## What happens next

| Step                                          | Target time                     |
| --------------------------------------------- | ------------------------------- |
| Acknowledgement of your report                | 3 working days                  |
| First assessment (confirmed or not, severity) | 10 working days                 |
| Fix released – critical / high                | 14 / 30 days after confirmation |
| Fix released – medium / low                   | 90 days after confirmation      |

We keep you informed about progress, agree on a disclosure date with you
(normally when the fix is released, at the latest 90 days after the report) and
publish a GitHub security advisory, with a CVE where appropriate. We are happy to
credit you in the advisory – tell us the name you would like to be credited with,
or that you prefer to stay anonymous.

This is a small open-source project without a bug bounty. The times above are
our commitment, but not a contractual guarantee.

## Scope

In scope: everything in this repository – for example parsing and importing
untrusted files (rhizo, Mermaid, D2, draw.io, OPML, OpenTofu code), what the MCP
server lets an agent read and write, how rhizo runs `tofu`, the compiled output
it writes, the viewer, and the release and CI workflows.

Out of scope:

- vulnerabilities in OpenTofu, its providers or modules (report those to their
  maintainers), and in dependencies that rhizo does not use in an exploitable
  way (we still welcome a hint)
- what an agent does with access you granted it on purpose (`--allow-write`),
  unless rhizo lets it go beyond that grant
- infrastructure that compiled OpenTofu code creates: review the plan before you
  apply it
- denial of service that needs an unreasonably large input, social engineering

## Safe harbour

We will not take legal action against research done in good faith that follows
this policy: test only on systems you own or are allowed to test, do not access
or change other people's data, give us reasonable time to fix the issue before
you disclose it, and stop and tell us if you come across sensitive data.

## Verifying a release

Every release archive has a SHA-256 checksum (`SHA256SUMS.txt`), a CycloneDX
SBOM and a signed build provenance attestation:

```bash
gh attestation verify rhizo-<version>-<target>.tar.gz --repo vxrz/rhizo
```
