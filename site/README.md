# AirWiki launch landing (private preview)

This directory contains a static landing page plus the smallest Sites build
wrapper needed for an owner-only deployment. The page itself has no runtime
application dependencies. A private deployment is a review artifact, **not** a
public launch; public access remains blocked until the project has a supported
stable release and its required product, legal, security, and platform gates
have passed.

## Local preview

Run `pnpm dev`, or build and preview the Cloudflare-compatible artifact with
`pnpm build && pnpm preview`. The landing keeps its media assets next to the
page. Its documentation links
deliberately use absolute GitHub `main` URLs. No browser request should be made
for analytics, cookies, web fonts, CDNs, or third-party scripts. Links to the
public source repository and GitHub Releases are intentional user-initiated
destinations.

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

The Sites configuration is intentionally owner-only. Do not change its access
policy, add a custom domain, or make it public without an explicit launch
decision. Before enabling public access:

1. Install from `pnpm-lock.yaml`, run `pnpm check`, and package the resulting
   `dist/` directory from the exact committed source SHA.
2. Confirm that public GitHub `main` still contains `SUPPORT.md`, `PRIVACY.md`,
   `FAQ.md`, and `ROADMAP.md` before launch. Those landing targets are present
   today and must remain valid at the release commit.
3. Confirm the host does not inject analytics, cookie banners, remote fonts,
   or unreviewed scripts.
4. Verify the deployed response headers. `public/_headers` defines a CSP
   equivalent to `default-src 'self'; script-src 'self'; style-src 'self';
   img-src 'self'; media-src 'self'; object-src 'none'; base-uri 'self';
   frame-ancestors 'none'; form-action 'self'`, plus `X-Content-Type-Options:
   nosniff`, a restrictive `Permissions-Policy`, and an appropriate
   `Referrer-Policy`. Test the exact deployed headers rather than relying only
   on the source declaration.
   The production build deliberately removes the static client `index.html`
   and generated active CSS/JavaScript. The Worker serves `/`, the referenced
   CSS/JavaScript, and virtual media routes with the same seven headers; the
   binary files remain as unreferenced backing assets for `ASSETS.fetch`.
   The Worker also supplies a bounded single-range fallback for the small demo
   video when a local or hosted asset layer ignores `Range`; verify playback and
   seeking on the deployed version.
   This routing is a workaround for private Sites previews that bypass the
   Worker for existing static files, so verify `/`, every referenced virtual
   route, a 404, and a disallowed method on the exact deployment. If the host
   bypasses any referenced route, or policy requires protected headers on the
   unreferenced backing-object URLs too, use a host that guarantees Worker-first
   routing instead of treating this workaround as a public-launch control.

   The HTML also carries an early CSP meta element and a referrer meta element
   as limited defense in depth when a private preview layer omits response
   headers. Those elements do not implement `frame-ancestors`, COOP, CORP,
   `Permissions-Policy`, `X-Content-Type-Options`, or `X-Frame-Options`,
   and never satisfy this production-header gate.
5. Do not add a canonical URL, `og:url`, or social-card URL until a hosting
   owner and canonical domain have been explicitly approved. The HTML contains
   a reminder comment rather than an invented URL.
6. Keep deployments bound to a committed source SHA. No domain or analytics
   provider is assumed by this page.
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
