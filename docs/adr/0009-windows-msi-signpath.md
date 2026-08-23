# ADR 0009: Use per-user MSI and origin-verified open-source signing on Windows

- Status: Accepted
- Date: 2026-08-10

## Context

AirWiki's internal Windows candidate uses a hardened per-user NSIS installer.
The final NSIS build creates an AirWiki-owned uninstaller and invokes a signing
command while packaging. A public candidate must sign the desktop, MCP bridge,
firewall helper, generated uninstaller and final installer with one trusted
publisher. Updater signatures do not satisfy Windows native package trust.

Azure Artifact Signing public trust does not accept an individual identity in
Uruguay. Purchasing and operating a commercial certificate is disproportionate
for the current open-source validation stage. SignPath Foundation offers
origin-verified signing to eligible open-source projects, but its artifact
model does not deeply inspect or sign NSIS payloads. It does support deep
signing for MSI packages and their nested PE files.

The repository has no supported public Windows release. Existing NSIS packages
are internal candidates, and mutable AirWiki state already lives outside the
installation directory.

## Decision

Windows public candidates use one WiX MSI package built by Tauri on a
GitHub-hosted Windows runner and submitted to SignPath through its trusted
GitHub build-system connector. The request signs the MSI plus the AirWiki
desktop, MCP bridge and firewall helper contained in it. Pinned third-party
runtime files are verified and are not re-signed as AirWiki.

The MSI remains per-user and installs immutable files only below the fixed
`%LOCALAPPDATA%\Programs\AirWiki` directory. It does not accept an alternate
installation directory. SQLite, OKF, configuration, models and identities keep
their existing roots. The stable WiX UpgradeCode is committed and must not
change. A deterministic generated fragment gives every immutable file a stable
component identity and HKCU key path, and registers package directories for
empty-only removal. Downgrades fail closed.

Windows Installer owns transactional install, repair, upgrade and removal.
AirWiki keeps exact-match cleanup for its optional autostart value and delegates
firewall-rule removal only to the existing elevated, same-publisher helper.
User data and firewall cleanup remain opt-in and cancellation preserves them.
Unsafe or ambiguous installation state, reparse points and mismatched package
or nested signatures fail closed.

Build and signing remain separate. Normal CI produces only an unsigned internal
MSI. The SignPath job accepts only a reviewed GitHub-hosted build artifact with
verified origin, uses a protected environment, returns signed bytes, verifies
them independently and only then creates the separate Tauri updater signature.
No SignPath token, updater private key or certificate material is versioned.

The existing NSIS path remains available only while MSI upgrade, uninstall and
installed acceptance are incomplete. After those gates pass, NSIS-specific
scripts, templates, tools and documentation are removed rather than retained as
a second supported package path.

## Consequences

- AirWiki can obtain Windows public trust without a project-owned certificate,
  subject to SignPath Foundation approval and policy.
- Signing becomes reproducibly tied to a reviewed repository commit and
  GitHub-hosted build.
- MSI/WiX becomes a security-sensitive packaging dependency and its generated
  tables, custom actions, payload and installed state require explicit tests.
- Existing internal NSIS testers may need to uninstall the old candidate while
  preserving data before the first MSI install. This is not a public migration
  because no supported Windows release exists.
- Release automation cannot proceed when SignPath is unavailable or rejects
  origin, policy, artifact structure or approval.

## Rejected alternatives

- **Keep NSIS and sign only its outer executable:** leaves the generated
  uninstaller unsigned and fails the native trust boundary.
- **Purchase a commercial individual certificate:** technically valid but
  disproportionate recurring cost for the current open-source stage.
- **Microsoft Store MSIX:** free signing is attractive, but Tauri does not
  currently produce the required package directly and package identity would
  change updater, integration, autostart and helper behavior more broadly.
- **Self-signed or private-trust certificate:** does not satisfy Smart App
  Control for ordinary users.
- **Disable Smart App Control on the acceptance host:** weakens the target
  environment and cannot prove the user-visible installation path.
