# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io),
certificate by [SignPath Foundation](https://signpath.org).

This policy applies to Windows release candidates signed for the
[airwiki/airwiki](https://github.com/airwiki/airwiki) repository. AirWiki is
Apache-2.0 software. Signing does not turn a development candidate into a
supported public release; the release checklist and installed-platform gates
remain authoritative.

## Roles

- Author and committer: [Michael Pintos (`machester4`)](https://github.com/machester4).
- Reviewer: [Michael Pintos (`machester4`)](https://github.com/machester4). Changes
  from non-committers require review before merge. Trust-boundary changes also
  require the independent review process in [CODE_REVIEW.md](../CODE_REVIEW.md).
- Signing approver: [Michael Pintos (`machester4`)](https://github.com/machester4).
  Every signing request requires manual approval in the protected
  `windows-signing` GitHub environment and in SignPath.

All role holders must use multi-factor authentication for GitHub and SignPath.
Signing approval is separate from authoring a build and is never automated.

## Build and signing controls

- Only the pinned workflow in
  [`.github/workflows/windows-signpath.yml`](../.github/workflows/windows-signpath.yml)
  may request a production signature.
- Inputs are built from a reviewed commit on GitHub-hosted Windows runners. The
  SignPath GitHub connector verifies that origin; self-hosted artifacts are not
  accepted.
- AirWiki signs its desktop, MCP bridge and fixed-purpose firewall helper first.
  The MCPB and two localized MSI packages are then built from those exact bytes.
  A second request verifies the nested signatures and signs only the MSI
  containers.
- The pinned llama.cpp runtime is verified from its source-build receipt and is
  not re-signed as AirWiki. Missing, unexpected or mismatched payloads fail the
  workflow.
- SignPath tokens and updater private keys are restricted to the protected
  environment and are never stored in the repository or build artifacts.
- Signed outputs are independently checked for the expected certificate,
  timestamp, package identity, nested signer, payload equality, architecture,
  runtime receipt and MCPB identity before they can be promoted.

The committed artifact definitions are
[`windows-binaries.xml`](../.signpath/windows-binaries.xml) and
[`windows-msi.xml`](../.signpath/windows-msi.xml). Corresponding SignPath
configuration changes require the same reviewed repository change before use.

## Privacy

This program will not transfer any information to other networked systems
unless specifically requested by the user or the person installing or operating
it. AirWiki has no telemetry. Model downloads, opted-in LAN sharing, opted-in
public federation, update checks and user-enabled external integrations are
described before activation and remain independently controllable. See the
[threat model](threat-model.md) and [installation guide](install.md).

## Incident response

Stop signing and promotion when source origin, credentials, package contents or
publisher identity are uncertain. Revoke affected SignPath credentials and
request certificate or signature revocation through SignPath when appropriate.
Report suspected compromise using [SECURITY.md](../SECURITY.md); never attach
private knowledge, credentials or raw local logs.
