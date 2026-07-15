# Recovery guide

General rule: exit AirWiki, preserve a copy of its complete application-data
directory, and leave watched originals unchanged. Never repair an index by
deleting or rewriting the source folder.

## Download or hash failure

1. Check system time, free disk space, and outbound HTTPS.
2. Retry from the application; resumable partial files remain temporary.
3. Stop after the same hash fails twice. Never rename a `.part` file into place
   or disable verification.
4. Preserve only the artifact name, expected and observed hashes, stable error
   code, and sanitized logs.

Packaging retries may remove only their verified staging or cache directories.
Windows runtime recovery must rerun the pinned source-build script; never replace
it with an unrelated upstream binary. A toolchain-policy failure, unexpected PE
import, invalid publisher signature, or non-reproducible clean build blocks the
candidate.

## Search models fail to load

- LAN and MCP must remain closed after embedding or relevance-model load and
  smoke-test failures.
- Repair through the application's verified download action. Do not copy ONNX
  files across platforms or edit revision markers.
- Confirm AVX2 and available memory on Windows and arm64 assets on macOS.
- The relevance smoke test must accept the pertinent synthetic passage and reject
  its distractor; output shape alone is insufficient.
- Stop after two failures of the same verified snapshot and keep only sanitized
  diagnostics, never questions, passages, or local paths.

## `llama-server` does not start

- Confirm the packaged runtime is complete. The validated Windows runtime
  contains `llama-server.exe` and `BUILD-MANIFEST.json`, with no auxiliary
  redistributable DLL bundle.
- Check AVX2, memory, and competing local workloads.
- The supervisor selects an ephemeral loopback port. Never bind the model server
  to LAN or remove its random token.
- If health does not respond within the bounded startup deadline, stop AirWiki
  and retry once before preserving diagnostics.

The server starts on demand and stops after idle time; its absence while no
inference job exists is normal.

## MCP is unavailable

- The endpoint is `http://127.0.0.1:43123/mcp`; check whether another process owns
  that port.
- A `Host` other than loopback must receive `403`.
- On macOS, allow AirWiki to access its device identity in the login
  keychain. Cancelling leaves LAN and MCP closed.
- Reopen the application to retry a cancelled keychain request. Do not delete the
  identity item casually: doing so creates a new device identity and invalidates
  existing trust and grants.
- MCP continues while the window is hidden and stops on **Exit completely**.

Never open port 43123 in the LAN firewall.

## A chat client cannot find AirWiki

- Open **Integrations** and select **Refresh**.
- Treat a same-name entry with different configuration as a conflict; never
  overwrite it manually.
- Open a new ChatGPT task, use `/mcp reload` in Gemini, or complete the MCPB
  approval from Claude Home → Chat.
- Use **Update integration** for an older managed bridge rather than replacing an
  executable that may be running.
- If AirWiki was closed, reopen it and wait for preflight. The bridge does
  not need reinstallation.
- Remove Claude integrations through **Settings → Extensions**. Administrative
  policy must be resolved by the device administrator.

## Devices are not discovered

1. Confirm both applications are open on the same private subnet.
2. Keep LAN disabled on a Windows Public network; change the profile through
   Windows settings when appropriate.
3. Resolve disabled firewall or “block all incoming connections” through Windows
   settings or the administrator, then select **Check again**.
4. The only managed inbound rules are `AirWiki LAN (TCP)` and
   `AirWiki Discovery (mDNS)`: Private/Domain, `LocalSubnet`, no edge
   traversal, and never Public.
5. Do not delete an unknown broad or conflicting rule automatically. Review it
   in advanced firewall settings.
6. Use **Configure firewall…** only from the trusted installed candidate and
   confirm UAC. Cancelling preserves local-only mode.
7. On macOS, confirm Local Network permission.
8. Check Wi-Fi client isolation, VLAN boundaries, and multicast blocking.

Manual multiaddresses are an advanced diagnostic only. Do not enable UPnP,
relay, or Internet traversal.

An authenticated peer may appear offline after an idle TCP connection closes.
Search redials a known address under the existing deadline and reauthenticates
the PeerId through Noise. It never bypasses trust, grants, or policy.

## Pairing or SAS expires

- Reject the session if any SAS word differs.
- Start a new session; never reuse a captured code.
- Complete both confirmations before the two-minute deadline.
- After revocation, wait for connection closure before pairing again.

## Document remains pending or failed

- A collection disables manual rescan while another scan is active. Wait for its
  final summary.
- Startup and 15-minute reconciliation recover missed watcher events.
- Verify supported extension, 50 MiB maximum size, unencrypted PDF, and a text
  layer. OCR, hidden files, temporary files, and symlinks are unsupported.
- One invalid structured LLM response may retry once; subsequent failure requires
  human metadata and never automatic publication.
- Interrupted ingestion jobs become failed at restart and may be reclaimed by an
  idempotent scan without increasing revision when content is unchanged.

Do not force a revision through rename, timestamp changes, or duplicate copies.
Identity follows content and persistent state.

## Publication was interrupted

Human approval is persisted before bundle writes. While materialization is in
progress, the revision stays unsearchable. Startup may resume only that durable
approved intent.

1. Reopen with the same source, application data, and wiki directory available.
2. Wait for `Published` or a diagnosed return to `NeedsReview`.
3. Approve again only after reviewing a returned draft.
4. If `Publishing` persists after restart, exit, preserve application data and
   sanitized logs, and escalate. Never edit concepts, `index.md`, `log.md`, or
   SQLite to force completion.

## Guided wiki repair

The Health tab may produce a read-only repair preview describing every affected
page, selected authority, and effect. Cancelling writes nothing.

On confirmation, AirWiki:

1. rejects a stale bundle fingerprint;
2. creates and verifies a local snapshot;
3. withdraws affected published concepts from LAN and MCP and returns them to
   review before changing their pages;
4. removes an orphan only when SQLite confirms it is not a published concept;
5. validates the result and writes a content-free audit event.

Failure restores files from the snapshot but does not restore withdrawn
authorization. Damaged `log.md` or ambiguous scope remains blocked for separate
human recovery. AirWiki never fabricates history, concepts, permissions,
or publication.

## Watched folder unavailable

Loss of the root or watcher quarantines the collection, removes searchable
chunks, and withdraws published concepts. The watcher retries periodically.

1. Restore the same root and permissions.
2. Wait for a successful complete scan.
3. Review recovered documents again; prior publication is not restored.
4. Preserve sanitized diagnostics if the watcher continues to fail.

An incomplete traversal also fails closed but does not create tombstones for
uncertain absences. Restore access and require one successful complete scan.

## SQLite or local index inconsistency

1. Exit every AirWiki process.
2. Copy the complete data directory, including WAL and SHM files.
3. Keep watched originals unchanged.
4. Diagnose only the copy; never run repair tools against the sole database.
5. Re-ingestion into a new database requires fresh human review and never
   authorizes publication automatically.

The current MVP does not promise automatic SQLite recovery.

## Device identity loss

The private identity lives in Keychain or Credential Manager, not SQLite.
Restoring only the database creates a different device relationship. Revoke the
old device from its peers, pair the new identity, and grant collections again.
Never copy a private device key between computers.
