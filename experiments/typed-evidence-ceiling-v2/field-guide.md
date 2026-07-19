# Typed-evidence field guide v2

This guide defines a private grammar for one retrieval diagnostic. It is not an
AirWiki, OKF, SQLite, MCP or LAN schema.

Annotators use only the blind input supplied to their role. They must not infer
hidden questions, sources, labels, rankings, expected answers or permissions.
The source and question sides share this grammar but never share their inputs.

## Normalized identifiers

`subject`, `relation`, `object_type`, `object_value`, qualifier names and
qualifier values are lowercase ASCII slugs matching:

```text
^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$
```

Lowercase, remove accents, replace punctuation and whitespace with one
underscore, and remove leading or trailing underscores. Prefix a structured
value that would start with a digit with its object type, such as `date_`,
`amount_`, `code_`, `version_` or `number_`. Preserve units in amount values.

Normalization is syntactic. It does not establish that two names are aliases.
Different normalized names never match, and annotators must not invent aliases.

## Subjects

`subject` is the stable named entity whose property, event or procedure is
stated or requested. Preserve every distinctive proper-name token and omit only
a generic noun such as project, system, document, organization or person.

There is deliberately no subject-kind field. A record may describe several
entities of different kinds. Each claim remains bound to its own subject.

Do not use an answer value, date, amount, status or procedure as the subject. If
the available text does not identify a stable subject, return `unresolved` with
`missing_subject` or `ambiguous_subject`.

## Relations and object types

Use these canonical relations when their definitions apply:

| Relation | Object types | Meaning |
| --- | --- | --- |
| `responsible_party` | `person`, `role` | Person or role explicitly responsible for the subject. |
| `target_date` | `date` | Explicit target date or deadline. |
| `event_date` | `date` | Date of a named event that is not a target or deadline. |
| `current_status` | `status` | Current operational or project state. |
| `recovery_procedure` | `procedure` | Primary steps that restore operation. |
| `fallback_procedure` | `procedure` | Steps triggered when a primary procedure fails. |
| `completion_condition` | `condition` | Condition defining completion or closure. |
| `approved_budget` | `amount` | Amount whose approval status is stated. |
| `reserved_code` | `code` | Code or identifier explicitly marked reserved. |
| `embedded_instruction` | `instruction` | Instruction quoted or embedded as content. |
| `authority_scope` | `status` | Whether text can replace, permit or automate another action. |

This relation vocabulary is deliberately fixture-informed: it was authored
after the diagnostic themes were known. It contains no labels or mappings, but
it does reveal likely relation dimensions to both isolated sides. The resulting
score measures the ceiling of this hand-specified representation on the
observed fixture; it does not measure transferable ontology discovery or
automatic extraction on a fresh domain.

For an unambiguous relation outside the table, create the shortest normalized
predicate slug that preserves its meaning. Both annotation sides follow the
same rule independently. Do not force an ambiguous statement into a listed
relation merely to obtain a match.

Allowed object types are:

```text
person role date status procedure instruction amount code condition statement other
```

Use the most specific allowed type. `statement` and `other` are fallbacks for a
clear value that no earlier type represents; they are not escape hatches for an
ambiguous relation.

`object_value` is concise but meaning-preserving. Structured values use their
type prefix. Procedures, instructions and conditions preserve ordered meaning
well enough to distinguish materially different values. A value-lookup or
existence question never predicts its answer value.

When a source explicitly denies that any value exists and gives no candidate
value, use `absent`. Do not use `absent` for a positive statement or for an
explicitly named negated value.

## State

Each source claim has three independent state fields:

- `polarity`: `positive` or `negative`;
- `lifecycles`: a sorted, non-empty set drawn from `current`, `planned`,
  `conditional` and `retracted`; and
- `provenance`: `direct` or `attributed`.

`negative` means the relation is explicitly denied. `planned` means intended,
scheduled or proposed rather than current. `conditional` means the claim holds
only if a stated condition is met. `retracted` means withdrawn, superseded or
explicitly no longer valid. `attributed` means the source reports or quotes a
different named source. Preserve every explicit state; do not collapse these
axes.

A question need declares `allowed_polarities`, `required_lifecycles` and
`allowed_provenances`. It requests an object type but never predicts an answer
value. This bounded schema supports value-lookup questions only. A question
that asks to verify a proposed value or only whether some value exists is
`unsupported_structure` rather than being widened into a lookup.

Ordinary factual questions require `current`. Add `planned`, `conditional` or
`retracted` only when the wording explicitly requests that state. Allow only
the polarities and provenances permitted by the question wording. Do not widen
constraints based on likely source content.

## Qualifiers

Qualifiers encode constraints explicit in the same source claim or question
need. Allowed names are:

- `access_scope`;
- `reporting_source`;
- `event_kind`;
- `time_scope`;
- `version`;
- `unit`; and
- `comparator`.

A qualifier is:

```json
{"name":"normalized_name","value":"normalized_value"}
```

Sort qualifiers by `(name, value)` byte order and reject duplicates. A source
claim may contain additional qualifiers, but every required question qualifier
must be present for a match. Do not add a qualifier because it might exist in
unseen context.

## Source records

Each source input line has an opaque `source_id` and one canonical `text`, with
navigation-only title or heading fields when available. A resolved output line
has this shape:

```json
{"source_id":"source_id","status":"resolved","claims":[{"subject":"normalized_subject","relation":"normalized_relation","object_type":"status","object_value":"normalized_value","qualifiers":[],"polarity":"positive","lifecycles":["current"],"provenance":"direct","support_quote":"exact source substring"}]}
```

`support_quote` is the smallest exact non-empty substring that contains the
expressed subject, relation, value and explicit state or qualifier wording.
It may be the complete text. It is an audit pointer, not proof that the semantic
fields are correct.

Include every unambiguous claim expressed by the source text, including
negative, planned, conditional, attributed and retracted claims. Title and
heading provide navigation context but cannot supply a missing claim. Preserve
claim order by first occurrence in the text, then by relation byte order when
two claims begin at the same position.

If the line cannot be represented without guessing, emit:

```json
{"source_id":"source_id","status":"unresolved","reason_code":"ambiguous_relation"}
```

## Question records

Each question input line has an opaque `question_id` and `question`. A resolved
output line has this shape:

```json
{"question_id":"question_id","status":"resolved","needs":[{"subject":"normalized_subject","relation":"normalized_relation","requested_object_types":["status"],"required_qualifiers":[],"allowed_polarities":["positive"],"required_lifecycles":["current"],"allowed_provenances":["attributed","direct"],"question_quote":"exact question substring"}]}
```

Split a compound question into the smallest independently answerable needs and
preserve question order. `question_quote` is the smallest exact non-empty
substring expressing that need. Quotes may overlap. It points to the wording;
it is not semantic proof.

Populate `requested_object_types` with every type explicitly permitted by the
wording. Add only explicit qualifiers and lifecycle constraints. Never include
or predict an answer value.

If any required need cannot be represented without guessing, mark the complete
question unresolved rather than dropping that need:

```json
{"question_id":"question_id","status":"unresolved","reason_code":"ambiguous_subject"}
```

## Resolution and semantic responsibility

Allowed unresolved reason codes are:

```text
missing_subject ambiguous_subject ambiguous_relation ambiguous_state unsupported_structure
```

Resolved records contain at least one claim or need. Arrays used as sets are
sorted by UTF-8 byte order and contain no duplicates. Unknown fields, nulls and
model commentary are invalid.

Structural validation checks schema, ordering, syntax and exact quote presence
only. It cannot decide whether a semantic field is supported. Draft annotators
and the context-isolated adjudicator must make that judgment from their allowed
text. An adjudicator corrects unsupported agreement as well as disagreement; it
returns `unresolved` when the available text does not decide.

## Exact matcher

A source claim covers a question need only when:

1. subject and relation are equal;
2. object type belongs to `requested_object_types`;
3. all required qualifiers and lifecycles occur in the claim;
4. polarity and provenance belong to the allowed sets; and
5. every required field is present without widening an unknown value.

Question needs never compare or predict an answer value. An unknown or missing
field never matches. There is no synonym table, fuzzy match, embedding or
inference call.

Within each source pool, compute all `(need_index, object_value)` match edges for
a candidate in frozen rank order. Remove only edges already retained earlier in
that source. Keep the candidate if any new edge remains, add those edges to the
source-local seen set, and stop after five retained candidates. A candidate
that repeats one edge but introduces another is retained. Distinct object values
for one need are not duplicates. Conjunctive gateway coverage is evaluated only
from the retained edges across authorized sources; any uncovered need makes the
complete case abstain.
