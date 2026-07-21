# Architecture decision records

Architecture decision records (ADRs) explain decisions that are expensive to
reverse. They do not replace current architecture documentation, implementation
tests, operational runbooks or evaluation reports.

## Index

| Number | Decision | Status | Date | Relationship |
| --- | --- | --- | --- | --- |
| [0001](0001-answerability-gated-search-v2.md) | Gate federated evidence by local answerability | Superseded | 2026-07-12 | Superseded by ADR 0007 |
| [0002](0002-local-chat-integrations.md) | Connect local chat clients through one MCP stdio bridge | Accepted | 2026-07-12 | Lifecycle and per-user autostart superseded by ADR 0003 |
| [0003](0003-desktop-lifecycle-and-signed-updates.md) | Keep desktop services available and require signed updates | Accepted | 2026-07-12 | Supersedes ADR 0002 only for lifecycle and per-user autostart |
| [0004](0004-sqlite-okf-authority-and-reconciliation.md) | Separate SQLite operational authority from visible OKF authority | Accepted | 2026-07-15 | — |
| [0005](0005-lan-identity-pairing-and-authorization.md) | Bind LAN authorization to persistent identity and human-confirmed SAS | Accepted | 2026-07-15 | Answerability-only disclosure clause superseded by ADR 0007 |
| [0006](0006-windows-firewall-privilege-boundary.md) | Isolate Windows firewall changes in a narrow elevated helper | Accepted | 2026-07-16 | — |
| [0007](0007-separate-evidence-from-authorized-candidates.md) | Separate answerability-accepted evidence from authorized external-chat candidates | Accepted | 2026-07-20 | Supersedes ADR 0001 and the answerability-only disclosure clause in ADR 0005 |

## Policy

Use one of these statuses:

- `Proposed`: under review and not yet authoritative;
- `Accepted`: the current durable decision;
- `Superseded`: replaced by a later ADR;
- `Rejected`: considered and deliberately not adopted.

Every ADR uses the heading `# ADR NNNN: Title`, followed by `Status` and `Date`
metadata, then Context, Decision, Consequences and Rejected alternatives.
Superseding relationships are recorded in both affected ADRs and in this index.

After an ADR is accepted, change only spelling, broken links or supersession
metadata. A material decision change requires a new ADR. Mutable implementation
values, benchmark results, checklists and incident notes belong in validation
reports, runbooks or release notes instead.
