# Retrieval research ledger

This ledger keeps durable conclusions from bounded search experiments without
making rejected evaluators, fixtures or candidate mechanisms a maintenance
dependency. Linked pull requests preserve the reviewed context available for
each experiment.
Green CI means that an experiment ran as designed; it does not mean that its
candidate belongs in the product.

A rejected or inconclusive probe leaves only its conclusion in this ledger;
its adapters, assets and one-off fixtures are removed rather than maintained.
An accepted mechanism remains in `main` only while an active product or
evaluation path consumes it and focused tests protect that behavior.

| Candidate | Evidence | Decision | Durable conclusion |
| --- | --- | --- | --- |
| Domain-separated retrieval baseline v2 | [#8](https://github.com/airwiki/airwiki/pull/8) | **Accepted baseline** | Keep the 17-case regression corpus and its structural validator. Do not keep the selectors or model-specific runners bundled into the research branch. |
| Local QA-entailment selector | [#8](https://github.com/airwiki/airwiki/pull/8) | **Rejected** | The candidate missed its predeclared quality gate. Generic answerability machinery is not justified in `main`. |
| Off-the-shelf QA/NLI thresholds | [#8](https://github.com/airwiki/airwiki/pull/8) | **Rejected** | Multilingual SQuAD2 readers, MiniLM and mDeBERTa NLI, and QNLI did not separate answer-bearing passages from negatives with a threshold. Do not repeat those model probes without a materially different contract. |
| Gemma 4 exact-quote selector | [#8](https://github.com/airwiki/airwiki/pull/8) | **Rejected** | Both development policies reached Recall@5 of 1.00 but retained two or three false-evidence facts. Per-call p95 was 14.170 or 19.068 seconds, and the second policy used about 4.8 GiB RSS. Query-time generation failed the zero-false-evidence and interactive-resource gates; its runner and fixtures remain retired. |
| Reviewed evidence anchors | [#9](https://github.com/airwiki/airwiki/pull/9) | **Rejected** | Coverage reached 9/9 positive needs, but precision was 10/13 and only 5/11 decisions were correct. Reviewed links can supply candidates, but cannot serve as an answerability decision by themselves. |
| Reviewed claim identity in the existing mMARCO passage | Preregistered local probe, 2026-07-18 | **Rejected** | Prepending reviewed subject, relation, scope, temporal and polarity fields improved expected-group coverage from 12/18 to 15/18, but unexpected evidence rose from 3 to 4 and one forbidden fact survived. Structured identity is useful candidate context, not an answerability decision; do not retain the adapter or add these fields to the product for this mechanism. |
| Compact OKF graph on development rankings | [#10](https://github.com/airwiki/airwiki/pull/10), [#11](https://github.com/airwiki/airwiki/pull/11) | **Superseded** | A positive synthetic signal required a real-ranking holdout; it was not sufficient evidence for graph infrastructure. |
| Compact OKF graph on sealed holdout | [#12](https://github.com/airwiki/airwiki/pull/12) | **Rejected** | Baseline, graph and structural sham all produced Recall@5 of 0.75; graph assembly p95 was 123 ms against a 25 ms budget. |
| GTE multilingual reranker | [Exploratory observation in #8](https://github.com/airwiki/airwiki/pull/8) | **Rejected** | All ten expected development groups were already inside the source top five, so coverage did not improve. Mean MRR@10 rose from 0.833 to 0.889 and nDCG@10 from 0.877 to 0.925, but the known paraphrase reciprocal rank regressed from 1.0 to 0.5 and the compound case from 1.0 to 0.75. Do not retain its runner, model or fixture. |
| mMARCO score ordering | [#13](https://github.com/airwiki/airwiki/pull/13) | **Rejected** | Score order was identical to the existing filter order. The observed bottleneck was the binary relevance mask, not ordering among accepted candidates. |
| mMARCO abstention calibration | [#14](https://github.com/airwiki/airwiki/pull/14) | **Rejected** | Cutoff support recall fell from 0.75 to 0.4167, four queries lost support and a hard negative remained. |
| Standalone OKF path signal | [#15](https://github.com/airwiki/airwiki/pull/15) | **Rejected** | The signal connected 17 of 24 hard negatives, so a path alone is not evidence of answerability. |
| Graph-conditioned bounded diffusion | [#16](https://github.com/airwiki/airwiki/pull/16) | **Inconclusive** | Baseline, real graph and degree-preserving sham all found 26/28 evidence groups, and the corpus exposed zero cutoff opportunities. Do not tune or promote from that fixture. |
| Retrieval-stage attribution | [Active evaluator](retrieval-quality-evaluation.md#v2-stage-attribution-observation) | **Accepted diagnostic** | Source-candidate Recall@10 was 1.00 for all 18 expected groups. mMARCO rejected all six missing groups and accepted three non-answering fragments; no expected group was lost in candidate generation, top-k truncation or revalidation. Keep stage attribution in the evaluator, not the product path. |
| Hybrid top-5 without relevance mask | Exact projection from the audited candidate batches, 2026-07-18 | **Rejected** | Returning each source's first five hybrid candidates recovered all 18 expected groups but also emitted 102 false and 3 forbidden facts. Candidate coverage is sufficient; exposing the pool without an answerability selector is not safe. Do not retain a passthrough path. |
| Rank-only rejected-candidate rescue | Exact candidate-rank audit, 2026-07-18 | **Rejected** | The six missing expected facts occupied source ranks 1, 1, 1, 2, 4 and 4, while the three accepted false facts occupied ranks 2, 3 and 4. No fixed rank cutoff separates them, so rank cannot replace answerability. Do not add a top-N rescue rule. |
| Explicit compound-question splitting | Static ceiling audit of the active schema-v2 observation, 2026-07-18 | **Rejected before implementation** | Only two v2 cases are compound. Even perfect recovery of their four missing groups caps total recall at 16/18 (0.8889), with regression at 5/6 and holdout at 7/8, below the 0.90 per-split gate; at least two non-compound false facts remain. Do not add a query-time splitter or extra inference call for this gate. |
| Qwen3-Reranker-0.6B Q8 selector | [Local bounded probe](#qwen3-reranker-probe), 2026-07-18 | **Rejected** | The llama.cpp rerank endpoint ranked a non-answering meta-summary above the exact answer. The upstream one-token `yes`/`no` prompt classified only 2/6 obvious synthetic pairs correctly and added about 1.06 GiB RSS. Do not add the model, an adapter or another inference runtime. |
| mMARCO text-only passage ablation | Local bounded probe, 2026-07-18 | **Rejected** | With the same model, query and candidate batches, removing title and heading reduced total Recall@5 from 0.6667 to 0.5556, increased expected groups rejected by the mask from 6 to 8, kept 3 unexpected survivors and introduced 1 forbidden hit. Keep the current passage contract; do not retain the adapter. |
| AirWiki-adapted mMARCO selector | [Frozen experiment profile](selector-adaptation-experiment.md#one-time-promotion-observation), 2026-07-18 | **Rejected** | The one-time offline promotion observation retained 70/72 answers but accepted 159 non-answer pairs: precision 0.3057, 134 high-risk false positives, evidence in all 12 no-answer pools and 2/48 exact pools. The promotion corpus also missed its ambiguity-negative coverage gate. Do not tune the cutoff, rerun the holdout, export the checkpoint or retain candidate runtime code. |
| Pool-local contrastive abstention | [Preregistered outcome](pool-null-selector-experiment.md#observed-outcome) and [#29](https://github.com/airwiki/airwiki/pull/29), 2026-07-18 | **Rejected** | The fixed-boundary and query-conditioned arms both achieved precision 1.00 with zero no-answer or high-risk acceptances, but recall was only 0.7083 and 0.7500, lowest-direction recall was 0.50, and exact-pool success was 0.5625 and 0.625. Pool-local training improved safety but discarded too much answer evidence. Do not retain the runner or checkpoints, create fresh holdouts, export either arm or add a query-conditioned empty-passage score to production. |
| Typed-evidence ceiling protocol v1 | Independent pre-observation review of [#30](https://github.com/airwiki/airwiki/pull/30), 2026-07-19 | **Rejected before observation** | No annotations or scored report were created. The protocol could accept semantic fields unsupported by the source, rejected valid records containing multiple entity kinds, specified receipts incompatible with the observed Codex JSON transport and did not bind blind-execution claims to durable traces. The typed-evidence hypothesis remains untested; do not retain this protocol or treat its green CI as experimental evidence. Any new attempt needs a smaller preregistration, observable traces and independent source-only and question-only semantic adjudication. |
| Typed-evidence ceiling protocol v2 | [Frozen preregistration](typed-evidence-ceiling-v2.md), 2026-07-19 | **Preregistered; not observed** | One exact typed matcher, a structure-only control and eight deterministic claim-assignment controls are frozen over the existing 17-case diagnostic. Hosted source and question annotations remain prohibited until the preregistration is reviewed and merged; no v2 annotation or score is yet evidence. |

### Qwen3 reranker probe

The probe used llama.cpp `b9946` (`fb30ba9a6`) and the official Q8 GGUF at
[revision `ff8a9aaee9ddbcdf483ec1b17bc62e864bcb8b04`](https://huggingface.co/ggml-org/Qwen3-Reranker-0.6B-Q8_0-GGUF/commit/ff8a9aaee9ddbcdf483ec1b17bc62e864bcb8b04), SHA-256
`22c9979ce4fbcdc5acdc310c6641c32797eff1aa980b8f7a2db8a8ea23429a48`.
It first exercised `/rerank`, then the upstream
[one-token prompt contract](https://huggingface.co/Qwen/Qwen3-Reranker-0.6B#using-transformers)
through `/completion`, comparing the exact lowercase `yes` and `no` log
probabilities with temperature zero. Six obvious synthetic pairs covered an
official English positive, a Spanish procedural positive, a related-topic
negative, a metadata-only negative, an absent-answer negative and a
cross-language positive. Only the two English-query positives were classified
correctly. Requests ran sequentially on loopback with context 4096, one slot and
four threads; process RSS after the matrix was 1,057.7 MiB. The model and probe
state were deleted after this observation.

## Constraint for the next experiment

Start from one selector failure now isolated by the accepted stage attribution.
The next candidate must improve both false-negative and false-positive behavior
on fresh domains; the observed v2 corpus remains regression and diagnostic
evidence, not promotion evidence. Reject any candidate that weakens privacy,
provenance, stability or known regression cases. Add reusable code to `main`
only when it has an active product or evaluation consumer.
