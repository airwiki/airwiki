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

When an existing installation first applies this profile, AirWiki preserves
legacy and future-version bundle bytes but atomically resets their policy to
local-only, removes LAN and application grants, and stops continuous indexing.
Previously public bundles receive a higher-sequence v1/v2 tombstone. The
withdrawal stays pending across restarts until at least one configured catalog
accepts it; only then may the now-unused public runtime stop.

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
fail closed. Applications never receive this authority. Wiki summaries use the
lowest trust among stable, non-deprecated concepts and show stale, warning and
outdated counts separately.

Only lifecycle `stable` may cross LAN, public federation or MCP search. `draft`,
`deprecated` and unknown lifecycle values remain local.

## AirWiki-produced concepts

The generated root index contains only `okf_version: "0.2"` in frontmatter.
Lifecycle belongs to each concept document, not to the bundle as a whole. The
bundle is only the distribution unit; Wiki-level counts such as reviewed,
draft and excluded are derived from its concepts and SQLite operational state.

Folder ingestion writes a concept page with `status: draft` as soon as the
current source revision has been extracted, enriched and indexed. That page
contains `generated.by`, `generated.at` and structured `sources`, but no
`verified` event. Explicit human approval atomically replaces the same concept
with `status: stable` and a human verification. The generated root index lists
only stable concepts, so draft pages remain browsable locally without becoming
searchable or externally discoverable.

AirWiki's `excluded` value is operational SQLite state, not an OKF lifecycle
extension. An excluded item remains an unverified OKF `draft` on disk and may be
reviewed later. Folder-produced concepts may carry
`airwiki.profile_version: 2` for AirWiki identity, review, language and
fingerprint reconciliation.

AI-memory concepts use the immutable application producer/version stored with
the capability and begin stable but unverified. An application cannot write
`verified`, permissions or another actor identity.

### Project-memory bundles

Project memory uses the same concept profile inside the fixed
`.airwiki/wiki/` bundle. Its sibling `project.yaml` is an AirWiki attachment
manifest, not OKF metadata, and contains only schema version, portable project
and Wiki UUIDs, and the display name. Permissions, local collection IDs,
capabilities, grants, indexes, embeddings, journals, logs and repair state never
enter the bundle.

The project root `index.md` declares `okf_version: "0.2"` and the Wiki title.
AirWiki does not regenerate it during concept writes. New pages use UUID
filenames. Updating a page replaces only AirWiki-managed metadata and the
requested Markdown body while preserving unknown OKF fields, provenance and
verification. The personal-vault aggregate quota does not apply, but the
existing per-concept, bundle-size and mutation-rate limits do.

Every read or disclosure also requires an active local attachment. Missing,
invalid or identity-conflicting bundles are retained on disk but expose no
search results. AirWiki does not automatically repair project bundles or
resolve source-control conflicts.

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

An application may hold at most 16 pending computation requests and submit at
most 30 per minute. Terminal sanitized run metadata is retained for no more than
24 hours and is pruned on startup or when another request arrives.
