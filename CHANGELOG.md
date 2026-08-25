# Changelog

All notable user-visible changes to AirWiki will be documented here. The project has no supported public release yet, so current work remains under `Unreleased`.

## [Unreleased]

- Added a 30-day unsigned Windows technical-beta artifact with exact workflow
  provenance, per-installer SHA-256 values, bilingual safety guidance and a
  fail-closed separation from GitHub Releases, the updater and SignPath
  promotion.
- Replaced the permanent system footer and combined connection drawer with a
  full-screen Settings workspace for General, Connections and AI apps. A
  three-segment accessible status donut, pending-approval badge and non-modal
  approval notice keep uncommon system work visible without crowding Library.
- Renamed the primary Wiki inventory to Library and grouped search results by
  origin, owner and Wiki, with bounded concept previews, counted origin filters,
  partial-coverage guidance and public search remaining explicit per query.
- Gave the desktop a distinct local-library visual language: hyperlegible body
  type, layered folio marks, a calmer archive palette, useful catalog context
  and one continuous Settings surface instead of a stack of generic cards.
- Refined desktop information density with compact Wiki and Library headers,
  richer Wiki rows, structured integration and device records, and loading
  skeletons that preserve layout during initial discovery.
- Fixed global-search focus loss, aligned compact toolbar and dialog actions,
  and distinguished queued model preparation from an active download.
- Matched the minimum desktop window size to its default 1180 × 760 opening
  size so the primary workspace cannot be compressed below its usable layout.
- Kept Wiki recovery actions fully visible at the minimum window size, restored
  the canonical transparent AirWiki mark across the app header, native window,
  executable and package icons, and made queued Local AI cancellation clear its
  pending state and feedback immediately.
- Added optional bounded Wiki presentation to search hits. LAN reveals only the
  exact authorized Wiki name and compatibility, while public presentation is
  derived only from the validated signed manifest; older clients remain
  compatible and neutral fallbacks never expose UUIDs.
- Refocused first run on a real first Wiki, added a direct empty-state action,
  surfaced time-limited AI approvals before the Wiki list, made search scope
  reflect currently authorized nearby devices, and clarified saved versus
  immediate settings and sharing changes.
- Replaced generic AI-client pictograms with the original ChatGPT, Codex,
  Claude Desktop, Claude Code and Gemini CLI artwork while retaining a neutral
  fallback for unknown MCP clients.
- Identified trusted nearby devices by name, operating system and availability
  in search, sharing and Connections, while keeping public publishers visually
  separate and withholding private device metadata from public federation.
- Fixed the missing AirWiki system-tray icon on macOS and Windows, with right-click reserved for the tray menu.
- Added an explicit, fail-closed recovery action after a device verification
  code is rejected; retrying never restores trust or wiki grants.

### Added

- Added portable project memory under `.airwiki`: explicit native
  initialization, per-clone application approval, bounded local lexical search,
  ordinary diff-friendly OKF concept files and non-destructive detach. Personal
  assistant memory remains in AirWiki's private vault.
- Added fail-closed project-memory reconciliation at startup and on filesystem
  changes. Missing, invalid or identity-conflicting bundles retain their files
  but cannot serve MCP, LAN, public or external-AI results.
- Added first-class folder, imported OKF, and assistant-memory Wiki origins,
  including continuous/manual indexing, atomic ZIP or directory import, safe
  deletion, and hierarchical OKF browsing.
- Added complete OKF v0.2 provenance, lifecycle, freshness, verification,
  compatibility, open concept types, and constrained `airwiki-wasm` attested
  computation support while retaining fail-closed v0.1 reading.
- Added a versioned global AirWiki skill and assisted-memory guide for
  ChatGPT/Codex, Claude Code and Gemini CLI, installed together with MCP after
  one native confirmation. Claude Desktop and generic clients receive the same
  safe list/read/write-with-fingerprint workflow through MCP instructions.
- Added Claude Code as a first-class, separately authorized application with a
  user-scoped stdio configuration and isolated Wiki ownership.

- Added a bounded two-region Azure beta deployment for the public federation
  index/relay, with a live USD 50 monthly cost gate, per-node budgets,
  availability alerts, hardened single-node services, private versioned
  bootstrap packaging, durable expiry and downgrade protection, sanitized
  status, failover, recoverable single-node replacement and whole-group
  retirement.
- Added experimental opt-in public federation with separate publisher identity, signed expiring routing manifests, replaceable indexes, QUIC/relay transport, owner-served search and paginated browse.
- Added expiring versioned bootstrap metadata, local publisher blocking, bounded fan-out pruning, and sanitized direct/relay and announcement status for public federation.
- Added a complete public collection card with profile, provenance, expiry, route, paginated concept metadata, and explicit advertised/expired/offline states.
- Initial development baseline for local ingestion, human-reviewed OKF publication, LAN federation, and read-only local chat integration.
- Contributor guidance, architecture decisions, synthetic fixtures, and proportional validation commands.
- A proportional code-review contract, pull-request template, and read-only DCO validation for contributions.
- Shared AirWiki branding for GitHub, application windows, tray icons, and desktop packages.
- A guided first-knowledge journey from local AI setup through folder processing, human review, publication, and a cited local search.
- A revision-bound evidence panel that lets reviewers compare locally extracted text with each proposed draft before publication.
- Direct navigation from a local search result to its exact published Wiki concept, with the cited source location and revision kept visible.
- A reproducible retrieval-quality corpus and evaluator covering local and peer-authorized source evidence, abstention, provenance, privacy, contradictions, deduplication, and stable ordering.
- A preregistered bilingual selector-adaptation corpus and structural validator that keep model inputs separate from evaluation labels, permissions, and provenance metadata.
- A durable negative-result record for the rejected adapted mMARCO selector, including its one-time aggregate promotion metrics and no-rerun decision.
- A separately typed external-chat candidate lane that lets a capable consumer inspect authorized passages rejected by AirWiki's lightweight answerability classifier without weakening publication or disclosure policy.

### Changed

- Migrated the desktop application, lifecycle, packaging, and updater to Tauri
  v2 with a typed Svelte frontend while keeping domain and authorization rules
  in Rust.
- Reframed the desktop as one familiar, file-oriented Wiki workspace with
  global search, contextual review and sharing, a compact system-status bar,
  and advanced diagnostics kept out of the normal path. Review keeps evidence,
  proposal, and the publish decision in one explicit sequence.
- Hardened public federation handoff and admission so withdrawal remains
  linearized through transport delivery, remote identity churn is memory-bound,
  and index manifests/tombstones have bounded lifetime and global capacity.
- Clarified privacy boundaries and separated internal development packages from future public distribution.
- Consolidated durable project guidance and documentation ownership.
- Improved the public project summary and contributor entry points.
- Kept LAN, chat clients, background operation, and updates out of the first useful-result path; they remain optional and require their existing confirmations.
- Reused a successful model integrity check for presentation state instead of hashing the same immutable assets twice during startup.
- Showed the local-model recommendation while private services finish starting on fresh installs, without weakening asset verification.
- Made the desktop layout responsive down to its explicit minimum size, keeping primary actions visible and limiting scrolling to bounded lists and detail panels.
- Made the first-run journey recoverable with visible processing counts, actionable terminal states, a truthful Wiki-health timestamp, and an explicit way to finish after a search returns no evidence.
- Made vector retrieval scan each collection with cursor pagination and hydrate full evidence only for the bounded candidate set, so query work no longer grows through repeated SQLite offsets or corpus-wide text loading.
- Separated direct answers, allowed supporting context, and forbidden evidence in the retrieval evaluation corpus, and corrected an Atlas question that previously depended on hidden fixture context; production search behavior and thresholds are unchanged.
- Kept local desktop search conservative while allowing external-chat consumers to evaluate bounded candidates independently; evidence wins duplicates and candidates are discarded first when a LAN response reaches its size limit.
- Refined the desktop visual system with an evidence-oriented palette, clearer type hierarchy, roomier controls, and contrast-tested primary and secondary text in dark and light themes.
- Completed the English presentation boundary for worker notices and advanced diagnostics so changing locale no longer leaves Spanish operational text in the interface.

### Fixed

- Made the normal search scope explicitly include the current device and
  authorized nearby devices, with public search as a separate opt-in. LAN and
  public results now open in one consistent, read-only Wiki workspace instead
  of using disconnected result and public-card experiences. Result provenance
  remains explicit even when a public publisher is also a paired device.
- Kept unsaved device preferences stable while background connectivity and
  status snapshots arrive, so a user's selection is never reverted before
  they can save it.
- Opened public search results in a dedicated, keyboard-accessible wiki viewer
  with explicit loading, route, OKF assurance, failure, pagination and return
  states instead of leaving the completed browse snapshot invisible.
- Replaced the WiX sample text in Windows installer license dialogs with the
  versioned Apache-2.0 terms and made packaging reject missing, stale, or
  placeholder license RTF input.
- Kept the desktop navigation rail fixed in place and made every System
  subsection an independently routed page, removing section jumps that could
  displace or freeze the application shell.
- Restored process-lifetime sanitized desktop logging, honored background
  autostart without exposing an inaccessible process, rejected remote WebView
  navigation, and moved sensitive desktop consent to Rust-owned native dialogs.
- Made release packaging prove that the configured updater key is embedded in
  the candidate binary, unified executable/package icons, and added mandatory
  frontend and npm supply-chain gates to pull-request CI.

- Prevented a concurrent filesystem preflight from restarting an active
  same-content ingestion, so restoring a withdrawn source completes in one
  enrichment pass while interrupted jobs remain retryable after restart.
- Replaced the installed-model activation enrichment with a bounded strict-JSON
  health probe, while retaining the full production enrichment budget and its
  installed-candidate acceptance journey.
- Made the installed Windows smoke fail immediately with a closed, durable
  model-installation stage when asset preparation fails before activation,
  instead of hiding the failure behind the general model-readiness timeout.
- Preserved the full bounded owner-response budget after a cross-region public
  catalog query and cold relay substream negotiation instead of aborting an
  accepted relay circuit early, and applied the complete one-second index
  budget to catalog updates.
- Separated the cold public-owner connection budget from its response budget,
  and made Direct/Relay evidence request-scoped so only a protocol-valid owner
  response on the matching connection can establish the reported route. This
  replaces the unsafe process-global reader route getter with request-scoped
  results while preserving the route-free streaming search API.
- Limited signed publisher manifests to relay routes with a live outbound
  reservation, and made readiness changes immediately reannounce the remaining
  routes or withdraw the manifest when none are usable. Announcement status is
  sequence-aware so a slow older update cannot replace newer local state, and
  renewal cancellation no longer waits through every pending catalog update.
- Kept local, LAN and MCP services available when an older installed candidate
  carries a bootstrap registry below the persisted high-water mark. The newer
  registry remains authoritative, while same-version mutation and invalid
  registries continue to fail closed.
- Gave CPU-only Windows model activation an explicit bounded request class so
  the supported Qwen profile is not held to the accelerated-generation
  deadline during its startup smoke test.
- Allowed fresh installations to defer the optional local-model download so public read-only discovery is not blocked by unrelated inference setup.
- Made public relay reservations advertise only operator-confirmed external addresses and covered owner search and browse through an outbound relay circuit.
- Made public source listeners recover automatically after rapid unpublish and republish by closing relay connections and waiting until every TCP and QUIC socket is reusable before starting the replacement swarm.
- Made public index outages report partial or offline state accurately instead of appearing as a complete empty search, and made public browse reject empty unreviewed or withdrawn revisions.
- Preserved both evidence and authorized-candidate lanes in flattened MCP output, and improved localized, width-safe diagnostics for source files that could not be processed.

- Made the first pending review immediately selectable and localized model diagnostics used by the normal setup flow.
- Showed files that could not be prepared alongside review drafts with a safe, localized cause and a folder rescan action.
- Removed flaky filesystem-watcher and reranker timing assumptions from macOS CI and made Windows hashing tests load their required PowerShell module explicitly.
- Replaced the GitHub advisory action that required unavailable check-write permission with a read-only `cargo audit` gate.
- Kept Wiki Health recovery actions honest: filesystem and ambiguous findings now require manual recovery instead of offering a guided repair that cannot complete, while missing pages are no longer rendered as dead links.

### Security

- Made workflow-guide installation receipt-bound, atomic and fail-closed:
  documented user roots only, no links or reparse points, preserved BOM and line
  endings, no overwrite or removal of user-modified files, and immediate
  capability revocation on disconnect.

- Added 15-minute, identifier-free relay outcome counts grouped by a fixed
  status allowlist, without reader-success events, per-event timestamps or
  guest log shipping; circuit I/O failures retain only fixed error-kind buckets.
- Made a completed local publisher block a final disclosure barrier for
  in-flight public search hits, partial updates, route evidence and browse
  responses, including manifests already held in the local route cache.
- Rejected malformed public rankings before RRF fusion and made early fan-out cancellation account conservatively for every accumulated challenger.
- Hardened Azure relay installation with exact binary checks, shared public-route preflight, and fail-closed service replacement.

- Sanitized desktop, LAN, MCP, and inference diagnostics so default logs and technical-detail panels expose stable error categories instead of peer IDs, request IDs, collection IDs, endpoints, local paths, or raw backend errors.
- Supervised model lifecycle and authorized-search tasks through owned Tokio task sets so shutdown cancels and joins in-flight work deterministically.
- External-chat access, peer sharing, grants, publication, and ambiguous repairs remain explicit human decisions and fail closed when authorization is uncertain.
- Remote search results remain read-only evidence cards and never expose a local Wiki navigation action.
- External-chat candidate output is revalidated at every disclosure boundary and reduced below the bridge transport budget, discarding candidates before evidence when necessary.
