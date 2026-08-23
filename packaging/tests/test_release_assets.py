from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "release_assets.py"
SPEC = importlib.util.spec_from_file_location("airwiki_release_assets", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load release asset module")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReleaseAssetTests(unittest.TestCase):
    version = "1.2.3"
    commit = "a" * 40

    def write_base_assets(self, directory: Path) -> None:
        root = SCRIPT.parents[1]
        for name in MODULE.base_asset_names(self.version) - {"latest.json"}:
            source = MODULE.LEGAL_PAYLOAD_SOURCES.get(name)
            if source is not None:
                content = (root / source).read_bytes()
            else:
                content = (
                    b"c3ludGhldGljLXNpZ25hdHVyZQ==\n"
                    if name.endswith(".sig")
                    else f"synthetic {name}\n".encode()
                )
            (directory / name).write_bytes(content)
        MODULE.atomic_json(
            directory / "latest.json",
            MODULE.expected_update_manifest(
                MODULE.regular_files(directory),
                self.version,
                "2026-08-15T12:00:00Z",
            ),
        )

    def generate(self, directory: Path) -> None:
        MODULE.generate(
            Namespace(
                directory=directory,
                version=self.version,
                commit=self.commit,
                created_at="2026-08-15T12:00:00Z",
                workflow_run="https://github.com/airwiki/airwiki/actions/runs/1",
            )
        )

    def verify(self, directory: Path) -> None:
        MODULE.verify(
            Namespace(directory=directory, version=self.version, commit=self.commit)
        )

    def rewrite_sums(self, directory: Path) -> None:
        checksummed = set(MODULE.base_asset_names(self.version)) | {
            f"airwiki-{self.version}.spdx.json",
            f"airwiki-{self.version}.provenance.json",
        }
        (directory / "SHA256SUMS").write_text(
            "".join(
                f"{MODULE.digest(directory / name)}  {name}\n"
                for name in sorted(checksummed)
            ),
            encoding="utf-8",
        )

    def test_generated_metadata_verifies_exact_assets(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)

            self.generate(directory)
            self.verify(directory)

            spdx = json.loads(
                (directory / f"airwiki-{self.version}.spdx.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertTrue(spdx["files"])
            self.assertTrue(
                all(
                    record["licenseInfoInFiles"] == ["NOASSERTION"]
                    for record in spdx["files"]
                )
            )

    def test_official_spdx_schema_rejects_a_missing_required_field(self) -> None:
        root = SCRIPT.parents[1]
        document = MODULE.spdx_document(
            root,
            {},
            self.version,
            self.commit,
            "2026-08-15T12:00:00Z",
        )
        del document["creationInfo"]

        with self.assertRaisesRegex(ValueError, "required field"):
            MODULE.validate_spdx_document(root, document)

    def test_npm_inventory_callouts_must_exist_in_the_package_table(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            inventory = Path(temporary) / "npm.md"
            inventory.write_text(
                "# Inventory\n\n"
                "## Packages\n\n"
                "| Package | Version(s) | Declared license |\n"
                "| --- | --- | --- |\n"
                "| example | 1.0.0 | MIT |\n\n"
                "## Packages without a published legal file\n\n"
                "- `missing@1.0.0`\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "package callout"):
                MODULE.parse_inventory(inventory, "npm")

    def test_platform_only_npm_packages_are_present_in_the_sbom_input(self) -> None:
        root = SCRIPT.parents[1]
        packages = MODULE.parse_inventory(
            root / "resources/licenses/NPM_LICENSES_MACOS_ARM64.md", "npm"
        )

        self.assertIn(
            "@tauri-apps/cli-darwin-arm64",
            {package["name"] for package in packages},
        )

    def test_unexpected_asset_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)
            (directory / "unexpected.bin").write_bytes(b"unexpected")

            with self.assertRaisesRegex(ValueError, "unexpected.bin"):
                self.generate(directory)

    def test_modified_asset_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)
            self.generate(directory)
            target = directory / f"AirWiki_{self.version}_aarch64.dmg"
            target.write_bytes(b"modified")

            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                self.verify(directory)

    def test_legal_payload_must_match_the_reviewed_commit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)
            self.generate(directory)
            (directory / "THIRD_PARTY_NOTICES.md").write_text(
                "modified legal notice\n", encoding="utf-8"
            )

            with self.assertRaisesRegex(ValueError, "legal payload"):
                self.verify(directory)

    def test_sbom_dependencies_must_match_reviewed_inventories(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)
            self.generate(directory)
            spdx_path = directory / f"airwiki-{self.version}.spdx.json"
            spdx = json.loads(spdx_path.read_text(encoding="utf-8"))
            spdx["packages"][1]["versionInfo"] = "9.9.9"
            MODULE.atomic_json(spdx_path, spdx)

            provenance_path = directory / f"airwiki-{self.version}.provenance.json"
            provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
            provenance["artifacts"][spdx_path.name]["sha256"] = MODULE.digest(
                spdx_path
            )
            provenance["artifacts"][spdx_path.name]["size"] = spdx_path.stat().st_size
            MODULE.atomic_json(provenance_path, provenance)
            sums = directory / "SHA256SUMS"
            checksummed = set(MODULE.base_asset_names(self.version)) | {
                spdx_path.name,
                provenance_path.name,
            }
            sums.write_text(
                "".join(
                    f"{MODULE.digest(directory / name)}  {name}\n"
                    for name in sorted(checksummed)
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "dependency inventories"):
                self.verify(directory)

    def test_updater_manifest_must_reference_the_exact_release(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)
            manifest = json.loads(
                (directory / "latest.json").read_text(encoding="utf-8")
            )
            manifest["platforms"]["darwin-aarch64"]["url"] = (
                "https://github.com/airwiki/airwiki/releases/download/v0.1.0/"
                "AirWiki.app.tar.gz"
            )
            MODULE.atomic_json(directory / "latest.json", manifest)
            self.generate(directory)

            with self.assertRaisesRegex(ValueError, "updater manifest"):
                self.verify(directory)

    def test_wrong_provenance_commit_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)
            self.generate(directory)
            provenance_path = directory / f"airwiki-{self.version}.provenance.json"
            provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
            provenance["commit"] = "b" * 40
            provenance_path.write_text(json.dumps(provenance), encoding="utf-8")
            self.rewrite_sums(directory)

            with self.assertRaisesRegex(ValueError, "provenance"):
                self.verify(directory)

    def test_provenance_size_must_match_the_final_asset(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)
            self.generate(directory)
            provenance_path = directory / f"airwiki-{self.version}.provenance.json"
            provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
            asset = f"AirWiki_{self.version}_aarch64.dmg"
            provenance["artifacts"][asset]["size"] += 1
            MODULE.atomic_json(provenance_path, provenance)
            self.rewrite_sums(directory)

            with self.assertRaisesRegex(ValueError, "provenance"):
                self.verify(directory)

    def test_provenance_workflow_run_must_identify_airwiki_actions(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.write_base_assets(directory)
            self.generate(directory)
            provenance_path = directory / f"airwiki-{self.version}.provenance.json"
            provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
            provenance["workflowRun"] = "https://example.com/actions/runs/1"
            MODULE.atomic_json(provenance_path, provenance)
            self.rewrite_sums(directory)

            with self.assertRaisesRegex(ValueError, "provenance"):
                self.verify(directory)

    def test_verified_subset_is_bound_to_the_initial_checksum_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "release"
            subset = Path(temporary) / "subset"
            directory.mkdir()
            subset.mkdir()
            self.write_base_assets(directory)
            self.generate(directory)
            asset = f"AirWiki_{self.version}_aarch64.dmg"
            (subset / asset).write_bytes((directory / asset).read_bytes())
            sums = directory / "SHA256SUMS"

            MODULE.verify_subset(
                Namespace(
                    directory=subset,
                    sums=sums,
                    sums_sha256=MODULE.digest(sums),
                    asset=[asset],
                )
            )

            with self.assertRaisesRegex(ValueError, "checksum inventory changed"):
                MODULE.verify_subset(
                    Namespace(
                        directory=subset,
                        sums=sums,
                        sums_sha256="b" * 64,
                        asset=[asset],
                    )
                )

    def test_verified_subset_rejects_modified_platform_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "release"
            subset = Path(temporary) / "subset"
            directory.mkdir()
            subset.mkdir()
            self.write_base_assets(directory)
            self.generate(directory)
            asset = f"AirWiki_{self.version}_aarch64.dmg"
            (subset / asset).write_bytes(b"modified")
            sums = directory / "SHA256SUMS"

            with self.assertRaisesRegex(ValueError, "subset digest mismatch"):
                MODULE.verify_subset(
                    Namespace(
                        directory=subset,
                        sums=sums,
                        sums_sha256=MODULE.digest(sums),
                        asset=[asset],
                    )
                )


if __name__ == "__main__":
    unittest.main()
