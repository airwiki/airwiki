from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "generate-update-manifest.py"
SPEC = importlib.util.spec_from_file_location("airwiki_update_manifest", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load updater manifest module")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class UpdateManifestTests(unittest.TestCase):
    def test_tauri_platform_keys_match_the_runtime_targets(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            macos = root / "AirWiki.app.tar.gz"
            macos_signature = root / "AirWiki.app.tar.gz.sig"
            windows = root / "AirWiki_1.2.3_x64_en-US.msi"
            windows_signature = root / "AirWiki_1.2.3_x64_en-US.msi.sig"
            macos_signature.write_text("mac-signature", encoding="utf-8")
            windows_signature.write_text("windows-signature", encoding="utf-8")

            manifest = MODULE.update_manifest(
                "1.2.3",
                "2026-08-15T12:00:00Z",
                "https://github.com/airwiki/airwiki/releases/download/v1.2.3",
                macos,
                macos_signature,
                windows,
                windows_signature,
            )

        self.assertEqual(
            set(manifest["platforms"]), {"darwin-aarch64", "windows-x86_64"}
        )

    def test_publication_time_requires_utc(self) -> None:
        with self.assertRaisesRegex(ValueError, "UTC"):
            MODULE.validate_publication_time("2026-08-15T12:00:00+01:00")

    def test_artifact_url_escapes_the_exact_filename(self) -> None:
        url = MODULE.artifact_url(
            "https://github.com/airwiki/airwiki/releases/download/v1.2.3",
            Path("AirWiki 1.2.3.dmg"),
        )

        self.assertEqual(
            url,
            "https://github.com/airwiki/airwiki/releases/download/v1.2.3/"
            "AirWiki%201.2.3.dmg",
        )

    def test_signature_rejects_empty_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "artifact.sig"
            path.write_text("", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "invalid updater signature"):
                MODULE.signature(path)

    def test_artifact_names_reject_a_noncanonical_windows_updater(self) -> None:
        with self.assertRaisesRegex(ValueError, "exact stable artifact names"):
            MODULE.validate_artifact_names(
                "1.2.3",
                Path("AirWiki.app.tar.gz"),
                Path("AirWiki.app.tar.gz.sig"),
                Path("AirWiki_1.2.3_x64_es-ES.msi"),
                Path("AirWiki_1.2.3_x64_es-ES.msi.sig"),
            )


if __name__ == "__main__":
    unittest.main()
