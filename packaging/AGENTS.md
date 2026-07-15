# Packaging guidance

These instructions apply under `packaging/` in addition to the repository root guidance.

## Milestone boundary

Keep three outcomes separate:

1. Core product behavior works from source.
2. An internal development candidate installs and completes one representative workflow.
3. A public package passes repository, legal, signing, notarization, updater, and clean-machine gates.

Do not make public-release hardening a prerequisite for an internal acceptance test unless that is the explicit goal. Never describe an unsigned or incompletely audited artifact as a supported public release.

## Package invariants

- Use the existing platform adapter and package script before adding a framework, generalized pipeline, or new configuration layer.
- Fail closed on missing or mismatched architecture, hash, manifest, license payload, signature, or helper layout. Never bypass a gate to produce a candidate.
- Keep credentials and private signing keys out of the repository, command output, logs, and package staging. CI secrets are the eventual public-signing boundary.
- Do not silently download unpinned tools or runtimes. Preserve upstream license material and distinguish the upstream artifact hash from any transformed or signed distributed payload.
- Build in clean staging directories, reject symlinks and unexpected files, and verify the final installed layout rather than only loose binaries.
- Package checks may write under `target/` or temporary directories but must not rewrite source manifests, generated legal inventories, or lockfiles unexpectedly.

The official repository is <https://github.com/airwiki/airwiki>. For routine development packaging, follow `docs/packaging.md`. Public distribution remains blocked until reporting contacts, legal closure, platform identities, and release gates are explicitly complete.
