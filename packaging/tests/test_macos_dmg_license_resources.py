from __future__ import annotations

import importlib.util
import plistlib
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "macos_dmg_license_resources.py"
SPEC = importlib.util.spec_from_file_location(
    "airwiki_macos_dmg_license_resources", SCRIPT
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load macOS DMG license resource module")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def resource(identifier: str, name: str, data: bytes = b"resource") -> dict[str, object]:
    return {
        "Attributes": "0x0000",
        "Data": data,
        "ID": identifier,
        "Name": name,
    }


def valid_resources(content_type: str = "TEXT") -> dict[str, object]:
    return {
        "LPic": [resource("5000", "")],
        "STR#": [
            resource("5000", "English buttons"),
            resource("5002", "English"),
        ],
        content_type: [resource("5000", "English", b"license")],
        "TMPL": [resource("128", "LPic")],
        "styl": [resource("5000", "English")],
        "blkx": [
            {
                "Attributes": "0x0050",
                "Data": b"image-specific block map",
                "ID": "3",
                "Name": "disk image",
            }
        ],
        "plst": [
            {
                "Attributes": "0x0050",
                "Data": b"image-specific properties",
                "ID": "0",
                "Name": "",
            }
        ],
    }


class MacOsDmgLicenseResourceTests(unittest.TestCase):
    def test_only_license_resources_are_preserved(self) -> None:
        filtered = MODULE.filter_license_resources(valid_resources())

        self.assertEqual(
            list(filtered), ["LPic", "STR#", "TEXT", "TMPL", "styl"]
        )
        self.assertNotIn("blkx", filtered)
        self.assertNotIn("plst", filtered)

    def test_rtf_license_is_supported(self) -> None:
        filtered = MODULE.filter_license_resources(valid_resources("RTF "))

        self.assertIn("RTF ", filtered)
        self.assertNotIn("TEXT", filtered)

    def test_missing_required_resource_is_rejected(self) -> None:
        resources = valid_resources()
        del resources["LPic"]

        with self.assertRaisesRegex(ValueError, "LPic.*invalid shape"):
            MODULE.filter_license_resources(resources)

    def test_ambiguous_content_resources_are_rejected(self) -> None:
        resources = valid_resources()
        resources["RTF "] = [resource("5000", "English")]

        with self.assertRaisesRegex(ValueError, "exactly one supported"):
            MODULE.filter_license_resources(resources)

    def test_unexpected_resource_fields_are_rejected(self) -> None:
        resources = valid_resources()
        resources["LPic"][0]["Unexpected"] = "value"

        with self.assertRaisesRegex(ValueError, "unexpected fields"):
            MODULE.filter_license_resources(resources)

    def test_oversized_license_text_is_rejected(self) -> None:
        resources = valid_resources()
        resources["TEXT"][0]["Data"] = b"x" * (MODULE.MAX_RESOURCE_DATA_BYTES + 1)

        with self.assertRaisesRegex(ValueError, "exceed the size limit"):
            MODULE.filter_license_resources(resources)

    def test_invalid_plist_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "resources.plist"
            path.write_bytes(b"not a plist")

            with self.assertRaisesRegex(ValueError, "no valid resource plist"):
                MODULE.load_and_filter(path)

    def test_filtered_plist_round_trips_without_image_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            input_path = root / "all.plist"
            output_path = root / "license.plist"
            with input_path.open("wb") as stream:
                plistlib.dump(valid_resources(), stream)

            filtered = MODULE.load_and_filter(input_path)
            with output_path.open("wb") as stream:
                plistlib.dump(filtered, stream)
            with output_path.open("rb") as stream:
                written = plistlib.load(stream)

            self.assertEqual(set(written), {"LPic", "STR#", "TEXT", "TMPL", "styl"})


if __name__ == "__main__":
    unittest.main()
