#!/usr/bin/env python3
"""Generate and verify the exact public AirWiki release asset set."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import tempfile
from datetime import datetime
from pathlib import Path
from urllib.parse import quote


SHA256 = re.compile(r"^[0-9a-f]{64}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
STABLE_SEMVER = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$"
)
REPOSITORY = "airwiki/airwiki"
SPDX_23_SCHEMA_SHA256 = "3ec6cd5b8ba0c9a3e821da48536fa1b814567dc7e4376efe98d3e7b2a7a8d230"


def base_asset_names(version: str) -> set[str]:
    windows = [
        f"AirWiki_{version}_x64_en-US.msi",
        f"AirWiki_{version}_x64_es-ES.msi",
    ]
    return {
        f"AirWiki_{version}_aarch64.dmg",
        "AirWiki.app.tar.gz",
        "AirWiki.app.tar.gz.sig",
        *(windows),
        *(f"{name}.sig" for name in windows),
        "LICENSE",
        "THIRD_PARTY_NOTICES.md",
        "THIRD_PARTY_LICENSES.md",
        "NPM_LICENSES_MACOS_ARM64.md",
        "NPM_LICENSES_WINDOWS_X64.md",
        "NON_CARGO_COMPONENTS.md",
        "latest.json",
    }


def metadata_asset_names(version: str) -> set[str]:
    return {
        "SHA256SUMS",
        f"airwiki-{version}.spdx.json",
        f"airwiki-{version}.provenance.json",
    }


def digest(path: Path) -> str:
    return file_digest(path, "sha256")


def file_digest(path: Path, algorithm: str) -> str:
    result = hashlib.new(algorithm)
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def regular_files(directory: Path) -> dict[str, Path]:
    files: dict[str, Path] = {}
    for entry in directory.iterdir():
        if entry.is_symlink() or not entry.is_file():
            raise ValueError(f"release directory contains a non-regular entry: {entry.name}")
        files[entry.name] = entry
    return files


def require_exact_files(directory: Path, expected: set[str]) -> dict[str, Path]:
    files = regular_files(directory)
    actual = set(files)
    if actual != expected:
        missing = ", ".join(sorted(expected - actual)) or "none"
        unexpected = ", ".join(sorted(actual - expected)) or "none"
        raise ValueError(f"release asset set differs; missing: {missing}; unexpected: {unexpected}")
    return files


def atomic_json(path: Path, value: object) -> None:
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with open(descriptor, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, indent=2, sort_keys=True)
            stream.write("\n")
        temporary.replace(path)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def parse_inventory(path: Path, ecosystem: str) -> list[dict[str, str]]:
    packages: list[dict[str, str]] = []
    in_packages = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line == "## Packages":
            in_packages = True
            continue
        if in_packages and line.startswith("## "):
            break
        if not in_packages or not line.startswith("|"):
            continue
        columns = [column.strip() for column in line.strip("|").split("|")]
        if len(columns) < 3 or columns[0] in {"Package", "---"}:
            continue
        for version in columns[1].split(", "):
            packages.append(
                {
                    "ecosystem": ecosystem,
                    "name": columns[0],
                    "version": version,
                    "declaredLicense": columns[2],
                }
            )
    if not packages:
        raise ValueError(f"license inventory contains no packages: {path.name}")
    if ecosystem == "npm":
        validate_missing_legal_file_inventory(path, packages)
    return packages


def validate_missing_legal_file_inventory(
    path: Path, packages: list[dict[str, str]]
) -> None:
    known = {(package["name"], package["version"]) for package in packages}
    seen: set[tuple[str, str]] = set()
    in_missing_legal_files = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line == "## Packages without a published legal file":
            in_missing_legal_files = True
            continue
        if in_missing_legal_files and line.startswith("## "):
            break
        if not in_missing_legal_files or not line.startswith("- "):
            continue
        match = re.fullmatch(r"- `([^`]+)`", line)
        if match is None or "@" not in match.group(1):
            raise ValueError(f"license inventory contains an invalid package callout: {path.name}")
        name, version = match.group(1).rsplit("@", 1)
        package = (name, version)
        if not name or not version or package not in known or package in seen:
            raise ValueError(
                f"license inventory package callout is missing, malformed or duplicated: {path.name}"
            )
        seen.add(package)


def load_spdx_schema(root: Path) -> dict[str, object]:
    path = root / "packaging/schemas/spdx-2.3.schema.json"
    if digest(path) != SPDX_23_SCHEMA_SHA256:
        raise ValueError("vendored SPDX 2.3 schema does not match its reviewed upstream bytes")
    schema = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(schema, dict):
        raise ValueError("vendored SPDX 2.3 schema is not a JSON object")
    return schema


def json_type_matches(value: object, expected: str) -> bool:
    if expected == "object":
        return isinstance(value, dict)
    if expected == "array":
        return isinstance(value, list)
    if expected == "string":
        return isinstance(value, str)
    if expected == "boolean":
        return isinstance(value, bool)
    if expected == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    raise ValueError(f"vendored SPDX schema uses unsupported JSON type: {expected}")


def validate_json_schema(value: object, schema: dict[str, object], location: str) -> None:
    expected_type = schema.get("type")
    if isinstance(expected_type, str) and not json_type_matches(value, expected_type):
        raise ValueError(f"SPDX schema type mismatch at {location}")

    allowed_values = schema.get("enum")
    if isinstance(allowed_values, list) and value not in allowed_values:
        raise ValueError(f"SPDX schema enum mismatch at {location}")

    if isinstance(value, dict):
        properties = schema.get("properties", {})
        required = schema.get("required", [])
        if not isinstance(properties, dict) or not isinstance(required, list):
            raise ValueError(f"vendored SPDX schema has an invalid object rule at {location}")
        missing = [name for name in required if isinstance(name, str) and name not in value]
        if missing:
            raise ValueError(f"SPDX schema required field is missing at {location}.{missing[0]}")
        if schema.get("additionalProperties") is False:
            unknown = set(value) - set(properties)
            if unknown:
                raise ValueError(
                    f"SPDX schema rejects field at {location}.{sorted(unknown)[0]}"
                )
        for name, item in value.items():
            item_schema = properties.get(name)
            if isinstance(item_schema, dict):
                validate_json_schema(item, item_schema, f"{location}.{name}")

    if isinstance(value, list):
        minimum = schema.get("minItems")
        if isinstance(minimum, int) and len(value) < minimum:
            raise ValueError(f"SPDX schema requires more items at {location}")
        item_schema = schema.get("items")
        if isinstance(item_schema, dict):
            for index, item in enumerate(value):
                validate_json_schema(item, item_schema, f"{location}[{index}]")


def validate_spdx_document(root: Path, document: dict[str, object]) -> None:
    validate_json_schema(document, load_spdx_schema(root), "$")
    files = document.get("files")
    if not isinstance(files, list) or any(
        not isinstance(record, dict)
        or record.get("licenseInfoInFiles") != ["NOASSERTION"]
        for record in files
    ):
        raise ValueError("release SPDX file records must declare licenseInfoInFiles")


def spdx_document(
    root: Path,
    files: dict[str, Path],
    version: str,
    commit: str,
    created_at: str,
) -> dict[str, object]:
    dependencies = parse_inventory(
        root / "resources/licenses/THIRD_PARTY_LICENSES.md", "cargo"
    )
    dependencies.extend(
        parse_inventory(
            root / "resources/licenses/NPM_LICENSES_MACOS_ARM64.md", "npm"
        )
    )
    dependencies.extend(
        parse_inventory(
            root / "resources/licenses/NPM_LICENSES_WINDOWS_X64.md", "npm"
        )
    )
    dependencies = list(
        {
            (
                dependency["ecosystem"],
                dependency["name"],
                dependency["version"],
                dependency["declaredLicense"],
            ): dependency
            for dependency in dependencies
        }.values()
    )
    dependencies.sort(key=lambda item: (item["ecosystem"], item["name"], item["version"]))
    package_records: list[dict[str, object]] = []
    relationships: list[dict[str, str]] = []
    for index, dependency in enumerate(dependencies, start=1):
        spdx_id = f"SPDXRef-Dependency-{index}"
        name = dependency["name"]
        version_value = dependency["version"]
        ecosystem = dependency["ecosystem"]
        package_records.append(
            {
                "SPDXID": spdx_id,
                "name": name,
                "versionInfo": version_value,
                "downloadLocation": "NOASSERTION",
                "filesAnalyzed": False,
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": "NOASSERTION",
                "licenseComments": (
                    "Exact declared metadata and legal texts are in the bundled "
                    f"AirWiki {ecosystem} license inventory: {dependency['declaredLicense']}"
                ),
                "externalRefs": [
                    {
                        "referenceCategory": "PACKAGE-MANAGER",
                        "referenceType": "purl",
                        "referenceLocator": (
                            f"pkg:{ecosystem}/{quote(name, safe='/')}@"
                            f"{quote(version_value, safe='.-_+')}"
                        ),
                    }
                ],
            }
        )
        relationships.append(
            {
                "spdxElementId": "SPDXRef-Package-AirWiki",
                "relationshipType": "DEPENDS_ON",
                "relatedSpdxElement": spdx_id,
            }
        )

    file_records: list[dict[str, object]] = []
    verification_hashes: list[str] = []
    for index, name in enumerate(sorted(files), start=1):
        spdx_id = f"SPDXRef-ReleaseFile-{index}"
        sha1 = file_digest(files[name], "sha1")
        verification_hashes.append(sha1)
        file_records.append(
            {
                "SPDXID": spdx_id,
                "fileName": f"./{name}",
                "checksums": [
                    {"algorithm": "SHA1", "checksumValue": sha1},
                    {"algorithm": "SHA256", "checksumValue": digest(files[name])},
                ],
                "licenseConcluded": "NOASSERTION",
                "licenseInfoInFiles": ["NOASSERTION"],
                "copyrightText": "NOASSERTION",
            }
        )
        relationships.append(
            {
                "spdxElementId": "SPDXRef-Package-AirWiki",
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": spdx_id,
            }
        )

    package_records.insert(
        0,
        {
            "SPDXID": "SPDXRef-Package-AirWiki",
            "name": "AirWiki",
            "versionInfo": version,
            "downloadLocation": f"https://github.com/{REPOSITORY}/releases/tag/v{version}",
            "filesAnalyzed": True,
            "packageVerificationCode": {
                "packageVerificationCodeValue": hashlib.sha1(
                    "".join(sorted(verification_hashes)).encode("ascii")
                ).hexdigest()
            },
            "licenseConcluded": "Apache-2.0",
            "licenseDeclared": "Apache-2.0",
            "copyrightText": "NOASSERTION",
        },
    )
    return {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"AirWiki {version} release",
        "documentNamespace": (
            f"https://github.com/{REPOSITORY}/releases/tag/v{version}/spdx/{commit}"
        ),
        "creationInfo": {
            "created": created_at,
            "creators": ["Organization: AirWiki", "Tool: packaging/release_assets.py"],
        },
        "documentDescribes": ["SPDXRef-Package-AirWiki"],
        "packages": package_records,
        "files": file_records,
        "relationships": relationships,
    }


def generate(args: argparse.Namespace) -> None:
    directory = args.directory.resolve()
    root = Path(__file__).resolve().parent.parent
    base_files = require_exact_files(directory, base_asset_names(args.version))
    spdx_path = directory / f"airwiki-{args.version}.spdx.json"
    spdx = spdx_document(root, base_files, args.version, args.commit, args.created_at)
    validate_spdx_document(root, spdx)
    atomic_json(spdx_path, spdx)
    provenance_inputs = {**base_files, spdx_path.name: spdx_path}
    provenance = {
        "schemaVersion": 1,
        "repository": REPOSITORY,
        "commit": args.commit,
        "tag": f"v{args.version}",
        "version": args.version,
        "generatedAt": args.created_at,
        "workflowRun": args.workflow_run,
        "artifacts": {
            name: {"sha256": digest(path), "size": path.stat().st_size}
            for name, path in sorted(provenance_inputs.items())
        },
    }
    provenance_path = directory / f"airwiki-{args.version}.provenance.json"
    atomic_json(provenance_path, provenance)

    checksummed = {**provenance_inputs, provenance_path.name: provenance_path}
    sums = "".join(f"{digest(path)}  {name}\n" for name, path in sorted(checksummed.items()))
    (directory / "SHA256SUMS").write_text(sums, encoding="utf-8", newline="\n")
    require_exact_files(
        directory, base_asset_names(args.version) | metadata_asset_names(args.version)
    )


def parse_sums(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([^/\\]+)", line)
        if match is None or match.group(2) in values:
            raise ValueError("SHA256SUMS contains an invalid or duplicate entry")
        values[match.group(2)] = match.group(1)
    return values


def verify_subset(args: argparse.Namespace) -> None:
    expected = set(args.asset)
    if not expected or len(expected) != len(args.asset):
        raise ValueError("release subset must name unique assets")
    files = require_exact_files(args.directory.resolve(), expected)
    if digest(args.sums) != args.sums_sha256:
        raise ValueError("release checksum inventory changed after initial verification")
    sums = parse_sums(args.sums)
    for name, path in files.items():
        expected_digest = sums.get(name)
        if expected_digest is None or digest(path) != expected_digest:
            raise ValueError(f"release subset digest mismatch: {name}")


def verify(args: argparse.Namespace) -> None:
    directory = args.directory.resolve()
    expected = base_asset_names(args.version) | metadata_asset_names(args.version)
    files = require_exact_files(directory, expected)
    sums = parse_sums(files["SHA256SUMS"])
    expected_sums = expected - {"SHA256SUMS"}
    if set(sums) != expected_sums:
        raise ValueError("SHA256SUMS does not describe the exact release asset set")
    for name, expected_digest in sums.items():
        if not SHA256.fullmatch(expected_digest) or digest(files[name]) != expected_digest:
            raise ValueError(f"release asset digest mismatch: {name}")

    provenance = json.loads(
        files[f"airwiki-{args.version}.provenance.json"].read_text(encoding="utf-8")
    )
    if (
        provenance.get("schemaVersion") != 1
        or provenance.get("repository") != REPOSITORY
        or provenance.get("commit") != args.commit
        or provenance.get("tag") != f"v{args.version}"
        or provenance.get("version") != args.version
    ):
        raise ValueError("release provenance does not match the requested release")
    described = provenance.get("artifacts")
    if not isinstance(described, dict) or set(described) != expected_sums - {
        f"airwiki-{args.version}.provenance.json"
    }:
        raise ValueError("release provenance describes an unexpected artifact set")
    for name, record in described.items():
        if not isinstance(record, dict) or record.get("sha256") != digest(files[name]):
            raise ValueError(f"release provenance digest mismatch: {name}")

    spdx = json.loads(files[f"airwiki-{args.version}.spdx.json"].read_text(encoding="utf-8"))
    validate_spdx_document(Path(__file__).resolve().parent.parent, spdx)
    if (
        spdx.get("spdxVersion") != "SPDX-2.3"
        or spdx.get("dataLicense") != "CC0-1.0"
        or spdx.get("documentDescribes") != ["SPDXRef-Package-AirWiki"]
    ):
        raise ValueError("release SBOM does not satisfy the SPDX document contract")
    packages = spdx.get("packages")
    if not isinstance(packages, list) or not packages:
        raise ValueError("release SBOM contains no packages")
    root_package = packages[0]
    if (
        not isinstance(root_package, dict)
        or root_package.get("name") != "AirWiki"
        or root_package.get("versionInfo") != args.version
    ):
        raise ValueError("release SBOM identifies a different product version")
    verification_code = root_package.get("packageVerificationCode")
    expected_verification_code = hashlib.sha1(
        "".join(
            sorted(file_digest(files[name], "sha1") for name in base_asset_names(args.version))
        ).encode("ascii")
    ).hexdigest()
    if (
        not isinstance(verification_code, dict)
        or verification_code.get("packageVerificationCodeValue")
        != expected_verification_code
    ):
        raise ValueError("release SBOM package verification code does not match final files")
    spdx_files = spdx.get("files")
    if not isinstance(spdx_files, list):
        raise ValueError("release SBOM contains no file inventory")
    described_files: dict[str, str] = {}
    for record in spdx_files:
        if not isinstance(record, dict):
            raise ValueError("release SBOM contains an invalid file record")
        name = record.get("fileName")
        checksums = record.get("checksums")
        if (
            not isinstance(name, str)
            or not name.startswith("./")
            or not isinstance(checksums, list)
        ):
            raise ValueError("release SBOM contains an invalid file checksum")
        sha256 = next(
            (
                checksum.get("checksumValue")
                for checksum in checksums
                if isinstance(checksum, dict) and checksum.get("algorithm") == "SHA256"
            ),
            None,
        )
        if not isinstance(sha256, str):
            raise ValueError("release SBOM contains no SHA-256 file checksum")
        if name[2:] in described_files:
            raise ValueError("release SBOM contains a duplicate file record")
        described_files[name[2:]] = sha256
    if set(described_files) != base_asset_names(args.version):
        raise ValueError("release SBOM describes an unexpected final file set")
    for name, expected_digest in described_files.items():
        if expected_digest != digest(files[name]):
            raise ValueError(f"release SBOM digest mismatch: {name}")


def stable_version(value: str) -> str:
    if STABLE_SEMVER.fullmatch(value) is None:
        raise argparse.ArgumentTypeError("version must be a stable three-part semver")
    return value


def exact_commit(value: str) -> str:
    if COMMIT.fullmatch(value) is None:
        raise argparse.ArgumentTypeError("commit must be a full lowercase SHA-1")
    return value


def exact_sha256(value: str) -> str:
    if SHA256.fullmatch(value) is None:
        raise argparse.ArgumentTypeError("digest must be a full lowercase SHA-256")
    return value


def exact_timestamp(value: str) -> str:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise argparse.ArgumentTypeError("timestamp must use ISO 8601") from error
    if parsed.tzinfo is None or not value.endswith("Z"):
        raise argparse.ArgumentTypeError("timestamp must be UTC and end in Z")
    return value


def main() -> None:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    for command in ("generate", "verify"):
        subparser = subparsers.add_parser(command)
        subparser.add_argument("--directory", required=True, type=Path)
        subparser.add_argument("--version", required=True, type=stable_version)
        subparser.add_argument("--commit", required=True, type=exact_commit)
        if command == "generate":
            subparser.add_argument("--created-at", required=True, type=exact_timestamp)
            subparser.add_argument("--workflow-run", required=True)
    subset = subparsers.add_parser("verify-subset")
    subset.add_argument("--directory", required=True, type=Path)
    subset.add_argument("--sums", required=True, type=Path)
    subset.add_argument("--sums-sha256", required=True, type=exact_sha256)
    subset.add_argument("--asset", required=True, action="append")
    args = parser.parse_args()
    if args.command == "generate":
        generate(args)
    elif args.command == "verify":
        verify(args)
    else:
        verify_subset(args)


if __name__ == "__main__":
    main()
