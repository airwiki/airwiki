# Public release process

AirWiki publishes supported binaries through a two-phase GitHub Actions process.
Preparation creates a private draft tied to one reviewed commit. Promotion
re-downloads and verifies that draft on macOS and Windows before a protected
human approval makes it public. A workflow run never moves or overwrites an
existing release tag.

Unsigned candidates from **Package technical candidates** are a separate
technical-testing channel. A protected run may publish them as a clearly marked
`v<version>-beta.<number>` GitHub pre-release, but those assets are never inputs
to this stable process, updater metadata or evidence that native signing passed.

Users obtain the current stable version from
[GitHub Releases](https://github.com/airwiki/airwiki/releases/latest):

- Apple silicon Macs use `AirWiki_<version>_aarch64.dmg`;
- Windows users select the `en-US` or `es-ES` per-user MSI; and
- the application updater reads only the signed `latest.json` attached to the
  latest stable release.

Drafts and prereleases are not updater channels. The Windows updater uses the
`en-US` MSI because both localized packages contain byte-identical product
payloads and updates run without the installer UI. Both MSI files remain
available for first installation.

The technical pre-release path has its own closed provenance, cross-platform
checksums and bilingual warning. It fixes GitHub Latest and updater eligibility
to false, labels the macOS DMG as ad-hoc and non-notarized, labels both Windows
MSI files as unsigned, and describes the Linux x64 artifact only as the
federation index server. Its protected publication approval grants visibility,
not stable support or native publisher identity.

GitHub Releases may redirect manifest and artifact requests to GitHub-managed
object storage. A redirect is transport, never authority: the client still
accepts only a strictly newer stable version and verifies the downloaded bytes
with the embedded Tauri updater key before native package verification and an
explicit installation confirmation. Release verification binds the manifest
URLs and signatures to the exact allowlisted assets; no redirect or hosting URL
can substitute unsigned bytes.

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
| `AIRWIKI_WINDOWS_SIGNER_SHA256` | Expected SignPath leaf-certificate fingerprint |

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
by `notarytool`, not an individual API key. Encode the downloaded `.p8` and the
exported Developer ID `.p12` as single-line base64 values before storing them.
The workflow decodes them only inside an ephemeral directory and deletes its
temporary keychain at the end of the signing step.

Keep the SignPath organization, project, signing policy, binary configuration
and MSI configuration variables in the `windows-signing` environment as described by the
[code-signing policy](code-signing-policy.md). SignPath approval remains
independent from GitHub environment approval.

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

1. builds origin-verified Windows binaries and obtains SignPath signatures;
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
4. independently re-verifies SignPath identity, nested Windows payload and both
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
