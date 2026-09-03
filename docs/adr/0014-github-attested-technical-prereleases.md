# ADR 0014: Attest public technical prerelease assets with GitHub

- Status: Accepted
- Date: 2026-08-31
- Refines: ADR 0012 public technical prereleases

## Context

AirWiki is an early open-source project without an operational Windows public
signing identity. Purchasing and operating native Windows signing or adapting
the application to another distribution platform is disproportionate to the
current technical-beta goal. A checksum inventory detects changed bytes, but a
checksum downloaded from the same release does not independently establish
which repository, commit and workflow produced them.

GitHub Artifact Attestations can bind an artifact digest to GitHub Actions OIDC
identity and Sigstore transparency evidence without a project signing key. They
establish build provenance for a public repository, but Windows and macOS do not
treat them as Authenticode, Developer ID or notarization. An attestation also
does not prove that source code or a workflow is safe.

## Decision

The protected `publish-technical-prerelease` job attests every regular file in
the final, closed release asset directory after `technical_prerelease.py
prepare` verifies the set and before any GitHub draft is created or recovered.
The `actions/attest` dependency is pinned to an immutable commit. The job
receives only the existing release-content permission plus `id-token: write`
and `attestations: write`; no long-lived attestation key or additional secret is
stored.

An attestation failure stops publication. The workflow continues to upload and
re-download the same closed assets, verify their SHA-256 inventory and verify
the downloaded bytes against the GitHub attestation before it binds the tag to
the workflow commit and publishes only a non-Latest prerelease. It does not
create `latest.json`, enter a native-signing environment or make the artifacts
eligible for the updater.

Testers verify a downloaded asset against both the official repository and the
exact signer workflow on a GitHub-hosted runner:

```text
gh attestation verify <asset> --repo airwiki/airwiki --signer-workflow airwiki/airwiki/.github/workflows/package-pilot.yml --source-ref refs/heads/main --source-digest <commit> --deny-self-hosted-runners
```

`SHA256SUMS.txt` remains part of the release for local integrity checks and
closed-set verification. Neither mechanism is described as an operating-system
publisher identity or a guarantee that the artifact is safe.

This makes explicitly reviewed, attested prereleases the only current public
binary channels. ADR 0017 refines this decision for its platform-split
`v<version>-rc.<n>` channel while preserving the same closed-set, provenance and
non-updater guarantees. Native stable signing and the updater remain deferred
and inactive; activating or replacing either path requires a separate reviewed
decision and does not weaken their release gates.

## Consequences

- Public beta users can verify that exact release bytes came from the official
  GitHub workflow and commit without an AirWiki company account, certificate or
  attestation secret.
- Online verification depends on GitHub and Sigstore availability. GitHub's
  documented offline bundle flow remains available to advanced consumers.
- Windows may still warn about or block the unsigned MSI, and macOS may still
  reject the ad-hoc, non-notarized application. Testers keep those protections
  enabled.
- A compromised reviewed workflow can produce a valid attestation for harmful
  bytes. Branch protection, pinned Actions, review and the closed asset verifier
  remain required.
- Normal CI and internal candidate artifacts do not receive release
  attestations. Only the final public prerelease asset set is in scope.

## Rejected alternatives

- **Describe GitHub provenance as native code signing:** Windows and macOS do not
  consume it as publisher trust and the description would mislead testers.
- **Add a separate Cosign signing flow:** duplicates the Sigstore machinery
  already supplied by GitHub and adds another verification contract.
- **Attest intermediate build outputs:** proves different bytes from the files a
  tester downloads.
- **Activate the Tauri updater for technical prereleases:** introduces a
  long-lived release key and conflicts with the explicit non-updater beta
  boundary.
