from __future__ import annotations

import importlib.util
import json
import re
import sys
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "platform_release_candidate.py"
sys.path.insert(0, str(SCRIPT.parent))
SPEC = importlib.util.spec_from_file_location("airwiki_platform_release_candidate", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load platform release candidate module")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
WORKFLOW = SCRIPT.parents[1] / ".github/workflows/package-platform-rc.yml"


class PlatformReleaseCandidateTests(unittest.TestCase):
    version = "1.2.3"
    rc_number = "2"
    commit = "a" * 40

    def inputs(self, root: Path) -> tuple[Path, Path, Path, Path]:
        macos = root / f"AirWiki_{self.version}_aarch64.dmg"
        macos.write_bytes(b"synthetic-dmg" + b"\0" * (512 - len(b"synthetic-dmg")))
        with macos.open("r+b") as stream:
            stream.seek(-512, 2)
            stream.write(b"koly")
        windows_en = root / f"AirWiki_{self.version}_x64_en-US.msi"
        windows_es = root / f"AirWiki_{self.version}_x64_es-ES.msi"
        windows_en.write_bytes(MODULE.technical.MSI_MAGIC + b"synthetic-en")
        windows_es.write_bytes(MODULE.technical.MSI_MAGIC + b"synthetic-es")
        receipt = root / "receipt.json"
        receipt.write_text(json.dumps({
            "schema": MODULE.RECEIPT_SCHEMA,
            "commit": self.commit,
            "version": self.version,
            "verification": MODULE.RECEIPT_VERIFICATION,
            "artifacts": [
                {"name": macos.name, "sha256": MODULE.technical.sha256(macos)},
            ],
        }), encoding="utf-8")
        return macos, receipt, windows_en, windows_es

    def prepare(self, inputs: Path, output: Path) -> None:
        macos = inputs / f"AirWiki_{self.version}_aarch64.dmg"
        receipt = inputs / "receipt.json"
        windows_en = inputs / f"AirWiki_{self.version}_x64_en-US.msi"
        windows_es = inputs / f"AirWiki_{self.version}_x64_es-ES.msi"
        if not macos.exists():
            macos, receipt, windows_en, windows_es = self.inputs(inputs)
        MODULE.prepare(Namespace(
            input_root=inputs, macos_dmg=macos, macos_receipt=receipt,
            windows_en=windows_en, windows_es=windows_es, output=output,
            version=self.version, rc_number=self.rc_number, commit=self.commit,
            repository=MODULE.REPOSITORY,
            workflow_run="https://github.com/airwiki/airwiki/actions/runs/123",
            generated_at="2026-09-02T12:00:00Z",
        ))

    def rewrite_sums(self, assets: Path) -> None:
        files = MODULE.technical.regular_files(assets)
        (assets / "SHA256SUMS.txt").write_text("".join(
            f"{MODULE.technical.sha256(path)}  {name}\n"
            for name, path in sorted(files.items()) if name != "SHA256SUMS.txt"
        ), encoding="utf-8")

    def test_prepare_and_verify_preserves_the_split_platform_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs, output = root / "inputs", root / "output"
            inputs.mkdir()
            self.prepare(inputs, output)
            MODULE.verify_output(output)
            assets = output / "assets"
            self.assertEqual(set(entry.name for entry in output.iterdir()), {"assets", "RELEASE_NOTES.md"})
            self.assertEqual(set(entry.name for entry in assets.iterdir()), MODULE.release_asset_names(self.version))
            provenance = json.loads((assets / "PROVENANCE.json").read_text())
            self.assertEqual(provenance["tag"], "v1.2.3-rc.2")
            self.assertFalse(provenance["supportedPublicRelease"])
            self.assertFalse(provenance["updaterChannel"])
            self.assertFalse(provenance["latest"])
            self.assertEqual(provenance["platforms"]["macos-arm64"]["nativeSignature"], "developer-id")
            self.assertTrue(provenance["platforms"]["macos-arm64"]["notarized"])
            self.assertEqual(provenance["platforms"]["windows-x64-en-US"]["authenticode"], "not-signed")

    def test_bilingual_notices_and_notes_make_no_stable_or_updater_claim(self) -> None:
        notice = MODULE.notice(self.version, self.rc_number)
        notes = MODULE.release_notes(self.version, self.rc_number, self.commit)
        self.assertIn("not a stable release", notice)
        self.assertIn("nunca es Latest", notice)
        self.assertIn("Never disable protections", notice)
        self.assertIn("Nunca desactives", notice)
        self.assertIn("Developer ID signed and notarized", notes)
        self.assertIn("unsigned technical", notes)
        self.assertIn("package-platform-rc.yml", notes)
        self.assertIn("never selected by the updater", notes)
        self.assertIn("## Español", notes)
        self.assertIn("nunca las desactives", notes)
        self.assertIn("assets as immutable", notes)
        self.assertIn("archivos publicados son inmutables", notes)

    def test_receipt_digest_mismatch_is_rejected_before_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            _, receipt, _, _ = self.inputs(inputs)
            value = json.loads(receipt.read_text())
            value["artifacts"][0]["sha256"] = "d" * 64
            receipt.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "DMG digest"):
                self.prepare(inputs, root / "output")

    def test_receipt_schema_duplicate_extra_and_wrong_name_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            _, receipt, _, _ = self.inputs(inputs)
            value = json.loads(receipt.read_text())
            value["schema"] = "airwiki-macos-notarization-rehearsal-receipt-v1"
            receipt.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "does not match"):
                self.prepare(inputs, root / "output")
            value["schema"] = MODULE.RECEIPT_SCHEMA
            value["artifacts"].append(dict(value["artifacts"][0]))
            receipt.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "artifact list"):
                self.prepare(inputs, root / "duplicate")
            value["artifacts"] = [{"name": "wrong.dmg", "sha256": "a" * 64}]
            receipt.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "artifact is invalid"):
                self.prepare(inputs, root / "wrong-name")

    def test_symlink_and_unexpected_assets_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs, output = root / "inputs", root / "output"
            inputs.mkdir()
            macos, _, windows_en, _ = self.inputs(inputs)
            link = inputs / "linked.msi"
            link.symlink_to(windows_en)
            with self.assertRaisesRegex(ValueError, "symlink"):
                MODULE.technical.require_input(inputs, link, MODULE.technical.MAX_MSI_BYTES)
            self.assertTrue(macos.is_file())
            self.prepare(inputs, output)
            (output / "assets" / "extra.bin").write_bytes(b"extra")
            with self.assertRaisesRegex(ValueError, "asset set"):
                MODULE.verify_output(output)

    def test_output_parent_symlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            real = root / "real"
            real.mkdir()
            linked = root / "linked"
            linked.symlink_to(real, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "contains a symlink"):
                MODULE.reject_symlink_ancestors(linked / "output")

    def test_modified_asset_digest_sums_and_weak_boundary_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs, output = root / "inputs", root / "output"
            inputs.mkdir()
            self.prepare(inputs, output)
            asset = output / "assets" / MODULE.primary_asset_names(self.version)["windows_en"]
            with asset.open("ab") as stream:
                stream.write(b"changed")
            with self.assertRaisesRegex(ValueError, "provenance"):
                MODULE.verify_output(output)
            self.prepare(inputs, root / "other")
            provenance = root / "other" / "assets" / "PROVENANCE.json"
            value = json.loads(provenance.read_text())
            value["latest"] = True
            MODULE.technical.atomic_json(provenance, value)
            self.rewrite_sums(root / "other" / "assets")
            with self.assertRaisesRegex(ValueError, "weakens"):
                MODULE.verify_output(root / "other")
            self.prepare(inputs, root / "sums")
            sums = root / "sums" / "assets" / "SHA256SUMS.txt"
            sums.write_text(sums.read_text().replace("a", "b", 1), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "SHA256SUMS|digest"):
                MODULE.verify_output(root / "sums")

    def test_supported_and_updater_flags_are_rejected_independently(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            for field in ("supportedPublicRelease", "updaterChannel"):
                output = root / field
                self.prepare(inputs, output)
                provenance = output / "assets" / "PROVENANCE.json"
                value = json.loads(provenance.read_text())
                value[field] = True
                MODULE.technical.atomic_json(provenance, value)
                self.rewrite_sums(output / "assets")
                with self.assertRaisesRegex(ValueError, "weakens"):
                    MODULE.verify_output(output)

    def test_json_boolean_integer_coercion_cannot_weaken_provenance(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs = root / "inputs"
            inputs.mkdir()
            for field in ("schemaVersion", "platforms"):
                output = root / field
                self.prepare(inputs, output)
                provenance = output / "assets" / "PROVENANCE.json"
                value = json.loads(provenance.read_text())
                if field == "schemaVersion":
                    value[field] = True
                else:
                    value[field]["macos-arm64"]["desktopApplication"] = 1
                MODULE.technical.atomic_json(provenance, value)
                self.rewrite_sums(output / "assets")
                with self.assertRaisesRegex(ValueError, "weakens"):
                    MODULE.verify_output(output)

    def test_bad_metadata_and_existing_output_are_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "RC number"):
            MODULE.validate_metadata(self.version, "0", self.commit, MODULE.REPOSITORY,
                                     "https://github.com/airwiki/airwiki/actions/runs/1", "2026-09-02T12:00:00Z")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            inputs, output = root / "inputs", root / "output"
            inputs.mkdir()
            output.mkdir()
            with self.assertRaisesRegex(ValueError, "already exists"):
                self.prepare(inputs, output)


class PlatformReleaseCandidateWorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.workflow = WORKFLOW.read_text(encoding="utf-8")
        self.publish = self.workflow.split("  publish-platform-rc:", 1)[1]

    def test_two_protected_approvals_and_exact_checks_are_required(self) -> None:
        self.assertEqual(self.workflow.count("environment: macos-signing"), 1)
        self.assertEqual(self.workflow.count("environment: public-release"), 1)
        for check in (
            "Frontend checks (macos-arm64)",
            "Frontend checks (windows-x64)",
            "Rust checks (macos-14)",
            "Rust checks (windows-2022)",
            "Advisories, licenses and sources",
            "Launch site checks",
        ):
            self.assertEqual(self.workflow.count(check), 1)
        self.assertEqual(self.workflow.count("gh api --paginate"), 5)
        self.assertEqual(
            self.workflow.count(
                "git fetch --no-tags origin "
                "+refs/heads/main:refs/remotes/origin/main"
            ),
            6,
        )

    def test_windows_beta_keeps_unsigned_and_installed_smoke_boundaries(self) -> None:
        windows = self.workflow.split("  windows-x64-unsigned-beta:", 1)[1].split(
            "  publish-platform-rc:", 1
        )[0]
        self.assertIn("test-smoke-windows-msi.ps1", windows)
        self.assertIn("-AuthorizeDestructiveMsiSmoke", windows)
        self.assertIn("SignatureStatus]::NotSigned", windows)
        self.assertIn("prepare-unsigned-windows-beta.ps1", windows)

    def test_publication_is_closed_non_latest_and_updater_free(self) -> None:
        self.assertIn("platform_release_candidate.py prepare", self.publish)
        self.assertIn("platform_release_candidate.py verify", self.publish)
        self.assertIn("--draft --prerelease --latest=false", self.publish)
        self.assertIn("-f make_latest=false", self.publish)
        self.assertIn("isImmutable", self.publish)
        self.assertNotIn("latest.json", self.publish)
        self.assertNotIn("AirWiki.app.tar.gz", self.publish)
        self.assertNotIn("linux", self.publish.lower())

    def test_final_tag_is_bound_only_after_download_and_attestation_verification(self) -> None:
        closed_set_index = self.publish.index("platform_release_candidate.py verify")
        attestation_index = self.publish.index("gh attestation verify")
        final_revalidation_index = self.publish.index(
            "platform RC source changed immediately before publication"
        )
        tag_creation_index = self.publish.index(
            'gh api --method POST "repos/$GITHUB_REPOSITORY/git/refs"'
        )
        publication_index = self.publish.index(
            'gh api --method PATCH "repos/$GITHUB_REPOSITORY/releases/$RELEASE_ID"'
        )
        self.assertLess(closed_set_index, attestation_index)
        self.assertLess(attestation_index, final_revalidation_index)
        self.assertLess(final_revalidation_index, tag_creation_index)
        self.assertLess(tag_creation_index, publication_index)

    def test_every_external_action_is_pinned_to_a_commit(self) -> None:
        references = re.findall(r"uses:\s*[^@\s]+@([^\s]+)", self.workflow)
        self.assertTrue(references)
        self.assertTrue(
            all(re.fullmatch(r"[0-9a-f]{40}", value) for value in references)
        )


if __name__ == "__main__":
    unittest.main()
