# ADR 0007: Separate answerability-accepted evidence from authorized external-chat candidates

- Status: Accepted
- Date: 2026-07-20
- Supersedes: [ADR 0001](0001-answerability-gated-search-v2.md) and the
  answerability-only disclosure clause in
  [ADR 0005](0005-lan-identity-pairing-and-authorization.md)

## Context

Hybrid retrieval can find an authorized passage that answers a question while
the lightweight source-node answerability classifier rejects it. Evaluation
showed that hiding every rejected passage creates false negatives, while
removing the classifier or exposing an untyped candidate pool creates too many
unrelated and forbidden results.

The external chat model is usually more capable than AirWiki's bounded local
classifier. It can evaluate additional authorized context, but it must not
mistake authorization for relevance. AirWiki remains responsible for
publication, collection policy, peer grants, revocation, minimization and
provenance.

## Decision

For searches with purpose `external_ai`, each source node returns two typed,
independently bounded lanes:

- `hits` are passages accepted by the local answerability classifier and become
  MCP `evidence`;
- `authorized_candidates` passed the same publication and disclosure checks but
  were rejected by that classifier.

Local-assistant searches return only `hits`. They do not compute a disclosure
lane that their current consumer does not use.

Both lanes are revalidated immediately before disclosure against the current
published revision, collection policy, peer grant and revocation state. A
candidate never bypasses `allow_external_ai`. Evidence wins when the same
content-stable chunk appears in both lanes. Each lane is limited by `top_k`; if
the global LAN or MCP response budget is reached, candidates are removed before
evidence. Cross-lane deduplication happens before candidate truncation so a
duplicate cannot displace a unique candidate.

The MCP contract labels candidates separately and instructs the consuming model
to use one only when its snippet explicitly supports a requested fact.
Candidates retain the same bounded snippet and five provenance fields as
evidence. AirWiki does not expose embeddings, internal scores, local paths,
collection listings or model reasoning.

The additive CBOR field uses a default when absent, so
`/airwiki/search/2.0.0` remains wire-compatible in both directions. A new
protocol version is not required for this additive, optional capability.

## Consequences

- A capable chat model can recover useful authorized passages that the local
  classifier rejected.
- External-chat output may contain unrelated authorized snippets, clearly
  separated from evidence. Users must minimize collections enabled for external
  AI, and consumer-model behavior remains part of manual acceptance.
- The desktop search UI remains conservative and unchanged.
- AirWiki adds no second generative model, reranker or answering path.
- Deterministic tests cover authorization, revalidation, deduplication, bounds,
  MCP transport and old/new CBOR compatibility. A synthetic client evaluation
  is still required before promotion to verify that a real consumer uses
  candidates only with explicit support.

## Rejected alternatives

- **Expose the unfiltered candidate pool:** evaluation found unacceptable false
  and forbidden evidence.
- **Remove local answerability:** loses the high-confidence evidence lane and
  makes every consumer repeat basic filtering.
- **Rescue candidates by rank:** accepted false facts and missing true facts
  overlap at the same ranks.
- **Run another LLM inside AirWiki:** adds latency, memory and a second answering
  responsibility without a demonstrated need.
- **Create search protocol v3:** unnecessary for an additive field that older
  decoders safely ignore and newer decoders default when absent.
