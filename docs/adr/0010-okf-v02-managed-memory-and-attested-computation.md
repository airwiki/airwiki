# ADR 0010: Adopt OKF v0.2 for managed memory and attested computation

- Status: Accepted
- Date: 2026-08-13
- Supersedes in part: ADR 0002 (read-only MCP contract) and ADR 0004
  (folder-only publication workflow)

## Context

OKF v0.2 adds structured generation, verification, provenance, freshness and
lifecycle metadata while keeping `type` as the only required concept field.
AirWiki also needs portable memory that an explicitly authorized local
application can maintain without committing it to a source repository.

Treating every Wiki as a watched source folder would make imported bundles and
application memory artificial. Treating every MCP client as a general editor
would instead bypass AirWiki's publication and sharing authority.

OKF defines the `Attested Computation` contract shape but deliberately leaves
runtime selection, ABI and sandbox policy to implementations. Executing an
arbitrary declared runtime would create an unbounded code-execution boundary.

## Decision

AirWiki produces OKF v0.2 and accepts three origins:

- a user-owned folder whose reviewed OKF bundle is managed by AirWiki;
- an imported OKF bundle copied into AirWiki-managed storage; and
- an AI-memory OKF bundle managed by AirWiki for one owning application.

SQLite remains authoritative for operational state, paths, capabilities,
grants, quotas, indexes and mutation journals. The managed OKF bundle remains
authoritative for visible imported or memory content. Every filesystem/SQLite
mutation uses staging or atomic replacement plus a recoverable journal.

Import is tolerant in the way required by OKF v0.2: `type` is the only required
concept field, the root index is optional, unknown types and fields are
preserved, and malformed optional metadata produces warnings instead of
inventing assurance. Future declared versions are local-only and cannot be
shared, edited, verified or executed. OKF v0.1 remains readable through explicit
fallbacks but cannot create new external publication.

Each connected application receives a random, revocable capability with an
immutable producer and version. Its secret is stored in a private file and only
the hash is stored in SQLite. Applications may create, read, write and deprecate
concepts only in AI-memory Wikis covered by owner/editor grants. They cannot
delete content, modify history or verification, share a Wiki, grant another
application, or operate on folder/imported Wikis. Cross-application reader or
editor grants require a Rust-owned native confirmation.

AirWiki executes only the product profile `runtime: airwiki-wasm`. Executor and
attester are internal, SHA-256-bound WebAssembly components implementing
`airwiki:attested@1`. They receive no WASI or host imports and run with bounded
bytes, memory, fuel, time, input and output. Execution requires a native
confirmation. A receipt is ephemeral for ten minutes; saving it requires a
second native confirmation and creates a new machine-confirmed concept attributed
to `process:airwiki-wasm`, never to a human reviewer.

## Consequences

- Portable bundles can retain unknown future metadata without AirWiki claiming
  to understand or trust it.
- Trust, freshness and verification age remain independent signals.
- Revoking an application capability stops new reads and mutations immediately
  without deleting its Wikis.
- MCP now has capability-authenticated memory and computation tools in addition
  to policy-authorized search; old search clients remain compatible.
- WebAssembly compilation cost is paid only when a confirmed computation runs.
- AirWiki's `airwiki-wasm` ABI is an application profile, not a claim that OKF
  v0.2 standardizes a universal execution runtime.

## Rejected alternatives

- **Require every optional v0.2 field:** rejects valid minimal OKF bundles.
- **Normalize unknown types into AirWiki types:** loses information and can
  falsely increase trust.
- **Give MCP direct filesystem or SQLite access:** bypasses quotas, journaling,
  grants and source authority.
- **Let an application declare its producer or verification:** permits identity
  and assurance forgery.
- **Enable WASI or arbitrary runtimes:** expands the execution boundary beyond
  the product need.
- **Persist receipts or parameters indefinitely:** retains unnecessary sensitive
  computation data.
