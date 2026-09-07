# Beta roadmap and feedback focus

This roadmap describes the current beta's public outcomes, not delivery dates or a promise to implement every request. Priorities can change when privacy, authorization, recoverability, or validation requires it.

## Delivered in the 0.3 desktop redesign

- A contextual sidebar and a reader with independent navigation scrolling.
- A linked-W Wiki symbol, simpler Library rows and a dedicated review queue.
- An evidence/proposal workspace with compact headings and full-width actions.
- One-row search, clearer Settings and onboarding that fits shorter windows.
- Restoration of the last valid local reading and sidebar preferences.

Everyday journeys were validated in installed development candidates on macOS
and Windows. Release installers have their own acceptance gate; see the
[0.3 candidate notes](docs/releases/0.3.0.md).

## Before a supported public release

1. **Trusted distribution.** Complete legal review, protected signing, notarization, updater custody and verification, and clean-install/upgrade validation on the supported macOS and Windows targets.
2. **A successful first evaluation.** Make the path from a synthetic folder to review, local search, evidence, and recovery clear to nontechnical testers.
3. **Privacy and resilience evidence.** Complete the installed acceptance matrix, two-node synthetic runbook, accessibility paths, and documented failure/recovery behavior.
4. **Community readiness.** Publish a monitored Code of Conduct enforcement contact and complete the owner-led legal and distribution review. These require an accountable human decision and are not satisfied by this file.

The complete release gate remains the [public release checklist](docs/release-checklist.md); this roadmap does not override it.

## Feedback that helps most now

- Whether a new evaluator can create a Wiki from synthetic files, understand draft versus reviewed knowledge, prepare local AI, and find evidence-backed search results.
- Clear, reproducible problems with reading, navigation, search, review, window layout, first-run recovery, update/install behavior, and privacy controls. Keyboard and accessibility regressions remain useful reports.
- Workflows where AirWiki's local-first and human-review boundaries are unclear or surprising.
- Missing ingestion, platform, or integration capability that prevents a safe evaluation, described as an outcome rather than a presumed implementation.

Use the [feedback channels](SUPPORT.md). Do not attach real knowledge, logs, queries, screenshots, credentials, local paths, or network identifiers.

## Deliberately not promised by this beta

The beta does not promise cloud sync, accounts, SSO, a centralized hosted service, web/mobile clients, automatic Git changes, arbitrary remote editing, or silent updates. Any new external service, telemetry, or trust boundary needs an explicit product and threat-model decision before it can be considered.
