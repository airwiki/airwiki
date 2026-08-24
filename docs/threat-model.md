# Threat model

## Purpose and limits

AirWiki reduces knowledge replication between workstations and requires
human approval before evidence is shared. It does not protect a compromised host,
a malicious administrator, screenshots, manual copying, or data deliberately
authorized for a cloud provider.

Controls are not considered effective end to end until the
[two-node runbook](two-node-runbook.md) passes on the tested candidate.

## Assets

- source Markdown and PDF files;
- extracted text, metadata, OKF concepts, and embeddings;
- SQLite state, audit events, and local paths;
- Ed25519 device identity and trusted-peer state;
- separate public-publisher identity, signed manifests and tombstones;
- collection grants and `allow_external_ai` policy;
- application capability secrets, per-memory grants and managed personal OKF
  bundles;
- portable `.airwiki` manifests and project OKF bundles, plus private local
  attachments, fingerprints and projections;
- pending attested-computation parameters and ephemeral receipts;
- local models, runtime, and pinned artifact identities;
- search snippets, MCP concept summaries, and complete published OKF pages
  returned to authorized LAN or public readers;
- client-owned ChatGPT, Claude, and Gemini configuration;
- per-user background, autostart, and update configuration; and
- integrity of the desktop, bridge, firewall helper, installer, and update data.

## Trust boundaries

1. **Watched folder → local node.** Every document is untrusted input.
2. **Local WebView → Tauri IPC → worker.** The WebView has a strict CSP and no
   direct filesystem, shell, HTTP, SQL, updater or process capability. Commands
   are explicit and validated; parsing, inference, storage, blocking OS calls,
   and network work remain in Rust.
3. **Node → `llama-server`.** Loopback only, random token, one concurrent
   inference.
4. **Node → LAN.** Noise authenticates the PeerId and encrypts transport; grants
   remain mandatory.
5. **MCP HTTP → stdio bridge.** MCP is loopback-only with exact `Host`; the bridge
   has a fixed endpoint, ignores proxies, and stores no credentials.
6. **Bridge → chat client.** Authorized snippets enter the provider boundary and
   become subject to that account or workspace policy.
7. **Desktop → operating system.** Tray, per-user autostart, and single-instance
   activation remain inside the user session; OS state is authoritative.
8. **Desktop → elevated Windows helper.** The helper accepts only `install` or
   `remove` and owns two narrow application rules, as defined by
   [ADR 0006](adr/0006-windows-firewall-privilege-boundary.md).
9. **Desktop → update transport.** Remote metadata and bytes remain untrusted
   until updater and native signatures pass. Public update distribution is not
   active in the current baseline.
10. **Publisher → public federation.** Replaceable indexes receive signed,
    expiring routing metadata only. Unpaired readers receive bounded search
    snippets and, after opening one result, the complete published OKF Wiki
    directly from the owner over Noise-authenticated QUIC or relay routes. The
    owner remains the authorization authority.
11. **Managed MCP application → memory backend.** The bridge authenticates with
    a private capability resolved from a public integration identifier. The
    backend uses bounded channels, per-Wiki grants, optimistic fingerprints,
    quotas and rate limits.
12. **OKF bundle → `airwiki-wasm`.** Components are untrusted bundle bytes. Only
    hash-bound, import-free component-model artifacts run, without WASI or host
    capabilities and under memory, fuel, time and payload limits.
13. **Desktop → client user instructions.** A native confirmation lists the
    documented global files before AirWiki installs a skill, `AirWiki.md` and one
    import. Staging, hashes, private receipts and exact-root checks constrain the
    write; no repository file is modified.
14. **Project working tree → project-memory backend.** `.airwiki` is untrusted,
    externally mutable repository content. AirWiki accepts only a canonical
    absolute non-linked root, a strict bounded manifest and an in-root bounded
    OKF bundle. Portable IDs identify files; a separate local attachment and
    application grant authorize each clone.

## Threats and controls

| Threat | Design control | Residual risk / validation |
| --- | --- | --- |
| Unauthorized LAN peer queries data, learns listener endpoints or device metadata, restores stale endpoints or steers private-network probes | Noise, SAS pairing, trust state, per-collection grants and rate limits; the wildcard listener is never announced, a dedicated bounded exchange sends OS-inspected private/on-link endpoints plus a display name and OS family only after durable trust, receivers persist metadata only for the already trusted PeerId, accept only the IP observed on that exact authenticated connection, and reject delayed state with session-scoped monotonic revisions | Excessive grants still disclose data; names and OS labels are mutable presentation metadata rather than authorization, an endpoint is discovery data only, and every redial authenticates the expected PeerId; test pre-trust non-disclosure of endpoints and metadata, multi-interface selection, stale revision and cross-host endpoint rejection, interrupted retry, and the full grant and revocation matrix |
| LAN browsing enumerates more knowledge than the selected result authorized | Browsing accepts one Wiki ID obtained from an authorized result, revalidates trust, the exact per-Wiki grant, policy, OKF compatibility and publication under a disclosure lease before every bounded frame or page, exposes only that Wiki's complete published OKF workspace, refuses mixed-generation frames and rejects dangling graph edges | A reader can retain the complete published knowledge of a granted Wiki; grant only Wikis intended for that device |
| Private or stale content reaches a public reader | Separate opt-in, stable reviewed-publication checks, fingerprint-bound page reads, signed sequence and fingerprint, final disclosure lease, immediate owner-side revocation | A third party can retain previously returned search metadata or published OKF content |
| Malicious index redirects or ranks content | Expected index PeerId is pinned; owner manifests are independently signed; index rank selects routes but never final ranking | An index can omit publishers, delay tombstones, or degrade availability until replaced |
| Public queries exhaust an owner | Three-index, 64-candidate, 12-peer and two-collection caps; bounded payloads, semaphores, per-peer rate limits with a 1,024-identity window cap, a 1 s index stage, a 3 s cold owner-connection budget and a separate 800 ms owner-response budget | Bounded connection setup, local verification and ranking add work after catalog selection; distributed abuse can still consume bounded relay and inference capacity |
| Public registrations exhaust an index | Manifest remaining and signed lifetimes are at most 24 hours with bounded future skew; each SQLite catalog admits at most 100,000 total rows and 1,000 per publisher; expiry removes payload/FTS data but retains a compact sequence high-water mark, as do tombstones; compact rows remain for the node lifetime and count against both ceilings | An attacker can consume the bounded shared capacity and deny new registrations; operators monitor sanitized database and disk consumption and may replace or retire the index |
| Public route evidence is misattributed | Route state is request-scoped and accepted only when a protocol-valid owner response arrives within the separate connection and response budgets on the matching observed connection; an index connection, timeout or concurrent search remains offline; Relay takes precedence for mixed successful search routes | Direct and relay observations describe transports used by accepted responses, not future reachability |
| Publisher advertises an unavailable relay | Only relay listeners with a live outbound reservation enter signed manifests; readiness changes advance the sequence and immediately reannounce remaining routes or tombstone the collection; local status rejects late lower-sequence completions | Listener events can still race a network outage, but the owner remains fail-closed and the stale route can only degrade bounded availability |
| A blocked publisher completes an in-flight response | The local publisher block is rechecked before owner requests, while accepting and emitting search results, at final fusion and immediately before accepting browse content; a completed block excludes cached and in-flight disclosure | A user who explicitly unblocks the same identity restores future access |
| Public identity reveals or correlates private devices | Readers use one ephemeral identity per application session; LAN and publisher identities are separate; public protocols and UI expose the stable public publisher ID but never the LAN display name, hostname or OS family | Network observers and relays can still correlate IP and timing, and public content may identify its author independently |
| Public beta infrastructure exceeds its operating budget | Two minimal independent nodes, live retail-price preflight, per-node monthly budgets, forecast/actual alerts, platform consumption review and whole-group retirement | Azure budgets alert but do not hard-cap spend; notification and cost data can lag |
| Public beta host or identity is replaced | Pinned image and binary hash, Azure-attested SSH host key, one persistent identity/database per node, compiled expiring registry and higher-version rotation | Older candidates can use only their unchanged or unexpired pinned entries |
| Pairing impersonation | Six-word SAS derived from identities and nonces, two-minute expiry | A user who skips comparison defeats the control |
| Private collection reaches cloud chat | `allow_external_ai` defaults off and is rechecked at the source | Human authorization can be wrong; use synthetic fixtures and audit |
| Ranking returns the least-wrong absent fact | Source node applies the pinned local answerability classifier to the bounded outgoing snippet; failures and timeouts close the path | The classifier is probabilistic; reassess both platforms when model, corpus, or policy changes |
| External chat treats an authorized candidate as relevant | Candidates exist only for `external_ai`, remain separately typed and bounded, pass the same final authorization checks, lose to duplicate evidence, and carry instructions requiring explicit support | A chat model can still misuse an unrelated authorized snippet; minimize externally enabled collections and run the golden prompt set |
| DNS rebinding reaches MCP | Loopback bind, exact authority including port, bounded body | Compromised local software can already call loopback |
| Application capability is forged, crossed or revoked mid-operation | Random secret, hash-only SQLite storage, fixed bridge resolution, active-capability recheck, per-Wiki role and immediate revocation | Malware in the same user account may read the private credential file |
| Assistant forges producer, human verification or permissions | Rust fixes immutable producer/version, generation time and lifecycle; MCP schemas exclude verification and sharing; only AI-memory bundles are writable | A user may still over-grant an application; review grants and revoke it |
| Opening a folder silently creates or authorizes project memory | Discovery only recognizes an existing manifest; initialize/open create a bounded expiring request, and native approval revalidates the canonical root and fingerprint before any file write or grant | A user can still approve the wrong selected folder; the confirmation names the folder without exposing its path to MCP |
| A cloned repository inherits another clone's authority | Portable project/Wiki IDs are distinct from the installation-local collection ID; approval is bound to application, canonical root and portable identity | Copying a working tree requires another local confirmation by design |
| Manifest substitution, symlink or path escape targets other files | Strict 64 KiB YAML, exact schema version and fields, canonical absolute root, symlink/reparse rejection, normalized in-root bundle paths and atomic sibling staging | A same-user process can race filesystem operations; reconciliation fails closed and preserves diagnosable files |
| Git conflict or external edit leaves a stale project projection exposed | Watcher and startup reconciliation mark the attachment unavailable and clear its projection before validation; missing, invalid and identity-conflicting bundles cannot serve MCP, LAN, public or external-AI results | Detection while the app is already running has watcher latency; review ordinary diffs and resolve conflicts outside AirWiki |
| Local detach destroys versioned project documentation | Detach revokes grants and sharing and removes only local attachments/projections; `.airwiki` remains untouched and AirWiki never executes Git | The user or another tool may still delete repository files outside AirWiki |
| Workflow-guide installation overwrites user instructions | Only documented absolute user roots, no symlink/reparse traversal, bounded UTF-8 parsing, atomic writes, BOM/EOL preservation and an exact single import; user-modified managed files become conflicts | Software under the same user account can race or alter the files; reinspection fails closed and requires manual resolution |
| A workflow guide expands an assistant's authority | The packaged skill contains no code, excludes secrets and transient data, and explicitly forbids verification, sharing, publication and permission changes; every MCP call still requires the application's capability and Wiki grant | Models may ignore instructions; Rust contracts and authorization, not the skill, remain authoritative |
| Disconnect removes unrelated global configuration | Private receipts identify exact installed hashes; removal occurs only while skill, guide and import still match, and rollback restores only bytes changed by the failed operation | A conflict may require the user to remove or reconcile stale text manually |
| Memory mutation splits filesystem and SQLite | Staging/atomic replacement, optimistic fingerprint, append-only journal and fail-closed startup reconciliation | Disk failure can require explicit recovery while the Wiki remains unavailable |
| Malicious OKF optional metadata raises trust or enables code | Unknown/invalid fields are preserved with warnings but ignored for assurance, sharing and execution; future versions are local-only | A future field remains visually present and must not be mistaken for interpreted policy |
| Attested component accesses host data or exhausts the node | No imports or WASI; in-bundle hash-bound artifacts; 8 MiB components, 64 MiB memory, 10M fuel, two-second deadline and bounded JSON | Compiler/runtime vulnerabilities remain in the local TCB; keep Wasmtime pinned and audited |
| Computation result is mistaken for human-reviewed knowledge | Executor and attester confirmation are separate from human verification; saved results require a second native prompt and use `process:airwiki-wasm` with machine-confirmed trust | The deterministic attester proves the configured procedure, not the truth of its inputs |
| Response-based exfiltration | At most ten items per typed lane, bounded snippets, a smaller global serialized MCP budget that drops candidates first, no paths/full documents/embeddings/indexes, and a global MCP rate limit | Repeated authorized queries may reconstruct information; minimize external-chat collections |
| Another local process imitates a bridge | Loopback and collection policy; client label is diagnostic only | Loopback is not per-process authentication; protect the local account |
| Duplicate desktop instances corrupt state | Per-user guard before SQLite/MCP/LAN/models, activation limited to `SHOW/OK` | Local name squatting can prevent startup but cannot start duplicate services |
| Activation message is forged | No parameters or content; effect only shows the window | Not an authorization boundary |
| Autostart changes without consent | Applied only after onboarding; exact OS entry is authoritative; conflicts are not overwritten | Another process under the same account can modify it |
| Hidden app cannot be recovered | Hide only with a working tray; second launch shows the same instance; bounded full exit | Shell failure may require relaunch; test Explorer/Finder recovery |
| Modified document remains shared | Watcher accelerates withdrawal; startup and periodic full reconciliation recover missed events | A bounded window exists before detection while the app remains open |
| Source changes while app is closed | LAN and MCP stay closed until full preflight completes | An inaccessible root keeps search unavailable |
| Watcher or root disappears | Atomic quarantine withdraws published state and indexes; watcher retries | Recovery requires complete scan and human review |
| Partial traversal is mistaken for deletion | Traversal or metadata error quarantines without uncertain tombstones | Collection stays unavailable until one complete scan succeeds |
| Crash splits approved DB and bundle state | Durable publication intent resumes idempotently before exposure | Altered sources or bundle return to human intervention |
| Peer forges citation node | Receiver replaces `node_id` with the Noise-authenticated PeerId | A compromised source controls content under its own identity |
| Prompt injection in evidence | Local LLM only proposes metadata; remote diagnostics are discarded; MCP instructions label evidence as untrusted data | A chat model may still follow hostile snippets; validate the golden prompt set |
| Compromised WebView invokes ambient native authority | One local WebView, strict CSP, explicit top-level navigation allowlist, no direct native plugins, explicit closed Tauri commands, bounded validated DTOs; grants, external policy, repairs, model licenses, updater installation and external links require an OS-native Rust-owned confirmation | A flaw in an exposed command or WebView runtime can still request a native prompt; test hostile and unknown payloads and keep commands narrow |
| Web content reads or supplies arbitrary local paths | Native folder picker returns an opaque one-use token that expires after five minutes; Rust consumes it for add/relink and never accepts a JavaScript path | The selected folder remains a deliberate user grant; relinking still requires a fresh selection |
| Out-of-order IPC state enables a stale decision | Snapshot/event schema versions, monotonic sequences, request IDs, review versions and fingerprints; lagged consumers request a full snapshot | A compromised local process is outside the model; saturation, reconnect and stale-approval tests remain mandatory |
| Stale or mismatched evidence is shown during review | Worker requests bind request, concept, revision and an opaque review version; storage revalidates pending state, draft and all chunk evidence in the publication transaction; approval stays disabled without current evidence | A compromised local database remains outside the threat model |
| Published Markdown loads hostile resources | Viewer disables images, files, network, SVG, and embeds; external HTTP links require confirmation | User can still choose to open a URL |
| Bundle is modified outside the app | Bounded tolerant inspector, normalized in-root paths, no symlinks, DB/filesystem health report, confirmed snapshot repair | A local writer already controls data; ambiguous history remains blocked |
| Model grants permission | Enrichment schema excludes collection, grants, publication, and cloud policy | Review schemas and preserve human approval |
| Runtime or model is replaced | Pinned revision/hash, temporary download, atomic move, build manifest | Upstream or build-host compromise remains possible |
| Installer is altered | Native publisher signature, independent updater signature, hashes/SBOM, platform gates | Internal unsigned candidates are not public releases; signing-key or CI compromise remains possible |
| Release dependency captures credentials | Secret-free build, isolated signing/verification, protected environments, scoped credentials | Actions, scripts, and signing services remain in the TCB |
| Elevated helper opens broad rules | Closed arguments, sibling app, same-publisher validation, Firewall COM API, Private/Domain `LocalSubnet`, no edge traversal | Admin or domain policy can override; conflict and cancellation keep local-only mode |
| Untrusted executable invokes helper | Helper derives its sibling desktop and requires the same valid publisher | Compromised publisher key or build host breaks the guarantee |
| Update metadata or artifact is substituted | Fixed HTTPS manifest endpoint; hosting redirects remain untrusted; updater and native signatures, protected file handles, strict version increase and explicit installation confirmation protect the final bytes | Hosting still observes request metadata; compromised signing keys require incident response |
| One target is promoted before another passes | Draft assembly depends on both native signing jobs; promotion re-downloads and independently verifies macOS and Windows against one fingerprinted `SHA256SUMS` inventory before protected human approval | A compromised CI or signing service remains in the release TCB; never bypass protected environments |
| Update check leaks knowledge | No installation ID, PeerId, query, or content | Hosting provider still observes IP, user agent, and requested version |
| Device is stolen | Secrets remain in OS credential storage; originals are not replicated | Disk and session protection are OS responsibilities |
| Paired peer causes denial of service | Bounded payloads, deadlines, rate limits, one relevance inference per node | A trusted peer can consume bounded resources; revoke and block it |
| Logs leak sensitive data | Sanitized structured logging writes through one process-lifetime non-blocking guard; no queries or snippets by default | Review every new error boundary and sanitize before sharing |

## Invariants

- Publication always requires an explicit human action.
- Watchers and reconciliation prepare revisions but never approve them.
- Incomplete traversal fails closed and creates no uncertain tombstones.
- A changed source withdraws the previous publication before sharing the new one.
- The source node authorizes every disclosed search item and gates the evidence
  lane by answerability. Rejected items may appear only as separately typed
  `external_ai` candidates.
- LAN accepts `/airwiki/search/2.0.0` for search and prefers
  `/airwiki/shared-wiki-browse/2.0.0` for the complete published, read-only OKF
  workspace. Browse v1 remains a summary-only compatibility fallback during
  rollout; neither version broadens the Wiki grant.
- Public catalog and search advertise v2 assurance metadata and fall back to
  v1 during rollout. Public browse advertises v4 for the complete published OKF
  workspace, generation-bound frames and fingerprint-bound pages, then falls
  back through v3 exact-result anchors and the cursor-compatible v2 and v1
  contracts. Frames from different generations are never merged, and graph
  edges must resolve to reconstructed published pages. The selected concept is
  revalidated after every adaptation. Absent legacy metadata is unknown, never
  verified. No version bypasses stable publication. Public browse cannot return
  original source files or paths, chunks, embeddings, the operational index, or
  edit operations.
- `external_ai` is never inferred from tags, classification, or model output.
- Original source files, local paths, embeddings, chunks, operational search
  indexes and peer-wide Wiki listings do not cross LAN. Opening a result may
  retrieve the complete published OKF workspace for only its exact currently
  granted Wiki; bounded transport frames are drained automatically.
- Bundle visualization is read-only. Graph layout and interaction are local and
  cause no polling after the bounded published graph has loaded.
- Unknown OKF fields/types are preserved; invalid optional metadata cannot raise
  trust, authorize disclosure or enable execution.
- Future OKF versions remain local-only. Legacy v0.1 remains readable but cannot
  create new publication.
- Applying the v0.2 compatibility gate removes legacy/future LAN and application
  grants, disables their continuous indexing, and durably retries signed v1/v2
  public tombstones without deleting their local bundle or source folder.
- Application memory capability is independent from LAN, public and external
  search policy; assistants cannot delete, share, grant or assert verification.
- Human verification of managed concepts requires a Rust-owned native prompt
  and the exact current fingerprint; changed permissions or malformed existing
  verification metadata fail closed.
- `airwiki-wasm` provides no host imports. Execution and saving each require a
  separate Rust-owned native confirmation; raw parameters and receipts expire
  from memory after ten minutes.
- Guided repair withdraws affected revisions first and never restores exposure
  without new approval.
- Revocation deletes grants and closes active connections.
- Hiding preserves background work; **Exit completely** stops it and the model
  runtime.
- Autostart and remote update checks require explicit consent.
- The firewall helper never opens MCP, Public profile, Internet, file sharing, or
  global network discovery.
- Public publishers reserve relays through outbound connections; public
  manifests contain only routes backed by a live reservation, and public
  federation never installs a Windows Public-profile rule.
- Updates require valid signatures and confirmation and are never silent.

## Current development blockers and residual risks

- No complete cross-platform public candidate has passed the protected release
  workflows and installed acceptance matrix. Windows public-trust enrollment,
  final SignPath configuration and native verification remain required before
  any public release.
- Trust and grants are per device, not per individual user.
- LAN assumes one private subnet and cannot overcome router multicast policy.
- OCR and advanced malicious-file analysis are out of scope.
- Cloud exposure remains deliberate for explicitly approved snippets.
- Loopback does not isolate processes running under the same user account.
- Relevance classification has residual false-positive and false-negative risk.
- A compromised or modified trusted source node can mislabel its own v2 results.
- Public federation still needs dedicated review and installed cross-NAT
  acceptance before promotion; bootstrap and relay hosts are validation
  infrastructure, not a supported availability promise.
- Bootstrap entries are versioned and expire. Community indexes remain
  replaceable, and an expired validation endpoint is ignored rather than being
  treated as an authority.
- The public beta uses two independent Azure nodes without guest telemetry or
  remote configuration. Budget alerts are operational signals, not a hard
  spending cap; the documented emergency control is bootstrap revocation and
  deletion of both dedicated resource groups.
- Local publisher blocks are checked through the final result-delivery boundary
  and contain only the stable public publisher identity; they do not notify the
  publisher.

## Temporary dependency-audit exceptions

These exceptions are narrow, visible, and reviewed before every candidate. They
must not be copied to another product.

| Advisory | Dependency path | Limited rationale | Removal condition |
| --- | --- | --- | --- |
| `RUSTSEC-2026-0118` | `libp2p-mdns → hickory-proto 0.25.2` | Hickory is built only through `libp2p-mdns`; the affected DNSSEC validator is absent | Compatible libp2p/hickory update or feature change |
| `RUSTSEC-2026-0119` | `libp2p-mdns → hickory-proto 0.25.2` | `libp2p-dns` and `hickory-resolver` are not built; outbound encoding is limited to bounded application-owned mDNS service records | libp2p adopts `hickory-proto >=0.26.1` or attacker input reaches the encoder |
| `RUSTSEC-2024-0436` (unmaintained) | `tokenizers → macro_rules_attribute → paste 1.0.15` | Maintenance warning, not a reported vulnerability; remains visible | Upstream removes or replaces `paste` |

`cargo-audit` passes only the two scoped hickory exceptions explicitly.
`cargo-deny` evaluates the two tested targets under the same constraint. Any
new advisory fails CI.

## Disclosure response

1. Exit affected nodes and chat clients.
2. Revoke the affected device identity and delete its grants from a trusted node.
3. Disable `allow_external_ai` on every collection.
4. Disconnect local chat integrations. If an advanced tunnel was used, disable
   it and rotate its API key outside AirWiki.
5. Preserve sanitized logs, audit events, hashes, and affected revisions.
6. Handle cloud-delivered snippets under the provider workspace's incident policy.
