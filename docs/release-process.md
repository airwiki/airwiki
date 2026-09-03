# Public release process

This is AirWiki's planned supported-release process; it is not currently
operational. Stable release remains blocked until SignPath Foundation accepts
the project, maintainers complete its required account and policy review, a
protected signing rehearsal succeeds, updater-key custody is confirmed, and an
installed update acceptance pass. Once those gates pass, a two-phase GitHub
Actions process prepares a private draft tied to one reviewed commit, then
re-downloads and verifies it on macOS and Windows before protected human
approval makes it public. A workflow run never moves or overwrites an existing
release tag.

## Selected SignPath Foundation prerequisite (inactive)

ADR 0016 selects the Windows signing route. It is inactive and does
not assert provider acceptance. Before it can be used, SignPath Foundation must
accept the project and maintainers must confirm its then-current terms and
privacy information. It requires MFA for the public requester `machester4` and independent approver
`bryanTechera`, a separate manual SignPath approval, and protected token, slugs
and certificate fingerprint. The existing `windows-signing` environment is
main-only with self-review and administrator bypass disabled; it has no
SignPath token or variables today, so the reusable workflow fails closed before
any request. The public release notes must include `Free code signing provided by SignPath.io, certificate by SignPath Foundation` and link the [Code signing policy](code-signing-policy.md).

Unsigned candidates from **Package technical candidates** are a separate
technical-testing channel. A protected run may publish them as a clearly marked
`v<version>-beta.<number>` GitHub pre-release, but those assets are never inputs
to this stable process, updater metadata or evidence that native signing passed.

## Platform-split macOS RC and Windows technical beta

ADR 0017 defines a separate, limited public channel while the Windows stable
signing route remains unavailable. It is neither a global stable release nor an
updater channel. **Package platform release candidate** (`package-platform-rc.yml`)
creates a `v<version>-rc.<n>` GitHub prerelease containing only:

- one macOS arm64 Developer ID signed, notarized and stapled DMG, labeled a
  macOS release candidate; and
- two Windows x64 per-user MSI installers (`en-US` and `es-ES`), labeled
  unsigned technical betas.

Linux is deliberately absent. The release is always a prerelease and never
GitHub Latest. It creates no `latest.json`, updater assets or updater metadata.
The Windows installers remain manual downloads; their hashes, provenance,
GitHub Artifact Attestations and fresh-package MSI install/uninstall smoke do
not establish a Windows publisher identity.

The protected macOS job reuses the stable notarization script, so it creates
and verifies an updater archive and signature transiently. The job excludes
those files from the handoff and the ephemeral runner is discarded: only the
notarized DMG and its verification receipt are uploaded, and no updater
material reaches the GitHub release.

Dispatch it only from `main` with the exact current 40-character tip SHA:

```bash
gh workflow run package-platform-rc.yml --ref main \
  -f commit_sha=<current-main-40-character-sha> \
  -f rc_number=<1-through-9999> \
  -f publication_confirmation=publish-v<version>-rc.<rc_number>
```

Immediately before dispatch, an administrator must confirm GitHub release
immutability is enabled for the repository. The workflow verifies the final
published release reports `isImmutable: true`; a tag ruleset separately blocks
updates and deletion for `v*` tags.

The workflow derives `<version>` from the reviewed manifests and rejects a
non-tip SHA, malformed number or non-exact confirmation. It revalidates the
closed source and required checks, final asset inventory, hashes and
provenance. `macos-signing` approval is required before any Apple secret is
used. After macOS verification and the unsigned Windows MSI smoke complete,
`public-release` requires a second, separate approval before the draft is
created and published. Neither approval permits stable promotion.

Do not add the unsigned MSI files to a stable Latest release, and do not
relabel an RC as stable. A stable release still follows the process below and
requires every applicable [public release checklist](release-checklist.md) gate,
including Windows native signing, updater-key custody and installed acceptance.

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
them without entering a signing environment. Do not create or populate any
SignPath value before the Foundation accepts the project and maintainers review
the resulting configuration:

| Variable | Purpose |
| --- | --- |
| `AIRWIKI_UPDATER_PUBLIC_KEY` | Tauri updater public key embedded in release builds |
| `AIRWIKI_MACOS_TEAM_ID` | Expected Apple Developer team identifier |
| `AIRWIKI_WINDOWS_SIGNER_SHA256` | Expected SignPath Foundation leaf-certificate SHA-256 fingerprint, obtained after acceptance |

After acceptance only, set the following protected `windows-signing`
environment variables from the configuration issued for AirWiki:
`SIGNPATH_FOUNDATION_ENROLLMENT=approved`, `SIGNPATH_ORGANIZATION_ID`,
`SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`,
`SIGNPATH_BINARIES_CONFIGURATION_SLUG`, and
`SIGNPATH_MSI_CONFIGURATION_SLUG`. The exact values are not versioned. Missing,
unexpected, or unapproved values keep the signing job closed.

Set `AIRWIKI_MACOS_SIGNING_IDENTITY` in `macos-signing` to the complete
`Developer ID Application: ... (TEAMID)` identity. Add these protected secrets:

| Environment | Secret |
| --- | --- |
| `macos-signing` | `APPLE_DEVELOPER_ID_CERTIFICATE_P12_BASE64` |
| `macos-signing` | `APPLE_DEVELOPER_ID_CERTIFICATE_PASSWORD` |
| `macos-signing` | `APPLE_API_PRIVATE_KEY_BASE64` |
| `macos-signing` | `TAURI_SIGNING_PRIVATE_KEY` |
| `macos-signing` | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| `windows-signing` | `SIGNPATH_API_TOKEN` |
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

Before enabling the protected Windows job, obtain SignPath Foundation acceptance,
review the issued organization, project, policy, and configuration slugs, and
record the expected certificate fingerprint in the protected configuration. The
workflow submits two separately origin-verified requests: first the three
AirWiki-owned PE files, then the two localized MSI containers. It selects the
pinned Microsoft `signtool` and verifies `/pa /all /tw`, indexed signatures,
code-signing and timestamp EKUs, the expected fingerprint, and nested MSI
payload signatures. The `windows-signing` approval is independent from
authoring the build, and SignPath retains the signing key in its service/HSM.
The visible Windows publisher is SignPath Foundation; SmartScreen and
organization policy may still warn or block a signed download. See the [code-signing policy](code-signing-policy.md).

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

1. builds secret-free Windows binaries, then the protected SignPath job submits
   the origin-verified AirWiki-owned PE files and verifies their signatures with
   `signtool`;
2. builds two localized MSI packages, submits their origin-verified containers
   for outer signatures, verifies the nested payload and final MSI signatures, and signs
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
4. independently re-verifies the SignPath Foundation certificate fingerprint,
   nested Windows payload and both
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
