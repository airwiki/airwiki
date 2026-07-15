# Deferred signed-release workflows

These files preserve experimental signed-release automation from the pre-baseline
development period. Their nonstandard `.disabled` suffix and location outside
`.github/workflows` prevent GitHub Actions from executing them.

They are historical references, not supported release procedures. Their repository
constants now name [airwiki/airwiki](https://github.com/airwiki/airwiki), but they
still assume unavailable signing identities and updater credentials that have never
been provisioned for a public project. Do not copy them back into
the active workflow directory without a separate release-engineering review that:

- confirms repository governance and public contacts;
- replaces every repository-specific constant;
- provisions protected signing and updater credentials;
- validates the exact artifacts on both supported development targets; and
- updates the threat model, release checklist, and current ADRs.

Active automation is limited to CI and the manually dispatched unsigned pilot
packaging workflow.
