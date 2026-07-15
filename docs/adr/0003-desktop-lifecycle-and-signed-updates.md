# ADR 0003: Keep desktop services available and require signed updates

- Status: Accepted
- Date: 2026-07-12
- Supersedes: ADR 0002 (desktop lifecycle and per-user autostart only)

## Context

Watchers, local MCP, wiki maintenance and LAN discovery work only while the
process is running. A non-technical user should not accidentally disable those
capabilities by closing the window after configuring them. A daemon or system
service would add privilege, operational complexity and attack surface without
a verified need.

A future public distribution also needs verifiable corrections without silent
updates or a persistent installation identifier. These choices are costly to
reverse because they affect process lifecycle, OS integration and package trust.

## Decision

AirWiki runs one instance per user session. A second normal launch sends
only `SHOW` through a bounded local activation channel and exits. The channel
accepts no paths, queries or business parameters.

On the tested development targets, macOS arm64 and Windows x64, closing the
window may hide it in the tray when the tray is operational. The tray exposes
status, open and **Exit completely**. Complete exit performs the normal bounded
shutdown and stops watchers, LAN, MCP and inference. If the tray cannot be
created, the application must not hide an inaccessible process.

Autostart is per-user, optional and enabled only after explicit onboarding
consent:

- macOS uses `SMAppService` and a LaunchAgent contained in the bundle;
- Windows uses one exact entry under
  `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run`.

The operating system is authoritative for autostart state. Conflicts are not
overwritten. AirWiki uses no `sudo`, daemon, system service, scheduled
task, manual LaunchAgent write or machine-wide registry key.

Public updates use one stable channel. The client embeds only the updater public
key; the private key remains outside the repository. Checks send no installation
identifier, document, query or PeerId. Download and installation require human
confirmation and reject downgrades and invalid signatures. Offline failure does
not block normal use. The stable manifest is published only after every artifact
has passed its release gates.

Native package trust remains an independent requirement:

- macOS uses Developer ID, Hardened Runtime, inside-out signing, notarization
  and stapling;
- Windows uses a public-trust signing service for the desktop, bridge, helper
  and final installer, followed by Authenticode verification.

Updater signatures do not replace native signatures. No signing credential is
versioned.

## Consequences

- AirWiki may continue working while hidden, but only in the logged-in
  user's session.
- Autostart and update checks are opt-in and reversible.
- The process cannot survive logout or run before login.
- Tray loss and shell restarts are explicit recovery scenarios.
- Release work must separate building, signing, verification, prerelease
  publication and stable-manifest promotion.
- Until real signing credentials and all release gates exist, packages are
  internal candidates rather than supported public releases.

## Rejected alternatives

- **System service or daemon:** unnecessary privilege and operational cost for
  the workstation use case.
- **Silent updates:** remove consent and complicate recovery.
- **Updater signature only:** does not satisfy native package trust or the
  elevated Windows helper boundary.
- **Autostart without onboarding confirmation:** changes OS state without
  informed consent.
