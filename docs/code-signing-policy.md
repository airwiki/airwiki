# Code signing policy

This policy covers future Windows stable candidates for
[airwiki/airwiki](https://github.com/airwiki/airwiki). It is an operational
target, not evidence of a completed signing program: Windows signing and the
stable updater remain blocked until SSL.com onboarding, protected credentials,
and an installed acceptance run succeed.

## Unsigned technical pre-release boundary

Until that gate passes, a technical prerelease may be published only as an
explicitly unsigned Windows beta. It has an immutable beta tag, `Latest` and
updater eligibility set false, exact SHA-256 inventory, closed provenance and a
GitHub Artifact Attestation over every final release asset. The attestation uses
GitHub Actions OIDC and Sigstore without an AirWiki attestation key. It proves
which official workflow and commit produced the bytes; it does not prove safety
and never supplies a Windows publisher identity. The prerelease receives no
`latest.json`, native-signing credential, eSigner request, or updater key.
Testers keep SmartScreen, Smart App Control, antivirus, and organization policy
enabled and stop if the device blocks the candidate.

## Chosen trust boundary

AirWiki will use [SSL.com eSigner](https://www.ssl.com/esigner/) with an OV code
signing certificate and the eSigner tier required for cloud signing. Before a
purchasing commitment, the project entity must be prevalidated with SSL.com and
the required validation, malware-scan, credential and eSigner subscription terms
must be confirmed in writing. The certificate
private key remains non-exportable in SSL.com's cloud HSM; neither developers,
GitHub Actions, nor repository files receive it.

Azure Artifact Signing Public Trust does not currently admit Uruguay, so it is
not viable for this project today. [DigiCert KeyLocker](https://www.digicert.com/signing/keylocker)
is the fallback if eSigner onboarding or commercial terms fail. An EV certificate
does not guarantee that Microsoft SmartScreen or organization policy will allow
a download; keep platform protections enabled and use Microsoft's [SmartScreen guidance](https://learn.microsoft.com/windows/security/application-security/application-control/windows-defender-smartscreen/)
as the authority on that behavior.

## Roles and secrets

The GitHub `windows-signing` environment is limited to `main`, requires a
second maintainer's approval, and disables self-review and administrator bypass.
It is the only environment permitted to read these secrets:

| Secret | Purpose |
| --- | --- |
| `SSL_COM_ESIGNER_USERNAME` | eSigner account authentication |
| `SSL_COM_ESIGNER_PASSWORD` | eSigner account authentication |
| `SSL_COM_ESIGNER_TOTP_SECRET` | time-based second factor used only in the protected job |
| `SSL_COM_ESIGNER_CREDENTIAL_ID` | selected eSigner cloud credential |
| `TAURI_SIGNING_PRIVATE_KEY` | separate Tauri updater-artifact signature |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | decrypts the separate Tauri key |

`AIRWIKI_WINDOWS_SIGNER_SHA256` is a non-secret repository variable containing
the expected SSL.com leaf certificate SHA-256 fingerprint. Verification compares
the certificate to that value; publisher display text is insufficient. Rotation
requires a reviewed overlap that accepts the old and new fingerprints, while
CKA loads only the one active signing certificate, followed by a verified
installed release.

## Build, scan, sign, verify

1. Normal CI builds and validates unsigned artifacts without signing or updater
   secrets.
2. The approved `windows-signing` runner obtains CKA only from the reviewed
   [SSLcom/eSignerCKA release](https://github.com/SSLcom/eSignerCKA/releases)
   and obtains CodeSignTool as a separate, reviewed SSL.com distribution. It
   verifies immutable SHA-256 pins for both files. Those pins must be renewed by
   reviewed source evidence when the provider changes a release; they are not a
   claim that an unfingerprinted vendor URL is reproducible. External Actions are
   commit-pinned; the exact JDK is selected by version. The job accepts only
   `signtool.exe` at the explicit `10.0.26100.0/x64` Windows SDK path, verifies
   its native Microsoft signature, exact `Microsoft Corporation` publisher name
   and file-version prefix, and fails closed when the hosted runner lacks that
   SDK. GitHub's current [Windows Server 2022 runner image inventory](https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md)
   lists that SDK; the workflow still treats a runner-image change as a failure.
   The repository does not claim a standalone
   Windows SDK file hash because Microsoft has not supplied one in reviewed
   project evidence.
3. Before any signature request, the job scans every AirWiki-owned PE file with
   SSL.com's [CodeSignTool malware scan](https://www.ssl.com/guide/code-signing-malware-scan/).
   A scan error, unknown result, unexpected file, or malware finding fails
   closed.
4. The job uses the documented eSigner CKA provider and `signtool` to timestamp
   and sign the desktop, MCP bridge, firewall helper, and both MSI containers.
   The pinned upstream runtime is hash-verified and is not re-signed as AirWiki.
5. Independent verification checks Authenticode, code-signing EKU, timestamp,
   `AIRWIKI_WINDOWS_SIGNER_SHA256`, MSI nesting, exact payload hashes, PE
   metadata, MCPB/bridge identity, and the fixed per-user package contract.
6. Only after native verification does the independent Tauri private key sign
   the final updater artifacts. It is never substituted for native signing.

No signer credential, HSM key, TOTP seed, updater key, certificate file, or
unverified tool binary is committed, retained in an artifact, or available to a
normal build.

eSigner enrollment may require a user PIN and QR/TOTP setup. That is a manual
account-enrollment step, not a CI workflow input. Store a CI TOTP seed only if
the SSL.com account policy explicitly permits it and the two maintainers approve
that custody; otherwise the protected job remains disabled rather than bypassing
the second factor. SSL.com certificate validity may be up to 458 days, subject
to the purchased product and current provider terms; track the actual expiration
and renew before the protected identity becomes unusable.

GitHub masks direct secret values automatically, and the signing workflow adds
explicit masks before invoking either vendor tool. Do not derive, echo or write
the password or TOTP seed to a file; if a future vendor command requires a
derived one-time value, confirm a stdin or restricted-file interface with SSL.com
first. Until that interface and its logging behavior are confirmed, stable
signing remains disabled rather than placing a derived secret in an argv.

`AIRWIKI_ESIGNER_SECRET_TRANSPORT_APPROVED` is a protected
`windows-signing` environment variable, not a repository-wide setting. Its
exact value is `sslcom-esigner-secret-transport-v1`. A maintainer may set it
only after retaining SSL.com's written confirmation of the relevant secret
transport, or after an explicit documented acceptance of the remaining argv
risk by the protected-environment approvers. Missing, stale or different values
stop the signing job before it invokes CKA or CodeSignTool.

## Update distribution

AirWiki has no project-operated update server. The single stable channel uses
the stable [GitHub Release](https://github.com/airwiki/airwiki/releases) to host
`latest.json` and its exact assets. GitHub and any object-storage redirect are
untrusted transport: the desktop accepts only a newer version whose updater
signature verifies against the embedded public key and asks the user to confirm
installation. Windows additionally rechecks the downloaded MSI's Authenticode
identity before launch. Windows Authenticode and macOS Developer ID signing plus
notarization are verified during release preparation and promotion. Prereleases,
drafts, and
technical beta assets contain no `latest.json` and are never update sources.

## Incident response

Stop signing and promotion on uncertainty about source, tool hashes, scan,
credentials, certificate fingerprint, payload, or update signature. Revoke or
disable the affected SSL.com credential, suspend the stable channel if needed,
preserve sanitized evidence, and follow [SECURITY.md](../SECURITY.md). Never
publish a compensating unsigned update.
