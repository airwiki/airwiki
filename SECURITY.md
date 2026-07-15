# Security policy

AirWiki handles private local knowledge. Suspected data exposure, authorization bypasses, unsafe update behavior, signing failures, path traversal, or credential leakage should be reported privately.

## Supported versions

There is no supported public release. Security fixes currently target the latest development revision in [airwiki/airwiki](https://github.com/airwiki/airwiki) only. A monitored public reporting channel, protected release environments, signing identities, notarization, updater verification, and clean platform gates remain public-release blockers.

## Reporting a vulnerability

Do not open a public issue or attach sensitive logs, documents, databases, screenshots, identities, or network addresses to a public discussion. Contact the project owner through the existing private project channel and include only:

- the affected revision and operating system;
- a minimal synthetic reproduction;
- the security property that failed;
- whether data may have been exposed.

Before public launch, the project must configure private vulnerability reporting or publish a monitored security address. Until then, the absence of a public contact is an explicit release blocker.

The project will assess a complete report, prepare a fix and tests, and coordinate disclosure. Never include real knowledge or credentials in a report.

## Operational containment

If exposure is suspected, quit AirWiki completely, revoke affected peer access, disable external-chat permission for affected collections, and disconnect local chat integrations. Preserve only sanitized audit records for investigation. See the [threat model](docs/threat-model.md) for the complete containment sequence.
