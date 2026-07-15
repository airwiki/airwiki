# July 2026 development summary

This document records the durable outcome of the internal planning work that
preceded the first Git baseline. The original agent-specific plans and open
checklists were retired because they described obsolete intermediate states and
were not suitable as contributor instructions.

## Implemented foundations

- The desktop, MCP bridge, and Windows firewall helper are separate executable
  boundaries.
- Windows firewall management is limited to two application-owned inbound rules
  for private or domain networks and the local subnet.
- LAN discovery, authenticated pairing, six-word SAS confirmation, collection
  grants, revocation, and fail-closed search are implemented.
- Windows packaging can produce an internal installer containing the desktop,
  bridge, helper, pinned local runtime, licenses, and build metadata.
- macOS packaging can produce an internal arm64 application and disk image.
- Installer, updater, and elevated-helper work stays behind background worker
  boundaries so it cannot block egui or an asynchronous runtime worker.

## Validated internal outcomes

- Internal macOS and Windows candidates completed the synthetic local knowledge
  workflow and discovered each other on a private LAN.
- The Windows candidate loaded its packaged local model without a repeated
  verification loop and returned to idle after inference.
- The firewall helper produced the exact narrow rule set and remained idempotent.
- Pairing recovery was validated after stale device and identity state was
  removed explicitly by the user.

These observations are development evidence, not public-release certification.
Current acceptance procedures live in the [two-node runbook](../two-node-runbook.md)
and current architecture decisions live in the [ADR index](../adr/README.md).

## Deliberately deferred

- repository governance and public contribution channels;
- Developer ID signing, notarization, Windows public-trust signing, and SBOM
  publication;
- updater key custody and stable-manifest promotion;
- public signed-release automation;
- web, mobile, tunnels, services, and silent updates.

The preserved experimental signed-release workflow sources are stored as inert
files under [the workflow archive](../archive/release-workflows/README.md). They
must not be treated as executable plans or current release policy.
