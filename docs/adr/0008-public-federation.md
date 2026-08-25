# ADR 0008: Opt-in public federation

- Status: Accepted
- Date: 2026-07-21

## Context

LAN federation requires pairing and per-collection grants. Public discovery has
a different audience and privacy boundary, so widening `/airwiki/search/2.0.0`
would make LAN authorization ambiguous and couple discovery to content serving.

## Decision

AirWiki adds an experimental, disabled-by-default `internet_public` opt-in. A
human enables it once per collection after an explicit disclosure warning.
Documents still require review and publication; drafts, withdrawn revisions and
incoherent publications are never exposed.

Publishers use a stable public Ed25519 identity separate from LAN identity.
Readers use an ephemeral identity per application session. Signed, expiring
manifests contain metadata, a bounded lexical routing sketch, a publication
fingerprint and direct or relay routes. Federated indexes store only these
manifests and signed tombstones. They never receive documents, chunks,
embeddings, source paths or complete local indexes.

The public publisher identity is also the only device-like identity shown to a
public reader. Public protocols and UI do not expose the owner's LAN display
name, hostname or operating-system family. A publisher that is also paired on
LAN therefore appears as two deliberately separate identities: a named trusted
device in the private channel and a shortened public publisher identifier in
the public channel.

An accepted public search hit may include a bounded collection presentation:
a name of at most 240 characters, an optional description of at most 1,000,
up to 16 language tags of at most 16 characters each, an optional concept count
and OKF compatibility. The reader reconstructs these fields only from the
already verified signed manifest for that exact collection; owner response data
cannot add LAN identity, device labels, paths or unsigned profile metadata.
The field is optional and additive, preserving the existing protocol versions:
old clients ignore it and new clients accept responses that omit it.

Manifest expiry and its signed update-to-expiry interval are at most 24 hours;
an update timestamp may be at most five minutes ahead of receipt. Each index
admits at most 100,000 total manifest or tombstone rows and 1,000 rows per
publisher. Compact tombstone rows retain the sequence high-water mark for the
node lifetime, so a future-dated or newly replayed older manifest cannot revive
a withdrawn collection. Expiry removes the manifest payload and FTS row but
retains the same compact high-water mark. These rows count against both
admission ceilings.
The in-memory peer rate limiter tracks at most 1,024 active identities per
window and rejects new identities while that bounded window is full.

The wire protocols are separate. Catalog and search advertise v2 with v1
fallback. Browse advertises v4, then v3, v2 and v1. Version 4 adds the complete
published OKF workspace and fingerprint-bound page reads. Its bounded concept
and graph frames are drained automatically; pagination is an internal transport
detail. Version 3 adds an exact concept anchor for opening a search result,
while the reader translates that anchor to the existing UUID cursor contract
when an older publisher is negotiated.

- `/airwiki/public-catalog/1.0.0`
- `/airwiki/public-catalog/2.0.0`
- `/airwiki/public-search/1.0.0`
- `/airwiki/public-search/2.0.0`
- `/airwiki/public-browse/1.0.0`
- `/airwiki/public-browse/2.0.0`
- `/airwiki/public-browse/3.0.0`
- `/airwiki/public-browse/4.0.0`

Indexes select at most 64 collections. A reader contacts at most three indexes
and twelve publisher peers, with two collections per peer, bounded concurrency,
deadlines and cancellation. A cold owner connection has a separate bounded
budget from the owner response, and route classification is scoped to one
request and accepted only with a protocol-valid response on that owner's
observed connection. A multi-owner search reports Relay when any accepted
response used a relay and reports Direct only when every accepted response was
direct, so relay evidence is not hidden by another direct response. Publishers
revalidate public policy, reviewed publication, manifest sequence and
publication fingerprint under a disclosure lease immediately before returning
content. Disabling public exposure changes durable policy first, stops serving
content, and emits a signed tombstone.

QUIC/Noise is preferred. AutoNAT, Circuit Relay and DCUtR support publishers
behind NAT; relay connections are outbound and no Windows Public-profile
firewall rule is installed. A publisher includes a relay route only after the
matching outbound reservation produces a usable circuit-listen address.
Readiness changes advance the manifest sequence and immediately reannounce the
remaining routes or emit a tombstone when none remain. ADR 0005 and LAN grants
remain unchanged.

## Consequences

Public availability depends on the owner being online. Third parties may retain
search snippets, bounded signed presentation and any published OKF Wiki content
they opened before revocation.
Index entries may remain stale until expiry, but owners fail closed and stop
serving immediately. There is no
offline replication, remote editing, account system, DHT, gossip, global
reputation or automated moderation in v1.

## Rejected alternatives

- Extending the LAN protocol, because public reads have different authorization.
- Replicating chunks or indexes, because revocation and privacy would weaken.
- A canonical index or DHT, because v1 needs replaceable, pinned routing services.
