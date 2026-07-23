# Internet federation acceptance runbook

This runbook validates the experimental public network between two installed
AirWiki candidates on different NATs. Use synthetic documents only.

## Setup

1. Start at least one `airwiki-federation-index` on a reachable host and record
   its exact PeerId and TCP/QUIC multiaddresses. Pass each externally reachable
   address with `--external-address`; bind addresses and externally advertised
   addresses are intentionally separate. A wildcard bind such as `0.0.0.0`
   must never be advertised.
2. Add that pinned PeerId and address to both desktops under **Public network**.
3. On the publisher, create a synthetic collection, review and publish one
   document, then enable public exposure and accept the disclosure warning.
4. Confirm index logs contain only bounded counts, duration and error classes;
   they must not contain PeerIds, IPs, queries, snippets or routes.

### Azure validation relay

For temporary Azure acceptance, use one Ubuntu 24.04 x64 VM with a Standard
static public IPv4 and a managed OS disk. A direct VM address is sufficient for
this bounded validation workload; do not add a load balancer, public database,
container registry or general inbound rule.

Associate a Network Security Group that allows SSH only from the maintainer's
current public address and allows TCP/UDP `42042` from the Internet. All other
inbound traffic remains denied by the Standard public IP secure-by-default
policy. Install the release binary with
`packaging/federation-index/azure-install.sh`; the script creates an
unprivileged `airwiki` service, persists SQLite and identity under
`/var/lib/airwiki-federation`, and advertises the same static IPv4 used by both
listeners.

Put every temporary Azure resource in one dedicated resource group. After
acceptance, expire the bootstrap registry first and then delete the whole
resource group so the VM, disk, NIC, NSG and public IP cannot continue billing
independently.

### Temporary validation host

Use a dedicated Windows x64 validation PC on a network different from both
installed AirWiki candidates. Run three isolated index/relay processes from the
exact candidate commit, each with its own SQLite database and secret directory,
on TCP/UDP ports 42042, 42044 and 42046. Prefer its globally routable IPv6
address and scope inbound firewall rules to those six validation transports.
The AirWiki desktop firewall helper must remain unchanged and must not create a
Public-profile rule on either publisher or reader.

Record each generated PeerId locally and install it as a versioned bootstrap
entry with an absolute expiry. Never commit the validation host's private keys,
database, address, operator username or raw logs. An expired bootstrap is
ignored even if it remains in an older local database.

For an installed acceptance candidate, inject the temporary registry at build
time with `AIRWIKI_BOOTSTRAP_FEDERATION_INDEXES`. The value contains at most
three semicolon-separated entries, each encoded as
`version|expiry-rfc3339|peer-id|multiaddr`. Keep the value in the local
acceptance workspace only; do not place it in source control, shared CI output
or package logs. The desktop validates every pinned identity and multiaddress,
rejects duplicate identities and registry downgrades, and ignores entries after
their absolute expiry.

Obtain each identity explicitly, outside normal logs:

```powershell
airwiki-federation-index.exe C:\AirWikiFederation\index-1.db --print-peer-id
```

Start each process with its wildcard bind addresses and the corresponding
public addresses confirmed from another network. For example:

```powershell
airwiki-federation-index.exe C:\AirWikiFederation\index-1.db `
  --external-address /ip6/2001:db8::10/tcp/42042 `
  --external-address /ip6/2001:db8::10/udp/42042/quic-v1 `
  /ip6/::/tcp/42042 /ip6/::/udp/42042/quic-v1
```

The documentation-only `2001:db8::/32` address above must be replaced locally;
never commit or share the validation host address. If neither transport is
reachable from another network, the machine cannot act as a relay even when
its local listener and firewall rule are healthy.

After the validation window, stop all three processes, move their databases and
secret directories to the Recycle Bin, and remove the validation-host firewall
rules. Remove or expire the matching bootstrap entries before describing a
later candidate as usable without community indexes.

## Acceptance matrix

- Search and browse succeed without LAN pairing or grants.
- A private collection and a draft never appear.
- Editing a source withdraws its old revision until review and republication.
- Direct QUIC is preferred when reachable; relay fallback works across NAT.
- With an index offline, the reader reports offline or partial state and remains
  responsive.
- Disabling exposure immediately makes search and browse fail at the owner;
  after the tombstone reaches the index the catalog entry disappears.
- Restarting the publisher renews the signed manifest with a higher sequence.
- A stale fingerprint, replayed sequence, invalid signature and expired
  manifest are rejected.
- On Windows, no AirWiki firewall rule is enabled for the Public profile. Relay
  use remains an outbound connection.
- Blocking a publisher removes its results and prevents browse or a new
  connection until that identity is explicitly unblocked locally.
- The UI reports whether the successful content route was direct or relayed,
  plus accepted-index count and announcement expiry without exposing addresses.

Record OS/build versions, index identity, direct-versus-relay outcome, timings
for first partial and complete results, and sanitized failure classes. Do not
copy public identities, addresses, queries or snippets into shared logs.

## Performance gate

```bash
cargo run --release --locked -p airwiki-federation-index --bin federation-benchmark
```

The corpus represents 10,000 publishers and 100,000 collections. The command
fails if catalog query p95 is at least 1.5 seconds. Investigate regressions with
SQLite query plans before changing indexes or concurrency budgets.
