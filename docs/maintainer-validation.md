# Maintainer validation records

This guide defines the sanitized evidence that may accompany a development
candidate. It does not replace the user-facing
[two-node acceptance runbook](two-node-runbook.md).

## Allowed record

Record only:

| Field | macOS | Windows |
| --- | --- | --- |
| Commit |  |  |
| Package SHA-256 |  |  |
| Application version |  |  |
| Operating-system version |  |  |
| Model profile and pinned revision |  |  |
| Relevant elapsed times |  |  |
| PASS/FAIL |  |  |

Do not record document content, questions, snippets, PeerIds, IP addresses,
ports, multiaddresses, SAS words, local paths, usernames, database copies, or
application logs. Logs used for diagnosis must stay local and be sanitized
before any excerpt is shared.

## Technical gates

Run repository checks against the exact recorded commit:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo run --locked -p xtask -- docs check
cargo run --locked -p xtask -- licenses check
cargo deny --locked check
```

Real-model evaluation is optional for ordinary CI and must use an already
verified local snapshot. Structural validation uses deterministic providers and
does not download models:

```bash
cargo run --locked -p xtask -- relevance validate
cargo run --locked -p xtask -- relevance evaluate --snapshot <verified-snapshot>
cargo run --locked -p xtask -- retrieval validate
cargo run --locked -p xtask -- retrieval evaluate \
  --embedding-snapshot <verified-e5-snapshot> \
  --relevance-snapshot <verified-mmarco-snapshot>
cargo run --locked -p xtask -- typed-evidence-v3 validate-contract
```

The generated reports stay under `target/evals/`. Persist only their fixture
hash, artifact revisions, platform, aggregate result, and SHA-256 when a
candidate requires that evidence. A real-model failure is a measured product
quality result; do not alter fixtures or thresholds solely to make a candidate
appear green. See the [retrieval-quality profile](retrieval-quality-evaluation.md)
for the pipeline scope and current platform observation. The typed-evidence
command validates only the frozen public experiment contract; private inputs,
annotations, receipts, scoring keys and reports must remain outside the
repository.

Manual platform gates must use installed applications in interactive desktop
sessions. A macOS build cannot certify Windows behavior, and an SSH-launched
process cannot substitute for the real Windows user session.

Before a silent Windows uninstall or upgrade probe, request **Quit completely**
through the installed application and assert that no AirWiki process remains.
`msiexec /qn` cannot present the normal files-in-use interaction; leaving the
process alive would test that suppressed-interaction edge case instead of the
supported installed-candidate lifecycle. The in-app updater remains a separate
journey and must prove that its coordinated shutdown completes after launching
Windows Installer.

Before starting Windows LAN acceptance, verify that Windows Application Control
allows the exact installer and that the installed desktop, MCP bridge and
firewall helper have valid Authenticode signatures from the same publisher.
Unsigned candidates remain valid for local-only product checks, but they cannot
exercise the fail-closed firewall boundary. A host-policy rejection is an
environment prerequisite failure: preserve the policy, do not add manual
firewall rules or bypass execution controls, and do not report the blocked LAN
journey as either PASS or a product-search failure.

The installed-model activation probe verifies only the selected model, its
supervised loopback transport, and one bounded strict-JSON response. Candidate
acceptance must separately exercise full production enrichment on at least two
synthetic documents, confirm that neither draft used fallback metadata, complete
human review and publication, and retrieve the published revision through MCP.
Record only the sanitized fields allowed above.
