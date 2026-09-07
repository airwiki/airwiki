# Windows 0.3.0 normal package status

Status: packaging completed successfully (exit code 0); normal installation
subsequently upgraded successfully. See the [installed acceptance report](windows-030-installed-acceptance.md).

- Candidate commit: `da71ce704d5bc44c5985e1be5c80dc0999f10250`.
- Candidate tree: `c255c355fb82fd483b62aa156db2ead50fc35c08`.
- Declared application version: 0.3.0.
- A new clean detached checkout was used with the versioned Windows
  packaging script, pinned portable Node and frozen frontend dependencies.
- This was the normal release build, without the E2E feature. Existing QA
  executables and the verified state backup were preserved; the normal
  installation was subsequently upgraded as recorded in the acceptance report.
- The pinned runtime source build and its reproducibility phase completed,
  followed by normal application release compilation and both localized MSI
  packages. The versioned packager completed its MSI policy and extracted
  payload checks, including the runtime, bridge, helper and integration payload.
- Windows Installer read-only database inspection confirmed ProductVersion
  0.3.0 in both final MSI files. Each file contains 35,676,160 bytes.
- The packaging run was not restarted. The candidate checkout remained clean
  at the same commit and tree after packaging.
- These local candidates are for the authorized normal Windows update. Public
  GitHub assets remain the responsibility of the protected workflow from main.

Installation proceeded only after confirmation and local verification of exact
tree equivalence with main `e574299b8586f33e37871cd80bcc578cd75f1ceb`.
No platform protection or permission was changed, and no private log or backup
is included in this report.

## Verified local artifacts

`AirWiki_0.3.0_x64_en-US.msi` SHA-256:

```text
9fb61e290ca8e74eb9830f423d179c894772124582d652f09edaa7dd1972ad20
```

`AirWiki_0.3.0_x64_es-ES.msi` SHA-256:

```text
ebc6b87fafd829a62f28f0c1a0a21d718a210770a5b158680fb96034b665ddcf
```

Digests were calculated from the completed local package outputs. These files
are retained locally; they have not been uploaded as public release assets.
The separate installed acceptance report records the subsequent normal update.
