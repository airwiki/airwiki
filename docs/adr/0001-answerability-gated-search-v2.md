# ADR 0001: Gate federated evidence by local answerability

- Status: Superseded
- Date: 2026-07-12
- Superseded by: [ADR 0007](0007-separate-evidence-from-authorized-candidates.md)

## Context

Hybrid BM25 and embedding retrieval ranks the closest available passages but
does not establish that any passage answers the question. A corpus can therefore
return an unrelated title or snippet for an absent fact simply because it is the
nearest candidate.

An absolute embedding threshold cannot safely solve this problem: measured
positive and negative distributions overlap, and lexical-overlap requirements
reject multilingual questions and paraphrases. This is a privacy boundary as
well as a retrieval-quality issue because irrelevant evidence must not cross LAN
or MCP.

## Decision

Each source node applies a local answerability classifier after BM25/embedding
candidate generation and RRF, but before constructing any `SearchHit`. The
classifier is non-generative, pinned by immutable revision and asset hashes, and
calibrated with reviewed synthetic positive and hard-negative query/passage
pairs. It returns only `relevant` or `irrelevant`; scores and model identifiers
never cross the LAN.

Classification fails closed. Missing assets, timeouts, invalid output, unknown
profiles or runtime failures make that search path unavailable. They never turn
unclassified candidates into evidence or a trustworthy empty result.

The LAN search protocol is `/airwiki/search/2.0.0`, with no v1 fallback.
Version 2 guarantees that every returned hit passed the source node's
answerability gate. A v1 peer may remain discovered and paired, but contributes
no hits and is reported as partial coverage.

MCP represents relevant evidence, scoped absence and coverage failures as
different result variants. The adapter does not infer contradictions or
synthesize an answer.

The active model profile, score policy, evaluation corpus and platform results
are operational evidence rather than architecture decisions. They are recorded
in the [relevance-gate evaluation profile](../relevance-gate-evaluation.md).
Changing the model or policy requires a new immutable profile and a reviewed
calibration and holdout report; changing this source-node guarantee requires a
new ADR.

## Consequences

- Search requires an additional bounded local inference step.
- Compound questions may need focused follow-up searches.
- Older peers must upgrade before contributing evidence.
- False negatives are preferred to disclosure of unrelated evidence.
- Ordinary CI validates deterministic providers and fixtures without model
  downloads; real-model evaluation is an explicit maintainer action.

## Rejected alternatives

- **Single cosine threshold:** positive and negative score distributions
  overlap.
- **Lexical overlap plus cosine:** rejects multilingual retrieval and
  paraphrases.
- **Generative LLM judge:** adds latency, nondeterminism and an unnecessary
  answering path.
- **Gateway-only filtering:** cannot enforce the guarantee at the private source
  node or safely accept results from older peers.
- **Automatic contradiction detection:** requires reviewed claims or another
  semantic interpretation layer that is outside the current need.
