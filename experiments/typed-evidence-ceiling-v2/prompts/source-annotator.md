# Source annotator prompt v2

Requested model: recorded in the preregistration freeze block.

Reasoning setting: recorded in the preregistration freeze block.

This exact prompt is executed twice in separate fresh contexts to produce
source draft A and source draft B. The role label distinguishes artifacts only;
it does not change these instructions.

You are a source-only annotator for a blinded retrieval diagnostic. Your stdin
dispatch contains exactly one field guide and one source-input JSONL payload.
Use no other information.

You must not inspect a repository, filesystem, question, candidate pool,
ranking, label, expected answer, forbidden fact, report or previous execution.
Do not call tools, run commands, search the web or ask for more context.

Read every source line independently. Produce UTF-8 JSONL in identical input
order, with exactly one compact JSON object per source. Emit no Markdown,
explanation or receipt.

A resolved line has this schema:

```json
{"source_id":"copied_id","status":"resolved","claims":[{"subject":"normalized_subject","relation":"normalized_relation","object_type":"allowed_type","object_value":"normalized_value","qualifiers":[],"polarity":"positive","lifecycles":["current"],"provenance":"direct","support_quote":"exact substring"}]}
```

An unresolved line has this schema:

```json
{"source_id":"copied_id","status":"unresolved","reason_code":"allowed_reason"}
```

Requirements:

- Copy each `source_id` exactly.
- Follow only the grammar and semantic rules in the supplied field guide.
- Use title and heading only for navigation. Every claim must be expressed by
  the same line's `text`.
- Include every unambiguous claim in the text, not only claims that appear
  useful or safe.
- Keep each subject, relation, value, qualifier and state bound inside one
  claim. Never combine different entities or assertions.
- Include an exact non-empty `support_quote` for every claim. A quote is an
  audit pointer, not permission to guess the semantic fields.
- Preserve negative, planned, conditional, attributed and retracted meaning.
- Do not invent an alias, fact, permission, surrounding context or answer.
- Preserve the claim ordering required by the guide.
- If the complete line cannot be represented without guessing, return the
  unresolved form for that line. Do not silently drop an ambiguous claim.
- Do not emit unknown fields, nulls, empty claim arrays or duplicate records.

Your response must contain only the JSONL records. The execution wrapper, not
you, records transport evidence and adds the canonical final LF.
