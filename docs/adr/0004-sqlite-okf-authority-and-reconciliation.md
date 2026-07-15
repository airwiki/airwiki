# ADR 0004: Separate SQLite operational authority from visible OKF authority

- Status: Accepted
- Date: 2026-07-15

## Context

AirWiki needs transactional state for ingestion, review, search, trust and
recovery, while its published wiki must remain an inspectable OKF bundle. Making
either representation universally authoritative would lose important behavior:
SQLite alone would hide the portable wiki, while OKF alone would be a poor job,
index and authorization store.

Publication and filesystem failure can leave the two representations temporarily
out of step. Silently choosing the newest or most convenient copy could expose
an unreviewed revision, discard human-visible history or overwrite an original.

## Decision

SQLite is authoritative for operational state: collections, source paths and
hashes, revisions, jobs, review and publication state, search indexes, trust,
grants and audit events. Local source paths never appear in OKF.

Managed OKF files are authoritative for the visible published wiki: concept
pages, `index.md` and append-only `log.md`. Original source documents remain
user-owned inputs and are never modified or replicated by reconciliation.

Human approval crosses the two authorities through a durable publication claim:

1. SQLite verifies that the reviewed source revision is still current and
   withdraws it from searchable exposure while publication is pending.
2. The publisher writes and validates the concept page, regenerated index and
   log entry with atomic file replacement where applicable.
3. SQLite marks the same revision published only after the OKF materialization
   remains current.

Startup recovery completes a still-current claim or cancels it and removes its
derived artifacts. A source modification, deletion or unavailable collection
withdraws SQLite/FTS exposure before removing the corresponding OKF artifact.
Search therefore fails closed even if filesystem cleanup is incomplete.

The OKF inspector is read-only. It compares stable concept identity, revision,
source hash and metadata against SQLite and reports disagreement as health or a
transient updating state. It never resolves a conflict by choosing one side.

Automation may regenerate only unambiguous derived artifacts from a coherent
published snapshot, such as `index.md` or local indexes. Concept content,
publication status, `log.md`, permissions and ambiguous corruption require a
guided repair with a verified snapshot and explicit human confirmation. Affected
content remains withdrawn until the result validates coherently.

## Consequences

- The application can recover interrupted publication without treating partial
  files as published knowledge.
- A bundle and SQLite may visibly disagree during recovery; this is reported
  rather than hidden.
- Direct edits to managed OKF do not silently rewrite operational state.
- Loss of one authority requires explicit recovery from validated remaining
  evidence, not last-write-wins synchronization.
- Publication and repair need atomic writes, durable claims and focused failure
  tests.

## Rejected alternatives

- **SQLite-only wiki:** removes the portable, human-readable OKF representation.
- **OKF-only operational store:** cannot safely own jobs, indexes, local paths,
  trust or transactional publication.
- **Automatic bidirectional synchronization:** makes authority ambiguous and can
  legitimize unreviewed edits.
- **Newest timestamp wins:** timestamps do not prove review, integrity or
  authorization.
- **Silent repair of history or concepts:** can invent intent and weaken the
  human-publication boundary.
