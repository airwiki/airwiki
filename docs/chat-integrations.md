# Local chat integrations

AirWiki exposes `search_airwiki` plus capability-authenticated memory and
attested-computation tools to ChatGPT Desktop/Work, Codex, Claude Desktop,
Claude Code, Gemini CLI and generic local MCP clients. AirWiki does not need
provider API keys; each client owns its account and session.

The managed bridge implements MCP `2026-07-28`: clients may begin with
`server/discover`, requests self-describe through `_meta`, and the fixed
Streamable HTTP hop is sessionless. AirWiki continues to accept the older MCP
lifecycle for compatible clients during the transition. Its deterministic tool
list is private and non-cacheable across authorization contexts, and every tool
advertises typed input/output schemas and explicit safety hints.

## Before connecting

1. Keep AirWiki open or hidden in the tray and wait until local models, Wiki
   reconciliation, LAN, and MCP are ready.
2. Publish only synthetic content or content deliberately approved for an
   external chat provider.
3. Enable **Allow in external chats** only on the required Wikis. This remains
   independent from peer sharing.
4. Open **Connections**, select **Refresh**, and review the two independent
   states for the client: **Local connection** and **Assisted memory**.

Connecting a client never publishes documents, grants Wikis, changes peer
permissions, or enables cloud access. Snippets requested from an authorized
Wiki may enter the provider cloud and are then governed by that provider's
policies.

Each managed integration receives a random capability stored in AirWiki's
private per-user files. The client configuration contains only a public
integration identifier; the bridge resolves and sends the secret to the fixed
loopback endpoint. The secret never appears in MCP arguments, tool responses,
events or logs.

The loopback endpoint rejects browser-origin requests, unexpected host headers,
redirects and oversized bodies. A tool failure is returned as a sanitized,
machine-readable result with a stable code and retryability hint; protocol
errors are reserved for malformed or unknown MCP requests. Client UI approval
and AirWiki's native confirmations remain the human-in-the-loop boundary.

An application may create and maintain only AI-memory Wikis it owns or has an
explicit reader/editor grant for. It may create, edit and deprecate concepts,
but cannot delete, share, verify, change history or operate on folder/imported
Wikis. Granting one application access to another application's memory requires
an OS-native confirmation in AirWiki. Revocation stops access immediately and
does not delete the memory.

## Assisted-memory guide

For Codex/ChatGPT, Claude Code and Gemini CLI, **Connect** uses one native
confirmation to install both the MCP connection and AirWiki's versioned,
instruction-only `airwiki` skill. It also installs a short `AirWiki.md` and adds
one `@AirWiki.md` import to the client's global instructions:

| Client | Skill | Guide and import |
| --- | --- | --- |
| Codex/ChatGPT | `~/.agents/skills/airwiki/` | `$CODEX_HOME/AirWiki.md` and `$CODEX_HOME/AGENTS.md` |
| Claude Code | `$CLAUDE_CONFIG_DIR/skills/airwiki/` | `$CLAUDE_CONFIG_DIR/AirWiki.md` and `$CLAUDE_CONFIG_DIR/CLAUDE.md` |
| Gemini CLI | `$GEMINI_CLI_HOME/.gemini/skills/airwiki/` | `$GEMINI_CLI_HOME/.gemini/AirWiki.md` and `$GEMINI_CLI_HOME/.gemini/GEMINI.md` |

The documented defaults are `~/.codex`, `~/.claude` and `~/.gemini` when the
corresponding root variable is unset. AirWiki never writes project-local
instructions. A custom root must be absolute and safely resolvable; otherwise
AirWiki reports an unsupported or conflict state instead of guessing.

The guide makes requests such as “create a wiki with AirWiki” self-contained.
It lists memories before selecting or creating one, pages through the selected
Wiki's concept metadata, and then reads the targeted concept body before an
edit. It uses optimistic fingerprints and retries one conflict at most once. In a code
repository it proposes `<project> — memory`; in general work it uses a named
thematic Wiki. Creation still requires an explicit user request, and ambiguous
matches require a choice.

Automatic capture is conversational agent behavior, not a background AirWiki
service. It begins only after the user selects or creates a Wiki for that
context, stores only durable reusable knowledge, and stops after “pause
AirWiki”. The guide excludes secrets, personal data, logs, transient state,
speculation and long file copies. It cannot verify, share, publish, grant access
or change permissions.

AirWiki stages and hashes managed resources, preserves UTF-8 BOM and LF/CRLF,
and writes private receipts. Modified user resources are never overwritten or
removed: **Assisted memory** shows **Conflict** and requires manual resolution.
Disconnect revokes the application capability immediately and removes only
files and the single import that still match AirWiki's receipt.

Attested computation requests remain pending until the user confirms them in
AirWiki. A result expires after ten minutes. Saving an accepted result requires
a second confirmation and records a machine-confirmed process concept rather
than a human review. Each application is limited to 16 pending computation
requests and 30 requests per minute. Sanitized terminal run metadata is retained
for at most 24 hours and is pruned on startup or the next request.

## ChatGPT Desktop/Work and Codex

Select **Connect**, review the global locations, and confirm. AirWiki uses the
compatible CLI found on `PATH` to register a local MCP server named `airwiki`.
A different entry with the same name is a conflict and is never overwritten.
Managed bridge paths are content-addressed. A bridge from an earlier candidate
appears as **Update available** only after its path and bytes pass integrity
checks; selecting **Update integration** installs and verifies the packaged
bridge before changing the client entry.

Open a new task or restart the client if the skill or tool is not visible.
**Disconnect** removes the entry only while it still matches the configuration
installed by AirWiki.

## Claude Code

Select **Connect**, review the global files listed by the native confirmation,
and confirm. AirWiki registers `airwiki` as a user-scoped stdio MCP server. It
rejects local/project-scoped entries, environment-bearing entries and any
configuration that does not exactly match the managed bridge.

Open a new Claude Code conversation after installation or an update. A
different global MCP entry or modified workflow guide remains a visible
conflict and is not replaced.

Before changing the packaged skill, its awareness guide, or memory-tool
instructions, run the synthetic [AirWiki skill evaluation](airwiki-skill-evals.md).
It covers activation, exact reuse, ambiguity, read-before-write, fingerprint
conflicts, pause, unavailable-service behavior, and authority boundaries across
the supported assistant clients.

## Gemini CLI

Select **Connect** and confirm. AirWiki first probes CLI capabilities in a
temporary home directory, then registers the bridge at user scope, restricted
to the managed AirWiki tools and without automatic trust. Use `/mcp reload` in
an open session or restart the CLI.

A different entry with the same name remains an unresolved conflict.

## Claude Desktop

Select **Install extension**. AirWiki opens a platform-specific MCPB and Claude
shows its own approval screen. Use the installed extension from **Claude Home →
Chat**. Claude Code does not load this desktop MCPB integration.

The integration remains pending until Claude starts the bridge or the user
confirms the installation in AirWiki. Claude owns removal through **Settings →
Extensions**; AirWiki does not edit Claude's internal files or bypass an
administrative extension policy.

Claude Desktop receives the workflow through MCP server instructions included
in the MCPB. AirWiki does not write to Claude Desktop configuration or unknown
instruction directories.

## Generic MCP clients

The setup panel exposes only the bridge command and public client identifier.
The server's first instructions describe the complete memory sequence: list,
explicitly select or create, read, then write with the current fingerprint.
AirWiki never guesses a client's instruction directory.

## Availability and recovery

The bridge can advertise the tools while AirWiki is closed, but calls then
return a stable unavailable-node error. Reopen AirWiki and wait for preflight;
the next call recovers without reinstalling the bridge.

If an integration stops working:

- select **Refresh** and check whether the connection or guide needs an update;
- use **Update guide** only when AirWiki offers it and after reviewing the native prompt;
- start a new conversation after installing or updating assisted memory;
- confirm that no unrelated process owns `127.0.0.1:43123`;
- never expose that port through the LAN firewall;
- resolve guide conflicts manually without deleting another client's configuration; and
- disconnect from AirWiki before uninstalling it when the client permits.

Hiding AirWiki preserves MCP and LAN within the current user session. **Exit
completely** stops both. Optional per-user autostart requires explicit consent;
no daemon or system service survives logout.
