# ADR 0012: Publish unsigned technical candidates as non-latest pre-releases

- Status: Accepted
- Date: 2026-08-25
- Refines: ADR 0003 (prerelease distribution) and ADR 0009 (unsigned Windows
  candidates)
- Refined by: ADR 0014 (GitHub build-provenance attestations)

## Context

GitHub Actions artifacts expire and require an authenticated repository session.
That is useful for a narrow internal test, but it makes sustained beta feedback
unnecessarily difficult while AirWiki waits for Windows public-trust signing and
macOS distribution identities. Publishing those same bytes as a stable release
would instead imply native publisher trust, activate the normal download path
and risk coupling unsigned bytes to the updater contract.

The available platform outputs are also asymmetric. macOS produces an Apple
silicon desktop application with an ad-hoc signature, Windows produces two
localized unsigned x64 MSI packages, and Linux produces only the x64 public
federation index server. AirWiki Desktop does not yet support Linux. A download
surface must make those differences unmistakable.

## Decision

AirWiki may publish a permanent GitHub technical pre-release from the exact,
clean tip of `main` with a tag shaped as `v<stable-version>-beta.<number>`. It is
always a GitHub pre-release, explicitly not `Latest`, and contains no
`latest.json`, updater signature or supported-public-release claim.

One manually dispatched workflow builds macOS arm64, Windows x64 and the Linux
x64 federation index from the same commit. Publication depends on all three
jobs and waits at the protected `public-release` environment. The initiating
user must also enter the exact derived tag as a bounded confirmation. Signing
environments, SignPath and updater credentials are not available to this path.

Before publication, a platform-neutral verifier accepts only the expected DMG,
two MSI files and x86-64 ELF server. It rejects symlinks, escapes, unexpected
formats, another repository, malformed version or commit identity, and an
existing output. It creates a closed asset set with reviewed legal payloads,
strict UTF-8 bilingual guidance, exact SHA-256 values and provenance that fixes:

- `supportedPublicRelease`, `updaterChannel` and `latest` to `false`;
- macOS trust to ad-hoc and not notarized;
- Windows Authenticode state to not signed; and
- Linux to a non-desktop federation-index server.

Installer filenames repeat those limitations. Linux is a deterministic archive
containing the executable, a maintainer notice and its legal payloads. The
workflow first creates or recovers only an exact private draft, uploads the
closed set, downloads it again and verifies every byte. Only then does it
publish the draft as a non-latest pre-release. A published tag is immutable and
the workflow never replaces assets on an already public release.

Stable release preparation, native signature verification and updater promotion
remain unchanged. A technical pre-release provides source and transport
provenance, not an operating-system publisher identity. Testers keep platform
and organization protections enabled and stop when policy refuses the package.

## Consequences

- Testers can download one durable, public and hash-verifiable candidate without
  GitHub Actions access or a 30-day deadline.
- The Releases page contains unsupported test software before the first stable
  release, so titles, filenames, notes and README links must preserve the
  pre-release boundary.
- Windows can warn about or block the MSI. macOS can reject the non-notarized
  application. Those outcomes are expected limitations, not gates to bypass.
- Linux maintainers receive a deployable federation service but Linux users do
  not receive or infer a desktop application.
- Each beta number is immutable. Corrections require a newer beta tag built from
  the then-current reviewed `main` commit.
- The updater continues to resolve only a signed stable `latest.json`; a
  technical pre-release is never an update source.

## Rejected alternatives

- **Publish the unsigned candidate as GitHub Latest:** conflates download
  availability with supported native trust and risks future updater ambiguity.
- **Attach unsigned assets to the stable tag:** mixes independently governed
  trust levels in one immutable release inventory.
- **Keep only expiring Actions artifacts:** unnecessarily limits public beta
  participation and forces authenticated downloads.
- **Describe the Linux federation index as AirWiki for Linux:** gives users a
  server without the desktop product they expected.
- **Disable platform protections in installation guidance:** moves risk to the
  tester and invalidates the environment AirWiki needs to support.
