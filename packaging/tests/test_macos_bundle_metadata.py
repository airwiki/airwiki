from __future__ import annotations

import importlib.util
import plistlib
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "macos_bundle_metadata.py"
SPEC = importlib.util.spec_from_file_location("airwiki_macos_bundle_metadata", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load macOS bundle metadata module")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class MacOsBundleMetadataTests(unittest.TestCase):
    def write_application(
        self, directory: Path, short_version: str, bundle_version: str
    ) -> Path:
        application = directory / "AirWiki.app"
        contents = application / "Contents"
        contents.mkdir(parents=True)
        with (contents / "Info.plist").open("wb") as stream:
            plistlib.dump(
                {
                    "CFBundleShortVersionString": short_version,
                    "CFBundleVersion": bundle_version,
                },
                stream,
            )
        return application

    def test_exact_bundle_version_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            application = self.write_application(Path(temporary), "1.2.3", "1.2.3")

            MODULE.verify_bundle_version(application, "1.2.3")

    def test_older_bundle_version_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            application = self.write_application(Path(temporary), "1.2.2", "1.2.2")

            with self.assertRaisesRegex(ValueError, "does not exactly match"):
                MODULE.verify_bundle_version(application, "1.2.3")

    def test_mismatched_build_version_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            application = self.write_application(Path(temporary), "1.2.3", "99")

            with self.assertRaisesRegex(ValueError, "does not exactly match"):
                MODULE.verify_bundle_version(application, "1.2.3")


if __name__ == "__main__":
    unittest.main()
