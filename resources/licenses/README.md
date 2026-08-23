# Included licenses

- `../../LICENSE`: Apache-2.0 license for AirWiki source code.
- `MIT.txt`: common MIT text used by pinned components where applicable.
- `Apache-2.0.txt`: Apache License 2.0 used by pinned components where applicable.
- `THIRD_PARTY_LICENSES.md`: generated transitive Rust inventory for the macOS
  arm64 and Windows x64 distributed graphs, with legal texts deduplicated by
  SHA-256.
- `NPM_LICENSES_MACOS_ARM64.md` and `NPM_LICENSES_WINDOWS_X64.md`: generated
  pnpm production and development closures for each verified desktop platform,
  with local paths removed and legal texts deduplicated by SHA-256.
- `NON_CARGO_COMPONENTS.md`: provenance, hashes, and verified licenses for
  bundled fonts and non-Cargo components used to build or inspect packages.
- `non-cargo/`: normalized legal texts copied from the exact artifacts or source
  revisions listed by the non-Cargo inventory.

Top-level notices summarize externally sourced components. `licenses check`
validates inventory hashes and rejects components without a verified
redistribution basis.

After dependency or component changes, run:

```bash
cargo run --locked -p xtask -- licenses generate
cargo run --locked -p xtask -- licenses check
pnpm --dir apps/desktop/ui run licenses:check macos-arm64 # on macOS arm64
pnpm.cmd --dir apps/desktop/ui run licenses:check windows-x64 # on Windows x64
```

Review the generated diff before accepting it. `Cargo.lock` fixes package
versions; the inventories add sources, SPDX metadata, and exact legal texts.
