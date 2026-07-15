# Included licenses

- `../../LICENSE`: Apache-2.0 license for AirWiki source code.
- `MIT.txt`: common MIT text used by pinned components where applicable.
- `Apache-2.0.txt`: Apache License 2.0 used by pinned components where applicable.
- `THIRD_PARTY_LICENSES.md`: generated transitive Rust inventory for the macOS
  arm64 and Windows x64 distributed graphs, with legal texts deduplicated by
  SHA-256.
- `NON_CARGO_COMPONENTS.md`: provenance, hashes, and verified licenses for
  non-Cargo components used to build or inspect Windows packages.
- `non-cargo/`: normalized legal texts copied from the exact artifacts or source
  revisions listed by the non-Cargo inventory.

Top-level notices summarize externally sourced components. `licenses check`
validates inventory hashes and rejects components without a verified
redistribution basis.

After dependency or component changes, run:

```bash
cargo run --locked -p xtask -- licenses generate
cargo run --locked -p xtask -- licenses check
```

Review the generated diff before accepting it. `Cargo.lock` fixes package
versions; the inventories add sources, SPDX metadata, and exact legal texts.
