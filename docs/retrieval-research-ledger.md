# Retrieval research ledger

This ledger keeps durable conclusions from bounded search experiments without
making rejected evaluators, fixtures or candidate mechanisms a maintenance
dependency. The linked pull requests preserve the reproducible implementation.
Green CI means that an experiment ran as designed; it does not mean that its
candidate belongs in the product.

| Candidate | Evidence | Decision | Durable conclusion |
| --- | --- | --- | --- |
| Domain-separated retrieval baseline v2 | [#8](https://github.com/airwiki/airwiki/pull/8) | **Accepted baseline** | Keep the 17-case regression corpus and its structural validator. Do not keep the selectors or model-specific runners bundled into the research branch. |
| Local QA-entailment selector | [#8](https://github.com/airwiki/airwiki/pull/8) | **Rejected** | The candidate missed its predeclared quality gate. Generic answerability machinery is not justified in `main`. |
| Reviewed evidence anchors | [#9](https://github.com/airwiki/airwiki/pull/9) | **Rejected** | Coverage reached 9/9 positive needs, but precision was 10/13 and only 5/11 decisions were correct. Reviewed links can supply candidates, but cannot serve as an answerability decision by themselves. |
| Compact OKF graph on development rankings | [#10](https://github.com/airwiki/airwiki/pull/10), [#11](https://github.com/airwiki/airwiki/pull/11) | **Superseded** | A positive synthetic signal required a real-ranking holdout; it was not sufficient evidence for graph infrastructure. |
| Compact OKF graph on sealed holdout | [#12](https://github.com/airwiki/airwiki/pull/12) | **Rejected** | Baseline, graph and structural sham all produced Recall@5 of 0.75; graph assembly p95 was 123 ms against a 25 ms budget. |
| mMARCO score ordering | [#13](https://github.com/airwiki/airwiki/pull/13) | **Rejected** | Score order was identical to the existing filter order. The observed bottleneck was the binary relevance mask, not ordering among accepted candidates. |
| mMARCO abstention calibration | [#14](https://github.com/airwiki/airwiki/pull/14) | **Rejected** | Cutoff support recall fell from 0.75 to 0.4167, four queries lost support and a hard negative remained. |
| Standalone OKF path signal | [#15](https://github.com/airwiki/airwiki/pull/15) | **Rejected** | The signal connected 17 of 24 hard negatives, so a path alone is not evidence of answerability. |
| Graph-conditioned bounded diffusion | [#16](https://github.com/airwiki/airwiki/pull/16) | **Inconclusive** | Baseline, real graph and degree-preserving sham all found 26/28 evidence groups, and the corpus exposed zero cutoff opportunities. Do not tune or promote from that fixture. |
| Retrieval-stage attribution | [Active evaluator](retrieval-quality-evaluation.md#v2-stage-attribution-observation) | **Accepted diagnostic** | Source-candidate Recall@10 was 1.00 for all 18 expected groups. mMARCO rejected all six missing groups and accepted three non-answering fragments; no expected group was lost in candidate generation, top-k truncation or revalidation. Keep stage attribution in the evaluator, not the product path. |
| Qwen3-Reranker-0.6B Q8 selector | [Local bounded probe](#qwen3-reranker-probe), 2026-07-18 | **Rejected** | The llama.cpp rerank endpoint ranked a non-answering meta-summary above the exact answer. The upstream one-token `yes`/`no` prompt classified only 2/6 obvious synthetic pairs correctly and added about 1.06 GiB RSS. Do not add the model, an adapter or another inference runtime. |
| mMARCO text-only passage ablation | Local bounded probe, 2026-07-18 | **Rejected** | With the same model, query and candidate batches, removing title and heading reduced total Recall@5 from 0.6667 to 0.5556, increased expected groups rejected by the mask from 6 to 8, kept 3 unexpected survivors and introduced 1 forbidden hit. Keep the current passage contract; do not retain the adapter. |

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
