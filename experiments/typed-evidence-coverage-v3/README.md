# Typed-evidence coverage v3 package contract

The normative protocol is
[`docs/typed-evidence-coverage-v3.md`](../../docs/typed-evidence-coverage-v3.md).
This directory contains only the frozen public contract. Real inputs, labels,
candidate outputs and reports remain outside the repository.

## Candidate interface

The candidate is one executable with two modes:

```text
candidate source
candidate question
```

It reads the complete UTF-8 JSONL input from stdin and writes only UTF-8 JSONL
to stdout. It must write nothing to stderr or the working directory. Record and
set order are ignored after validation; semantic content, exact spans and IDs
are not changed. Each mode runs twice in a fresh directory and must produce
byte-identical canonical records before a receipt is installed.

Run the candidate on the matching physical platform:

```bash
cargo run --locked -p xtask -- typed-evidence-v3 run \
  --package <private-package> \
  --candidate <frozen-executable> \
  --platform macos_arm64
```

Use `windows_x64` on Windows. A successful run refuses to overwrite an existing
platform receipt.

## Blind inputs

Source input:

```json
{"source_id":"src_0123456789abcdef0123456789abcdef","title":"Navigation only","heading":"Navigation only","text":"Atlas owner is Ana."}
```

Question input:

```json
{"question_id":"qry_fedcba9876543210fedcba9876543210","question":"Who owns Atlas?"}
```

Source extraction:

```json
{"source_id":"src_0123456789abcdef0123456789abcdef","status":"resolved","claims":[{"subject":{"normalized":"atlas","spans":[{"start":0,"end":5}]},"relation":{"normalized":"responsible_party","spans":[{"start":6,"end":11}]},"object_type":{"normalized":"person","spans":[{"start":15,"end":18}]},"object_value":{"normalized":"ana","spans":[{"start":15,"end":18}]},"qualifiers":[],"polarity":"positive","lifecycles":["current"],"provenance":"direct","support_spans":[{"start":0,"end":19}]}],"reason_code":null}
```

Question extraction:

```json
{"question_id":"qry_fedcba9876543210fedcba9876543210","status":"resolved","needs":[{"subject":{"normalized":"atlas","spans":[{"start":9,"end":14}]},"relation":{"normalized":"responsible_party","spans":[{"start":4,"end":8}]},"requested_object_types":[{"normalized":"person","spans":[{"start":0,"end":3}]}],"required_qualifiers":[],"allowed_polarities":["positive"],"required_lifecycles":["current"],"allowed_provenances":["direct","attributed"],"support_spans":[{"start":0,"end":15}]}],"reason_code":null}
```

For an unresolved source, emit empty `claims` and one field-guide reason. For
an unresolved question, emit empty `needs` and one reason. Resolved records
must have at least one claim or need and no reason.

## Private package preparation

Before either candidate run, create:

```text
manifest.json
inputs/sources.jsonl
inputs/questions.jsonl
```

`manifest.json` is:

```json
{
  "schema_version": 1,
  "experiment_id": "typed_evidence_coverage_v3",
  "contract_sha256": "<sha256 of contract.json>",
  "candidate_revision_sha256": "<frozen candidate source revision sha256>"
}
```

After both platform runs, isolated reviewers add canonical source and question
gold files, a review receipt binding every input/label/key hash, and the private
scoring key. The scoring key contains cases with this shape:

```json
{
  "case_id": "promotion_case_1",
  "domain_id": "promotion_domain_1",
  "split": "promotion",
  "question_id": "qry_fedcba9876543210fedcba9876543210",
  "tags": ["direct"],
  "source_pools": [
    {
      "pool_id": "local",
      "candidate_source_ids": [
        "src_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "src_0123456789abcdef0123456789abcdef"
      ]
    }
  ],
  "relevant_source_ids": ["src_0123456789abcdef0123456789abcdef"],
  "expected_groups": [["src_0123456789abcdef0123456789abcdef"]],
  "allowed_support_source_ids": [],
  "forbidden_source_ids": []
}
```

The complete key has `schema_version: 1` and a `cases` array. The validator
enforces split isolation, pool limits, label disjointness, complete expected
groups and the frozen domain/case composition.

Validate and score only after blind review is complete:

```bash
cargo run --locked -p xtask -- typed-evidence-v3 validate-package \
  --package <private-package>

cargo run --locked -p xtask -- typed-evidence-v3 score \
  --package <private-package> \
  --report <private-report.json>
```

The scorer writes a report even when a quality gate fails, then exits
unsuccessfully. Do not commit the package or report.
