# ADR 0015: Use SSL.com eSigner cloud-HSM signing and GitHub Releases update transport

- Status: Accepted
- Date: 2026-08-30
- Supersedes: ADR 0009 (Windows signing provider decision)
- Refines: ADR 0003 and ADR 0012

## Context

AirWiki could not complete SignPath onboarding, so it cannot rely on SignPath
for a public Windows publisher identity. [Azure Artifact Signing Public Trust](https://learn.microsoft.com/azure/artifact-signing/overview)
does not currently support validation in Uruguay. The project needs a provider
that keeps the private signing key non-exportable without introducing an
AirWiki-operated download or update service.

The app already uses the Tauri updater model: `latest.json` describes an update
whose bytes are verified with an embedded Tauri updater public key. GitHub
Releases can host that manifest and its release assets, but a GitHub release,
tag, redirect, checksum, or `Latest` label is not cryptographic authority.

## Decision

For Windows stable releases, AirWiki will use SSL.com eSigner with an OV code
signing certificate and the required eSigner tier after the legal entity has
been prevalidated. Provider validation, malware-scan, credential and
subscription requirements must be confirmed before purchase; this ADR does not
assume them as contractual facts. The signing key stays in SSL.com's cloud HSM and is never
exported to GitHub, a runner, or this repository. [SSL.com's eSigner documentation](https://www.ssl.com/guide/esigner-codesigntool-command-guide/)
defines the supported CKA and `signtool` invocation.

Normal builds remain secret-free. A separately protected `windows-signing`
environment, requiring a second maintainer's approval and no administrator
bypass, runs only after the reviewed build is ready. It obtains the eSigner
credentials `SSL_COM_ESIGNER_USERNAME`, `SSL_COM_ESIGNER_PASSWORD`,
`SSL_COM_ESIGNER_TOTP_SECRET`, and `SSL_COM_ESIGNER_CREDENTIAL_ID`. It installs
hash-pinned CKA from the reviewed `SSLcom/eSignerCKA` release and separately
hash-pinned CodeSignTool input, selects the explicit `10.0.26100.0/x64`
`signtool` from the GitHub-hosted Windows SDK and verifies its native signature
and file version, scans each
AirWiki-owned executable with the SSL.com CodeSignTool malware scan before
signing, invokes CKA plus `signtool` to sign the desktop, MCP bridge, firewall
helper, and MSI containers with a timestamp, then independently verifies the
result. The expected certificate fingerprint is the repository variable
`AIRWIKI_WINDOWS_SIGNER_SHA256`; display names are never an identity check. The
pinned third-party runtime remains verified, not re-signed.

The Tauri updater private key and its password remain separate protected
secrets. They sign the final update artifacts only after native signing and
release verification. The client exposes one stable channel, accepts only a
strictly newer release with a valid Tauri signature, rechecks Authenticode on a
downloaded Windows MSI, and requires explicit human confirmation before
installation. GitHub Releases hosts the stable release's `latest.json` and
exact assets; AirWiki runs no update server and does not delegate update
authority to GitHub.

Public technical prereleases remain unsigned or unnotarized, non-Latest,
manual-only and excluded from `latest.json`. This decision does not claim that
the current build is signed or that in-app updating works: stable release is
blocked until SSL.com entity onboarding, credential provisioning, a protected
signing run, and a real installed update acceptance test pass.

eSigner PIN and QR/TOTP enrollment are manual account-setup steps. A CI TOTP
seed is permitted only if SSL.com policy and two maintainer approvals explicitly
allow that custody; otherwise signing stays disabled. Certificate duration may
be up to 458 days, subject to the purchased product and current provider terms.

## Consequences

- The project must prevalidate its entity and purchase the SSL.com OV certificate
  and eSigner tier before Windows stable signing can start.
- GitHub Actions stays out of private-key custody, but the protected signing
  runner, SSL.com service, and approved tooling become part of the release TCB.
- An OV or EV certificate does not itself bypass Microsoft SmartScreen; Microsoft
  documents that reputation and policy still apply. See the [SmartScreen overview](https://learn.microsoft.com/windows/security/application-security/application-control/windows-defender-smartscreen/).
- Releases remain portable in operational terms: GitHub supplies distribution
  transport, while signed updater metadata and native verification supply the
  cryptographic trust decision.

## Rejected alternatives

- **SignPath:** unavailable to the project after failed onboarding.
- **Azure Artifact Signing Public Trust:** not currently available for Uruguay.
- **DigiCert KeyLocker:** acceptable cloud-HSM fallback if eSigner onboarding or
  commercial terms fail, but not the selected first path.
- **EV as a SmartScreen bypass:** unsupported; EV is not a guarantee of
  reputation or policy acceptance.
- **An AirWiki update server:** creates unnecessary operational and privacy
  surface when immutable GitHub Release assets satisfy distribution needs.
