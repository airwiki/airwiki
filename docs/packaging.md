# Internal packaging

AirWiki currently produces internal development candidates with the pinned
Tauri v2 CLI and bundler. Packaging does not create a supported public release.
Public signing, notarization, updater promotion, and repository-hosted release
automation remain deferred until the [public release checklist](release-checklist.md)
is complete.

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
6. builds and signs the Tauri application bundle without altering the
   hash-pinned runtime resources; and
7. creates a licensed Tauri DMG and copies the verified `.app` and `.dmg`
   artifacts under `target/packages/macos`.

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

Do not re-sign a hash-pinned upstream runtime without a reviewed distinction
between upstream identity and distributed payload identity.

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
4. creates the deterministic MCPB and per-user NSIS installer; and
5. verifies the installer payload and uninstaller contracts.

The signed wrapper also verifies the updater public key embedded in
`airwiki.exe` before creating the final signed NSIS artifact. The Tauri window
icons and packaging brand assets are byte-identical and enforced by tests, so
the executable, shortcuts and installer cannot silently diverge.

The accepted runtime directory contains exactly `llama-server.exe` and
`BUILD-MANIFEST.json`. An unexpected DLL, executable, import, reparse point,
missing build receipt, or byte mismatch blocks the candidate.

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
payload. The directory is fixed: the installer has no directory-selection page
and rejects, before any page or write, any effective path (including `/D`) that
is not the case-insensitive exact fixed path. Rejecting aliases instead of
canonicalizing them avoids 8.3, junction, traversal, and trailing-dot
equivalence bypasses. Existing `Programs` and `AirWiki` path components are
also rejected if Windows marks either as a reparse point.

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
- NSIS, 7-Zip, source archives, and their legal texts are pinned and verified;
  host `PATH` does not select release tools implicitly.

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

## Public distribution remains deferred

A public release requires repository governance, monitored security and conduct
contacts, protected environments, Developer ID plus notarization, Windows
public-trust signing, updater-key custody, final-byte SBOM and hashes, and clean
platform acceptance. None of those requirements may be inferred from a green
internal packaging run.

No active workflow in this baseline signs or promotes a public release. Restoring
such automation is a separate goal and requires reviewing the archived design
against current platform contracts rather than copying it back unchanged.
The guarded release scripts nevertheless produce signable Tauri updater
artifacts: a final stapled macOS app archive and the final Authenticode-signed
Windows NSIS installer. Each script creates the adjacent Tauri `.sig` only when
the updater private key is present and verifies it with the compiled public key
before the stable manifest can be generated.
