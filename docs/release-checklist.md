# Public release checklist

AirWiki remains an internal development candidate until every applicable
item below is complete. Internal packaging and an Apache-2.0 source tree do not
constitute a supported public release.

## Project identity and community

- [x] Select [airwiki/airwiki](https://github.com/airwiki/airwiki) as the official
  repository with `main` as its default branch.
- [x] Add the final repository URL to workspace package metadata and every
  distributed package.
- [x] Protect `main` with pull requests, strict required checks, linear history,
  conversation resolution, and no force pushes or deletion.
- [x] Configure protected `macos-signing`, `windows-signing`, and
  `public-release` environments for `main`, with required reviewers,
  self-review disabled, and administrator bypass disabled.
- [x] Enable GitHub private vulnerability reporting and link it from the security
  policy.
- [x] Publish public beta support, privacy, FAQ, roadmap, and synthetic-data
  issue-reporting guidance that routes suspected security issues privately.
- [ ] Publish a monitored Code of Conduct enforcement contact.
- [x] Document proportional review and add read-only DCO validation for pull
  requests.
- [x] Require DCO and CI checks through branch protection or repository rulesets.
- [x] Require immutable commit SHAs for external GitHub Actions and make both
  platform frontend checks mandatory on `main`.
- [x] Enable secret scanning with push protection, Dependabot security updates,
  and GitHub CodeQL default setup.
- [ ] Review Apache-2.0, model terms, third-party notices, package metadata, and
  distribution terms with the project owner.

## Reproducible baseline

- [ ] Select an exact reviewed commit with a clean worktree.
- [ ] Run formatting, Clippy, workspace tests, documentation checks, license
  inventory, dependency policy, and advisory review from that commit.
- [ ] Produce final hashes, SBOM, provenance, and legal inventories from the same
  bytes that will be distributed.
- [ ] Build unsigned artifacts without release credentials.
- [ ] Perform native signing and post-signing verification in separate protected
  jobs with credentials scoped to the minimum step.
- [ ] Ensure release automation is reimplemented or revalidated against current
  platform contracts; archived experimental workflows are not acceptable inputs
  without review.

## macOS arm64

- [ ] Sign every owned nested executable with the approved Developer ID identity
  before signing the outer application and DMG.
- [ ] Enable and verify Hardened Runtime.
- [ ] Notarize and staple the application, updater archive, and final DMG as
  applicable.
- [ ] Pass `codesign`, `spctl`, `notarytool`, `stapler`, architecture, runtime
  closure, MCPB, DMG checksum, EULA-resource, and legal-payload checks.
- [ ] Audit the upstream llama.cpp binary against its linked-source and legal
  closure before public redistribution.

## Windows x64

- [ ] Build the pinned llama.cpp runtime twice in isolated roots and require
  byte-identical output plus a complete build manifest.
- [ ] Use the origin-verified SignPath workflow to sign the desktop, bridge and
  firewall helper before packaging, then sign both localized MSI containers
  with the approved public-trust identity and RFC3161 timestamps.
- [ ] Validate Authenticode, code-signing EKU, durable publisher identity, PE
  version metadata, helper elevation manifest, runtime imports, nested MSI
  signatures, and exact payload.
- [ ] Build MCPB from the already signed bridge and compare its bytes with the
  MSI payload.
- [ ] Install both localized MSIs under a clean standard user; verify the fixed
  per-user path, Start-menu entry, upgrade and uninstall; confirm mutable data is
  retained unless a separately confirmed cleanup flow is used.

## Updater and promotion

- [x] Generate the updater key in a trusted administrative environment and
  configure the matching public key as a repository variable.
- [ ] Store encrypted private material and its password separately in a protected
  environment; retain a tested offline recovery copy.
- [ ] Embed the reviewed public key and stable endpoint in the exact release build.
- [ ] Cryptographically verify updater signatures after all native signing and
  notarization.
- [ ] Verify that `latest.json` names the exact release version, platform keys,
  artifact URLs and detached signatures in the candidate.
- [ ] Reject invalid signatures, equal versions, downgrades, replayed historical
  installers, symlinks, reparse points, and unexpected assets. Treat hosting
  redirects as untrusted transport and verify the final downloaded bytes with
  both the updater signature and native platform trust.
- [ ] Create a draft prerelease tied to the exact audited commit.
- [ ] Protect `v*` tags against update and deletion; resolve the release target
  and final tag to the exact audited commit immediately before publication.
- [ ] Require a candidate newer than the current stable version; permit recovery
  only when the candidate is already the stable latest release.
- [ ] Re-download and verify the complete draft before human promotion.
- [ ] Match every legal payload and the complete SPDX dependency inventory to
  the reviewed release commit; match macOS bundle/build metadata and Windows PE
  metadata to the exact requested version.
- [ ] Bind both native verification jobs and final publication to the same
  fingerprinted `SHA256SUMS` inventory.
- [ ] Keep the stable manifest private and verified until the final release
  publication; never point it at a prerelease.
- [ ] Keep the previous stable manifest and artifacts intact on failure.

## Manual acceptance

- [ ] Clean install and upgrade pass on macOS arm64, Windows 10 x64, and Windows
  11 x64.
- [ ] The complete permission, local-network, firewall, tray, autostart,
  accessibility, local-chat, update, recovery, and uninstall paths pass.
- [ ] The [two-node runbook](two-node-runbook.md) passes using only synthetic
  fixtures and sanitized evidence.
- [ ] Wiki repair cancellation writes nothing, confirmed repair withdraws before
  mutation, stale preview is rejected, and ambiguous history remains blocked.
- [ ] At least five nontechnical participants complete onboarding, Wiki
  review, pairing, background recovery, and permission recovery without a terminal
  or internal identifiers.
- [ ] A human owner approves public promotion after reviewing final hashes,
  notices, SBOM, provenance, acceptance records, and known limitations.

## Current deliberate blockers

The official source repository is [airwiki/airwiki](https://github.com/airwiki/airwiki).
The repository now contains fail-closed preparation and promotion workflows,
and the protected environments plus macOS credentials are configured. No
signed and notarized candidate has completed the protected workflow yet. Public
release remains blocked on Windows public-trust signing configuration,
validated updater-key recovery custody, a monitored Code of Conduct contact,
legal review and the complete installed acceptance matrix. Clearing any one
blocker does not waive the others. Follow the
[public release process](release-process.md).
