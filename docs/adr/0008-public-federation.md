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

The wire protocols are separate:

- `/airwiki/public-catalog/1.0.0`
- `/airwiki/public-search/1.0.0`
- `/airwiki/public-browse/1.0.0`

Indexes select at most 64 collections. A reader contacts at most three indexes
and twelve publisher peers, with two collections per peer, bounded concurrency,
deadlines and cancellation. Publishers revalidate public policy, reviewed
publication, manifest sequence and publication fingerprint under a disclosure
lease immediately before returning content. Disabling public exposure changes
durable policy first, stops serving content, and emits a signed tombstone.

QUIC/Noise is preferred. AutoNAT, Circuit Relay and DCUtR support publishers
behind NAT; relay connections are outbound and no Windows Public-profile
firewall rule is installed. ADR 0005 and LAN grants remain unchanged.

## Consequences

Public availability depends on the owner being online. Third parties may retain
metadata and returned snippets after revocation. Index entries may remain stale
until expiry, but owners fail closed and stop serving immediately. There is no
offline replication, remote editing, account system, DHT, gossip, global
reputation or automated moderation in v1.

## Rejected alternatives

- Extending the LAN protocol, because public reads have different authorization.
- Replicating chunks or indexes, because revocation and privacy would weaken.
- A canonical index or DHT, because v1 needs replaceable, pinned routing services.
