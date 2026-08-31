#!/usr/bin/env python3
"""Prepare and verify the exact public AirWiki technical prerelease assets."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import re
import shutil
import stat
import tarfile
import tempfile
from datetime import datetime
from pathlib import Path
from typing import cast


REPOSITORY = "airwiki/airwiki"
VERSION = re.compile(r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$")
BETA_NUMBER = re.compile(r"^[1-9][0-9]{0,3}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
GENERATED_AT = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")
WORKFLOW_RUN = re.compile(
    rf"^https://github\.com/{re.escape(REPOSITORY)}/actions/runs/[1-9][0-9]*$"
)
MSI_MAGIC = bytes.fromhex("d0cf11e0a1b11ae1")
ELF_MAGIC = b"\x7fELF"
MAX_DMG_BYTES = 2 * 1024 * 1024 * 1024
MAX_MSI_BYTES = 2 * 1024 * 1024 * 1024
MAX_LINUX_BYTES = 256 * 1024 * 1024
LEGAL_PAYLOADS = {
    "LICENSE": Path("LICENSE"),
    "THIRD_PARTY_NOTICES.md": Path("THIRD_PARTY_NOTICES.md"),
    "THIRD_PARTY_LICENSES.md": Path("resources/licenses/THIRD_PARTY_LICENSES.md"),
    "NPM_LICENSES_MACOS_ARM64.md": Path(
        "resources/licenses/NPM_LICENSES_MACOS_ARM64.md"
    ),
    "NPM_LICENSES_WINDOWS_X64.md": Path(
        "resources/licenses/NPM_LICENSES_WINDOWS_X64.md"
    ),
    "NON_CARGO_COMPONENTS.md": Path("resources/licenses/NON_CARGO_COMPONENTS.md"),
}


def prerelease_tag(version: str, beta_number: str) -> str:
    return f"v{version}-beta.{beta_number}"


def primary_asset_names(version: str) -> dict[str, str]:
    return {
        "macos": f"AirWiki_{version}_aarch64_UNSIGNED-NOT-NOTARIZED.dmg",
        "windows_en": f"AirWiki_{version}_x64_en-US_UNSIGNED.msi",
        "windows_es": f"AirWiki_{version}_x64_es-ES_UNSIGNED.msi",
        "linux": f"AirWiki_federation-index_{version}_linux-x64.tar.gz",
    }


def release_asset_names(version: str) -> set[str]:
    return {
        *primary_asset_names(version).values(),
        *LEGAL_PAYLOADS.keys(),
        "PROVENANCE.json",
        "SHA256SUMS.txt",
        "TECHNICAL-PRE-RELEASE.txt",
    }


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def unique_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for name, item in pairs:
        if name in value:
            raise ValueError(f"JSON contains a duplicate field: {name}")
        value[name] = item
    return value


def read_json(path: Path) -> dict[str, object]:
    if path.stat().st_size > 64 * 1024:
        raise ValueError(f"JSON exceeds the technical prerelease limit: {path.name}")
    try:
        value = json.loads(
            path.read_text(encoding="utf-8"), object_pairs_hook=unique_json_object
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"invalid JSON: {path.name}") from error
    if not isinstance(value, dict):
        raise ValueError(f"JSON root must be an object: {path.name}")
    return value


def atomic_json(path: Path, value: object) -> None:
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with open(descriptor, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2, sort_keys=True)
            stream.write("\n")
        temporary.replace(path)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def validate_metadata(
    version: str,
    beta_number: str,
    commit: str,
    repository: str,
    workflow_run: str,
    generated_at: str,
) -> None:
    if VERSION.fullmatch(version) is None:
        raise ValueError("version must be stable major.minor.patch syntax")
    if BETA_NUMBER.fullmatch(beta_number) is None:
        raise ValueError("beta number must be an integer from 1 to 9999")
    if COMMIT.fullmatch(commit) is None:
        raise ValueError("commit must be one lowercase 40-character SHA")
    if repository != REPOSITORY:
        raise ValueError("technical prereleases are restricted to airwiki/airwiki")
    if WORKFLOW_RUN.fullmatch(workflow_run) is None:
        raise ValueError("workflow run must identify an official AirWiki Actions run")
    if GENERATED_AT.fullmatch(generated_at) is None:
        raise ValueError("generated-at must be an RFC 3339 UTC second")
    try:
        datetime.strptime(generated_at, "%Y-%m-%dT%H:%M:%SZ")
    except ValueError as error:
        raise ValueError("generated-at is not a real UTC date") from error


def require_input(root: Path, candidate: Path, maximum_bytes: int) -> Path:
    if root.is_symlink():
        raise ValueError("input root must not be a symlink")
    resolved_root = root.resolve(strict=True)
    if not resolved_root.is_dir():
        raise ValueError("input root must be a directory")
    try:
        relative = candidate.absolute().relative_to(root.absolute())
    except ValueError as error:
        raise ValueError("input path must remain inside the input root") from error
    current = root.absolute()
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise ValueError(f"input path contains a symlink: {candidate.name}")
    resolved = candidate.resolve(strict=True)
    try:
        resolved.relative_to(resolved_root)
    except ValueError as error:
        raise ValueError("resolved input path escapes the input root") from error
    metadata = resolved.stat()
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"input is not a regular file: {candidate.name}")
    if metadata.st_size <= 0 or metadata.st_size > maximum_bytes:
        raise ValueError(f"input size is invalid: {candidate.name}")
    return resolved


def validate_dmg(path: Path) -> None:
    if path.stat().st_size < 512 or path.stat().st_size > MAX_DMG_BYTES:
        raise ValueError("macOS candidate is not a UDIF DMG")
    with path.open("rb") as stream:
        stream.seek(-512, io.SEEK_END)
        if stream.read(4) != b"koly":
            raise ValueError("macOS candidate is not a UDIF DMG")


def validate_msi(path: Path) -> None:
    if path.stat().st_size <= len(MSI_MAGIC) or path.stat().st_size > MAX_MSI_BYTES:
        raise ValueError(f"Windows candidate MSI size is invalid: {path.name}")
    with path.open("rb") as stream:
        if stream.read(len(MSI_MAGIC)) != MSI_MAGIC:
            raise ValueError(
                f"Windows candidate is not an MSI compound file: {path.name}"
            )


def validate_linux_binary_bytes(content: bytes) -> None:
    if (
        len(content) < 20
        or content[:4] != ELF_MAGIC
        or content[4] != 2
        or content[5] != 1
        or content[18:20] != b"\x3e\x00"
    ):
        raise ValueError(
            "Linux federation candidate is not an x86-64 little-endian ELF"
        )


def validate_linux_binary(path: Path) -> None:
    with path.open("rb") as stream:
        validate_linux_binary_bytes(stream.read(20))


def notice(version: str, beta_number: str) -> str:
    tag = prerelease_tag(version, beta_number)
    return f"""AIRWIKI {tag} — UNSUPPORTED TECHNICAL PRE-RELEASE

This public technical pre-release is provided for testing and feedback. It is
not a supported stable release and is never selected by the AirWiki updater.

- Windows: the MSI files are intentionally unsigned. Keep Windows and
  organization protections enabled; stop if policy does not permit them.
- macOS: the DMG contains an ad-hoc-signed application and is not notarized or
  signed with Developer ID. Keep macOS protections enabled.
- Linux: the archive contains only the x86-64 federation index server for
  maintainers. It is not the AirWiki desktop application.

Verify SHA256SUMS.txt and the GitHub build-provenance attestation before
installation or deployment. The attestation does not provide an operating-system
publisher identity. Report reproducible issues without attaching private
knowledge, credentials, identities, paths or raw logs.

AIRWIKI {tag} — PRE-RELEASE TÉCNICA SIN SOPORTE

Esta pre-release técnica pública se ofrece para pruebas y comentarios. No es
una versión estable con soporte y el actualizador de AirWiki nunca la elige.

- Windows: los MSI no tienen firma intencionalmente. Mantén activas las
  protecciones de Windows y de la organización; detente si la política no los
  permite.
- macOS: el DMG contiene una aplicación con firma ad-hoc, sin notarización ni
  firma Developer ID. Mantén activas las protecciones de macOS.
- Linux: el archivo contiene solamente el servidor de índice federado x86-64
  para mantenedores. No es la aplicación de escritorio AirWiki.

Verifica SHA256SUMS.txt y la atestación de procedencia de GitHub antes de instalar
o desplegar. La atestación no proporciona una identidad de editor reconocida por
el sistema operativo. Reporta problemas reproducibles sin adjuntar conocimiento
privado, credenciales, identidades, rutas ni logs sin sanitizar.
"""


def linux_readme(version: str, beta_number: str) -> bytes:
    return (
        f"AirWiki {prerelease_tag(version, beta_number)} Linux x64 federation index\n\n"
        "This archive contains a server component for AirWiki maintainers.\n"
        "It is not the AirWiki desktop application and provides no desktop UI.\n"
        "Use only with the versioned federation runbook; verify the release SHA-256 "
        "and GitHub attestation.\n"
    ).encode("utf-8")


def release_notes(version: str, beta_number: str, commit: str) -> str:
    tag = prerelease_tag(version, beta_number)
    return f"""# AirWiki {tag} — public technical pre-release

> **Unsupported and not selected by the updater.** These artifacts are for
> technical testing while public platform signing is incomplete.

Built from reviewed commit `{commit}`.

## Downloads

- **Windows 10/11 x64:** choose the `en-US` or `es-ES` MSI. Both are unsigned.
- **Apple silicon, macOS 13+:** use the DMG. The app has an ad-hoc signature and
  is not notarized or signed with Developer ID.
- **Linux x64 maintainers:** the archive is the federation index server only;
  AirWiki Desktop is not available for Linux in this pre-release.

Download `SHA256SUMS.txt` and verify the exact asset before use. Then verify that
GitHub Actions produced those bytes from the official release workflow:

```text
gh attestation verify <asset> --repo airwiki/airwiki --signer-workflow airwiki/airwiki/.github/workflows/package-pilot.yml --source-ref refs/heads/main --source-digest {commit} --deny-self-hosted-runners
```

The attestation establishes build provenance, not safety or an operating-system
publisher identity. Keep operating-system and organization protections enabled.
The signed stable release and updater channel remain separate and unavailable
until every public gate passes.

See `TECHNICAL-PRE-RELEASE.txt` and `PROVENANCE.json` for the complete boundary.
"""


def add_tar_bytes(
    archive: tarfile.TarFile, name: str, content: bytes, mode: int
) -> None:
    info = tarfile.TarInfo(name=name)
    info.size = len(content)
    info.mode = mode
    info.mtime = 0
    info.uid = 0
    info.gid = 0
    info.uname = ""
    info.gname = ""
    archive.addfile(info, io.BytesIO(content))


def write_linux_archive(
    path: Path,
    binary: Path,
    root: Path,
    version: str,
    beta_number: str,
) -> None:
    with path.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(
                fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT
            ) as archive:
                add_tar_bytes(
                    archive,
                    "airwiki-federation-index",
                    binary.read_bytes(),
                    0o755,
                )
                add_tar_bytes(
                    archive, "README.txt", linux_readme(version, beta_number), 0o644
                )
                for release_name, source_name in sorted(LEGAL_PAYLOADS.items()):
                    add_tar_bytes(
                        archive,
                        release_name,
                        (root / source_name).read_bytes(),
                        0o644,
                    )


def expected_platforms(version: str) -> dict[str, object]:
    names = primary_asset_names(version)
    return {
        "linux-x64": {
            "asset": names["linux"],
            "desktopApplication": False,
            "purpose": "federation-index-server",
            "signature": "not-applicable",
        },
        "macos-arm64": {
            "asset": names["macos"],
            "desktopApplication": True,
            "nativeSignature": "ad-hoc-only",
            "notarized": False,
        },
        "windows-x64-en-US": {
            "asset": names["windows_en"],
            "authenticode": "not-signed",
            "desktopApplication": True,
        },
        "windows-x64-es-ES": {
            "asset": names["windows_es"],
            "authenticode": "not-signed",
            "desktopApplication": True,
        },
    }


def regular_files(directory: Path) -> dict[str, Path]:
    files: dict[str, Path] = {}
    for entry in directory.iterdir():
        if entry.is_symlink() or not entry.is_file():
            raise ValueError(
                f"asset directory contains a non-regular entry: {entry.name}"
            )
        files[entry.name] = entry
    return files


def parse_sums(path: Path) -> dict[str, str]:
    if path.stat().st_size > 64 * 1024:
        raise ValueError("SHA256SUMS.txt exceeds the technical prerelease limit")
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([^/\\]+)", line)
        if match is None or match.group(2) in values:
            raise ValueError("SHA256SUMS.txt contains an invalid or duplicate entry")
        values[match.group(2)] = match.group(1)
    return values


def verify_linux_archive(
    path: Path, root: Path, version: str, beta_number: str
) -> None:
    if (
        path.stat().st_size <= 0
        or path.stat().st_size > MAX_LINUX_BYTES + 16 * 1024 * 1024
    ):
        raise ValueError("Linux federation archive size is invalid")
    expected = {
        "airwiki-federation-index",
        "README.txt",
        *LEGAL_PAYLOADS.keys(),
    }
    with tarfile.open(path, mode="r:gz") as archive:
        members = archive.getmembers()
        if {member.name for member in members} != expected or len(members) != len(
            expected
        ):
            raise ValueError("Linux archive contains an unexpected file set")
        for member in members:
            if (
                not member.isfile()
                or member.uid != 0
                or member.gid != 0
                or member.mtime != 0
            ):
                raise ValueError(
                    "Linux archive contains unsafe or non-deterministic metadata"
                )
            if member.name == "airwiki-federation-index":
                maximum_size = MAX_LINUX_BYTES
            elif member.name == "README.txt":
                maximum_size = len(linux_readme(version, beta_number))
            else:
                source = LEGAL_PAYLOADS.get(member.name)
                if source is None:
                    raise ValueError("Linux archive contains an unexpected member")
                maximum_size = (root / source).stat().st_size
            if member.size <= 0 or member.size > maximum_size:
                raise ValueError("Linux archive member size is invalid")
            extracted = archive.extractfile(member)
            if extracted is None:
                raise ValueError("Linux archive member could not be read")
            content = extracted.read()
            if member.name == "airwiki-federation-index":
                if member.mode != 0o755:
                    raise ValueError("Linux federation binary is not executable")
                validate_linux_binary_bytes(content[:20])
            elif member.name == "README.txt":
                if content != linux_readme(version, beta_number):
                    raise ValueError(
                        "Linux archive README differs from the release contract"
                    )
            else:
                source = LEGAL_PAYLOADS.get(member.name)
                if source is None or content != (root / source).read_bytes():
                    raise ValueError(
                        "Linux archive legal payload differs from the repository"
                    )


def verify_output(output: Path) -> None:
    if output.is_symlink() or not output.is_dir():
        raise ValueError("technical prerelease output must be a regular directory")
    entries = {entry.name: entry for entry in output.iterdir()}
    if set(entries) != {"assets", "RELEASE_NOTES.md"}:
        raise ValueError("technical prerelease output contains unexpected entries")
    assets_dir = entries["assets"]
    notes_path = entries["RELEASE_NOTES.md"]
    if assets_dir.is_symlink() or not assets_dir.is_dir():
        raise ValueError("technical prerelease assets directory is invalid")
    if notes_path.is_symlink() or not notes_path.is_file():
        raise ValueError("technical prerelease notes are invalid")
    files = regular_files(assets_dir)
    provenance_path = files.get("PROVENANCE.json")
    if provenance_path is None:
        raise ValueError("technical prerelease provenance is missing")
    provenance = read_json(provenance_path)
    version = provenance.get("version")
    beta_number = provenance.get("betaNumber")
    commit = provenance.get("commit")
    repository = provenance.get("repository")
    workflow_run = provenance.get("workflowRun")
    generated_at = provenance.get("generatedAt")
    if not all(
        isinstance(value, str)
        for value in (
            version,
            beta_number,
            commit,
            repository,
            workflow_run,
            generated_at,
        )
    ):
        raise ValueError("technical prerelease provenance metadata is incomplete")
    version = cast(str, version)
    beta_number = cast(str, beta_number)
    commit = cast(str, commit)
    repository = cast(str, repository)
    workflow_run = cast(str, workflow_run)
    generated_at = cast(str, generated_at)
    validate_metadata(
        version, beta_number, commit, repository, workflow_run, generated_at
    )
    expected_names = release_asset_names(version)
    if set(files) != expected_names:
        missing = ", ".join(sorted(expected_names - set(files))) or "none"
        unexpected = ", ".join(sorted(set(files) - expected_names)) or "none"
        raise ValueError(
            f"technical prerelease asset set differs; missing: {missing}; unexpected: {unexpected}"
        )
    expected_top_level = {
        "schemaVersion",
        "artifactKind",
        "supportedPublicRelease",
        "updaterChannel",
        "latest",
        "repository",
        "commit",
        "tag",
        "version",
        "betaNumber",
        "generatedAt",
        "workflowRun",
        "platforms",
        "artifacts",
    }
    if set(provenance) != expected_top_level:
        raise ValueError("technical prerelease provenance has an unexpected schema")
    if (
        provenance["schemaVersion"] != 1
        or provenance["artifactKind"] != "airwiki-public-technical-prerelease"
        or provenance["supportedPublicRelease"] is not False
        or provenance["updaterChannel"] is not False
        or provenance["latest"] is not False
        or provenance["tag"] != prerelease_tag(version, beta_number)
        or provenance["platforms"] != expected_platforms(version)
    ):
        raise ValueError("technical prerelease provenance weakens the release boundary")
    base_files = {
        name: path
        for name, path in files.items()
        if name not in {"PROVENANCE.json", "SHA256SUMS.txt"}
    }
    expected_artifacts = {
        name: {"sha256": sha256(path), "size": path.stat().st_size}
        for name, path in sorted(base_files.items())
    }
    if provenance["artifacts"] != expected_artifacts:
        raise ValueError("technical prerelease provenance does not match its assets")
    sums = parse_sums(files["SHA256SUMS.txt"])
    expected_sums = set(files) - {"SHA256SUMS.txt"}
    if set(sums) != expected_sums:
        raise ValueError("SHA256SUMS.txt does not describe the exact prerelease assets")
    for name, expected_digest in sums.items():
        if sha256(files[name]) != expected_digest:
            raise ValueError(f"technical prerelease digest mismatch: {name}")
    root = Path(__file__).resolve().parent.parent
    for release_name, source_name in LEGAL_PAYLOADS.items():
        if files[release_name].read_bytes() != (root / source_name).read_bytes():
            raise ValueError(
                f"technical prerelease legal payload differs: {release_name}"
            )
    names = primary_asset_names(version)
    validate_dmg(files[names["macos"]])
    validate_msi(files[names["windows_en"]])
    validate_msi(files[names["windows_es"]])
    verify_linux_archive(files[names["linux"]], root, version, beta_number)
    if files["TECHNICAL-PRE-RELEASE.txt"].read_text(encoding="utf-8") != notice(
        version, beta_number
    ):
        raise ValueError(
            "technical prerelease notice differs from the release contract"
        )
    if notes_path.read_text(encoding="utf-8") != release_notes(
        version, beta_number, commit
    ):
        raise ValueError("technical prerelease notes differ from the release contract")


def prepare(args: argparse.Namespace) -> None:
    validate_metadata(
        args.version,
        args.beta_number,
        args.commit,
        args.repository,
        args.workflow_run,
        args.generated_at,
    )
    input_root = args.input_root
    macos = require_input(input_root, args.macos_dmg, MAX_DMG_BYTES)
    windows_en = require_input(input_root, args.windows_en, MAX_MSI_BYTES)
    windows_es = require_input(input_root, args.windows_es, MAX_MSI_BYTES)
    linux = require_input(input_root, args.linux_binary, MAX_LINUX_BYTES)
    inputs = (
        (macos, f"AirWiki_{args.version}_aarch64.dmg"),
        (windows_en, f"AirWiki_{args.version}_x64_en-US.msi"),
        (windows_es, f"AirWiki_{args.version}_x64_es-ES.msi"),
        (linux, "airwiki-federation-index"),
    )
    if len({path for path, _ in inputs}) != len(inputs):
        raise ValueError("technical prerelease inputs must be four distinct files")
    for path, expected in inputs:
        if path.name != expected:
            raise ValueError(f"technical prerelease input name differs: {path.name}")
    validate_dmg(macos)
    validate_msi(windows_en)
    validate_msi(windows_es)
    validate_linux_binary(linux)
    output = args.output.absolute()
    if output.exists() or output.is_symlink():
        raise ValueError("technical prerelease output already exists")
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.parent.is_symlink():
        raise ValueError("technical prerelease output parent must not be a symlink")
    staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.", dir=output.parent))
    try:
        assets = staging / "assets"
        assets.mkdir()
        names = primary_asset_names(args.version)
        shutil.copyfile(macos, assets / names["macos"])
        shutil.copyfile(windows_en, assets / names["windows_en"])
        shutil.copyfile(windows_es, assets / names["windows_es"])
        root = Path(__file__).resolve().parent.parent
        write_linux_archive(
            assets / names["linux"], linux, root, args.version, args.beta_number
        )
        for release_name, source_name in LEGAL_PAYLOADS.items():
            shutil.copyfile(root / source_name, assets / release_name)
        (assets / "TECHNICAL-PRE-RELEASE.txt").write_text(
            notice(args.version, args.beta_number), encoding="utf-8", newline="\n"
        )
        base_files = regular_files(assets)
        provenance = {
            "schemaVersion": 1,
            "artifactKind": "airwiki-public-technical-prerelease",
            "supportedPublicRelease": False,
            "updaterChannel": False,
            "latest": False,
            "repository": args.repository,
            "commit": args.commit,
            "tag": prerelease_tag(args.version, args.beta_number),
            "version": args.version,
            "betaNumber": args.beta_number,
            "generatedAt": args.generated_at,
            "workflowRun": args.workflow_run,
            "platforms": expected_platforms(args.version),
            "artifacts": {
                name: {"sha256": sha256(path), "size": path.stat().st_size}
                for name, path in sorted(base_files.items())
            },
        }
        atomic_json(assets / "PROVENANCE.json", provenance)
        checksummed = regular_files(assets)
        (assets / "SHA256SUMS.txt").write_text(
            "".join(
                f"{sha256(path)}  {name}\n"
                for name, path in sorted(checksummed.items())
            ),
            encoding="utf-8",
            newline="\n",
        )
        (staging / "RELEASE_NOTES.md").write_text(
            release_notes(args.version, args.beta_number, args.commit),
            encoding="utf-8",
            newline="\n",
        )
        verify_output(staging)
        staging.replace(output)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def parser() -> argparse.ArgumentParser:
    argument_parser = argparse.ArgumentParser()
    subcommands = argument_parser.add_subparsers(dest="command", required=True)
    prepare_parser = subcommands.add_parser("prepare")
    prepare_parser.add_argument("--input-root", type=Path, required=True)
    prepare_parser.add_argument("--macos-dmg", type=Path, required=True)
    prepare_parser.add_argument("--windows-en", type=Path, required=True)
    prepare_parser.add_argument("--windows-es", type=Path, required=True)
    prepare_parser.add_argument("--linux-binary", type=Path, required=True)
    prepare_parser.add_argument("--output", type=Path, required=True)
    prepare_parser.add_argument("--version", required=True)
    prepare_parser.add_argument("--beta-number", required=True)
    prepare_parser.add_argument("--commit", required=True)
    prepare_parser.add_argument("--repository", required=True)
    prepare_parser.add_argument("--workflow-run", required=True)
    prepare_parser.add_argument("--generated-at", required=True)
    verify_parser = subcommands.add_parser("verify")
    verify_parser.add_argument("--output", type=Path, required=True)
    return argument_parser


def main() -> None:
    args = parser().parse_args()
    if args.command == "prepare":
        prepare(args)
    else:
        verify_output(args.output)
    print(f"technical prerelease {args.command}: PASS")


if __name__ == "__main__":
    main()
