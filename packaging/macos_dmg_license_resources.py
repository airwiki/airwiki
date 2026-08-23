#!/usr/bin/env python3
"""Extract the license agreement resources from a macOS DMG resource plist."""

from __future__ import annotations

import argparse
import plistlib
import sys
from pathlib import Path
from typing import Any


MAX_RESOURCE_DATA_BYTES = 1024 * 1024
RESOURCE_IDENTITIES = {
    "LPic": (("5000", ""),),
    "STR#": (("5000", "English buttons"), ("5002", "English")),
    "TMPL": (("128", "LPic"),),
    "styl": (("5000", "English"),),
}
CONTENT_RESOURCE_IDENTITIES = (("5000", "English"),)


def _validate_resource_group(
    resource_type: str,
    entries: Any,
    expected_identities: tuple[tuple[str, str], ...],
) -> list[dict[str, Any]]:
    if not isinstance(entries, list) or len(entries) != len(expected_identities):
        raise ValueError(f"DMG {resource_type} license resources have an invalid shape")

    actual_identities: list[tuple[str, str]] = []
    total_data_bytes = 0
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {
            "Attributes",
            "Data",
            "ID",
            "Name",
        }:
            raise ValueError(
                f"DMG {resource_type} license resource has unexpected fields"
            )
        if entry["Attributes"] != "0x0000":
            raise ValueError(
                f"DMG {resource_type} license resource has unexpected attributes"
            )
        if not isinstance(entry["ID"], str) or not isinstance(entry["Name"], str):
            raise ValueError(
                f"DMG {resource_type} license resource identity is invalid"
            )
        if not isinstance(entry["Data"], bytes):
            raise ValueError(f"DMG {resource_type} license resource data is invalid")
        total_data_bytes += len(entry["Data"])
        actual_identities.append((entry["ID"], entry["Name"]))

    if tuple(actual_identities) != expected_identities:
        raise ValueError(f"DMG {resource_type} license resource identity is invalid")
    if total_data_bytes > MAX_RESOURCE_DATA_BYTES:
        raise ValueError(f"DMG {resource_type} license resources exceed the size limit")

    return entries


def filter_license_resources(resources: Any) -> dict[str, list[dict[str, Any]]]:
    if not isinstance(resources, dict):
        raise ValueError("DMG resource plist is not a dictionary")

    content_types = [kind for kind in ("TEXT", "RTF ") if kind in resources]
    if len(content_types) != 1:
        raise ValueError("DMG must contain exactly one supported license text resource")

    filtered: dict[str, list[dict[str, Any]]] = {}
    for resource_type in ("LPic", "STR#"):
        filtered[resource_type] = _validate_resource_group(
            resource_type,
            resources.get(resource_type),
            RESOURCE_IDENTITIES[resource_type],
        )

    content_type = content_types[0]
    filtered[content_type] = _validate_resource_group(
        content_type,
        resources[content_type],
        CONTENT_RESOURCE_IDENTITIES,
    )

    for resource_type in ("TMPL", "styl"):
        filtered[resource_type] = _validate_resource_group(
            resource_type,
            resources.get(resource_type),
            RESOURCE_IDENTITIES[resource_type],
        )

    return filtered


def load_and_filter(input_path: Path) -> dict[str, list[dict[str, Any]]]:
    try:
        if input_path == Path("-"):
            resources = plistlib.loads(sys.stdin.buffer.read())
        else:
            with input_path.open("rb") as stream:
                resources = plistlib.load(stream)
    except (OSError, plistlib.InvalidFileException) as error:
        raise ValueError("DMG has no valid resource plist") from error
    return filter_license_resources(resources)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    resources = load_and_filter(args.input)
    if args.output is not None:
        with args.output.open("wb") as stream:
            plistlib.dump(resources, stream, fmt=plistlib.FMT_XML, sort_keys=False)


if __name__ == "__main__":
    main()
