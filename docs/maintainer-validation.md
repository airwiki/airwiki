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
verified local snapshot:

```bash
cargo run --locked -p xtask -- relevance validate
cargo run --locked -p xtask -- relevance evaluate --snapshot <verified-snapshot>
```

The generated report stays under `target/evals/`. Persist only its fixture hash,
artifact revision, platform, aggregate result, and SHA-256 when a candidate
requires that evidence.

Manual platform gates must use installed applications in interactive desktop
sessions. A macOS build cannot certify Windows behavior, and an SSH-launched
process cannot substitute for the real Windows user session.
