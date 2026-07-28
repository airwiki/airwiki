# Public federation beta v1 infrastructure

This runbook operates the experimental AirWiki public federation beta on two
independent Azure virtual machines. It is an internal beta service without an
availability SLA. It does not make AirWiki a supported public release.

The user-visible outcome is that a candidate with the bundled beta registry can
discover, search and browse owner-opted-in public wikis without asking the user
to configure a tunnel, router, firewall Public profile or community index.
Direct DCUtR remains conditional on both NATs; the outbound relay path is the
required fallback.

## Fixed scope

Each node has its own dedicated resource group, region, virtual network,
network-security group, Standard static IPv4 address, B1s Linux VM, E4 Standard
SSD OS disk, SQLite database and Ed25519 identity. The two nodes share no disk,
database, identity, process, availability zone or resource group.

The reviewed deployment uses East US and West US 2. The regions are far enough
apart to avoid a single regional failure while retaining the same low-cost VM
class. Each host runs exactly one index/relay process as the unprivileged
`airwiki` user. The service accepts only the existing public TCP and QUIC
transports. SSH accepts only one maintainer IPv4 `/32`; password authentication
is disabled. All other inbound traffic is denied by the Standard public-IP and
network-security-group defaults.

The template pins Ubuntu 24.04 x64 image version `24.04.202607140`, enables
Trusted Launch, Secure Boot and vTPM, and keeps boot diagnostics disabled.
Systemd limits capabilities, namespaces, address families, file writes, memory,
tasks and file descriptors. The binary is staged, architecture-checked,
hash-checked and preflighted before the prior service is stopped.

Deliberately absent: containers, Kubernetes, a load balancer, a registry, a
shared database, DHT/gossip, remote configuration, an administrative panel,
user telemetry and a new protocol.

## Cost gate

Run the live retail-price calculation before every deployment:

```bash
packaging/federation-index/azure-beta-cost.sh --check
```

The checked estimate on 2026-07-25 is:

| Component | Monthly USD |
| --- | ---: |
| Two B1s Linux VMs at 730 hours | 15.18 |
| Two E4 Standard SSD OS disks | 4.80 |
| Two Standard static IPv4 addresses at 730 hours | 7.30 |
| 250 GB total Internet egress, after the first 100 GB | 13.05 |
| Disk-operation allowance | 0.40 |
| Two VM-availability-alert allowance | 0.20 |
| Planned total | 40.93 |
| Conservative total if the account free tier is already consumed | 49.63 |
| Configured budget ceiling | 50.00 |

The script reads the current
[Azure Retail Prices API](https://learn.microsoft.com/rest/api/cost-management/retail-prices/azure-retail-prices)
instead of trusting this snapshot. Azure includes the
[first 100 GB of monthly Internet egress](https://azure.microsoft.com/pricing/details/bandwidth/)
and currently prices the next tier per GB. Contract pricing, taxes and currency
conversion may differ.

The ceiling is a control target, not an Azure hard spending cap. Deployment
creates one USD 25 monthly Cost Management budget per node before creating the
billable resources, with actual alerts at 50%, 75%, 90% and 100%, plus a 100%
forecast alert. At 75%, investigate network consumption; at 90%, stop beta
admission and prepare retirement; at 100%, revoke the bootstrap and delete both
dedicated groups. Budget notifications can lag, so the operator also checks
month-to-date spend during routine status review. Deployment verifies that
Azure returns `USD` as the budget and current-spend unit and rolls back before
creating a VM if the billing scope uses another currency.

No billable resource may be created until a human explicitly approves the
USD 50 monthly ceiling and the exact clean candidate commit. The deployment
script enforces both values:

```bash
export AIRWIKI_BETA_SUBSCRIPTION_ID="<approved active subscription id>"
export AIRWIKI_BETA_COST_APPROVED_USD=50.00
export AIRWIKI_BETA_APPROVAL_SHA="$(git rev-parse HEAD)"
export AIRWIKI_BETA_BUDGET_EMAIL="<operator email>"
export AIRWIKI_BETA_MAINTAINER_CIDR="<current public IPv4>/32"
export AIRWIKI_BETA_SSH_PUBLIC_KEY_FILE="<operator Ed25519 public key>"
packaging/federation-index/azure-beta.sh deploy
```

The values containing operator or network data stay in the local shell. Never
copy them into Git, a pull request, CI logs or shared acceptance evidence. The
script refuses a dirty tree, an existing beta resource group, a private or
non-canonical SSH source, a non-Ed25519 key, a different commit or a different
cost or subscription approval. Every Azure command requires the active
subscription to equal the locally supplied approved identifier. A failed first
deployment deletes only the newly created dedicated groups and confirms their
absence; if Azure cannot confirm rollback, the script reports that a billable
group may remain instead of claiming success.

## Install the index candidate

Build the Linux x64 index from the same commit selected for the desktop
candidates. The manual **Package unsigned pilot** workflow produces a
hash-verifiable Linux artifact without using bootstrap data or release
credentials. Make the downloaded binary executable and verify that its workflow
commit equals the approved commit.

Install the same exact bytes on both VMs:

```bash
export AIRWIKI_BETA_SSH_PRIVATE_KEY_FILE="<matching operator private key>"
packaging/federation-index/azure-beta.sh install \
  "<x86-64 Linux airwiki-federation-index candidate>"
```

The installer obtains each SSH host-key fingerprint through the Azure control
plane, compares it with the network key before transfer, uploads the candidate
over SSH, and requires the same SHA-256 on both nodes. It never fetches a binary
or configuration from a remote application endpoint. Before mutation, the
installer snapshots the prior binary, unit files, enablement and active state.
If the new service does not become active, it restores that known prior install
and emits an explicit incomplete-rollback class if restoration itself fails.

## Versioned private bootstrap

After both services are active, create the private build input:

```bash
packaging/federation-index/azure-beta.sh bootstrap \
  <positive-registry-version> <absolute-UTC-expiry>
```

The expiry must be 30–120 days away. The script reads the two persistent
identities and static endpoints without printing them, rejects a shared
identity, and writes a mode-`0600` registry below the ignored `target/private`
directory. Its output contains only the version, expiry and registry SHA-256.
The registry itself must never enter source control, CI, a pull request or
shared evidence.

Build both candidates from the same clean commit and that same private file:

```bash
export AIRWIKI_BETA_CANDIDATE_SHA="$(git rev-parse HEAD)"
export AIRWIKI_SIGNING_PURPOSE=development
export AIRWIKI_SIGNING_IDENTITY="<stable Apple Development identity>"
packaging/package-beta-macos.sh \
  target/private/federation-beta-v1.bootstrap
```

Installed macOS beta candidates use one stable Apple Development identity so
the operating-system keychain can recognize successive builds without granting
each ad-hoc binary access to the persistent device identities. This is internal
development signing only; it does not make the candidate a supported public
release and does not replace Developer ID signing, notarization or stapling.
Omit both signing variables only for isolated source/package checks that never
reuse persistent keychain state.

On the Windows x64 builder:

```powershell
$env:AIRWIKI_BETA_CANDIDATE_SHA = (git rev-parse HEAD).Trim()
.\packaging\package-beta-windows.ps1 `
  -BootstrapFile .\target\private\federation-beta-v1.bootstrap
```

Record the commit, package SHA-256 values and bootstrap SHA-256. Matching hashes
prove that both packages used the same private registry without disclosing it.
The desktop still performs its existing startup validation: common positive
version, pinned unique identities, valid addresses, absolute expiry, no
downgrade, no same-version mutation, and atomic replacement of older bootstrap
entries. Community indexes remain user-owned and are not removed.
Schema version 6 persists the highest accepted bootstrap registry version
separately from its effective rows, so a fully expired higher registry can
retire every older bootstrap entry without reopening the downgrade path.

There is no remote bootstrap download. Rotation requires a new candidate with a
higher registry version.

## Operations and sanitized observability

Run:

```bash
packaging/federation-index/azure-beta.sh status
```

For each node it emits only:

- VM power state and Azure availability metric;
- month-to-date budget spend;
- systemd service state, restart count and current memory;
- counts grouped by fixed, sanitized error class for the last 24 hours;
- 15-minute relay outcome counts grouped by a fixed `relay_class` allowlist.

It never prints an identity, address, route, request, snippet or raw log.
The index emits those fixed classes at startup, identity, worker, persistence,
verification, capacity, query, server, relay lifecycle and shutdown boundaries.
Relay lifecycle output is aggregated, omits normal reader circuit
acceptance/closure, has no per-event timestamp and flushes once during graceful
shutdown. Circuit I/O failures expose only a fixed operating-system error-kind
bucket, never the underlying error text or endpoint.
Azure's platform availability metric and action-group email require no guest
telemetry agent or Log Analytics workspace. CPU, disk and network consumption
remain available as standard Azure platform metrics for incident diagnosis.
Do not enable guest log shipping for this beta.

Routine checks:

- daily during the first beta week, then weekly: both services active, no
  unexpected restart increase, availability healthy and spend below 75%;
- weekly: public probe from a network outside Azure, once per node;
- before every candidate: live cost check, bootstrap expiry and version;
- 30 days before expiry: start registry renewal;
- after any binary update: exact hash on both nodes, service restart and
  per-node public probe.

## Failure and recovery

For the installed failover gate, deallocate only one node:

```bash
packaging/federation-index/azure-beta.sh stop-node east
packaging/federation-index/azure-beta.sh start-node east
```

Repeat with `west`. A candidate must stay responsive through the other node,
report partial/offline state accurately, and recover after the stopped node
returns. Deallocation preserves the disk and identity but the disk and static
address continue to cost money.

Recovery order:

1. If the process failed, inspect only sanitized error-class counts and restart
   the existing unit. Its database and identity remain on the managed disk.
2. If the VM failed, restart it and confirm the same identity implicitly by a
   successful pinned probe. Do not publish the identity in evidence.
3. If a disk or identity is lost, use the single-node replacement sequence
   below and issue a higher bootstrap registry. Do not pretend to recover the
   old identity. Publishers repopulate the routing-only SQLite catalog.
4. If one region is unavailable, operate on the other node until the failed
   region recovers or is replaced. Never point both registry entries at one
   host.

Offline replicas and a shared backup database are out of scope. This is safe
because the indexes contain routing metadata only; the publisher remains the
content authority.

## Identity rotation, expiry and revocation

Normal rotation or disk-loss recovery replaces one whole node at a time while
the other remains active:

```bash
export AIRWIKI_BETA_REPLACE_CONFIRM=replace-east-beta-node
packaging/federation-index/azure-beta.sh replace-node east
packaging/federation-index/azure-beta.sh install \
  "<x86-64 Linux airwiki-federation-index candidate>" east
packaging/federation-index/azure-beta.sh bootstrap \
  <higher-version> <absolute-UTC-expiry>
```

Repeat with `west` and the matching confirmation when required. The replacement
command accepts an already absent target, so a failed first attempt is
recoverable. It requires the other node to remain a tagged beta node, destroys
only the selected old resource group after the retained VM and service are
confirmed active, provisions its replacement under the same approved ceiling
and subscription, and removes a partially created replacement if provisioning
fails. Installing only the selected side creates its new persistent identity.
Build the higher registry only after both services are active, then prove new
candidates reach both and older candidates still reach the unchanged node.

For suspected compromise, generate a higher registry that omits the affected
identity, distribute that candidate, stop the affected service, and replace or
delete its resource group:

```bash
packaging/federation-index/azure-beta.sh revoke-bootstrap \
  <higher-version> <absolute-UTC-expiry> <east|west>
```

The revocation command contacts only the node that remains in the registry, so
the affected node may already be offline. A pinned identity mismatch must fail
closed.

An expired entry is ignored even if an old local database retains it. Generate
an intentionally expired higher-version test registry with:

```bash
packaging/federation-index/azure-beta.sh expired-bootstrap <higher-version>
```

At expiry,
an unrenewed two-entry registry therefore becomes unavailable rather than
silently trusting a replacement. Expiry testing uses a deliberately expired
higher-version internal candidate and must confirm that neither entry is
dialed. Revocation testing uses a higher version that omits one identity and
must confirm failover through the remaining node.

## Incident response and retirement

For availability incidents, preserve only timestamps, commit/package hashes,
PASS/FAIL, durations and sanitized error classes. Raw journals, identities,
addresses, routes and application traffic remain local.

For privacy or integrity incidents:

1. Disable public exposure at affected owners; serving stops immediately.
2. Revoke a compromised index identity with a higher registry.
3. Stop the affected node and preserve its disk only while local diagnosis is
   required.
4. Rotate the identity or replace the node.
5. Re-run stale fingerprint, replay, invalid signature, expiry and tombstone
   gates before returning it to beta.

Retire the bootstrap in a higher-version candidate before deleting Azure:

```bash
export AIRWIKI_BETA_RETIRE_CONFIRM=delete-airwiki-federation-beta-v1
export AIRWIKI_BETA_BOOTSTRAP_RETIRED_VERSION=<higher-version-already-tested>
packaging/federation-index/azure-beta.sh retire
```

This deletes exactly the two dedicated resource groups, including VM, disk,
network, static address, alert and resource-group budget. Confirm both groups
are absent and no beta-tagged billable resource remains. Do not merge a pull
request while an unreported billable deployment or failed rollback is pending.
The command validates every entry in the private retirement registry as the
matching higher version and already expired, validates both target groups before
deleting either, and confirms that both groups are absent before reporting
success.

## Installed acceptance

Use synthetic public collections only and run the complete matrix in
[the Internet federation acceptance runbook](internet-federation-runbook.md).
The beta-specific closure requires:

- installed macOS arm64 and Windows x64 packages from one commit and registry;
- search and browse in both directions without pairing or grants;
- outbound relay success across the two real NATs;
- one-node failure, failover and recovery in each direction;
- expired and revoked higher-version registry failures;
- no AirWiki Windows Public-profile rule before or after the tests;
- repository gates, release benchmark, independent security review, green CI
  and DCO.

Direct DCUtR is recorded separately and remains conditional. Public signing,
notarization and updater promotion remain deferred.
