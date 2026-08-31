# AirWiki beta FAQ and known limitations

## Is AirWiki ready for daily or production use?

No. AirWiki is an active-development technical beta with no supported stable download. Public pre-releases are manual test candidates, not an updater channel or a platform-trust promise. Do not use them as the only copy of important knowledge. Read [docs/install.md](docs/install.md) before evaluating one.

## Which desktop platforms can I evaluate?

The technical beta targets macOS 13+ on Apple silicon and Windows 10/11 x64 with AVX2. Linux desktop, web, and mobile clients are not available. The Linux federation-index artifact is a maintainer service, not a desktop application.

## Is my knowledge uploaded by default?

No. New Wikis are private from LAN and Internet, and AirWiki has no default cloud sync or centralized knowledge service. Some explicit choices can send data or metadata outside the device, including model downloads, LAN grants, experimental publication, public search, and connected clients. See [PRIVACY.md](PRIVACY.md) for the exact boundaries.

## Can I test it with real work documents?

For beta evaluation, use synthetic or non-sensitive material. The product is not yet a supported release, and public feedback channels must never receive real documents, queries, logs, screenshots, paths, identities, or credentials. Keep your own backups; AirWiki is not a backup product.

## How do I report a problem or suggest an improvement?

Use the public forms in [SUPPORT.md](SUPPORT.md) with a minimal synthetic reproduction. Report potential vulnerabilities or exposure privately using [SECURITY.md](SECURITY.md), never through a public issue.

## Why does search say that local AI is not ready?

Search stays unavailable until the required local assets pass verification and startup checks. Model preparation can require substantial disk space, network access, and time. You may complete onboarding before preparing the model, but that does not make local search ready; follow the recovery guidance in [docs/install.md](docs/install.md#first-start-and-local-models).

## Can I share a Wiki with another device or publicly?

Only reviewed, current knowledge can cross a sharing boundary, and only after explicit human action. LAN access needs both device verification and a specific Wiki grant. Experimental public federation is separate and opt-in. Neither choice uploads your original folder contents or automatically shares future material. See [docs/search-and-federation.md](docs/search-and-federation.md).

## Known limitations

- No supported stable download, signed public macOS installer, Windows public-trust signature, or active stable updater channel exists yet.
- The beta has not completed the installed acceptance matrix or five-person nontechnical onboarding study required for public release.
- Onboarding can finish before local models are prepared; search then remains unavailable until preparation succeeds.
- The app does not provide OCR, DOCX ingestion, image/audio/video processing, cloud sync, accounts, SSO, source-document replication, arbitrary remote editing, automatic Git operations, a system daemon, silent updates, or web/mobile access.
- Public federation is experimental and has no supported always-on public relay service.
- There is no published Code of Conduct enforcement contact yet. This is a documented public-release blocker, not an invitation to report incidents in public issues.

The release gates and remaining work are tracked in the [release checklist](docs/release-checklist.md).
