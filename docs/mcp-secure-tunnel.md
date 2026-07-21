# Local MCP and Secure MCP Tunnel (advanced)

> This procedure is not part of normal workstation setup. Use **Integrations**
> and the [local integration guide](chat-integrations.md) for ChatGPT
> Desktop/Work, Claude Desktop, and Gemini CLI. The tunnel is retained only as
> an advanced reference for a future remote or web use case. Its credentials
> are managed outside AirWiki.

AirWiki exposes one read-only Streamable HTTP endpoint:

```text
http://127.0.0.1:43123/mcp
```

`search_airwiki` accepts `question` and optional `top_k`. AirWiki forces the
`external_ai` purpose and returns typed `evidence`, a separately bounded
`authorized_candidates` list, an optional flattened `search_items` lane view,
and an optional `coverage_gap`. Evidence contains
either citable relevant items or `no_relevant_evidence`. Candidates passed the
same publication and disclosure policy but were not verified as answering the
question by AirWiki's lightweight classifier. `search_items` is intended for
client implementations that prefer one stream and must be treated as
untrusted evidence metadata with explicit `lane` context.

The serialized MCP result has a global budget below the bridge's 64 KiB HTTP
limit. AirWiki removes lower-ranked candidates before evidence and reports
incomplete coverage if reduction was necessary.

Every source node applies its local relevance gate before creating evidence.
For external-chat searches only, rejected passages may cross MCP as explicitly
typed candidates after the source rechecks publication, `allow_external_ai`,
peer grants and revocation. The chat client may use a candidate only when its
snippet explicitly supports a requested fact. Authorization alone is never a
relevance claim. Peer and backend diagnostics are discarded, and AirWiki does
not synthesize a second answer. The listener accepts only
`Host: 127.0.0.1:43123` or `Host: localhost:43123`.

The endpoint is stateless. Requests are independently validated and no
`Mcp-Session-Id` is created. Every search rechecks `allow_external_ai` and
source-node authorization.

## Prerequisites

- a reviewed, published synthetic collection with explicit
  `allow_external_ai`;
- a running local MCP endpoint, not merely a ready UI label;
- a `tunnel_id` from [Platform tunnel settings](https://platform.openai.com/settings/organization/tunnels);
- a runtime API key supplied directly to `tunnel-client`;
- Tunnels **Read + Use** permissions; and
- developer mode enabled for the ChatGPT workspace.

Creating or editing tunnels requires **Read + Manage**. Running the client needs
**Read + Use**. Developer mode is a separate permission. See the
[official Secure MCP Tunnel guide](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels).

## Configure `tunnel-client`

Download the binary from Platform or the
[latest public release](https://github.com/openai/tunnel-client/releases/latest).
Do not pin another version in this repository without reviewing its security
notes.

Check the available syntax first:

```bash
tunnel-client help quickstart
```

Configure an HTTP profile. Replace the identifier and provide the key only in
the client process environment:

```bash
export CONTROL_PLANE_API_KEY="sk-..."

tunnel-client init \
  --sample sample_mcp_remote_no_auth \
  --profile airwiki-local \
  --tunnel-id tunnel_0123456789abcdef0123456789abcdef \
  --mcp-server-url http://127.0.0.1:43123/mcp \
  --health-listen-addr 127.0.0.1:0

tunnel-client doctor --profile airwiki-local --explain

health_url_file="/tmp/airwiki-tunnel-health.url"
rm -f "$health_url_file"
tunnel-client run \
  --profile airwiki-local \
  --health.listen-addr 127.0.0.1:0 \
  --health.url-file "$health_url_file"
```

`sample_mcp_remote_no_auth` is appropriate because AirWiki already hosts
Streamable HTTP and deliberately does not advertise OAuth. Port `0` requests an
unused loopback health port. Keep `run` active during discovery and calls.

Never store `CONTROL_PLANE_API_KEY` in AirWiki, SQLite, watched files,
screenshots, or this repository. The client needs outbound HTTPS to
`api.openai.com:443`, or `mtls.api.openai.com:443` for an organization using
mTLS. It needs no Internet ingress.

## Check health

`tunnel-client` exposes `/healthz`, `/readyz`, `/metrics`, and a local `/ui`.
The UI is loopback-only by default.

1. Both OAuth metadata probes must return `404` with a nonempty body. This means
   OAuth is intentionally absent:

   ```bash
   curl -i http://127.0.0.1:43123/.well-known/oauth-protected-resource/mcp
   curl -i http://127.0.0.1:43123/.well-known/oauth-protected-resource
   ```

2. `doctor --explain` in tunnel-client v0.0.10 may report a false negative after
   those expected responses. Do not weaken local validation or add OAuth
   metadata to satisfy it.
3. With `run` active, `/readyz` at the URL written by `--health.url-file` is the
   authoritative check:

   ```bash
   health_url_file="/tmp/airwiki-tunnel-health.url"
   health_base_url="$(cat "$health_url_file")"
   curl -fsS "$health_base_url/readyz"
   open "$health_base_url/ui"
   ```

4. Confirm that the client is ready and polling.
5. Confirm that AirWiki responds on loopback.
6. Confirm that every cloud-enabled collection contains synthetic content.
7. Confirm that each source node enforces its grants.

## Connect ChatGPT

In ChatGPT, open **Settings → Plugins** or
[chatgpt.com/plugins](https://chatgpt.com/plugins), create a developer-mode app,
choose **Tunnel**, and select the tunnel. If it is absent, verify that it belongs
to the correct ChatGPT workspace rather than only the Platform organization.

Use a synthetic acceptance question such as:

```text
How is Project Atlas recovered, who is responsible, and what is the target date? Cite each item with node, resource, heading, revision, and hash.
```

The answer must use synthetic evidence from both nodes. A compound question may
require focused searches for procedure, owner, and date. Then stop one node and
confirm an explicitly partial answer.

Before changing `ServerInfo.instructions`, the tool description, or its schema,
run the synthetic [golden prompt set](mcp-prompt-evals.md). It checks tool
selection, grounding, citations, missing evidence, contradictions, partial
coverage, and prompt injection in evidence. It is manual, never uses real
collections, and never runs in CI.

A tunnel test does not replace two-node acceptance. Missing remote evidence must
remain missing and must never be inferred by the chat client.

## Stop and revoke

1. Stop `tunnel-client run`.
2. Disable the app or tunnel in OpenAI when the test is complete.
3. Remove `allow_external_ai` from the test collections.
4. Rotate the API key if it was exposed or persisted in shell history.

The tunnel is transport only. It never replaces review, grants, cloud policy,
or source-node audit.
