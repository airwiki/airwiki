# AirWiki OKF v0.2 profile

The [official OKF v0.2 specification] is normative for bundle structure. This
document records AirWiki's compatibility and product-profile choices; it does
not redefine OKF.

[official OKF v0.2 specification]: https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md

## Compatibility

| State | Local browse/search | New external disclosure | Edit/verify/execute |
| --- | --- | --- | --- |
| Declared `0.2` | Yes | Stable concepts only, subject to policy | Subject to origin and authority |
| No declared version, v0.2-compatible | Yes | Stable concepts only, subject to policy | Subject to origin and authority |
| Declared `0.1` | Yes, with legacy fallbacks | No new publication | No |
| Future declared version | Understood fields only | No | No |

AirWiki accepts a bundle without `index.md`. When present, the root index may
declare only `okf_version` in frontmatter. Nested indexes and every `log.md` are
reserved structural pages and cannot contain frontmatter. Concept pages require
only a non-empty `type`. Unknown fields, types, extensions and resources are
preserved. Broken links and invalid optional metadata are warnings.

Legacy reading uses `timestamp` only when `generated` is absent and a Markdown
`# Citations` section only when `sources` is absent. These fallbacks never turn
legacy metadata into a new v0.2 publication.

## Assurance

AirWiki derives, rather than stores in OKF, three independent signals:

- trust: unverified, machine-confirmed or human-reviewed;
- freshness: not declared, fresh, stale or invalid; and
- verification outdated: a valid verification predates generation or its
  AirWiki fingerprint no longer matches.

Malformed optional metadata is preserved but cannot raise trust, enable
execution or authorize disclosure. A user may append a human verification to
the exact current revision of an imported or AI-memory concept after a native
confirmation; stale fingerprints and malformed existing verification metadata
fail closed. Applications never receive this authority. Wiki summaries use the lowest trust among
stable, non-deprecated concepts and show stale, warning and outdated counts
separately.

Only lifecycle `stable` may cross LAN, public federation or MCP search. `draft`,
`deprecated` and unknown lifecycle values remain local.

## AirWiki-produced concepts

The generated root index contains only `okf_version: "0.2"` in frontmatter.
Concepts contain `generated.by`, `generated.at`, `status: stable`, structured
`sources` when available and `verified` when a human or deterministic process
actually confirmed them. Folder-produced concepts may carry
`airwiki.profile_version: 2` for AirWiki identity, review, language and
fingerprint reconciliation.

AI-memory concepts use the immutable application producer/version stored with
the capability and begin stable but unverified. An application cannot write
`verified`, permissions or another actor identity.

## `airwiki-wasm` execution profile

OKF runtimes other than `airwiki-wasm` remain visible but are not executable.
The AirWiki profile uses component-model package `airwiki:attested@1.0.0`:

- executor: parameters JSON to receipt JSON;
- attester: contract JSON, parameters JSON and receipt JSON to an accepted or
  rejected verdict.

Both components must be internal to the bundle, at most 8 MiB each, SHA-256
bound and free of imports. AirWiki provides no WASI, network, filesystem, clock,
entropy, environment or process access. Each store is limited to 64 MiB, one
memory, 10 million fuel units and a two-second epoch deadline. Input is limited
to 64 KiB and output to 256 KiB.

Requests wait for native confirmation and authorization, compatibility,
lifecycle, freshness and the contract fingerprint are checked again immediately
before execution. Parameters and receipts stay in memory
for at most ten minutes. SQLite stores only the actor, timestamps, contract and
component hashes, parameter names/types, receipt hash and verdict. Saving an
accepted receipt is a separate confirmed action into an AI-memory Wiki and
records machine confirmation by `process:airwiki-wasm`.
