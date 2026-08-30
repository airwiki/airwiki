# ADR 0002: Connect local chat clients through one MCP stdio bridge

- Status: Accepted
- Date: 2026-08-14
- Superseded in part by: ADR 0003 (desktop lifecycle and per-user autostart)
- Superseded in part by: ADR 0010 (capability-authenticated memory and computation tools)
- Superseded in part by: ADR 0011 (project-memory discovery and selection workflow)

## Context

AirWiki hosts policy-scoped search plus capability-authenticated memory and
computation tools on a loopback HTTP listener. Desktop
clients register local servers differently: ChatGPT Desktop/Work and Gemini CLI
can register MCP processes, while Claude Desktop distributes local extensions
as MCPB packages. Requiring tunnels, API keys or manual configuration editing
would contradict the goal of a simple local installation.

Relevant contracts are the official [Codex MCP documentation], [Codex skills
documentation], [Claude Code memory documentation], [Gemini CLI skills
documentation], [Gemini CLI MCP server documentation], the normative [MCP
2026-07-28 specification], Anthropic's [MCP guidance], [security guidance] and
[tool design guidance], [Anthropic skill authoring best practices], and [MCPB manifest
v0.3].

[Codex MCP documentation]: https://developers.openai.com/codex/mcp/
[Codex skills documentation]: https://developers.openai.com/codex/skills/
[Claude Code memory documentation]: https://code.claude.com/docs/en/memory
[Gemini CLI skills documentation]: https://geminicli.com/docs/cli/tutorials/skills-getting-started/
[Gemini CLI MCP server documentation]: https://geminicli.com/docs/tools/mcp-server/
[MCP 2026-07-28 specification]: https://modelcontextprotocol.io/specification/2026-07-28
[MCP guidance]: https://code.claude.com/docs/en/mcp
[security guidance]: https://code.claude.com/docs/en/security
[tool design guidance]: https://www.anthropic.com/engineering/writing-tools-for-agents
[Anthropic skill authoring best practices]: https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices
[MCPB manifest v0.3]: https://github.com/modelcontextprotocol/mcpb/blob/70fe3b34cd6dff1b3bba046638edc72a6467a4fb/MANIFEST.md

## Decision

The desktop keeps Streamable HTTP MCP at `127.0.0.1:43123/mcp` as the canonical
internal endpoint. A separate Rust executable, `airwiki-mcp-bridge`,
exposes the same contract over stdio and forwards only to that fixed endpoint.
Managed ChatGPT Desktop/Work, Codex, Claude Desktop, Claude Code and Gemini CLI
integrations use the same bridge. Each client retains a distinct capability and
application identity.

The gateway and bridge implement MCP `2026-07-28` through the stable Rust SDK.
Modern requests self-describe through `_meta`, use `server/discover`, omit
sessions, and carry the standard routing headers on Streamable HTTP. Tool lists
are deterministic, complete, immediately stale (`ttlMs: 0`) and private because
the visible set can depend on an integration capability. Older lifecycle
requests remain accepted only for client interoperability during the upgrade.
Every tool publishes JSON Schema 2020-12 input and output contracts plus
accurate read-only, destructive, idempotent and open-world hints. Recoverable
execution failures return sanitized typed tool results; JSON-RPC errors remain
reserved for malformed or unroutable protocol requests.

Installation is per-user, visible, confirmed and reversible. The bridge accepts
no arbitrary endpoint, ignores ambient proxies, stores no credentials and
grants no Wiki access. Each client's configuration remains its own source
of truth.

AirWiki materializes the packaged bridge below a private path keyed by both the
application version and the bridge SHA-256. This prevents two reviewed
candidates with the same pre-release application version from aliasing
different executables. A prior content-addressed bridge is accepted only when
its bytes match the digest in its path and is presented as an available update;
the current bridge must also match the immutable packaged copy. Legacy
version-only paths remain replaceable for migration, while malformed paths,
links, reparse points and altered content-addressed bridges fail closed as
conflicts.

The HTTP boundary accepts only the exact loopback host and port, rejects all
browser `Origin` requests, ignores ambient proxies and redirects, bounds request
and response bodies, and never exposes a configurable endpoint. User approval
in AirWiki or the client remains mandatory for connecting integrations,
granting another application access, executing a computation and saving its
result. Tool annotations and instructions are usability hints, never an
authorization boundary.

For Codex/ChatGPT, Claude Code and Gemini CLI, the same native confirmation also
installs a versioned, instruction-only user skill plus an `AirWiki.md` imported
once from the client's global instructions. AirWiki uses only documented user
roots, never modifies repository files, stages and hashes managed bytes, and
records a private receipt. User-modified resources become conflicts and are not
overwritten or deleted. Claude Desktop receives equivalent server instructions
inside the MCPB; generic MCP clients receive standard MCP instructions without
filesystem writes.

The guide directs agents to page through concept metadata, read the targeted
Markdown body before mutation, use optimistic fingerprints, create only after
an explicit request, and avoid secrets,
personal data and transient state. It grants no publication, sharing,
verification or permission authority. If AirWiki is unavailable, the primary
task continues with one pending-sync notice and no repository-local fallback.

The application must remain running in the user's session, either visible or
hidden. ADR 0003 supersedes this ADR's original exclusion of tray operation and
per-user autostart, while preserving the prohibition on daemons and system
services.

## Consequences

- There is one definition of MCP tools, instructions and response schemas.
- The managed bridge verifies discovery, protocol version, complete typed tool
  metadata and the exact tool set before installing a client configuration.
- A plain-language request can invoke a consistent memory workflow across
  clients that support user skills or MCP instructions.
- All managed clients share bridge limits, errors and fixes.
- Skill or instruction updates require a new conversation and never replace
  user-modified files.
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
