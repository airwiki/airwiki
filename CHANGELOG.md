# Changelog

All notable user-visible changes to AirWiki will be documented here. The project has no supported public release yet, so current work remains under `Unreleased`.

## [Unreleased]

### Added

- Added experimental opt-in public federation with separate publisher identity, signed expiring routing manifests, replaceable indexes, QUIC/relay transport, owner-served search and paginated browse.
- Added expiring versioned bootstrap metadata, local publisher blocking, bounded fan-out pruning, and sanitized direct/relay and announcement status for public federation.
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

- Made public relay reservations advertise only operator-confirmed external addresses and covered owner search and browse through an outbound relay circuit.
- Preserved both evidence and authorized-candidate lanes in flattened MCP output, and improved localized, width-safe diagnostics for source files that could not be processed.

- Made the first pending review immediately selectable and localized model diagnostics used by the normal setup flow.
- Showed files that could not be prepared alongside review drafts with a safe, localized cause and a folder rescan action.
- Removed flaky filesystem-watcher and reranker timing assumptions from macOS CI and made Windows hashing tests load their required PowerShell module explicitly.
- Replaced the GitHub advisory action that required unavailable check-write permission with a read-only `cargo audit` gate.
- Kept Wiki Health recovery actions honest: filesystem and ambiguous findings now require manual recovery instead of offering a guided repair that cannot complete, while missing pages are no longer rendered as dead links.

### Security

- Sanitized desktop, LAN, MCP, and inference diagnostics so default logs and technical-detail panels expose stable error categories instead of peer IDs, request IDs, collection IDs, endpoints, local paths, or raw backend errors.
- Supervised model lifecycle and authorized-search tasks through owned Tokio task sets so shutdown cancels and joins in-flight work deterministically.
- External-chat access, peer sharing, grants, publication, and ambiguous repairs remain explicit human decisions and fail closed when authorization is uncertain.
- Remote search results remain read-only evidence cards and never expose a local Wiki navigation action.
- External-chat candidate output is revalidated at every disclosure boundary and reduced below the bridge transport budget, discarding candidates before evidence when necessary.
