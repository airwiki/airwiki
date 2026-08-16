# Typed-evidence coverage experiment v3

Status: **preregistration in review; no promotion data observed**.

This experiment addresses the selector failure isolated by the active
[retrieval-quality evaluator](retrieval-quality-evaluation.md). The existing
hybrid retriever places every expected evidence group in an authorized source
pool, while the binary relevance mask rejects five answer-bearing groups and
accepts two unrelated facts. Version 3 tests a smaller and more transferable
hypothesis:

> Can independently extracted, span-bound source claims and question needs be
> joined exactly with enough coverage and abstention quality to justify a
> product prototype?

The experiment does not change product search, authorization, OKF, SQLite,
network protocols or model assets. It does not score the already observed
retrieval-v3 corpus as promotion evidence.

## Why this is a new protocol

Typed-evidence v1 was rejected before observation because structural checks
could be mistaken for semantic validation and its schema rejected valid
multi-entity text. Version 2 corrected those boundaries, but its first hosted
draft was rejected solely because the model returned otherwise valid claims in
a noncanonical order. Its one-shot transport then prohibited a retry.

Version 3 removes hosted annotation from the evaluator and separates four
responsibilities:

1. a frozen automatic candidate extracts source claims without questions;
2. the same candidate extracts question needs without sources;
3. source-only and question-only reviewers create blind gold annotations; and
4. a deterministic Rust scorer canonicalizes order, validates exact spans and
   joins the untouched candidate outputs.

Array and record order are not semantic. The scorer sorts sets and records
before hashing and comparison, while preserving only the candidate rank order
from the private scoring key. This prevents formatting trivia from deciding an
experiment without repairing, reordering or semantically changing candidate
output.

## Frozen representation

The versioned
[field guide](../experiments/typed-evidence-coverage-v3/field-guide.md) is the
normative annotation grammar. Both sides use open normalized identifiers for
entities, relations, object types and qualifiers. No fixture-specific entity
alias or answer mapping is permitted.

A source claim binds:

```text
subject, relation, object type, object value, qualifiers,
polarity, lifecycle, provenance, exact source spans
```

A question need binds:

```text
subject, relation, requested object types, required qualifiers,
allowed polarity, required lifecycle, allowed provenance, exact question spans
```

Every subject, relation, object and qualifier field references one or more
exact UTF-8 byte spans in its own visible text. A mechanical validator proves
only that the spans are nonempty, in bounds, on character boundaries and
contained by the record's support spans. It cannot prove that a normalized
field is the correct interpretation of those bytes. Semantic correctness comes
only from the isolated human review and remains auditable as disagreement with
the automatic candidate.

There is no subject-kind field, fuzzy alias table, embedding comparison or
answer-value prediction. Missing or unresolved fields never widen a match.

## Fresh data and blindness

The package format is private and unversioned. It contains:

```text
manifest.json
inputs/sources.jsonl
inputs/questions.jsonl
runs/macos_arm64/{sources,questions,receipt}.json
runs/windows_x64/{sources,questions,receipt}.json
review/source-gold.jsonl
review/question-gold.jsonl
review/receipt.json
private/scoring-key.json
```

Development and promotion use disjoint domains, entities and templates. Each
split has at least twelve cases across at least four domains, including at
least three no-answer and two compound cases. The promotion split is authored
after the candidate, contract, scorer and gates are frozen.

The source reviewer receives only `sources.jsonl`, the source grammar and an
opaque source identifier. The question reviewer receives only
`questions.jsonl`, the question grammar and an opaque question identifier.
Neither reviewer sees candidate output, pool membership, rank, the opposite
side, expected groups, permissions or scores. The receipt binds their final
canonical annotations and the private scoring key by SHA-256. A receipt is
review evidence, not a cryptographic identity claim.

Candidate runs never receive the scoring key or human gold. The package runner
supplies one side's exact JSONL on stdin, captures stdout and stderr separately,
canonicalizes valid JSONL only after the process exits, and runs each side
twice. A run fails closed on timeout, nonzero exit, stderr, malformed output,
missing or extra IDs, invalid spans, nondeterministic canonical bytes or an
unexpected file mutation. The receipt records input, raw output, canonical
output, binary and frozen source-revision hashes. It does not claim to prove
network isolation or internal model behavior.

## Exact matcher

A source claim covers a question need only when:

- subject and relation are byte-equal normalized identifiers;
- the claim object type belongs to the requested set;
- every required qualifier and lifecycle is present;
- polarity and provenance are explicitly allowed; and
- every field is resolved.

For each authorized source pool, the scorer visits candidates in frozen rank
order. It retains a candidate only if it introduces a new
`(need index, object value)` edge, then stops after five retained candidates.
Distinct values remain visible so contradictions are not hidden. Pools remain
independent until their coverage is combined.

All needs are conjunctive. If any need is uncovered, the case returns no
evidence. Otherwise it returns only retained candidates that cover at least one
need. Authorization and lifecycle filtering happen while the private pool is
built; annotations cannot add a candidate to a pool.

## Causal controls and gates

The treatment is compared with the same two falsification controls used by the
reviewed v2 design:

- **structure-only:** retains annotated candidates without inspecting their
  semantic fields;
- **claim-assignment permutations:** rotates complete claim bundles between
  candidates in each authorized pool for eight deterministic shifts.

Controls cannot be selected as a product mechanism. They must be worse so an
apparent pass cannot be explained by annotation density or candidate rank.

Both macOS arm64 and Windows x64 must produce byte-identical canonical source
and question annotations from the same frozen candidate revision. The
experiment passes only when all of these gates hold independently for
development and promotion:

- Recall@5 is at least `0.90`;
- exact-case success is at least `0.85`;
- exact source-record and question-record agreement with blind human gold are
  each at least `0.85`;
- unexpected, forbidden, authorization, provenance, duplicate and stability
  errors are zero;
- compound cases return all expected groups or abstain completely;
- conflict cases retain every expected value;
- no annotation remains unresolved;
- treatment exact-case success exceeds structure-only and the best assignment
  permutation by at least `0.10` absolute; and
- a second scorer replay is byte-identical.

The unchanged retrieval-v3 and relevance-v2 suites remain regression gates,
not tuning targets. A pass authorizes a bounded product prototype followed by
those regressions and installed-platform tests. It does not authorize changing
the public release gate, adding a runtime dependency or weakening privacy.

## Decision boundary

Before promotion data is authored, review must freeze:

- the contract and field-guide bytes;
- all Rust validation, canonicalization, matching and scoring code, whose
  complete source hash is bound by `contract.json`;
- the automatic candidate source revision and invocation contract;
- split sizes, causal controls and thresholds; and
- the empty promotion package location outside the repository.

After the first promotion score, no prompt, extractor, annotation, vocabulary,
matcher, data, label or threshold may change under v3. A failed gate rejects
this candidate. A transport or integrity failure invalidates the run and may be
repeated only from the same frozen candidate on newly generated execution
receipts; semantic output cannot be repaired or selectively retried.

No private package, raw document, question, annotation, reviewer identifier,
binary or report is committed. Only contract hashes and a sanitized aggregate
PASS/FAIL conclusion may enter the research ledger.
