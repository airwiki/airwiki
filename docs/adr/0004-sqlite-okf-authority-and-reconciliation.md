# ADR 0004: Separate SQLite operational authority from visible OKF authority

- Status: Accepted
- Date: 2026-07-15
- Extended by: ADR 0010 (imported and AI-memory managed bundles)

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

Managed OKF files are authoritative for the locally visible wiki: unverified
`draft` concept pages, human-approved `stable` concept pages, `index.md` and
append-only `log.md`. The index and log describe the stable publication; draft
pages are visible only in the local workspace. Original source documents remain
user-owned inputs and are never modified or replicated by reconciliation.

Ingestion crosses the authority boundary before human review only to
materialize the current proposal as an OKF `draft`. SQLite keeps the exact
source revision, evidence and the operational distinction between pending and
excluded; OKF uses `status: draft` for both. This transition never adds the
concept to search, the root index, LAN, public federation or MCP.

Human approval promotes one current draft through a durable publication claim:

1. SQLite verifies that the reviewed source revision is still current and
   withdraws it from searchable exposure while publication is pending.
2. The publisher writes and validates the concept page, regenerated index and
   log entry with atomic file replacement where applicable.
3. SQLite marks the same revision published only after the OKF materialization
   remains current.

Startup recovery completes a still-current claim or cancels it, removes any
partial stable artifact and restores a coherent local draft when the source is
still current. A source modification, deletion or unavailable collection
withdraws SQLite/FTS exposure before replacing or removing the corresponding
OKF artifact. Search therefore fails closed even if filesystem cleanup is
incomplete.

The OKF inspector is read-only. Its local scope compares managed draft and
stable concept identity, revision, source hash, lifecycle and metadata against
SQLite. Its disclosure scope projects only coherent stable concepts and never
reads or returns draft pages. Both report disagreement as health or a transient
updating state and never resolve a conflict by choosing one side.

Automation may regenerate only unambiguous derived artifacts from a coherent
published snapshot, such as `index.md` or local indexes. Concept content,
publication status, `log.md`, permissions and ambiguous corruption require a
guided repair with a verified snapshot and explicit human confirmation. Affected
content remains withdrawn until the result validates coherently.

## Consequences

- The application can recover interrupted publication without treating partial
  files as published knowledge.
- People can browse generated resources before review without making them
  searchable or shareable.
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
