# Product Hunt launch

This copy describes the current technical beta and can be saved in the existing
Product Hunt draft. Saving a draft does not schedule or publish a launch, approve
a release, or change the [launch gates](README.md#launch-gates). The repository's
stable-launch policy requires a supported stable release. The owner has explicitly
authorized a technical-beta announcement for September 9, 2026, with refreshed
branding, media, and direct downloads of the latest **0.3 beta**. The exact
packages are published in [v0.3.0-rc.1](https://github.com/airwiki/airwiki/releases/tag/v0.3.0-rc.1):
macOS is signed and notarized, and Windows remains an unsigned technical beta.
Use those assets and their documented installation status; do not substitute
older 0.2 downloads for the launch.

Product Hunt's [official guidance on unreleased products](https://help.producthunt.com/en/articles/484932-can-i-submit-an-unreleased-product)
prioritizes products people can try and leaves pre-launch inclusion to its
discretion. It does not impose AirWiki's internal stable-release policy. Re-check
the [launch guidance](https://www.producthunt.com/launch/preparing-for-launch)
and live submission interface before submitting; the live form takes precedence
for fields and media requirements.

## Message

Lead with reusable knowledge for people and AI. Show three concrete situations:
finding another team's expertise, sharing a study explanation, and carrying
repository decisions into the next AI session. These are illustrative workflows,
not testimonials, measured outcomes, or production deployments. Explain AI
readiness as structured concepts and relevant evidence retrieved through MCP;
do not claim model training, token savings, or improved answer accuracy.
Name Open Knowledge Format (OKF) as the open standard introduced by Google Cloud.
Explain AirWiki's independent OKF v0.2 implementation through readable files,
portability, and traceable context; adoption is not certification or endorsement.
Keep the attribution aligned with the [landing's sources](../../site/README.md#editorial-focus)
and the [OKF compatibility profile](../okf-v02-profile.md).

## Ready copy

### Tagline

> Turn your docs into knowledge people and AI can use

### Description

> Turn team docs, study notes, and repository decisions into knowledge people and AI can use. Built on Open Knowledge Format (OKF), the open standard introduced by Google Cloud. Share reviewed wikis with authorized colleagues on a private LAN, publish your own notes to an experimental public network, or give coding assistants project memory via MCP. Open source, private by default, with separate sharing and AI controls. Desktop technical beta for macOS and Windows; no supported stable release yet.

### Links

- Main destination: <https://airwiki.github.io/airwiki/>
- Source: <https://github.com/airwiki/airwiki>
- Evaluation builds, clearly labelled technical pre-releases:
  <https://github.com/airwiki/airwiki/releases/tag/v0.3.0-rc.1>

Use **AirWiki** as the name. The current form allows 60 characters for the tagline
and 500 for the description; paste each quoted field as one paragraph. Suggested
topics, if available in the form: Open Source, Productivity, and Developer Tools.

### Maker comment

> Hi Product Hunt! We're building AirWiki because useful knowledge often gets
> stuck in a folder or an old conversation. The next person, or their AI
> assistant, has to start from scratch to find the same explanation.
>
> AirWiki turns that material into searchable wikis. Here are three workflows
> we're building it for:
>
> **Teams:** support needs to know how engineering recovers a failed import.
> Engineering shares a reviewed troubleshooting Wiki with verified colleagues
> on the local network. They can find the explanation themselves; connected
> assistants can retrieve evidence when the source owner permits that AI access.
>
> **Students:** your notes finally make database normalization click. Turn them
> into a reviewed Wiki and choose to publish it to the experimental public
> network, so classmates and other students studying the topic can discover it.
> Share only material you have the right to publish. Publishers must be reachable,
> and public readers can retain what you share.
>
> **Developers:** keep architecture decisions and project conventions in a
> portable `.airwiki` Wiki beside your code. Authorized tools such as Codex,
> Claude Code, and Gemini CLI can consult it in later tasks, so you spend less
> effort rebuilding context in every conversation. You review the files and
> decide what enters Git; AirWiki never commits or pushes.
>
> For AI tools, the value is context they can look up: structured concepts,
> relationships, and relevant passages with source references through MCP.
> Preparation and retrieval run locally. A connected cloud AI provider may
> process evidence its app receives; sharing and AI access are separate choices.
>
> **Built on an open standard:** AirWiki uses Open Knowledge Format (OKF),
> introduced by Google Cloud. Our independent OKF v0.2 implementation keeps
> knowledge in readable Markdown files with structured metadata for provenance,
> verification, and freshness. You can inspect the files, review changes in Git,
> and export your Wiki for reuse with compatible tools.
> Read the open specification: https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md
>
> **Current stage: open-source desktop technical beta, not a supported stable
> release.** The 0.3 beta is available for Apple silicon Macs and Windows x64
> as v0.3.0-rc.1. The Mac installer is signed and notarized; Windows installers
> are unsigned. Downloads need no account, and updates are manual. The gallery
> shows this version's interface with synthetic data.
> Public federation is experimental, with no supported always-on relay service.
> Private sharing uses LAN devices; there is no SSO or cloud sync.
>
> We'd love developers, students, and people who manage team knowledge to try
> one workflow with sample material and help shape it. What context do you keep
> having to explain again, and where would a shared Wiki help? Feedback,
> documentation, and code contributions are welcome.
>
> Explore AirWiki: https://airwiki.github.io/airwiki/
> Download the 0.3 beta: https://airwiki.github.io/airwiki/#download
> Feedback: https://github.com/airwiki/airwiki/blob/main/SUPPORT.md
> Report security concerns privately: https://github.com/airwiki/airwiki/security/advisories/new

Keep this as prepared copy if the edit form does not expose a maker-comment
field. Do not post it into a public discussion just to save it. Keep the package
version, direct-download destination and platform signing/notarization facts
aligned with the exact published release before scheduling.

## Gallery story

The September 2026 beta announcement uses the current W essential app icon and
four unmodified synthetic screenshots: reader, AI apps, search sources, and
review flow. They show the 0.3 interface. Confirm that the launch packages
contain that interface before scheduling; older 0.2 packages are not a substitute.

Use the [existing synthetic product images](asset-checklist.md#existing-repository-assets).
Order the gallery around the value before explaining the controls. Captions must
describe the actual image; these headlines are editorial framing, not claims
that a screenshot proves a live sharing session:

| Frame | Headline | Image and supporting caption |
| --- | --- | --- |
| Cover | Knowledge for people and AI | Reader: reviewed concepts in a portable Wiki. |
| Teams | Find the expertise another team already has | Search origins: local, authorized nearby, and public results remain distinguishable. |
| Students | Let your explanation help the next class | Reader: an example study Wiki; public discovery is experimental and opt-in. Use a new synthetic study capture if the current image does not show study content. |
| Developers | Carry project context into the next AI session | AI-app access: compatible assistants retrieve permitted knowledge through MCP. |
| Control | Review it. Choose who can use it. | Review flow: compare source evidence with a proposal before approving it. |

Do not label the existing ten-second interface montage as an end-to-end demo of
these scenarios. The live form currently accepts YouTube or Loom video links;
do not upload or publish a video on another service merely to fill that field.

## Submission checklist

- [ ] The launch owner has settled the launch scope. A stable launch requires
  the gates in [the launch index](README.md#launch-gates); preparing or saving
  the current beta draft does not satisfy or waive them.
- [ ] The maker profile is the individual human who will monitor replies, not a
  company profile, shared credential, or invented persona. No account, OAuth
  permission, or scheduled launch has been created as part of this document.
- [ ] The title, tagline, description, gallery, demo, and maker comment reflect
  the exact candidate or release, including platform and experimental limits.
- [ ] The destination gives readers an accurate next step. The beta landing
  points directly to the verified 0.3 installers and source; a stable-download
  CTA requires a separate verified stable release URL.
- [ ] The listing names material limits: desktop-only support, opt-in sharing,
  and the distinction between local, nearby, and public knowledge where
  relevant.
- [ ] The listing does not promise cloud sync, accounts, mobile/web access,
  automatic publication, silent updates, telemetry-free status beyond the
  documented product behavior, future features, testimonials, or performance
  figures that have not been verified for this release.
- [ ] A human is scheduled to monitor comments, redirect security reports to
  the private route, and enforce the Code of Conduct using the published
  contact.
- [ ] The owner has checked the live Product Hunt fields and preview; no action
  is submitted without their final confirmation.
- [ ] Nobody asks for, exchanges, or coordinates votes/upvotes, incentives, or
  engagement rings. The launch invites honest product feedback only.

## Reply boundaries

Answer product questions with links to the versioned repository documents. Do
not request or accept private documents, diagnostic logs, credentials, network
details, or security vulnerabilities in a comment. Do not diagnose an
individual installation in public; move only non-sensitive support to the
published support route once one exists. Keep feature requests as feedback, not
commitments.
