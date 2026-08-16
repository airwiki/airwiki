# Typed-evidence field guide v3

This grammar belongs only to the typed-evidence coverage experiment. It is not
an OKF, AirWiki, SQLite, MCP or network schema.

Source and question annotators work independently. Use only the text supplied
to the current side. Never infer hidden questions, sources, labels, ranks,
permissions, likely answers or aliases.

Input IDs are opaque 128-bit tokens prefixed with `src_` or `qry_`. Their only
purpose is lossless record correlation; do not interpret or transform them.

## Normalized identifiers

Entities, relations, object types, qualifier names and normalized values use
lowercase ASCII slugs:

```text
^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$
```

Lowercase, remove accents, replace punctuation and whitespace with one
underscore, and trim underscores. Prefix a value that would begin with a digit
with a neutral type such as `date_`, `number_`, `version_` or `amount_`.
Normalization is syntactic and never establishes an alias.

Use the shortest open relation that preserves the wording, such as
`responsible_party`, `target_date`, `recovery_procedure`, `current_status` or
`completion_condition`. These are examples, not a closed vocabulary. Do not
force text into an example relation when its meaning differs.

## Exact spans

Every semantic field has one or more UTF-8 byte spans into the visible input.
Offsets are zero-based and the end is exclusive. A span must select the
smallest wording that supports that field. A record also has one or more
support spans containing all of its field spans and explicit state wording.
Discontinuous wording uses multiple sorted, nonoverlapping spans.

Spans are audit pointers, not semantic proof. The validator checks only byte
boundaries, containment and syntax. Human reviewers decide whether the field
is actually supported.

## Source claims

The subject is the stable named entity whose property, event or procedure is
stated. Preserve distinctive proper-name tokens. Do not use an answer value,
date, amount, status or procedure as the subject.

The relation describes one property of that subject. The object type and value
describe the relation's value. Qualifiers contain constraints explicit in the
same claim, such as access scope, event kind, time scope, version, unit or
comparator. Qualifier names remain open slugs.

Each claim declares:

- `polarity`: `positive` or `negative`;
- `lifecycles`: one or more open normalized states, normally `current`,
  `planned`, `conditional` or `retracted`; and
- `provenance`: `direct` or `attributed`.

Use `current`, `positive` and `direct` only when ordinary declarative wording
supports those defaults. Bind explicit negation, planning, conditions,
retraction or attribution in a support span. Include every unambiguous claim in
the source. A source with no safely representable claim is `unresolved`.

## Question needs

Split a compound question into the smallest independently answerable needs in
text order. A need names a subject and relation but never predicts its answer
value. It requests one or more object types and constrains qualifiers,
polarity, lifecycle and provenance only when the question wording does so.

Ordinary factual questions normally allow positive direct or attributed
evidence and require `current`. Do not add an entity alias or widen a state
based on likely source content. If any required part of a question cannot be
represented safely, mark the whole question `unresolved` rather than dropping
that need.

## Unresolved reasons

Use exactly one of:

```text
missing_subject
ambiguous_subject
ambiguous_relation
ambiguous_state
unsupported_structure
```

Unresolved records contain no claims or needs. Resolved records contain at
least one. Unknown fields, nulls and commentary are invalid.

## Matching responsibility

Annotators do not decide candidate rank, authorization, relevance labels or
whether the final question is answerable. The deterministic scorer joins exact
normalized fields. Missing data never matches. Semantic mistakes are recorded
as candidate-versus-gold disagreement; they are not repaired by the scorer.
