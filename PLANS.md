# Persistent implementation plans

Most changes do not need a checked-in plan. Use a persistent plan only when work spans multiple checkpoints, requires durable coordination, or contains decisions that would otherwise be lost between sessions. Keep only one active persistent plan at a time and keep it short enough to review as a whole.

A plan records intent and acceptance, not a transcript, command log, or speculative design. Update it when evidence changes the approach. Finish it as `Completed` or `Superseded`; move durable architectural decisions into an ADR and user-visible changes into `CHANGELOG.md`.

## Windows MSI and open-source signing

Status: Active
Last updated: 2026-08-10

### User-visible outcome

Windows users install, update and remove AirWiki through a public-trust-signed
per-user MSI without purchasing a project-owned certificate. The package keeps
the existing local data, identity, model, Wiki and integration locations and
passes Smart App Control with every AirWiki-owned executable covered by the
same SignPath signing request.

### Minimum acceptance path

1. Build one deterministic unsigned MSI from a reviewed GitHub-hosted Windows
   job and submit it to SignPath with verified origin.
2. Verify the returned MSI and its desktop, MCP bridge and firewall helper,
   install it on the real Windows host with Smart App Control enforced, and
   complete local, LAN, public and MCP journeys against the matching macOS
   candidate.
3. Upgrade, uninstall while preserving data, opt into firewall/data cleanup,
   recover from cancellation and reject downgrade, unsafe paths, reparse points
   and invalid or incomplete signatures.

### Constraints

- Preserve `%LOCALAPPDATA%\Programs\AirWiki` for immutable installed files and
  the existing separate mutable data roots.
- Preserve current per-user behavior, updater consent, firewall-helper
  authority, MCPB layout, WebView2 bootstrap policy and Windows 10/11 x64 scope.
- Sign only AirWiki-owned binaries; verify rather than re-sign pinned upstream
  runtime files.
- Signing credentials and SignPath identifiers remain outside source control.
- The normal CI and unsigned pilot remain credential-free and offline except
  for already reviewed tool downloads.

### Deliberately deferred

- Microsoft Store/MSIX distribution, machine-wide installation and Windows on
  ARM.
- Public promotion before the existing release checklist is complete.

### Checkpoints

- [ ] Record the MSI/SignPath architecture and transition policy.
- [ ] Add a fixed per-user WiX template and deterministic package verification.
- [ ] Port upgrade, downgrade, autostart, firewall and opt-in cleanup behavior.
- [ ] Add deep-signing configuration and an origin-verified SignPath workflow.
- [ ] Pass unsigned MSI build and destructive smoke tests on Windows.
- [ ] Complete SignPath onboarding and verify the signed candidate under Smart
      App Control.
- [ ] Pass macOS-Windows local, LAN, public, MCP, update and recovery acceptance.
- [ ] Remove NSIS-only implementation and documentation after the MSI gates pass.

### Evidence and recovery

- Evidence: reviewed commit, package hash, signature publisher, OS/build,
  version, PASS/FAIL and bounded durations only.
- Recovery: keep NSIS as the internal package until MSI acceptance completes;
  never publish an unsigned MSI or promote its updater manifest.

### Decisions

- 2026-08-10: Azure Artifact Signing public trust cannot validate an individual
  resident in Uruguay. A commercial IV certificate is disproportionate for the
  current validation stage.
- 2026-08-10: SignPath cannot deeply sign an NSIS executable or its generated
  uninstaller. Move to MSI so Windows Installer owns removal and SignPath can
  sign the package and nested AirWiki PE files in one verified-origin request.
- 2026-08-10: The current NSIS candidate is internal and unsupported. Its
  transition may require one explicit uninstall that preserves the separate
  data roots; no public upgrade compatibility is claimed until MSI acceptance.

## Public infrastructure beta v1 closure

Status: Completed
Last updated: 2026-08-08

### User-visible outcome

Installed macOS arm64 and Windows x64 beta candidates discover and browse
public wikis across real NATs through either Azure index/relay node without
manual network or community-index configuration.

### Minimum acceptance path

1. Build and install both candidates from one commit and the same private v1
   bootstrap.
2. Search and browse synthetic public collections in both directions, including
   one relayed route and each single-node failover.
3. Prove revocation, expiry, downgrade protection, clean recovery and zero
   AirWiki Windows Public-profile rules.

### Constraints

- Keep monthly Azure cost at or below the approved USD 50 ceiling.
- Publish only sanitized evidence; never persist network or user identifiers,
  addresses, queries, paths or raw logs.
- Preserve LAN, MCP, public protocol and firewall boundaries.
- Stop acceptance on a reproducible defect; fix the root cause without a
  workaround and rebuild every candidate from the new commit.

### Deliberately deferred

- Public signing, notarization, updater promotion and incompatible-NAT direct
  route guarantees.

### Checkpoints

- [x] Deploy and harden two independent nodes with budgets and alerts.
- [x] Validate repository gates and install the macOS v1 candidate.
- [x] Classify and correct the Windows pre-activation installation failure.
- [x] Complete the rebuilt Windows installation, model, enrichment and MCP
      smoke without a Public-profile firewall rule.
- [x] Correct the reproduced cross-NAT owner-stage and route-evidence defects.
- [x] Correct the reproduced relay-readiness announcement and concurrent local
      publisher-block defects with deterministic regressions.
- [x] Keep private services available when an older candidate encounters a
      persisted higher bootstrap registry without relaxing downgrade defense.
- [x] Rebuild and pass installed smoke on both final v1 candidates.
- [x] Pass bidirectional cross-NAT, relay and both failover recoveries.
- [x] Pass sequential isolated v1, revoked v2, expired v3 and clean v1 recovery.
- [x] Reconfirm cost, observability, firewall, CI and DCO; mark PR ready without
      merging.

### Evidence and recovery

- Evidence: package/bootstrap hashes, OS/build versions, PASS/FAIL, durations,
  route class, accepted-index count, sanitized service state, restarts, usage
  and cost only.
- Recovery: keep the PR in draft, preserve the last known-good private v1
  bootstrap, revoke a faulty node in a higher registry, and retire both exact
  Azure groups if the budget or trust boundary fails.

### Decisions

- 2026-07-28: The installed Windows smoke reproduced an asset-installation
  failure before activation, but the previous status contract reduced it to a
  readiness timeout. Extend the closed durable contract at the inference
  boundary before another installed run; the resulting stage must identify the
  underlying correction.
- 2026-07-29: The corrected installed status then reproduced
  `generation_timeout` twice on eligible minimum-class Windows hardware during
  the full enrichment used as an activation check. Replace only that check with
  a bounded strict-JSON model/runtime probe; keep production enrichment
  unchanged and require two real synthetic enrichments, human review,
  publication and MCP retrieval from the rebuilt installed candidate.
- 2026-07-29: Bidirectional installed cross-NAT search reached valid catalog
  candidates while both publishers had two ready relay reservations, but the
  owner stage failed before connection. The 800 ms response budget was also
  clipping cold relay setup, and global route state could misattribute another
  connection or concurrent request. Keep the publisher response budget
  unchanged, add a separate bounded cold-connection budget, and return route
  evidence only with the matching request's valid owner response. Independent
  review of the correction found no remaining P0-P2; installed cross-NAT
  measurement remains the acceptance authority.
- 2026-07-29: A single-node outage exposed manifests that still listed a relay
  before its outbound reservation was ready, and a deterministic concurrency
  test proved that an in-flight public response could outlive a completed local
  publisher block. Publish only routes backed by live reservations, reannounce
  or tombstone immediately when readiness changes, and hold the local block
  barrier through response acceptance and delivery. Serialize sequence
  allocation, reject late local status completions by sequence, and make
  renewal cancellation interrupt pending catalog updates. Both defects
  invalidate the previous candidate commit and require the complete build and
  installed acceptance matrix to restart from the corrected commit.
- 2026-07-29: Installing v1 over the isolated v2 lifecycle state preserved the
  registry high-water but aborted all private services when the strict storage
  downgrade rejection reached the composition root. Treat only a strictly
  older bundled registry as an expected no-op before mutation, retain the
  transactional downgrade and same-version defenses, and require installed
  v2-to-v1 and expired-v3-to-v1 proof that local, LAN and MCP remain available.
  This defect invalidates the current candidates and restarts the complete
  build and installed acceptance matrix from the next corrected commit.

## Template

```markdown
# <Outcome-oriented title>

Status: Draft | Active | Blocked | Completed | Superseded
Last updated: <YYYY-MM-DD>

## User-visible outcome

<What a user or contributor can do when this is complete.>

## Minimum acceptance path

1. <Shortest representative action.>
2. <Observable result.>
3. <Failure or recovery behavior that must also hold.>

## Constraints

- <Privacy, compatibility, architecture, or platform boundary.>

## Deliberately deferred

- <Related work that is not required for this outcome.>

## Checkpoints

- [ ] <Small verifiable checkpoint.>

## Evidence and recovery

- Evidence: <Smallest sanitized proof that the outcome and recovery path passed.>
- Recovery: <How to return to a safe state if the implementation or rollout fails.>

## Decisions

- <Date or checkpoint>: <Decision and reason; omit implementation diaries.>

## Outcome and retrospective

<Complete when closing the plan: actual outcome, decisive evidence, and any remaining follow-up.>
```
