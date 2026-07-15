# Public release checklist

AirWiki remains an internal development candidate until every applicable
item below is complete. Internal packaging and an Apache-2.0 source tree do not
constitute a supported public release.

## Project identity and community

- [x] Select [airwiki/airwiki](https://github.com/airwiki/airwiki) as the official
  repository with `main` as its default branch.
- [x] Add the final repository URL to workspace package metadata and every
  distributed package.
- [ ] Configure protected branches and protected release environments.
- [ ] Publish monitored security and Code of Conduct contacts.
- [ ] Enable contribution sign-off enforcement and document the review process.
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
  closure, MCPB, and legal-payload checks.
- [ ] Audit the upstream llama.cpp binary against its linked-source and legal
  closure before public redistribution.

## Windows x64

- [ ] Build the pinned llama.cpp runtime twice in isolated roots and require
  byte-identical output plus a complete build manifest.
- [ ] Sign desktop, bridge, firewall helper, uninstaller, and final NSIS with the
  approved public-trust publisher identity and RFC3161 timestamps.
- [ ] Validate Authenticode, code-signing EKU, durable publisher identity, PE
  version metadata, helper elevation manifest, runtime imports, and exact payload.
- [ ] Build MCPB from the already signed bridge and compare its bytes with the
  installer payload.
- [ ] Install under a clean standard user, verify the materialized uninstaller,
  uninstall, and confirm only explicitly selected AirWiki state is removed.

## Updater and promotion

- [ ] Generate the updater key in a trusted administrative environment.
- [ ] Store encrypted private material and its password separately in a protected
  environment; retain a tested offline recovery copy.
- [ ] Embed the reviewed public key and stable endpoint in the exact release build.
- [ ] Cryptographically verify updater signatures after all native signing and
  notarization.
- [ ] Reject invalid signatures, equal versions, downgrades, replayed historical
  installers, redirects, symlinks, reparse points, and unexpected assets.
- [ ] Create a draft prerelease tied to the exact audited commit.
- [ ] Re-download and verify the complete draft before human promotion.
- [ ] Publish the stable manifest last and never point it at a prerelease.
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
- [ ] At least five nontechnical participants complete onboarding, collection
  review, pairing, background recovery, and permission recovery without a terminal
  or internal identifiers.
- [ ] A human owner approves public promotion after reviewing final hashes,
  notices, SBOM, provenance, acceptance records, and known limitations.

## Current deliberate blockers

The official source repository is [airwiki/airwiki](https://github.com/airwiki/airwiki).
The development baseline still has no public contact, signing credentials,
updater key, or active signed-release workflow. Clearing any one blocker does not
waive the others.
