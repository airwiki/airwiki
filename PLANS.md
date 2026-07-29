# Persistent implementation plans

Most changes do not need a checked-in plan. Use a persistent plan only when work spans multiple checkpoints, requires durable coordination, or contains decisions that would otherwise be lost between sessions. Keep only one active persistent plan at a time and keep it short enough to review as a whole.

A plan records intent and acceptance, not a transcript, command log, or speculative design. Update it when evidence changes the approach. Finish it as `Completed` or `Superseded`; move durable architectural decisions into an ADR and user-visible changes into `CHANGELOG.md`.

# AirWiki Screens v2 desktop redesign

Status: Active
Last updated: 2026-07-29

## User-visible outcome

AirWiki follows the supplied broadsheet screen reference across its primary
desktop journeys while every action still reflects real local, LAN, MCP and
public-federation state.

## Minimum acceptance path

1. Launch an installed candidate with synthetic state at wide and compact sizes.
2. Navigate Today, Review, Wiki, Ask, Public, Connections and Settings with
   visible keyboard focus and no clipping or stale selection.
3. Verify loading, empty and sanitized error recovery without manual public-index
   configuration or regressions to private services.

## Constraints

- Keep protocols, persistence, worker ownership and asynchronous service
  boundaries unchanged.
- Reuse repository assets and dependencies; do not add remote UI resources.
- Preserve opt-in publication, publisher blocking, LAN policy and MCP
  authorization.
- Use only synthetic content in captures and evidence.

## Deliberately deferred

- Public signing, notarization and updater distribution.

## Checkpoints

- [x] Verify and use the supplied standalone HTML as the visual authority.
- [x] Apply the broadsheet system to the complete primary desktop navigation.
- [x] Remove normal-interface community-index configuration without weakening
      publisher blocking.
- [x] Pass focused formatting, Clippy and Desktop tests on the final diff.
- [x] Resolve independent-review findings.
- [x] Pass installed macOS wide, compact, focus, loading, empty and error QA.
- [ ] Pass installed Windows wide, compact, focus, loading, empty and error QA.
- [ ] Record sanitized evidence and publish the focused review branch.

## Evidence and recovery

- Evidence: focused automatic gates plus synthetic installed screenshots and a
  state-by-state PASS/FAIL matrix with no content, identities or network details.
- Recovery: revert the focused redesign commit; storage, protocols and user data
  require no migration.

## Decisions

- 2026-07-29: Treat the supplied HTML as presentation authority while repository
  behavior and privacy contracts remain authoritative for product semantics.
- 2026-07-29: Show chat-client status and actions inline in Ask by reusing the
  existing integrations view-model; retain the detailed subroute for recovery.
- 2026-07-29: Keep automatic beta bootstrap discovery as the only normal path.
  A count-only advanced-recovery card may disable inherited community indexes
  without exposing or allowing users to add network identities.

## Outcome and retrospective

The final macOS candidate passed the synthetic wide/compact visual matrix,
keyboard-focus inspection and empty/loading/attention states. Focused formatting,
Clippy, 330 Desktop tests, documentation and license checks passed. Independent
review found no remaining P0-P2 issues after fixes for bounded integration
refresh, serialized public-topology recovery and two keyboard-accessibility
gaps. Windows installed visual QA and publication of the focused review branch
remain pending.

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
