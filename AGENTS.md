# AirWiki contributor guidance

## Mission and priorities

AirWiki is a local-first knowledge application for individuals, communities, teams, and organizations. Do not hard-code enterprise assumptions, organization-specific policy, or one network topology into the domain model.

When priorities conflict, use this order:

1. Privacy, authorization, and data integrity.
2. Correctness and recoverability.
3. Clarity and maintainability.
4. Portability and contributor experience.
5. Performance demonstrated by measurement.
6. New functionality.

Apply YAGNI, KISS, and DRY. Deliver the smallest complete user journey, prefer explicit state and boring technology, and share a rule only when it has multiple real consumers or protects a security invariant. Stop and re-scope when a supporting task grows beyond its user-visible outcome.

The source is Apache-2.0 and the official repository is [airwiki/airwiki](https://github.com/airwiki/airwiki), but there is no supported public build yet. Treat packages as development or internal release candidates. Public signing, notarization, updating, and distribution must not block core product validation unless they are the stated goal.

## Before changing anything

- Read `README.md`, `CONTRIBUTING.md`, and the documentation relevant to the change. When touching `apps/**` or `packaging/**`, you must also read the closest nested `AGENTS.md` before editing; it supplements this file and prevails on conflicts within its subtree.
- Inspect the current branch, `git status`, and existing diffs. Preserve unrelated and user-owned changes.
- State the user-visible outcome, the shortest acceptance path, and what is deliberately deferred before a multi-step change. Use `PLANS.md` only when durable coordination is genuinely useful.
- Prefer a focused change. Do not combine behavior changes, broad refactors, dependency upgrades, and formatting churn.
- Use synthetic fixtures. Never commit private documents, personal data, credentials, models, runtimes, databases, logs, certificates, or generated local state.

## Workspace ownership

- `airwiki-types`: dependency-light contracts and domain value types.
- `airwiki-core`: ingestion, storage, OKF publication and inspection, local policy, and search.
- `airwiki-inference`: model catalog, assets, inference clients, and runtime supervision.
- `airwiki-network`: authenticated LAN transport, pairing, grants, and peer protocol.
- `airwiki-mcp`: canonical read-only MCP contracts and search adapter.
- `apps/desktop`: composition root, worker orchestration, OS integration, and egui UI.
- `apps/mcp-bridge`: per-user stdio bridge to the fixed loopback MCP endpoint.
- `apps/windows-firewall-helper`: narrow elevated Windows firewall boundary.
- `xtask`: reproducible repository checks and packaging support.
- `packaging`: platform package scripts and manifests.

Dependencies point toward contracts and domain logic. Business rules do not belong in widgets or transports. Create a crate only for a durable dependency boundary; a module is normally sufficient. Additional rules for application and package work live in `apps/AGENTS.md` and `packaging/AGENTS.md`.

SQLite is the source of operational state and local paths. Published OKF files are the source of truth for the visible wiki. Reconciliation reports disagreement and never silently chooses the convenient side.

## Safety and Rust rules

- Automation may detect, extract, enrich, index, retry, inspect health, and regenerate validated derived artifacts. It may never publish, grant access, enable external AI, alter source documents, or repair ambiguous knowledge or history without explicit human approval.
- Fail closed when publication or authorization is uncertain: withdraw the affected revision from LAN and MCP, preserve diagnosable state, and request human action.
- A local model proposes metadata only. It never decides publication, collection membership, permissions, or whether content may leave the device.
- Keep sharing opt-in. Do not add telemetry, cloud transfer, remote fetching, or new trust boundaries without an explicit product decision and threat-model update.
- Use the pinned stable toolchain and safe, idiomatic Rust. Keep public APIs small; prefer enums and newtypes for domain states and typed errors at reusable boundaries.
- When Codex skills are available, use `rust-best-practices` for Rust implementation and review, and `rust-async-patterns` for Tokio or concurrency work. Skills guide the work; versioned code, tests, ADRs, and documentation remain authoritative, and contributor workflows must not depend on a local skill installation.
- Avoid `unwrap`, `expect`, `panic!`, unchecked indexing, unnecessary cloning, unnecessary locks or trait objects, and `unsafe` in production paths. Any unavoidable invariant must be documented and tested locally.
- Never block egui or a Tokio runtime worker with filesystem traversal, parsing, hashing, heavy SQLite work, inference, or network I/O. Use the existing worker boundary and `spawn_blocking` where appropriate.
- Logs are structured and sanitized. Never log document content, queries, snippets, tokens, secrets, embeddings, source paths, PeerIds, IPs, ports, or multiaddresses by default.
- Released migrations are append-only. Persisted schema, OKF profile, wire protocol, and public API changes require compatibility tests and documentation.

## Validation

Choose the smallest tier that proves the change.

**Documentation only**

```bash
cargo run --locked -p xtask -- docs check
git diff --check
```

**Focused Rust package** (replace `<package>`)

```bash
cargo fmt --all -- --check
cargo clippy --locked -p <package> --all-targets --all-features -- -D warnings
cargo test --locked -p <package> --all-features
```

**Workspace-wide or cross-cutting**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo run --locked -p xtask -- docs check
cargo run --locked -p xtask -- licenses check
```

Run `cargo deny --locked check` when dependencies or `Cargo.lock` change. Tests may use loopback and in-process fake peers, but must not download models, contact real peers or external services, open external URLs, or require private credentials.

UI, operating-system, LAN, installer, and packaging changes also require the shortest relevant manual test on the affected installed candidate. A change is complete only when its success and applicable failure/recovery paths are covered, documentation is current, privacy invariants hold, and unrelated files remain untouched.

Changes intended for `main` follow the proportional review process in `CODE_REVIEW.md`: focused branch, pull request, applicable review, green required checks, then merge. Never use a direct push to bypass a failed or missing check.
