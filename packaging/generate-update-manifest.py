#!/usr/bin/env python3
"""Create the stable cargo-packager updater manifest from final artifacts."""

from __future__ import annotations

import argparse
import json
import os
import re
import tempfile
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
    parsed = urlparse(args.base_url)
    if parsed.scheme != "https" or parsed.netloc != "github.com" or parsed.query or parsed.fragment:
        raise ValueError("base URL must be an HTTPS github.com release path")
    expected_path = f"{REPOSITORY_RELEASES_PATH}/v{args.version}"
    if parsed.path != expected_path:
        raise ValueError(f"base URL path must be {expected_path}")

    manifest = {
        "version": args.version,
        "notes": "AirWiki stable release. See the release page for details.",
        "pub_date": args.published_at,
        "platforms": {
            "macos-aarch64": {
                "signature": signature(args.macos_signature),
                "url": artifact_url(args.base_url, args.macos),
                "format": "app",
            },
            "windows-x86_64": {
                "signature": signature(args.windows_signature),
                "url": artifact_url(args.base_url, args.windows),
                "format": "nsis",
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
