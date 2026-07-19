# Typed-evidence ceiling v2

Status: **frozen preregistration; not observed**.

This protocol replaces the rejected typed-evidence ceiling v1. Version 1 was
rejected before observation because its structural checks could be mistaken
for semantic validation, its subject-kind rule rejected valid multi-entity
text, and its execution receipts were not bound to observable Codex traces.
No v1 annotation or score is evidence for this experiment.

Version 2 is intentionally smaller. It asks one question:

> On the fixed, already observed retrieval diagnostic, what quality ceiling is
> obtained when source claims and question needs are independently drafted,
> independently adjudicated and then joined by one deterministic exact matcher?

It is a model-adjudicated diagnostic of a representation. It is not an
automatic extractor evaluation, a human oracle, a promotion corpus or a
product implementation.

## Scope and limits

The experiment uses the existing schema-v3 retrieval fixture and the exact
authorized candidate pools produced by the accepted stage-attribution
boundary. It does not regenerate candidates during scoring. Candidate order,
source boundaries, top-five truncation, authorization, grants, external-chat
policy, deduplication and revision-bound revalidation remain fixed.

The following limits are part of the protocol:

- only the 17 existing diagnostic cases and their fixed source pools are used;
- each source pool contains at most ten candidates and can disclose at most its
  first five matched candidates;
- the only experiment payload visible to source roles is the source prompt,
  field guide and source input; the only experiment payload visible to question
  roles is the corresponding question material;
- standard Codex system/developer context may still be visible, but the frozen
  wrapper removes the local execution environment, disables integrations and
  denies the filesystem root before dispatch; any unexpected tool event still
  invalidates the execution;
- the scoring key is absent from every dispatch and fresh working directory,
  and scoring is not invoked until all final annotations and execution evidence
  are frozen;
- there is one typed-evidence ceiling, with no alternate threshold, learned
  model, tuning arm or second candidate matcher;
- two deterministic causal controls can falsify an apparent treatment result,
  but are not alternate ceilings;
- after the first score, no prompt, annotation, vocabulary, matching rule,
  fixture or gate may be changed for v2; and
- the protocol uses no Python or PyTorch and adds no model, dependency or
  product runtime.

The corpus is synthetic and already observed. A pass can justify a fresh-domain
experiment, but cannot justify a production change by itself.

## Blind annotation design

The structural grammar is frozen in the
[field guide](../experiments/typed-evidence-ceiling-v2/field-guide.md). It has no
fixture identifiers, answers, entity aliases or source-to-question mappings.
The source and question sides are isolated from each other and from the scoring
key.

Exactly six fresh Codex executions produce the annotation evidence:

1. source draft A;
2. source draft B;
3. source adjudication over the source input and both source drafts;
4. question draft A;
5. question draft B; and
6. question adjudication over the question input and both question drafts.

Draft A and draft B use the same versioned prompt in separate fresh contexts.
An adjudicator is not a majority vote. It must inspect the available source or
question text, correct an agreement that is unsupported, resolve a disagreement
when the visible text permits it, and return `unresolved` otherwise. A source
adjudicator never sees questions, and a question adjudicator never sees sources.
Neither side sees pools, ranks, labels, expected groups, forbidden facts or the
other side's annotations.

The wrapper exposes no output-path argument. It creates exactly
`experiments/typed-evidence-ceiling-v2/evidence`; creation fails if that path
already exists. This protects one worktree from accidental overwrite or an
in-place retry; it is not an append-only external receipt and cannot prove that
another clone or a previously deleted directory never existed. No-retry is
therefore an explicit operating commitment: the first attempt is reviewed and
frozen immediately, and a failed execution rejects v2 rather than permitting a
retry under the same protocol.

The final source and question adjudications are the only semantic inputs to the
matcher. Drafts exist to expose disagreement; they are never merged
mechanically and are never scored.

### Honest validation boundary

Mechanical validation is deliberately narrow. It may prove only that:

- JSONL is well formed and follows the versioned schema;
- input IDs are complete, ordered and unique;
- enums and normalized identifiers are syntactically valid;
- a source `support_quote` is an exact non-empty substring of that source text;
- a `question_quote` is an exact non-empty substring of that question; and
- traces and extracted annotation bytes satisfy the transport contract below.

An exact quote proves where an annotation points; it does **not** prove that its
subject, relation, value, qualifiers or state are semantically supported. No
hash, span, enum, subject token or Rust validator is described as semantic
validation. Semantic judgment belongs only to the context-isolated A/B drafts
and adjudicator. If a later audit finds a semantic error, v2 is rejected rather
than silently corrected.

The representation has no `subject.kind`. A named subject is one normalized
string, and a source may contain claims about several differently typed entities
without becoming invalid.

## Observable execution contract

Every annotation execution uses the transport label
`codex_exec_json_v1`. It means:

- a new `codex exec` process and fresh empty working directory;
- `--ephemeral`, ignored user configuration and repository instructions, the
  preregistered model and reasoning setting, and JSON event output;
- no Codex local execution environment, web search, apps, plugins, multi-agent
  tools, shell tools, shell snapshot or injected skill instructions;
- a permission profile that denies the filesystem root and network, with no
  approval prompts, as a second fail-closed boundary;
- one exact UTF-8 stdin dispatch containing the role prompt and only the role's
  allowed inputs;
- stdout and stderr captured separately without shell interpolation;
- exact stdout JSONL retained as the execution trace; and
- no continuation, retry, tool result or hidden repair step.

A valid stdout trace contains, in order, one `thread.started`, one
`turn.started`, exactly one completed `agent_message`, and one
`turn.completed`. Completed reasoning items are allowed and retained. Every
other item type, including a tool call, tool result, search, file read or shell
command, invalidates the execution. An error, missing terminal event or extra
agent message also invalidates it. The six thread identifiers must be distinct.

The JSON event trace proves only the observable event sequence and absence of
side-effecting items. It cannot prove argv, environment settings, permission
flags or working-directory state. The frozen wrapper therefore writes one canonical
invocation record per role containing the exact argv, sanitized environment
variable names, opaque fresh-directory token, empty-directory checks before and
after execution, preregistration commit, dispatch hash, process status, and
stdout and stderr hashes. The environment is cleared before launch and only
`HOME`, literal `CODEX_EXEC_SERVER_URL=none`, and `RUST_LOG=off` are supplied;
values are never recorded. `OPENAI_API_KEY`, `CODEX_HOME`, `PATH` and every
other variable are not inherited. The wrapper source, exact Codex version and
binary hash, invocation-record schema and permission configuration are frozen
before annotation. This is an auditable process contract, not a claim of
OS-level attestation.

`CODEX_EXEC_SERVER_URL=none` removes the local execution environment used to
register shell, patch and image tools in the frozen CLI. The deny-root profile,
disabled integrations and trace rejection remain independent defense layers.

The wrapper captures stderr in memory and requires it to be empty. It records
the SHA-256 of the empty byte string and a zero byte count, but does not retain
a raw stderr log. A non-empty stderr stream invalidates the run.

The annotation artifact is extracted only from the completed
`agent_message.text`. The raw trace is never rewritten. If the decoded text has
no final LF, the extractor appends one; if it already has exactly one final LF,
it is retained. Carriage returns, NUL bytes, more than one terminal LF or any
non-JSONL commentary invalidate the execution. The manifest records separate
hashes for the exact dispatch, invocation-record bytes, raw stdout trace and
canonical annotation bytes. The invocation record contains the required empty
stderr hash and byte count.

The six dispatches, invocation records and stdout traces, four draft
annotations, two final adjudications and their manifest are retained locally as
private execution evidence before scoring. The raw evidence directory is
ignored by Git. Only its reviewed manifest SHA-256 and a sanitized PASS/FAIL
conclusion may be versioned. Receipt fields authored by the model are not
evidence of isolation or tool usage. The wrapper record is reviewable evidence
of the requested process configuration, not OS attestation; the trace proves
only the observable model event sequence.

Hosted model outputs are not assumed deterministic. Reproduction and review use
the frozen bytes, not a regenerated annotation.

## Frozen representation and matcher

The private evaluation types are defined by the field guide. In summary, a
source claim binds these fields inside one record:

```text
subject, relation, object_type, object_value, qualifiers,
polarity, lifecycles, provenance, support_quote
```

A question need binds:

```text
subject, relation, requested_object_types, required_qualifiers,
allowed_polarities, required_lifecycles, allowed_provenances,
question_quote
```

The matcher performs exact comparison after syntactic normalization. A claim
covers a need only when:

- `subject` and `relation` are equal;
- the claim object type is requested;
- every required qualifier and lifecycle is present; and
- claim polarity and provenance are explicitly allowed.

Question needs never predict an answer value in this ceiling. Missing, unknown
or contradictory fields never widen a match. There is no alias table, stemming,
embedding, classifier or model call in the matcher.

Within each fixed source pool, a candidate is retained when at least one of its
claims covers at least one question need. Existing candidate order is preserved
and the first five retained candidates are eligible. Matching and duplicate
suppression happen before the top-five cutoff:

1. compute every `(need_index, object_value)` match edge for the candidate;
2. remove only edges already retained earlier in that source pool;
3. retain the candidate when at least one edge remains; and
4. add its remaining edges to the source-local seen set, then stop after five
   retained candidates.

A later candidate that duplicates one need but uniquely covers another is
therefore retained for its new edge. Distinct values for one need also remain so
that conflicts are not hidden. Suppression never removes a complete candidate
before its unique edges are considered.

All needs in a question are conjunctive across the eligible authorized sources.
If any need is uncovered, the ceiling returns an empty result for that case. If
all needs are covered, it returns only retained candidates that cover at least
one need. Thus a compound question is either fully supported or abstains; fields
from separate claims cannot be assembled to cover one need.

Any final `unresolved` source or question record, missing annotation, invalid
trace, invalid schema or failed quote binding is an experiment-integrity failure
and scoring must not begin.

## Causal controls

The report evaluates one treatment and two preregistered controls. Controls use
the same frozen candidate pools, source order, top-five limit, conjunctive
gateway rule and gold scorer. They are falsification tests, not candidates for
product integration.

1. **Structure-only control.** A candidate is eligible when it has at least one
   adjudicated claim, without inspecting subject, relation, object type, value,
   qualifiers, polarity, lifecycle or provenance. A case is structurally
   complete when the retained claim count is at least its need count. This
   preserves annotation density and abstention shape while removing typed
   semantics.
2. **Claim-assignment permutations.** Eight deterministic controls rotate each
   candidate's complete claim array onto a different candidate inside the same
   authorized source pool. Candidates use source order; permutation `p` uses
   shift `1 + p % (n - 1)` for a pool of `n > 1`. A singleton receives no
   claims. The original claim bundle is never split, and no candidate keeps its
   own bundle. These deliberately false assignments are not re-adjudicated or
   presented as semantically supported.

The treatment is the only ceiling. The structure-only result and the best of
the eight assignment permutations must remain materially worse, or the result
can be explained by annotation shape or candidate sparsity rather than typed
semantics.

## Single-ceiling gates

The scorer consumes the fixture's frozen `relevant_fact_ids`,
`expected_groups`, `allowed_support_fact_ids`, `forbidden_fact_ids` and case
tags. The tags `compound` and `conflict` are the only classifications used by
their corresponding gates; annotators never see them. Returned facts are the
ordered concatenation of each authorized source's independently limited top
five, in the source order frozen by the candidate artifact.

For one case, an expected group is found when at least one returned fact belongs
to that group. Recall is `found_group_count / expected_group_count`; overall and
split recall sum both counts before division. An answerable case is exact when
all expected groups are found and it has no unexpected, forbidden,
authorization, provenance or duplicate error. A no-answer case is exact only
when it returns no facts.

An unexpected fact belongs to neither `relevant_fact_ids` nor
`allowed_support_fact_ids`. A forbidden fact belongs to
`forbidden_fact_ids`. Support-only facts may be returned, but are disjoint from
expected groups by fixture validation and never increment found-group count.
A duplicate error is either the same fact ID returned twice or more than one
member of one equivalence group returned. An authorization error is a returned
fact absent from that case's frozen authorized pool. A provenance error is a
returned fact assigned to a different source record than in the candidate
artifact. These predicates are evaluated from IDs and mappings in the scoring
key; annotations never see them.

For a `compound` case, partial means that at least one but fewer than all
expected groups are found; the gate permits either all or none. For a
`conflict` case, every expected group must be found. A stability error means
either that the frozen candidate artifact reports incomplete/unstable source
auditing or that the second deterministic scorer replay differs from the first.

The semantic report uses one frozen, versioned Rust schema, BTree-ordered maps,
compact UTF-8 JSON and one final LF. It contains no elapsed time or local path;
timing, if collected, is a separate diagnostic. Deterministic replay means the
complete report bytes are identical without post-processing.

The one typed-evidence ceiling passes only if every gate is satisfied:

- Recall@5 is at least `0.90` overall and independently in regression,
  calibration and diagnostic holdout;
- exact-case success is at least `0.85`;
- zero unexpected evidence and zero forbidden evidence are returned;
- zero provenance, authorization, duplicate or stability errors occur;
- every compound case either covers every expected group or returns no facts;
- every expected group in a conflict case remains represented;
- all source and question adjudications are resolved and all six execution
  traces pass the observable transport checks; and
- treatment exact-case success exceeds the structure-only control and the best
  claim-assignment permutation by at least `0.10` absolute; and
- a second deterministic scoring replay over the same frozen bytes produces a
  byte-identical semantic report.

The report contains one candidate result, the two causal controls and the
already recorded production baseline only as historical context. It does not
rerun the production control. Any failed gate rejects v2. There is no threshold
adjustment, annotation repair or rerun with a new hosted output after scoring.

## Decision boundary

A failure records a compact conclusion in the retrieval research ledger and
removes the one-off runner, annotations and generated evidence from maintained
`main`, following the repository's research policy. It does not reject every
possible typed representation; it rejects this frozen version.

A pass establishes only an adjudicated ceiling on this observed diagnostic. A
separate preregistered experiment must then test automatic source-only and
question-only extraction on fresh domains and both supported platforms before
any product proposal.

This protocol changes no AirWiki crate, SQLite migration, OKF field, public
type, UI, model catalog, embedding, LAN/MCP message, authorization rule or
production search path.

## Freeze block — complete

The values below were derived from the final preregistration bytes. Annotation
is permitted only from a clean `main` whose `HEAD` exactly equals the local
`origin/main`; that complete Git commit is recorded in every invocation and in
the evidence manifest and is the reproducibility authority for dependencies,
toolchain and source. The runner-source digest is an additional review aid. It
covers, in the displayed order, each relative UTF-8 path, a NUL, its exact bytes
and a trailing NUL:

1. `xtask/Cargo.toml`;
2. `xtask/src/main.rs`;
3. `xtask/src/retrieval.rs`;
4. `xtask/src/typed_evidence_trace.rs`; and
5. `xtask/src/typed_evidence_v2.rs`.

```text
preregistration parent commit       e42b3feaa50699afc6894558cc2e027780003858
fixture SHA-256                     8a04bf7eec4aa35e6f5cdfa1c7000ab6d9f666814281c466fb82e5c4b10986ff
fixed candidate artifact SHA-256    d52dbf20fec553ee38f29a01bd72f7430bda16ae96978caf925ab38c7bc046f6
field guide SHA-256                 1b9286680c1b9186a570c87d4660e8b0a3732fbecb62fc80374f1c5a9da4ea02
source annotator prompt SHA-256     2181cfdecb26daac761fa22887afb6642697a2ff444798353e6237d1fed3776d
source adjudicator prompt SHA-256   c1bdf1e107f07841e62b427918e10594ee409981831d3b61ca378e857d3e4404
question annotator prompt SHA-256   182e79754591e372cbd2447a7185d36cb46fe56c1e64199553f511d8e1540efc
question adjudicator prompt SHA-256 f3b3f36ab6abeb4b32f99b253039696df078bd61f2fd2f0c7e4ef1eda0e75c52
source blind input SHA-256          4303eba592c5174c5f37f3aaf35e56df3a25a9270e75a165d35bfebc7516400a
question blind input SHA-256        d71238bf3fa9072a226b995e956a99d0318136b74ac2b60c8e01d22571dff395
runner source SHA-256               14d58e27e8cf482364dbc5ddabba2ab3bbc87cd0ec65219d3fb3fda54dcd0997
invocation-record schema revision   1
semantic-report schema revision     1
codex CLI version                   codex-cli 0.144.4
codex CLI binary SHA-256            3302acbda5f53de1a71ebdb0c0f2aae0d47f9324aa9fb6b4e78a47014fd51c7d
requested model                     gpt-5.6-sol
reasoning setting                   high
```

The scorer is disabled while the versioned sibling file
`experiments/typed-evidence-ceiling-v2/evidence-manifest.sha256` is absent.
After this protocol is reviewed and merged, one invocation of
`typed-evidence-v2 annotate` creates the fixed, Git-ignored evidence directory.
After private review of every exact dispatch, invocation record, trace and
annotation, `typed-evidence-v2 freeze-evidence` writes the manifest hash once to
the sibling file without reading the scoring key, but only while the recorded
execution commit still equals the current reviewed `main`. A separate evidence-only
review versions that 65-byte hash file and a sanitized review conclusion; it
does not version raw traces or annotations. Only after that review is merged
may `typed-evidence-v2 score` read the fixed evidence directory and write its
one-shot report to `target/evals/typed-evidence-v2.json`. No source constant,
runner byte or scoring destination changes between preregistration and scoring.
The evidence hashes cannot appear in this block because they are outputs of the
still-unobserved annotation phase.
