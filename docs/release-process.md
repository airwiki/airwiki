# Public release process

This is AirWiki's planned supported-release process; it is not currently
operational. Stable release remains blocked until SSL.com entity onboarding,
eSigner credentials, a protected signing run, updater-key custody, and an
installed update acceptance pass. Once those gates pass, a two-phase GitHub
Actions process prepares a private draft tied to one reviewed commit, then
re-downloads and verifies it on macOS and Windows before protected human
approval makes it public. A workflow run never moves or overwrites an existing
release tag.

## Proposed SignPath Foundation prerequisite

ADR 0016 is a proposed, Windows-only alternative that has not replaced the
current provider decision. If accepted, it requires SignPath Foundation project
acceptance, MFA for the public requester `machester4` and independent approver
`bryanTechera`, a separate manual SignPath approval, and protected token, slugs
and certificate fingerprint. The existing `windows-signing` environment is
main-only with self-review and administrator bypass disabled; it has no
SignPath token or variables today, so the reusable workflow fails closed before
any request. The public release notes must include `Free code signing provided by SignPath.io, certificate by SignPath Foundation` and link the [Code signing policy](code-signing-policy.md).

Unsigned candidates from **Package technical candidates** are a separate
technical-testing channel. A protected run may publish them as a clearly marked
`v<version>-beta.<number>` GitHub pre-release, but those assets are never inputs
to this stable process, updater metadata or evidence that native signing passed.

Users obtain the current stable version from
[GitHub Releases](https://github.com/airwiki/airwiki/releases/latest):

- Apple silicon Macs use `AirWiki_<version>_aarch64.dmg`;
- Windows users select the `en-US` or `es-ES` per-user MSI; and
- the application updater reads only the stable `latest.json`, whose artifact
  entries carry detached Tauri signatures, attached to the latest release.

Drafts and prereleases are not updater channels. The Windows updater uses the
`en-US` MSI because both localized packages contain byte-identical product
payloads and updates run without the installer UI. Both MSI files remain
available for first installation.

The technical pre-release path has its own closed provenance, cross-platform
checksums, GitHub Artifact Attestation and bilingual warning. After assembling
the final closed asset directory and before creating a draft, the protected job
uses GitHub Actions OIDC to attest those exact bytes. A failure stops
publication. The path fixes GitHub Latest and updater eligibility to false,
labels the macOS DMG as ad-hoc and non-notarized, labels both Windows MSI files
as unsigned, and describes the Linux x64 artifact only as the federation index
server. Its attestation and protected publication approval establish build
provenance and visibility, not stable support, software safety or native
publisher identity.

GitHub Releases may redirect manifest and artifact requests to GitHub-managed
object storage. A redirect is transport, never authority: the client still
accepts only a strictly newer stable version and verifies the downloaded bytes
with the embedded Tauri updater key before an explicit installation
confirmation. Windows also rechecks the downloaded MSI's Authenticode identity;
native signatures for Windows and Developer ID signing plus notarization for
macOS are checked before
publication. Release verification binds the manifest URLs and signatures to the
exact allowlisted assets; no redirect or hosting URL can substitute unsigned
bytes.

## One-time repository configuration

Create protected GitHub environments named `macos-signing`, `windows-signing`
and `public-release`. Restrict deployment branches to `main`. Require a human
reviewer for both signing environments and for `public-release`; do not allow an
initiating administrator to bypass the final promotion approval.
The technical pre-release publisher also uses `public-release` for the narrow
act of making an already verified draft public; it receives no signing secret
and cannot create stable or updater metadata.
Protect tags matching `v*` against updates and deletion. The promotion workflow
creates the exact release tag only after approval and never moves an existing
tag.

Keep the default workflow token read-only, require every external GitHub Action
to use an immutable commit SHA, and require the macOS and Windows frontend,
Rust, supply-chain and DCO checks on `main`. Enable private vulnerability
reporting, secret scanning with push protection, Dependabot security updates
and GitHub CodeQL default setup before accepting a public candidate.

Configure these non-secret repository variables so verification jobs can use
them without entering a signing environment:

| Variable | Purpose |
| --- | --- |
| `AIRWIKI_UPDATER_PUBLIC_KEY` | Tauri updater public key embedded in release builds |
| `AIRWIKI_MACOS_TEAM_ID` | Expected Apple Developer team identifier |
| `AIRWIKI_WINDOWS_SIGNER_SHA256` | Expected SSL.com eSigner leaf-certificate fingerprint |
| `AIRWIKI_ESIGNER_SECRET_TRANSPORT_APPROVED` | Exact protected-environment gate for SSL.com-confirmed secret transport |

Set `AIRWIKI_ESIGNER_SECRET_TRANSPORT_APPROVED` only in the protected
`windows-signing` environment and only to `sslcom-esigner-secret-transport-v1`
after SSL.com has confirmed the secret transport in writing or the approvers have
explicitly accepted the residual risk. Any other value keeps stable signing
closed.

Set `AIRWIKI_MACOS_SIGNING_IDENTITY` in `macos-signing` to the complete
`Developer ID Application: ... (TEAMID)` identity. Add these protected secrets:

| Environment | Secret |
| --- | --- |
| `macos-signing` | `APPLE_DEVELOPER_ID_CERTIFICATE_P12_BASE64` |
| `macos-signing` | `APPLE_DEVELOPER_ID_CERTIFICATE_PASSWORD` |
| `macos-signing` | `APPLE_API_PRIVATE_KEY_BASE64` |
| `macos-signing` | `TAURI_SIGNING_PRIVATE_KEY` |
| `macos-signing` | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| `windows-signing` | `SSL_COM_ESIGNER_USERNAME` |
| `windows-signing` | `SSL_COM_ESIGNER_PASSWORD` |
| `windows-signing` | `SSL_COM_ESIGNER_TOTP_SECRET` |
| `windows-signing` | `SSL_COM_ESIGNER_CREDENTIAL_ID` |
| `windows-signing` | `TAURI_SIGNING_PRIVATE_KEY` |
| `windows-signing` | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |

`APPLE_API_KEY_ID` and `APPLE_API_ISSUER_ID` are non-secret variables in
`macos-signing`. The API key must be an App Store Connect **team** key accepted
by `notarytool`, not an individual API key. The `.p8` authorizes notarization
API access; it is not a code-signing certificate. Generate the Developer ID
Application certificate from a Certificate Signing Request, then export that
identity and its private key as a `.p12`. Encode the downloaded `.p8` and the
exported Developer ID `.p12` as single-line base64 values before storing them.
The workflow decodes them only inside an ephemeral directory and deletes its
temporary keychain at the end of the signing step.

Creating or rotating the App Store Connect API key, assigning its minimal
notarization role, and approving protected environments are human account
administration gates. Do not run the stable workflow until the account owner has
confirmed those approvals; no CI setting substitutes for them.

## Rehearse macOS signing and notarization

After the `macos-signing` environment is configured, run **Rehearse macOS
notarization** from `main` before preparing any stable draft. Provide the exact
current 40-character `commit_sha` and set `rehearsal_confirmation` to exactly
`rehearse-macos-notarization-<commit_sha>`. The workflow permits only the clean,
current green tip of `airwiki/airwiki` `main`, uses the same protected macOS
identity and ephemeral keychain/API-key handling as release preparation, and
verifies the final DMG and updater archive. Immediately before recording any
evidence it again requires that the checkout and `origin/main` resolve to that SHA,
that the checkout is clean, and that every named release context has exactly one
successful run from the GitHub Actions app: both Frontend and Rust platform
checks, Advisories/licenses/sources, and Launch site checks. It repeats that
same revalidation in a separate, credential-free step immediately before the
step that creates a keychain, decodes a secret or submits anything for
notarization. `Sign-off` is intentionally not queried on a
`main` SHA: it is a pull-request-only DCO check, enforced by branch protection
before the merge that created `main`. It never uploads the signed binaries:
GitHub Actions artifacts are not a private package channel. The only retained
evidence is a 14-day JSON receipt with the commit, version, verification result,
artifact names and SHA-256 values; it contains no installer bytes, paths,
signing identity or credentials. It cannot create tags or releases, publish
`latest.json`, or request OIDC credentials; a successful rehearsal is evidence
of the macOS path only and does not promote a release.

Before enabling the protected job, prevalidate the project entity with SSL.com
and purchase the OV code-signing certificate plus required eSigner tier. Keep
the CKA installer from the reviewed `SSLcom/eSignerCKA` release and CodeSignTool
archive versions and hashes in reviewed configuration; the build fails if either
hash differs. The workflow selects only the explicit `10.0.26100.0/x64`
`signtool` from the Windows SDK and requires a valid native signature and
matching file-version prefix. The `windows-signing` approval is independent from authoring the
build, and the certificate key remains non-exportable in SSL.com's cloud HSM.
See the [code-signing policy](code-signing-policy.md).

Generate the Tauri updater key once on a trusted administrative Mac with the
pinned repository CLI:

```bash
apps/desktop/ui/node_modules/.bin/tauri signer generate \
  --write-keys /private/offline/location/airwiki.key
```

Store the encrypted private key, its password and a tested offline recovery copy
separately. Commit neither key. Copy only the public key into
`AIRWIKI_UPDATER_PUBLIC_KEY`. The same private key must be configured in both
platform signing environments so one embedded public key verifies every update.

Apple Developer Program membership alone is not a signing credential. Create a
Developer ID Application certificate in the Apple developer account, install it
with its private key on the administrative Mac, export the identity as a
password-protected `.p12`, and create the App Store Connect team API key used for
notarization. The private key is downloadable only once; preserve it offline.

## Version preparation

Version changes are normal reviewed pull requests. Update every version-bearing
source to the same stable `major.minor.patch` value:

- `Cargo.toml` under `[workspace.package]`;
- every explicit `version` on an internal AirWiki path dependency in workspace
  `Cargo.toml` files;
- `apps/desktop/tauri.conf.json`; and
- `apps/desktop/ui/package.json`.

Run the contract locally:

```bash
node packaging/release-version.mjs --expect 0.2.0 --tag v0.2.0
node --test packaging/tests/release-version.test.mjs
python3 -m unittest discover -s packaging/tests -p 'test_*.py'
```

Replace the example with the intended version. CI rejects prerelease syntax,
missing values and drift between manifests. Merge the version pull request and
wait for every macOS, Windows, frontend and supply-chain check on the resulting
`main` commit to succeed.

## Prepare a private draft

From the Actions page, run **Prepare signed public release** on `main` with:

- `version`: the exact stable version without `v`;
- `commit_sha`: the complete 40-character SHA at the current tip of `main`.

The workflow fails closed if the commit moved, a tag or release already exists,
the manifests differ, or the required checks are not green. It then:

1. builds secret-free Windows binaries, then the protected eSigner job scans and
   signs AirWiki-owned files with CKA plus `signtool`;
2. builds two localized MSI packages, obtains their outer signatures and signs
   their final bytes for the Tauri updater;
3. imports the ephemeral Developer ID identity, builds the macOS app, notarizes
   and staples the app and DMG, restores only validated EULA resources while
   rebuilding the DMG, verifies its image checksums, and signs the final updater
   archive;
4. verifies native identity, architecture, payload, MCPB, runtime and updater
   signatures on each platform; and
5. creates a private draft prerelease containing the exact installers, updater
   artifacts and manifest, hashes, SPDX SBOM, provenance and legal inventories.

The generated `SHA256SUMS` covers every draft asset except itself. Provenance is
bound to the repository, full commit, version and workflow run. The SPDX file
enumerates the final release files and the exact Cargo/npm dependency inventories
used by the packages. Promotion compares every legal payload and the complete
SBOM dependency model with the files in the checked-out release commit. It also
requires both copies of the notarized macOS application to carry the exact
requested bundle and build version. `latest.json` remains private with the draft
and its bytes are covered by those metadata checks. Promotion also requires its version,
publication time, platform keys, artifact URLs and updater signatures to match
the exact files in that release.

## Acceptance and promotion

Download the private draft through an authenticated maintainer session. Complete
every applicable item in the [public release checklist](release-checklist.md),
including clean installation, upgrade, LAN/public search and the two-node runbook.
Record only sanitized evidence.

When acceptance is complete, run **Promote verified stable release** with the
same version. The workflow:

1. requires either the exact private draft or its exact stable recovery state,
   tied to a full commit still reachable from `main`;
2. re-downloads the exact asset allowlist and verifies every SHA-256, provenance
   entry and SBOM identity;
3. independently re-verifies Developer ID, notarization, stapling and the macOS
   updater signature, explicitly accepts the bundled Apache-2.0 agreement for
   the noninteractive mount, and compares its application bytes with the updater;
4. independently re-verifies SSL.com certificate fingerprint, nested Windows payload and both
   MSI updater signatures; and
5. waits at the protected `public-release` environment.

The first verification fingerprints the complete `SHA256SUMS` inventory. Both
native platform jobs require that exact inventory and verify their downloaded
bytes against it. Immediately before publication, the protected job downloads
the complete draft again, revalidates the release target, creates or resolves the
immutable tag at the verified commit, and refuses to continue if the inventory
fingerprint or target changed while approval was pending.

A new candidate must be strictly newer than the current stable release. Recovery
of an interrupted publication is accepted only when that exact release is
already GitHub's latest stable version; promotion never moves the latest pointer
backward.

After approval, it publishes that same verified draft as the latest stable
release without changing its assets. Consequently `latest.json` and the
installers become visible together, the promotion is safe to retry after an
interruption, and the updater manifest can never point at an unpublished
prerelease.

If GitHub completed publication but the runner stopped before recording the
final checks, rerunning promotion accepts only that exact stable recovery state.
It repeats the complete metadata and native platform verification, passes
through `public-release` approval again and then confirms the release is the
stable latest version. Mixed draft/prerelease states always fail closed.

Do not delete or move old stable tags, assets or updater manifests. If any gate
fails, leave the candidate private, correct the source through a new pull request
and prepare a new version. Stop all signing and promotion when credential,
publisher, source-origin or package integrity is uncertain.
