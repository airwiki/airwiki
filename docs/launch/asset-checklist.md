# Launch asset checklist

All public assets use synthetic content only. Re-check that no image, video, or
caption discloses a document, query, path, identity, credential, network
address, or unsupported product claim.

## Existing repository assets

| Asset | Current repository location | Suitable use after review |
| --- | --- | --- |
| App icon | [`resources/branding/airwiki-app-icon.png`](../../resources/branding/airwiki-app-icon.png) | App/profile identity where a square raster image is accepted. |
| GitHub avatar | [`resources/branding/github-avatar.png`](../../resources/branding/github-avatar.png) | GitHub profile/repository branding. |
| Social preview | [`resources/branding/github-social-preview.png`](../../resources/branding/github-social-preview.png) | Repository/social preview after checking current crop and text. |
| Product tour GIF, MP4, and poster | [`docs/assets/airwiki-demo.gif`](../assets/airwiki-demo.gif), [`airwiki-demo.mp4`](../assets/airwiki-demo.mp4), [`airwiki-demo-poster.png`](../assets/airwiki-demo-poster.png) | Product Hunt gallery, social post, and documentation demo once they match the released build. |
| Review-flow screenshot | [`docs/assets/airwiki-review-flow.png`](../assets/airwiki-review-flow.png) | Gallery image explaining human review before search. |
| AI-app search screenshot | [`docs/assets/airwiki-ai-app-search.png`](../assets/airwiki-ai-app-search.png) | Gallery image explaining per-app public-search consent. |
| Search-origin screenshot | [`docs/assets/airwiki-search-sources.png`](../assets/airwiki-search-sources.png) | Gallery image explaining local, nearby, and public origins. |
| Branding source notes | [`resources/branding/README.md`](../../resources/branding/README.md) | Reuse constraints and source information. |

## Required preflight

- [ ] Confirm every selected asset reflects the exact stable release; re-capture
  it if interface, wording, platform behavior, or trust boundaries changed.
- [ ] Prepare a gallery sequence that tells one story: create a Wiki from a
  folder, review a proposal, search with visible origins, then explain the
  explicit sharing/AI boundary.
- [ ] Produce platform-accurate installer or download imagery only after native
  signing, notarization, and installed acceptance have passed. Do not mock a
  publisher identity, installer dialog, verification badge, or compatibility
  result.
- [ ] Add captions and alt text that describe what is visible without claiming
  that every source is private, every AI integration is local, or every
  platform is supported.
- [ ] Check the live submission UI for current dimensions, file formats,
  duration, accessibility, and ordering requirements. Do not preserve obsolete
  asset specifications in this repository.
- [ ] Verify that the source repository, stable-release URL, version, and
  supported platform list match every caption and call to action.

## Still needed before a stable public launch

- A release-specific gallery export and final cover image, reviewed for privacy
  and accurate platform claims.
- A short, accessible product demo recorded from the supported installed build.
- Stable release notes and a verified download destination.
- A public support route and a monitored Code of Conduct enforcement contact.
- Human-reviewed alt text, captions, and a final claim audit.

Do not collect analytics pixels, third-party session recordings, audience
profiles, or link-level cross-site tracking as part of asset preparation unless
the project makes and documents a separate privacy decision.
