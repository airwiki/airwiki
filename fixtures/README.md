# Synthetic acceptance corpus

These fictional files validate federation without exposing real organizational
or personal information.

- `mac/proyecto-atlas.md`: recovery procedure and technical validation.
- `mac/atlas-estado-verde.md` and `mac/atlas-estado-ambar.md`: contradictory
  status sources with no precedence rule.
- `mac/atlas-nota-no-confiable.md`: a harmless fact plus a hostile instruction
  that must remain untrusted data.
- `windows/proyecto-atlas-responsable.md`: synthetic owner and target date.
- `private/no-compartir.md`: an Orion canary that must never leave its local-only
  collection.
- `relevance/answerability.json`: schema-v2 relevance corpus with calibration,
  holdout, missing facts, contradictions, prompt injection, and cross-language
  cases.
- `retrieval/search-quality-v1.json`: immutable initial source-side retrieval
  baseline retained for reproducibility.
- `retrieval/search-quality-v2.json`: active source-side retrieval corpus with
  separate regression, calibration and domain-disjoint transfer cases for local
  and federated scopes, policy, grants, provenance, deduplication, stability and
  honest abstention. Its former holdout has been observed and cannot approve a
  production profile; final promotion requires fresh domains.
- `retrieval/reviewed-anchors-v1.json`: development-only overlay of synthetic
  claim and literal-anchor records intended to model a reviewed representation.
  It compares passage QA, claim selection and deterministic conflict detection,
  was authored with the known search-quality questions and is not a promotion
  holdout.
- `retrieval/mini-graph-v1.json`: development-only mechanistic ablation with
  frozen synthetic candidate orders and reviewed internal concept links. It
  compares C10, C32 and bounded one-hop graph expansion without a model or a
  production search change; it is not a promotion holdout.
- `retrieval/mini-graph-real-development-v1.json`: visible development corpus
  with four domain-separated OKF bundles, 12 questions and no authored ranking
  fields. It compares the production BM25/E5/RRF order with one-hop expansion
  and a degree-preserving sham graph; it is not a promotion holdout.
- `retrieval/mini-graph-final-holdout-v1.json`: independently authored and
  sealed multichunk candidate holdout with eight unseen fictional domains and
  40 cases. Its expectations score final top-five citations but never enter
  ranking, graph expansion, chunk selection or relevance classification.

The expected federated question asks how Atlas is recovered, who is responsible,
and the target date. A correct answer combines the Mac procedure with the
fictional Windows owner and date and cites each source node separately.

Always test invalidation on a copy. Change `validación sintética v3` to `v4` and
confirm that the published revision disappears until a human approves the new
one. Search for `ORION-PRIVATE-731` to test access control; no remote title,
metadata, or snippet may be returned.

Structural relevance validation downloads no model:

```bash
cargo run --locked -p xtask -- relevance validate
```

Real evaluation runs separately per platform against an already verified mMARCO
snapshot and writes aggregate synthetic evidence under `target/evals/`:

```bash
cargo run --locked -p xtask -- relevance evaluate --snapshot <snapshot-directory>
```

Structural retrieval validation uses deterministic providers but exercises the
real publication, SQLite, hybrid-search, policy and provenance path:

```bash
cargo run --locked -p xtask -- retrieval validate
```

The platform-specific retrieval profile requires both already verified model
snapshots:

```bash
cargo run --locked -p xtask -- retrieval evaluate \
  --phase development \
  --embedding-snapshot <verified-e5-snapshot-directory> \
  --relevance-snapshot <verified-mmarco-snapshot-directory>
```

The reviewed-anchor mechanism ablation reuses the installed, verified AirWiki
assets and writes a sanitized development report under `target/evals/`:

```bash
cargo run --locked -p xtask -- retrieval evaluate-reviewed-anchors \
  --data-root <AirWiki-data-root> \
  --llama-server <verified-llama-server> \
  --model-id <catalog-id>
```

The mini-graph mechanism ablation needs no model or external service and should
be measured in the release profile:

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-mini-graph
```

The real-ranking replay requires only an already verified multilingual E5
snapshot. It publishes temporary OKF bundles and downloads nothing:

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-real-mini-graph \
  --embedding-snapshot <verified-e5-snapshot-directory>
```

The sealed final mini-graph holdout additionally uses the already verified
mMARCO relevance snapshot and runs only after the evaluator is frozen:

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-final-mini-graph \
  --embedding-snapshot <verified-e5-snapshot-directory> \
  --relevance-snapshot <verified-mmarco-snapshot-directory>
```

See the [retrieval-quality evaluation profile](../docs/retrieval-quality-evaluation.md)
for metrics, report hygiene and current results.

For MCP evaluation:

- the Mac and Windows fixture collections must be published, peer-shareable,
  granted where required, and explicitly enabled for external chat;
- `private/` must remain separate, local-only, and disabled for external chat;
- every new document requires manual review before the golden prompt set; and
- expected URNs, revisions, hashes, and PeerIds come from actual hits and are
  never hard-coded in this directory.
