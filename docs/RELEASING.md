# Releasing rhizo

rhizo lives on the owner's self-hosted Forgejo, where Woodpecker runs
`.woodpecker/ci.yaml`. GitHub (`github.com/vxrz/rhizo`) is a **push
mirror**: Forgejo pushes every branch and tag there, and GitHub Actions runs CI
and builds the releases. Nothing is pushed to GitHub by hand.

| Workflow (`.github/workflows/`) | Runs on | What it does |
|---|---|---|
| `ci.yml` | push to main, pull request, by hand | `cargo fmt`, clippy `-D warnings` and `cargo test --workspace` on Linux, macOS and Windows; the WASM check; every declared `rust-version` |
| `security.yml` | push to main, pull request, weekly | `cargo deny check` (`deny.toml`: RustSec advisories, licences, sources); actionlint and zizmor on the workflows |
| `codeql.yml` | push to main, pull request, weekly | CodeQL for Rust and for the workflows |
| `scorecard.yml` | push to main, weekly | OpenSSF Scorecard |
| `tofu.yml` | weekly, by hand | the `#[ignore]`d OpenTofu suites (`cargo test -- --ignored`, needs `tofu` and the registry) |
| `release.yml` | tag `v*.*.*`, by hand | builds, signs (when set up) and publishes the release |

## Cutting a release

1. **Check main is green**: Woodpecker on Forgejo, and the CI, Security and
   OpenTofu workflows on GitHub (run OpenTofu by hand: Actions → OpenTofu →
   Run workflow).
2. **Bump the version** in `Cargo.toml` (`[workspace.package] version`), then
   `cargo check --workspace` so `Cargo.lock` follows. The release workflow
   refuses a tag that does not match this version.
3. **Update `CHANGELOG.md`** if there is one: a section `## [X.Y.Z] - YYYY-MM-DD`
   becomes the release notes. Without a matching section, GitHub generates notes
   from the commits.
4. **Commit** (signed, as every commit): `chore(release): vX.Y.Z`.
5. **Tag, signed with the YubiKey**:
   ```bash
   git tag -s vX.Y.Z -m "rhizo vX.Y.Z"
   git tag -v vX.Y.Z
   ```
   A tag with a hyphen (`v0.3.0-beta.1`) becomes a pre-release.
6. **Push to Forgejo**: `git push origin main vX.Y.Z`. The push mirror syncs
   on push and hands the tag to GitHub, which starts `release.yml`.
7. **Watch the run** (Actions → Release). When it is done, the release has:
   - `rhizo-X.Y.Z-<target>.tar.gz` (`.zip` on Windows), each with `rhizo`,
     `rhizo-view`, the README and the licence files, for
     `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
     `aarch64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`;
   - `SHA256SUMS.txt`;
   - `rhizo-X.Y.Z.cdx.json` and `rhizo-view-X.Y.Z.cdx.json` (CycloneDX SBOMs);
   - signed build provenance and SBOM attestations (Sigstore, keyless), checked with
     `gh attestation verify rhizo-X.Y.Z-<target>.tar.gz --repo vxrz/rhizo`.

To try the build without releasing: Actions → Release → Run workflow on a
branch. The archives land in the run's artifacts; no release, no attestations.

If a release run fails after the tag is on GitHub, fix the cause on main, then
delete the tag on Forgejo and GitHub, tag again and push. Never move a tag that
has a published release; make a new patch version instead.

## Forgejo push mirror

Done once by the owner. The token is entered only in Forgejo.

1. **GitHub token.** GitHub → Settings → Developer settings → Personal access
   tokens → Fine-grained tokens → Generate new token:
   - Resource owner: `vxrz` (the owner's personal account).
   - Repository access: **Only select repositories** → `rhizo`.
   - Repository permissions: **Contents: Read and write**, **Workflows: Read and
     write** (GitHub refuses pushes that change `.github/workflows/` without it).
     Metadata: Read is added automatically. Nothing else.
   - Expiration: the longest you accept (a year at most), with a calendar
     reminder to rotate it.
2. **Forgejo.** The rhizo repository → Settings → Repository → **Mirror
   settings** → Push to a remote repository:
   - Git remote repository URL: `https://github.com/vxrz/rhizo.git`
   - Username: your GitHub user name; Password: the token
   - **Sync when commits are pushed**: on
   - Mirror interval: e.g. `8h` (a fallback; pushes sync right away)
   - Add push mirror, then **Synchronize now**, and check the branch and tags
     on GitHub.
3. To rotate the token, edit the push mirror's credentials (or delete and add
   it again) and revoke the old token on GitHub.

What a push mirror means for GitHub:

- It runs `git push --mirror`: GitHub's branches and tags are made equal to
  Forgejo's. **Branches created on GitHub are deleted on the next sync**, which
  closes their pull requests. That includes Dependabot's version-update
  branches: treat a Dependabot PR as a notice and make the update on Forgejo
  (or run Renovate on Forgejo instead). Dependabot **alerts** and security
  advisories are unaffected.
- Contributors' pull requests come from forks, so they survive. Merge one by
  fetching it (`git fetch https://github.com/vxrz/rhizo pull/N/head`),
  applying and signing it locally and pushing to Forgejo. Close the PR with a
  link to the commit.
- Merges made in the Forgejo web UI are signed with Forgejo's key, which GitHub
  cannot verify. With "require signed commits" on GitHub (below), such a push is
  rejected and the mirror stops. Merge locally (rebase or fast-forward, signed
  with the YubiKey) and push.

## macOS signing and notarization (optional)

Without these secrets the macOS archives are built unsigned and the signing
steps are skipped. Unsigned downloads are quarantined by Gatekeeper; users run
`xattr -d com.apple.quarantine rhizo rhizo-view` once (README, Install).

Needs an Apple Developer Program membership.

1. **Developer ID Application certificate**: developer.apple.com → Certificates
   → + → Developer ID Application. Install it in Keychain Access, export it with
   its private key as a `.p12` with a strong password.
2. **App Store Connect API key** for notarization: App Store Connect → Users
   and Access → Integrations → App Store Connect API → Team keys → + (role
   Developer). Download the `.p8` once; note the key ID and the issuer ID.
3. GitHub → the repository → Settings → Secrets and variables → Actions →
   **New repository secret**:

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE_P12` | `base64 -i developer-id.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Name (TEAMID)` (`security find-identity -v -p codesigning`) |
| `APPLE_API_KEY_P8` | the contents of `AuthKey_XXXX.p8` |
| `APPLE_API_KEY_ID` | the key ID |
| `APPLE_API_ISSUER_ID` | the issuer ID |

The workflow signs `rhizo` and `rhizo-view` with the hardened runtime and a
secure timestamp, then submits them to the notary service and waits. Bare
executables cannot be stapled: Gatekeeper fetches the ticket online on first
launch. Check a download with `codesign -dv --verbose=4 rhizo` and
`spctl -a -vv -t install rhizo`.

## Windows code signing (optional)

Without these, the Windows archive is unsigned and SmartScreen warns on first
start. Code-signing keys must live in hardware or a cloud HSM nowadays, so the
workflow uses **Azure Artifact Signing** (formerly Trusted Signing):

1. Azure portal: create an Artifact Signing account, finish identity
   validation, create a certificate profile (Public Trust).
2. Create an app registration (Microsoft Entra ID) with a client secret, and
   give it the role **Artifact Signing Certificate Profile Signer** on the
   account.
3. GitHub → Settings → Secrets and variables → Actions:
   - Secrets: `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`
   - Variables: `AZURE_SIGNING_ENDPOINT` (e.g. `https://weu.codesigning.azure.net/`),
     `AZURE_SIGNING_ACCOUNT`, `AZURE_CERTIFICATE_PROFILE`

The step is skipped while `AZURE_CLIENT_SECRET` is not set.

Optional hardening for both: put the signing secrets in a GitHub
**environment** (`release`, deployment rule: tags `v*` only) instead of
repository secrets, and add `environment: release` to the build job in
`release.yml`. "Run workflow" on a branch then needs the rule widened.

## Recommended GitHub settings

A checklist for the owner. Nothing here is applied automatically.

**Before the first push from the mirror**

- [ ] Create `vxrz/rhizo` empty (no README, licence or `.gitignore`),
      **private** at first. Make it public only after CI and a test release
      (a `v0.1.0-rc.1` tag) ran green and the security review is clean.

      While the repository is private on a free personal plan, GitHub offers
      no artifact attestations, code scanning (CodeQL), Scorecard, rulesets or
      push protection for secrets. The attestation steps, CodeQL and Scorecard
      skip themselves while private; the settings below take effect once the
      repository is public. Do the public flip only after a clean review.
- [ ] Settings → General: Features: Issues on; Wiki, Projects, Discussions as
      wanted. Pull requests: allow only **squash** or **rebase** merging (main's
      history stays linear).

**Branch rules** (Settings → Rules → Rulesets → New branch ruleset)

- [ ] Name `main`, enforcement Active, target: default branch.
- [ ] Bypass list: **Repository admin** (the mirror pushes as you, straight to
      main, without a pull request or a finished check).
- [ ] Restrict deletions; Block force pushes.
- [ ] Require linear history.
- [ ] Require signed commits (your YubiKey's GPG or SSH key must be added to
      your GitHub account as a **signing** key, or your commits show as
      unverified and the mirror push is rejected).
- [ ] Require a pull request before merging (applies to everyone except the
      bypass list).
- [ ] Require status checks to pass: `fmt`, `test (ubuntu-latest)`,
      `test (macos-latest)`, `test (windows-latest)`, `wasm`, `msrv`,
      `Dependencies (cargo-deny)` (pick them after the first run, they are
      listed only once they have run).

**Tag rules** (Rulesets → New tag ruleset)

- [ ] Name `release tags`, target `v*`, bypass list: Repository admin.
- [ ] Restrict creations, Restrict updates, Restrict deletions: only the
      mirror (you) creates release tags, and nobody moves or deletes one.

**Actions** (Settings → Actions → General)

- [ ] Actions permissions: **Allow vxrz, and select non-vxrz,
      actions and reusable workflows**, with "Allow actions created by GitHub"
      on and this list:
      `Swatinem/rust-cache@*, taiki-e/install-action@*, opentofu/setup-opentofu@*, ossf/scorecard-action@*, azure/artifact-signing-action@*`
      (`github/codeql-action` counts as created by GitHub).
- [ ] **Require actions to be pinned to a full-length commit SHA**.
- [ ] Workflow permissions: **Read repository contents and packages
      permissions**; "Allow GitHub Actions to create and approve pull requests"
      off. (Each workflow asks for more where it needs it.)
- [ ] Fork pull request workflows: **Require approval for all external
      contributors**.

**Security** (Settings → Advanced Security, formerly "Code security")

- [ ] Private vulnerability reporting: **Enable** (`.github/SECURITY.md`
      points reporters there).
- [ ] Dependency graph: on. Dependabot alerts: **Enable**. Dependabot security
      updates: on (their PR branches are subject to the mirror note above).
- [ ] Secret scanning: **Enable**, with **Push protection** on.
- [ ] Code scanning: leave the default setup off (`codeql.yml` is the advanced
      setup).

**Releases** (Settings → General → Releases, optional)

- [ ] Release immutability: on, so a published release's assets and tag
      cannot be changed. Check that the first release after turning it on still
      publishes.

**The same through the API** (`gh` signed in as the owner; only where the UI
has no equivalent or you prefer a script):

```bash
R=vxrz/rhizo
gh api -X PUT repos/$R/private-vulnerability-reporting
gh api -X PUT repos/$R/vulnerability-alerts
gh api -X PUT repos/$R/automated-security-fixes
gh api -X PATCH repos/$R -F 'security_and_analysis[secret_scanning][status]=enabled' \
  -F 'security_and_analysis[secret_scanning_push_protection][status]=enabled'
gh api -X PUT repos/$R/actions/permissions -F enabled=true -f allowed_actions=selected -F sha_pinning_required=true
gh api -X PUT repos/$R/actions/permissions/selected-actions -F github_owned_allowed=true -F verified_allowed=false \
  -f 'patterns_allowed[]=Swatinem/rust-cache@*' -f 'patterns_allowed[]=taiki-e/install-action@*' \
  -f 'patterns_allowed[]=opentofu/setup-opentofu@*' -f 'patterns_allowed[]=ossf/scorecard-action@*' \
  -f 'patterns_allowed[]=azure/artifact-signing-action@*'
gh api -X PUT repos/$R/actions/permissions/workflow -f default_workflow_permissions=read -F can_approve_pull_request_reviews=false
```

Rulesets are easiest in the UI; via the API they are `POST repos/$R/rulesets`
with a JSON body (export one from the UI with `gh api repos/$R/rulesets/<id>`
to keep it in sync).
