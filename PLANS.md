# Persistent implementation plans

Most changes do not need a checked-in plan. Use a persistent plan only when work spans multiple checkpoints, requires durable coordination, or contains decisions that would otherwise be lost between sessions. Keep only one active persistent plan at a time and keep it short enough to review as a whole.

A plan records intent and acceptance, not a transcript, command log, or speculative design. Update it when evidence changes the approach. Finish it as `Completed` or `Superseded`; move durable architectural decisions into an ADR and user-visible changes into `CHANGELOG.md`.

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
