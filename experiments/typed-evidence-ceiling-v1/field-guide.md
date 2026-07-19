# Typed evidence field guide v1

This guide defines a structural annotation grammar. It does not define which
source answers which question. Annotators must use only the blind input they
receive. They must not search for or infer hidden identifiers, labels, pools,
rankings or expected answers.

## Identifier grammar

Every authored semantic identifier or normalized value is a lowercase ASCII slug that
matches:

```text
^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$
```

Strip accents, lowercase, replace punctuation and whitespace with one
underscore, and remove leading or trailing underscores. A value that would
begin with a digit receives its object-type prefix: `date_`, `amount_`,
`code_`, `version_` or `number_`. Preserve units in amount values.

The Rust preparer guarantees that opaque input IDs match the same slug grammar.
`atom_id` and `need_id` are deterministic local identifiers. Use the opaque
input identifier plus `_atom_01` or `_need_01`, increasing the two-digit suffix
in the prescribed order.

## Subjects

A subject has two fields:

```json
{"name":"distinctive_name","kind":"closed_kind"}
```

`name` preserves every distinctive proper-name token and omits only the
generic kind words mapped below. `kind` is one of:

- `initiative`: project, trial, exercise, drill, campaign or their direct
  translations;
- `system`: system, service, platform or their direct translations;
- `device`: sensor, device, machine or their direct translations;
- `document`: note, document, policy, report or runbook;
- `organization`: named group, institution, library or company;
- `person`: a named human subject;
- `other`: an explicit generic kind not covered above; or
- `unspecified`: no generic kind is explicit in the available text.

The subject is the named entity whose property, event or procedure is being
described or requested. If no stable subject name is stated, return
`missing_subject`. Do not use an answer person, date, amount, status or
procedure as the subject.

Matching always requires an exact `name`. Two explicit kinds must also match.
`unspecified` is compatible with one explicit kind because a sentence can omit
an entity's kind, but different explicit kinds are never compatible. Do not
invent aliases between different names.

## Relations and object types

Use one relation and one compatible object type from this matrix:

| Relation | Allowed object type | Selection rule |
| --- | --- | --- |
| `responsible_party` | `person`, `role` | Person or role explicitly responsible for the subject. |
| `target_date` | `date` | Explicit target date or deadline. |
| `event_date` | `date` | Date of a named event that is not a target or deadline. |
| `current_status` | `status` | Current operational or project state. |
| `recovery_procedure` | `procedure` | Primary steps that restore operation. |
| `completion_condition` | `condition` | Condition defining completion or closure. |
| `embedded_instruction` | `instruction` | Instruction quoted, recorded or embedded as content. |
| `approved_budget` | `amount` | Amount whose approval status is stated. |
| `reserved_code` | `code` | Code or identifier explicitly marked reserved. |
| `authority_scope` | `status` | Whether text can replace, permit or automate another action. |
| `fallback_procedure` | `procedure` | Steps explicitly triggered when a primary procedure fails. |
| `other_statement` | `statement`, `other` | Unambiguous relation outside this table. |

Precedence is deterministic:

1. A target or deadline is `target_date`; another event is `event_date`.
2. Failure-triggered steps are `fallback_procedure`; primary restoration steps
   are `recovery_procedure`.
3. A quoted or imperative instruction itself is `embedded_instruction`;
   quoted factual content keeps its factual relation, and executable
   restoration steps are a procedure.
4. A statement about whether a text can authorize, replace or automate is
   `authority_scope`. This describes source content, not AirWiki policy.
5. Use `other_statement` only when the statement is unambiguous and no earlier
   rule applies. Otherwise return `ambiguous_relation`.

The complete object-type vocabulary is:

- `person`
- `role`
- `date`
- `status`
- `procedure`
- `instruction`
- `amount`
- `code`
- `condition`
- `statement`
- `other`

A source claim includes a concise normalized `object_value`. Prefix structured
values with their type, for example `date_2030_01_02`,
`amount_2500_eur`, `status_green` or `code_z9`. Prefix procedures,
instructions and conditions with `procedure_`, `instruction_` or `condition_`
and preserve enough ordered meaning to distinguish different values. A
value-lookup need never contains or predicts an answer value; a verification
need may copy only the value explicitly tested by its question.

When a negative claim explicitly states that no value exists and supplies no
candidate value, use the reserved `object_value` `absent` with the relation's
normal object type. Do not use `absent` for a positive claim or when the
negated value is explicitly stated.

## Claim state and question constraints

Source state uses three independent fields:

- `polarity`: `positive` or `negative`;
- `lifecycles`: a non-empty sorted set drawn from `current`, `planned`,
  `conditional` and `retracted`; and
- `provenance`: `direct` or `attributed`.

`negative` means the relation is explicitly denied. `planned` means proposed,
scheduled, intended or expected but not current. `conditional` means the claim
holds only if a stated condition is met. `retracted` means withdrawn,
superseded or explicitly no longer valid. `attributed` means the source reports
or quotes another speaker, document or named source. These axes must not be
collapsed into one label. Lifecycle values can coexist: preserve every explicit
value, such as both `conditional` and `planned`.

Each question need declares `answer_intent` as `value_lookup`,
`value_verification` or `existence_verification`, plus sorted duplicate-free arrays of allowed polarities,
required lifecycles and allowed provenances:

- who/what/when/which/how questions always use `value_lookup`, even when they
  request a negated value. Allow `positive`, `negative`, or both exactly as the
  wording requires;
- yes/no or whether questions that state a candidate value use
  `value_verification`. Copy that normalized value to `tested_object_value`;
  both polarities are allowed unless the wording explicitly asks only one;
- existence questions that state no candidate value use
  `existence_verification`, omit `tested_object_value`, and allow both
  `negative` and `positive` unless the wording explicitly asks only one;
- `value_lookup` also omits `tested_object_value` and never predicts its answer
  value;
- `current` means the statement is presently valid, regardless of whether the
  event it describes occurred in the past. Ordinary questions, including
  historical fact lookups, require `current`; add `planned`, `conditional`
  or `retracted` when each is explicit. Multiple required lifecycle values can
  coexist;
- ordinary questions allow `attributed` and `direct`. A question naming the
  document being consulted also allows both, because a direct statement inside
  that document is not attributed merely by being requested. Restrict to
  `attributed` only when the question asks for content quoted or reported by a
  third party.

Sort every set by UTF-8 byte order. Matching requires exact membership for
polarity and provenance and requires every question lifecycle to be present in
the source lifecycle set. The claim object type must belong to
`requested_object_types`. For `value_verification`, `tested_object_value` must
equal the claim object value; the other two intents do not compare an object
value. An existence verification can therefore match either a positive value
or an explicit negative `absent` claim. There is no implicit widening.

Populate `requested_object_types` from the relation matrix. Include every type
the wording permits: an unrestricted “who is responsible” request uses both
`person` and `role`; an explicit request for a person or role uses only that
type. For `other_statement`, use both `other` and `statement` unless the
wording explicitly selects one.

## Qualifiers

Use qualifiers only for constraints explicit in the available text. Names are
closed:

- `access_scope`: `public`, `internal`, `restricted` or `confidential`;
- `reporting_source`: a normalized named source such as `source_a`;
- `event_kind`: the normalized event phrase after removing the subject;
- `time_scope`: `past`, `current` or `future` when explicitly contrasted;
- `version`: a `version_`-prefixed normalized value;
- `unit`: a normalized unit slug; and
- `comparator`: `equal`, `less_than`, `at_most`, `greater_than` or `at_least`.

Each qualifier is:

```json
{"name":"closed_name","value":"normalized_value"}
```

Sort qualifiers by `(name, value)` byte order and reject duplicates. A question
qualifier is required; a source claim may contain additional qualifiers. Do not
add a qualifier merely because it might exist in unseen context.

## Source anchors

This ceiling uses one-sentence canonical chunks. Every resolved claim must be
anchored to the complete input `text`:

- `byte_start` is `0`;
- `byte_end` is the supplied UTF-8 `byte_length`; and
- `text_sha256` is copied exactly from the input.

Do not calculate a new hash, trim the text or select a smaller span. If the
whole text does not support the claim, do not emit that claim.

## Resolution and ordering

One source can contain multiple claims and one question can require multiple
needs. Keep every claim internally bound: subject, relation, value, qualifiers,
state and anchor come from the same statement. Never assemble them from
separate entities or sentences.

If any required component on a line is ambiguous or unsupported, mark the
entire source or question line unresolved. Do not emit a partial resolved line
and do not silently omit an ambiguous claim or need. Allowed reason codes are:

- `missing_subject`
- `ambiguous_subject`
- `ambiguous_relation`
- `ambiguous_state`
- `unsupported_structure`

For source claims sharing the required full-chunk anchor, sort by relation,
subject name, object type and object value in UTF-8 byte order, then assign
atom suffixes. For question needs, preserve question order, using relation byte
order only when two needs occupy the same phrase, then assign need suffixes.

Do not infer AirWiki authorization or permission metadata. A textual claim
whose subject matter is authority can still be represented as
`authority_scope`. Do not guess to avoid an unresolved result, and do not emit
commentary outside the required JSONL records.
