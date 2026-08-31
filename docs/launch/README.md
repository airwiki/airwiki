# Public launch kit

This directory contains reusable, public-facing launch material for AirWiki. It
is deliberately a preparation kit, not a release approval or a publication
workflow. Nothing here authorizes a post, a listing, a directory submission, or
a claim that a technical pre-release is supported.

AirWiki's public promise is intentionally narrow: it is a private, portable,
reviewable wiki built from knowledge that people already have. Knowledge stays
on the owning device by default; people decide before sharing it or connecting
it to an AI app. The [README](../../README.md), [privacy and security
boundaries](../../docs/threat-model.md), [installation guide](../../docs/install.md),
and [release process](../../docs/release-process.md) remain the authoritative
product and release documents.

## Launch gates

Do not replace these gates with a launch date, a waitlist, a pre-order, or a
marketing exception. Product Hunt and stable-directory copy may be published
only after all of the following are true:

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
- every public claim has been checked against the exact stable build rather than
  a source checkout or technical pre-release.

An openly labelled technical beta can be discussed only in channels that allow
it, with its current trust limits stated plainly. It is never a substitute for
the gates above, never a stable-download call to action, and never an updater
channel.

## How to use this kit

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
