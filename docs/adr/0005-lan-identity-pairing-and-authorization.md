# ADR 0005: Bind LAN authorization to persistent identity and human-confirmed SAS

- Status: Accepted
- Date: 2026-07-15
- Answerability clause superseded by: [ADR 0007](0007-separate-evidence-from-authorized-candidates.md)

## Context

mDNS discovery shows that a peer is reachable on the local network; it does not
establish that the device is the one the user intends to trust. IP addresses,
ports and display names can change or be impersonated. At the same time, a
device-level trust decision must not automatically expose every collection or
authorize content for an external AI client.

Pairing, grants and revocation are security boundaries that must survive normal
restarts, address changes and transient network failures without broadening
access.

## Decision

Each installation generates one persistent Ed25519 libp2p identity. Production
stores the private key in the operating-system credential store, never in
SQLite or logs. Its public key derives the stable PeerId. Noise authenticates
the key used by each transport connection. mDNS and manual addresses locate a
PeerId before connection. After the dynamic listener port is known, the desktop
registers a bounded set of its active, OS-inspected private or on-link IPv4
endpoints across non-point-to-point interfaces; the wildcard listener is never
announced. This covers Wi-Fi and Ethernet concurrently without treating a VPN
default route as the only LAN. AirWiki exchanges those endpoints through
`/airwiki/lan-address-exchange/1.0.0` only after the Noise-authenticated peer is
already durably trusted. The exchange is repeated when concrete endpoints change
and whenever a trusted search connection is established, so a result remains
openable after that short-lived connection closes. Unknown and merely paired-in-
progress peers receive an empty response. An announced address never creates
trust or a grant. A receiver accepts only listener addresses whose IP equals the
remote IP observed on that exact Noise-authenticated connection, preventing a
peer from steering probes toward another private host. Each runtime session adds
a monotonic revision, and receivers reject an older revision or a retired
session arriving on an older connection, so delayed exchanges cannot restore a
withdrawn endpoint. A browse retry reuses an existing authenticated connection
immediately; only a disconnected peer enters the redial queue. Every redial must
still complete Noise authentication for the expected PeerId.

The same authenticated exchange may carry a bounded display name and operating-
system family so the owner can distinguish trusted devices in search results and
per-Wiki grants. Receivers persist this presentation metadata only for an
already trusted, unblocked PeerId. A name or platform never creates trust,
restores a revoked grant or participates in authorization. Unknown and pairing-
in-progress peers receive neither endpoints nor presentation metadata. Public
federation never carries this LAN device metadata.

New trust requires an explicit pairing session over Noise. Both devices exchange
fresh 32-byte nonces and derive the same six-word short authentication string
from the ordered PeerIds and their bound nonces. The human must compare and
confirm the words on both devices within two minutes. Durable trust is written
only after both confirmations. Expired, mismatched, cancelled or interrupted
sessions grant nothing.

Trust is per PeerId, while access is a separate, source-node-owned grant per
collection. A LAN search may expose a collection only when all of these remain
true immediately before handoff:

- the caller's Noise-authenticated PeerId is durably trusted and not blocked;
- a grant exists for that PeerId and collection;
- the collection is `peer_shareable`;
- for an `external_ai` request, the collection is also
  `allow_external_ai`;
- the requested revision remains published and passed the answerability gate.

After an authorized search identifies a specific Wiki, the reader may open its
complete published OKF workspace over `/airwiki/shared-wiki-browse/2.0.0`.
AirWiki retrieves bounded concept and graph frames automatically and individual
published pages by fingerprint; there is no user-visible pagination and no
silent preview truncation. The workspace includes the published hierarchy,
root `index.md`, root `log.md`, stable concept pages, metadata, backlinks and
internal graph edges. Version 1 remains a summary-only fallback for older peers.

The source revalidates the same trusted identity, exact per-Wiki grant,
`peer_shareable` policy, OKF compatibility and publication state under a
disclosure lease immediately before every handoff. This is not a catalog API:
it accepts one explicit Wiki ID and returns neither other Wiki names, original
source files or paths, chunks, embeddings nor the operational search index.

Runtime access is only a cache of durable state. Authorization intersects the
runtime snapshot with SQLite and fails closed on missing, stale, malformed or
conflicting state. Discovery, a connection or a client-provided label never
creates trust or a grant.

Revocation first blocks the runtime access snapshot, retires in-flight searches,
cancels pairing state and closes active connections. A SQLite transaction then
deletes the peer's grants and records it as untrusted and blocked. If persistence
is unhealthy, runtime access remains narrowed rather than reopening the
connection. Rediscovery does not restore access. Re-pairing a blocked identity
is an explicit human action; any failed, cancelled, expired or interrupted
attempt restores the block. Successful re-pairing restores device trust but not
previous collection grants.

## Consequences

- Trust survives address and port changes but remains bound to the device key.
- Trusted-device names and operating-system labels improve recognition but
  remain mutable presentation metadata, never authorization identifiers.
- Losing or replacing the private key creates a new PeerId and requires new
  pairing and grants.
- Users must perform a two-device SAS comparison and grant collections
  separately.
- Revocation takes effect without waiting for mDNS expiry or a process restart.
- Authorized LAN results can reuse the local file-oriented Wiki workspace in a
  bounded read-only mode without replicating source documents.
- Either endpoint can reopen an authorized result after an idle connection
  closes, including the endpoint that originally accepted the connection.
- Keychain or credential-store failure disables the trusted LAN identity rather
  than falling back to an unprotected production key.

## Rejected alternatives

- **Trust by IP address, hostname or display name:** identifiers are mutable and
  unauthenticated.
- **Trust every device discovered by mDNS:** discovery is presence, not consent.
- **Noise alone without human SAS:** encrypts and authenticates a presented key
  but does not prove that it belongs to the intended nearby device.
- **One device-level all-collections grant:** violates least privilege and makes
  new collections implicitly visible.
- **Restore grants after re-pairing:** revocation would not be durable and could
  silently re-expose knowledge.
