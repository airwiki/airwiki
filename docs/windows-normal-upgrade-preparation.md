# Windows normal-installation upgrade preparation

Historical preparation record for the normal 0.2.0 → 0.3.0 update. The update
subsequently completed; see the [installed acceptance report](windows-030-installed-acceptance.md)
and [verified package record](windows-030-package-status.md).

At the time of this preparation, the final source commit and normal installer
were pending. No installer had been executed and no new QA/E2E candidate had
been built during this preparation step.

## Observed state

- The normal installation is registered with Windows Installer as AirWiki 0.2.0.
  Its executable reports product and file version 0.2.0.
- No AirWiki process was running during the preparation or after backup.
- The original checkout and the Windows validation checkout were clean.
  Their inspected commits were `326687b9e38af782ffc7d48885cedf69b5151b19`
  and `26353476c68de067980cfbf5dd709ee982d6b22f`, respectively.
- The remote main reference was verified as
  `4f4bdac59a72d7402507e334ea6f822a58861f2e`. No checkout was reset or switched.
- The separately installed QA copies and synthetic profiles were preserved.

## Local backup

A separate local backup of the normal application's local data and roaming
configuration was created while the application was stopped. All 55 copied
files were verified against their sources using SHA-256. The inspected state
roots contained no reparse points and sufficient free disk space was available.
The backup manifest and its location remain local and are not published.
No knowledge-source folder, sharing permission, integration, model license, or
application configuration was modified.

This is a preparation snapshot, not an assertion about state at a future
installation time. If the user runs the application before the final installer
arrives, perform coordinated shutdown and create a new verified snapshot of
the resulting state immediately before upgrading. Preserve this snapshot too.

## Prepared update sequence

1. Receive the final source commit and normal Windows installer. Confirm its
   version, architecture, MSI identity and supplied digest; do not substitute
   an E2E executable or an earlier development copy.
2. Recheck the normal application's process state. If running, use its normal
   Quit completely action and confirm shutdown before invoking the installer.
   Refresh the local backup if state has changed since preparation.
3. Use the normal per-user upgrade procedure, retaining application data.
   Preserve the prior installation identity and available rollback artifact;
   do not run the destructive install/uninstall smoke script against this
   user's normal installation.
4. Verify the installed executable and registered version match the delivered
   candidate, then open the normal application in the interactive desktop.
5. Check startup succeeds and existing configuration and knowledge state are
   retained, comparing locally without publishing content or identities.
   Observe the existing reading destination where applicable; do not publish,
   pair, grant access, change integrations, or accept model licenses.
6. Record only sanitized results, final source identity, installer digest and
   installed executable digest. If the upgrade fails, preserve both the
   resulting state and the backup for recovery. A binary downgrade alone is
   not assumed safe after a database migration.

The observations above describe the pre-upgrade state only. Version 0.3.0 was
the announced target at that point. The linked acceptance report records the
subsequent installation; this preparation record does not establish public
release availability.
