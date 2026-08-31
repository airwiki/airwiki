# Privacy and data handling

This document describes AirWiki's intended product behavior as of the current
technical beta. It is technical documentation, not legal advice, a contract,
or a substitute for a review by the project owner or qualified counsel.

## The default: local and private

AirWiki is designed to keep knowledge on the device that owns it by default.
It stores operational state and local paths in SQLite, and manages visible Wiki
knowledge as local OKF files. Folder sources, extracted text, concepts,
embeddings, and local search indexes remain on that device unless a person
explicitly enables a separate disclosure path.

AirWiki does not provide accounts, cloud sync, centralized document storage,
or a default telemetry service. Default application logs are structured and are
intended to omit document content, queries, snippets, credentials, local paths,
and network identities. No implementation can protect a compromised device,
screenshots, manual copying, or content that a person deliberately authorizes
for another service.

## When data can leave the device

Each of these choices is independent and opt-in:

| Choice | What may leave | Important boundary |
| --- | --- | --- |
| Install or prepare local model assets | A download request and normal network metadata to the selected asset host | Local models process knowledge on-device after verified assets are available |
| Trust a LAN device and grant a Wiki | Bounded, authorized search evidence and, when opened, that exact published read-only OKF Wiki | Discovery alone grants nothing; original sources, paths, chunks, embeddings, and operational indexes stay local |
| Publish a reviewed Wiki experimentally | Signed, expiring routing metadata to public indexes; authorized evidence and published OKF content to public readers | It never publishes drafts or source folders; public federation is experimental |
| Enable public search for a Library search or connected AI app | That query can reach configured public indexes and selected publishers | This does not publish a local Wiki; public search is off by default |
| Connect a chat or MCP client | The client receives only knowledge the person explicitly authorizes it to read | Data entering a third-party client or provider is subject to that service's account or workspace policy |
| Check for updates, once a stable channel exists | Standard request metadata such as IP address, user agent, and requested version | The current baseline has no active public update distribution |

AirWiki's local model may propose metadata. It cannot decide to publish, grant
access, or allow data to leave the device.

## Retention and control

AirWiki keeps its local operational state and managed Wiki files until the
person deletes them or uses the applicable product recovery/uninstall flow.
Windows uninstall keeps local data by default. A public reader who receives a
published OKF Wiki or another service that receives authorized data may retain
it according to that recipient's controls; AirWiki cannot delete copies outside
the owner's device. Revoking access prevents future authorized responses but
cannot recall data already disclosed.

Do not use public issues for private support material. Their content is public
and governed by GitHub's policies. Use only synthetic reproductions; see
[SUPPORT.md](SUPPORT.md). For suspected security or data exposure, use the
private process in [SECURITY.md](SECURITY.md).

## Read the technical boundaries

The authoritative technical detail is the [threat model](docs/threat-model.md),
including limitations and containment steps. The [search and federation guide](docs/search-and-federation.md)
explains local, LAN, and public paths, while [chat integrations](docs/chat-integrations.md)
explains the connected-app boundary. The project owner must complete the legal
and distribution review listed in the [release checklist](docs/release-checklist.md)
before any supported public release.
