# Privacy notice and data handling

This notice describes the data flows implemented or documented for the official
AirWiki desktop technical beta and its static project landing as of the
repository revision from which it is read. It is technical documentation, not
legal advice, a contract, or a substitute for a review by the project owner or
qualified counsel.

It distinguishes the open-source software from sites and community channels
operated by third parties. A person building from source, self-hosting a
federation service, or modifying a fork controls that deployment and must
assess its own data handling. This notice does not make a maintainer the
controller of another person's self-hosted deployment.

## The default: local and private

AirWiki is designed to keep knowledge on the device that owns it by default.
It stores operational state and local paths in SQLite, and manages visible Wiki
knowledge as local OKF files. Folder sources, extracted text, concepts,
embeddings, and local search indexes remain on that device unless a person
explicitly enables a separate disclosure path.

The official desktop candidate has no accounts, cloud sync, centralized
document-storage service, or configured updater endpoint. It does not include
a default telemetry or feedback-upload service. Default application logs are
structured and intended to omit document content, queries, snippets,
credentials, local paths, and network identities. No implementation can protect
a compromised device, screenshots, manual copying, or content that a person
deliberately authorizes for another service.

The desktop interface is a packaged local WebView. It does not make AirWiki a
web service: its documented content-security policy permits its packaged
resources and local IPC, while filesystem, network, and other privileged work
remain in the native application. On Windows, the installer may download and
install Microsoft's WebView2 bootstrapper when that runtime is absent; that
request goes to Microsoft and is governed by Microsoft's terms and privacy
practices. See [installation](docs/install.md#windows-x64).

## When data can leave the device

Each of these choices is independent and opt-in:

| Choice | What may leave | Important boundary |
| --- | --- | --- |
| Visit the project landing, source, or releases on GitHub | A request and normal network metadata to GitHub or its delivery providers | GitHub handles Pages, repository, release-download, and account data under its own policies; the AirWiki project does not receive visitor-level request logs from the static landing |
| Install or prepare local model assets | A download request and normal network metadata to the selected asset host | The current catalog uses third-party model hosts; local models process knowledge on-device after verified assets are available |
| Install required Windows WebView2 | A download request and normal network metadata to Microsoft when the runtime is absent | The installer reports a failed network request without a partial AirWiki install |
| Establish or retain LAN trust | The peer can learn AirWiki's PeerId and, after durable trust, an operating-system-inspected private/on-link endpoint, display name, and OS family; the peer and network can also observe the authenticated connection's IP address and timing | Pairing or trust does not grant any Wiki. The endpoint and display data are discovery/presentation metadata, not authorization |
| Grant a Wiki to a trusted LAN device | Bounded, authorized search evidence and, when opened, that exact published read-only OKF Wiki | The Wiki grant is a separate decision. Original sources, paths, chunks, embeddings, and operational indexes stay local |
| Publish a reviewed Wiki experimentally | Signed, expiring routing metadata to public indexes; authorized evidence and published OKF content to public readers | It never publishes drafts or source folders; public federation is experimental |
| Enable public search for a Library search or connected AI app | That query can reach configured public indexes and selected publishers | This does not publish a local Wiki; public search is off by default |
| Connect a chat or MCP client | Authorized search evidence and, for explicitly authorized memory workflows, the requested memory content | The local bridge is loopback-only, but data returned to a chat client can enter that client's provider or workspace and is subject to its policy |
| Submit feedback or a support report | Whatever the person deliberately writes and attaches in the chosen public form | Public GitHub issues are public; use synthetic material only. Potential security or exposure reports use GitHub's private vulnerability-reporting form |
| Check for updates, once a stable channel exists | Standard request metadata such as IP address, user agent, requested version, and download request | The current technical-beta configuration has no updater endpoint; a stable channel requires separate release and legal review |

AirWiki's local model may propose metadata. It cannot decide to publish, grant
access, or allow data to leave the device. The exact authorization, revocation,
and failure behavior for these paths is in the [threat model](docs/threat-model.md)
and the [search and federation guide](docs/search-and-federation.md).

## Retention, deletion, and requests

Retention and deletion vary by local artifact and operating-system behavior.
SQLite state, managed Wiki files, local models, trust/grant state, and audit
records can remain until the person uses the specific applicable deletion,
recovery, or uninstall flow. Recovery is not a promise to delete every related
artifact. Windows uninstall removes immutable application files but keeps local
data by default. There is no global retention period for local logs or audit
records, and no general cloud-side deletion, export, correction, or recovery
service because AirWiki has no central account or knowledge store.

A public reader who receives a published OKF Wiki or another service that
receives authorized data may retain it under that recipient's controls; AirWiki
cannot delete copies outside the owner's device. Revoking access prevents future
authorized responses but cannot recall data already disclosed.

The project has not published a general retention schedule, dedicated privacy
contact, or jurisdiction-specific rights process for community operations. The
static landing does not collect submissions and does not give the AirWiki
project access to visitor-level request logs. For request metadata held by
GitHub, use GitHub's privacy and account controls. Do not post a request
containing personal, confidential, or security-sensitive material in a public
issue. Before a supported public product release, the project owner and
qualified counsel must determine the applicable operator, contact route,
retention, legal basis, and any access, deletion, correction, objection, or
portability process; see the [release checklist](docs/release-checklist.md).

## Project sites and community channels

The official static project landing is configured for publication at
<https://airwiki.github.io/airwiki/> through GitHub Pages. The source
repository, release downloads, issue forms, and private vulnerability form are
also provided through GitHub. The landing has no AirWiki-operated analytics,
cookies, forms, remote fonts, or third-party scripts. The AirWiki project does
not receive Pages visitor-level request logs. GitHub processes ordinary request
metadata, including IP address and browser information, to operate and secure
the service under its
[General Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement),
including GitHub's own retention and request processes.

When deployed, the landing is an informational technical-beta surface, not a
supported public web application, central knowledge service, account system, or
stable download channel. The local-first desktop boundary does not cover a
visitor's relationship with GitHub or any linked destination. Product Hunt,
Hacker News, Reddit, directories, model hosts, chat providers, and social
channels are separate services governed by their own notices and settings.

Any future host, CDN, form, cookie, analytics provider, or other third party
requires a new service-specific review and notice before use. The public
[support route](SUPPORT.md) accepts synthetic reproductions only; suspected
security or data exposure belongs in the private process in
[SECURITY.md](SECURITY.md).

Do not use public issues for private support material. Their content is public
and governed by GitHub's policies. Use only synthetic reproductions; see
[SUPPORT.md](SUPPORT.md). For suspected security or data exposure, use the
private process in [SECURITY.md](SECURITY.md).

## Read the technical boundaries

The authoritative technical detail is the [threat model](docs/threat-model.md),
including limitations and containment steps. The
[search and federation guide](docs/search-and-federation.md) explains local,
LAN, and public paths, while [chat integrations](docs/chat-integrations.md)
explains the connected-app boundary. For safe feedback and private security
reporting, read [SUPPORT.md](SUPPORT.md) and [SECURITY.md](SECURITY.md). The
project owner must complete the legal and distribution review listed in the
[release checklist](docs/release-checklist.md) before any supported public
release.
