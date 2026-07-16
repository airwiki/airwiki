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

xtask ──────────────┬──> airwiki-core
                    └──> airwiki-inference
```

- `airwiki-types` contains dependency-light identifiers and contracts.
- `airwiki-core` owns ingestion, SQLite, search, OKF publication and inspection.
- `airwiki-inference` owns the model catalog, verified assets and local runtime.
- `airwiki-network` owns authenticated LAN discovery, pairing and transport.
- `airwiki-mcp` owns the read-only MCP contract, loopback gateway and stdio bridge
  implementation.
- `apps/desktop` is the composition root. It owns background orchestration and
  renders egui.
- `apps/mcp-bridge` is a thin executable over `airwiki-mcp`. At runtime it exposes
  stdio to a local chat client and forwards only to the desktop's fixed loopback
  MCP endpoint.
- `apps/windows-firewall-helper` is a narrow elevated process. The desktop may
  invoke only its fixed install/remove operations on Windows. See
  [ADR 0006](adr/0006-windows-firewall-privilege-boundary.md) for the privilege
  boundary and managed rule scope.
- `xtask` contains reproducible repository, legal and packaging checks.

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
```

Reusable crates never depend on the desktop UI. Transport adapters do not own
curation or collection policy. Add a crate only for a durable dependency or
privilege boundary; otherwise prefer a module.

## Sources of truth

SQLite is the source of operational state, local paths, jobs, trust, grants and
audit events. Published OKF files are the source of truth for the visible wiki
representation. Reconciliation reports disagreement rather than silently
selecting one side. Original documents are never changed or replicated.

The watcher is a latency optimization. Startup and periodic full reconciliation
are idempotent recovery paths for missed events, renames and interruptions.

```text
detected -> extracted -> enriched -> needs review -> published
                                      ^ human approval
```

A modified source withdraws its published revision before preparing a new one.
Automation may retry, inspect and rebuild unambiguous derived artifacts, but it
may not publish, grant access or enable external chat. Guided repair withdraws
affected concepts before changing ambiguous state, preserves a verified
snapshot and requires explicit confirmation. It never invents content or
history, and it never restores exposure without a coherent reviewed revision.

See [ADR 0004](adr/0004-sqlite-okf-authority-and-reconciliation.md) for the
durable SQLite/OKF authority boundary.

## Execution model

egui renders completed view models and sends intent to the desktop worker.
Filesystem traversal, parsing, hashing, SQLite-heavy operations, inference and
network I/O never run on the egui thread. Async work runs on the Tokio runtime;
blocking filesystem, parser, database and CPU work uses a blocking pool. UI
requests and worker responses carry request identifiers so stale results can be
discarded.

Operating-system calls that may block or launch external work also cross the
worker boundary. Immediate window and tray integration may run on the eframe
thread when the platform API requires it, but handlers must remain bounded and
must not perform filesystem traversal, network I/O or business logic.

The process has one instance per user session. Hiding the window preserves the
worker, watchers, MCP and LAN. **Exit completely** performs a bounded shutdown.
Optional autostart is per-user and confirmed; there is no daemon or system
service.

## Trust and authorization

- Local publication always requires review.
- LAN search requires authenticated pairing, collection policy and a grant at
  the source node.
- Local MCP requires `allow_external_ai`; it does not imply peer sharing.
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
an SBOM. The stable manifest is published last. See the
[release checklist](release-checklist.md).
