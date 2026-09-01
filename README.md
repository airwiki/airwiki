# AirWiki

<p align="center">
  <img src="resources/branding/airwiki-app-icon.png" alt="AirWiki" width="128">
</p>

<p align="center">
  <strong>Your private, portable wiki—built from the knowledge you already have.</strong>
</p>

<p align="center">
  macOS 13+ (Apple silicon) · Windows 10/11 x64 (AVX2) · OKF v0.2 · Apache-2.0
</p>

<p align="center">
  <a href="#how-airwiki-works">How it works</a> ·
  <a href="#availability">Availability</a> ·
  <a href="https://github.com/airwiki/airwiki/releases">Technical beta</a> ·
  <a href="FAQ.md">Beta FAQ</a> ·
  <a href="#run-from-source">Run from source</a> ·
  <a href="CONTRIBUTING.md">Contribute</a> ·
  <a href="SUPPORT.md">Feedback</a> ·
  <a href="docs/code-signing-policy.md">Code signing policy</a>
</p>

AirWiki is an open-source desktop app that turns folders, [Open Knowledge Format (OKF)](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md) bundles, and assistant conversations into wikis you can search, review, and selectively share. Knowledge stays on the device that owns it by default. Local AI can organize and index it, but cannot publish it or grant access.

> [!IMPORTANT]
> AirWiki is in active development and has no supported stable download yet. Public technical pre-releases are unsigned or unnotarized test candidates, are never selected by the updater, and may be blocked by platform policy. Read [Availability](#availability) before installing one.

<p align="center">
  <a href="docs/assets/airwiki-demo.mp4">
    <img src="docs/assets/airwiki-demo.gif" alt="Animated AirWiki tour showing the Wiki library, progressive review, federated search, and AI app settings">
  </a>
  <br>
  <sub>10-second product tour · <a href="docs/assets/airwiki-demo.mp4">MP4 version</a> · synthetic data only</sub>
</p>

## How AirWiki works

1. **Create a Wiki.** Start from a folder, import an OKF v0.2 folder or ZIP, create private personal memory, or explicitly initialize portable project memory in `.airwiki`.
2. **Review progressively.** Folder sources appear immediately as local, unverified OKF `draft` concepts. Approve a draft to make it stable and searchable, leave it for later, or exclude it without deleting its evidence. **Update from folder** detects new or changed files and reanalyzes current drafts without rewriting reviewed or excluded knowledge.
3. **Search accessible knowledge.** One Library groups results from this device, authorized nearby devices, and—only when selected—public wikis. Origins and partial coverage remain visible.
4. **Share deliberately.** LAN and Internet exposure are independent, opt-in choices for your own stable wikis.
5. **Connect AI apps separately.** ChatGPT, Claude, Codex, Gemini, and generic MCP clients can search permitted local and authorized LAN knowledge. Public search is a separate, per-app preference because it may send that app's query outside the device.

Folder wikis can watch their source or update manually. Imported OKF wikis have no source watcher. Personal and project memory can be edited only by applications you explicitly authorize. AirWiki never stages, commits, merges, pulls, or pushes Git.

## Sharing and AI access are different

AirWiki keeps the two decisions separate so that “connect my assistant” never means “publish my wiki.”

| Decision | What it controls | Default |
| --- | --- | --- |
| **Share** | Which of your stable wikis may leave this device through a verified LAN grant or public publication | Off |
| **Connect an AI app** | Which compatible local wikis the app may read, plus knowledge already authorized by nearby owners | Explicit connection; local read is granted by default and remains revocable per wiki |
| **Search public knowledge** | Whether that exact app may send queries to configured public indexes and publishers | Off |

Enabling public search for an AI app does **not** publish any of your wikis. A remote owner still decides what it exposes, and AirWiki revalidates authorization before returning evidence.

## Search without centralizing knowledge

AirWiki sends a question to the places that still own the knowledge instead of copying every wiki into a central service.

```mermaid
flowchart LR
    question["Question"] --> local["This device<br/>local index"]
    question -- "verified device + Wiki grant" --> lan["Authorized LAN owners<br/>their local indexes"]
    question --> catalog["Public routing indexes<br/>signed metadata only"]
    catalog --> public["Opted-in public owners<br/>their local indexes"]

    local --> results["Ranked evidence<br/>with provenance"]
    lan --> results
    public --> results
```

Each owner runs retrieval locally and checks current exposure before returning bounded evidence. AirWiki combines the independent rankings while keeping local, nearby, and public origins visible. If one source is unavailable, successful branches still return results with an explicit coverage gap.

LAN discovery alone grants nothing: devices must be verified and the owner must grant the wiki. Public routing indexes locate opted-in publishers but do not receive their documents, snippets, embeddings, or operational indexes. Opening an authorized remote result loads its published OKF wiki in a read-only workspace.

The Library decides public search per query. Each connected AI app has its own disabled-by-default **Search public knowledge** preference. Read the [search and federation guide](docs/search-and-federation.md) for the complete journey and privacy boundaries.

## See the flow

| Review a draft before it becomes searchable | Choose public search per AI app |
| --- | --- |
| [![AirWiki review panel showing source evidence beside a draft proposal and review actions](docs/assets/airwiki-review-flow.png)](docs/assets/airwiki-review-flow.png) | [![AirWiki AI app settings showing the disabled Search public knowledge preference for ChatGPT](docs/assets/airwiki-ai-app-search.png)](docs/assets/airwiki-ai-app-search.png) |
| Evidence stays beside the proposal; approval changes its knowledge state, not its network exposure. | The preference controls query egress for one app and never publishes the owner's wikis. |

<p align="center">
  <a href="docs/assets/airwiki-search-sources.png">
    <img src="docs/assets/airwiki-search-sources.png" alt="AirWiki search results grouped by this device, a verified nearby device, and the public network">
  </a>
  <br>
  <sub>Local, nearby, and public origins remain distinct in one search experience.</sub>
</p>

## What works today

- **Build knowledge:** create manual or watched wikis from Markdown and text-based PDFs; import hierarchical OKF v0.2 folders and ZIPs; browse draft, reviewed, and excluded concepts.
- **Review safely:** compare a proposal with revision-bound source evidence, approve at your pace, and withdraw changed source knowledge until its replacement is reviewed.
- **Find it:** combine lexical and vector search across local, authorized LAN, and explicitly selected public sources with provenance, assurance, and partial-coverage state.
- **Share it:** verify nearby devices and grant individual wikis; independently opt reviewed wikis into experimental public discovery.
- **Use it with assistants:** connect ChatGPT/Codex, Claude, Gemini, or generic MCP clients without provider API keys; keep local wiki exceptions and public-query consent per app.
- **Inspect it:** view the wiki's knowledge state, Local/LAN/Internet exposure, AI-app access, freshness, lifecycle, provenance, compatibility, and health.

<details>
<summary>Advanced workflows</summary>

- Create isolated personal-memory wikis with fingerprint-based updates and revocable capabilities.
- Initialize a portable `.airwiki` project wiki, approve each local clone once, and review its changes as ordinary files.
- Open authorized LAN or public results in the same read-only, file-oriented workspace used for local knowledge.
- Run explicitly confirmed `airwiki-wasm` attested computations in a constrained, no-WASI sandbox.

</details>

AirWiki is intended for people, researchers, communities, small teams, and organizations that need portable knowledge and explicit trust boundaries without centralizing every document.

## Privacy by default

- New wikis are private from LAN and Internet.
- Original folder contents remain on their source device and are never deleted by AirWiki.
- Sharing and AI connections require separate human decisions.
- Connecting an app never changes LAN or Internet exposure; public query egress stays off until enabled for that app.
- Only stable knowledge enters search or external disclosure. A changed source revision is withheld while its replacement remains visible locally as a draft.
- The local model may propose metadata; it cannot publish, grant access, or decide whether content leaves the device.
- Default logs omit documents, queries, snippets, credentials, local paths, and network identities.

Opening an authorized remote result loads the complete published OKF wiki: its hierarchy, `index.md`, `log.md`, stable concept pages, metadata, and relationship graph. Original source files, source paths, chunks, embeddings, and operational indexes remain on the owner's device.

Read the [threat model](docs/threat-model.md) for complete trust boundaries and failure behavior.

## Availability

| Platform | Technical pre-release | Current boundary |
| --- | --- | --- |
| macOS | Apple silicon, macOS 13+ DMG | Ad-hoc signed and not notarized |
| Windows | Windows 10/11 x64 with AVX2 | Unsigned `en-US` and `es-ES` MSI |
| Linux x64 | Federation index server | Maintainer service, not AirWiki Desktop |
| Linux desktop, web, mobile | None | Not currently supported |

Reviewed builds may appear on [GitHub Releases](https://github.com/airwiki/airwiki/releases) as clearly marked technical pre-releases. They are permanent manual downloads, but they are not `Latest`, are never selected by the updater, and do not establish a Windows or macOS publisher identity. Verify `SHA256SUMS.txt` and the GitHub build-provenance attestation, keep operating-system and organization protections enabled, and stop when local policy blocks the candidate.

After the complete acceptance checklist passes, signed installers will use the separate [stable download](https://github.com/airwiki/airwiki/releases/latest). See [Installing and running AirWiki](docs/install.md) for current candidate requirements and first-run behavior.

## Run from source

You need the native build tools for your platform, the Rust toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml), Node.js 24.15.0, Corepack, and the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
cd apps/desktop/ui
corepack pnpm install --frozen-lockfile --ignore-scripts --prod=false
cd ..
./ui/node_modules/.bin/tauri dev
```

The first-run flow explains local privacy and offers a direct path to the first folder wiki. Initial model preparation needs disk space and network access; curation and local search work offline after the required assets are verified.

Contributions around Rust, local AI, privacy-preserving search, knowledge management, and accessible desktop UX are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md) and the [open issues](https://github.com/airwiki/airwiki/issues).

For beta setup questions, reproducible feedback, and reporting boundaries, read
[SUPPORT.md](SUPPORT.md). Public issue reports must use synthetic data only.
Potential security or data-exposure concerns belong in the private process in
[SECURITY.md](SECURITY.md).

## Architecture at a glance

AirWiki is a Rust workspace with a Tauri v2 desktop shell and Svelte UI. SQLite owns operational state and local paths; managed OKF `draft` and `stable` files are the source of truth for each locally visible wiki. Domain rules stay outside widgets and transports.

```mermaid
flowchart LR
    source["Folder, import,<br/>or memory"] --> prepare["Ingest + local AI"]
    prepare --> draft["Local OKF draft"]
    draft --> review["Human review"]
    review --> stable["Stable knowledge"]

    stable --> local["Local search"]
    stable -- "verified device + Wiki grant" --> lan["LAN sharing"]
    stable -- "explicit publication" --> public["Public network"]
    stable -- "app capability" --> mcp["AI apps via MCP"]

    mcp -. "permitted local" .-> local
    mcp -. "source-authorized" .-> lan
    mcp -. "per-app public query consent" .-> public
```

Every route beyond local use crosses an independently managed access boundary. See the [architecture overview](docs/architecture.md) and [architecture decisions](docs/adr/README.md).

<details>
<summary>Repository map</summary>

- `crates/`: contracts, domain logic, inference, networking, and MCP behavior.
- `apps/`: the Tauri desktop application and narrowly scoped helpers.
- `packaging/`: development packaging and platform manifests.
- `xtask/`: reproducible documentation, licensing, evaluation, and repository checks.
- `docs/`: architecture, decisions, security, operations, and release guidance.
- `fixtures/`: synthetic test material only.

</details>

## Documentation

### Use and evaluate AirWiki

- [Installation and local operation](docs/install.md)
- [Search across local, LAN, and public wikis](docs/search-and-federation.md)
- [Local chat integrations and assisted memory](docs/chat-integrations.md)
- [AirWiki's OKF v0.2 profile](docs/okf-v02-profile.md)
- [Two-node acceptance runbook](docs/two-node-runbook.md)
- [Recovery](docs/recovery.md)

### Build and contribute

- [Contributing](CONTRIBUTING.md)
- [Architecture](docs/architecture.md)
- [Threat model](docs/threat-model.md)
- [Code review](CODE_REVIEW.md)
- [Development packaging](docs/packaging.md)
- [Public release process](docs/release-process.md)
- [Code signing policy](docs/code-signing-policy.md)
- [Security policy](SECURITY.md)
- [Privacy and data handling](PRIVACY.md)
- [Beta FAQ and known limitations](FAQ.md)
- [Beta roadmap and feedback focus](ROADMAP.md)
- [Support and feedback](SUPPORT.md)
- [Changelog](CHANGELOG.md)

## Deliberate limits

AirWiki does not currently provide OCR, DOCX ingestion, image/audio/video processing, cloud sync, accounts, SSO, source-document replication, arbitrary remote editing, automatic Git operations or conflict resolution, arbitrary script runtimes, a system daemon, silent updates, or web/mobile access. MCP mutation is limited to explicitly authorized personal or project memory wikis. Public federation remains experimental and has no supported always-on public relay service.

## License

AirWiki is open source under the [Apache License 2.0](LICENSE). The current public technical beta is unsigned or unnotarized. A proposed SignPath Foundation route for future Windows stable signing is not operational until provider acceptance, protected configuration, a separate manual approval, and installed acceptance pass. The stable Tauri updater will use one confirmed channel hosted on GitHub Releases, whose assets remain independently signed and verified. See the [Code signing policy](docs/code-signing-policy.md).

ChatGPT, Codex, Claude, and Gemini names and marks belong to OpenAI, Anthropic, and Google respectively. AirWiki uses their official artwork only to identify optional integrations; this does not imply sponsorship or endorsement. See [third-party notices](THIRD_PARTY_NOTICES.md).
