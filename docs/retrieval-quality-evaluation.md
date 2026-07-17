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

Answerability is a separate selective-prediction problem. SQuAD 2.0 established
that a system must abstain when a similar-looking passage does not state an
answer, and selective-QA research shows that raw model confidence is unreliable
under domain shift
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

Question-answering entailment is closer to AirWiki's safety boundary than
generic semantic similarity. QA-entailment research evaluates whether a passage
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

These runs show that generative selection can recover evidence missed by the
current mMARCO gate, but neither policy meets AirWiki's zero-false-evidence rule
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

The run falsified the rationale for promoting GTE on this small profile: it did
not improve evidence coverage and regressed known rankings. The one-off harness,
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

The next candidate must be developed against a larger, licensed and traceable
corpus before any new production code. The development profile will sample
ranking cases from MIRACL and answerability/support cases from SQuAD2, XQuAD,
MLQA, ContractNLI, MuSiQue and Natural Questions, while preserving each
dataset's license and attribution requirements
([MIRACL](https://arxiv.org/abs/2210.09984),
[XQuAD](https://aclanthology.org/2020.acl-main.421/),
[ContractNLI](https://aclanthology.org/2021.findings-emnlp.164/),
[MuSiQue](https://aclanthology.org/2022.tacl-1.31/),
[Natural Questions](https://research.google/pubs/natural-questions-a-benchmark-for-question-answering-research/)).
Documents, translations and multi-hop chains must remain grouped before any
split so equivalent evidence cannot cross development, calibration and final
holdout.

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
