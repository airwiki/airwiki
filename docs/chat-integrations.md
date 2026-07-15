# Local chat integrations

AirWiki exposes one read-only tool, `search_airwiki`, to
ChatGPT Desktop/Work, Claude Desktop, and Gemini CLI. AirWiki does not
need provider API keys; each client owns its account and session.

## Before connecting

1. Keep AirWiki open or hidden in the tray and wait until local models,
   collection reconciliation, LAN, and MCP are ready.
2. Publish only synthetic content or content deliberately approved for an
   external chat provider.
3. Enable **Allow in external chats** only on the required collections. This is
   independent from peer sharing.
4. Open **Integrations**, select **Refresh**, and review the detected client and
   version.

Connecting a client never publishes documents, grants collections, changes peer
permissions, or enables cloud access. Snippets requested from an authorized
collection may enter the provider cloud and are then governed by that provider's
policies.

## ChatGPT Desktop/Work

Select **Connect**, review the bridge location, and confirm. AirWiki uses
the CLI bundled with ChatGPT or a compatible CLI found on `PATH` to register a
local MCP server named `airwiki`. A different entry with the same name is
a conflict and is never overwritten.

Open a new task or restart ChatGPT if the tool is not visible. **Disconnect**
removes the entry only while it still matches the configuration installed by
AirWiki.

## Gemini CLI

Select **Connect** and confirm. AirWiki first probes CLI capabilities in a
temporary home directory, then registers the bridge at user scope, restricted to
`search_airwiki` and without automatic trust. Use `/mcp reload` in an
open session or restart the CLI.

A different entry with the same name remains an unresolved conflict.

## Claude Desktop

Select **Install extension**. AirWiki opens a platform-specific MCPB and
Claude shows its own approval screen. Use the installed extension from
**Claude Home → Chat**. Claude Code does not load this desktop MCPB integration.

The integration remains pending until Claude starts the bridge or the user
confirms the installation in AirWiki. Claude owns removal through
**Settings → Extensions**; AirWiki does not edit Claude's internal files or
bypass an administrative extension policy.

## Availability and recovery

The bridge can advertise the tool while AirWiki is closed, but searches
then return a stable unavailable-node error. Reopen AirWiki and wait for
preflight; the next call recovers without reinstalling the bridge.

If an integration stops working:

- select **Refresh** and check whether the client or bridge needs an update;
- confirm that no unrelated process owns `127.0.0.1:43123`;
- never expose that port through the LAN firewall;
- do not resolve conflicts by editing client configuration manually; and
- disconnect from AirWiki before uninstalling it when the client permits.

Hiding AirWiki preserves MCP and LAN within the current user session.
**Exit completely** stops both. Optional per-user autostart requires explicit
consent; no daemon or system service survives logout.
