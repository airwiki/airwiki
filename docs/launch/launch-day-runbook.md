# Launch-day runbook

This runbook assumes a supported stable release. It never authorizes someone to
publish on a channel, create an account, accept an OAuth permission, disclose
sensitive data, or submit a listing without the responsible human's immediate
approval.

## Human roles

| Role | Responsibility |
| --- | --- |
| Release owner | Confirms the exact release, gates, checksums/provenance, installer facts, and pause/rollback decision. |
| Product owner | Approves the final claim set, landing/destination URL, platform scope, and channel order. |
| Community responder | Monitors public replies, keeps discussion constructive, routes support safely, and records only minimal public facts. |
| Security responder | Receives reports only through the private route in [SECURITY.md](../../SECURITY.md); never investigates vulnerabilities in public comments. |
| Conduct responder | Uses the monitored enforcement contact in [CODE_OF_CONDUCT.md](../../CODE_OF_CONDUCT.md). Do not launch without a named, monitored contact. |

One person may hold multiple roles only if they can actually staff them during
the launch window. Name the assigned people privately; do not add their personal
details to this repository merely for a launch.

## Preflight

- [ ] The gates in [README.md](README.md#launch-gates) are complete and the
  release owner has reviewed the exact stable tag and GitHub Release.
- [ ] The final installer files, checksums, provenance, signatures, notarization
  results, and supported platform list match the public copy.
- [ ] Clean-install and upgrade acceptance are complete with only sanitized
  evidence, following [maintainer validation](../maintainer-validation.md).
- [ ] The public support, private security-reporting, and Code of Conduct
  enforcement routes have been tested as reachable by their designated humans.
- [ ] The selected landing or download destination links to the current
  [privacy notice](../../PRIVACY.md). If the project operates that site, its
  service-specific notice identifies the operator, host, CDN, third parties,
  data handling, retention, and contact route; do not treat the desktop notice
  as a notice for the site.
- [ ] The known limits in the README and installation guide are visible from the
  chosen destination; no beta artifact is presented as the stable release.
- [ ] The Product Hunt, Show HN, directory, and community text has been checked
  against the current platform rules and approved by the product owner.
- [ ] All selected visual assets pass the [asset checklist](asset-checklist.md).
- [ ] A human has a low-risk reply plan for expected questions and an explicit
  rule not to request sensitive diagnostic material publicly.

## Execution

1. Publish or confirm the stable GitHub Release through the protected release
   process. Verify the public destination in a clean browser session.
2. Freeze the release facts: tag, version, platform list, supported download
   URL, support route, security route, conduct route, and known limits.
3. Post only the approved copy to the first selected channel. Check its visible
   preview and the published result; do not edit the claim set casually after
   publication.
4. Monitor public replies with the assigned community responder. Answer from
   the frozen facts and repository documentation; record a correction if a
   material claim is wrong.
5. Proceed to another channel only after the first channel is stable enough to
   staff. Staggering is preferable to publishing everywhere at once.

## Support, security, and conduct escalation

| Situation | Public response | Private follow-up |
| --- | --- | --- |
| Ordinary product question | Answer briefly with the verified documentation or known limit. | Use the published support route if it needs account/device details. |
| Sensitive diagnostic material | Ask the person not to post it and point to the safe support route. | Do not copy it into an issue, chat, launch spreadsheet, or repository. |
| Suspected vulnerability or exposure | Ask them not to disclose details publicly; link the private vulnerability-reporting route. | Security responder handles it under [SECURITY.md](../../SECURITY.md). |
| Harassment or conduct concern | Do not debate enforcement in the thread. | Conduct responder uses the monitored contact and Code of Conduct process. |
| Incorrect public claim | Correct the claim concisely and update the channel material if possible. | Release and product owners decide whether the issue requires pausing. |

## Pause and rollback

Pause new promotion immediately if a material public claim is inaccurate, a
stable installer/download route fails verification, a vulnerability may affect
users, the support/security/conduct routes are unavailable, or the team cannot
staff replies safely. State only verified facts publicly; do not speculate about
security impact or publish private diagnostics.

For a stable-release defect, follow the [release process](../release-process.md)
and preserve the previous stable artifacts and updater manifest as required.
Do not replace a release asset in place, move a protected tag, or redirect a
stable link to a technical pre-release. Resume promotion only after the release
owner verifies the corrected state and the product owner approves updated copy.

## Metrics without unapproved analytics

Do not add tracking pixels, client telemetry, session recording, link-level
cross-site identifiers, or a new analytics service for launch day. A human may
keep a minimal internal launch log with the date/time, channel, release version,
public post URL, factual corrections made, and aggregate counts displayed by the
channel itself. Do not attach user names, handles, private messages, browsing
history, documents, queries, or diagnostic material.

Review the log after the launch window for actionable, aggregated themes:
first-value confusion, unsupported platform demand, documentation gaps, and
privacy-boundary questions. Treat feature requests and comments as input, not
as an authorization to change product scope or public commitments.
