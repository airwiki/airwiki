# Installing and running AirWiki

This guide covers the tested development targets: macOS arm64 and Windows
10/11 x64 with AVX2. LAN collaboration requires both devices on the same private
subnet with multicast available. Hiding the window keeps the node running;
**Exit completely** stops it.

AirWiki is still a development candidate. Until an official release
exists, use only artifacts supplied through an agreed private channel and verify
their SHA-256 independently. Do not bypass Gatekeeper, SmartScreen, model hashes,
or runtime verification.

## Before installing

1. Keep at least 7 GiB free on a 16 GiB Mac or 5 GiB on an 8 GiB Windows PC.
   The application calculates the exact missing-assets requirement plus a 1 GiB
   safety margin.
2. Use only the synthetic fixtures until the two-node runbook passes.
3. On Windows, confirm AVX2 support and a Private or Domain network profile.
4. Treat unsigned internal candidates as development artifacts, never as public
   releases.

## macOS arm64

1. Open the DMG and move **AirWiki** to Applications.
2. For an internal candidate, compare its hash through the agreed channel. A
   future public release must pass Developer ID signing and notarization.
3. The onboarding wizard explains local-network collaboration before requesting
   permission. Grant it only when LAN search is desired.
4. Review the hardware recommendation, licenses, and remaining download size.
5. Allow the application to prepare only the selected local model assets.

No daemon, system service, or Internet-facing port is installed. Optional
per-user autostart uses `SMAppService` only after consent.

## Windows x64

1. Run the per-user NSIS installer from an interactive desktop session.
2. Verify Authenticode when using a signed candidate. An unknown-publisher
   warning means the artifact is not a validated public release.
3. Keep LAN disabled on a Public network. On a Private or Domain network, the
   wizard may request UAC to install exactly two inbound rules owned by AirWiki:
   application TCP and mDNS UDP 5353, both limited to `LocalSubnet` and
   never enabled for the Public profile.
4. Complete hardware diagnosis and approve the selected local assets.

AirWiki does not enable Windows Firewall globally, change “block all
incoming connections,” or delete inherited broad rules. Resolve those states in
Windows settings or with the device administrator, then select **Check again**.
Cancelling UAC leaves the node in local-only mode.

Uninstall keeps local data by default. Removing application data and managed
firewall rules are separate, explicit options.

## First start and local models

The application diagnoses hardware, recommends a profile, verifies installed
assets, and downloads only missing files. Model identities are immutable by
revision and hash; an updated artifact receives a new catalog identity and never
silently replaces the active model.

| Profile | Intended behavior |
| --- | --- |
| Automatic | Select the best verified profile supported by current memory, CPU, architecture, and available disk. |
| Efficient | Prefer the smaller supported generative model. |
| Quality | Prefer the larger supported model and visibly fall back when hardware is insufficient. |

A clean installation prepares one generative model, multilingual embeddings,
and the local relevance classifier. A verified legacy Qwen installation may be
retained as a fallback but is not downloaded on a clean install. Candidate model
updates become active only after verification and restart; a failed smoke test
keeps the previous model.

The pinned model revisions, filenames, sizes, hashes, and platform constraints
live in the `airwiki-inference` catalog and its tests. Build manifests authenticate
the packaged llama.cpp runtime. Documentation does not duplicate those mutable
catalog values.

Before enabling search, startup:

1. verifies installed hashes and runtime provenance;
2. loads embeddings and the relevance classifier;
3. runs local embedding, relevance, and enrichment smoke tests; and
4. reconciles every watched collection.

LAN and MCP remain closed when any required step fails.

## Collection automation and reconciliation

Filesystem watchers reduce latency but are not a source of truth. AirWiki
runs a complete idempotent scan at startup and every 15 minutes while the process
and required models remain ready.

- Repeating a scan over unchanged content creates no duplicate revisions, jobs,
  or tombstones.
- An incomplete traversal quarantines the collection and withdraws it from
  search, but does not interpret uncertain absences as deletions.
- A changed source withdraws the published revision before processing the new
  content.
- Extraction, enrichment, indexing, and bounded retries may be automatic;
  publication always returns to explicit human review.
- A durable publication intent interrupted after approval may resume at startup,
  but remains unsearchable until SQLite and the complete OKF bundle agree.
- Wiki health may regenerate unambiguous derived artifacts. Content, history,
  permissions, and ambiguous damage require guided human recovery.

See [recovery](recovery.md) for failure paths.

## Create a synthetic collection

1. Copy `fixtures/mac` and `fixtures/windows` outside the repository on their
   respective devices.
2. Create one collection per copied folder.
3. Keep external chat disabled during the first local test.
4. Wait for automatic ingestion and review every proposal.
5. Publish only recognized synthetic content.
6. Enable peer sharing and grant only the synthetic Atlas collection when the
   two-node test requests it.

Use the [two-node runbook](two-node-runbook.md) for acceptance. A visible screen
is not evidence by itself; verify the stated effect.

## Connect a local chat client

After preflight is ready, open **Integrations** and refresh detection. AirWiki can register ChatGPT Desktop/Work and Gemini CLI through supported CLIs and
open Claude's MCPB installer. Every configuration change shows the client,
versioned bridge, and cloud boundary before confirmation.

**Allow in external chats** is independent from **Share with authorized peers**.
Connecting a client never enables a collection. See
[local chat integrations](chat-integrations.md).

## Background operation, autostart, and updates

Tray operation, per-user autostart, and update checks are applied only after
onboarding consent and can be disabled later.

- macOS registers the bundled per-user agent through `SMAppService`.
- Windows registers one exact command under the current user's Run key.
- No service survives logout.
- Updates require confirmation and never change collections, permissions, or
  models implicitly.
- Version checks send no documents, queries, PeerIds, or installation identifier.
- Offline checks are non-blocking; invalid signatures and downgrades fail closed.

If a watched root becomes unavailable, AirWiki quarantines the collection,
retries the watcher, and performs a new scan when access returns. Recovered
documents require review before publication.

## Windows uninstall

The uninstaller removes autostart only when the stored command exactly matches
the AirWiki entry. It can optionally request UAC to remove the two exact
managed firewall rules. Cancelling that request does not block uninstall.

Local data is also retained unless explicitly selected for deletion. Watched
source folders are never application-data paths and are never removed.

## Local data

Typical application-data roots are:

- macOS: `~/Library/Application Support/io.github.airwiki.AirWiki/`
- Windows: `%LOCALAPPDATA%\airwiki\AirWiki\data\`

SQLite, identity references, generated OKF bundles, and logs live under the
system application-data location. Original documents remain in the user-selected
folder and must never be deleted as an index-repair shortcut.
