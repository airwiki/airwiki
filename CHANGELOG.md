# Changelog

All notable user-visible changes to AirWiki will be documented here. The project has no supported public release yet, so current work remains under `Unreleased`.

## [Unreleased]

### Added

- Initial development baseline for local ingestion, human-reviewed OKF publication, LAN federation, and read-only local chat integration.
- Contributor guidance, architecture decisions, synthetic fixtures, and proportional validation commands.
- A proportional code-review contract, pull-request template, and read-only DCO validation for contributions.
- Shared AirWiki branding for GitHub, application windows, tray icons, and desktop packages.
- A guided first-knowledge journey from local AI setup through folder processing, human review, publication, and a cited local search.
- A revision-bound evidence panel that lets reviewers compare locally extracted text with each proposed draft before publication.

### Changed

- Clarified privacy boundaries and separated internal development packages from future public distribution.
- Consolidated durable project guidance and documentation ownership.
- Improved the public project summary and contributor entry points.
- Kept LAN, chat clients, background operation, and updates out of the first useful-result path; they remain optional and require their existing confirmations.
- Reused a successful model integrity check for presentation state instead of hashing the same immutable assets twice during startup.
- Showed the local-model recommendation while private services finish starting on fresh installs, without weakening asset verification.
- Made the desktop layout responsive down to its explicit minimum size, keeping primary actions visible and limiting scrolling to bounded lists and detail panels.

### Fixed

- Made the first pending review immediately selectable and localized model diagnostics used by the normal setup flow.
- Showed files that could not be prepared alongside review drafts with a safe, localized cause and a folder rescan action.
- Removed flaky filesystem-watcher and reranker timing assumptions from macOS CI and made Windows hashing tests load their required PowerShell module explicitly.
- Replaced the GitHub advisory action that required unavailable check-write permission with a read-only `cargo audit` gate.

### Security

- External-chat access, peer sharing, grants, publication, and ambiguous repairs remain explicit human decisions and fail closed when authorization is uncertain.
