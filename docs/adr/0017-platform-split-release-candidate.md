# ADR 0017: Publish a platform-split macOS release candidate and Windows technical beta

- Status: Accepted
- Date: 2026-09-02
- Refines: ADRs 0012 and 0014 for the defined `v<version>-rc.<n>` platform-split channel; ADR 0016 remains in force

## Context

AirWiki can verify a Developer ID signed and notarized macOS package before the
Windows open-source signing route is available. Windows signing remains
conditional on the external SignPath Foundation gates in ADR 0016. Treating
either condition as a global stable release would misstate the status of the
other platform and would activate an updater before its stable acceptance gates
are complete.

The project needs a narrowly bounded public testing channel: a reviewer can
evaluate a notarized macOS package while Windows testers can evaluate the
current unsigned MSI installers. Provenance, hashes and the existing MSI smoke
remain useful in that channel, but none creates a Windows publisher identity or
substitutes for the stable acceptance checklist.

## Decision

The protected **Package platform release candidate** workflow publishes at most
one `v<version>-rc.<n>` GitHub prerelease from the exact current, green
40-character `main` SHA. It accepts these inputs:

- `commit_sha`: the full SHA at the current tip of `main`;
- `rc_number`: an integer from `1` through `9999`; and
- `publication_confirmation`: exactly `publish-v<version>-rc.<rc_number>`.

The release is always a GitHub prerelease, never GitHub Latest, and contains no
`latest.json`, updater assets or updater channel metadata. It has a closed,
hash-verified asset set, GitHub Artifact Attestations and bounded provenance.
Repository release immutability must be enabled before dispatch, and the
workflow must verify GitHub reports the published release as immutable. Those
records establish the origin of the released bytes, not a general support claim
or native publisher identity.

The set contains only these desktop platforms:

- macOS arm64: one Developer ID signed, notarized and stapled DMG, explicitly
  described as a macOS release candidate; and
- Windows x64: the `en-US` and `es-ES` per-user MSI installers, explicitly
  described as unsigned technical betas.

Linux is not part of this release candidate. The Windows MSIs must pass the
fresh-package install/uninstall smoke and retain their unsigned status. The
workflow cannot send them to the stable signing path and must not assert a
Windows publisher identity.

The macOS job reuses the existing verified notarization path. That path creates
and verifies an updater archive and signature transiently inside the protected
runner, then the RC job discards them before inter-job upload. Only the DMG and
its verification receipt cross that boundary; this temporary key use does not
activate or publish an updater channel.

Two separate protected approvals are required: `macos-signing` before any
Apple credential is used, and `public-release` immediately before creating and
publishing the prerelease. The latter approval does not waive the former and
neither approval authorizes stable promotion.

Stable release and updater eligibility continue to require every applicable
item in the public release checklist, including Windows native signing and the
full installed acceptance matrix. A successful macOS notarization rehearsal is
limited evidence that the credentialed path can execute; it does not complete a
stable gate or make an RC stable.

## Consequences

- Public availability may truthfully be platform-specific without calling the
  overall product stable.
- macOS testers receive Apple-native identity and notarization verification;
  Windows testers retain normal unsigned-software warnings and manual-download
  responsibilities.
- The immutable RC tag and release assets, plus the final inventory, permit
  independent checksum and attestation verification without making those files
  updater inputs.
- A later stable release must build and verify new final bytes through its own
  protected path; it cannot promote, relabel or reuse the unsigned Windows MSIs
  from this channel as a Latest release.

## Rejected alternatives

- **Calling the macOS package a global stable release:** Windows signing and
  stable acceptance remain incomplete.
- **Putting unsigned Windows MSIs in a Latest stable release:** attestations and
  hashes do not replace Windows native signing or updater verification.
- **Adding Linux to the RC:** no AirWiki Desktop Linux package is supported.
- **Making the RC an updater channel:** it would bypass the stable updater and
  recovery gates.
