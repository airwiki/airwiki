# ADR 0013: Let connected applications search accessible federated knowledge

- Status: Accepted
- Date: 2026-08-29
- Refines: ADR 0002, ADR 0005 and ADR 0008

## Context

AirWiki previously treated a connected application's MCP search as a strictly
local operation. That forced a user to repeat a discovery task in the desktop
Library even when a verified LAN device or a public owner had already decided
that its reviewed knowledge was available. Adding a second matrix of
application permissions for every remote Wiki or every result would make
access difficult to explain without giving the reader meaningful authority:
the remote owner is still the only party able to disclose that knowledge.

Sharing and connecting an application are different decisions. Sharing
controls which of this device's Wikis may leave it. Connecting an application
controls which knowledge that application may ask AirWiki to find. Public
search adds a separate risk because the application's query may leave this
device even though no local Wiki is published.

## Decision

AirWiki derives a process-local `SearchDisclosureScope` at each trusted
boundary. `LocalUser`, `LocalApplication`, `AuthorizedPeer` and `Public` are
never serialized. A Noise-authenticated LAN handler replaces any caller state
with `AuthorizedPeer`; the public publisher uses `Public`; and the capability-
authenticated MCP gateway uses `LocalApplication`. `SearchPurpose` continues
to describe retrieval behavior and does not grant disclosure authority.

A connected application's search uses one accessible-knowledge coordinator:

- the local branch searches only collections granted to that application;
- the LAN branch searches verified peers concurrently without receiving the
  application's local collection scope; and
- the public branch runs only when that exact active application has enabled
  **Search public knowledge**.

The public preference starts disabled, is confirmed by a native dialog because
it permits query egress, and is reset by capability rotation or revocation.
Disabling it is immediate. It never changes any Wiki's LAN or Internet policy.
The desktop's interactive public-search choice remains a separate per-query
decision.

At a LAN publisher, Noise identity, durable trust, a per-Wiki grant, LAN
exposure and the current reviewed publication authorize disclosure. The
publisher's `allow_external_ai` flag applies only to applications connected on
that publisher; it does not veto an already authorized receiver device. New or
regranted LAN grants record owner consent that the receiver and its connected
applications may search the shared Wiki. Pre-existing grants remain usable by
AirWiki itself but do not answer receiver-AI searches until the owner accepts
one non-blocking, all-active-grants migration. Ignoring the notice preserves
the earlier behavior.

At a public publisher, the signed, current public policy authorizes disclosure.
`allow_external_ai` does not apply. Readers accept public evidence only after
manifest signature, fingerprint, sequence, expiry, publisher-block and final
publication checks. LAN response identity is replaced with the authenticated
PeerId; public presentation and identity come only from the verified manifest.

Enabled branches use the existing bounds, deadlines and reciprocal-rank
fusion. One failed branch produces results from the others with an explicit
coverage gap. Failure of every enabled branch returns a fixed typed
unavailable error. Queries, snippets, paths, addresses, PeerIds and content are
not logged.

The MCP gateway reauthorizes the application after federation. Revocation
discards the complete response. A public preference turned off during a public
request also discards the complete response and requires a retry. Changed
local grants remove only newly unauthorized local hits; remote hits remain
governed by their authenticated source.

## Consequences

Connected applications can discover approved knowledge on this device,
verified LAN devices and—after per-application consent—the public network
without remote-Wiki or per-result permission prompts. Owners retain one clear
outbound policy: local application access, LAN sharing and Internet publication
remain independent.

During mixed-version rollout, a new reader does not bypass an older
publisher's policy. An older publisher can therefore omit receiver-AI LAN
results until it is upgraded or its earlier external-AI policy already permits
them. This temporary false negative is safer than silently broadening a grant.

Public query text can be observed by configured indexes, selected publishers
and their network providers. The native confirmation and per-application
switch make that egress explicit; result content remains untrusted evidence.

## Rejected alternatives

- **Permissions for each remote Wiki or result:** duplicates source authority,
  creates stale local policy and adds friction without preventing an owner from
  disclosing what it already shared.
- **Reuse `allow_external_ai` for every channel:** conflates a publisher's local
  applications with a verified remote device and public publication.
- **Enable public search for every connected application:** silently sends
  queries outside the device and violates the default local-first boundary.
- **Serialize disclosure scope or local grants:** lets an untrusted caller claim
  authority and leaks the local application policy to remote transports.
- **Silently upgrade old LAN grants:** expands a previous human decision without
  consent.
