# AirWiki

<p align="center">
  <img src="resources/branding/airwiki-mark.png" alt="AirWiki logo" width="220">
</p>

AirWiki is an open-source, local-first Rust desktop knowledge base that turns folders of text-based Markdown and PDF files into human-reviewed OKF wikis. It is intended for individuals, communities, small teams, and organizations without assuming a particular company structure or network topology.

Each Wiki joins one source folder, its reviewed OKF bundle, and its access policy. Original documents remain on their source device. LAN collaboration returns bounded authorized snippets after pairing and grants. An experimental, separately opted-in public network can advertise reviewed Wikis through replaceable signed indexes and serve search or browse results directly from the owner without replicating documents. Local chat integrations distinguish answerability-accepted evidence from separately typed candidates that the chat model must verify, use a per-user MCP bridge, and never require AirWiki to store provider API keys.

## Project status

This repository is the development baseline for [airwiki/airwiki](https://github.com/airwiki/airwiki), not a supported public release. The source is available under [Apache-2.0](LICENSE). Builds must be described as development or internal release candidates until public contacts, bundled notices, clean platform gates, signing, and notarization are complete.

Tested development targets are macOS arm64 and Windows x64. Linux, web, and mobile are not currently supported.

## Current capabilities

- Watch folders containing Markdown and text-based PDF files.
- Extract and enrich documents with a locally managed model.
- Compare each proposed draft with revision-bound local evidence and require human review before publishing an OKF wiki concept.
- Search locally with lexical and vector evidence.
- Discover, pair, authorize, and search other AirWiki nodes on a LAN.
- Opt selected reviewed Wikis into experimental public search and browse through signed federated routing indexes.
- Expose explicitly approved evidence through local MCP and let a
  capability-authorized integration maintain isolated OKF memory Wikis.
- Install a global, instruction-only AirWiki skill for Codex/ChatGPT, Claude
  Code and Gemini CLI so a plain request can create or maintain project and
  thematic memory without adding files to the repository.
- Import compatible OKF v0.2 folders or ZIP bundles while preserving unknown
  types and fields, and browse future versions locally under restrictions.
- Run explicitly confirmed `airwiki-wasm` attested computations without WASI,
  host imports, network or external filesystem access.
- Inspect published OKF bundles and their health without changing source files.

AirWiki is early-stage and welcomes contributors interested in Rust, local AI, privacy-preserving search, knowledge management, and accessible desktop UX. Start with [CONTRIBUTING.md](CONTRIBUTING.md) or browse the [open issues](https://github.com/airwiki/airwiki/issues).

## Quick start for development

Install the native build tools for your platform, the Rust version pinned in
`rust-toolchain.toml`, Node.js 24, and Corepack. Install the exact frontend graph
without package lifecycle scripts, then start the Tauri development runner:

```bash
cd apps/desktop/ui
corepack pnpm install --frozen-lockfile --ignore-scripts --prod=false
cd ..
./ui/node_modules/.bin/tauri dev
```

The first-run flow checks the machine, proposes a local model profile, and explains every permission before applying it. Model downloads require disk space and network access on first use; curation and search work offline after the required assets have been verified.

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io),
certificate by [SignPath Foundation](https://signpath.org). Windows signing
roles, privacy guarantees, origin verification and release controls are defined
in the [AirWiki code signing policy](docs/code-signing-policy.md).

Before submitting a cross-cutting change, run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo run --locked -p xtask -- docs check
cargo run --locked -p xtask -- licenses check
```

Run `cargo deny --locked check` when dependencies or `Cargo.lock` change. See [CONTRIBUTING.md](CONTRIBUTING.md) for proportional validation and contribution rules.

## Architecture

The workspace separates dependency-light contracts, local domain logic, inference, LAN transport, MCP, and desktop composition. SQLite owns operational state and local paths; published OKF files are the source of truth for the visible wiki representation. See the [architecture overview](docs/architecture.md) and [architecture decision records](docs/adr/README.md).

## Privacy defaults

- New Wikis are not available to external chats unless a person enables that policy.
- Peer sharing requires an explicit Wiki policy, pairing, and a grant from the source node.
- Modified knowledge is withdrawn until its new revision is reviewed and published.
- The local model cannot publish, grant access, or decide whether content may leave a device.
- The Wiki viewer does not load remote resources or request complete peer documents.
- Logs omit knowledge, queries, snippets, credentials, local paths, and network identities by default.
- Cloud-facing tests use only [synthetic fixtures](fixtures/README.md).

## Repository map

- `crates/`: domain contracts and reusable core, inference, LAN, and MCP behavior.
- `apps/`: the desktop application and its narrowly scoped helper executables.
- `packaging/`: development package scripts and platform manifests.
- `xtask/`: reproducible documentation, licensing, evaluation, and repository
  checks.
- `docs/`: architecture, decisions, operations, security, and internal release guidance.
- `fixtures/`: synthetic test material only.

## Documentation

- [Installation and local operation](docs/install.md)
- [Architecture](docs/architecture.md)
- [Architecture decisions](docs/adr/README.md)
- [Retrieval-quality evaluation](docs/retrieval-quality-evaluation.md)
- [Selector adaptation experiment](docs/selector-adaptation-experiment.md)
- [Pool-level abstention experiment](docs/pool-null-selector-experiment.md)
- [Local chat integrations](docs/chat-integrations.md)
- [AirWiki OKF v0.2 profile](docs/okf-v02-profile.md)
- [Two-node manual acceptance](docs/two-node-runbook.md)
- [Public federation beta infrastructure](docs/public-beta-infrastructure.md)
- [Recovery](docs/recovery.md)
- [Threat model](docs/threat-model.md)
- [Development packaging](docs/packaging.md)
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)
- [Code review](CODE_REVIEW.md)
- [Code signing policy](docs/code-signing-policy.md)
- [Security policy](SECURITY.md)
- [Third-party notices](THIRD_PARTY_NOTICES.md)

## Deliberate limits

The current product does not provide OCR, DOCX ingestion, image/audio/video processing, DHT, a supported always-on public relay service, source-document replication, accounts, SSO, arbitrary remote editing, arbitrary script runtimes, a system daemon, silent updates, or web/mobile access. MCP mutation is limited to explicitly granted AI-memory Wikis. Experimental federation can use explicitly configured validation or community relays.
