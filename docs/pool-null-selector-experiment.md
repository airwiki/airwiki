# Pool-level abstention experiment

Status: **preregistered; not executed**. This experiment is disposable research
and does not authorize a model, policy, runtime or production-search change.

## Question

AirWiki's hybrid retriever already places every expected retrieval-v3 evidence
group inside the source candidate pool. The rejected adapted mMARCO selector
then retained 70 of 72 answers on its promotion observation, but accepted 159
non-answer passages, including evidence in all twelve no-answer pools. The
failure is therefore not a lack of semantic similarity. Candidate-independent
binary training did not learn the pool-level ordering AirWiki needs:

```text
answer > abstain > support or misleading evidence
```

The bounded hypothesis is:

> Pool-local contrastive training, with an explicit query-conditioned
> no-evidence candidate, can improve answerability and abstention together
> without adding a second model, service or production dependency.

The experiment includes a fixed-boundary control with identical pool batching.
This separates the effect of training on local candidate contrasts from the
effect of a query-conditioned abstention score. It is not described as a
single-variable comparison with the retired BCE run.

Calibration, score-gap heuristics and ensembles are excluded. They can abstain
on uncertain pools, but cannot repair a confidently wrong entity or relation
and would add another tuning surface or runtime cost.

## Shared training contract

Both arms reuse the existing `fixtures/selector/answerability-v1/`
train/development split and its [sealed
hashes](selector-adaptation-experiment.md#data-contract). No row, label, world,
entity, relation, template or model-visible candidate string may change. The
rejected promotion material was retired and must not be reconstructed or used.

Both arms retain:

- `cross-encoder/mmarco-mMiniLMv2-L12-H384-v1` at revision
  `1427fd652930e4ba29e8149678df786c240d8825`;
- the pinned tokenizer and production serialization of `question` paired with
  trimmed non-empty `title`, `heading` and `text` joined by line feeds;
- maximum length 512, three epochs, AdamW learning rate `2e-5`, weight decay
  `0.01` outside biases and LayerNorm, and ten percent linear warmup; and
- seed `4057359121`, derived from the first 32 bits of the frozen development
  judgments SHA-256 `f1d66311...`. No observed metric selected this seed.

The shared environment is also frozen: Python `3.13.6`, PyTorch `2.7.1`,
Transformers `4.56.1`, tokenizers `0.22.0`, safetensors `0.6.2` and NumPy
`2.3.5`, running on CPU in `float32`. AdamW uses `betas=(0.9, 0.999)` and
`eps=1e-8`. Training uses the pinned model configuration's dropout in
`model.train()`; evaluation uses
`model.eval()` with no stochastic or Monte Carlo dropout. The runner seeds
Python, NumPy and PyTorch, requests deterministic CPU algorithms, and fixes
PyTorch to four intra-op threads and one inter-op thread. With 120 pools,
three epochs and one pool per optimizer step, training has exactly 360 steps
and 36 linear-warmup steps.

Visit every six-candidate training pool once per epoch in the seeded shuffle,
with one complete pool per optimizer step. Use every row; do not resample,
mine, add or remove negatives. Each mean term below has equal weight so the
number of negatives cannot silently redefine the objective.

Training and evaluation tooling stays under ignored `target/` state. It runs
offline, pins every input and output hash, persists aggregate metrics only, and
never writes questions, passages, per-pair scores or labels to logs.

## Frozen arms

For a pool, let `A` be answer scores and `N` be scores for both `support` and
`hard_negative` candidates.

### A. Fixed-boundary poolwise control

This arm retains the active score policy and trains all local contrasts:

```text
answerable pool:
  mean(softplus(n - a) for a in A for n in N)
+ mean(softplus(-a) for a in A)
+ mean(softplus(n) for n in N)

no-answer pool:
  mean(softplus(n) for n in N)
```

At evaluation, reject the whole pool when its best score is negative. Otherwise
accept only candidates whose score is non-negative and no more than 3.6 logits
below the best candidate. This exactly matches the current decision policy.

### B. Query-conditioned no-evidence candidate

For each pool, compute one additional score
`z_q = model(question, empty passage)`. The empty passage is a fixed zero-length
second input. It introduces no special token, label text or trainable parameter
outside the existing model, while allowing the no-evidence score to depend on
the question.

```text
answerable pool:
  mean(softplus(n - a) for a in A for n in N)
+ mean(softplus(z_q - a) for a in A)
+ mean(softplus(n - z_q) for n in N)

no-answer pool:
  mean(softplus(n - z_q) for n in N)
```

At evaluation, reject the whole pool unless its best candidate is strictly
greater than `z_q`. Otherwise accept only candidates strictly greater than
`z_q` and no more than 3.6 logits below the best candidate. Ties reject. The
relative 3.6 policy remains unchanged. The extra score is internal and never
becomes a `SearchHit`.

The arms start independently from the same pinned base weights. Neither arm is
initialized from the rejected checkpoint. No cutoff, margin, seed, epoch,
optimizer or loss weight is selected after an observation.

Before loading any visible development or holdout data, the runner writes an
exclusive attempt receipt that binds the environment, runner, corpus, model
and tokenizer hashes. It later records only `completed` or a sanitized failure
code. An existing receipt or report blocks another attempt. The same mechanism
applies independently to every diagnostic and holdout; crashes do not authorize
a rerun with the same experiment version.

## Staged decision

### 1. Observed compatibility diagnostic

The selector-v1 development split was already observed and previously selected
another candidate's seed and cutoff. It is therefore a diagnostic, not an
independent falsification or promotion set.

Freeze both runner and checkpoint hashes, then evaluate each arm once. An arm
is eligible for the next diagnostic only if it satisfies all of these gates:

- answer recall at least 0.90 overall and 0.85 in every language direction;
- precision at least 0.99;
- zero accepted pairs in no-answer pools;
- zero high-risk false positives;
- support acceptance at most 0.10; and
- exact-pool success at least 0.85.

If neither arm passes, reject the experiment. If one passes, freeze that arm.
If both pass, choose by this preregistered order: fewer high-risk false
positives, fewer no-answer acceptances, higher precision, higher answer recall,
then the fixed-boundary control. This selection is allowed only because every
later acceptance gate uses fresh, separately sealed data. Do not tune either
arm after seeing this diagnostic.

### 2. Observed regression diagnostics

Run only the selected frozen arm once through the existing relevance-v2 and
retrieval-v3 evaluators. Those corpora are already observed; they may reject the
candidate but cannot promote it. The candidate must retain every current
provenance, authorization, deduplication, stability and ordering invariant,
meet the documented per-split recall gates, and emit zero unexpected or
forbidden evidence. Any failure retires the experiment without a fresh corpus.

### 3. Fresh rejection holdout

Only a diagnostic pass permits authoring a fresh sealed rejection holdout. It
contains 32 ten-candidate pools, balanced across ES-to-ES, ES-to-EN, EN-to-ES
and EN-to-EN, with new worlds, entity families, relation families and
paraphrase templates. Each direction contains six answerable pools with exactly
two `answer`, two `support` and six `hard_negative` candidates, plus two
no-answer pools with exactly two `support` and eight `hard_negative`
candidates. The high-risk taxonomy is exactly `wrong_entity`,
`wrong_relation`, `wrong_date`, `wrong_version`, `wrong_scope`, `negation`,
`absent_fact`, `ambiguity`, `metadata_only` and `unrelated_injection`. Each
high-risk kind occurs exactly four times per direction; the remaining twelve
hard negatives per direction are `random`.

Candidate inputs and the adjudication key are frozen separately. Two blind
context-isolated model reviewers may prepare and adjudicate labels without
seeing the key or each other's decisions. This evidence is recorded as
model-assisted and can reject the candidate only. It can never authorize
production, even when all gates pass.

Run the selected candidate once, offline on CPU, using the same semantic and
coverage gates as above. Do not retain per-pair scores. A failure retires the
experiment. A pass only permits the next stage.

### 4. Human-reviewed promotion holdout

A fresh promotion corpus must use new domains and remain unobserved by the
candidate. It contains 48 ten-candidate pools, twelve per language direction
and three no-answer pools per direction. Each direction contains nine
answerable pools with exactly two `answer`, two `support` and six
`hard_negative` candidates, plus three no-answer pools with exactly two
`support` and eight `hard_negative` candidates. It uses the same ten high-risk
kinds as the rejection holdout, each exactly six times per direction; the
remaining eighteen hard negatives per direction are `random`. A person must
audit the entire key and every exact evidence span before model execution.
Model-assisted drafts may reduce editing work, but cannot replace or condition
that audit on the candidate's output.

After the key, candidate and gates are frozen, run exactly once. A label error
found later invalidates the experiment version; it never permits relabeling and
rerunning. The semantic gates remain:

- answer recall at least 0.90 overall and 0.85 in every language direction;
- precision at least 0.99;
- zero accepted pairs in no-answer pools;
- zero high-risk false positives;
- support acceptance at most 0.10; and
- exact-pool success at least 0.85.

All pools must also match the preregistered role and negative-taxonomy shape.
A later attempt requires a new hypothesis and fresh promotion domains.

For every stage, an accepted pair is one the frozen policy marks `Relevant`.
Answer recall is accepted `answer` pairs divided by all `answer` pairs.
Precision is accepted `answer` pairs divided by all accepted pairs. Support
acceptance is accepted `support` pairs divided by all `support` pairs. A
high-risk false positive is any accepted `hard_negative` carrying one of the
ten high-risk kinds above; `random` negatives remain false positives for
precision but are not counted in that separate gate. No-answer acceptance
counts every accepted pair in a no-answer pool. An answerable pool succeeds
exactly only when both required answer pairs and no support or hard-negative
pairs are accepted. A no-answer pool succeeds exactly only when it accepts
nothing. Exact-pool success is successful pools divided by all pools.
If no pair is accepted, precision is recorded as `1.0`; the non-zero answer
denominator and recall gates still reject an arm that abstains everywhere.

## Export and performance gate

Only a semantic promotion pass permits ONNX export and platform measurement.
Compare the candidate and installed baseline in the same quantized ONNX format,
on macOS arm64 and Windows x64/AVX2, with the same ten candidates, thread count,
process state, twenty warmup runs and one hundred measured runs. Record p50,
p95, peak RSS and artifact bytes without content or device identity.

The fixed-boundary arm may add no inference and must stay within 1.05 times the
baseline p95 and peak RSS. The query-conditioned arm scores eleven sequences
instead of ten and must stay within 1.15 times baseline p95 and 1.05 times peak
RSS. Either checkpoint must stay within 1.01 times the baseline artifact size.
The frozen PyTorch checkpoint, ONNX `float32` export, macOS arm64 INT8 artifact
and Windows x64/AVX2 INT8 artifact must make exactly identical accept/reject
decisions on development, relevance-v2, retrieval-v3 and both fresh corpora,
including the `z_q` comparison and relative 3.6 window. Each exported artifact
must also satisfy every semantic gate independently; platform agreement alone
is insufficient.

## Product boundary

No production code changes during the experiment. The fixed-boundary arm uses
the existing policy unchanged. A passing query-conditioned arm would make one
internal extra score, replace the absolute zero comparison with `z_q`, preserve
the relative 3.6 window, and still return the same ordered
`Vec<EvidenceDecision>`. Neither arm requires SQLite, OKF, LAN, MCP or wire
changes.

If this experiment fails, retire its runner and checkpoints. The next distinct
hypothesis is a typed evidence-coverage gate: atomic claims with exact source
spans, explicit entity/relation/qualifier fields and fail-closed slot coverage.
Graph edges would be a compact derived index only after typed units prove value;
paths or diffusion never become evidence by themselves.

## Evidence basis

The design is informed by primary work, not treated as guaranteed transfer:

- [Localized Contrastive Estimation](https://arxiv.org/abs/2101.08751) trains
  rerankers against the local candidates produced by their retriever.
- [RankT5](https://arxiv.org/abs/2210.10634) reports that ranking losses can
  improve ranking and out-of-domain transfer over classification losses.
- [SQuAD 2.0](https://aclanthology.org/P18-2124/) demonstrates that plausible,
  topically related unanswerable passages require an explicit abstention task.
- [Selective QA under domain shift](https://aclanthology.org/2020.acl-main.503/)
  shows that raw model confidence is overconfident out of domain; this is why a
  post-hoc cutoff alone is not the selected first experiment.

These papers motivate the hypothesis. AirWiki's sealed gates decide whether it
works for this product.
