# Architecture

AirWiki is a local-first desktop application. It turns files selected by
the user into human-reviewed knowledge, keeps operational state on that device,
and can federate bounded search evidence to explicitly paired devices or local
chat clients.

## Dependency direction

Arrows below mean "depends on" at build time.

```text
apps/desktop ───────┬──> airwiki-core ─────────────> airwiki-types
                    ├──> airwiki-inference
                    ├──> airwiki-network ───────────> airwiki-types
                    ├──> airwiki-mcp ───────────────> airwiki-types
                    ├──> airwiki-types
                    └──> airwiki-windows-firewall-helper (Windows only)

apps/mcp-bridge ───────> airwiki-mcp ───────────> airwiki-types
apps/federation-index ─> airwiki-network ───────> airwiki-types

xtask ──────────────┬──> airwiki-core
                    ├──> airwiki-inference
                    └──> airwiki-types
```

- `airwiki-types` contains dependency-light identifiers and contracts.
- `airwiki-core` owns ingestion, SQLite, search, OKF publication and inspection.
- `airwiki-inference` owns the model catalog, verified assets and local runtime.
- `airwiki-network` owns authenticated LAN discovery, pairing and transport.
- `airwiki-mcp` owns search plus capability-authenticated memory/computation MCP
  contracts, the loopback gateway and stdio bridge implementation.
- `apps/desktop` is the composition root. It owns Tauri lifecycle and IPC,
  background orchestration, the local Svelte WebView, client integration
  discovery, and atomic installation of versioned global workflow guides.
- `apps/mcp-bridge` is a thin executable over `airwiki-mcp`. At runtime it exposes
  stdio to a local chat client and forwards only to the desktop's fixed loopback
  MCP endpoint.
- `apps/federation-index` stores signed, expiring public routing manifests in
  SQLite/WAL. It has no publication authority and stores no document content,
  snippets, embeddings or source paths.
- `apps/windows-firewall-helper` is a narrow elevated process. The desktop may
  invoke only its fixed install/remove operations on Windows. See
  [ADR 0006](adr/0006-windows-firewall-privilege-boundary.md) for the privilege
  boundary and managed rule scope.
- `xtask` contains reproducible repository, legal, packaging and evaluation
  checks.

The important runtime boundaries are separate from Rust dependency direction:

```text
local chat client -- MCP stdio --> airwiki-mcp-bridge
                                      |
                                      | fixed loopback Streamable HTTP
                                      v
desktop process -----------------> airwiki-mcp gateway
      |
      | explicit elevated install/remove request (Windows only)
      v
airwiki-windows-firewall-helper

desktop integration worker -- confirmed, receipt-bound writes --> documented
                                                            client user roots
```

Reusable crates never depend on the desktop UI. Transport adapters do not own
curation or collection policy. Add a crate only for a durable dependency or
privilege boundary; otherwise prefer a module.

Workflow-guide resources are immutable package inputs. The desktop resolves
only documented client roots, performs filesystem inspection and mutation in
`spawn_blocking`, and owns rollback. MCP capabilities remain separate from
skills: instructions can guide an agent, but Rust authorization still gates
every read or mutation.

## Sources of truth

SQLite is the source of operational state, local paths, jobs, trust, grants and
audit events. Published OKF files are the source of truth for the visible wiki
representation. Reconciliation reports disagreement rather than silently
selecting one side. Original documents are never changed or replicated.

Imported and AI-memory Wikis have no source folder. Their managed OKF v0.2
bundle is the visible authority and SQLite is a searchable operational
projection. Filesystem/SQLite mutations use atomic replacement and an
append-only recovery journal. See the [AirWiki OKF v0.2 profile](okf-v02-profile.md).

The watcher is a latency optimization. Startup and periodic full reconciliation
are idempotent recovery paths for missed events, renames and interruptions.

```text
detected -> extracted -> enriched -> needs review -> published
                                      ^ human approval
```

A modified source withdraws its published revision before preparing a new one.
The review screen loads bounded pages of extracted chunks through the desktop
worker. An opaque version binds the visible draft, source revision and complete
chunk set; storage revalidates it in the publication transaction. Stale responses
are discarded and publication remains unavailable until current evidence loads.
Automation may retry, inspect and rebuild unambiguous derived artifacts, but it
may not publish, grant access or enable external chat. Guided repair withdraws
affected concepts before changing ambiguous state, preserves a verified
snapshot and requires explicit confirmation. It never invents content or
history, and it never restores exposure without a coherent reviewed revision.

See [ADR 0004](adr/0004-sqlite-okf-authority-and-reconciliation.md) for the
durable SQLite/OKF authority boundary.

## Execution model

One local Tauri WebView renders completed, serde-defined snapshots and sends
only explicit capability-specific commands. `ts-rs` generates the committed
TypeScript contract; CI rejects stale bindings. JavaScript receives no direct
filesystem, shell, HTTP, SQL, updater, autostart or process capability.

`AppRuntime` and the domain services run on Tauri's Tokio runtime. UI intents
use a bounded `mpsc` channel, consolidated state uses `watch`, transient
progress uses bounded `broadcast`, and request-scoped responses use identifiers
that discard stale results. Filesystem traversal, parsing, hashing,
SQLite-heavy operations, inference, process work and blocking operating-system
calls use the worker boundary and `spawn_blocking`; no lock is held across an
await.

The WebView loads only bundled assets under a strict CSP. Markdown becomes a
typed safe AST in Rust; it never reaches `innerHTML`, and images, SVG, embeds,
files and remote resources are excluded. Folder selection returns an opaque,
single-use, expiring token, so JavaScript cannot submit arbitrary paths.
An explicit navigation guard allows only the local application origin. Rust
uses parented OS-native dialogs—not caller-supplied booleans—to establish human
authority for sharing, external policy, repairs, model licenses, updates and
external HTTP(S) destinations.
Window, tray and lifecycle callbacks remain bounded and contain no business
logic.

The process has one instance per user session. Hiding the window preserves the
worker, watchers, MCP and LAN. **Exit completely** performs a bounded shutdown.
Optional autostart is per-user and confirmed; there is no daemon or system
service. Autostart's exact `--background` mode remains hidden only after the
tray is known to be operational; all foreground launches start visible.

## Trust and authorization

- Local publication always requires review.
- LAN search requires authenticated pairing, collection policy and a grant at
  the source node.
- Public federation is a separate opt-in. Discovery needs no pairing or grant,
  but the owner revalidates current publication, sequence and fingerprint under
  a disclosure lease before every response. See [ADR 0008](adr/0008-public-federation.md).
- Local MCP requires `allow_external_ai`; it does not imply peer sharing.
- Application memory uses a separate random capability and per-Wiki
  owner/reader/editor grant. It never implies LAN, public or other-application
  access. Grants and computation execution require native confirmation.
- `airwiki-wasm` runs only internal hash-bound components with no imports and
  bounded memory, fuel, time and payloads. Saving an accepted receipt requires
  a second confirmation and creates machine-confirmed knowledge.
- External-chat search separates answerability-accepted evidence from bounded
  authorized candidates. Both lanes are revalidated against current policy;
  local desktop search receives only evidence.
- MCP and model runtime listeners bind only to loopback.
- The client label reported by a bridge is diagnostic, never authentication.
- When state is uncertain, searchable exposure fails closed.

See [ADR 0005](adr/0005-lan-identity-pairing-and-authorization.md) for LAN
identity, pairing and revocation, and the [threat model](threat-model.md) for
boundaries and residual risks.

## Portability and releases

The tested development targets are macOS arm64 and Windows x64. Platform code
stays in the desktop composition root or the Windows helper. A new target
requires CI and a clean manual smoke test before it is claimed. These targets
are not a public support promise while the project remains an internal release
candidate.

The root source license is Apache-2.0. Release artifacts add model and
third-party terms, native publisher signatures, updater signatures, hashes and
an SBOM. Preparation creates a private draft; an independent two-platform
verification and protected human approval publish the stable manifest with the
release. See the [release process](release-process.md) and
[release checklist](release-checklist.md).
