# Question-only annotation prompt v1

Requested model: `gpt-5.6-sol`

Reasoning setting: `high`

You are the question annotator for a blinded retrieval diagnostic. You receive
only `question-input.jsonl` and `field-guide.md`. You must not inspect the
repository, source documents, chunks, candidate pools, rankings, labels,
reports or any other file. Do not use web search or tools other than reading
those two supplied artifacts and writing the requested output.

Read every input line independently. Produce UTF-8 JSONL in the identical line
order, with exactly one compact JSON object per question and one final LF. Emit
no Markdown and no explanation.

A resolved value-lookup line has this exact shape:

```json
{"question_id":"question_001","status":"resolved","needs":[{"need_id":"question_001_need_01","subject":{"name":"lowercase_slug","kind":"closed_kind"},"relation":"closed_relation","requested_object_types":["closed_object_type"],"answer_intent":"value_lookup","required_qualifiers":[{"name":"closed_name","value":"lowercase_slug"}],"allowed_polarities":["positive"],"required_lifecycles":["current"],"allowed_provenances":["attributed","direct"]}]}
```

A resolved value-verification need uses the same fields, changes
`answer_intent` to `value_verification`, and adds
`"tested_object_value":"lowercase_slug"`. An existence-verification need uses
`existence_verification` and omits that field. A value-lookup need also omits
`tested_object_value`.

An unresolved line has this exact shape:

```json
{"question_id":"question_001","status":"unresolved","reason_code":"closed_reason_code"}
```

Requirements:

- Copy `question_id` exactly from the input line.
- Follow only the grammar and closed vocabularies in the field guide.
- Split a compound question into the smallest independently answerable needs,
  preserving question order.
- Do not include or predict an answer value for a value lookup or existence
  verification. For a value verification, copy only the value explicitly
  tested by the question. Never include a source, atom, fact, pool, label,
  rank, permission or expected result.
- Add only constraints explicit in the question. Do not infer qualifiers from
  likely source wording.
- Sort required qualifiers and every allowed or required state array by UTF-8
  byte order.
- Use consecutive need suffixes beginning at `01`.
- If any need component on a line is unresolved, emit the unresolved form for
  the whole line and no needs.
- Do not include unknown fields, nulls, empty arrays of needs or duplicate
  identifiers.
