# Persistent implementation plans

Most changes do not need a checked-in plan. Use a persistent plan only when work spans multiple checkpoints, requires durable coordination, or contains decisions that would otherwise be lost between sessions. Keep only one active persistent plan at a time and keep it short enough to review as a whole.

A plan records intent and acceptance, not a transcript, command log, or speculative design. Update it when evidence changes the approach. Finish it as `Completed` or `Superseded`; move durable architectural decisions into an ADR and user-visible changes into `CHANGELOG.md`.

## Public infrastructure beta v1 closure

Status: Active
Last updated: 2026-07-28

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
- [ ] Classify and correct the Windows pre-activation installation failure.
- [ ] Rebuild and pass installed smoke on both final v1 candidates.
- [ ] Pass bidirectional cross-NAT, relay and both failover recoveries.
- [ ] Pass sequential isolated v1, revoked v2, expired v3 and clean v1 recovery.
- [ ] Reconfirm cost, observability, firewall, CI and DCO; mark PR ready without
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
