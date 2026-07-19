# Typed evidence coverage ceiling

Status: **preregistered; not yet observed**. The field guide, exact annotation
prompts, blind inputs, candidate pools, control decisions and their hashes are
frozen by the reviewed preregistration commit. It is a bounded diagnostic of a
representation, not a product implementation or a promotion corpus.

## Question

The active schema-v3 retrieval evaluator shows that all 18 expected evidence
groups enter their authorized source candidate pools. The current binary
mMARCO mask retains 13 groups and accepts two ordinary non-answering facts.
Candidate generation, source top five and final revalidation do not account for
those errors.

The bounded hypothesis is:

> Matching independently authored atomic question needs against independently
> authored, typed and source-anchored claims can recover at least four of the
> five missing evidence groups while eliminating both false-evidence facts,
> without changing candidate generation, ranking, authorization or final
> revalidation.

This is materially different from prepending claim identity to an mMARCO
passage. That rejected probe still asked a passage-level classifier to infer
answerability. This experiment makes the claimed relation and its exact source
anchor the unit of comparison and requires complete coverage for compound
questions.

The design is informed, but not proven, by prior work. FEVER records the
sentences required to support or refute a claim; FActScore shows why a binary
judgment over text containing several facts is too coarse; Dense X Retrieval
reports gains from proposition-level rather than passage-level retrieval; and
QDMR represents a complex question as the ordered atomic steps required to
answer it. These results motivate the experiment but do not establish that the
proposed representation works for AirWiki:

- [FEVER](https://aclanthology.org/N18-1074/)
- [FActScore](https://aclanthology.org/2023.emnlp-main.741/)
- [Dense X Retrieval](https://aclanthology.org/2024.emnlp-main.845/)
- [Break/QDMR](https://aclanthology.org/2020.tacl-1.13/)
- [QA-SRL](https://aclanthology.org/P18-1191/)

## Fixed diagnostic boundary

The first observation uses the already observed
`fixtures/retrieval/search-quality-v3.json`, whose SHA-256 is
`8a04bf7eec4aa35e6f5cdfa1c7000ab6d9f666814281c466fb82e5c4b10986ff`.
It contains 17 cases, 18 documents and 27 chunks. Its regression, calibration
and serialized `holdout` splits are diagnostic only. A passing result cannot
authorize product integration, model selection or corpus promotion.

The control is the pinned macOS arm64 mMARCO profile already used by the active
evaluator:

```text
profile   mmarco-mMiniLMv2-L12-H384-v1@1427fd652930e4ba29e8149678df786c240d8825/evidence-v1
artifact  onnx/model_qint8_arm64.onnx
sha256    1825907d6c1a9001ff78124780bbde20a614a8c3df3b63409cf3c72c6fe5c8b4
```

Candidate generation uses the pinned E5 snapshot:

```text
profile   multilingual-e5-small@614241f622f53c4eeff9890bdc4f31cfecc418b3
artifact  onnx/model.onnx
sha256    ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665
```

The production baseline starts from commit
`3f4fe82dc783c2d28c16a1799fac35e5d20db63f`. Before annotations begin, the
preparation command must run that unchanged production search boundary and
version a canonical JSONL artifact containing only opaque question and source
IDs, ordered candidate source IDs, exact candidate snippet hashes and mMARCO
decisions. It also writes separate source-only and question-only blind inputs.
Their SHA-256 values are added here in the preregistration commit. The scorer
refuses any regenerated pool whose bytes differ. This artifact, rather than a
later model replay, is the fixed control input.

The frozen preparation artifacts are:

```text
field guide             26d3cdd0d5628c01e4c83e8ce3000f14c41501e1d3f952a88e59ffd3de3e348c
source prompt           69569f449aa8db1c798f154d59b7f0c558e14484af6412461b781e050f4ba150
question prompt         f0b094d31492f4ab7b5144d9b2880ee8fd07fdf1b0567e53bec418cdf5209fa4
source-input.jsonl      c580a9f44623121d0dcd6cb3f2e558812abce81c4cd40b82c3770f2ffcca21ed
question-input.jsonl    185f6a2013e8452a05faaeaed355629fedfa550ace2cbaa5e7c508176858e7f5
control.jsonl           6121e9195ef0f17d76873ff1dfd43550cf98e745d556966ee43e53ac5d702e7f
completion marker       80e1ccc8c03337fda39cacb923504e7d68b9ea0cc8ae339e2d8ed490464cbbe0
```

They live under
[`experiments/typed-evidence-ceiling-v1/`](../experiments/typed-evidence-ceiling-v1/).
The Rust preparer claimed a new output directory, synchronized all three JSONL
files and published the completion marker last. Two independent executions on
the pinned macOS snapshots produced byte-identical artifacts. The expected
control quality result remains `false`; preparation freezes that failed
control and rejects only evaluator-integrity failures.

The treatment and shams receive exactly the same authorized source candidate
pools, candidate order and visible snippets as the control. BM25, E5, RRF,
source top ten, source top five, deduplication, collection policy, grants,
external-AI policy and revision-bound revalidation remain fixed.

## Independent annotations

Before any matcher is run, two context-isolated annotators produce separate
artifacts:

1. The source annotator sees only opaque chunk IDs, titles, headings and chunk
   text. It cannot see questions, pools, splits, relevance labels, expected
   groups or forbidden facts.
2. The question annotator sees only opaque case IDs and question text. It
   cannot see source documents, chunks, candidates, rankings, expected groups
   or labels.

Both annotators use the same versioned structural field guide but work
independently. The guide is part of the preregistration commit and its hash is
recorded before blind inputs are generated. It contains only identifier
grammar, closed object and claim-state enums, qualifier syntax and generic
naming rules. It contains no fixture entity, case or chunk ID, surface phrase,
alias, answer, example from the corpus or mapping between a question and a
source. A guide containing any such material invalidates the experiment.

Annotators must represent negative, conditional, planned, attributed and
retracted statements rather than creating atoms only for positive current
evidence. An unsupported or ambiguous construction is explicitly
`unresolved`; it is never silently coerced into a positive current fact.

The exact source-only and question-only prompts, requested model family,
reasoning setting and output schema are also versioned before annotation. The
annotation artifacts are labelled `blind_model_assisted`. They are not
human-reviewed claims. Reproduction relies on the frozen annotation bytes, not
on deterministic regeneration by a hosted model. Their bytes and the two blind
input files are hashed before the scoring key is joined. Editing an annotation,
matcher rule, field guide or sham after the first scored report creates a new
experiment version; the current fixture may not be scored again.

## Private representation

The evaluator uses private `xtask` types. Nothing in this section is a public
AirWiki, OKF, SQLite or wire contract.

```text
SourceAnnotation =
  ResolvedSource { source_id, claims: [ResolvedClaim...] }
  | UnresolvedSource { source_id, reason_code }

ResolvedClaim {
  atom_id
  subject { name, kind }
  relation
  object_type
  object_value
  qualifiers[]
  polarity
  lifecycles[]
  provenance
  anchor { byte_start, byte_end, text_sha256 }
}

QuestionAnnotation =
  ResolvedQuestion { question_id, needs: [ResolvedNeed...] }
  | UnresolvedQuestion { question_id, reason_code }

ResolvedNeed {
  need_id
  subject { name, kind }
  relation
  requested_object_types[]
  answer_intent
  tested_object_value?
  required_qualifiers[]
  allowed_polarities[]
  required_lifecycles[]
  allowed_provenances[]
}
```

Subject names, relations, object types, qualifier names and qualifier values
are validated lowercase ASCII slugs. Matching is exact after validation,
including exact membership in its closed polarity and provenance sets and
required-subset matching for lifecycle; the matcher
has no synonym table, stemming, embeddings or model call. Subject kinds must
match when both are explicit; `unspecified` is compatible with one explicit
kind. A `ResolvedNeed` never contains a fact ID, atom ID, answer-group ID or
answer value. A value-verification need may contain only the object value
explicitly tested by its question; value lookup and existence verification do
not contain one. Gold equivalence groups remain available only to the existing
scorer.

An anchor is valid only when:

- offsets are non-empty UTF-8 byte boundaries inside the canonical chunk;
- the SHA-256 of the exact anchored bytes matches `text_sha256`;
- the source chunk is in the already authorized candidate pool; and
- the complete anchored text appears in the snippet that would be disclosed.

Offsets alone are not treated as identity. A later product experiment would
also need collection, source-document, revision and source-hash bindings, but
those are deliberately not added to production storage for this ceiling.

## Frozen matching rule

A claim covers one resolved need only when subject name, compatible subject
kind and relation match, its object type is requested, its polarity and
provenance are explicitly allowed, every required lifecycle and qualifier is
present, and a value-verification value matches exactly. An unknown,
contradictory or missing required field does not match.

Matching has two evaluation-only stages because a federated question may need
evidence from more than one node:

1. **Source mask:** inside each already authorized source pool of at most ten
   candidates, mark a candidate relevant when one of its resolved claims covers
   at least one resolved need. For the same need and object value, keep only the
   earliest candidate in source order. Preserve different values as potential
   conflict evidence. The unchanged search pipeline then applies its existing
   per-source deduplication and top-five truncation.
2. **Gateway coverage:** after the unchanged federated merge, verify coverage
   across only the returned, authorized hits. All needs are conjunctive. If any
   need is uncovered, replace the treatment result with an empty result. If all
   are covered, retain the existing federated order and every distinct value
   for a covered need. The gateway never loads claims for candidates that a
   source did not disclose.

This allows legitimate Mac-Windows compound evidence while preventing a final
partial compound answer. Subject and required qualifiers remain bound inside
each claim; fields from different entities, scopes, revisions or times cannot
be combined to cover one need.

Any `UnresolvedSource` or `UnresolvedQuestion` in this small ceiling corpus
fails annotation preflight before the scoring key is joined. Invalid anchors,
unknown or duplicate output IDs and annotation lookup failures are evaluator
errors. They do not become a clean no-answer result. Legitimate no-answer cases
still contain resolved needs but have no matching authorized claim.

## Arms and shams

The one scored diagnostic reports four arms:

1. **mMARCO control:** the current production relevance decisions.
2. **Typed treatment:** the complete frozen matching rule above.
3. **Coverage-count sham:** a claim covers a need when object type and qualifier
   count match, ignoring subject, relation, all three state axes, qualifier
   names and all values. It uses the same two-stage selection, duplicate and
   conflict rules as treatment.
4. **Bundle-rotation sham:** order resolved claims by `(object_type, atom_id)`
   inside each object-type stratum. For a stratum of size `n > 1`, rotate the
   complete semantic bundle `(subject, relation, polarity, lifecycles,
   provenance, qualifiers)` left by `1 + seed % (n - 1)` positions while
   leaving atom ID, object type, object value and anchor fixed. For `n = 1`,
   replace every slug in the bundle with the reserved valid slug
   `sham_unmatched` while preserving field count. No sham claim can retain its
   original semantic bundle.

The sham seed is the unsigned big-endian value of the first four fixture-hash
bytes, `0x8a04bf7e`. Sorting uses UTF-8 byte order and rotation moves the bundle
as one unit; no RNG is used. A sham that passes the treatment gates invalidates
the proposed causal interpretation: the apparent gain could come from
sparsity, anchor length or set shape rather than typed semantics.

## Gates

The treatment must satisfy every absolute gate:

- at least 17 of 18 expected groups returned (Recall@5 at least `0.90`);
- Recall@5 at least `0.85` in regression, calibration and diagnostic holdout;
- at least `0.85` exact-case success. An answerable case is exact when every
  expected equivalence group has a returned fact and there is no unexpected,
  forbidden, provenance or duplicate violation; a no-answer case is exact only
  when it returns no facts;
- zero unexpected, forbidden, stale, unauthorized or no-answer evidence;
- zero partial compound answers: a `compound` case either returns at least one
  fact from every expected group or returns no facts;
- complete preservation of expected conflicts: every expected group in a
  `conflict` case is returned;
- zero provenance, duplicate, stability, audit or invalid-anchor errors;
- zero unresolved source or question annotations; and
- three byte-identical reports from deterministic replays over the frozen
  candidate artifact, excluding only a separately reported elapsed-time field.

It must also improve both observed error classes over the fixed control: more
than 13 expected groups returned and fewer than two unexpected facts. Both
shams must fail at least one absolute safety or quality gate, and treatment
recall must exceed bundle-rotation recall by at least `0.20`.

Any failed gate rejects the representation for this experiment version. No
field vocabulary, annotation, matcher rule, seed or threshold may be adjusted
after the scored report.

## Decision boundary

A rejection removes matcher code, annotations, generated inputs and reports;
only the compact outcome remains in the research ledger. A pass establishes
only that this representation has a useful oracle ceiling on an observed
diagnostic corpus.

A pass would justify a separate experiment for automatic, offline extraction.
That later experiment needs fresh domains, source-only claim annotation,
question-only need annotation, an independently sealed scoring key, human
review, all four ES/EN directions and explicit negatives for entity, relation,
polarity, attribution, scope, time, version, units, comparators, ambiguity,
support-only context, injection and Frankenstein cross-source bindings. It must
also reproduce decisions with the Windows ONNX artifact before any production
change.

This experiment never adds a graph engine, claim table, migration, embedding,
model, dependency, public type, OKF field, UI, LAN/MCP field or search-protocol
change.
