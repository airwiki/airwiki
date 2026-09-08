# AirWiki launch landing

This directory contains the official static landing for AirWiki. The page is a
public, informational technical-beta surface: it introduces the upcoming 0.3
beta and links to its source, but it is not a supported web
application, stable download channel, account service, or release approval.

The source intentionally produces two separate artifacts:

- `dist/` is the Worker-backed, owner-only preview used to review response
  headers and Worker routing before a future supported release.
- `dist-pages/` is the static artifact configured for deployment at
  <https://airwiki.github.io/airwiki/> by GitHub Actions from `main`.

Neither artifact changes the desktop product's release state.

## Editorial focus

The hero and first section explain reusable knowledge for people and AI through
three illustrative workflows: private knowledge sharing across teams on a LAN,
student explanations in experimental public discovery, and portable repository
memory for coding assistants. Each pairs a concrete question with the benefit
and current limits. Keep these aligned with the root README and
[`docs/launch/product-hunt.md`](../docs/launch/product-hunt.md).

Explain AI readiness through structured concepts, provenance, and relevant
evidence retrieved through MCP. Do not invent customer stories, token savings,
answer-quality measurements, or always-on network availability. The final call
to action invites sample-data evaluation, feedback, and contributions.

Make the open foundation visible in the hero, product facts, and OKF explanation.
Attribute Open Knowledge Format to its [Google Cloud introduction](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing/)
and link the [open specification](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md).
Explain readable files, portability, and provenance using the supported
[AirWiki OKF v0.2 profile](../docs/okf-v02-profile.md). AirWiki is an independent
implementation; adopting the format is not Google certification or endorsement.

## Local preview

Run `pnpm dev` for the source page. Run `pnpm build && pnpm preview` to inspect
the Cloudflare-compatible private-preview artifact. The Worker bundles the four
small media assets from `src/assets/`, so its production package contains no
static client objects that can bypass response headers. Run `pnpm pages:build`
to create the separate static GitHub Pages artifact in `dist-pages/`; that
builder copies the allowlisted assets and converts only the Worker-protected
local references to project-relative paths.

The documentation links deliberately use absolute GitHub `main` URLs. No
browser request is made for analytics, cookies, web fonts, CDNs, forms, or
third-party scripts. Links to GitHub source, documentation, Releases, and
privacy information are intentional visitor-initiated destinations.
The Google Cloud announcement is also an intentional outbound link, included
in the Pages verifier's origin allowlist to substantiate the OKF attribution.

## Release state

`launch-config.js` keeps the `technical-beta` release state. The launch targets
**0.3 beta**, including the current interface shown in the gallery and tour.
Until its packages are published, the hero points to the platform information
and source; it must not substitute an older 0.2 installer for this launch.

The owner has authorized direct beta downloads and a beta announcement; neither
promotes the packages to stable. Once the exact 0.3 installers are published,
replace the informational CTAs with direct macOS and Windows downloads. Add the
Spanish Windows installer, requirements, release notes, checksums, installation
help, and each package's verified signing/notarization status. Do not infer the
status or filename from another release.

Keep configuration, HTML fallbacks, README links, and the installation guide
aligned with the exact public release assets. Verify those assets before
changing their URLs; links must also work without JavaScript. Do not select a
platform automatically or require an account, signup, or intermediary page.

Do not label any download as a supported stable release until all of the
following are true:

1. The public-release checklist has passed, including signing, notarization,
   updater, and installed-platform evidence.
2. A signed stable release exists at the exact configured URL.
3. A reviewer checks the page copy, links, screenshots, keyboard navigation,
   reduced-motion behavior, and narrow-screen layout against that release.

Then change `releaseState`, `primaryCta`, and `secondaryCta` in
`launch-config.js` in the same reviewable change as the release decision. Do
not infer a stable URL.

## Current product media

The header, footer, favicon and social preview use the approved **W essential**
identity. Regenerate them with the shared
[branding workflow](../resources/branding/README.md); keep the original linked-W
geometry consistent with the desktop app.

The tour and review image show the 0.3 desktop redesign using committed synthetic
macOS acceptance captures from `apps/desktop/ui/e2e/baselines/darwin/`:
`en-light-library-1180x728.png`, `en-light-reader-1180x728.png`,
`en-light-review-comparison-1180x728.png` and
`en-light-settings-1180x728.png`. The poster uses the reader capture. The ten-second
video is a montage of these four states, not a recording of a continuous task or
proof of model output. Update the transcript when replacing its scenes.

The documentation's search-origin and AI-app gallery images are captured from
the current `apps/desktop/ui/visual.html` development fixture, with synthetic
data (`media=1`, English, dark appearance). These two gallery images demonstrate
UI states; they are not native acceptance evidence or real search results.

## GitHub Pages boundary

GitHub Pages is suitable for this read-only technical-beta landing because the
page has no sensitive interaction or project-operated request logging. It does
not provide per-site response-header configuration. The HTML therefore keeps a
defense-in-depth CSP meta element and a `no-referrer` meta element, but these
cannot implement `frame-ancestors`, COOP, CORP, `Permissions-Policy`,
`X-Content-Type-Options`, or `X-Frame-Options`.

That residual limitation is accepted only for this static informational page.
Do not add authentication, forms, visitor-specific state, embedded remote
content, analytics, or a stable download trust boundary to the Pages artifact.
A supported release site that needs those controls must use a host with
reviewed response-header and routing guarantees.

The visible page identifies GitHub Pages as the host and links to GitHub's
privacy statement. The AirWiki project receives no visitor-level request logs,
while GitHub processes ordinary request metadata under its own policy. See the
repository [privacy notice](../PRIVACY.md) for the complete boundary.

## Deployment

`.github/workflows/pages.yml` is the only publication path. It runs for the
official `airwiki/airwiki` repository at `main`, builds from the exact committed
SHA, uploads an allowlisted static artifact, and deploys through the
`github-pages` environment. Pull requests run the same static checks through
the ordinary CI workflow without publishing.

The repository owner must first enable GitHub Pages with **GitHub Actions** as
its source. `actions/configure-pages` verifies that setting but deliberately
cannot create or change it with the workflow token; merge only after this
one-time repository setting is present.

Before enabling or changing the deployment:

1. Confirm that public GitHub `main` still contains `SUPPORT.md`, `PRIVACY.md`,
   `FAQ.md`, and `ROADMAP.md`.
2. Run `pnpm pages:check`; it also rebuilds and verifies the Worker artifact.
3. Confirm the output contains only the allowlisted page files, media, social
   image. The Actions artifact is deployed directly and does not use Jekyll.
4. Keep the repository Pages source set to **GitHub Actions**. Do not publish a
   branch directory or a generated artifact committed to source control.
5. Verify the exact deployed URL, internal assets, outbound links, video
   playback, social metadata, keyboard navigation, reduced-motion behavior,
   and desktop/tablet/mobile layouts.
6. Keep the Sites configuration owner-only unless a separate reviewed hosting
   decision changes it.

The canonical and social-card URLs are intentionally fixed to the approved
project-site deployment target. A custom domain is not required and must not be
introduced without a separate ownership and DNS decision.

## Worker preview checks

The Worker applies a CSP equivalent to `default-src 'self'; script-src 'self';
style-src 'self'; img-src 'self'; media-src 'self'; object-src 'none'; base-uri
'self'; frame-ancestors 'none'; form-action 'self'`, plus
`X-Content-Type-Options: nosniff`, a restrictive `Permissions-Policy`, and
`Referrer-Policy: no-referrer`. `public/_headers` mirrors that policy for local
host compatibility, but the production build removes every static client file
and leaves only the empty binding directory because Sites can bypass the Worker
for existing static files.

The Worker serves `/`, the referenced CSS/JavaScript, and bundled virtual media
routes with the same seven headers. Media is decoded lazily on first use rather
than during isolate startup, and the build fails if the gzipped Worker exceeds
the repository's conservative 2,400,000-byte budget. It also supplies bounded
single-range responses for the small demo video.

Test `/`, every referenced virtual route, a 404, a disallowed method, video
seeking, and the former `/assets/*`, `/_headers`, and `/.assetsignore` paths on
the exact private deployment. Those former backing-object paths must return the
protected Worker 404; any `200` is a blocker for that hosting path. If a host
bypasses any referenced route, use a host that guarantees Worker-first routing.

## Static checks

From the repository root:

```bash
git diff --check
pnpm --dir site pages:check
rg -n 'prefers-reduced-motion|skip-link|<main|<nav|<footer|alt=|figcaption|transcript' site
```

The Pages verifier enforces the small outbound-origin allowlist and resolves
every local HTML asset against the `/airwiki/` project-site base path.
