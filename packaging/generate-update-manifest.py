#!/usr/bin/env python3
"""Create the stable Tauri v2 updater manifest from final artifacts."""

from __future__ import annotations

import argparse
import json
import os
import re
import tempfile
from datetime import datetime
from pathlib import Path
from urllib.parse import quote, urlparse


SEMVER = re.compile(r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$")
REPOSITORY_RELEASES_PATH = "/airwiki/airwiki/releases/download"


def regular_file(value: str) -> Path:
    path = Path(value)
    if path.is_symlink() or not path.is_file():
        raise argparse.ArgumentTypeError(f"not a regular file: {path}")
    return path


def signature(path: Path) -> str:
    value = path.read_text(encoding="utf-8").strip()
    if not value or len(value) > 16 * 1024:
        raise ValueError(f"invalid updater signature: {path}")
    return value


def artifact_url(base_url: str, artifact: Path) -> str:
    return f"{base_url.rstrip('/')}/{quote(artifact.name)}"


def validate_publication_time(value: str) -> None:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("published-at must use ISO 8601") from error
    if parsed.tzinfo is None or not value.endswith("Z"):
        raise ValueError("published-at must be UTC and end in Z")


def validate_artifact_names(
    version: str,
    macos: Path,
    macos_signature: Path,
    windows: Path,
    windows_signature: Path,
) -> None:
    expected = {
        "macos": "AirWiki.app.tar.gz",
        "macos-signature": "AirWiki.app.tar.gz.sig",
        "windows": f"AirWiki_{version}_x64_en-US.msi",
        "windows-signature": f"AirWiki_{version}_x64_en-US.msi.sig",
    }
    actual = {
        "macos": macos.name,
        "macos-signature": macos_signature.name,
        "windows": windows.name,
        "windows-signature": windows_signature.name,
    }
    if actual != expected:
        raise ValueError("updater inputs do not match the exact stable artifact names")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--published-at", required=True)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--macos", required=True, type=regular_file)
    parser.add_argument("--macos-signature", required=True, type=regular_file)
    parser.add_argument("--windows", required=True, type=regular_file)
    parser.add_argument("--windows-signature", required=True, type=regular_file)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    if not SEMVER.fullmatch(args.version):
        raise ValueError("version must be a stable three-part semver")
    validate_publication_time(args.published_at)
    parsed = urlparse(args.base_url)
    if parsed.scheme != "https" or parsed.netloc != "github.com" or parsed.query or parsed.fragment:
        raise ValueError("base URL must be an HTTPS github.com release path")
    expected_path = f"{REPOSITORY_RELEASES_PATH}/v{args.version}"
    if parsed.path != expected_path:
        raise ValueError(f"base URL path must be {expected_path}")
    validate_artifact_names(
        args.version,
        args.macos,
        args.macos_signature,
        args.windows,
        args.windows_signature,
    )

    manifest = {
        "version": args.version,
        "notes": "AirWiki stable release. See the release page for details.",
        "pub_date": args.published_at,
        "platforms": {
            "macos-aarch64": {
                "signature": signature(args.macos_signature),
                "url": artifact_url(args.base_url, args.macos),
            },
            "windows-x86_64": {
                "signature": signature(args.windows_signature),
                "url": artifact_url(args.base_url, args.windows),
            },
        },
    }

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=f".{output.name}.", dir=output.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(manifest, stream, ensure_ascii=False, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, output)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise


if __name__ == "__main__":
    main()
