# Windows 0.3.0 normal installed acceptance

Result: **PASS** for the authorized normal-installation upgrade and brief
interactive startup/state-preservation smoke. No public asset was uploaded by
this local task; GitHub distribution remains a separate protected workflow.

Related records: [upgrade preparation](windows-normal-upgrade-preparation.md)
and [verified local packages](windows-030-package-status.md).

## Source and artifact identity

- Built source: `da71ce704d5bc44c5985e1be5c80dc0999f10250`.
- Merged main: `e574299b8586f33e37871cd80bcc578cd75f1ceb`.
- Both trees were locally verified as
  `c255c355fb82fd483b62aa156db2ead50fc35c08` before installation.
- The versioned normal Windows packager completed successfully, without the
  E2E feature, including both localized MSI payload checks.
- Windows Installer database inspection confirmed 0.3.0 in both MSI files.

English MSI `AirWiki_0.3.0_x64_en-US.msi` SHA-256:

```text
9fb61e290ca8e74eb9830f423d179c894772124582d652f09edaa7dd1972ad20
```

Spanish MSI `AirWiki_0.3.0_x64_es-ES.msi` SHA-256, used for the update:

```text
ebc6b87fafd829a62f28f0c1a0a21d718a210770a5b158680fb96034b665ddcf
```

Installed `airwiki.exe` SHA-256:

```text
1a3f8d4968a243c7f05e09f3cefde4d050c5d08e7141d73f0f6da184cd4020ba
```

The installed executable digest was verified against a read-only administrative
extraction of the exact Spanish MSI and matched. Both registered MSI version
and installed executable product/file versions are **0.3.0**.

## Upgrade and installed interaction

The previous normal MSI installation was 0.2.0. Immediately before updating,
no AirWiki process was running and the entire prepared data/configuration
snapshot was rechecked against its sources with no differences. The verified
backup remains local and preserved.

The normal per-user Windows Installer update completed with exit code 0, with
no reboot requested and no protection bypass. The normal installed executable
was then launched in the interactive Windows desktop. Its normal main window,
Library navigation and Settings were available; no onboarding reset or blocking
dialog was present. The application was left open for the user.

## State preservation

SQLite `quick_check` returned `ok` after startup. Counts and pre-existing
persisted values were compared locally for collections, source documents,
concepts, peers, grants, application capabilities, application Wiki grants and
publication claims. Existing content and permission values were preserved.
The configuration file was byte-identical to its pre-upgrade backup.

Schema-aware comparison accounted for two new migration columns: collection
memory scope was added with null values, and receiver AI consent version was
added with its default zero. These were not new grants or consent approvals.
Source-document update timestamps changed during startup; all other previously
present source-document fields matched the backup. No identities or row content
were exported in this report.

No source folder, Wiki permission, integration setting, model license or grant
was changed by the validation actions. No model download was requested. The
original checkouts, QA installations, synthetic profiles and backups were
preserved. Local main was free and was synchronized to the verified merged main
without switching or resetting any checkout.

## Scope

This is the brief normal-installed update smoke requested after the earlier
redesign acceptance. It does not claim a rerun of the earlier visual matrix,
Narrator checks, LAN permission boundary, public signing or release workflow.
No installation blocker remains within this task's authorized scope.
