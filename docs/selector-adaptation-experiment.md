# Selector adaptation experiment

Status: **preregistered development experiment**. This profile does not describe
a shipped model and does not authorize a production-model change.

## Why this experiment exists

The schema-v3 retrieval baseline places all 18 expected evidence groups in the
source candidate pools. The current binary mMARCO mask keeps 13, rejects five
answer-bearing groups and accepts two unrelated fragments. Candidate retrieval,
policy, provenance, deduplication and stability hold on that corpus. The next
bounded hypothesis is therefore:

> Fine-tuning the existing multilingual mMARCO cross-encoder on AirWiki's
> answerability boundary can reduce both false negatives and false positives
> without changing retrieval, authorization, protocols or runtime architecture.

The experiment retains
`cross-encoder/mmarco-mMiniLMv2-L12-H384-v1` at base revision
`1427fd652930e4ba29e8149678df786c240d8825`. It changes only candidate weights
and, if promoted, the frozen answerability cutoff. A new model family, graph,
query decomposition, generative selector, runtime or Rust dependency is out of
scope.

## Data contract

The versioned development corpus lives under
`fixtures/selector/answerability-v1/` and separates:

- `queries.jsonl`: query identifier, visible text and language;
- `passages.jsonl`: passage identifier, title, heading, text and language; and
- `judgments.jsonl`: split, world, role, disclosure, review and evidence labels.

Only the following strings may enter the model:

```text
question
title + "\n" + heading + "\n" + text
```

The join matches the production passage contract. Identifiers, split, world,
role, answer group, disclosure, tags, negative kind, evidence spans, review
reason and review state are evaluation metadata and must never be serialized
into model input.

Within each six-candidate development pool, every passage shares the same
neutral title and heading. This prevents visible metadata such as “approved” or
“discarded” from leaking the answer label while preserving the exact production
serialization boundary. Structural validation also rejects internal corpus IDs
or candidate ordinals in model-visible text and high-overlap five-token
templates reused across train and development passages of the same language.

Roles have deliberately different meanings:

- `answer` explicitly states a requested fact and is the only positive training
  label;
- `support` is useful context that does not answer a requested fact and is a
  strict negative for selector training; and
- `hard_negative` is plausible but wrong because of entity, relation, date,
  version, scope, polarity, ambiguity, metadata-only content or unrelated
  injection.

Disclosure is independent. A semantically answering passage remains an answer
even when marked forbidden; authorization must filter it structurally rather
than teaching the model to infer permissions.

The first corpus contains 120 train and 32 development query pools with six
candidates each. ES→ES, EN→EN, EN→ES and ES→EN directions are balanced by
query pool. One quarter of each split has no answer and therefore no support.
Worlds, entities, values and authored templates do not cross splits. All content
is fictional and initially marked `synthetic_draft`; it can prove tooling and
development signal, but cannot by itself authorize promotion.

The sealed development-corpus files have these SHA-256 digests:

```text
queries.jsonl   41d6b1a2c093a920339081f4f2c616e81027e7f69409673f7c511167ecf61c4f
passages.jsonl  3418cf2e5604894800da388ba6e41afc0e0f620c9f64173f4ac1f321b4559696
judgments.jsonl f1d66311a1b799452564a25407ae54980b89d86f8558c255e7be6b28347eee6e
```

Any content change creates a different experiment version; the hashes above
must never be silently updated to preserve a preferred result.

The existing relevance-v2 and retrieval-v3 fixtures remain regression evidence.
They have already been observed and are not training or promotion holdouts.

## Frozen development recipe

Training tooling, caches and checkpoints remain outside the Rust workspace. The
first run uses only the following recipe:

- base model and revision listed above;
- binary cross-entropy with one output logit;
- maximum sequence length 512;
- three epochs, batch size 8, learning rate `2e-5`, weight decay `0.01` and ten
  percent linear warmup;
- deterministic seeds 17, 29 and 43;
- balanced positive/negative sampling during training while evaluating the
  original corpus distribution; and
- a mixture of same-domain hard negatives and random negatives.

For each seed, the cutoff is selected on development data only: choose the
lowest logit that produces zero false-positive development pairs, then measure
the resulting recall. Select the seed by zero false positives, highest recall,
lowest development loss and finally lowest seed number. No parameter may be
tuned against retrieval-v3 or the promotion set.

This follows the established cross-encoder training pattern while addressing a
known hard-negative risk: retrieved negatives can contain unlabeled positives
and must be reviewed before training. See the
[Sentence Transformers cross-encoder guidance](https://sbert.net/docs/cross_encoder/training_overview.html),
the [mMARCO paper](https://arxiv.org/abs/2108.13897) and
[RocketQA](https://arxiv.org/abs/2010.08191).

## Promotion set and gates

After the recipe, seed and cutoff are frozen, create a separate 48-pool
promotion set with twelve pools per language direction and new worlds,
entities, values and wording. Its labels must be independently human-reviewed
before it can support a product decision. Once observed, it becomes regression
data and a later attempt needs fresh promotion domains.

The candidate must satisfy all of these gates:

- answer recall at least 0.90 overall and 0.85 in every language direction;
- precision at least 0.99;
- zero accepted evidence in no-answer pools;
- zero false positives involving wrong entity, date, version, scope, polarity,
  absent facts or unrelated injection;
- support acceptance at most 0.10, with support never receiving answer credit;
- at least 0.85 exact-pool success, meaning every required answer is retained
  without any hard negative;
- relevance-v2 remains above its existing per-split recall gate with zero false
  positives and stable candidate-order and batch-size checks; and
- retrieval-v3 reaches at least 0.90 Recall@5 in every split with zero
  unexpected, forbidden, provenance, duplicate, stability or audit violations.

Improving only false positives or only false negatives is insufficient.
Authorization and lifecycle policy remain independent structural gates.

## Export and platform validation

Only a candidate that passes the semantic gates is exported to ONNX FP32 and
then dynamically quantized. ONNX Runtime recommends dynamic quantization for
transformer models; start with signed 8-bit weights and retain the existing
unsigned AVX2 variant only where platform validation requires it. See the
[ONNX Runtime quantization guidance](https://onnxruntime.ai/docs/performance/model-optimizations/quantization.html).

PyTorch, ONNX FP32, macOS arm64 INT8 and Windows x64/AVX2 INT8 must produce the
same decisions on development, promotion and regression corpora. Both platform
artifacts must stay within the existing two-second deadline for a ten-candidate
pool and must not increase p95 latency, peak memory or artifact size by more
than ten percent without a separately justified product decision.

Probability calibration is not part of this experiment. Temperature scaling
can improve confidence calibration but cannot correct the observed ordering and
sign errors by itself; see
[On Calibration of Modern Neural Networks](https://arxiv.org/abs/1706.04599).

## Decision record

Record one compact result:

```text
hypothesis → sealed corpus hashes → frozen recipe → metrics → decision → reason
```

If rejected, delete candidate code, checkpoints, model files, flags and
dependencies; keep only the result in the research ledger and reusable fixture
or validator improvements. If accepted, update the existing model pins, hashes,
policy tests, notices and platform evidence without introducing a second
runtime or user-selectable selector.
