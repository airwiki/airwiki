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
- local models, runtime, and pinned artifact identities;
- snippets returned through LAN or MCP;
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
    expiring routing metadata only. Unpaired readers receive bounded snippets or
    concept summaries directly from the owner over Noise-authenticated QUIC or
    relay routes. The owner remains the authorization authority.

## Threats and controls

| Threat | Design control | Residual risk / validation |
| --- | --- | --- |
| Unauthorized LAN peer queries data | Noise, SAS pairing, trust state, per-collection grants, rate limits | Excessive grants still disclose data; test full grant and revocation matrix |
| Private or stale content reaches a public reader | Separate opt-in, reviewed-publication checks, signed sequence and fingerprint, final disclosure lease, immediate owner-side revocation | A third party can retain previously returned metadata or snippets |
| Malicious index redirects or ranks content | Expected index PeerId is pinned; owner manifests are independently signed; index rank selects routes but never final ranking | An index can omit publishers, delay tombstones, or degrade availability until replaced |
| Public queries exhaust an owner | Three-index, 64-candidate, 12-peer and two-collection caps; bounded payloads, semaphores, per-peer rate limits with a 1,024-identity window cap, a 1 s index stage, a 3 s cold owner-connection budget and a separate 800 ms owner-response budget | Bounded connection setup, local verification and ranking add work after catalog selection; distributed abuse can still consume bounded relay and inference capacity |
| Public registrations exhaust an index | Manifest remaining and signed lifetimes are at most 24 hours with bounded future skew; each SQLite catalog admits at most 100,000 total rows and 1,000 per publisher; expiry removes payload/FTS data but retains a compact sequence high-water mark, as do tombstones; compact rows remain for the node lifetime and count against both ceilings | An attacker can consume the bounded shared capacity and deny new registrations; operators monitor sanitized database and disk consumption and may replace or retire the index |
| Public route evidence is misattributed | Route state is request-scoped and accepted only when a protocol-valid owner response arrives within the separate connection and response budgets on the matching observed connection; an index connection, timeout or concurrent search remains offline; Relay takes precedence for mixed successful search routes | Direct and relay observations describe transports used by accepted responses, not future reachability |
| Publisher advertises an unavailable relay | Only relay listeners with a live outbound reservation enter signed manifests; readiness changes advance the sequence and immediately reannounce remaining routes or tombstone the collection; local status rejects late lower-sequence completions | Listener events can still race a network outage, but the owner remains fail-closed and the stale route can only degrade bounded availability |
| A blocked publisher completes an in-flight response | The local publisher block is rechecked before owner requests, while accepting and emitting search results, at final fusion and immediately before accepting browse content; a completed block excludes cached and in-flight disclosure | A user who explicitly unblocks the same identity restores future access |
| Public identity correlates readers | Readers use one ephemeral identity per application session; LAN and publisher identities are separate | Network observers and relays can still correlate IP and timing |
| Public beta infrastructure exceeds its operating budget | Two minimal independent nodes, live retail-price preflight, per-node monthly budgets, forecast/actual alerts, platform consumption review and whole-group retirement | Azure budgets alert but do not hard-cap spend; notification and cost data can lag |
| Public beta host or identity is replaced | Pinned image and binary hash, Azure-attested SSH host key, one persistent identity/database per node, compiled expiring registry and higher-version rotation | Older candidates can use only their unchanged or unexpired pinned entries |
| Pairing impersonation | Six-word SAS derived from identities and nonces, two-minute expiry | A user who skips comparison defeats the control |
| Private collection reaches cloud chat | `allow_external_ai` defaults off and is rechecked at the source | Human authorization can be wrong; use synthetic fixtures and audit |
| Ranking returns the least-wrong absent fact | Source node applies the pinned local answerability classifier to the bounded outgoing snippet; failures and timeouts close the path | The classifier is probabilistic; reassess both platforms when model, corpus, or policy changes |
| External chat treats an authorized candidate as relevant | Candidates exist only for `external_ai`, remain separately typed and bounded, pass the same final authorization checks, lose to duplicate evidence, and carry instructions requiring explicit support | A chat model can still misuse an unrelated authorized snippet; minimize externally enabled collections and run the golden prompt set |
| DNS rebinding reaches MCP | Loopback bind, exact authority including port, bounded body | Compromised local software can already call loopback |
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
| Update metadata or artifact is substituted | HTTPS transport plus updater and native signatures, protected file handles, signed version checks, explicit upgrade-only install | Public updater remains deferred; compromised signing keys require incident response |
| One target is promoted before another passes | Stable metadata is intended to be published only after both target gates and human approval | Promotion automation is archived and must be re-reviewed before use |
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
- LAN accepts only `/airwiki/search/2.0.0`; there is no v1 fallback.
- Public catalog, search and browse use separate v1 protocols and never bypass
  reviewed publication. Public browse cannot return originals, paths, chunks,
  embeddings or edit operations.
- `external_ai` is never inferred from tags, classification, or model output.
- Originals, local paths, embeddings, indexes, and collection listings do not
  cross LAN.
- Bundle visualization is local, read-only, and causes no network traffic.
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

- No public security contact, public signing identity, or
  active protected release workflow exists.
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
| `RUSTSEC-2026-0118` | `libp2p-mdns → hickory-proto 0.25.2` | Only the `mdns` feature is built; the affected DNSSEC validator is absent | Compatible libp2p/hickory update or feature change |
| `RUSTSEC-2026-0119` | `libp2p-mdns → hickory-proto 0.25.2` | Outbound encoding is limited to application-owned service records | libp2p adopts `hickory-proto >=0.26.1` or attacker input reaches the encoder |
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
