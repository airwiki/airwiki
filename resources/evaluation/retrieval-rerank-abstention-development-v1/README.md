# Retrieval rerank and abstention development corpus v1

This manifest is development and rejection evidence for the H-CAL1 reranking
and abstention experiment. It is not a benchmark, a promotion gate or evidence
that a threshold generalizes to AirWiki knowledge. It contains training and
calibration groups only. A fresh, sealed holdout must be selected after the
candidate features, calibration method and decision policy are frozen.

Only immutable source metadata, source-native record identifiers and
content-free locators are versioned. Questions, answers and passages remain in
ignored local artifacts and must never be copied into the repository, packages,
logs or evaluation reports.

Validate the closed manifest without reading or downloading corpus content:

```bash
cargo run --locked -p xtask -- retrieval calibration-corpus validate
```

After placing the three pinned files below an ignored local source root, verify
their hashes, locators, parallel records and candidate roles with:

```bash
cargo run --locked -p xtask -- retrieval calibration-corpus verify \
  --source-root <source-root>
```

The expected local layout is:

```text
<source-root>/
  squad_v2/dataset/train-v2.0.json
  xquad_en_es/xquad.en.json
  xquad_en_es/xquad.es.json
```

The verifier does not download or persist dataset content and emits only
content-free counts and fingerprints.

## Frozen development shape

The manifest contains exactly:

| Dimension | Training | Calibration | Total |
| --- | ---: | ---: | ---: |
| Cases | 24 | 24 | 48 |
| Document/translation families | 20 | 20 | 40 |
| Candidates | 240 | 240 | 480 |
| Support-present cases | 12 | 12 | 24 |
| Support-absent cases | 12 | 12 | 24 |

Every case has ten candidates. The language cells contain 24 English-to-English
cases, including the sixteen SQuAD cases, and eight cases in each of
English-to-Spanish, Spanish-to-English and Spanish-to-Spanish. Each XQuAD
direction contains equal support-present and support-absent cases.

`expectation` describes whether the constructed candidate pool contains
support. `source_expectation` independently preserves the upstream label. Thus
an XQuAD question remains source-answerable while a deliberately support-absent
pool is expected to abstain.

### XQuAD construction

- `data_1` through `data_16` are training families; `data_17` through
  `data_32` are calibration families. English and Spanish translations of one
  `data_N` are always one family and never cross the split boundary.
- The eight-case pattern is repeated within each split:
  English-to-English present, English-to-English absent,
  Spanish-to-Spanish present, Spanish-to-Spanish absent,
  Spanish-to-English present, Spanish-to-English absent,
  English-to-Spanish present and English-to-Spanish absent.
- A support-present pool contains `paragraph_0` through `paragraph_4` of the
  query document plus the same five paragraphs of the next document in the
  split. Only the parallel `paragraph_0` is support.
- A support-absent pool contains `paragraph_1` through `paragraph_4` of the
  query document, all five paragraphs of the next document and `paragraph_0`
  of the second-next document. Wraparound stays inside `1..16` or `17..32`.

### SQuAD 2.0 construction

- Training uses `data_5`, `data_15`, `data_16` and `data_17`; calibration uses
  `data_18`, `data_20`, `data_21` and `data_22`.
- Each family contributes one source-answerable and one source-unanswerable
  case from `paragraph_0`.
- Both cases use `paragraph_0` through `paragraph_9`. The answerable case marks
  `paragraph_0` as support; the impossible case treats all ten passages as
  related but unsupported candidates.

No family, query, parallel translation or candidate document crosses from
training into calibration.

## Source ledger

### SQuAD 2.0

- Repository: [rajpurkar/SQuAD-explorer](https://github.com/rajpurkar/SQuAD-explorer)
- Revision: `eee5fdbf62f8613a7812b03419e6b29617b74fd1`
- Dataset license: CC BY-SA 4.0, as declared by the official SQuAD page at the
  pinned revision
- Artifact: `squad_v2/dataset/train-v2.0.json`
- Size: `42123633` bytes
- SHA-256: `68dcfbb971bd3e96d5b46c7177b16c1a4e7d4bdef19fb204502738552dede002`

### XQuAD English and Spanish

- Repository: [google-deepmind/xquad](https://github.com/google-deepmind/xquad)
- Revision: `7d30520c717524000f0d9d2f9c10a069acd9d285`
- Dataset license: CC BY-SA 4.0
- English artifact: `xquad_en_es/xquad.en.json`, `609383` bytes,
  SHA-256 `e4c57d1c9143aaa1c5d265ba5987a65f4e69528d2a98f29d6e75019b10344f29`
- Spanish artifact: `xquad_en_es/xquad.es.json`, `684322` bytes,
  SHA-256 `dcbae93ec3a9f4b9e78fd834a171d6f96c1a875e10e15b7530b7e4ef4971e37e`

## Semantic contamination review

The first source record was used unless its answer recurred in, or its question
could be answered by, a candidate declared negative. All support-absent and
source-impossible pools were read semantically. Every exact-answer collision in
support-present negative candidates received a targeted semantic review.

The following source-native locators replace the initially requested first
record while retaining the same source document, split and candidate pool:

| Source family | Role | Initial record | Selected record | Reason |
| --- | --- | --- | --- | --- |
| XQuAD `data_3` | present | `56dfa0d84a1a83140091ebb7` | `56dfa0d84a1a83140091ebb8` | Initial answer recurred in a negative candidate. |
| XQuAD `data_18` | absent | `5725edfe38643c19005ace9f` | `5725edfe38643c19005acea2` | Initial answer recurred in the same-document negative pool. |
| XQuAD `data_29` | present | `57273a465951b619008f86ff` | `57273a465951b619008f8701` | Initial answer recurred across same-document negatives. |
| XQuAD `data_31` | present | `5727aec03acd2414000de991` | `5727aec03acd2414000de994` | Initial answer recurred in a same-document negative. |
| XQuAD `data_32` | absent | `5727c94bff5b5019007d954a` | `5727c94bff5b5019007d954b` | Initial answer recurred across same-document negatives. |
| SQuAD `data_5` | answerable | `56cdaf0362d2951400fa6808` | `56cdaf0362d2951400fa6809` | Initial answer recurred in negative candidates. |
| SQuAD `data_16` | answerable | `56dc686a14d3a41400c26867` | `56dc686a14d3a41400c2686a` | Initial answer recurred broadly; the replacement's lexical-only collision was adjudicated as unrelated. |
| SQuAD `data_18` | answerable | `56dd20d966d3e219004dabf3` | `56dd20d966d3e219004dabf4` | Another candidate directly supported the initial question; the replacement's lexical-only occurrences were adjudicated as unrelated. |
| SQuAD `data_20` | answerable | `56ddb46c9a695914005b958e` | `56ddb46c9a695914005b9590` | Initial answer recurred in a negative candidate. |
| SQuAD `data_21` | answerable | `56ddd7179a695914005b9600` | `56ddd7179a695914005b9601` | Initial aliases recurred in later candidates. |
| SQuAD `data_16` | impossible | `5ad5f6585b96ef001a10af32` | `5ad5f6585b96ef001a10af33` | The initial question was semantically answerable from the pool. |
| SQuAD `data_20` | impossible | `5a6afdb2a9e0c9001a4e9e38` | `5a6afdb2a9e0c9001a4e9e39` | The initial question was semantically answerable from later candidates. |

This review reduces known label contamination but does not make the corpus an
independently adjudicated benchmark. Calibration results can reject H-CAL1 or
freeze a candidate for a later holdout; they cannot promote production search.
