# Contributing to AirWiki

Thank you for helping make local, human-reviewed knowledge easier to use. Contributions should serve individuals, communities, teams, and organizations without weakening privacy or requiring a particular deployment.

## Before you start

Read [AGENTS.md](AGENTS.md), the [architecture](docs/architecture.md), the [architecture decisions](docs/adr/README.md), and the [threat model](docs/threat-model.md). If your change touches `apps/**`, you must also read [apps/AGENTS.md](apps/AGENTS.md); if it touches `packaging/**`, you must read [packaging/AGENTS.md](packaging/AGENTS.md). The closest `AGENTS.md` supplements the root instructions and prevails on conflicts within its subtree. Inspect the current branch and working tree before making changes. Preserve unrelated edits and do not introduce an abstraction, protocol, dependency, or persistent field for a hypothetical future need.

Use [PLANS.md](PLANS.md) for substantial work only when durable coordination is useful. Expensive-to-reverse decisions belong in an ADR, while user-visible changes belong in [CHANGELOG.md](CHANGELOG.md).

Never include real documents, personal information, credentials, model or runtime files, databases, logs, certificates, or generated local state. Tests and examples use synthetic fixtures.

## Development workflow

1. Use the toolchain pinned in `rust-toolchain.toml` and keep `Cargo.lock` versioned.
2. Make one focused change and preserve existing behavior unless the change explicitly requires otherwise.
3. Test the success path and the applicable failure and recovery paths.
4. Update user documentation for visible behavior. Update the threat model when a trust boundary or data flow changes.
5. Review the final diff and run validation proportional to the change.

Changes enter `main` through a focused branch and pull request. Complete the applicable review described in [CODE_REVIEW.md](CODE_REVIEW.md), obtain green required checks, and only then merge. A direct push must never bypass a failed or missing check.

For documentation-only work:

```bash
cargo run --locked -p xtask -- docs check
git diff --check
```

For a focused Rust package, replace `<package>`:

```bash
cargo fmt --all -- --check
cargo clippy --locked -p <package> --all-targets --all-features -- -D warnings
cargo test --locked -p <package> --all-features
```

Before review of a workspace-wide or cross-cutting change:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo run --locked -p xtask -- docs check
cargo run --locked -p xtask -- licenses check
```

Run `cargo deny --locked check` whenever dependencies or `Cargo.lock` change. UI, operating-system, LAN, installer, and packaging changes also require the shortest relevant manual test on an installed development candidate.

Tests must not download models, contact real peers or external services, open external URLs, or require private credentials. Loopback, in-process peers, fake providers, and temporary directories are acceptable.

Manual evidence attached to a pull request must be sanitized according to [docs/maintainer-validation.md](docs/maintainer-validation.md). Record only the minimum commit, package, version, timing, and PASS/FAIL facts needed for review. Never attach document content, questions, snippets, identities, addresses, local paths, databases, or application logs.

## Rust and architecture expectations

- Prefer safe, idiomatic Rust, typed errors, small modules, and explicit state machines.
- Keep domain rules out of widgets and transport crates. Respect the ownership boundaries in `AGENTS.md`.
- Do not block egui or a Tokio runtime worker with filesystem, parsing, hashing, database-heavy, inference, network, or blocking operating-system work.
- Logs must not contain document content, queries, snippets, credentials, embeddings, local paths, or network identities.
- Released migrations are append-only.
- Automation may prepare knowledge but may never publish, grant access, enable external AI, or make ambiguous repairs without explicit human confirmation.

## Developer Certificate of Origin

Every commit must contain a Developer Certificate of Origin 1.1 sign-off certifying that the contributor has the right to submit the change under the project license:

```text
Signed-off-by: Your Name <you@example.com>
```

Create it with `git commit -s`. The read-only DCO workflow checks every non-merge pull-request commit for a valid sign-off belonging to its author or committer. See [docs/DCO.md](docs/DCO.md) for the complete certificate. A sign-off is not a copyright assignment.

## Review and conduct

Review depth is proportional: documentation-only changes receive a documentation review, ordinary code receives a fresh code-review pass, and trust-boundary changes receive dedicated security review plus applicable negative and installed-platform tests. The complete reviewer contract is in [CODE_REVIEW.md](CODE_REVIEW.md).

By participating, you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). Report security issues privately according to [SECURITY.md](SECURITY.md), never in a public issue.
