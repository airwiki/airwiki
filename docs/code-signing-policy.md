# Code signing policy

This policy describes a proposed Windows signing route for future stable AirWiki releases. It is not evidence of provider acceptance, a certificate, credentials, project configuration, or a supported signed download.

## Technical pre-release boundary

Technical prereleases remain unsigned/manual-only, non-Latest and excluded from the updater. GitHub Artifact Attestations establish build provenance, not a Windows publisher identity. Keep platform protections enabled and stop if a device blocks a candidate.

## Proposed SignPath Foundation route

ADR 0016 is Proposed. Before application, maintainers must confirm current SignPath Foundation terms and the project's [privacy policy](../PRIVACY.md). The Windows publisher would be SignPath Foundation. Every stable release note and this policy must disclose exactly: `Free code signing provided by SignPath.io, certificate by SignPath Foundation`.

Proposed GitHub roles: `machester4` is author/committer; `machester4` and `bryanTechera` are reviewers; both are signing approvers, except that the person who initiated a request never approves it. MFA is required before application and must be confirmed, not assumed. SignPath's manual approval is separate from GitHub environment approval.

## Controlled build and signing

Normal builds are secret-free. After acceptance only, `windows-signing` may hold a SignPath API token, organization/project/policy/configuration slugs and the expected certificate fingerprint. It is main-only, has no self-review or admin bypass, and fails closed until enrollment is explicitly approved. No value is versioned. SignPath retains its signing key in its service/HSM; AirWiki submits only origin-verified AirWiki-owned binaries from GitHub Actions.

The protected job independently verifies Windows signatures with the pinned Microsoft `signtool`, `verify /pa /all /tw`, indexed-signature checks, code-signing and timestamp EKUs, certificate fingerprint and signer consistency. It signs updater artifacts only after native PE and MSI validation. SmartScreen and organization policy can still block a signed download; signing is not a reputation guarantee.

## Incident response

Stop signing and promotion when origin, approval, signature, timestamp, fingerprint, artifact layout or provider status is uncertain. Disable the Foundation/SignPath route, preserve only sanitized evidence, and follow [SECURITY.md](../SECURITY.md). Never ship an unsigned compensating stable update.
