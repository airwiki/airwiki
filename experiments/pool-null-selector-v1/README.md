# Pool-null selector experiment v1

This directory preserves the exact maintainer-side runner reviewed for the
[preregistered pool-level abstention experiment](../../docs/pool-null-selector-experiment.md).
It is not part of the AirWiki application, Cargo workspace, packages or runtime.

The runner has five explicit commands:

```bash
python3 experiments/pool-null-selector-v1/runner.py self-test
python3 experiments/pool-null-selector-v1/runner.py runtime-self-test
python3 experiments/pool-null-selector-v1/runner.py network-self-test
python3 experiments/pool-null-selector-v1/runner.py prepare-model --allow-network
python3 experiments/pool-null-selector-v1/runner.py run-compatibility
```

The final command is a one-shot observed diagnostic. Do not run it until the
runner commit and its preregistration are frozen. On macOS it automatically
reexecutes exactly once inside its own `sandbox-exec` profile with network access
denied and refuses to create the receipt unless a kernel-denied loopback probe
inside that child confirms the isolation. Model
downloads and checkpoints stay under `target/pool-null-experiment/`. The durable
attempt receipt and aggregate report are written under `evidence/` so
`cargo clean` cannot authorize another attempt. They contain hashes,
version/build metadata, aggregate counts and PASS/FAIL only—never questions,
passages, individual identifiers, labels or scores.

The runner assumes a trusted maintainer account and a quiescent workspace. Its
exact manifests and repeated hashes detect accidental or persistent mutations;
they are not a defense against a malicious process running as the same user.

If the candidate fails, remove maintained runner code after recording the
outcome in the research ledger; Git history preserves the audited source. If it
passes, fresh rejection and human-reviewed promotion holdouts still precede any
Rust/ONNX product integration.
