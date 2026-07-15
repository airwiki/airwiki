# Internal packaging

AirWiki currently produces internal development candidates with
`cargo-packager 0.11.8`. Packaging does not create a supported public release.
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
cargo install cargo-packager --version 0.11.8 --locked
cargo run --locked -p xtask -- licenses check
```

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
5. builds a deterministic platform MCPB from those exact bridge bytes; and
6. creates `.app` and `.dmg` artifacts under `target/packages/macos`.

The wrapper validates architecture, Mach-O identity, bridge bytes, MCPB layout,
runtime closure, legal files, and traversal safety. A development ad-hoc seal is
not Developer ID signing and must never be presented as public trust.

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

The Windows path:

1. builds `llama-server.exe` from the pinned source under the reviewed compiler,
   linker, import, and reproducibility policy;
2. records source, toolchain, imports, smoke test, size, and SHA-256 in
   `BUILD-MANIFEST.json`;
3. builds the desktop with that exact runtime hash, plus the MCP bridge and
   firewall helper;
4. creates the deterministic MCPB and per-user NSIS installer; and
5. verifies the installer payload and uninstaller contracts.

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
