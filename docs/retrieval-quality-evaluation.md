# Retrieval-quality evaluation profile

This document records the mutable source-side retrieval profile and its measured
results. It complements the narrower [relevance-gate evaluation](relevance-gate-evaluation.md):
that profile classifies a reviewed candidate set, while this profile starts with
published synthetic knowledge and exercises the search pipeline that produces
those candidates.

The corpus is informed by recurring LLM Wiki and OKF design concerns such as
cross-document answers, contradictions, ambiguous names, near duplicates and
honest no-answer behavior. The
[LLM Wiki discussion](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f)
is useful design input, not a normative AirWiki or
[OKF](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
contract.

## Scope

The active schema-v2 fixture at
`fixtures/retrieval/search-quality-v2.json` has SHA-256
`2b83ffb0939b4e91a9fdb799d92a4b6ed4e4f775298694c5b9abe3761a2f52f6`.
Its 17 regression, calibration and holdout cases cover direct and paraphrased
retrieval, cross-language and compound questions, absent and withdrawn facts,
contradictions, common-name ambiguity, prompt injection, near duplicates,
authorization, external-chat policy and stable ordering.

V2 preserves the five observed failures from the initial run as regression
cases. Every case and document belongs to a domain, and the validator rejects
overlap between regression, calibration and holdout domains, cross-domain gold
evidence, answerable cases with merely related evidence, no-answer facts that
are not forbidden, and a corpus without a peer-without-grant case. The split
remains serialized as `holdout`, but it has already been observed and now serves
only as diagnostic transfer evidence. A future promotion decision requires
fresh domains rather than relabeling this fixture as an unobserved holdout.

The original schema-v1 fixture remains unchanged at
`fixtures/retrieval/search-quality-v1.json` with SHA-256
`accd40d8473ad499469c0fd105eec9f34d70f660c9bdada1254d2325f609e727`.
It preserves the initial measurement and is not the active acceptance corpus.

The evaluator builds temporary origin and peer databases and uses the production
publication and search interfaces. It covers:

- reviewed publication and withdrawal boundaries;
- SQLite FTS5 and vector candidate retrieval;
- reciprocal-rank fusion and evidence relevance;
- deduplication and deterministic ordering;
- local, peer-authorized, external-AI and federated search scopes;
- collection policy, grants and result-provenance revalidation.

Report schema 3 also attributes each missing expected group to its source
candidate pool, binary relevance mask, source top-k truncation or final
revalidation. The audit wraps the existing relevance provider and never changes
its decisions. It is an evaluation boundary only; no logits, questions,
passages or audit state enter the product search API or persistent storage. The
evaluator also fails if the audited source set is incomplete or repeated
decisions are unstable.

For a multi-node case, Recall@5 means that required evidence must appear within
the top five returned by its source node. The evaluator combines that source
coverage for scoring; it does not claim that up to ten source hits are the
gateway's final top five. The production coordinator's second RRF, cross-node
deduplication and partial-result behavior remain covered by focused
`airwiki-network` tests.

The evaluator does not cover file parsing, generative enrichment, network
transport, pairing UX, chat-client synthesis or installed-platform behavior.
Those boundaries retain their focused tests and manual acceptance paths.

## Deterministic CI validation

Ordinary CI uses fixture embedding and relevance providers. It downloads no
models, contacts no peer and creates only disposable state under `target/`:

```bash
cargo run --locked -p xtask -- retrieval validate
```

This command proves the pipeline and corpus contract. It is deliberately not a
claim about the quality of the shipped embedding and relevance artifacts.

## Real-model evaluation

A maintainer can evaluate the current profile with already verified local
snapshots of multilingual E5 and mMARCO:

```bash
cargo run --locked -p xtask -- retrieval evaluate \
  --embedding-snapshot <verified-e5-snapshot-directory> \
  --relevance-snapshot <verified-mmarco-snapshot-directory>
```

No evaluation command downloads models. A run is platform-specific and writes
`target/evals/retrieval-pipeline-<os>-<arch>.json` whether it passes or fails.
The command exits unsuccessfully when the measured profile misses an acceptance
threshold.

## Metrics and acceptance

Each regression, calibration and holdout split must independently satisfy:

- Recall@5 of at least 0.90 across expected evidence groups;
- zero unexpected evidence facts;
- zero forbidden evidence facts;
- zero provenance errors;
- zero duplicate violations; and
- stable repeated results, stable top-5 prefixes and stable results after
  reversing insertion order.

Every regression case must also pass individually, so a strong aggregate cannot
hide the return of a known failure.

MRR@5 uses the first returned member of an expected evidence group. Every
answerable case is included in the denominator, and a miss contributes zero.
MRR@5 and elapsed time are diagnostics rather than acceptance thresholds.

## Report privacy

The JSON report contains fixture and artifact identity, target platform, thread
count, aggregate and stage-attribution metrics, synthetic case and fact
identifiers, stability flags, elapsed times and PASS/FAIL. It contains no
question or passage text, snippets, source-document paths, source-document
hashes, local usernames, peer identities, IP addresses, ports or multiaddresses.
Reports remain ignored under `target/`; maintainer evidence should retain only
the aggregate fields allowed by
[the validation-record policy](maintainer-validation.md).

## Initial platform observation

The first macOS arm64 observation on 2026-07-16 used the pinned E5 revision
`614241f622f53c4eeff9890bdc4f31cfecc418b3` and mMARCO revision
`1427fd652930e4ba29e8149678df786c240d8825`. It produced:

- calibration Recall@5: 0.75;
- holdout Recall@5: 0.625;
- overall Recall@5: 0.6667;
- calibration MRR@5: 0.625, holdout MRR@5: 0.80 and overall MRR@5: 0.7222;
- three false-evidence cases, each with one unexpected fact; and
- zero forbidden-evidence, provenance, duplicate or stability violations.

The failing case identifiers were `calibration_paraphrase_recovery`,
`holdout_date_cross_language`, `holdout_compound_federated`,
`holdout_external_ai_policy` and `holdout_unrelated_injection`. These identifiers
are synthetic diagnostics and contain no question or document content.

The v1 real-model observation therefore **failed** this retrieval-quality gate.
The result established an honest baseline: the authorization, provenance,
deduplication and stability boundaries held, while retrieval completeness and
false-evidence control need focused improvement. This goal does not tune the
fixture, add query decomposition, introduce another model or change product
protocols merely to turn that observation green.

Schema v2 deterministically validates the corrected corpus and pipeline
contract. It has not established a passing semantic-quality result for the real
model artifacts.

## V2 stage-attribution observation

The macOS arm64 run on 2026-07-18 used the same pinned model revisions and the
active schema-v2 corpus. Its final quality result still failed:

- regression Recall@5: 0.50 with three unexpected facts;
- calibration Recall@5: 1.00;
- diagnostic holdout Recall@5: 0.625; and
- overall Recall@5: 0.6667 with no provenance, duplicate or stability errors.

The stage attribution isolated the failure:

- source-candidate Recall@10: 1.00 (18 of 18 expected groups; each node keeps
  its own pool of up to ten candidates);
- mask-surviving recall: 0.6667 (12 of 18 groups);
- six groups rejected by the binary mMARCO mask;
- zero expected groups absent from retrieval, outside the source top five or
  removed by final revalidation; and
- three non-answering fragments accepted by the selector.

Three hard-negative facts were present in candidate pools. This is expected:
candidate retrieval is an internal ranking stage, not evidence disclosure. The
quality failure is that the selector accepted three non-answering candidates
while rejecting six answer-bearing candidates. This result supports work on one
better selector contract; it does not justify changing BM25, E5, RRF, the pool
size, top-k, policies or the wire protocol.

Windows real-model evaluation is pending. A macOS result must never be used to
infer the behavior of the Windows artifacts.

Rejected or inconclusive candidate mechanisms are summarized in the
[retrieval research ledger](retrieval-research-ledger.md). Their implementation
remains in the linked pull requests for reproducibility but is not maintained in
`main` without an active product consumer.
