from __future__ import annotations

import importlib.util
import json
import re
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "technical_prerelease.py"
SPEC = importlib.util.spec_from_file_location("airwiki_technical_prerelease", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load technical prerelease module")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
WORKFLOW = SCRIPT.parents[1] / ".github/workflows/package-pilot.yml"


class TechnicalPrereleaseTests(unittest.TestCase):
    version = "1.2.3"
    beta_number = "4"
    commit = "a" * 40

    def inputs(self, root: Path) -> tuple[Path, Path, Path, Path]:
        macos = root / f"AirWiki_{self.version}_aarch64.dmg"
        macos.write_bytes(b"synthetic-dmg" + b"\0" * (512 - len(b"synthetic-dmg")))
        with macos.open("r+b") as stream:
            stream.seek(-512, 2)
            stream.write(b"koly")
        windows_en = root / f"AirWiki_{self.version}_x64_en-US.msi"
        windows_es = root / f"AirWiki_{self.version}_x64_es-ES.msi"
        windows_en.write_bytes(MODULE.MSI_MAGIC + b"synthetic-en")
        windows_es.write_bytes(MODULE.MSI_MAGIC + b"synthetic-es")
        linux = root / "airwiki-federation-index"
        header = bytearray(20)
        header[:4] = MODULE.ELF_MAGIC
        header[4] = 2
        header[5] = 1
        header[18:20] = b"\x3e\x00"
        linux.write_bytes(bytes(header) + b"synthetic-linux")
        return macos, windows_en, windows_es, linux

    def prepare(self, input_root: Path, output: Path) -> None:
        macos, windows_en, windows_es, linux = self.inputs(input_root)
        MODULE.prepare(
            Namespace(
                input_root=input_root,
                macos_dmg=macos,
                windows_en=windows_en,
                windows_es=windows_es,
                linux_binary=linux,
                output=output,
                version=self.version,
                beta_number=self.beta_number,
                commit=self.commit,
                repository=MODULE.REPOSITORY,
                workflow_run="https://github.com/airwiki/airwiki/actions/runs/123",
                generated_at="2026-08-25T12:00:00Z",
            )
        )

    def rewrite_sums(self, assets: Path) -> None:
        files = MODULE.regular_files(assets)
        (assets / "SHA256SUMS.txt").write_text(
            "".join(
                f"{MODULE.sha256(path)}  {name}\n"
                for name, path in sorted(files.items())
                if name != "SHA256SUMS.txt"
            ),
            encoding="utf-8",
        )

    def test_prepared_assets_verify_the_closed_public_beta_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"
            inputs = root / "inputs"
            inputs.mkdir()
            self.prepare(inputs, output)

            MODULE.verify_output(output)
            provenance = json.loads(
                (output / "assets" / "PROVENANCE.json").read_text(encoding="utf-8")
            )
            self.assertFalse(provenance["supportedPublicRelease"])
            self.assertFalse(provenance["updaterChannel"])
            self.assertFalse(provenance["latest"])
            self.assertFalse(provenance["platforms"]["linux-x64"]["desktopApplication"])

    def test_notice_and_release_notes_are_explicit_about_every_platform(self) -> None:
        notice = MODULE.notice(self.version, self.beta_number)
        notes = MODULE.release_notes(self.version, self.beta_number, self.commit)
        self.assertIn("intentionally unsigned", notice)
        self.assertIn("not notarized", notice)
        self.assertIn("not the AirWiki desktop application", notice)
        self.assertNotIn(
            "latest", MODULE.prerelease_tag(self.version, self.beta_number)
        )
        self.assertIn("not available for Linux", notes)

    def test_symlinked_input_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            macos, windows_en, windows_es, linux = self.inputs(inputs)
            linked = inputs / "linked.msi"
            linked.symlink_to(windows_en)
            with self.assertRaisesRegex(ValueError, "symlink"):
                MODULE.require_input(inputs, linked, MODULE.MAX_MSI_BYTES)
            self.assertTrue(
                macos.is_file() and windows_es.is_file() and linux.is_file()
            )

    def test_input_escape_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            outside = root / "outside.msi"
            outside.write_bytes(MODULE.MSI_MAGIC)
            with self.assertRaisesRegex(ValueError, "inside the input root"):
                MODULE.require_input(inputs, outside, MODULE.MAX_MSI_BYTES)

    def test_wrong_native_formats_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bad = root / "bad"
            bad.write_bytes(b"not-a-package")
            with self.assertRaisesRegex(ValueError, "MSI"):
                MODULE.validate_msi(bad)
            with self.assertRaisesRegex(ValueError, "ELF"):
                MODULE.validate_linux_binary(bad)
            with self.assertRaisesRegex(ValueError, "DMG"):
                MODULE.validate_dmg(bad)

    def test_modified_asset_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            output = root / "output"
            self.prepare(inputs, output)
            name = MODULE.primary_asset_names(self.version)["windows_en"]
            with (output / "assets" / name).open("ab") as stream:
                stream.write(b"modified")
            with self.assertRaisesRegex(ValueError, "provenance|digest"):
                MODULE.verify_output(output)

    def test_unexpected_asset_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            output = root / "output"
            self.prepare(inputs, output)
            (output / "assets" / "unexpected.bin").write_bytes(b"unexpected")
            with self.assertRaisesRegex(ValueError, "unexpected.bin"):
                MODULE.verify_output(output)

    def test_provenance_cannot_claim_stable_or_updater_status(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            output = root / "output"
            self.prepare(inputs, output)
            provenance_path = output / "assets" / "PROVENANCE.json"
            provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
            provenance["supportedPublicRelease"] = True
            MODULE.atomic_json(provenance_path, provenance)
            self.rewrite_sums(output / "assets")
            with self.assertRaisesRegex(ValueError, "weakens the release boundary"):
                MODULE.verify_output(output)

    def test_official_repository_and_run_are_required(self) -> None:
        with self.assertRaisesRegex(ValueError, "restricted"):
            MODULE.validate_metadata(
                self.version,
                self.beta_number,
                self.commit,
                "fork/airwiki",
                "https://github.com/airwiki/airwiki/actions/runs/1",
                "2026-08-25T12:00:00Z",
            )
        with self.assertRaisesRegex(ValueError, "official"):
            MODULE.validate_metadata(
                self.version,
                self.beta_number,
                self.commit,
                MODULE.REPOSITORY,
                "https://example.com/run/1",
                "2026-08-25T12:00:00Z",
            )

    def test_existing_output_is_not_replaced(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            output = root / "output"
            output.mkdir()
            with self.assertRaisesRegex(ValueError, "already exists"):
                self.prepare(inputs, output)


class TechnicalPrereleaseWorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_publication_is_protected_non_latest_and_updater_free(self) -> None:
        self.assertIn("- public-prerelease", self.workflow)
        self.assertIn("environment: public-release", self.workflow)
        self.assertEqual(self.workflow.count("contents: write"), 1)
        self.assertIn("--prerelease", self.workflow)
        self.assertIn("--latest=false", self.workflow)
        self.assertIn("-f make_latest=false", self.workflow)
        self.assertNotIn("latest.json", self.workflow)

    def test_publication_depends_on_every_platform_and_reverification(self) -> None:
        publish = self.workflow.split("  publish-technical-prerelease:", 1)[1]
        for job in (
            "linux-x64-federation-index",
            "macos-arm64",
            "windows-x64",
        ):
            self.assertIn(f"      - {job}", publish)
        self.assertIn("technical_prerelease.py prepare", publish)
        self.assertIn("gh release download", publish)
        self.assertIn("technical_prerelease.py verify", publish)

    def test_tag_is_bound_after_draft_reverification_before_publication(self) -> None:
        publish = self.workflow.split("  publish-technical-prerelease:", 1)[1]
        premature_tag_check = (
            '          git fetch --no-tags origin "refs/tags/$TAG:refs/tags/$TAG"\n'
            '          if [[ "$(git rev-list -n 1 "$TAG")" != "$GITHUB_SHA" ]]'
        )
        self.assertNotIn(premature_tag_check, publish)

        verify_index = publish.index("technical_prerelease.py verify")
        tag_creation_index = publish.index(
            '"repos/$GITHUB_REPOSITORY/git/refs"'
        )
        release_publication_index = publish.index(
            '"repos/$GITHUB_REPOSITORY/releases/$RELEASE_ID"'
        )
        self.assertLess(verify_index, tag_creation_index)
        self.assertLess(tag_creation_index, release_publication_index)
        self.assertIn("published_commit=", publish[release_publication_index:])

    def test_every_external_action_is_pinned_to_a_commit(self) -> None:
        references = re.findall(r"uses:\s*[^@\s]+@([^\s]+)", self.workflow)
        self.assertTrue(references)
        self.assertTrue(
            all(re.fullmatch(r"[0-9a-f]{40}", value) for value in references)
        )


if __name__ == "__main__":
    unittest.main()
