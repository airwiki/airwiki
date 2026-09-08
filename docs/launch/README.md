# Public launch kit

This directory contains reusable, public-facing launch material for AirWiki. It
is deliberately a preparation kit, not a release approval or a publication
workflow. Nothing here authorizes a post, a listing, a directory submission, or
a claim that a technical pre-release is supported.

AirWiki's public message is **turn your docs into knowledge people and AI can
use**. Lead with concrete workflows: colleagues finding another team's expertise
through private LAN sharing, students publishing their own explanations through
experimental public discovery, and developers keeping portable project memory
for coding assistants. Explain the benefit and the applicable limit together;
these examples are not customer stories or measured results.

AI readiness means structured, portable concepts and relevant evidence retrieved
through MCP, not model training or a token-savings claim. Knowledge stays
on the owning device by default; people decide before sharing it or connecting
it to an AI app, whose provider may process the evidence it receives.
The [README](../../README.md), [privacy and security
boundaries](../../docs/threat-model.md), [installation guide](../../docs/install.md),
and [release process](../../docs/release-process.md) remain the authoritative
product and release documents.

## Launch gates

A supported stable launch, including a Product Hunt listing presented as stable,
requires all of the following. A beta announcement follows the separate scope
below and does not waive the stable-release gates:

- a supported stable release exists, with a verified version, release page,
  signed and notarized macOS installer, and signed Windows installers;
- installed clean-install and upgrade acceptance has passed on the supported
  platforms, with sanitized evidence recorded according to
  [maintainer validation](../maintainer-validation.md);
- the [public release checklist](../release-checklist.md) has no applicable
  unchecked blocker, including a monitored Code of Conduct enforcement contact
  and required legal review;
- the launch owner has reviewed the final, version-specific platform list,
  download URL, known limits, support route, security-reporting route, and
  rollback plan; and
- the selected landing or download destination links to the current
  [privacy notice](../../PRIVACY.md). A project-operated site also has its own
  reviewed service-specific notice; the desktop notice does not describe its
  host, CDN, cookies, server logs, forms, or third-party services; and
- every public claim has been checked against the exact stable build rather than
  a source checkout or technical pre-release.

## Technical-beta announcement

The owner may explicitly authorize a scheduled Product Hunt beta announcement
with direct links to an existing public technical pre-release. The beta label,
platform requirements, signing limits, and support routes must remain visible.
Use verified asset URLs that work without an account or source build. Preview
images from an upcoming version must be identified separately from the version
people can download. Never describe an unsigned beta as a stable, signed, or
notarized release, and never advise disabling platform protections.

The approved informational landing at <https://airwiki.github.io/airwiki/>
may make those direct beta downloads its primary actions. Source, installation
help, release notes, and checksums remain available. This announcement does not
enable the updater or authorize a new release, a signing approval, or a bypass
of required build and installation checks.

## How to use this kit

The [Product Hunt copy](product-hunt.md) includes wording for the current
technical-beta announcement. Saving a draft alone is preparation; scheduling or
publishing requires the owner's explicit instruction. Its accurate beta status
must remain visible until verified release facts change.

1. Complete the preflight in [launch-day-runbook.md](launch-day-runbook.md).
2. Replace every bracketed marker only after the corresponding stable-release
   fact is verified. Do not make up a date, URL, platform, metric, testimonial,
   or availability claim.
3. Select channels from [channel-matrix.md](channel-matrix.md) and re-read each
   channel's current rules immediately before posting.
4. Prepare the needed files from [asset-checklist.md](asset-checklist.md).
5. Use the ready copy in [product-hunt.md](product-hunt.md) or
   [show-hn.md](show-hn.md), preserving the scope and limitations that apply to
   that release.

## Contents

- [Product Hunt copy and checklist](product-hunt.md)
- [Show HN copy and checklist](show-hn.md)
- [Open-source channel matrix](channel-matrix.md)
- [Asset checklist](asset-checklist.md)
- [Launch-day runbook](launch-day-runbook.md)

## Ownership and safety

The release owner authorizes a public launch. The product owner approves the
claim set. A designated human responds to community and Code of Conduct
reports; security reports use the private path in
[SECURITY.md](../../SECURITY.md). Do not put user documents, queries, logs,
credentials, personal contact details, or private test evidence into launch
assets or public replies.
