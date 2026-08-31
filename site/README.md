# AirWiki launch landing (review-only)

This directory is a static, dependency-free landing page for launch review. It
is deliberately **not** wired to any hosting provider or deployment workflow.
It must not be treated as a public launch until the project has a supported
stable release and its required product, legal, security, and platform gates
have passed.

## Local preview

Open `index.html` directly in a browser, or serve the `site/` directory with a
local static server. The landing keeps its media assets next to the page, so it
is deployable as this directory alone. Its documentation links deliberately use
absolute GitHub `main` URLs. No browser request should be made for analytics,
cookies, web fonts, CDNs, or third-party scripts. Links to the public source
repository and GitHub Releases are intentional user-initiated destinations.

## Release state

`launch-config.js` is the only release-state switch. It is set to
`technical-beta` and intentionally makes **View source** the primary action.
The secondary action leads to technical pre-releases for evaluators and repeats
the unsigned / non-notarized warning.

Do not replace the primary action with a download link until all of the
following are true:

1. The public-release checklist has passed, including signing, notarization,
   updater, and installed-platform evidence.
2. A signed stable release exists at the exact configured URL.
3. A reviewer checks the page copy, links, screenshots, keyboard navigation,
   reduced-motion behavior, and narrow-screen layout against that release.

Then change `releaseState`, `primaryCta`, and `secondaryCta` in
`launch-config.js` in the same reviewable change as the release decision. Do
not infer a stable URL.

## Hosting and deployment checks

Before enabling any manual deployment:

1. Copy or deploy this directory as one static payload, keeping `assets/` next
   to `index.html`.
2. Confirm that public GitHub `main` contains `SUPPORT.md`, `PRIVACY.md`,
   `FAQ.md`, and `ROADMAP.md` before launch. Those landing links depend on
   [PR #80](https://github.com/airwiki/airwiki/pull/80); until it merges, they
   intentionally do not describe a completed public-support surface.
3. Confirm the host does not inject analytics, cookie banners, remote fonts,
   or unreviewed scripts.
4. Configure and verify response headers at the host. At minimum, review a CSP
   equivalent to `default-src 'self'; script-src 'self'; style-src 'self';
   img-src 'self'; media-src 'self'; object-src 'none'; base-uri 'self';
   frame-ancestors 'none'; form-action 'self'`, plus `X-Content-Type-Options:
   nosniff`, a restrictive `Permissions-Policy`, and an appropriate
   `Referrer-Policy`. Test the exact deployed headers; this static page cannot
   set them itself.
5. Do not add a canonical URL, `og:url`, or social-card URL until a hosting
   owner and canonical domain have been explicitly approved. The HTML contains
   a reminder comment rather than an invented URL.
6. Use a manual, SHA-pinned workflow only after a hosting owner and domain have
   been explicitly chosen. No domain, analytics provider, or deployment target
   is assumed by this page.
7. Re-run the static checks below and add a current visual review at desktop,
   tablet, and narrow mobile widths.

## Static checks

From the repository root:

```bash
git diff --check
rg -n 'https?://(?!github\\.com/airwiki/airwiki)' site
rg -n 'prefers-reduced-motion|skip-link|<main|<nav|<footer|alt=|figcaption|transcript' site
```

The second command requires an `rg` build with PCRE2; otherwise inspect the
small allowlist of outbound links manually.
