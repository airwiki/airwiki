# Application-layer guidance

These instructions apply under `apps/` in addition to the repository root guidance.

## Boundaries

- `desktop` is the composition root. Keep domain, authorization, publication, ranking, and protocol rules in reusable crates rather than widgets or worker glue.
- egui renders completed view models. Filesystem traversal, parsing, hashing, database-heavy work, inference, networking, and blocking operating-system calls run through the worker or an appropriate blocking boundary. Immediate window and tray integration may remain on the eframe thread when the platform API requires it.
- Worker commands and snapshots use request identifiers where stale responses could alter the visible state. UI actions must remain idempotent across retries and repaint cycles.
- `mcp-bridge` implements stdio transport only. Canonical server information, tools, schemas, limits, and sanitized errors remain in `airwiki-mcp`. The bridge uses the fixed loopback endpoint and never accepts arbitrary URLs, tokens, redirects, or proxies.
- `windows-firewall-helper` is a narrow elevated boundary. It accepts only the fixed install/remove operations, derives the sibling application path, and manages only exact AirWiki LAN rules. Do not add general command execution, arbitrary paths, ports, or rule definitions.

## User and platform behavior

- Keep the normal path understandable without UUIDs, PeerIds, multiaddresses, hashes, ports, MCP terminology, or terminal commands. Put sanitized technical detail behind an advanced disclosure.
- Never imply that pairing grants access, that chat connection enables a collection, or that hiding the window stops the node.
- Platform permission prompts remain human decisions. Cancellation or policy denial preserves local-only behavior and offers a recoverable next action.
- Closing, tray, autostart, integration setup, and firewall changes require focused lifecycle tests on the affected platform. Tests use fake platform adapters where possible; acceptance uses an installed development candidate.
- Application and helper logs go to stderr or the configured sanitized sink, never protocol stdout and never document or network identity data.
