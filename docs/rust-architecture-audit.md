# Rust architecture audit

Status: Completed — findings corrected and installed-platform acceptance passed.

## Scope and acceptance

Review the desktop Rust composition layer and the public federation index for
correctness, privacy, recoverability, concurrency and measured resource use.
Keep visual changes separate. Preserve the existing domain ownership, human
publication authority, signed public metadata, protocol limits and local data.

Acceptance requires reproducible findings, focused corrections, regression and
recovery tests, current architecture documentation, proportional repository
checks and the applicable installed-platform checks. Code review and CI remain
required before integration through the normal pull-request process.

## Findings

### Index blocking-pool contention

`CatalogBackend` submitted every operation to the Tokio blocking pool before
`CatalogStore` acquired its single SQLite connection mutex. The network server
already bounds active requests to 64, but those requests could consume separate
blocking threads while competing for the same connection. Cancelling an async
caller does not cancel a blocking job that has started.

The backend now shares one asynchronous admission permit across its clones.
Every register, withdrawal and query passes through the same execution helper.
The blocking closure owns the permit until it finishes, including after its
caller is cancelled. Cancelled waiters never submit their operation. Existing
network admission, persistence transactions and error mapping remain in force.

Evidence:

- `cancelled_catalog_caller_does_not_overlap_blocking_database_jobs` failed
  before admission was added and passes with it. A controlled two-thread
  blocking pool observes overlapping operations in the former implementation
  and one active operation in the corrected implementation.
- `cancelled_catalog_waiter_never_runs_and_later_requests_recover` covers a
  cancelled waiter and admission recovery after a database operation error.
- The federation-index package tests include signed registration, expiry,
  tombstone high-water marks, versioned transactions and a loopback round trip.

This establishes a bound on blocking work per backend. It does not establish a
throughput or latency improvement for a deployed index.

### Desktop shutdown and detached blocking jobs

The desktop worker aborted and joined its asynchronous jobs before attempting
`Arc::try_unwrap` on the service graph. Some jobs had already started nested
`spawn_blocking` operations that retained that graph. Joining the cancelled
async parent did not join those operations; shutdown could therefore skip
`DesktopServices::shutdown` because the graph still had another owner.

Service-owning nested blocking jobs now use the graph's task tracker. Shutdown
joins the async jobs, then waits for their tracked blocking work, before
consuming the service graph. The existing outer application exit deadline
remains the bound on process shutdown.

`shutdown_waits_for_blocking_service_jobs_after_async_callers_exit` reproduced
the premature completion before that wait was added. It uses a synthetic shared
resource, an explicitly started blocking operation and a cancelled async parent;
shutdown must remain pending until the operation releases the resource, after
which exclusive ownership must be recoverable. The corrected desktop package
tests pass. The installed macOS and Windows lifecycle checks below also pass.

### Catalog language selection

Both catalog operations applied their SQL limit before decoding and filtering
the declared manifest languages. With a one-result request, two higher-ranked
entries in another language hid a third matching entry. The search and browse
regressions both failed with an empty result before the correction.

Language-filtered requests use an indexed SQL projection before ranking and
limiting. They load payloads individually, with at most twice the requested
candidate count selected by SQL, and recheck their declared languages. The
result cap, protocol preference, signatures and withdrawal semantics remain.

Independent review identified that scanning every payload after ranking spent
the catalog's full capacity budget on a single language miss while retaining
database admission. That earlier implementation took 514 ms for search and
523 ms for browse at 100,000 collections. It met the 1,500 ms latency gate but
unnecessarily serialized payload decoding against registrations and withdrawals.
The SQL projection removes that catalog-wide decoding work from queries.
The release benchmark with the SQL projection passed at the same 100,000
synthetic collections: the no-match search took 28.354 ms and browse took
0.130 ms. Selective search p95 increased from below 1 ms to 11 ms; both workloads
remain below the existing 1,500 ms gate. These local runs establish the measured
tradeoff and the regression test establishes the bound on payload decoding;
they do not establish deployment throughput or an improvement for every query.

The projection is additive and rebuilt atomically on startup from previously
admitted stored manifests, including after a stopped older binary changed the
database. Registration, replacement, withdrawal and expiry maintain it in the
same transactions as payload and FTS metadata. Rebuilding reads one payload at
a time; an unreadable payload aborts the transaction without losing prior
projection state or replay high-water marks.

`language_queries_do_not_decode_unrelated_payloads` fails with the prior query
implementation and passes with SQL language selection. Additional regressions
cover original and versioned catalog schemas, repeated reopening, rollback on
startup corruption, updates across two connections, empty and duplicate
languages, and v2 preference when the legacy and current languages differ.

The latest benchmark's load phase plus selective samples took approximately
591 seconds (589 seconds before the SQL projection).
Registration repeatedly checks distinct-entry capacity and expiry; profiling
that cost is a separate performance follow-up before proposing transactional
counter projections or changing maintenance cadence. Any optimization must keep
SQLite authoritative across multiple connections and retain replay high-water
marks and registration ceilings.

### Background runtime ownership on failure

Dropping a public runtime dropped ordinary `JoinHandle`s and detached its source
and renewal tasks. The same ownership issue affected other composition-layer
tasks when startup, restart or shutdown was abandoned. Normal explicit
cancellation did not cover every such exit.

The LAN runtime and forwarder, public source and renewal, and MCP application
worker now own `AbortOnDropHandle`s. Explicit shutdown still cancels and joins;
owner loss aborts unfinished async tasks. Service destruction also signals the
application and public cancellation tokens. Model activity leases and watcher
handles already have explicit cancellation behavior and retain it.

`dropping_public_runtime_stops_its_source_and_renewal_tasks` failed before the
ownership change and passes after it. It observes both synthetic tasks starting,
then verifies their lifetimes end when the real runtime container is dropped.

### Lost MCP mutation completion

An accepted MCP request whose response channel closed returned a retryable
availability failure even for mutations. Its blocking operation could still
finish after the async worker exited. Only the response timeout previously
received the existing non-retryable `outcome_unknown` classification.

Both lost-response cases now use that classification for mutation tools. Reads
and requests rejected before admission retain their availability error. The
canonical message covers completion uncertainty without claiming a specific
transport failure. `lost_mcp_worker_response_preserves_mutation_uncertainty`
reproduced the incorrect classification and now covers both mutation and read
responses. This preserves the existing fingerprint/reconciliation recovery
contract and changes no capability, grant or publication authority.

## Validation evidence

The implementation at `d40a2fa00789873db0466181d4b13f7991702ff6` passed:

- Workspace Clippy with all targets, all features and warnings denied.
- Workspace tests: 1,344 passed, one ignored, across 25 suites.
- Documentation and license-inventory checks, plus `cargo deny --locked check`
  (advisories, bans, licenses and sources). Existing duplicate-dependency
  warnings remain; no dependency version changed.
- The release index benchmark and cancellation, recovery, protocol preference,
  expiry and withdrawal regressions described above.

Cross-platform CI for `e04add6fce2d004b48f62fbe387fee1df621f700` also passed:
[CI run 34107633322](https://github.com/airwiki/airwiki/actions/runs/34107633322)
completed workspace Clippy, tests, documentation, generated contracts and both
native desktop E2E journeys on macOS and Windows. Dependency, license and
frontend checks passed, as did
[CodeQL](https://github.com/airwiki/airwiki/actions/runs/34107630466) and
[DCO](https://github.com/airwiki/airwiki/actions/runs/34107682388).
Hosted-runner E2E supplements the separate installed-platform evidence below.

The SQL-projection follow-up passed all 38 index-package tests, package Clippy
with all targets and features and warnings denied, workspace formatting and
documentation checks. Its release benchmark results are recorded above. CI must
also pass on the final pull-request revision before integration.

An isolated development candidate was installed on macOS 26.6.2, arm64. It used
the `e2e` feature, a distinct app identifier, synthetic fixtures and a temporary
profile. The executable SHA-256 was
`1ceecfa794c0b525617b79fbaaab7290feb3041ef159ecc62253d6d8d4e7b17e`.
This is lifecycle evidence for the changed Rust services, not release-signing
or distribution acceptance.

| Installed macOS journey | Result |
| --- | --- |
| Existing onboarding and real IPC journey | PASS |
| Coordinated quit through the UI event, with successful process exit | PASS |
| Relaunch and restore the saved page and panel state | PASS |
| Select **Quit AirWiki** in the native app menu | PASS; process exited with code 0 |
| Relaunch after native-menu quit and repeat session restoration | PASS |

The checks used no real peers or downloaded models and left the normal app's
profile and the parallel UI checkout untouched. The native-menu check exercised
the installed app interactively; the existing WebDriver journey checked state
restoration and clean process exit. Visual baselines were disabled.

An isolated development candidate built from
`e04add6fce2d004b48f62fbe387fee1df621f700` was installed on Windows 11 Home
25H2, build 26200.9278, x64. The test ran in an interactive desktop session with
the pinned Rust 1.96.1 toolchain, the `e2e` feature, a separate installation and
the existing temporary-profile guards and synthetic fixtures. Incremental
compilation and development debug symbols were disabled to reduce disk use.
The installed executable matched the build output by SHA-256:
`96a923571a082c24561c85428a3111e5caa87aac39d8d576f5aa0fdf22468a28`.

| Installed Windows journey | Result |
| --- | --- |
| Existing real IPC journey and automated session restoration | PASS; runner exited with code 0 |
| Open the same installed candidate and select a synthetic page through the native UI | PASS |
| Hide the sidebar, request native close and select **Quit completely** | PASS; process exited with code 0 and no longer existed |
| Relaunch the same candidate and temporary profile | PASS |
| Recover the selected page and hidden sidebar through the native UI | PASS |
| Quit again through the native UI and verify no QA process remains | PASS; process exited with code 0 |

The Windows task's native-control observations and process-exit checks establish
this lifecycle result separately from hosted CI. The normal application remained
running, real data and the parallel UI work were preserved, and the QA copy was
left closed. The test used no real peers or downloaded models. As on macOS,
this development candidate does not establish public signing, installer,
firewall, LAN or distribution acceptance.

## Independent review

A separate reviewer context inspected the Rust changes against the architecture,
threat model and ADR 0008. It identified the catalog-wide decoding cost described
above. After the SQL projection correction, a second pass found that issue
resolved and no further actionable integrity, v1/v2 compatibility or availability
regressions in the follow-up diff. That second pass was static; the author ran
the 38 passing index-package tests and Clippy with warnings denied separately.
The separate installed-platform checks complement this review. Maintainers
remain responsible for accepting the changes through the normal PR flow.

## Delivery and deferred work

Integration requires green applicable CI and DCO checks on the final pull-request
revision. Confirmed architecture conclusions are synchronized in the project
AirWiki memory at task closure; versioned repository documentation remains
authoritative.

Broad module splitting, a database pool, additional crates and protocol changes
need a demonstrated benefit before implementation. Public infrastructure
deployment and visual redesign are outside this audit.

## Authoritative references

- [Architecture](architecture.md)
- [Threat model](threat-model.md)
- [Public federation decision](adr/0008-public-federation.md)
- [Contributor workflow](../CONTRIBUTING.md)
- [Code review](../CODE_REVIEW.md)
- [Installed validation requirements](maintainer-validation.md)
