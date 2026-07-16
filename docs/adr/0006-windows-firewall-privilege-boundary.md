# ADR 0006: Isolate Windows firewall changes in a narrow elevated helper

- Status: Accepted
- Date: 2026-07-16

## Context

AirWiki LAN discovery and authenticated peer connections may require inbound
Windows Firewall rules. The desktop normally runs without administrator rights,
while changing those rules crosses an elevated operating-system boundary. A
general-purpose elevated command path, a shell invocation or a broad application
exception would expand the attack surface beyond the two permissions AirWiki
needs.

The rules must remain understandable and removable without granting access from
public networks, exposing the loopback MCP listener or changing Windows Network
Discovery globally. Cancellation, enterprise policy and conflicting existing
rules must not weaken local-only operation.

## Decision

Windows firewall mutation is isolated in the separate
`airwiki-windows-firewall-helper` executable. Its manifest requests
administrator rights, and its command line accepts exactly one argument:
`install` or `remove`. It accepts no caller-provided path, port, rule name or
other command.

The helper derives `airwiki.exe` as a sibling executable, canonicalizes both
paths and verifies through Windows code-signing APIs that the helper and desktop
have valid signatures from the same durable publisher. Missing, unsigned,
mismatched or unexpectedly laid-out binaries fail closed before firewall state
is changed.

The helper owns exactly two inbound allow rules for the sibling desktop:

- TCP for the sibling application, with all local ports because the
  authenticated LAN listener selects its port dynamically; and
- UDP local port 5353 for mDNS discovery.

Both rules are limited to Domain and Private profiles, remote `LocalSubnet`, the
exact application path and disabled edge traversal. They never enable the Public
profile, open the loopback MCP endpoint, grant Internet-wide access, enable file
sharing or change global Network Discovery.

Installation preflights conflicts and verifies the resulting rules. Removal
deletes only rules whose complete settings still match the managed definition.
UAC cancellation, unsupported policy, signature failure or conflicting rules
leave AirWiki in local-only mode and surface a recoverable diagnostic; they do
not trigger a broader fallback.

The unprivileged desktop may request only the fixed install or remove operation.
It does not inherit elevated firewall APIs or use the helper as a general command
executor.

## Consequences

- The elevated trusted computing base remains one small Windows-only executable.
- Desktop and helper must be packaged as signed siblings from the same publisher.
- A dynamic TCP listener requires a program-scoped rule rather than a fixed-port
  rule; transport authentication and collection authorization remain mandatory.
- Domain policy or a user declining UAC can disable LAN without preventing local
  ingestion, curation, wiki viewing or search.
- Rule changes require focused Windows tests for exact scope, idempotency,
  conflicts, rollback, removal and signature/layout failures.

## Rejected alternatives

- **PowerShell or `netsh`:** creates a shell-command boundary, complicates exact
  postcondition checks and makes argument expansion harder to constrain.
- **Broad application, Public-profile or Internet-wide rules:** grant more
  network reachability than same-subnet LAN collaboration requires.
- **Elevating the desktop process:** places UI, parsing, storage, inference and
  network orchestration inside an unnecessary administrator boundary.
- **Letting the desktop mutate firewall COM state directly:** mixes privileged
  platform policy with the composition root and makes elevation scope harder to
  audit.
- **Opening a fixed MCP or LAN port:** MCP must remain loopback-only, while the
  authenticated LAN listener intentionally selects its port dynamically.
