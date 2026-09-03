#!/usr/bin/env python3
"""Prepare a deliberately non-stable macOS RC and unsigned Windows beta set."""

from __future__ import annotations

import argparse
import json
import re
import shutil
import tempfile
from argparse import Namespace
from pathlib import Path
from typing import cast

import technical_prerelease as technical


REPOSITORY = technical.REPOSITORY
RC_NUMBER = re.compile(r"^[1-9][0-9]{0,3}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
RECEIPT_SCHEMA = "airwiki-macos-platform-rc-verification-v1"
RECEIPT_VERIFICATION = "release-macos-and-verify-macos-release-completed"
RELEASE_WORKFLOW = ".github/workflows/package-platform-rc.yml"
MACOS_RECEIPT_NAME = "MACOS-VERIFICATION.json"
NOTICE_NAME = "PLATFORM-RELEASE-CANDIDATE.txt"


def release_tag(version: str, rc_number: str) -> str:
    return f"v{version}-rc.{rc_number}"


def primary_asset_names(version: str) -> dict[str, str]:
    return {
        "macos": f"AirWiki_{version}_aarch64_SIGNED-NOTARIZED-RC.dmg",
        "windows_en": f"AirWiki_{version}_x64_en-US_UNSIGNED-BETA.msi",
        "windows_es": f"AirWiki_{version}_x64_es-ES_UNSIGNED-BETA.msi",
    }


def release_asset_names(version: str) -> set[str]:
    return {
        *primary_asset_names(version).values(),
        *technical.LEGAL_PAYLOADS.keys(),
        MACOS_RECEIPT_NAME,
        "PROVENANCE.json",
        "SHA256SUMS.txt",
        NOTICE_NAME,
    }


def validate_metadata(
    version: str,
    rc_number: str,
    commit: str,
    repository: str,
    workflow_run: str,
    generated_at: str,
) -> None:
    if RC_NUMBER.fullmatch(rc_number) is None:
        raise ValueError("RC number must be an integer from 1 to 9999")
    technical.validate_metadata(
        version, rc_number, commit, repository, workflow_run, generated_at
    )


def receipt_artifacts(
    receipt: dict[str, object], version: str, commit: str, dmg_digest: str
) -> list[dict[str, str]]:
    expected_top_level = {"schema", "commit", "version", "verification", "artifacts"}
    if set(receipt) != expected_top_level:
        raise ValueError("macOS verification receipt has an unexpected schema")
    if (
        receipt["schema"] != RECEIPT_SCHEMA
        or receipt["commit"] != commit
        or receipt["version"] != version
        or receipt["verification"] != RECEIPT_VERIFICATION
    ):
        raise ValueError("macOS verification receipt does not match the release")
    artifacts = receipt["artifacts"]
    if not isinstance(artifacts, list) or len(artifacts) != 1:
        raise ValueError("macOS verification receipt has an invalid artifact list")
    expected_name = f"AirWiki_{version}_aarch64.dmg"
    normalized: list[dict[str, str]] = []
    for artifact in artifacts:
        if not isinstance(artifact, dict) or set(artifact) != {"name", "sha256"}:
            raise ValueError("macOS verification receipt has an invalid artifact")
        name = artifact.get("name")
        digest = artifact.get("sha256")
        if not isinstance(name, str) or not isinstance(digest, str):
            raise ValueError("macOS verification receipt artifact is incomplete")
        if name != expected_name or SHA256.fullmatch(digest) is None:
            raise ValueError("macOS verification receipt artifact is invalid")
        normalized.append({"name": name, "sha256": digest})
    if normalized[0]["sha256"] != dmg_digest:
        raise ValueError("macOS verification receipt DMG digest does not match")
    return normalized


def read_receipt(path: Path, version: str, commit: str, dmg_digest: str) -> dict[str, object]:
    return {
        "schema": RECEIPT_SCHEMA,
        "commit": commit,
        "version": version,
        "verification": RECEIPT_VERIFICATION,
        "artifacts": receipt_artifacts(technical.read_json(path), version, commit, dmg_digest),
    }


def expected_platforms(version: str) -> dict[str, object]:
    names = primary_asset_names(version)
    return {
        "macos-arm64": {
            "asset": names["macos"],
            "desktopApplication": True,
            "releaseMaturity": "release-candidate",
            "nativeSignature": "developer-id",
            "notarized": True,
        },
        "windows-x64-en-US": {
            "asset": names["windows_en"],
            "desktopApplication": True,
            "releaseMaturity": "technical-beta",
            "authenticode": "not-signed",
        },
        "windows-x64-es-ES": {
            "asset": names["windows_es"],
            "desktopApplication": True,
            "releaseMaturity": "technical-beta",
            "authenticode": "not-signed",
        },
    }


def notice(version: str, rc_number: str) -> str:
    tag = release_tag(version, rc_number)
    return f"""AIRWIKI {tag} — PLATFORM RELEASE CANDIDATE, NOT STABLE

This public candidate is never Latest and is never selected by the AirWiki
updater. It does not establish a supported public release channel.

- macOS (Apple silicon, macOS 13+): this DMG is Developer ID signed and
  notarized by Apple, but it is a release candidate, not a stable release.
- Windows (x64): these MSI files are unsigned technical betas. Keep SmartScreen,
  Windows, and organization protections enabled. Never disable protections or
  bypass local policy to install them; stop if they block the candidate.

Verify SHA256SUMS.txt and the GitHub build-provenance attestation before use.
That attestation is not a promise of safety or a Windows publisher identity.
After publication, GitHub must report the release tag and assets as immutable.

AIRWIKI {tag} — CANDIDATO DE LANZAMIENTO POR PLATAFORMA, NO ESTABLE

Este candidato público nunca es Latest y el actualizador de AirWiki nunca lo
elige. No establece un canal de lanzamiento público con soporte.

- macOS (Apple silicon, macOS 13+): este DMG tiene firma Developer ID y está
  notarizado por Apple, pero es un candidato de lanzamiento, no una versión
  estable.
- Windows (x64): estos MSI son betas técnicas sin firma. Mantén activas las
  protecciones de SmartScreen, Windows y la organización. Nunca desactives
  protecciones ni eludas la política local para instalarlos; detente si bloquean
  el candidato.

Verifica SHA256SUMS.txt y la atestación de procedencia de GitHub antes de usarlo.
Esa atestación no promete seguridad ni proporciona una identidad de editor para
Windows. Después de publicarse, GitHub debe informar que la etiqueta y los
archivos de la release son inmutables.
"""


def release_notes(version: str, rc_number: str, commit: str) -> str:
    tag = release_tag(version, rc_number)
    return f"""# AirWiki {tag} — platform release candidate

> **Never Latest and never selected by the updater.** This is not a stable
> AirWiki release channel.

Built from reviewed commit `{commit}`.

## Platform availability

- **macOS 13+ (Apple silicon):** `AirWiki_{version}_aarch64_SIGNED-NOTARIZED-RC.dmg`
  is Developer ID signed and notarized, but remains a release candidate rather
  than a stable release.
- **Windows 10/11 x64:** `en-US` and `es-ES` MSI files are unsigned technical
  betas. Keep SmartScreen, Windows, and organization protections enabled; never
  disable or bypass them to install this candidate.

Verify the exact download with `SHA256SUMS.txt`, then verify GitHub build
provenance:

```text
gh attestation verify <asset> --repo airwiki/airwiki --signer-workflow airwiki/airwiki/{RELEASE_WORKFLOW} --source-ref refs/heads/main --source-digest {commit} --deny-self-hosted-runners
```

The attestation establishes build provenance, not safety or a Windows publisher
identity. `MACOS-VERIFICATION.json` records the completed Developer ID and
notarization verification for the original DMG. See
`PLATFORM-RELEASE-CANDIDATE.txt` for bilingual safety boundaries.
GitHub must report the published release tag and assets as immutable.

## Español

> **Nunca Latest y nunca elegido por el actualizador.** Este no es un canal de
> lanzamiento estable de AirWiki.

- **macOS 13+ (Apple silicon):** el DMG está firmado con Developer ID y
  notarizado, pero sigue siendo un candidato de lanzamiento, no una versión
  estable.
- **Windows 10/11 x64:** los MSI `en-US` y `es-ES` son betas técnicas sin firma.
  Mantén activas las protecciones de SmartScreen, Windows y la organización;
  nunca las desactives ni las eludas para instalar este candidato.

Verifica `SHA256SUMS.txt` y la atestación de procedencia de GitHub indicada
arriba. La atestación prueba la procedencia de la compilación, no la seguridad ni
una identidad de editor para Windows. `MACOS-VERIFICATION.json` conserva la
verificación completada del DMG original. Consulta
`PLATFORM-RELEASE-CANDIDATE.txt` para los límites bilingües completos.
GitHub debe informar que la etiqueta y los archivos publicados son inmutables.
"""


def _required_input(root: Path, path: Path, maximum: int) -> Path:
    return technical.require_input(root, path, maximum)


def exact_json_value(value: object, expected: object) -> bool:
    """Compare JSON values without Python's bool/int equality coercion."""
    if type(value) is not type(expected):
        return False
    if isinstance(expected, dict):
        if not isinstance(value, dict) or set(value) != set(expected):
            return False
        return all(exact_json_value(value[key], item) for key, item in expected.items())
    if isinstance(expected, list):
        if not isinstance(value, list) or len(value) != len(expected):
            return False
        return all(
            exact_json_value(actual, item)
            for actual, item in zip(value, expected, strict=True)
        )
    return value == expected


def reject_symlink_ancestors(path: Path) -> None:
    """Reject every existing ancestor before creating a public staging tree."""
    ancestor = path.absolute()
    while not ancestor.exists():
        ancestor = ancestor.parent
    while True:
        # macOS exposes /var and /tmp as fixed OS compatibility aliases for
        # /private/var. They are not caller-selected staging components.
        system_alias = ancestor in {Path("/var"), Path("/tmp")}
        if ancestor.is_symlink() and not system_alias:
            raise ValueError("platform release candidate output parent contains a symlink")
        if ancestor == ancestor.parent:
            return
        ancestor = ancestor.parent


def verify_output(output: Path) -> None:
    if output.is_symlink() or not output.is_dir():
        raise ValueError("platform release candidate output must be a regular directory")
    entries = {entry.name: entry for entry in output.iterdir()}
    if set(entries) != {"assets", "RELEASE_NOTES.md"}:
        raise ValueError("platform release candidate output contains unexpected entries")
    assets_dir = entries["assets"]
    notes_path = entries["RELEASE_NOTES.md"]
    if assets_dir.is_symlink() or not assets_dir.is_dir() or notes_path.is_symlink() or not notes_path.is_file():
        raise ValueError("platform release candidate output layout is invalid")
    files = technical.regular_files(assets_dir)
    provenance_path = files.get("PROVENANCE.json")
    if provenance_path is None:
        raise ValueError("platform release candidate provenance is missing")
    provenance = technical.read_json(provenance_path)
    required = ("version", "rcNumber", "commit", "repository", "workflowRun", "generatedAt")
    if not all(isinstance(provenance.get(key), str) for key in required):
        raise ValueError("platform release candidate provenance metadata is incomplete")
    version, rc_number, commit, repository, workflow_run, generated_at = (
        cast(str, provenance[key]) for key in required
    )
    validate_metadata(version, rc_number, commit, repository, workflow_run, generated_at)
    if set(files) != release_asset_names(version):
        raise ValueError("platform release candidate asset set differs")
    expected_top = {
        "schemaVersion", "artifactKind", "supportedPublicRelease", "updaterChannel",
        "latest", "repository", "commit", "tag", "version", "rcNumber",
        "generatedAt", "workflowRun", "platforms", "artifacts",
    }
    if set(provenance) != expected_top:
        raise ValueError("platform release candidate provenance has an unexpected schema")
    if (
        type(provenance["schemaVersion"]) is not int
        or provenance["schemaVersion"] != 1
        or provenance["artifactKind"] != "airwiki-platform-release-candidate"
        or provenance["supportedPublicRelease"] is not False
        or provenance["updaterChannel"] is not False
        or provenance["latest"] is not False
        or provenance["tag"] != release_tag(version, rc_number)
        or not exact_json_value(provenance["platforms"], expected_platforms(version))
    ):
        raise ValueError("platform release candidate provenance weakens the release boundary")
    base = {name: path for name, path in files.items() if name not in {"PROVENANCE.json", "SHA256SUMS.txt"}}
    expected_artifacts = {name: {"sha256": technical.sha256(path), "size": path.stat().st_size} for name, path in sorted(base.items())}
    if not exact_json_value(provenance["artifacts"], expected_artifacts):
        raise ValueError("platform release candidate provenance does not match its assets")
    sums = technical.parse_sums(files["SHA256SUMS.txt"])
    if set(sums) != set(files) - {"SHA256SUMS.txt"}:
        raise ValueError("SHA256SUMS.txt does not describe the exact candidate assets")
    for name, digest in sums.items():
        if technical.sha256(files[name]) != digest:
            raise ValueError(f"platform release candidate digest mismatch: {name}")
    root = Path(__file__).resolve().parent.parent
    for name, source in technical.LEGAL_PAYLOADS.items():
        if files[name].read_bytes() != (root / source).read_bytes():
            raise ValueError(f"platform release candidate legal payload differs: {name}")
    names = primary_asset_names(version)
    technical.validate_dmg(files[names["macos"]])
    technical.validate_msi(files[names["windows_en"]])
    technical.validate_msi(files[names["windows_es"]])
    read_receipt(files[MACOS_RECEIPT_NAME], version, commit, technical.sha256(files[names["macos"]]))
    if files[NOTICE_NAME].read_text(encoding="utf-8") != notice(version, rc_number):
        raise ValueError("platform release candidate notice differs from the release contract")
    if notes_path.read_text(encoding="utf-8") != release_notes(version, rc_number, commit):
        raise ValueError("platform release candidate notes differ from the release contract")


def prepare(args: Namespace) -> None:
    validate_metadata(args.version, args.rc_number, args.commit, args.repository, args.workflow_run, args.generated_at)
    input_root = args.input_root
    macos = _required_input(input_root, args.macos_dmg, technical.MAX_DMG_BYTES)
    receipt = _required_input(input_root, args.macos_receipt, 64 * 1024)
    windows_en = _required_input(input_root, args.windows_en, technical.MAX_MSI_BYTES)
    windows_es = _required_input(input_root, args.windows_es, technical.MAX_MSI_BYTES)
    expected_inputs = (
        (macos, f"AirWiki_{args.version}_aarch64.dmg"),
        (windows_en, f"AirWiki_{args.version}_x64_en-US.msi"),
        (windows_es, f"AirWiki_{args.version}_x64_es-ES.msi"),
    )
    if len({path for path, _ in expected_inputs}) != len(expected_inputs):
        raise ValueError("platform release candidate inputs must be distinct")
    for path, name in expected_inputs:
        if path.name != name:
            raise ValueError(f"platform release candidate input name differs: {path.name}")
    technical.validate_dmg(macos)
    technical.validate_msi(windows_en)
    technical.validate_msi(windows_es)
    verified_receipt = read_receipt(receipt, args.version, args.commit, technical.sha256(macos))
    output = args.output.absolute()
    if output.exists() or output.is_symlink():
        raise ValueError("platform release candidate output already exists")
    reject_symlink_ancestors(output.parent)
    output.parent.mkdir(parents=True, exist_ok=True)
    reject_symlink_ancestors(output.parent)
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.", dir=output.parent))
    try:
        assets = staging / "assets"
        assets.mkdir()
        names = primary_asset_names(args.version)
        shutil.copyfile(macos, assets / names["macos"])
        shutil.copyfile(windows_en, assets / names["windows_en"])
        shutil.copyfile(windows_es, assets / names["windows_es"])
        root = Path(__file__).resolve().parent.parent
        for name, source in technical.LEGAL_PAYLOADS.items():
            shutil.copyfile(root / source, assets / name)
        technical.atomic_json(assets / MACOS_RECEIPT_NAME, verified_receipt)
        (assets / NOTICE_NAME).write_text(notice(args.version, args.rc_number), encoding="utf-8", newline="\n")
        base = technical.regular_files(assets)
        technical.atomic_json(assets / "PROVENANCE.json", {
            "schemaVersion": 1,
            "artifactKind": "airwiki-platform-release-candidate",
            "supportedPublicRelease": False,
            "updaterChannel": False,
            "latest": False,
            "repository": args.repository,
            "commit": args.commit,
            "tag": release_tag(args.version, args.rc_number),
            "version": args.version,
            "rcNumber": args.rc_number,
            "generatedAt": args.generated_at,
            "workflowRun": args.workflow_run,
            "platforms": expected_platforms(args.version),
            "artifacts": {name: {"sha256": technical.sha256(path), "size": path.stat().st_size} for name, path in sorted(base.items())},
        })
        checksummed = technical.regular_files(assets)
        (assets / "SHA256SUMS.txt").write_text("".join(f"{technical.sha256(path)}  {name}\n" for name, path in sorted(checksummed.items())), encoding="utf-8", newline="\n")
        (staging / "RELEASE_NOTES.md").write_text(release_notes(args.version, args.rc_number, args.commit), encoding="utf-8", newline="\n")
        verify_output(staging)
        staging.replace(output)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    subcommands = result.add_subparsers(dest="command", required=True)
    prepare_parser = subcommands.add_parser("prepare")
    prepare_parser.add_argument("--input-root", type=Path, required=True)
    prepare_parser.add_argument("--macos-dmg", type=Path, required=True)
    prepare_parser.add_argument("--macos-receipt", type=Path, required=True)
    prepare_parser.add_argument("--windows-en", type=Path, required=True)
    prepare_parser.add_argument("--windows-es", type=Path, required=True)
    prepare_parser.add_argument("--output", type=Path, required=True)
    prepare_parser.add_argument("--version", required=True)
    prepare_parser.add_argument("--rc-number", required=True)
    prepare_parser.add_argument("--commit", required=True)
    prepare_parser.add_argument("--repository", required=True)
    prepare_parser.add_argument("--workflow-run", required=True)
    prepare_parser.add_argument("--generated-at", required=True)
    verify_parser = subcommands.add_parser("verify")
    verify_parser.add_argument("--output", type=Path, required=True)
    return result


def main() -> None:
    args = parser().parse_args()
    if args.command == "prepare":
        prepare(args)
    else:
        verify_output(args.output)
    print(f"platform release candidate {args.command}: PASS")


if __name__ == "__main__":
    main()
