# Deferred signed-release workflows

The remaining file preserves experimental promotion automation from the pre-baseline
development period. Its nonstandard `.disabled` suffix and location outside
`.github/workflows` prevent GitHub Actions from executing it.

It is a historical reference, not a supported release procedure. Its repository
constants now name [airwiki/airwiki](https://github.com/airwiki/airwiki), but it
still assumes unavailable signing identities and updater credentials that have never
been provisioned for a public project. Do not copy it back into
the active workflow directory without a separate release-engineering review that:

- confirms repository governance and public contacts;
- replaces every repository-specific constant;
- provisions protected signing and updater credentials;
- validates the exact artifacts on both supported development targets; and
- updates the threat model, release checklist, and current ADRs.

The obsolete cargo-packager release candidate workflow was removed during the Tauri v2
migration so this archive cannot be mistaken for a supported build recipe. Active
automation is limited to CI and the manually dispatched unsigned pilot packaging workflow.
