# ADR 0002: Connect local chat clients through one MCP stdio bridge

- Status: Accepted
- Date: 2026-07-12
- Superseded in part by: ADR 0003 (desktop lifecycle and per-user autostart)

## Context

AirWiki hosts one read-only MCP tool on a loopback HTTP listener. Desktop
clients register local servers differently: ChatGPT Desktop/Work and Gemini CLI
can register MCP processes, while Claude Desktop distributes local extensions
as MCPB packages. Requiring tunnels, API keys or manual configuration editing
would contradict the goal of a simple local installation.

Relevant contracts are the official [Codex MCP documentation],
[Gemini CLI MCP server documentation] and [MCPB manifest v0.3].

[Codex MCP documentation]: https://developers.openai.com/codex/mcp/
[Gemini CLI MCP server documentation]: https://geminicli.com/docs/tools/mcp-server/
[MCPB manifest v0.3]: https://github.com/modelcontextprotocol/mcpb/blob/70fe3b34cd6dff1b3bba046638edc72a6467a4fb/MANIFEST.md

## Decision

The desktop keeps Streamable HTTP MCP at `127.0.0.1:43123/mcp` as the canonical
internal endpoint. A separate Rust executable, `airwiki-mcp-bridge`,
exposes the same contract over stdio and forwards only to that fixed endpoint.
Managed ChatGPT Desktop/Work, Claude Desktop and Gemini CLI integrations use the
same bridge.

Installation is per-user, visible, confirmed and reversible. The bridge accepts
no arbitrary endpoint, ignores ambient proxies, stores no credentials and
grants no collection access. Each client's configuration remains its own source
of truth.

The application must remain running in the user's session, either visible or
hidden. ADR 0003 supersedes this ADR's original exclusion of tray operation and
per-user autostart, while preserving the prohibition on daemons and system
services.

## Consequences

- There is one definition of MCP tools, instructions and response schemas.
- All managed clients share bridge limits, errors and fixes.
- Claude requires an additional confirmation in its MCPB installer.
- Loopback does not authenticate operating-system accounts. Any process running
  as the user may attempt to call it; `allow_external_ai`, the answerability
  gate, response limits and rate limiting bound exposure but do not replace OS
  session security.
- Web, mobile, Secure MCP Tunnel, API-key management, marketplace distribution
  and system services remain outside this decision.

## Rejected alternatives

- **One MCP implementation per client:** duplicates contracts and security
  policy.
- **User-managed tunnel or API key:** introduces unnecessary cloud setup for a
  workstation-local workflow.
- **Editable bridge endpoint:** expands the local bridge into an unneeded proxy
  and weakens the loopback boundary.
- **Silent configuration edits:** make conflicts and ownership ambiguous.
