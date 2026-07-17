# Retrieval answerability development corpus v1

This manifest is the traceable seed for AirWiki's next retrieval experiment. It
does not change production search and is not a quality benchmark by itself. It
pins source repositories, immutable revisions, artifact sizes, SHA-256 hashes,
licenses, grouped development splits and a small set of source-native record
identifiers. The third-party passages and questions are not committed,
packaged, logged or copied into evaluation reports.

Validate the committed metadata without downloading any dataset:

```bash
cargo run --locked -p xtask -- retrieval corpus validate
```

The command validates the closed schema and privacy-safe invariants. It does not
download or hash local artifacts. A later importer will place verified inputs
under ignored `target/` state and must compare both size and SHA-256 before
reading a record.

## Source ledger

### SQuAD 2.0

- Repository: [rajpurkar/SQuAD-explorer](https://github.com/rajpurkar/SQuAD-explorer)
- Revision: `eee5fdbf62f8613a7812b03419e6b29617b74fd1`
- Dataset license: [CC BY-SA 4.0, as declared by the official SQuAD page](https://raw.githubusercontent.com/rajpurkar/SQuAD-explorer/eee5fdbf62f8613a7812b03419e6b29617b74fd1/views/index.pug)
- Role: answerable examples and adversarial, lexically plausible no-answer
  examples described by the
  [SQuAD 2.0 paper](https://aclanthology.org/P18-2124/).

The repository's root software license is not the dataset license. The manifest
records the dataset's CC BY-SA 4.0 declaration.

### XQuAD English and Spanish

- Repository: [google-deepmind/xquad](https://github.com/google-deepmind/xquad)
- Revision: `7d30520c717524000f0d9d2f9c10a069acd9d285`
- Dataset license: [CC BY-SA 4.0](https://raw.githubusercontent.com/google-deepmind/xquad/7d30520c717524000f0d9d2f9c10a069acd9d285/README.md)
- Role: parallel English/Spanish answerability transfer described by the
  [XQuAD paper](https://aclanthology.org/2020.acl-main.421/).

XQuAD is derived from SQuAD 1.1 and contains answerable questions. It is not a
source of SQuAD 2.0-style unanswerable examples. Parallel translations remain
in the same group and split so they cannot leak across calibration boundaries.

### ContractNLI

- Repository: [stanfordnlp/contract-nli](https://github.com/stanfordnlp/contract-nli)
- Revision: `eced6528dd3c1d14d73f9a87df8f7bdbc03126f9`
- Dataset license: [CC BY 4.0](https://raw.githubusercontent.com/stanfordnlp/contract-nli/eced6528dd3c1d14d73f9a87df8f7bdbc03126f9/index.md)
- Access terms: [official download page](https://stanfordnlp.github.io/contract-nli/#download)
- Role: document-level entailment, contradiction, missing relations, scope and
  negation, as introduced in the
  [ContractNLI paper](https://aclanthology.org/2021.findings-emnlp.164/).

The official artifact is the complete ZIP, including its own license and terms.
AirWiki does not redistribute it. A maintainer must review and accept the
upstream terms before downloading it.

## Split policy

- `training` may be used to implement the experimental verifier.
- `calibration` may be used to choose a selective decision threshold after the
  verifier structure is frozen.
- No final holdout is included. Fresh document groups will be selected only
  after the candidate, prompt, features and thresholding method are frozen.
- A complete document, article and parallel translation family is one group.
  No group may cross training and calibration.

Selection and need identifiers are local AirWiki identifiers.
`upstream_record_id` retains the source-native question or annotation ID, while
`artifact_path`, optional archive `member_path` and `document_id` disambiguate
reused IDs across files, languages and contracts. In SQuAD and XQuAD,
`document_id` is `data_<index>` and `segment_id` is `paragraph_<index>`; both
indices are zero-based and address the ordered arrays in the pinned JSON. In
ContractNLI, `document_id` is the source-native numeric document ID and
`segment_id` is `span_<index>` from the selected archive member. These locators
avoid storing or normalizing document titles.

A support is a passage expected to justify the need. A hard negative is a
different passage that does not contain the required support and must be
rejected for the declared reason. The SQuAD and XQuAD negatives deliberately
use another paragraph rather than another question from the support paragraph.
These seed selections exist to validate the representation; a promotion
evaluation needs a larger reviewed corpus and grouped uncertainty estimates.

## Scientific boundary

The candidate design follows a staged hypothesis:

1. retrieve a fixed candidate pool with BM25, multilingual E5 and RRF;
2. identify each atomic information need;
3. extract a literal candidate answer span;
4. construct a complete question-answer claim; and
5. accept evidence only when the passage supports that claim, otherwise abstain.

The design is motivated by selective QA and QA-entailment research, but cited
results do not establish transfer to AirWiki data. Promotion requires a fresh
grouped holdout, selective risk and coverage with uncertainty, and the existing
authorization, provenance, latency and memory gates.
