# Contributing to AirWiki

Thank you for helping make local, human-reviewed knowledge easier to use. Contributions should serve individuals, communities, teams, and organizations without weakening privacy or requiring a particular deployment.

## Before you start

Read [AGENTS.md](AGENTS.md), the [architecture](docs/architecture.md), the [architecture decisions](docs/adr/README.md), and the [threat model](docs/threat-model.md). If your change touches `apps/**`, you must also read [apps/AGENTS.md](apps/AGENTS.md); if it touches `packaging/**`, you must read [packaging/AGENTS.md](packaging/AGENTS.md). The closest `AGENTS.md` supplements the root instructions and prevails on conflicts within its subtree. Inspect the current branch and working tree before making changes. Preserve unrelated edits and do not introduce an abstraction, protocol, dependency, or persistent field for a hypothetical future need.

For desktop interface work, follow the [AirWiki desktop design guidelines](docs/ui-design-guidelines.md). They translate Apple Human Interface Guidelines into semantic, accessible rules for AirWiki's cross-platform Tauri/WebView shell without copying Apple branding or materials.

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
# Node.js 24.15.0 is required.
corepack enable
corepack prepare pnpm@10.18.3 --activate
pnpm --dir apps/desktop/ui install --frozen-lockfile --ignore-scripts --prod=false
pnpm --dir apps/desktop/ui run check
pnpm --dir apps/desktop/ui run lint
pnpm --dir apps/desktop/ui run test
pnpm --dir apps/desktop/ui run check:e2e
pnpm --dir apps/desktop/ui audit --audit-level high --prod
```

Run `cargo deny --locked check` whenever dependencies or `Cargo.lock` change. UI, operating-system, LAN, installer, and packaging changes also require the shortest relevant manual test on an installed development candidate.

Tests must not download models, contact real peers or external services, open external URLs, or require private credentials. Loopback, in-process peers, fake providers, and temporary directories are acceptable.

`pnpm --dir apps/desktop/ui e2e` builds and runs the isolated desktop journey.
The normal journey then exits through the UI quit handler and starts a second
process against the same temporary database to verify local reading and panel
restoration. It does not restore a query or remote browsing session.
Set `AIRWIKI_E2E_REVIEW_FIXTURE=1` for the review journey with synthetic drafts,
stored evidence and real publication IPC, without installing inference models.
The fixture is compiled only with the debug-only `e2e` feature and requires a
temporary E2E data root. The runner creates and removes that root automatically.
Every `e2e` build requires an existing `airwiki-e2e-*` directory directly inside
the operating system's temporary directory. Launching it without that explicit
root fails before profile discovery; it never falls back to the normal user
profile. Retained profiles can be reused, but linked data/config directories
are rejected. Use the runner or provide its isolated environment before attaching
a UI inspection tool to an installed debug candidate.
Rebuild with `e2e` after other Cargo commands rebuild the desktop binary so the
runner uses the Tauri E2E configuration and embedded UI.
Its quit-event checks complement, but do not replace, installed-platform checks
of native menu, tray and window controls.

Visual references are native and platform-specific. Both journeys cover English
and Spanish, light and dark themes, and window sizes of 1024×720, 1180×760 and
1440×900. Baseline filenames record the actual WebView dimensions; window chrome
can reduce the available height. The normal journey captures the empty library,
General settings, reading and sources; the review journey captures the proposal,
evidence and comparison layouts using only synthetic fixtures.

To inspect a proposed visual change, run each journey with
`AIRWIKI_E2E_VISUAL=0 AIRWIKI_E2E_CAPTURE_MATRIX=1` and inspect the PNGs in
`apps/desktop/ui/.artifacts/visual/matrix`. After confirming that they satisfy the
design, run with `UPDATE_VISUAL_BASELINES=1` and visual checks enabled. This
replaces only the current platform's references for that journey; it preserves
the other journey and platform. Repeat both journeys without the update variable
to prove comparison against the reviewed references. Captures freeze animation
and transient pointer decoration; they do not prove keyboard focus, screen-reader
behavior or installed-platform acceptance. Hosted CI runs the functional journeys
with `AIRWIKI_E2E_VISUAL=0` because its window chrome is not a stable visual target.

The embedded WebDriver sends synthetic keyboard events: `Enter` and `Space` do
not reproduce the browser's default button activation. Use a click to exercise
the action in E2E. The driver also focuses a click target after its handler,
which can overwrite the application's focus return. Verify Tab, native keyboard
activation and focus return in the installed candidate. Do not add application
key handlers to compensate for these test-driver limitations.

Manual evidence attached to a pull request must be sanitized according to [docs/maintainer-validation.md](docs/maintainer-validation.md). Record only the minimum commit, package, version, timing, and PASS/FAIL facts needed for review. Never attach document content, questions, snippets, identities, addresses, local paths, databases, or application logs.

## Rust and architecture expectations

- Prefer safe, idiomatic Rust, typed errors, small modules, and explicit state machines.
- Keep domain rules out of widgets and transport crates. Respect the ownership boundaries in `AGENTS.md`.
- Do not block a Tauri command or Tokio runtime worker with filesystem, parsing, hashing, database-heavy, inference, network, or blocking operating-system work.
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

By participating, you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). For
product feedback and non-security questions, use the public channels in
[SUPPORT.md](SUPPORT.md) with synthetic material only. Report security issues
privately according to [SECURITY.md](SECURITY.md), never in a public issue.

The Code of Conduct does not yet list a monitored enforcement contact; this is
a documented release blocker. Do not use public issues to report conduct
incidents or private information.
