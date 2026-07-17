# Relevance-gate evaluation profile

This document records the mutable implementation profile and validation
evidence for the answerability boundary defined by
[ADR 0001](adr/0001-answerability-gated-search-v2.md). It is not an architecture
decision. Update it when a reviewed profile or platform evaluation changes.

This evaluation receives an already selected candidate set. The separate
[retrieval-quality profile](retrieval-quality-evaluation.md) exercises the
published-knowledge, SQLite, hybrid-search, policy, deduplication and provenance
path that produces those candidates.

## Active profile

- Model: `cross-encoder/mmarco-mMiniLMv2-L12-H384-v1`
- Revision: `1427fd652930e4ba29e8149678df786c240d8825`
- Policy: `evidence-v1`
- Maximum sequence length: 512 tokens
- Inference micro-batch: at most eight candidates
- Concurrent relevance inference: one request
- Candidate pool: ten deduplicated hybrid-search candidates for every `top_k`
- Queue plus inference deadline: two seconds

macOS arm64 uses `onnx/model_qint8_arm64.onnx` (118,620,017 bytes, SHA-256
`1825907d6c1a9001ff78124780bbde20a614a8c3df3b63409cf3c72c6fe5c8b4`).
Windows x64 with AVX2 uses `onnx/model_quint8_avx2.onnx` (118,620,016 bytes,
SHA-256
`6c2513767fb63d008a4377bef7a7a3555433d9436342bb53e35a3a72ffc52d4b`).
The tokenizer and configuration files are pinned by the same revision and by
the asset manifests in `airwiki-core` and `airwiki-inference`.

The complete candidate set is rejected when its best logit is negative.
Otherwise, a passage is accepted only when its logit is non-negative and no
more than 3.6 below the best passage in that candidate set. The fixed candidate
pool prevents the decision from changing merely because a caller requests a
different `top_k`.

## Reviewed fixture

The schema-v2 fixture at `fixtures/relevance/answerability.json` has SHA-256
`5fd867ba5757828d29203fa2401e5ad24348cb26fb7a066f237758c6de750d31`.
A passage is positive only when it directly supports the requested fact.
Context useful for another question is a hard negative for the current one.

Calibration and holdout domains are disjoint. Each split covers absent facts,
conflicts, prompt injection and cross-language retrieval. Stability checks
reverse candidate order and expand each set to 8, 10, 40 and 80 candidates with
duplicated hard negatives; duplicates do not contribute to classification
metrics.

## Last reviewed platform results

On 2026-07-12, independent macOS arm64 and Windows x64/AVX2 runs both produced:

- calibration: 7 true positives and 14 true negatives;
- holdout: 9 true positives and 21 true negatives;
- zero false positives and zero false negatives;
- all 17 reversed-order checks passed;
- all expanded 8/10/40/80-candidate checks passed.

The macOS run used four threads and completed in 31,150 ms. This is a diagnostic
observation for that machine, not a portable performance requirement. Platform
results are independent; success on one platform must not be inferred for
another artifact.

## Reproduction and acceptance

Ordinary CI validates the fixture without downloading models:

```bash
cargo run --locked -p xtask -- relevance validate
```

A maintainer with an already verified local model snapshot can run:

```bash
cargo run --locked -p xtask -- relevance evaluate --snapshot <verified-snapshot-directory>
```

The evaluation writes
`target/evals/relevance-model-<os>-<arch>.json`. It passes only when each split
has zero false positives, at least 90% recall and every stability check passes.
The report records fixture and artifact identity, target, thread count, policy
and aggregate results; it does not contain question or passage text.
