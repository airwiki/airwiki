# Candidate packaging

AirWiki produces internal development candidates with the pinned Tauri v2 CLI
and bundler. The protected public workflows add native signing, notarization,
updater signatures, exact release metadata and a human promotion gate. Their
existence does not make a candidate supported: every applicable item in the
[public release checklist](release-checklist.md) must pass first. See the
[release process](release-process.md) for repository configuration and operation.

A separate protected technical pre-release path may make unsigned test
candidates publicly downloadable without claiming stable support or entering
the updater channel. ADR 0012 defines that boundary, and ADR 0014 adds
GitHub-hosted build-provenance attestations for the exact final assets without
claiming native publisher identity.

Installers contain the desktop application, local MCP bridge, platform runtime,
licenses, and platform-specific integration assets. Model weights and future
multimodal projectors are downloaded and verified at first start; they are not
embedded in packages.

## Preparation

From the repository root:

```bash
# Node.js 24.15.0 is required.
corepack enable
corepack prepare pnpm@10.18.3 --activate
pnpm --dir apps/desktop/ui install --frozen-lockfile --ignore-scripts --prod=false
pnpm --dir apps/desktop/ui run licenses:check macos-arm64 # use windows-x64 on Windows
cargo run --locked -p xtask -- licenses check
```

`--prod=false` is mandatory so an inherited `NODE_ENV=production` cannot omit
the pinned Tauri, Svelte validation, and Vite build tools. Package wrappers fail
before runtime preparation when any of those tools is absent.
The UI manifest and `.npmrc` reject any Node or pnpm version other than the
exact pinned toolchain used by CI and package pilots.
The platform npm inventory must also match the installed frozen graph; generate
it only on the corresponding verified platform and review all metadata and legal
text changes.
Every pull request runs the frozen install, Svelte/TypeScript checks, ESLint,
Vitest/axe coverage, native-E2E typecheck, both platform license inventories and
a production-dependency advisory scan. Package pilots repeat the platform
inventory before bundling.

If the generated legal inventory is stale, regenerate it and review the complete
diff before packaging:

```bash
cargo run --locked -p xtask -- licenses generate
cargo run --locked -p xtask -- licenses check
```

Never change pinned runtime sources, revisions, hashes, build policies, or legal
texts merely to make packaging pass.

## macOS arm64 candidate

Run on macOS with Xcode Command Line Tools:

```bash
./packaging/package-macos.sh
```

The internal wrapper:

1. downloads or reuses the pinned llama.cpp archive and verifies its hash;
2. materializes the complete runtime under the ignored resource cache;
3. builds the desktop and MCP bridge for `aarch64-apple-darwin` with the lockfile;
4. signs the bridge ad hoc for development when no release identity is supplied;
5. builds a deterministic platform MCPB from those exact bridge bytes;
6. for a public build only, signs the already verified staging copies of every
   nested llama.cpp Mach-O with Developer ID, a secure timestamp and Hardened
   Runtime, while leaving the hash-pinned upstream cache unchanged;
7. builds and signs the Tauri application bundle and verifies that it contains
   exactly that staging payload; and
8. creates a licensed Tauri DMG and copies the verified `.app` and `.dmg`
   artifacts under `target/packages/macos`.

After stapling the application, the public release wrapper rebuilds the DMG so
the installer and updater contain identical application bytes. It restores only
the validated EULA resource entries; image-specific block maps and property
resources are never copied from the previous DMG. The rebuilt image must pass
`hdiutil verify` before signing and again after notarization.

The wrapper validates architecture, Mach-O identity, bridge bytes, MCPB layout,
runtime closure, legal files, and traversal safety. A development ad-hoc seal is
not Developer ID signing and must never be presented as public trust.
Because each ad-hoc build has no stable Developer ID identity, macOS can require
the interactive user to reauthorize access to an existing AirWiki Keychain item
after an internal upgrade. The candidate must preserve that item, wait without
blocking the async runtime, and fail closed if access is denied; deleting or
silently replacing the device identity is never an acceptance shortcut.

For repeated installed user-testing candidates, use one stable Apple Development
identity so subsequent builds keep the same Keychain trust boundary:

```bash
AIRWIKI_SIGNING_PURPOSE=development \
AIRWIKI_SIGNING_IDENTITY='Apple Development: Developer Name (TEAMID)' \
./packaging/package-macos.sh
```

This remains an internal development signature. It is not Developer ID signing,
not notarized, and must not be presented as a publicly trusted build.

The signed release wrapper additionally proves that the public updater key in
the signing environment is present in the final desktop executable before it
signs or publishes the updater archive. Cargo rebuilds the desktop whenever an
updater endpoint, updater key or bootstrap registry build input changes.

Expected package content includes:

- the complete pinned `llama-server` runtime;
- `airwiki-mcp-bridge`;
- the platform MCPB;
- `THIRD_PARTY_NOTICES.md`;
- Cargo and non-Cargo license inventories.

The immutable source cache retains the reviewed upstream identity and hashes.
The public macOS staging copy has a distinct distributed-payload identity so
Apple can validate every nested Mach-O; post-signing verification checks the
Developer ID team, secure timestamp, Hardened Runtime, architecture and exact
copy into the application. The public desktop build pins the exact SHA-256 of
that signed `llama-server`, while contributor and ad-hoc builds retain the
upstream trust anchor. Windows keeps the source-built runtime identity and does
not re-sign it as AirWiki.

## Windows x64 candidate

Run on Windows x64 with the pinned Visual Studio C++ toolchain described by the
build policy:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File packaging/package-windows.ps1
```

When Node.js 24.15.0 is supplied as the official portable Windows archive
instead of a machine-wide installation, pass its extracted directory explicitly:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File packaging/package-windows.ps1 `
  -NodeBinDir .\target\node-v24.15.0-win-x64
```

The packaging process verifies the exact Node version, provisions the locked
pnpm shim in that directory, scopes it to the child build, and restores `PATH`
afterward.

The Windows path:

1. builds `llama-server.exe` from the pinned source under the reviewed compiler,
   linker, import, and reproducibility policy;
2. records source, toolchain, imports, smoke test, size, and SHA-256 in
   `BUILD-MANIFEST.json`;
3. builds the desktop with that exact runtime hash, plus the MCP bridge and
   firewall helper;
4. creates the deterministic MCPB and two localized per-user MSI installers;
   and
5. verifies the WiX policy, administrative-image payload and package contracts.

The signed workflow also verifies the updater public key embedded in
`airwiki.exe` before creating the final signed MSI artifact. The Tauri window
icons and packaging brand assets are byte-identical and enforced by tests, so
the executable, shortcuts and installer cannot silently diverge.

The accepted runtime directory contains exactly `llama-server.exe` and
`BUILD-MANIFEST.json`. An unexpected DLL, executable, import, reparse point,
missing build receipt, or byte mismatch blocks the candidate.

### Prepare an unsigned Windows beta artifact

The manual **Package technical candidates** workflow defaults to
`windows-x64-beta`. It builds the same two validated localized MSI packages on a
GitHub-hosted Windows runner from the exact clean `main` commit, verifies that
their Authenticode state is `NotSigned`, and stages only:

- the two MSI installers;
- `SHA256SUMS.txt` for those exact bytes;
- bounded `PROVENANCE.json` containing the repository, commit, version,
  workflow-run URL and 30-day expiry; and
- bilingual `UNSIGNED-BETA.txt` installation and protection guidance.

The uploaded artifact is named
`airwiki-windows-x64-unsigned-beta-<commit>` and expires after 30 days. It uses
no SSL.com eSigner or updater credentials and is never attached to a release, exposed
through `latest.json`, or accepted by the signed promotion workflow. Select
`all-internal-candidates` only when the Linux federation index and macOS
candidate are also required. Select `public-prerelease` only for the separately
confirmed and protected publication path below.

Select `windows-msi-smoke-no-artifact` for an internal, destructive validation
of the freshly built `en-US` MSI without retaining a candidate. That dispatch
runs only the exact-source validation and the Windows build/clean
install-uninstall smoke; it skips Linux and macOS jobs, beta staging,
`upload-artifact`, artifact summaries, attestation, signing, publication and
release credentials. It does not create a downloadable artifact or release
evidence, and must not be treated as a technical prerelease.

`prepare-unsigned-windows-beta.ps1` rejects input outside `target`, reparse
points, unexpected files, a non-official repository identity, a mismatched
version or commit shape, and anything other than exactly two MSI compound
files. The artifact still has no operating-system publisher identity. Testers
must verify the commit and checksums, keep platform protections enabled and stop
when device or organization policy rejects unsigned software.

### Publish a public technical pre-release

Dispatch **Package technical candidates** from the exact current `main` with:

- `target`: `public-prerelease`;
- `prerelease_number`: the next positive beta number; and
- `publication_confirmation`:
  `publish-v<current-version>-beta.<prerelease-number>`.

The request validator derives the stable application version from the reviewed
manifests and rejects a fork, another branch, a moved or dirty checkout, malformed
beta number or mismatched confirmation. macOS, Windows and Linux then build from
the same workflow commit. Linux produces only the x86-64 federation index
server; it is never labeled as AirWiki Desktop.

`technical_prerelease.py` accepts only the verified DMG, two MSI compound files
and x86-64 ELF server below the workflow staging root. It rejects symlinks,
escapes, invalid formats and output reuse, then emits a closed asset set:

- visibly labeled macOS and Windows candidate filenames;
- a deterministic Linux server archive with its legal payloads;
- the reviewed license and third-party inventories;
- strict UTF-8 bilingual safety guidance;
- exact cross-platform `SHA256SUMS.txt`; and
- closed provenance that fixes `supportedPublicRelease`, `updaterChannel` and
  `latest` to false and records each platform's actual trust state.

The publication job has the only `contents: write` token and waits at the
protected `public-release` environment after all platform jobs pass. Once the
closed set exists, a commit-pinned GitHub Action uses the job's short-lived OIDC
identity to attest every final asset; no project attestation key is stored, and
failure stops before draft creation. The job then creates or safely recovers
only an exact private draft, uploads the closed set, re-downloads every asset
and verifies it independently. Only then does it create or validate the
immutable `v<version>-beta.<number>` tag at the workflow commit and publish the
draft with GitHub pre-release enabled and Latest disabled. It never creates
`latest.json`, enters signing environments, requests SSL.com signing, reads
updater keys, rewrites an already public beta or promotes those bytes into the
stable workflow. A failed attempt can resume only an exact private draft at the
same immutable tag and commit with no unexpected assets. The GitHub attestation
proves build provenance, not software safety or operating-system publisher
identity.

### Repackage an already validated internal bundle

To exercise the per-user installer without rebuilding or signing, use a bundle
that already passed runtime and payload validation:

```powershell
.\packaging\package-validated-windows-bundle.ps1 `
  -BundleRoot 'C:\path\to\validated-bundle'

.\packaging\smoke-validated-windows-installer.ps1 `
  -Installer '.\target\packages\windows-validated\airwiki_0.2.0_x64-setup.exe' `
  -BundleRoot 'C:\path\to\validated-bundle' `
  -AuthorizeDestructiveInstallerSmoke
```

The destructive smoke gate requires a clean initial state, installs in the real
interactive user session, opens the application, verifies local MCP/model
operation, and uninstalls. It is an internal acceptance test, not public-release
evidence.

The complete model-readiness pipeline writes a bounded, schema-versioned status
record atomically under the local logs directory. Installation starts the
record before transfer; a pre-activation failure records only one closed stage
class for network, integrity, storage, promotion, bundled-runtime verification,
capacity, configuration or internal failure. Activation then replaces the same
record with its existing starting, ready or failed state. Explicit cancellation
is a closed installation terminal rather than an inferred token state. The record contains
only state, a closed error class, an elapsed-time bucket and a process-exit
class. The smoke ignores a stale record, validates every field against closed
allowlists, reports installation and activation failures separately, and uses
the sanitized log fallback only until it observes a durable status from the
current readiness attempt. A disappearing runtime is only an observation: the
smoke waits a bounded interval for the product's terminal status and otherwise
reports an unknown exit class. Desktop exit is a separate failure class.
Neither path emits the status file, raw logs, local paths, model output or user
data.

Current-user packages install binaries below `%LOCALAPPDATA%\Programs\AirWiki`.
The separate local-first data roots remain under the documented
`%LOCALAPPDATA%\airwiki\AirWiki` and `%APPDATA%\airwiki\AirWiki` locations, so
launching the installed app cannot add mutable data to the verified package
payload. The directory is fixed by the WiX directory table: the installer has
no directory-selection property or page and does not inherit an earlier
arbitrary path. Existing `Programs` and `AirWiki` path components are rejected
before costing if Windows marks either as a reparse point. Windows Installer
owns only declared immutable files and never performs recursive
application-data cleanup.

### MSI install/uninstall smoke

The Windows technical-candidate workflow runs a destructive, per-user smoke on
the freshly packaged `en-US` MSI. It is deliberately narrower than the
validated installer smoke: it does not start AirWiki, download WebView2, or
prepare models. It requires an already-present WebView2 Runtime, a clean AirWiki
installer/shortcut state, reads the MSI identity through the Windows Installer
COM API, installs silently with `AUTOLAUNCHAPP=0`, validates ARP,
`InstallLocation`, essential installed files and the Start Menu target, then
uninstalls by the MSI `ProductCode`. It rejects malformed product identifiers,
any pre-existing registration for either requested product code, and reparse
points or paths outside the canonical Windows known-folder roots. The smoke
revalidates the exact AirWiki identity, location and payload immediately before
uninstalling; an incomplete or ambiguous state is preserved for manual cleanup.
ARP is not the sole source of truth: the smoke also asks Windows Installer for
the product state and rejects advertised, installed or otherwise non-absent
product codes before installation.

For a controlled Windows client, invoke it only with explicit authorization:

```powershell
.\packaging\smoke-windows-msi.ps1 `
  -Installer '.\target\packages\windows\AirWiki_<version>_x64_en-US.msi' `
  -AuthorizeDestructiveMsiSmoke
```

An optional `-UpgradeInstaller` exercises an upgrade only when it has the same
`UpgradeCode` and a strictly higher `ProductVersion`. The smoke writes one
hash-checked marker in each documented AirWiki data root, verifies that
uninstall preserves both, and removes only unchanged markers it created. It
never uses `Win32_Product` and never removes user data roots. If an MSI client
times out or returns a partial state, the smoke rechecks registration, payload,
shortcut and installer-process state; it does not assume terminating the client
has stopped Windows Installer, and leaves affected state for a human to inspect.
It records only marker-parent directories created by that invocation, then
removes those directories leaf-to-root only when they remain empty; existing
roots and directories are never deleted.

This manual smoke is for a trusted, independently verified artifact from the
reviewed build path. It validates installation behavior, not the provenance or
intent of an arbitrary local MSI, and does not claim to defend a hostile local
installer or compromised host.

The release scripts generate a deterministic WiX fragment for the exact
runtime, helper, bridge, MCPB and legal-resource payload. Every component below
the current-user profile has a stable HKCU registry key path, and every package
directory has an uninstall row that removes it only when empty. This preserves
Windows Installer repair and roaming-profile semantics without suppressing
ICE38 or ICE64 validation.

An unsupported development candidate previously installed directly under
`%LOCALAPPDATA%\AirWiki` is not migrated in place because that tree can already
contain user data. The new installer fails closed without uninstalling it.
Uninstall that older candidate while leaving the data-removal option unchecked,
then install the new candidate; the two data roots remain intact by default.

### Windows trust boundaries

- The firewall helper is a sibling of the desktop, requests administrator only
  for `install` or `remove`, and accepts no path, port, or arbitrary rule input.
- A signed public candidate must verify that helper and desktop share the same
  publisher before firewall mutation. Unsigned local builds fail closed.
- Managed rules are limited to application TCP and mDNS UDP 5353,
  Private/Domain, `LocalSubnet`, and no edge traversal.
- The installer does not silently remove broad inherited rules or user data.
- WiX, source archives, and their legal texts are pinned and verified; host
  `PATH` does not select release tools implicitly. The MSI path uses Windows
  Installer for payload inspection and does not prepare 7-Zip.

### Windows open-source signing

The `Windows eSigner MSI candidate` workflow implements the protected signing
path but is not evidence that onboarding or an installed release has passed.
After a secret-free build, it verifies hash-pinned CKA from the reviewed
`SSLcom/eSignerCKA` release and separately hash-pinned CodeSignTool input,
selects only the explicit `10.0.26100.0/x64` Windows SDK `signtool` with a
valid native signature and matching file-version prefix,
scans the exact AirWiki-owned PE and MSI set for malware, signs the desktop, MCP
bridge, firewall helper, and localized MSI containers using the non-exportable
eSigner cloud-HSM credential, and independently verifies the payload and
`AIRWIKI_WINDOWS_SIGNER_SHA256`. Tauri updater signing is a separate final
operation. Normal pull-request CI and `package-pilot.yml` remain unsigned and
consume no signing secret. See the [code signing policy](code-signing-policy.md)
and [ADR 0015](adr/0015-sslcom-esigner-and-github-releases-updates.md).

The older `package-signed-windows.ps1` and `sign-windows-artifact.ps1` NSIS
experiment targets Microsoft Artifact Signing and is not a release path. Both
entry points fail closed unless a developer deliberately enables the legacy
experiment; no active workflow does so. Its read-only verifier and destructive
fixtures remain temporarily for transition coverage and are removed only after
the installed MSI matrix passes.

Detailed public-signing experiments are preserved only as inert historical
references in the [workflow archive](archive/release-workflows/README.md). They
are not executable release policy.

## Candidate verification

Before sharing an internal candidate:

1. Run the complete workspace, documentation, license, and dependency gates.
2. Verify package hashes through a separate channel.
3. Install in a clean interactive user account without development tools.
4. Confirm the selected model and auxiliary snapshots are downloaded once and
   reused after restart.
5. Confirm no public-publisher claim is shown for an unsigned candidate.
6. Run the [macOS–Windows acceptance](two-node-runbook.md).
7. Preserve only the sanitized evidence allowed by the
   [maintainer guide](maintainer-validation.md).

Hash artifacts locally with:

```bash
shasum -a 256 target/packages/macos/*
```

or on Windows:

```powershell
Get-FileHash target\packages\windows\* -Algorithm SHA256
```

## Stable public distribution gate

A supported stable release requires repository governance, monitored security
and conduct contacts, protected environments, Developer ID plus notarization,
Windows public-trust signing, updater-key custody, final-byte SBOM and hashes,
and clean platform acceptance. None of those requirements may be inferred from
a green candidate packaging run.

`prepare-release.yml` creates only a private draft. `promote-release.yml`
re-downloads and verifies that draft on both platforms, then waits for the
protected human release approval. The stable manifest is derived from final
signed bytes and becomes public with the installers. Archived workflows remain
inert historical references and are not release inputs.

Before any stable draft, **Rehearse macOS notarization** can exercise the
protected macOS signing path against the exact green `main` tip. It creates no
tag, release, updater manifest or public download. It retains no installer
bytes: GitHub Actions artifacts are not a private package channel. The 14-day
workflow artifact is only a non-sensitive receipt of commit, version,
verification result, artifact names and hashes. Follow the exact dispatch inputs
in the [release process](release-process.md#rehearse-macos-signing-and-notarization).
