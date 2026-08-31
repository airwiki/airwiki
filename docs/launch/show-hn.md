# Show HN launch

[Show HN](https://news.ycombinator.com/showhn.html) is a discussion format, not
a press release. Read the current [Hacker News guidelines](https://news.ycombinator.com/newsguidelines.html)
and [Show HN guidance](https://news.ycombinator.com/yli.html) immediately before
posting.

A Show HN discussion may be appropriate for a clearly labelled technical beta
when the product is usable by its intended technical audience and the post is
truthful about its unsigned or unnotarized status. Prefer the stable release
for broad promotion. Never present a beta as supported, safe for every
environment, or equivalent to a signed installer.

## Stable-release title and post

### Title

> Show HN: AirWiki – A private, reviewable wiki for local knowledge

### Post

> Hi HN — we made AirWiki, an open-source desktop app for turning folders and
> OKF bundles into a private, reviewable wiki.
>
> The core design choice is to keep the authority boundaries separate. New
> wikis are private by default. A person reviews draft knowledge before it is
> searchable, decides independently whether to share a stable wiki, and decides
> separately which AI apps may read it. Public search is off per connected app
> until enabled.
>
> AirWiki can search local knowledge, authorized nearby devices, and selected
> public sources while keeping those origins visible in the results. It does not
> provide cloud sync, accounts, automatic Git operations, web/mobile clients,
> or automatic publication.
>
> Source: https://github.com/airwiki/airwiki
> Try it: [CONCRETE SELF-SERVICE STABLE ARTIFACT URL — direct GitHub Release
> page or downloadable artifact; insert only after final verification]
> Version: [STABLE VERSION]
> Platforms: [VERIFIED PLATFORM LIST]
>
> I would value direct feedback on whether the review and sharing boundaries are
> understandable, and on the first-value path from a folder to a useful wiki.
> Please use the private security-reporting route for security issues rather
> than posting them here.

## Technical-beta adaptation

Use only if the release owner explicitly chooses a beta discussion and the
channel rules permit it. Replace the stable-release lines with this concise
disclosure and a verified self-service beta artifact marker; do not include a
stable-download marker:

> This is an unsupported technical beta for evaluation. Its macOS candidate is
> ad-hoc signed and not notarized; its Windows MSI candidates are unsigned.
> They are manual downloads, are not selected by the updater, and may be
> blocked by platform policy. Current limits and installation details are in
> the repository's installation guide.
>
> Evaluation artifact: [CONCRETE SELF-SERVICE BETA ARTIFACT URL — verify it is
> live and points to this exact beta immediately before posting]

Keep the source link. Link technical candidates only as clearly labelled test
artifacts, never with wording such as “download AirWiki” or “latest release.”
If there is no live, self-service artifact that a reader can access without an
invitation or credential, do not make a beta Show HN post.

## Before posting

- [ ] Confirm whether this is a stable or technical-beta discussion and use only
  the matching copy.
- [ ] Confirm the exact self-service artifact URL is live, public, and usable
  without an invitation or credential. It must be the stable release for a
  stable post; for a beta post, it must be the exact, visibly labelled beta
  artifact verified immediately before publication.
- [ ] Verify the exact version, platform list, source link, demo, and known
  limits against the released build.
- [ ] Prepare a concise answer to “why not use a normal wiki?” focused on local
  ownership, review before search, and explicit sharing boundaries—not on
  unverified performance or privacy absolutes.
- [ ] Assign a human to participate constructively, including answering hard
  technical questions and declining to handle sensitive support or security
  data in public.
- [ ] Do not coordinate voting, ask for upvotes, use sockpuppets, or repost a
  prior Show HN submission.
