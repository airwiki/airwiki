# ADR 0016: Propose SignPath Foundation for Windows stable-signing review

- Status: Proposed
- Date: 2026-08-31
- Conditional relationship: would replace only the Windows signing-provider choice in ADR 0015 if accepted; ADR 0014 remains unchanged

## Context

AirWiki's public technical prereleases remain unsigned and are not updater
inputs. The project needs a no-cost Windows-native signing route suitable for a
public Apache-2.0 repository, without exporting a private signing key. SignPath
Foundation may provide that route, but acceptance, project configuration,
certificate identity, manual request approval, and its current terms must first
be confirmed by maintainers. This proposal does not assert that an application
has been accepted or that a privacy notice, secret, slug, or certificate exists.

## Decision

If SignPath Foundation accepts AirWiki and maintainers accept the terms,
Windows stable candidates will be built secret-free on GitHub-hosted runners and
submitted as two origin-verified requests: the three AirWiki PE files, then the
two per-user localized MSI containers. The `windows-signing` environment remains
main-only and requires independent review; `machester4` is the release requester
and `bryanTechera` is the independent reviewer/approver. MFA must be confirmed
before application and the SignPath request requires its separate manual approval.

The workflow is intentionally fail-closed until the protected environment has
an explicit enrollment gate, a SignPath API token, approved organization/project/
policy/configuration slugs, and an expected certificate fingerprint. None is
versioned. Verification independently invokes `signtool verify /pa /all /tw`,
checks timestamp and EKUs, expected certificate fingerprint and signer
consistency for PE and MSI payloads, then signs updater bytes only afterwards.

The public policy and each stable release note must disclose exactly:

`Free code signing provided by SignPath.io, certificate by SignPath Foundation`

While Proposed, this ADR supersedes nothing. If accepted it would replace only
the Windows provider choice in ADR 0015 while retaining its GitHub Releases
transport; it does not alter ADR 0014, macOS notarization, GitHub Artifact
Attestations, the updater design, or the per-user MSI/data boundary.

## Consequences

- The visible Windows publisher is SignPath Foundation, not AirWiki.
- A stable Windows release remains blocked until acceptance, protected
  configuration, a signed rehearsal, and clean-machine acceptance evidence.
- Before application and release use, maintainers must review SignPath
  Foundation's then-current terms and privacy information; the published
  signing policy already links AirWiki's [privacy policy](../../PRIVACY.md).
- Rejection, a configuration mismatch, missing approval, or verification failure
  stops the release; unsigned technical prereleases remain the only fallback.

## Rejected alternatives

- **Buying a commercial certificate now:** exceeds the current project budget.
- **Treating GitHub attestations as Authenticode:** provenance does not create a
  Windows publisher identity.
- **Activating SignPath credentials before acceptance:** adds a trust boundary
  without verified provider configuration.
- **Changing macOS signing or updater transport:** outside this Windows-only
  proposal.
