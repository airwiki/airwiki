#!/usr/bin/env python3
"""Validate immutable metadata in a packaged macOS AirWiki application."""

from __future__ import annotations

import argparse
import plistlib
from pathlib import Path


def verify_bundle_version(application: Path, expected_version: str) -> None:
    info_path = application / "Contents" / "Info.plist"
    try:
        with info_path.open("rb") as stream:
            info = plistlib.load(stream)
    except (OSError, plistlib.InvalidFileException) as error:
        raise ValueError("macOS application has no valid Info.plist") from error
    if not isinstance(info, dict):
        raise ValueError("macOS application Info.plist is not a dictionary")

    short_version = info.get("CFBundleShortVersionString")
    bundle_version = info.get("CFBundleVersion")
    if short_version != expected_version or bundle_version != expected_version:
        raise ValueError(
            "macOS application version metadata does not exactly match "
            f"{expected_version}"
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--application", required=True, type=Path)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    verify_bundle_version(args.application, args.version)


if __name__ == "__main__":
    main()
