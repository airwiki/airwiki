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
Its 17 cases cover direct and paraphrased retrieval, cross-language and compound
questions, absent and withdrawn facts, contradictions, common-name ambiguity,
prompt injection, near duplicates, peer grants, external-chat policy and stable
ordering.

V2 separates cases by their permitted use:

- `regression` contains the five observed failures from the initial v1 run;
- `calibration` is a development/tuning split containing reviewed examples that
  may guide model selection or a decision policy; its name does not imply that
  the profile measures probabilistic calibration; and
- `holdout` contains Harbor, library, sensor and Quasar transfer domains.

Every document and case declares a domain. The validator requires regression,
calibration and holdout domains to be pairwise disjoint and rejects a case that
references evidence outside its own domain. If a holdout result guides an
implementation change, that holdout is no longer unobserved and a future
profile must reserve new domains rather than relabel the same examples. The v2
holdout was observed during the initial baseline audit on 2026-07-17, so it is
now diagnostic transfer evidence and **cannot** approve a production profile.
A future final evaluation must introduce fresh structural domains after the
candidate model, prompt and decision policy are frozen.

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

For a multi-node case, Recall@5 means that required evidence must appear within
the top five returned by its source node. The evaluator combines that source
coverage for scoring; it does not claim that up to ten source hits are the
gateway's final top five. The production coordinator's second RRF, cross-node
deduplication and partial-result behavior remain covered by focused
`airwiki-network` tests.

The evaluator does not cover file parsing, generative enrichment, network
transport, pairing UX, chat-client synthesis or installed-platform behavior.
Those boundaries retain their focused tests and manual acceptance paths.

## Research method for an emerging domain

LLM Wiki, reviewed local knowledge and OKF federation do not yet have one
established end-to-end solution. AirWiki therefore uses literature as prior
evidence and a source of baselines, datasets and known failure modes—not as an
implementation recipe. New mechanisms may originate from observed product
failures, contributor hypotheses, architectural constraints or adversarial
analysis. They earn adoption only through reproducible evidence.

This document distinguishes four kinds of statement:

- **External evidence** is a result reported by cited work. It may motivate a
  control, but it does not establish transfer to AirWiki.
- **AirWiki hypothesis** is an original, falsifiable engineering thesis. It must
  state its predicted mechanism, comparison baseline and rejection condition.
- **Product invariant** is a deliberate privacy, authorization, integrity or
  usability requirement. It is not an empirical average and cannot be traded
  away by a better aggregate score.
- **Observation** is a result for one pinned corpus, implementation, model and
  platform. Its claim ends at that boundary.

Before implementation, a non-trivial hypothesis records: the user-visible
failure; the proposed causal mechanism; a minimal baseline and ablation; the
primary metric and safety veto; expected invariances; counterfactuals that must
change the result; resource budgets; allowed development data; a frozen
holdout; and a predeclared rejection condition. A negative result is retained
when it rules out a plausible design; it is not tuned away case by case.

Evidence advances through inexpensive deterministic invariants, metamorphic and
counterfactual tests, synthetic development cases, reviewed AirWiki-like cases,
a fresh grouped holdout and finally installed-platform measurement. Behavioral
tests are especially useful here: reordering candidates, adding an irrelevant
distractor or paraphrasing a need should preserve the decision, while changing
the subject, date, negation, approval state, revision or grant should change it.
This follows the capability-oriented testing principle illustrated by
[CheckList](https://aclanthology.org/2020.acl-main.442/), while AirWiki's exact
relations and safety vetoes remain its own hypotheses and product decisions.

Benchmarks enter through a capability matrix rather than by accumulation. A
public dataset is included only when it supplies a missing control; reviewed
AirWiki-shaped scenarios remain necessary for revisions, grants, temporal
conflicts, compound coverage and human publication. Gold evidence must declare
whether spans are complementary (`all_of`), interchangeable (`one_of`) or only
corroborating, and ambiguous promotion cases require independent human
adjudication. The same model may propose an annotation, but cannot be its final
gold authority.

## Evidence basis

The current candidate stage intentionally remains BM25 plus multilingual E5,
followed by RRF with `k=60`. Multilingual E5 was evaluated on multilingual and
cross-lingual retrieval benchmarks, while the original RRF study shows why
combining ranks avoids comparing arbitrary scores from heterogeneous retrievers
([Wang et al., 2024](https://arxiv.org/abs/2402.05672),
[Cormack et al., 2009](https://cormack.uwaterloo.ca/cormacksigir09-rrf.pdf)).
RRF `k=60` is a literature-motivated baseline, not a claim that the value is
optimal for AirWiki. Before changing this stage, a promotion profile must report
pre-selector candidate Recall@10 separately from selector errors.

Answerability is a separate selective-prediction problem. SQuAD 2.0
operationalized answerable questions together with plausible, similar-looking
passages that do not state an answer. AirWiki turns that failure mode into a
fail-closed abstention policy. Selective-QA research also shows that raw model
confidence is unreliable under domain shift
([Rajpurkar et al., 2018](https://aclanthology.org/P18-2124/),
[Kamath et al., 2020](https://aclanthology.org/2020.acl-main.503/)). Neural
scores are also not probabilities without calibration
([Guo et al., 2017](https://proceedings.mlr.press/v70/guo17a.html)). For that
reason, AirWiki does not promote another reranking threshold merely because it
improves a known example. This small synthetic corpus is a regression and
privacy gate, not statistical evidence of calibration. Production promotion
also requires a fresh, larger domain-separated profile that reports selective
risk and coverage with uncertainty, plus fail-closed behavior on malformed
output.

**AirWiki hypothesis H-A1:** question-answering entailment may represent
AirWiki's evidence boundary better than generic semantic similarity.
QA-entailment research evaluates whether a passage
supports a complete question-and-answer claim, while QNLI asks only whether a
sentence contains an answer and generic NLI assumes a declarative hypothesis
([Chen et al., 2021](https://aclanthology.org/2021.findings-emnlp.324/),
[Wang et al., 2018](https://aclanthology.org/W18-5446/),
[Conneau et al., 2018](https://aclanthology.org/D18-1269/)). These tasks are
useful experimental controls, not interchangeable production gates. A candidate
answer span must first be turned into a complete claim without changing its
subject, relation, scope or negation. The passage must entail that claim; merely
locating a plausible date, person or phrase is insufficient.

Evidence-verification work also supports testing entailment as a secondary
ranking signal, and SemQA combines question similarity with bidirectional answer
entailment rather than treating either signal as sufficient on its own
([Yang et al., 2021](https://aclanthology.org/2021.ranlp-1.174/),
[Indrehus et al., 2025](https://aclanthology.org/2025.fever-1.14/)). AirWiki
therefore evaluates a staged verifier—candidate retrieval, answer-span
extraction, complete-claim construction and passage support—before considering
changes to the first-stage retriever. This is a testable design hypothesis, not
evidence that the cited systems transfer to private organizational documents.

Every future promotion-oriented model experiment therefore starts with a
falsifiable hypothesis, an immutable artifact identity, a development-only
corpus, a primary metric and a predeclared rejection condition. Aggregate
ranking improvements cannot override a known false-evidence regression. Model
output never overrides authorization, publication state, literal-span
provenance or final revision revalidation.

Model selection uses only regression and calibration domains. Repeatedly using
a test set for selection biases the reported result, so final domains are used
once after freezing the candidate
([Cawley and Talbot, 2010](https://www.jmlr.org/papers/v11/cawley10a.html)).
Prompt-injection passages remain adversarial data rather than instructions;
deterministic validation and authorization remain the security boundary because
prompt-only defenses are insufficient
([Liu et al., 2024](https://www.usenix.org/conference/usenixsecurity24/presentation/liu-yupei)).
Passing the included attacks is a regression result, not a robustness guarantee;
a final security profile must report false-selection or attack-success rates
across multiple indirect-injection variants.

## Answerability corpus provenance

The next experiment begins from the closed, content-free manifest documented in
[retrieval answerability development corpus v1](../resources/evaluation/retrieval-answerability-development-v1/README.md).
It pins SQuAD 2.0, parallel English/Spanish XQuAD and ContractNLI artifacts by
repository revision, byte size, SHA-256 and dataset license. The sources cover
plausible no-answer passages, cross-language transfer and document-level
relation, scope and negation. They are complementary controls, not evidence that
one dataset or model represents private AirWiki knowledge.

Only traceability metadata—source-native record identifiers, local non-content
locators, grouping decisions and expected support roles—is versioned. Dataset
text remains outside the repository and packages.
The initial manifest contains training and calibration groups only; a fresh
grouped holdout will be selected after the candidate structure, model inputs and
decision policy are frozen. This prevents a repeatedly observed test set from
becoming an implicit tuning set.

CI validates the manifest without downloading any dataset:

```bash
cargo run --locked -p xtask -- retrieval corpus validate
```

The validator rejects unknown fields, unsupported licenses, malformed hashes,
unsafe artifact paths, duplicate identifiers, dangling source references,
invalid answerability labels and document or translation groups split across
training and calibration. It reports counts only. It does not read questions,
passages or answers and does not claim that a pinned artifact has been locally
downloaded or verified. After separately accepting the upstream terms and
placing the referenced files under an ignored local root, a maintainer can
verify hashes and source-native locators without downloading or reporting
content:

```bash
cargo run --locked -p xtask -- retrieval corpus verify \
  --source-root <source-root>
```

The verifier rejects symlinks and unsafe archive members, parses each referenced
artifact once and emits only identifiers, fingerprints and counts. The exact
local layout is documented with the
[corpus manifest](../resources/evaluation/retrieval-answerability-development-v1/README.md).

### Experimental QA-entailment contract

The next experiment keeps BM25, multilingual E5 and RRF fixed and evaluates one
manifest-provided atomic need at a time. It deliberately separates two model
calls:

1. a proposal call selects one or more exact passage quotes and, for a question,
   converts the proposed answer into a complete declarative claim; and
2. a verification call receives the original atomic need, that frozen claim and
   only the selected passages, then separately decides whether the claim answers
   the need and whether the passages entail it. It cannot rewrite the claim.

Because the manifest supplies atomic needs, this experiment does not evaluate
question decomposition, cross-need synthesis or conflict presentation.

This experiment is inspired by the staged QA-to-NLI formulation studied by
[Chen et al. (2021)](https://aclanthology.org/2021.findings-emnlp.324/) and
retains multi-span evidence because
[ContractNLI](https://aclanthology.org/2021.findings-emnlp.164/) shows that a
document-level hypothesis may require more than one evidence span. The two
calls are not statistically independent when they use the same local model;
their purpose is to make proposal and verification separately observable and
falsifiable. This does not reproduce the trained NLI verifier in the paper or
inherit its results. A single call that writes both the claim and its verdict
would let the model weaken the claim to make its own evidence appear sufficient.

Rust remains the authority for the closed JSON schema, known identifiers,
literal quote provenance, length bounds and all-or-nothing coverage. A question
proposal must copy its answer text from a selected passage and include that
literal answer in the complete claim. If the supporting quote is a different
literal span in the same selected passage, Rust derives the minimal answer quote
deterministically rather than asking the model to duplicate the same span twice.
Source-native SQuAD or XQuAD reference answers are never passed into either
model call or used to change its control flow. The evaluation harness compares
an accepted answer with those references after inference using the official
SQuAD v2
[normalized exact-match policy](https://github.com/rajpurkar/SQuAD-explorer/blob/eee5fdbf62f8613a7812b03419e6b29617b74fd1/evaluate-v2.0.py);
an accepted non-match fails the gate
but is not misclassified as malformed JSON.
This separation prevents the gold answer from becoming a runtime oracle while
still detecting a weakened answer. XQuAD itself uses the official SQuAD
evaluation script and documents the limits of its English-specific
normalization
([XQuAD repository](https://github.com/google-deepmind/xquad#training-and-evaluation)).
Candidate identifiers and their deterministic order are blinded to
the support or hard-negative role so the prompt cannot reveal the gold label;
this also avoids always placing gold evidence in one prompt position, since
language-model use of evidence can change with its context position
([Liu et al., 2024](https://aclanthology.org/2024.tacl-1.9/)). A declarative
ContractNLI need is already the frozen claim and cannot be rewritten. Any
timeout, malformed output, unknown identifier, invented quote, need mismatch or
unsupported claim produces abstention. The verifier never receives or changes
authorization, publication or provenance state.

The report fingerprints the answer matcher and the complete scoring policy,
including failure-as-abstention, proposed-versus-accepted evidence, complete
support coverage and language-parity rules. Evidence is canonicalized into the
input candidate order before verification. If the proposal succeeds but the
verification call fails, proposed evidence remains a diagnostic observation
while accepted evidence remains empty. These rules make two runs comparable;
they do not make the two calls statistically independent or prevent a shared
model from repeating its own error.

The six-selection seed is only a smoke and rejection gate. It must retain every
reviewed support in the final accepted evidence, retain no hard negative or
unanswerable case in final evidence, produce no invalid output or timeout, and
preserve the English/Spanish XQuAD decision. It
does not justify a confidence threshold, a statistical safety claim or
production integration. Selective-QA work shows that raw model confidence is
unreliable under domain shift, while conformal risk control requires assumptions
and sample sizes that this seed does not establish
([Kamath et al., 2020](https://aclanthology.org/2020.acl-main.503/),
[Angelopoulos et al., 2022](https://arxiv.org/abs/2208.02814)). A future frozen
candidate must be evaluated once on a fresh holdout grouped by document,
translation family and domain.

After the corpus and the already-installed AirWiki assets verify, a maintainer
can execute this development-only gate with:

```bash
cargo run --locked -p xtask -- retrieval evaluate-answerability \
  --source-root <source-root> \
  --data-root <airwiki-data-root> \
  --llama-server <llama-server-path> \
  --model-id <catalog-model-id>
```

The command does not download assets or change production search. It runs one
local model request at a time and writes only fingerprints, aggregate
training/calibration counts and latency summaries under `target/evals/`. A
failed gate rejects this experimental structure; a passed gate only permits the
work to proceed to a frozen, fresh holdout.

### QA-entailment development observation

The macOS arm64 run on 2026-07-17 used the pinned Gemma 4 E4B Q4 artifact and
`llama.cpp` build `b9946`. The final development candidate fingerprint was
`22b8163839b6d4d2a7f39192c2908f2b60045b099b5a007597e386f24b3e14f1`.
The run used answer-match policy `squad-v2-normalized-exact-match-v2` and
scoring policy `answerability-scoring-gate-v1`.
It produced:

- one accepted ContractNLI support set and zero accepted hard negatives;
- one hard-negative proposal that the second stage rejected, an observation
  consistent with reporting proposal diagnostics separately from final
  evidence;
- three effective false-negative decisions across the three QA training cases;
- two invalid literal answer spans and one inconsistent generated claim;
- zero timeouts, provider failures, accepted hard negatives or final false
  positives;
- descriptive proposal latency of p50 10.977 seconds and p95 15.183 seconds,
  and descriptive verification latency of p50 2.256 seconds and p95 4.656
  seconds;
  the call counts are too small to estimate platform performance; and
- 66.392 seconds of aggregate evaluator time, excluding process compilation.

The candidate therefore fails the seed rejection gate. In particular, a zero
observed final false-positive count cannot compensate for zero QA coverage.
This is one descriptive run; temperature zero does not establish deterministic
model behavior, and a promotion candidate would require repeated-decision and
evidence-set stability measurements.
AirWiki must not tune further prompts against these six observed selections or
promote this verifier into production. A next candidate requires a larger
licensed development corpus and either a reader designed for multilingual
answerability or another independently justified, lower-complexity method; it
must then be frozen before a fresh grouped holdout is revealed.

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
snapshots of multilingual E5 and mMARCO. Development mode excludes both holdout
cases and holdout documents:

```bash
cargo run --locked -p xtask -- retrieval evaluate \
  --phase development \
  --embedding-snapshot <verified-e5-snapshot-directory> \
  --relevance-snapshot <verified-mmarco-snapshot-directory>
```

No evaluation command downloads models. A run is platform-specific and writes
`target/evals/retrieval-pipeline-v2-current-development-<os>-<arch>.json`
whether it passes or fails. The command exits unsuccessfully when the measured
profile misses an acceptance threshold. Final mode is deliberately rejected for
the active fixture because its former holdout has already been observed.

### Generative-selector experiment

Maintainers can compare the current relevance model with a strict local
generative selector without changing the desktop or production search path. The
experiment accepts only assets already pinned and verified by AirWiki:

```bash
cargo run --locked -p xtask -- retrieval evaluate-selector \
  --phase development \
  --data-root <verified-AirWiki-data-root> \
  --llama-server <verified-bundled-llama-server> \
  --model-id gemma-4-e4b-q4
```

The model must identify one to four atomic information needs and return an exact
quote from every selected candidate. Rust rejects unknown fields, unknown or
duplicate candidate identifiers, invented quotes and oversized quotes. Candidate
content is untrusted data, not an authorization boundary. Invalid output fails
the affected case and returns no evidence. Exact substring validation proves
provenance integrity only; it does not by itself prove that the quote entails the
answer. A production gate must add reviewed support spans or an equivalent
grounded-support check.

The selector exists only in `xtask`. A development-quality pass cannot promote
it into `airwiki-core` or the desktop. Promotion first requires a fresh final
profile plus installed macOS and Windows measurements that satisfy production
latency, memory and shutdown budgets. It must also pass a maximum-size candidate
payload profile: the development fixture intentionally does not prove behavior
for ten maximum-length snippets within the generation context budget. Its report
is written to
`target/evals/retrieval-pipeline-v2-selector-development-<os>-<arch>.json`.

## Metrics and acceptance

Every regression case must pass individually so aggregate recall cannot hide a
known failure. In addition, each regression, calibration and holdout split must
independently satisfy:

- Recall@5 of at least 0.90 across expected evidence groups;
- zero unexpected evidence facts;
- zero forbidden evidence facts;
- zero provenance errors;
- zero duplicate violations; and
- stable repeated results, stable top-5 prefixes and stable results after
  reversing insertion order.

The report's MRR@5 field is first-evidence reciprocal rank: it uses the first
returned member of any expected evidence group. Every answerable case is
included in the denominator, and a miss contributes zero. It does not measure
completion of every need in a compound question; the all-groups pass condition
does. MRR@5 and elapsed time are diagnostics rather than acceptance thresholds.
Promotion also requires separate platform-specific latency and memory gates;
quality success on macOS cannot waive the Windows CPU budget or the LAN/MCP
deadlines.

## Report privacy

The JSON report is written as
`target/evals/retrieval-pipeline-v2-<profile>-<phase>-<os>-<arch>.json`. It
contains the evaluation phase, a candidate fingerprint, fixture and artifact
identity, target platform, thread count, per-split aggregate metrics, synthetic
case and fact identifiers, stability flags, elapsed times and PASS/FAIL. A
selector report additionally contains only aggregate call counts, sanitized
failure categories and p50, p95 and maximum call latency. It contains no
question or passage text, generated needs or quotes, snippets, source-document
paths, source-document hashes, local usernames, peer identities, IP addresses,
ports, endpoints, tokens or multiaddresses. Reports remain ignored under
`target/`; maintainer evidence should retain only the aggregate fields allowed by
[the validation-record policy](maintainer-validation.md).

The answerability experiment writes
`target/evals/retrieval-answerability-development-<os>-<arch>.json`. It contains
only corpus, candidate and artifact fingerprints; versioned policy identifiers;
platform and bounded runtime parameters; aggregate split counts; sanitized
failure categories; latency summaries; and PASS/FAIL. It follows the same
content, path, identity, endpoint and token exclusions above. In particular, it
does not serialize needs, answers, claims, quotes, passages, candidate IDs,
source-native record IDs or local roots.

The reviewed-anchor ablation report contains fixture, model and policy
fingerprints; target and bounded runtime parameters; synthetic case identifiers;
per-case expected and observed decisions, counts and elapsed times; aggregate
coverage, precision, conflict and failure counts; call and latency summaries;
and the two development-readiness booleans. It does not serialize questions,
atomic needs, claims, anchors, structured keys or values, selected fact
identifiers, snippets, source-document paths or source-document hashes, peer
data, endpoints or model output. `production_promotion_ready` is always false
for this observed development fixture.

## Initial v1 platform observation

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

The current real-model profile therefore **fails** this retrieval-quality gate.
The result established an honest baseline: the authorization, provenance,
deduplication and stability boundaries held, while retrieval completeness and
false-evidence control needed focused improvement. It did not by itself justify
tuning the fixture or changing product protocols merely to turn the observation
green.

These measurements apply only to schema v1.

## Corrected v2 development observation

The macOS arm64 development run on 2026-07-17 used the same pinned E5 and
mMARCO revisions against the corrected schema-v2 corpus. Its immutable candidate
fingerprint was
`2b83cc6fb163da61ccce78bc10448dfed82230e5484db8027d26e117c0dfc9be`. It
produced:

- regression Recall@5: 0.50 and MRR@5: 0.75;
- calibration Recall@5 and MRR@5: 1.00;
- overall Recall@5: 0.70 and MRR@5: 0.8571;
- three false-evidence facts, all in observed regression cases;
- zero forbidden-evidence, provenance, duplicate, stability or provider
  failures; and
- 3.657 seconds of aggregate evaluator time, excluding process compilation and
  model initialization.

The failing regression identifiers were
`regression_atlas_paraphrase_recovery`,
`regression_atlas_compound_federated`,
`regression_atlas_external_ai_policy` and
`regression_atlas_unrelated_injection`. Calibration success therefore does not
hide the known failures, and the current profile remains rejected.

Windows real-model evaluation is pending. A macOS result must never be used to
infer the behavior of the Windows artifacts.

## Generative-selector development observations

Two Gemma 4 E4B Q4 policies were evaluated on macOS arm64 on 2026-07-17. Both
used temperature zero, a 4,096-token context, a 30-second call timeout and the
same schema-v2 development corpus. Neither was evaluated on the disqualified
holdout or promoted into product code.

The strict exact-quote policy with candidate fingerprint
`967118cd5c6896878dd8d454a1317c2e0b150125720bcdb407116482cf55f5d0`
produced:

- regression, calibration and overall Recall@5: 1.00;
- two false-evidence facts across two regression cases;
- zero provider, forbidden-evidence, provenance, duplicate or stability
  failures;
- 60 model calls with p50 4.575 seconds, p95 14.170 seconds and maximum 17.756
  seconds; and
- 263.271 seconds of aggregate evaluator time plus 8.597 seconds of model
  startup.

A second policy required an explicit evidence-to-need mapping and preserved
more query qualifiers. Its candidate fingerprint was
`2eaa686a61bfca1d1fa0e42de35123bd4f91060bc15e77fe55c6910c7bf254b4`.
It retained Recall@5 of 1.00 but increased false evidence from two facts to three
and slowed calls to p50 6.177 seconds, p95 19.068 seconds and maximum 23.239
seconds. It was rejected and the first policy remains only the better
experimental baseline.

Generative selection recovered evidence missed by the current mMARCO gate in
these observed cases, but neither policy meets AirWiki's zero-false-evidence rule
or interactive latency budget. The result does not justify a Windows run,
because the macOS process alone used approximately 4.8 GiB of resident memory
during the second experiment. Production search remains unchanged.

## Exploratory relative-reranker observation

The mGTE paper reports strong multilingual reranking results and therefore
motivated a bounded GTE experiment
([Zhang et al., 2024](https://arxiv.org/abs/2407.19669)). The macOS arm64 run
used the INT8 ONNX conversion revision
`ee64367e35a2db0da46bb6497e13a18f8bd585cb`, whose model SHA-256 was
`ccf51dba7f8aa9205753761cfaa68c55f741792501463a3bf25d7e5bcdac7c35`.
The conversion was used for local research only; a distributable artifact would
have to be reproduced from the licensed upstream checkpoint.

The candidate generator already placed all ten expected development evidence
groups in its bounded per-source pools and all ten within the source top five.
Here, source-list Recall@5 is the fraction of required evidence groups that have
at least one authorized current candidate in the first five results of their
source list. GTE therefore could not improve this primary coverage result.
Across nine answerable per-source lists, using binary relevance over expected
evidence groups and macro-averaging the lists, it changed mean first-evidence
MRR@10 from 0.833 to 0.889 and mean nDCG@10 from 0.877 to 0.925. It worsened the
known Atlas paraphrase case from reciprocal rank 1.0 to 0.5 and the compound
case from 1.0 to 0.75. A release-mode diagnostic observed 1.001 seconds of
startup and per-source calls of p50 238 ms and p95 382 ms. These timings are not
end-to-end query latency.

The run rejected this GTE candidate under this small profile: it did not improve
evidence coverage and regressed known rankings. The one-off harness,
sanitized report and exact fixture manifest were not retained, so these numbers
are an exploratory local observation, not reproducible promotion evidence. A
future reranker must be evaluated by the versioned protocol below. Production
search remains unchanged.

## Exploratory reader and entailment observations

Two multilingual encoders fine-tuned on English SQuAD2 were explored with the
standard best-span-minus-CLS/null margin. Neither produced a scalar boundary
that separated the reviewed recovery evidence from relational hard negatives.
Their complete artifact identities and fixtures were not retained, so this
reader run is not reproducible evidence and must not be used for promotion. The
observation is consistent with cross-lingual unanswerable-QA results
([Gorodissky et al., 2025](https://aclanthology.org/2025.starsem-1.8/)) and with
the transfer gap reported by MLQA
([Lewis et al., 2020](https://aclanthology.org/2020.acl-main.653/)).

Two NLI controls used 17 reviewed positive and hard-negative
subject/relation/scope pairs; the English-only QNLI control used 15 translated
question/passage pairs. Human-written canonical claims isolated the NLI
verifiers from automatic question-to-statement conversion:

- [`MoritzLaurer/multilingual-MiniLMv2-L6-mnli-xnli`](https://huggingface.co/MoritzLaurer/multilingual-MiniLMv2-L6-mnli-xnli/tree/0a71e92a985b6e1ad1828cf67ce9c459639c1dca)
  revision `0a71e92a985b6e1ad1828cf67ce9c459639c1dca` used its official FP32 ONNX file
  with SHA-256
  `79f8cda2b1230585a95ea0514a6f1bd21c5c986ba0529bb3261213a3e195fa6e`;
  its lowest positive entailment margin was `-0.684`, while a negative reached
  `0.931`;
- [`MoritzLaurer/mDeBERTa-v3-base-mnli-xnli`](https://huggingface.co/MoritzLaurer/mDeBERTa-v3-base-mnli-xnli/tree/8adb042d524ecd5c26d3e3ba0e3fbcf7e2d0864c)
  revision `8adb042d524ecd5c26d3e3ba0e3fbcf7e2d0864c` used its official quantized ONNX
  file with SHA-256
  `27c39e884c14b03cf46cfc5485971b6db70ff330220d93dfe729c63fde43af0e`;
  its lowest positive margin was `-0.602`, below the highest negative margin of
  `-0.197`; and
- the English-only
  [`cross-encoder/qnli-distilroberta-base`](https://huggingface.co/cross-encoder/qnli-distilroberta-base/tree/7dd04ee0a6040c06fb381ad7edcb8585f4d937fd)
  revision `7dd04ee0a6040c06fb381ad7edcb8585f4d937fd` used the official arm64 INT8 file
  with SHA-256
  `4c3d2853f28c9a450054b40e02a683a10ab74076d726fb0ac9c8f19fbc27a3c3`;
  its lowest positive logit was `-5.657`, while an unsupported negative reached
  `3.021`.

No threshold separated positives from negatives for any control. Generic NLI
missed procedural paraphrase and cross-language relations; QNLI scored a
withdrawn, explicitly unapproved budget above a valid procedure answer. The 17
and 15 reviewed pair fixtures were not retained as a versioned corpus, so these
figures are exploratory local observations rather than reproducible promotion
evidence. They reject using these off-the-shelf thresholds in the current
profile; they do not reject a future multilingual model trained specifically on
QA entailment and relational hard negatives.

## Next research gate

The next promotion-oriented candidate must use a larger, licensed and traceable
development corpus before any production change, but more benchmarks are not
automatically better.
[`MIRACL`](https://aclanthology.org/2023.tacl-1.63/),
[`SQuAD2`](https://aclanthology.org/P18-2124/),
[`XQuAD`](https://aclanthology.org/2020.acl-main.421/),
[`MLQA`](https://aclanthology.org/2020.acl-main.653/),
[`ContractNLI`](https://aclanthology.org/2021.findings-emnlp.164/),
[`MuSiQue`](https://aclanthology.org/2022.tacl-1.31/) and
[`Natural Questions`](https://research.google/pubs/natural-questions-a-benchmark-for-question-answering-research/)
are a menu of possible transfer controls, not a required bundle. Each source
must fill a declared capability gap and preserve its license and attribution.
Documents, translations and multi-hop chains remain grouped before any split so
equivalent evidence cannot cross development, calibration and final holdout.

### Reviewed-evidence-anchor ablation (H-AWK-1)

**AirWiki hypothesis H-AWK-1:** a short reviewed factual claim bound to an exact
literal source anchor may be a safer and cheaper selection unit than asking the
same small local model to extract, rewrite and verify an answer from raw
passages at query time. The predicted mechanism is less query-time generation,
explicit subject/relation/scope/temporal identity and deterministic
counterevidence detection without weakening publication or authorization.

The `xtask`-only development ablation keeps the production publication and
hybrid-search path, but uses a pass-through relevance provider to retain its
bounded candidates. It compares:

- **A — raw passages:** the rejected two-call QA-entailment verifier;
- **B — reviewed claims:** one model call maps fixture-supplied atomic needs to
  opaque claim identifiers, while Rust validates the closed output and literal
  anchor binding; and
- **C — reviewed claims plus conflicts:** arm B followed by a deterministic
  scan for current, authorized claims with the same subject, relation, scope
  and temporal key, where opposite polarities apply to the same value or two
  affirmed values differ for an explicitly single-valued relation.

The selector receives only atomic needs and reviewed claim text. It never
receives expected groups, fixture roles, structured claim keys, authorization
state or expected outcomes. Rust applies the production publication, revision
and authorization filters before selection. No production schema, OKF profile,
SQLite contract, search API, LAN protocol or desktop behavior changes.

`fixtures/retrieval/reviewed-anchors-v1.json` is a manually authored synthetic
development overlay pinned to `search-quality-v2.json`. It was created after
the base questions and transfer cases had already been observed. The command
evaluates regression and calibration cases only; its holdout-shaped entries
validate structural binding and are not evaluated as a holdout. The fixture
also supplies atomic decomposition and expected groups, so the run does not
evaluate decomposition, claim-authoring quality, independent adjudication or
human review time. Because B and C change both indexed representation and
selector, any improvement belongs to the combined mechanism rather than to
anchors alone.

```bash
cargo run --locked -p xtask -- retrieval evaluate-reviewed-anchors \
  --data-root <verified-AirWiki-data-root> \
  --llama-server <verified-bundled-llama-server> \
  --model-id <catalog-model-id>
```

The command downloads nothing and writes a sanitized report under
`target/evals/`. Zero forbidden evidence, selective risk, failures and false
conflicts are safety vetoes. It also requires every expected outcome to be
correct, every required evidence group to remain represented in the effective
model-input pool, every expected conflict to be detected, coverage no worse
than arm A and at most one model call per case. Passing sets only
`mechanism_promising`;
`production_promotion_ready` remains false by design. Promotion requires a
separately authored and frozen domain-grouped holdout, independent human
adjudication, review-time measurement and installed macOS and Windows latency
and peak-memory evidence.

### In-process mini-graph hypothesis (H-AWK-2)

**AirWiki hypothesis H-AWK-2:** starting from BM25/E5 seeds and expanding only
reviewed internal OKF links for one bounded hop may nominate useful concepts
that vector or lexical similarity misses, while remaining small enough for a
workstation process. H-AWK-2 is evaluated at the candidate stage and does not
consume the rejected H-AWK-1 selector.

`fixtures/retrieval/mini-graph-v1.json` freezes synthetic hybrid rankings and
reviewed-link projections for a mechanistic ablation. Expectations score the
result but never enter the expansion algorithm. The four arms are:

- **C10:** the first 10 current, authorized hybrid candidates;
- **C32:** the same ranking widened to 32 candidates;
- **G1-out:** C10, one outgoing reviewed-link hop and then hybrid-rank
  backfill up to the same 32-candidate budget; and
- **G1-bidir:** C10, outgoing neighbors and then incoming neighbors of those
  same C10 seeds, followed by hybrid-rank backfill to 32.

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-mini-graph
```

The graph uses concept nodes only. Canonical concept UUIDs are sorted and
addressed through projection-local `u32` IDs; outgoing and incoming neighbors
are sorted, deduplicated boxed slices. The projection retains no titles, tags,
text, paths, queries or embeddings and adds no graph dependency. It accepts
only current concepts and reviewed internal links, ignores self-links and
rejects links across collections. The graph retains no permission state;
caller authorization and external-AI collection policy are applied before
seeding, traversal and hybrid backfill. One query may inspect at most 128
edges, never traverses a newly added node and returns at most 32 candidates.
The scale profile is fixed at 500 concepts and 2,000 directed links. Neo4j, a
daemon, a graph query language, similarity edges, entity edges, tags and
two-hop traversal remain deliberately out of scope.

The synthetic development gate requires at least two outgoing-link rescues
that C32 misses, a separate group rescued only by C32, higher outgoing recall
and support density no lower than C32 at the same candidate budget, one
backlink-only rescue, zero forbidden candidates in every arm, stable frozen
expectations, less than 1 MiB retained payload, release-profile build p95 below
50 ms and one-hop expansion plus hybrid-backfill p95 below 5 ms.
Candidate support density is not evidence precision: no selector or answer
generator runs in this experiment. Passing can set only
`synthetic_gate_passed`; `production_promotion_ready` is always false.

The frozen macOS arm64 release-profile observation on 2026-07-18 used evaluator
commit `97106026ceb594524155811783714dd499841d7e`, Rust 1.96.1 and fixture SHA-256
`60fc5cf1d3d99648b2755c8fa7af8eb7608b896ff2a6f2258b028a4337d012ba`.
C10 recovered 0 of 6 required groups, C32 recovered 1 of 6, G1-out recovered 3
of 6 and G1-bidir recovered 4 of 6. Outgoing links added three groups absent
from C32; backlinks added one more; and the wider-pool control retained one
graph-independent rescue. C32, G1-out and G1-bidir each evaluated 232 aggregate
candidates across the mixed-size cases, and all arms returned zero forbidden
candidates. At the 500-concept/2,000-link scale, the projection retained 48,168
payload bytes, built in 182 microseconds at p95 over 25 iterations and expanded
with hybrid backfill in 1 microsecond at p95 over 1,000 post-warm-up
iterations. These timings are one descriptive workstation observation; the
structural limits and release thresholds are the reproducible gates.

This synthetic mechanistic gate demonstrates that the bounded graph code and
controls behave as designed; it does not establish retrieval utility or change
production search. The rankings are manually frozen synthetic inputs rather
than captured BM25/E5 output, and the timing excludes OKF inspection, SQLite
loading and revalidation. The next gate must build the projection from a
healthy, fingerprinted `KnowledgeBundleView`, replay fresh real hybrid
rankings and compare C32/G1 on new domains. Any future integration must discard
a stale or unhealthy projection and retain SQLite publication, purpose, grant
and final hit revalidation as the disclosure authority.

#### Real-ranking development replay

The next development replay removes the hand-authored ranking from the
experiment. `fixtures/retrieval/mini-graph-real-development-v1.json` contains
documents, reviewed links, questions and evidence groups, but no rank, score or
candidate-position field. Its SHA-256 is
`e4d084dc9e7e84b9b9f82a157a298b052283251bbbb84f2736778e2038e056e8`.
The four visible development domains are observatory maintenance, museum
conservation, community irrigation and transit accessibility. Each contributes
an outgoing-link case, a backlink case and an unlinked direct-retrieval
control. Deterministic related distractors bring the corpus to 164 concepts so
the 32-candidate control is meaningful.

The evaluator publishes every source through SQLite and
`OkfPublicationMaterializer`, then obtains nodes and edges only from
`OkfBundleInspector`. A collection contributes to the projection only while it
is `Ready`, has zero health errors and retains the same bundle fingerprint
before and after ranking. Only resolved concept-to-concept internal links are
accepted. Every concept in this first replay has exactly one current chunk, so
concept nomination maps to a citable chunk without introducing a second
selection algorithm.

An evaluation-only `airwiki-core` feature exposes a content-free prefix of the
production BM25/E5/RRF ranking. It returns collection, concept, stable chunk and
revision identities, but no text, snippet or score. The normal desktop build
still uses the fixed 10-candidate relevance pool. In addition to C10, C32,
G1-out and G1-bidir, this replay evaluates **G1-sham**: the same source and
target degrees under the deterministic structural-sham contract described
below. This control tests whether any bounded expansion would appear to help.

For this visible v1 fixture, the development regression gate requires G1-bidir
to gain at least two evidence groups over both C32 and G1-sham, improve over
both controls in at least three of four domains, produce outgoing-link and
backlink rescues in at least two domains each, and regress no individual case
by losing an evidence group already covered by C32. These thresholds prevent a
one-case or one-domain result from
passing; they are not production-promotion criteria.

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-real-mini-graph \
  --embedding-snapshot <verified-multilingual-e5-small-snapshot>
```

The amended v1.1 macOS arm64 release-profile observation used multilingual E5 revision
`614241f622f53c4eeff9890bdc4f31cfecc418b3`. Across 12 cases and 20 required
evidence groups, C10 covered 12 groups, C32 covered 13, G1-out covered 16,
G1-bidir covered all 20 and G1-sham covered 13. Outgoing links produced three
groups absent from C32; backlinks added four more beyond outgoing expansion;
the sham graph added none over C32. Support density was 3.39% for C32, 4.17%
for G1-out and 5.21% for G1-bidir at the same aggregate 384-candidate budget.
The v1.1 replay rewired all eight sham edges and retained no original edge.
Both real and sham projections together retained 21,456 bytes. Healthy-bundle
inspection plus projection took 49,628 microseconds, and all 12 real rankings
plus expansions took 106,301 microseconds in aggregate. These are descriptive
single-workstation measurements, not percentile claims.

The real-ranking development gate passed, but
`production_promotion_ready` remains false. The corpus and questions were
visible while the mechanism was developed, every concept has only one chunk,
and no relevance reranker or top-five answer path was evaluated. Before a
production shadow integration, the direction and chunk-selection rule must be
frozen, a separately authored set of at least four new domains must be scored
once, and G1 must outperform both C32 and the degree-preserving sham without a
privacy, current-revision, latency or evidence-quality regression.

#### Sealed multichunk top-five holdout

The final H-AWK-2 candidate gate is frozen before its first execution.
`fixtures/retrieval/mini-graph-final-holdout-v1.json` was authored independently
from the development corpus and evaluator. Its SHA-256 is
`96c0efbe5acdfbe77f4c3c7bece68b7991d0066a721b54bb90b055ba02e9383d`.
It contains eight new fictional domains—four Spanish and four English—40 cases,
54 curated concepts with 162 sections, 62 reviewed links and 236 related
distractors. Each domain contributes an outgoing-link rescue, backlink rescue,
direct control, two-group compound question and plausible no-answer case. The
fixture contains no authored rank, score, arm or expansion-direction hint.

The frozen arms are:

- **B32:** the first 32 current chunks from production BM25/E5/RRF;
- **G1:** the first ten exact B32 chunks, one bidirectional hop over reviewed
  internal OKF links, then the unused B32 prefix as backfill; and
- **G1-sham:** the same procedure over a deterministic structural sham that
  preserves each concept's incoming and outgoing degree while minimizing the
  number of original links retained, with stable synthetic logical IDs as the
  only tie-breaker.

The first sealed command at commit `c80eeb7` aborted while constructing the
sham, before evaluator warm-up, case execution, ranking, scoring or report
creation. A cyclic target rotation is not defined for every valid directed
degree sequence: some original links may be mathematically unavoidable. No
question, ranking or metric was observed. Protocol amendment v1.1 therefore
keeps the fixture SHA, models, B32, G1, metrics, thresholds and gates unchanged,
but realizes the sham per collection with deterministic min-cost bipartite
b-matching. Capacity-one concept pairs prohibit duplicates, self-links and
cross-collection edges; costs prefer non-original links. Forced original links
remain, making the control more conservative rather than favoring G1. The
report records aggregate retained/rewired edges and unchanged collections.
This is a deterministic structural ablation, not a uniform sample from all
directed graphs with the same degrees; uniform directed-graph sampling requires
a different switch-chain protocol ([Erdos et al.](https://arxiv.org/abs/0912.3834)).

The graph nominates concepts, never evidence text. For every graph-only concept,
SQLite selects at most two current chunks by query-E5 cosine, with stable public
chunk identity as the tie-breaker. The ordered arm is capped at 32 chunks,
deduplicated by source and chunk content, classified by the existing pinned
mMARCO relevance provider, and truncated to the first five relevant hits. The
mMARCO component is a relevance filter, not a score-ordering reranker; arm order
therefore remains authoritative. Citation construction and final publication,
revision and collection-policy revalidation are exactly the local production
path. A provider error is a failed case, never a valid no-answer.
The evaluator rejects an expansion that exhausts its edge or candidate budget,
rather than accepting a subset whose membership could depend on runtime UUIDs.

The in-process projection remains deliberately small: projection-local `u32`
node IDs plus boxed outgoing and incoming adjacency slices. It stores no text,
titles, tags, paths, queries, embeddings or authorization state and introduces
no graph database, daemon or dependency. SQLite and the inspected OKF bundle
remain the authorities; an unhealthy, updating or fingerprint-changed bundle
invalidates the run.

The result is eligible only for a production **shadow** experiment when every
frozen condition passes:

- group Recall@5 at least 0.90 and citation precision at least 0.80;
- all no-answer cases return zero hits and no forbidden evidence is cited;
- no evidence group already covered by B32 is lost;
- macro-domain recall exceeds both B32 and G1-sham by at least 0.05, with
  improvement over both controls in at least five of eight domains;
- the deterministic paired-domain bootstrap 95% lower bound is above zero
  against each control, and MRR@5 is no worse than either control;
- provider failures and invalid or stale provenance are zero;
- real and sham graph payload together remain below 1 MiB, projection remains
  below one second, and G1 candidate assembly p95 remains below 25 ms. Candidate
  assembly includes one-hop nominee expansion plus query-E5/SQLite selection of
  graph-nominated chunks, before the shared mMARCO relevance filter;
- full G1 query p95 remains below three seconds and no worse than the greater
  of 110% of B32 p95 or B32 p95 plus 10 ms; and
- every bundle stays healthy with its original fingerprint.

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-final-mini-graph \
  --embedding-snapshot <verified-multilingual-e5-small-snapshot> \
  --relevance-snapshot <verified-mmarco-snapshot>
```

**Observed sealed result (v1.1, 2026-07-18).** The one permitted amended run
used the pinned multilingual E5 and mMARCO revisions above on macOS arm64. The
structural sham rewired 61 of 62 links, retaining one mathematically
unavoidable original link; no collection remained unchanged. The three arms
produced identical retrieval quality:

| Measure | B32 | G1 | G1-sham |
| --- | ---: | ---: | ---: |
| Group Recall@5 | 0.75 | 0.75 | 0.75 |
| Macro-domain recall | 0.75 | 0.75 | 0.75 |
| MRR@5 | 0.9375 | 0.9375 | 0.9375 |
| Citation precision | 1.00 | 1.00 | 1.00 |
| No-answer accuracy | 1.00 | 1.00 | 1.00 |
| Full-query p95 | 454 ms | 447 ms | 467 ms |

G1 improved over both controls in zero of eight domains, both paired-bootstrap
lower bounds were zero and it lost no group already covered by B32. Provider
failures, forbidden citations and stale or invalid provenance were all zero.
The graph pair retained 38,448 bytes and projected in 84 ms, but G1 candidate
assembly p95 was 123 ms against the frozen 25 ms limit.

The hypothesis therefore fails both the quality and candidate-assembly gates.
`shadow_eligible` and `production_promotion_ready` remain false, and production
search is unchanged. The result rejects this one-hop concept-nomination rule;
it does not establish that every future graph representation is ineffective.
Any materially different graph thesis requires a new development corpus and a
new independently sealed holdout rather than tuning against this result.

### H-AWK-1 development observation

The first macOS arm64 release-profile run on 2026-07-18 used Gemma 4 E4B Q4 at
revision `7edc6763a77bbca236126a361613b834c5ea0f7a`, llama.cpp `b9946`, four
threads and reviewed-anchor fixture SHA-256
`21d6fc5623b388a8c8ed2659c48e8faaf276fddc66ad4b1e88dbc1547f69ce8c`.
The candidate fingerprint was
`a303b79c080095483d52de072cb36642ba5f5b7731a5d71654c8a13783a37e80`.

All three arms made all 10 required evidence groups available in their
effective selection pools; this measures candidate availability, not final
accepted-evidence recall. The raw two-call arm covered 1 of 9 positive atomic
needs, returned 4 of 11 correct outcomes and returned `unavailable` in 7 cases.
Reviewed claims covered all 9 positive needs and made 13 per-case evidence
selections, of which 10 were supportive (precision 10/13). They produced four
selective-risk cases, including one fixture-forbidden semantic selection; this
was not an authorization leak, because every pre-model policy exclusion held.
Of four expected no-answer cases, the selector incorrectly accepted one and
returned `unavailable` for the other three. Deterministic conflict detection
found the one expected conflict with no false conflict, reduced selective-risk
cases from four to three, retained 10/13 evidence precision and produced 5 of
11 correct outcomes.

The reviewed-claim arm used one model call per case and had per-case evaluator
p50/p95 of 2.970/6.457 seconds, compared with 7.615/11.351 seconds for the raw
arm. These distributions are not comparable speed evidence: only one raw-arm
case reached verification, producing 12 calls across 11 cases, while most raw
cases failed earlier. Human review time and peak memory were not measured.

The current one-call H-AWK-1 selector therefore **fails** the development gate.
Its coverage and conflict result do not compensate for false evidence,
no-answer failures and low outcome correctness. Production search remains
unchanged. This rejects the measured selector contract, not reviewed anchors as
a future representation; another candidate must establish a safer selection
mechanism on new development domains before graph expansion can consume it.

The primary measures have fixed meanings:

- candidate Recall@10 is the fraction of required atomic needs with at least
  one authorized current candidate in the first ten results of its source list;
- atomic-need recall is the fraction of required needs covered by verified
  evidence in the verifier's final accepted evidence set;
- evidence precision is the number of final evidence items whose mapping to at
  least one atomic need is judged supportive, divided by every item in that set;
- query-level selective risk is the fraction of accepted queries whose final
  evidence set contains at least one unsupported evidence-to-need mapping;
- coverage is the fraction of eligible queries for which every required atomic
  need is verified and the evidence set is accepted; an incomplete compound
  query must abstain; and
- the risk-coverage curve evaluates query-level risk as a frozen decision-score
  threshold changes.

The acceptance threshold is chosen only from the separate calibration split.
Conformal risk control is eligible as a later calibration method only if the
versioned protocol states and satisfies its exchangeability and monotone-loss
assumptions; otherwise AirWiki reports empirical risk-coverage without a formal
guarantee
([Angelopoulos et al., 2024](https://arxiv.org/abs/2208.02814)).

Documents, translations and multi-hop chains are grouped to prevent evident
split leakage; that design choice does not by itself establish statistical
independence. Confidence intervals and resampling units must follow the metric
and experimental design. Standard and bootstrap intervals are established
options for common IR measures
([Soboroff, 2014](https://www.nist.gov/publications/computing-confidence-intervals-common-ir-measures)).
If, and only if, the accepted grouped evaluation units can be modeled as i.i.d.
Bernoulli trials, then zero observed false-evidence events among `n` units gives
the illustrative one-sided 95% upper bound `1 - 0.05^(1/n)`: 17 such clean units
would still permit about 16.2% risk, and 299 are required before the bound falls
below 1%. The 17 exploratory NLI pairs above do not establish those assumptions.
Until a frozen candidate passes a fresh domain-separated holdout, AirWiki must
describe all such runs as development evidence, not a safety guarantee.

## mMARCO score-order ablation

AirWiki's pinned mMARCO cross-encoder currently classifies each of the first ten
hybrid candidates as relevant or irrelevant, after which search preserves the
original BM25/E5/RRF order. This is intentionally conservative, but it does not
exercise the model as a reranker. The model card and the standard cross-encoder
contract instead order passages by descending score
([model card](https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1),
[Sentence Transformers](https://www.sbert.net/docs/cross_encoder/usage/usage.html));
the original BERT passage-ranking formulation likewise sorts passage relevance
estimates to construct the final list
([Nogueira and Cho, 2019](https://arxiv.org/abs/1901.04085)).

### H-RR1 frozen development contract

H-RR1 asks whether AirWiki loses useful evidence by discarding score order. It
compares two outputs from **one** mMARCO inference over the **same** ordered set
of at most ten authorized, current candidates:

- **A0:** apply the existing relevance mask and retain hybrid RRF order; and
- **A1:** apply the identical mask, then stable-sort relevant candidates by
  descending mMARCO score, breaking equal-score ties by their original RRF
  position.

The evaluator may expose a permutation of candidate indices, but never raw
scores, questions, passage text, snippets or local identities. A0 and A1 must
have identical candidate identities and relevance decisions. Both arms pass
through the same citation construction and final publication/policy
revalidation. A concurrent publication or authorization change invalidates the
paired case rather than favoring either arm.

The first run is a mechanism diagnostic on already visible development data,
not promotion evidence. H-RR1 advances to a new licensed, domain-separated
development corpus only if A1 improves at least one answerable case, loses no
evidence group found by A0, does not reduce MRR@5, and does not increase false
or forbidden evidence. No-answer accuracy must remain 100%; provider,
provenance and stale-revision failures must remain zero; stable sorting must add
less than one millisecond p95 outside the shared inference. Otherwise H-RR1 is
rejected without changing production.

If H-RR1 advances, candidate depths 10, 16 and 32 become a separate H-RR2
experiment on the new development corpus. Pool depth, score fusion and
abstention thresholds must not be changed in H-RR1. A later production proposal
requires a frozen candidate and a newly authored holdout that has not been used
to select any of those choices. Results must be stratified by Spanish-to-Spanish,
English-to-English and both cross-language directions; multilingual training
does not establish equivalent cross-language calibration.

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-rerank-order \
  --embedding-snapshot <verified-multilingual-e5-small-snapshot> \
  --relevance-snapshot <verified-mmarco-snapshot>
```

**Observed visible-development result (2026-07-18).** The diagnostic used the
pinned multilingual E5 and mMARCO snapshots on macOS arm64. It evaluated five
eligible local cases and 39 candidates with exactly five shared relevance calls
and no provider, provenance, forbidden-evidence or partial-result failures.
A0 and A1 were identical: each found two of three evidence groups, for 0.667
Recall@5 and MRR@5, with 1.00 citation precision and 1.00 no-answer accuracy.
A1 improved zero answerable cases and lost zero groups. Stable score ordering
was below the timer's microsecond resolution.

H-RR1 is therefore rejected and production search remains unchanged. In the
missed Atlas case the fixed relevance mask removed every candidate before order
could matter; the other answerable cases retained at most one result. Pool-depth
H-RR2 must not proceed from this result. The next experiment must separate
ranking from abstention on a new licensed development corpus instead of tuning
the existing threshold against these five visible cases.

## Absolute abstention calibration (H-CAL1)

H-CAL1 isolates the remaining decision that H-RR1 could not exercise: the
absolute mMARCO cutoff. The cross-encoder returns logits, not calibrated
probabilities. Applying a sigmoid or another monotone mapping would not change
ranking or the candidate set selected by one corresponding cutoff, so this
first experiment deliberately avoids a fitted probability model. Platt scaling,
isotonic regression and conformal risk control remain deferred until AirWiki has
enough independent grouped calibration queries to justify their additional
assumptions and parameters.

The new development corpus contains 48 queries in 40 document and translation
families, with exactly ten candidates per query. XQuAD contributes parallel
English and Spanish questions and passages in all four query-to-passage language
directions. SQuAD 2.0 contributes paired answerable and source-native impossible
questions over the same candidate pools. Complete documents, parallel
translations and every document used as a distractor remain within one split.
Twenty-four queries select the candidate rule in `training`; 24 queries held
out from training across 20 grouped families are read once as `calibration`.
Neither split is a promotion holdout, and all third-party text remains in
ignored, hash-verified local source artifacts.

One shared mMARCO inference produces the following content-free arms:

- **B0:** the current `evidence-v1` decisions in the blinded input order;
- **T1:** replace the fixed zero floor with one absolute logit cutoff selected
  on `training`, retain the current 3.6-logit relative window and blinded input
  order; and
- **T1-score:** use the identical T1 decisions but stable-sort them by descending
  mMARCO score, with blinded input position as the tie-breaker.

The cutoff candidates are the finite training scores plus the fail-closed
abstain-all boundary. Selection maximizes complete-query coverage subject to
zero returned hard negatives and zero accepted support-absent queries; ties use
the higher cutoff. A query is complete only when its returned top five contains
every required support. Candidate rows are correlated and never counted as
independent calibration units. Raw scores stay in memory and must not appear in
logs or reports; only the selected cutoff, fingerprints and aggregate metrics
may be persisted.

The closed manifest and source ledger live under
`resources/evaluation/retrieval-rerank-abstention-development-v1/`. The real
model run is:

```bash
cargo run --release --locked -p xtask -- retrieval evaluate-rerank-calibration \
  --source-root <verified-corpus-root> \
  --relevance-snapshot <verified-mmarco-snapshot>
```

The calibration gate requires zero provider or provenance failures, 100%
support-absent accuracy, zero returned hard negatives, no loss of a query
covered by B0, at least 0.05 absolute improvement in complete-query coverage
over B0, improvement in at least two language/source strata, non-regressing
MRR@5 and less than one millisecond p95 decision overhead. Passing authorizes
only a newly authored sealed end-to-end holdout and a Windows replay of the same
frozen rule. It does not authorize `evidence-v2`, a production threshold or a
user-facing confidence value.

**Observed grouped-calibration result (2026-07-18).** The first frozen macOS
arm64 release-profile run used corpus manifest SHA-256
`24ce3ab44361b483176d90b1344d2507d95ddb70da788545d3d85588f094aa6b`
and mMARCO revision `1427fd652930e4ba29e8149678df786c240d8825`. Training
selected an absolute logit cutoff of `6.1731644`; this value is an internal
model score, not a probability or user-facing confidence.

| Calibration measure | B0 | T1-score |
| --- | ---: | ---: |
| Complete-query coverage | 0.75 | 0.4167 |
| Support Recall@5 | 0.9167 | 0.4167 |
| MRR@5 | 0.8611 | 0.4167 |
| No-answer accuracy | 0.6667 | 0.9167 |
| Citation precision | 0.6111 | 0.8333 |
| Returned hard negatives | 7 | 1 |

T1-score lost four queries completed by B0, improved no frozen stratum and
still returned one hard negative on a support-absent query. Its complete-query
coverage changed by `-0.3333`, well below the required `+0.05`. The decision
step itself remained below one microsecond p95 outside the shared inference;
model inference was 475 ms p95 over the 48 grouped queries on this workstation.

H-CAL1 is therefore rejected. `sealed_holdout_authorized` and
`production_promotion_ready` remain false, and production search is unchanged.
The result shows that one global raw-logit cutoff does not separate useful from
misleading evidence reliably enough on this bilingual grouped corpus. Further
work requires a materially different abstention or evidence-composition thesis;
the calibration split must not be reused to tune this cutoff.
