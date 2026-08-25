# Architecture decision records

Architecture decision records (ADRs) explain decisions that are expensive to
reverse. They do not replace current architecture documentation, implementation
tests, operational runbooks or evaluation reports.

## Index

| Number | Decision | Status | Date | Relationship |
| --- | --- | --- | --- | --- |
| [0001](0001-answerability-gated-search-v2.md) | Gate federated evidence by local answerability | Superseded | 2026-07-12 | Superseded by ADR 0007 |
| [0002](0002-local-chat-integrations.md) | Connect local chat clients through one MCP stdio bridge | Accepted | 2026-08-14 | Lifecycle and per-user autostart superseded by ADR 0003 |
| [0003](0003-desktop-lifecycle-and-signed-updates.md) | Keep desktop services available and require signed updates | Accepted | 2026-07-12 | Supersedes ADR 0002 only for lifecycle and per-user autostart |
| [0004](0004-sqlite-okf-authority-and-reconciliation.md) | Separate SQLite operational authority from visible OKF authority | Accepted | 2026-07-15 | — |
| [0005](0005-lan-identity-pairing-and-authorization.md) | Bind LAN authorization to persistent identity and human-confirmed SAS | Accepted | 2026-07-15 | Answerability-only disclosure clause superseded by ADR 0007 |
| [0006](0006-windows-firewall-privilege-boundary.md) | Isolate Windows firewall changes in a narrow elevated helper | Accepted | 2026-07-16 | — |
| [0007](0007-separate-evidence-from-authorized-candidates.md) | Separate answerability-accepted evidence from authorized external-chat candidates | Accepted | 2026-07-20 | Supersedes ADR 0001 and the answerability-only disclosure clause in ADR 0005 |
| [0008](0008-public-federation.md) | Separate opt-in public federation from LAN authorization | Accepted | 2026-07-21 | Keeps ADR 0005 unchanged |
| [0009](0009-windows-msi-signpath.md) | Use per-user MSI and origin-verified open-source signing on Windows | Accepted | 2026-08-10 | Refines ADR 0003 for the Windows package and signing provider |
| [0010](0010-okf-v02-managed-memory-and-attested-computation.md) | Adopt OKF v0.2 for managed memory and attested computation | Accepted | 2026-08-13 | Supersedes ADR 0002 read-only MCP and extends ADR 0004 beyond folder publication |
| [0011](0011-portable-project-memory.md) | Store portable project memory in `.airwiki` | Accepted | 2026-08-23 | Supersedes ADR 0002 project selection and ADR 0010 private-only application memory |
| [0012](0012-public-technical-prereleases.md) | Publish unsigned technical candidates as non-latest pre-releases | Accepted | 2026-08-25 | Refines ADR 0003 prerelease distribution and ADR 0009 unsigned Windows candidates |

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
