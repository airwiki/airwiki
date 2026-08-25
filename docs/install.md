# Installing and running AirWiki

This guide covers the tested development targets: macOS arm64 and Windows
10/11 x64 with AVX2. LAN collaboration requires both devices on the same private
subnet with multicast available. Hiding the window keeps the node running;
**Exit completely** stops it.

AirWiki is still a development candidate. When an official release exists, use
only the signed installers on the
[latest GitHub release](https://github.com/airwiki/airwiki/releases/latest) and
verify them against its `SHA256SUMS`. Until then, use only an agreed development
candidate and verify its provenance and SHA-256 independently. Never disable
Gatekeeper, SmartScreen, Smart App Control, antivirus, model hashes or runtime
verification to make a candidate run.

## Before installing

1. Keep at least 7 GiB free on a 16 GiB Mac or 5 GiB on an 8 GiB Windows PC.
   The application calculates the exact missing-assets requirement plus a 1 GiB
   safety margin.
2. Use only the synthetic fixtures until the two-node runbook passes.
3. On Windows, confirm AVX2 support and a Private or Domain network profile.
4. Treat unsigned internal candidates as development artifacts, never as public
   releases.

## Windows unsigned technical beta

The manual
[Package unsigned pilot](https://github.com/airwiki/airwiki/actions/workflows/package-pilot.yml)
workflow can produce a 30-day Windows x64 artifact for invited technical
testing while SignPath enrollment is pending. It runs on a GitHub-hosted Windows
builder and does not receive signing or updater credentials. It is not attached
to GitHub Releases, selected by `latest.json`, or supported as a public package.

For each candidate:

1. Open the successful workflow run for the reviewed commit and download the
   `airwiki-windows-x64-unsigned-beta-<commit>` artifact. GitHub sign-in and
   repository read access are required.
2. Read `UNSIGNED-BETA.txt` and confirm the repository, complete commit, version,
   workflow-run URL and expiry in `PROVENANCE.json`.
3. Compare both installers with the SHA-256 values in the workflow summary and
   `SHA256SUMS.txt` before opening either MSI:

   ```powershell
   Get-Content .\SHA256SUMS.txt
   Get-FileHash -Algorithm SHA256 .\<installer>.msi
   ```

4. Use a non-production test account or device. Do not disable or relax any
   platform or organization protection. If Windows offers a per-file
   confirmation and local policy permits unsigned beta software, use it only
   after the hash matches. If Smart App Control or organization policy blocks
   the installer, stop.
5. Choose the MSI language, follow the Windows steps below, and report the
   candidate commit with reproducible PASS/FAIL facts. Do not attach documents,
   queries, identities, addresses, credentials, databases or raw logs.

Every workflow run is independent. Never reuse a checksum from an earlier beta,
and never redistribute the extracted MSI as though it were an AirWiki release.

## macOS arm64

1. Open the DMG and move **AirWiki** to Applications.
2. For an internal candidate, compare its hash through the agreed channel. A
   future public release must pass Developer ID signing and notarization.
3. When replacing one ad-hoc internal candidate with another, macOS can ask
   whether the new build may access AirWiki's existing device identity in the
   login Keychain. Authorize that access only after verifying the candidate
   hash. Cancelling fails closed and never creates a replacement identity.
4. The onboarding wizard offers a direct path to a first local folder Wiki.
   Local-network collaboration remains disabled until you choose it later in
   **Settings**; grant the system permission only when LAN search is desired.
5. Review the optional hardware recommendation, licenses, and remaining
   download size. You can finish onboarding and prepare local AI later.
6. Allow the application to prepare only the selected local model assets.

No daemon, system service, or Internet-facing port is installed. Optional
per-user autostart uses `SMAppService` only after consent.

## Windows x64

1. Run the per-user MSI installer from an interactive desktop session.
   If Microsoft Edge WebView2 is absent, the installer downloads Microsoft's
   bootstrapper and installs the runtime before AirWiki. A network failure is
   reported without a partial AirWiki install and can be retried; the package
   intentionally does not embed the offline WebView2 runtime.
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

Windows Installer removes only immutable application files. Local data is kept
by default. Remove managed firewall rules from AirWiki before uninstalling when
that access is no longer wanted; cancelling UAC keeps the rules and does not
affect the application removal.

## First start and local models

The application diagnoses hardware, recommends a profile, verifies installed
assets, and downloads only missing files. Model identities are immutable by
revision and hash; an updated artifact receives a new catalog identity and never
silently replaces the active model.

AirWiki revalidates installed model files before enabling local search. Large
profiles can take several minutes to hash and start, especially in a development
build. If preparation is requested during that check, Settings shows it as a
queued request rather than false download progress. Cancelling removes that
queued download request immediately; the read-only check already in progress may
still finish and safely enable the model that was already installed.

| Profile | Intended behavior |
| --- | --- |
| Automatic | Select the best verified profile supported by current memory, CPU, architecture, and available disk. |
| Efficient | Prefer the smaller supported generative model. |
| Quality | Prefer the larger supported model and visibly fall back when hardware is insufficient. |

A clean installation prepares one generative model, multilingual embeddings,
and the local relevance classifier. On the minimum-memory Windows profile,
Automatic and Efficient select the smaller pinned Qwen model so the complete
installed acceptance journey remains representative; systems with more memory
select the eligible Gemma profile. Candidate model updates become active only
after verification and restart; a failed smoke test keeps the previous model.
Activation uses one compact, bounded strict-JSON health probe. The catalog's
production enrichment budget and hardware-specific request deadline remain
unchanged. The activation probe does not replace installed validation of real
enrichment, human review and publication, and MCP retrieval from a synthetic
document.

The pinned model revisions, filenames, sizes, hashes, and platform constraints
live in the `airwiki-inference` catalog and its tests. Build manifests authenticate
the packaged llama.cpp runtime. Documentation does not duplicate those mutable
catalog values.

Before enabling search, startup:

1. verifies installed hashes and runtime provenance;
2. loads embeddings and the relevance classifier;
3. runs local embedding and relevance smoke tests plus the bounded strict-JSON
   generation health probe; and
4. reconciles every watched folder Wiki.

LAN and MCP remain closed when any required step fails.

## Wiki automation and reconciliation

Filesystem watchers reduce latency but are not a source of truth. AirWiki
runs a complete idempotent scan at startup and every 15 minutes while the process
and required models remain ready.

- Repeating a scan over unchanged content creates no duplicate revisions, jobs,
  or tombstones.
- An incomplete traversal quarantines the Wiki and withdraws it from
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

## Create or import a synthetic Wiki

1. Copy `fixtures/mac` and `fixtures/windows` outside the repository on their
   respective devices.
2. Select **New Wiki → From a folder** for each copied folder. Leave continuous
   indexing enabled for this acceptance journey; disabling it stops discovery
   of new files but keeps manual update available.
3. To validate portable knowledge separately, use **New Wiki → Import OKF** and
   review the validation summary before AirWiki copies the bundle. Imported
   Wikis have no source watcher and remain managed from their OKF content.
4. Keep external chat disabled during the first local test.
5. Wait for automatic ingestion and review every proposal.
6. Publish only recognized synthetic content.
7. Enable peer sharing and grant only the synthetic Atlas Wiki when the
   two-node test requests it.

Use the [two-node runbook](two-node-runbook.md) for acceptance. A visible screen
is not evidence by itself; verify the stated effect.

## Connect a local chat client

After preflight is ready, open **Settings → AI apps** and refresh detection. AirWiki
can register ChatGPT Desktop/Work, Codex, Claude Code and Gemini CLI through
supported CLIs, and can open Claude Desktop's MCPB installer. For clients with
documented user skills, one confirmation installs the local MCP connection and
the global AirWiki workflow guide; the prompt lists every global file first.
Open a new conversation after installing or updating the guide.

**Allow in external chats** is independent from **Share with authorized peers**.
Connecting a client never enables a Wiki, sharing or publication. See
[local chat integrations](chat-integrations.md).

The Settings button uses three labelled status rings for local knowledge,
Connections and AI apps. Open **Settings → Connections** for LAN permission,
trusted devices, pairing, Wiki grants and advanced public configuration. Open
**Settings → General** for local-model preparation, appearance, language,
background behavior, startup and updates. A Back action restores the Library
state from which Settings was opened.

## Background operation, autostart, and updates

Tray operation, per-user autostart, and update checks are applied only after
their explicit choices in Settings and can be disabled later.

- macOS registers the bundled per-user agent through `SMAppService`.
- Windows registers one exact command under the current user's Run key.
- No service survives logout.
- Updates require confirmation and never change Wikis, permissions, or
  models implicitly.
- Version checks send no documents, queries, PeerIds, or installation identifier.
- Offline checks are non-blocking; invalid signatures and downgrades fail closed.

If a watched root becomes unavailable, AirWiki quarantines the Wiki,
retries the watcher, and performs a new scan when access returns. Recovered
documents require review before publication.

## Windows uninstall

Select **Quit completely** from the AirWiki tray menu before starting the
uninstall. Closing only the window may leave AirWiki running in the tray so it
can continue serving approved local integrations and network requests.

Windows Installer removes the fixed per-user program and Start-menu entry. The
application removes autostart only when the stored command exactly matches the
AirWiki entry. Disable LAN sharing in AirWiki before uninstalling to request
removal of the two exact managed firewall rules; cancelling that UAC request
does not block later uninstall.

Local data is retained. The MSI transition is not complete until an explicit,
separately confirmed data-removal path has passed the same containment tests as
the previous candidate. Watched source folders are never application-data paths
and are never removed.

## Local data

Typical application-data roots are:

- macOS: `~/Library/Application Support/io.github.airwiki.AirWiki/`
- Windows: `%LOCALAPPDATA%\airwiki\AirWiki\data\`

SQLite, identity references, generated OKF bundles, and logs live under the
system application-data location. Original documents remain in the user-selected
folder and must never be deleted as an index-repair shortcut.
