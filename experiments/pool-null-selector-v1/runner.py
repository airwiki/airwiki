#!/usr/bin/env python3
"""Auditable, fail-closed runner for the preregistered pool-null experiment."""

from __future__ import annotations

import argparse
import errno
import hashlib
import importlib.metadata
import json
import math
import os
import platform
import random
import shutil
import socket
import struct
import sys
import tempfile
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Literal


REPO_ROOT = Path(__file__).resolve().parents[2]
EXPERIMENT_ROOT = REPO_ROOT / "experiments" / "pool-null-selector-v1"
RUNNER_PATH = EXPERIMENT_ROOT / "runner.py"
EVIDENCE_ROOT = EXPERIMENT_ROOT / "evidence"
ARTIFACT_ROOT = REPO_ROOT / "target" / "pool-null-experiment"
MODEL_ROOT = ARTIFACT_ROOT / "base-model"
MODEL_MANIFEST_PATH = ARTIFACT_ROOT / "base-model-manifest.json"
ATTEMPT_PATH = EVIDENCE_ROOT / "compatibility-attempt.json"
REPORT_PATH = EVIDENCE_ROOT / "compatibility-report.json"
MODEL_REPO = "cross-encoder/mmarco-mMiniLMv2-L12-H384-v1"
MODEL_REVISION = "1427fd652930e4ba29e8149678df786c240d8825"
EXPERIMENT_ID = "pool-null-selector-v1-compatibility-v1"
SEED = 4_057_359_121
MAX_LENGTH = 512
EPOCHS = 3
LEARNING_RATE = 2e-5
WEIGHT_DECAY = 0.01
WARMUP_STEPS = 36
TRAINING_STEPS = 360
MAX_GRAD_NORM = 1.0
RELATIVE_WINDOW = 3.6
MAX_POOL_SIZE = 10
HIGH_RISK_KINDS = frozenset(
    {
        "wrong_entity",
        "wrong_relation",
        "wrong_date",
        "wrong_version",
        "wrong_scope",
        "negation",
        "absent_fact",
        "ambiguity",
        "metadata_only",
        "unrelated_injection",
    }
)
ALL_NEGATIVE_KINDS = HIGH_RISK_KINDS | {"random"}

CORPUS_FILES = {
    "queries.jsonl": "41d6b1a2c093a920339081f4f2c616e81027e7f69409673f7c511167ecf61c4f",
    "passages.jsonl": "3418cf2e5604894800da388ba6e41afc0e0f620c9f64173f4ac1f321b4559696",
    "judgments.jsonl": "f1d66311a1b799452564a25407ae54980b89d86f8558c255e7be6b28347eee6e",
}
MODEL_FILES = {
    "config.json": "cc2cfe51aa3fd759d21d21acf5dfd6994aa67a3c9210636d22e143699d336c77",
    "model.safetensors": "5daeca2481a76b5976a2bdc32f0a78532b6716da4f8cd3ff59460ef8d2f359b4",
    "sentencepiece.bpe.model": "cfc8146abe2a0488e9e2a0c56de7952f7c11ab059eca145a0a727afce0db2865",
    "special_tokens_map.json": "378eb3bf733eb16e65792d7e3fda5b8a4631387ca04d2015199c4d4f22ae554d",
    "tokenizer.json": "62c24cdc13d4c9952d63718d6c9fa4c287974249e16b7ade6d5a85e7bbb75626",
    "tokenizer_config.json": "e7fbfbfa6347b4e414c1cee50d142e2c2f9a895dad68b068ae83a8b564c3837e",
}
CHECKPOINT_FILES = frozenset({"config.json", "model.safetensors"})
EXPECTED_VERSIONS = {
    "python": "3.13.6",
    "torch": "2.7.1",
    "transformers": "4.56.1",
    "tokenizers": "0.22.0",
    "safetensors": "0.6.2",
    "numpy": "2.3.5",
}
REQUIRED_PROCESS_ENVIRONMENT = {
    "PYTHONHASHSEED": str(SEED),
    "OMP_NUM_THREADS": "4",
    "MKL_NUM_THREADS": "4",
    "TOKENIZERS_PARALLELISM": "false",
    "HF_HUB_OFFLINE": "1",
    "TRANSFORMERS_OFFLINE": "1",
    "HF_HUB_DISABLE_TELEMETRY": "1",
}
NETWORK_SANDBOX_PROFILE = "(version 1) (allow default) (deny network*)"
NETWORK_SANDBOX_MARKER = "AIRWIKI_POOL_NULL_SANDBOX"

Role = Literal["answer", "support", "hard_negative"]
Arm = Literal["arm_a", "arm_b"]


class ExperimentError(RuntimeError):
    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


@dataclass(frozen=True)
class Candidate:
    passage_id: str
    passage_language: str
    passage: str
    role: Role
    negative_kind: str


@dataclass(frozen=True)
class Pool:
    query_id: str
    query_language: str
    question: str
    split: str
    candidates: tuple[Candidate, ...]

    @property
    def direction(self) -> str:
        languages = {candidate.passage_language for candidate in self.candidates}
        if len(languages) != 1:
            raise ExperimentError("mixed_passage_languages")
        return f"{self.query_language}-to-{next(iter(languages))}"

    @property
    def answerable(self) -> bool:
        return any(candidate.role == "answer" for candidate in self.candidates)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fsync_directory(path: Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise ExperimentError("state_sync_failed") from error
    try:
        os.fsync(descriptor)
    except OSError as error:
        raise ExperimentError("state_sync_failed") from error
    finally:
        os.close(descriptor)


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        fsync_directory(path.parent)
    finally:
        temporary.unlink(missing_ok=True)


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                raise ExperimentError("blank_jsonl_row")
            value = json.loads(line)
            if not isinstance(value, dict):
                raise ExperimentError("invalid_jsonl_row")
            rows.append(value)
    return rows


def require_exact_keys(value: dict[str, Any], expected: set[str], code: str) -> None:
    if set(value) != expected:
        raise ExperimentError(code)


def verify_hashes(root: Path, expected: dict[str, str], code: str) -> dict[str, str]:
    observed: dict[str, str] = {}
    for relative, expected_hash in expected.items():
        path = root / relative
        if not path.is_file() or path.is_symlink():
            raise ExperimentError(code)
        observed_hash = sha256_file(path)
        if observed_hash != expected_hash:
            raise ExperimentError(code)
        observed[relative] = observed_hash
    return observed


def verify_exact_hashes(
    root: Path, expected: dict[str, str], code: str
) -> dict[str, str]:
    if not root.is_dir() or root.is_symlink():
        raise ExperimentError(code)
    expected_directories = {
        parent.as_posix()
        for relative in expected
        for parent in Path(relative).parents
        if parent != Path(".")
    }
    observed_paths: set[str] = set()
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ExperimentError(code)
        if path.is_dir():
            if path.relative_to(root).as_posix() not in expected_directories:
                raise ExperimentError(code)
            continue
        if not path.is_file():
            raise ExperimentError(code)
        observed_paths.add(path.relative_to(root).as_posix())
    if observed_paths != set(expected):
        raise ExperimentError(code)
    return verify_hashes(root, expected, code)


def hash_exact_files(
    root: Path, expected_paths: set[str] | frozenset[str], code: str
) -> dict[str, str]:
    if not root.is_dir() or root.is_symlink():
        raise ExperimentError(code)
    expected_directories = {
        parent.as_posix()
        for relative in expected_paths
        for parent in Path(relative).parents
        if parent != Path(".")
    }
    observed_paths: set[str] = set()
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ExperimentError(code)
        if path.is_dir():
            if path.relative_to(root).as_posix() not in expected_directories:
                raise ExperimentError(code)
            continue
        if not path.is_file():
            raise ExperimentError(code)
        observed_paths.add(path.relative_to(root).as_posix())
    if observed_paths != set(expected_paths):
        raise ExperimentError(code)
    return {
        relative: sha256_file(root / relative) for relative in sorted(observed_paths)
    }


def require_canonical_runner() -> None:
    source = Path(__file__)
    if source.is_symlink() or source.resolve() != RUNNER_PATH.resolve():
        raise ExperimentError("runner_location_mismatch")


def require_safe_repository_paths() -> None:
    if not EXPERIMENT_ROOT.is_dir() or not EVIDENCE_ROOT.is_dir():
        raise ExperimentError("repository_layout_mismatch")
    for path in (EXPERIMENT_ROOT, EVIDENCE_ROOT, REPO_ROOT / "target", ARTIFACT_ROOT):
        current = path
        while current != REPO_ROOT:
            if current.is_symlink():
                raise ExperimentError("repository_layout_mismatch")
            current = current.parent


def network_isolation_active() -> bool:
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            probe.settimeout(0.1)
            probe.connect(("127.0.0.1", 9))
    except PermissionError as error:
        return error.errno in {errno.EACCES, errno.EPERM}
    except OSError:
        return False
    return False


def ensure_network_isolation(command: str) -> None:
    if command not in {"network-self-test", "run-compatibility"}:
        return
    if os.environ.get(NETWORK_SANDBOX_MARKER) == EXPERIMENT_ID:
        if not network_isolation_active():
            raise ExperimentError("network_isolation_unavailable")
        return
    sandbox = Path("/usr/bin/sandbox-exec")
    if platform.system() != "Darwin" or not sandbox.is_file():
        raise ExperimentError("network_isolation_unavailable")
    environment = dict(os.environ)
    environment[NETWORK_SANDBOX_MARKER] = EXPERIMENT_ID
    os.execve(
        sandbox,
        [
            str(sandbox),
            "-p",
            NETWORK_SANDBOX_PROFILE,
            sys.executable,
            str(RUNNER_PATH),
            *sys.argv[1:],
        ],
        environment,
    )


def serialize_passage(row: dict[str, Any]) -> str:
    values = [row[field] for field in ("title", "heading", "text")]
    if not all(isinstance(value, str) for value in values):
        raise ExperimentError("passage_value")
    parts = [value.strip() for value in values]
    return "\n".join(part for part in parts if part)


def f32(value: float) -> float:
    try:
        rounded = struct.unpack("!f", struct.pack("!f", float(value)))[0]
    except (OverflowError, TypeError, ValueError, struct.error) as error:
        raise ExperimentError("invalid_score") from error
    if not math.isfinite(rounded):
        raise ExperimentError("non_finite_score")
    return rounded


def load_corpus() -> dict[str, list[Pool]]:
    corpus_root = REPO_ROOT / "fixtures" / "selector" / "answerability-v1"
    verify_exact_hashes(corpus_root, CORPUS_FILES, "corpus_hash_mismatch")
    query_rows = read_jsonl(corpus_root / "queries.jsonl")
    passage_rows = read_jsonl(corpus_root / "passages.jsonl")
    judgment_rows = read_jsonl(corpus_root / "judgments.jsonl")

    queries: dict[str, dict[str, Any]] = {}
    for row in query_rows:
        require_exact_keys(row, {"query_id", "text", "language"}, "query_schema")
        query_id = row["query_id"]
        if not isinstance(query_id, str) or not query_id or query_id in queries:
            raise ExperimentError("query_identity")
        if (
            row["language"] not in {"es", "en"}
            or not isinstance(row["text"], str)
            or not row["text"].strip()
        ):
            raise ExperimentError("query_value")
        queries[query_id] = row

    passages: dict[str, dict[str, Any]] = {}
    passage_keys = {"passage_id", "title", "heading", "text", "language"}
    for row in passage_rows:
        require_exact_keys(row, passage_keys, "passage_schema")
        passage_id = row["passage_id"]
        if not isinstance(passage_id, str) or not passage_id or passage_id in passages:
            raise ExperimentError("passage_identity")
        if row["language"] not in {"es", "en"} or not serialize_passage(row):
            raise ExperimentError("passage_value")
        passages[passage_id] = row

    judgment_keys = {
        "query_id",
        "passage_id",
        "split",
        "world_id",
        "role",
        "answer_group_id",
        "disclosure",
        "negative_kind",
        "tags",
        "review_state",
        "review_reason",
        "evidence_spans",
    }
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    seen_pairs: set[tuple[str, str]] = set()
    for row in judgment_rows:
        require_exact_keys(row, judgment_keys, "judgment_schema")
        if not isinstance(row["query_id"], str) or not isinstance(
            row["passage_id"], str
        ):
            raise ExperimentError("judgment_identity")
        pair = (row["query_id"], row["passage_id"])
        if pair in seen_pairs or pair[0] not in queries or pair[1] not in passages:
            raise ExperimentError("judgment_identity")
        seen_pairs.add(pair)
        if row["split"] not in {"train", "dev"}:
            raise ExperimentError("judgment_split")
        role = row["role"]
        if role not in {"answer", "support", "hard_negative"}:
            raise ExperimentError("judgment_role")
        negative_kind = row["negative_kind"]
        if role == "answer" and negative_kind not in {None, ""}:
            raise ExperimentError("answer_negative_kind")
        if role == "support" and negative_kind != "support_context":
            raise ExperimentError("support_negative_kind")
        if role == "hard_negative" and (
            not isinstance(negative_kind, str)
            or negative_kind not in ALL_NEGATIVE_KINDS
        ):
            raise ExperimentError("hard_negative_kind")
        grouped[pair[0]].append(row)

    if len(seen_pairs) != len(passages):
        raise ExperimentError("unjoined_passage")

    splits: dict[str, list[Pool]] = {"train": [], "dev": []}
    for query_id in sorted(queries):
        rows = sorted(grouped[query_id], key=lambda row: row["passage_id"])
        if len(rows) != 6 or len({row["split"] for row in rows}) != 1:
            raise ExperimentError("pool_shape")
        split = rows[0]["split"]
        candidates = tuple(
            Candidate(
                passage_id=row["passage_id"],
                passage_language=passages[row["passage_id"]]["language"],
                passage=serialize_passage(passages[row["passage_id"]]),
                role=row["role"],
                negative_kind=row["negative_kind"] or "",
            )
            for row in rows
        )
        roles = [candidate.role for candidate in candidates]
        answer_count = roles.count("answer")
        support_count = roles.count("support")
        hard_count = roles.count("hard_negative")
        if (answer_count, support_count, hard_count) not in {(2, 1, 3), (0, 0, 6)}:
            raise ExperimentError("pool_role_shape")
        pool = Pool(
            query_id=query_id,
            query_language=queries[query_id]["language"],
            question=queries[query_id]["text"].strip(),
            split=split,
            candidates=candidates,
        )
        if pool.direction not in {"es-to-es", "es-to-en", "en-to-es", "en-to-en"}:
            raise ExperimentError("direction")
        splits[split].append(pool)

    if len(splits["train"]) != 120 or len(splits["dev"]) != 32:
        raise ExperimentError("split_pool_count")
    verify_exact_hashes(corpus_root, CORPUS_FILES, "corpus_hash_mismatch")
    return splits


def fixed_boundary_decisions(scores: Iterable[float]) -> list[bool]:
    values = [f32(value) for value in scores]
    if len(values) > MAX_POOL_SIZE:
        raise ExperimentError("pool_too_large")
    if not values:
        return []
    best = max(values)
    if best < 0.0:
        return [False] * len(values)
    minimum = f32(best - f32(RELATIVE_WINDOW))
    return [value >= 0.0 and value >= minimum for value in values]


def query_boundary_decisions(
    scores: Iterable[float], query_boundary: float
) -> list[bool]:
    values = [f32(value) for value in scores]
    boundary = f32(query_boundary)
    if len(values) > MAX_POOL_SIZE:
        raise ExperimentError("pool_too_large")
    if not values:
        return []
    best = max(values)
    if best <= boundary:
        return [False] * len(values)
    minimum = f32(best - f32(RELATIVE_WINDOW))
    return [value > boundary and value >= minimum for value in values]


def self_test() -> None:
    assert fixed_boundary_decisions([]) == []
    assert fixed_boundary_decisions([-0.01, -4.0, -1.5]) == [False, False, False]
    assert fixed_boundary_decisions([-0.1, -0.0]) == [False, True]
    boundary = f32(f32(5.0) - f32(RELATIVE_WINDOW))
    assert fixed_boundary_decisions([5.0, boundary, boundary - 0.0001, -0.1]) == [
        True,
        True,
        False,
        False,
    ]
    assert query_boundary_decisions([1.0, 0.5], 1.0) == [False, False]
    assert query_boundary_decisions([1.0001, 1.0, -3.0], 1.0) == [True, False, False]
    assert query_boundary_decisions([5.0, boundary, boundary - 0.0001], 0.0) == [
        True,
        True,
        False,
    ]
    assert fixed_boundary_decisions([5.0, 1.4]) == [True, False]
    assert query_boundary_decisions([-0.2, -0.6], -0.5) == [True, False]
    assert query_boundary_decisions([2.0, 2.0, 1.0], 0.0) == [True, True, True]
    for invalid in (math.nan, math.inf, -math.inf):
        try:
            fixed_boundary_decisions([invalid])
        except ExperimentError as error:
            assert error.code == "non_finite_score"
        else:
            raise AssertionError("non-finite score accepted")
    try:
        fixed_boundary_decisions([0.0] * 11)
    except ExperimentError as error:
        assert error.code == "pool_too_large"
    else:
        raise AssertionError("oversized pool accepted")
    try:
        serialize_passage({"title": None, "heading": "", "text": "body"})
    except ExperimentError as error:
        assert error.code == "passage_value"
    else:
        raise AssertionError("non-string passage value accepted")

    with tempfile.TemporaryDirectory() as temporary:
        temporary_root = Path(temporary)
        source = temporary_root / "source"
        destination = temporary_root / "destination"
        source.mkdir()
        payload = source / "payload"
        payload.write_bytes(b"synthetic model payload")
        payload.chmod(0o400)
        publish_read_only_directory(source, destination)
        assert not source.exists()
        assert destination.is_dir()
        assert (destination / "payload").read_bytes() == b"synthetic model payload"
        destination.chmod(0o700)


def synthetic_pool(roles: list[Role]) -> Pool:
    candidates = []
    for index, role in enumerate(roles):
        if role == "answer":
            negative_kind = ""
        elif role == "support":
            negative_kind = "support_context"
        else:
            negative_kind = "wrong_entity"
        candidates.append(
            Candidate(
                passage_id=f"p{index}",
                passage_language="en",
                passage="synthetic passage",
                role=role,
                negative_kind=negative_kind,
            )
        )
    return Pool(
        query_id="q",
        query_language="en",
        question="synthetic question",
        split="train",
        candidates=tuple(candidates),
    )


def runtime_self_test() -> None:
    numpy, torch, functional, _model_type, _tokenizer_bundle = configure_runtime()
    reset_seed(numpy, torch)

    answerable = synthetic_pool(["answer", "answer", "support", "hard_negative"])
    duplicated = synthetic_pool(
        ["answer", "answer", "support", "hard_negative", "support", "hard_negative"]
    )
    base_logits = torch.tensor([2.0, 1.0, -1.0, -2.0, 0.75], dtype=torch.float32)
    duplicate_logits = torch.tensor(
        [2.0, 1.0, -1.0, -2.0, -1.0, -2.0, 0.75],
        dtype=torch.float32,
        requires_grad=True,
    )
    assert torch.allclose(
        pool_loss(functional, base_logits, answerable, "arm_a"),
        pool_loss(functional, duplicate_logits, duplicated, "arm_a"),
        atol=0.0,
        rtol=0.0,
    )
    arm_a_loss = pool_loss(functional, duplicate_logits, duplicated, "arm_a")
    arm_a_loss.backward()
    assert duplicate_logits.grad is not None
    assert float(duplicate_logits.grad[-1]) == 0.0

    arm_b_logits = torch.tensor(
        [2.0, 1.0, -1.0, -2.0, -1.0, -2.0, 0.75],
        dtype=torch.float32,
        requires_grad=True,
    )
    arm_b_loss = pool_loss(functional, arm_b_logits, duplicated, "arm_b")
    arm_b_loss.backward()
    assert arm_b_logits.grad is not None
    assert torch.isfinite(arm_b_logits.grad).all()
    assert float(arm_b_logits.grad[-1]) != 0.0

    no_answer = synthetic_pool(["hard_negative", "hard_negative"])
    no_answer_a = torch.tensor([-1.0, 1.0, 0.25], dtype=torch.float32)
    expected_a = functional.softplus(no_answer_a[:-1]).mean()
    assert torch.allclose(
        pool_loss(functional, no_answer_a, no_answer, "arm_a"), expected_a
    )
    no_answer_b = torch.tensor([-1.0, 1.0, 0.25], dtype=torch.float32)
    expected_b = functional.softplus(no_answer_b[:-1] - no_answer_b[-1]).mean()
    assert torch.allclose(
        pool_loss(functional, no_answer_b, no_answer, "arm_b"), expected_b
    )

    serialized = serialize_passage(
        {"title": " Title ", "heading": "  ", "text": " Body "}
    )
    assert serialized == "Title\nBody"


def package_versions() -> dict[str, str]:
    return {
        "python": platform.python_version(),
        "torch": importlib.metadata.version("torch"),
        "transformers": importlib.metadata.version("transformers"),
        "tokenizers": importlib.metadata.version("tokenizers"),
        "safetensors": importlib.metadata.version("safetensors"),
        "numpy": importlib.metadata.version("numpy"),
    }


def verified_package_versions() -> dict[str, str]:
    try:
        versions = package_versions()
    except importlib.metadata.PackageNotFoundError as error:
        raise ExperimentError("environment_version_mismatch") from error
    if versions != EXPECTED_VERSIONS:
        raise ExperimentError("environment_version_mismatch")
    return versions


def environment_manifest(versions: dict[str, str]) -> dict[str, Any]:
    return {
        "packages": dict(sorted(versions.items())),
        "platform": {
            "machine": platform.machine(),
            "python_build": list(platform.python_build()),
            "python_implementation": platform.python_implementation(),
            "system": platform.system(),
        },
    }


def model_manifest(hashes: dict[str, str]) -> dict[str, Any]:
    return {
        "files": hashes,
        "model_repo": MODEL_REPO,
        "revision": MODEL_REVISION,
    }


def write_model_manifest(hashes: dict[str, str]) -> None:
    atomic_json(MODEL_MANIFEST_PATH, model_manifest(hashes))


def seal_model_directory(root: Path) -> None:
    for path in root.iterdir():
        if not path.is_file() or path.is_symlink():
            raise ExperimentError("model_hash_mismatch")
        path.chmod(0o400)
    root.chmod(0o500)
    fsync_directory(root.parent)


def verify_model_ready() -> dict[str, str]:
    hashes = verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
    if MODEL_ROOT.stat().st_mode & 0o777 != 0o500:
        raise ExperimentError("model_permissions_mismatch")
    for path in MODEL_ROOT.iterdir():
        if path.stat().st_mode & 0o777 != 0o400:
            raise ExperimentError("model_permissions_mismatch")
    if not MODEL_MANIFEST_PATH.is_file() or MODEL_MANIFEST_PATH.is_symlink():
        raise ExperimentError("model_manifest_mismatch")
    try:
        with MODEL_MANIFEST_PATH.open("r", encoding="utf-8") as handle:
            observed_manifest = json.load(handle)
    except (OSError, json.JSONDecodeError) as error:
        raise ExperimentError("model_manifest_mismatch") from error
    if observed_manifest != model_manifest(hashes):
        raise ExperimentError("model_manifest_mismatch")
    return hashes


def publish_read_only_directory(source: Path, destination: Path) -> None:
    if destination.exists():
        raise ExperimentError("model_state_conflict")
    os.rename(source, destination)
    try:
        seal_model_directory(destination)
    except (OSError, ExperimentError) as error:
        try:
            destination.chmod(0o700)
            shutil.rmtree(destination)
        except OSError:
            pass
        raise ExperimentError("model_publish_failed") from error


def prepare_model(allow_network: bool) -> None:
    if not allow_network:
        raise ExperimentError("network_not_authorized")
    os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    from huggingface_hub import snapshot_download

    if ATTEMPT_PATH.exists() or REPORT_PATH.exists():
        raise ExperimentError("attempt_exists")
    ARTIFACT_ROOT.mkdir(parents=True, exist_ok=True)
    if MODEL_ROOT.exists():
        hashes = verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
        seal_model_directory(MODEL_ROOT)
        hashes = verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
        write_model_manifest(hashes)
        verify_model_ready()
        return

    MODEL_MANIFEST_PATH.unlink(missing_ok=True)

    with tempfile.TemporaryDirectory(
        prefix=".model-download.", dir=ARTIFACT_ROOT
    ) as temporary:
        temporary_root = Path(temporary)
        download_root = temporary_root / "download"
        snapshot_download(
            repo_id=MODEL_REPO,
            revision=MODEL_REVISION,
            local_dir=download_root,
            allow_patterns=sorted(MODEL_FILES),
        )
        verify_hashes(download_root, MODEL_FILES, "model_hash_mismatch")

        clean_root = temporary_root / "snapshot"
        clean_root.mkdir()
        for relative in sorted(MODEL_FILES):
            source = download_root / relative
            if not source.is_file() or source.is_symlink():
                raise ExperimentError("model_hash_mismatch")
            shutil.copyfile(source, clean_root / relative)
        hashes = verify_exact_hashes(clean_root, MODEL_FILES, "model_hash_mismatch")
        publish_read_only_directory(clean_root, MODEL_ROOT)

    hashes = verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
    write_model_manifest(hashes)
    verify_model_ready()


def configure_runtime() -> tuple[Any, Any, Any, Any, Any]:
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"
    os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    os.environ["TOKENIZERS_PARALLELISM"] = "false"
    os.environ["OMP_NUM_THREADS"] = "4"
    os.environ["MKL_NUM_THREADS"] = "4"
    verified_package_versions()
    import numpy
    import torch
    import torch.nn.functional as functional
    from transformers import AutoModelForSequenceClassification, AutoTokenizer
    from transformers.optimization import get_linear_schedule_with_warmup

    torch.set_num_threads(4)
    torch.set_num_interop_threads(1)
    torch.use_deterministic_algorithms(True)
    torch.set_float32_matmul_precision("highest")
    return (
        numpy,
        torch,
        functional,
        AutoModelForSequenceClassification,
        (AutoTokenizer, get_linear_schedule_with_warmup),
    )


def reset_seed(numpy: Any, torch: Any) -> None:
    random.seed(SEED)
    numpy.random.seed(SEED % (2**32))
    torch.manual_seed(SEED)


def encode_pool(tokenizer: Any, pool: Pool, include_boundary: bool) -> dict[str, Any]:
    passages = [candidate.passage for candidate in pool.candidates]
    if include_boundary:
        passages.append("")
    return tokenizer(
        [pool.question] * len(passages),
        passages,
        padding=True,
        truncation=True,
        max_length=MAX_LENGTH,
        return_tensors="pt",
    )


def pool_loss(functional: Any, logits: Any, pool: Pool, arm: Arm) -> Any:
    candidate_scores = logits[:-1]
    boundary = logits[-1]
    answer_indices = [
        index
        for index, candidate in enumerate(pool.candidates)
        if candidate.role == "answer"
    ]
    negative_indices = [
        index
        for index, candidate in enumerate(pool.candidates)
        if candidate.role != "answer"
    ]
    negatives = candidate_scores[negative_indices]
    if answer_indices:
        answers = candidate_scores[answer_indices]
        pairwise = functional.softplus(negatives[:, None] - answers[None, :]).mean()
        if arm == "arm_a":
            return (
                pairwise
                + functional.softplus(-answers).mean()
                + functional.softplus(negatives).mean()
            )
        return (
            pairwise
            + functional.softplus(boundary - answers).mean()
            + functional.softplus(negatives - boundary).mean()
        )
    if arm == "arm_a":
        return functional.softplus(negatives).mean()
    return functional.softplus(negatives - boundary).mean()


def optimizer_for(torch: Any, model: Any) -> Any:
    no_decay_markers = ("bias", "LayerNorm.weight", "layer_norm.weight")
    decay_parameters = []
    no_decay_parameters = []
    for name, parameter in model.named_parameters():
        if not parameter.requires_grad:
            continue
        target = (
            no_decay_parameters
            if any(marker in name for marker in no_decay_markers)
            else decay_parameters
        )
        target.append(parameter)
    return torch.optim.AdamW(
        [
            {"params": decay_parameters, "weight_decay": WEIGHT_DECAY},
            {"params": no_decay_parameters, "weight_decay": 0.0},
        ],
        lr=LEARNING_RATE,
        betas=(0.9, 0.999),
        eps=1e-8,
        foreach=False,
        fused=False,
    )


def freeze_checkpoint(model: Any, arm: Arm) -> dict[str, str]:
    destination = ARTIFACT_ROOT / arm
    if destination.exists():
        raise ExperimentError("checkpoint_state_conflict")
    temporary = Path(tempfile.mkdtemp(prefix=f".{arm}.", dir=ARTIFACT_ROOT))
    try:
        model.save_pretrained(temporary, safe_serialization=True)
        hashes = hash_exact_files(
            temporary, CHECKPOINT_FILES, "checkpoint_manifest_mismatch"
        )
        if destination.exists():
            raise ExperimentError("checkpoint_state_conflict")
        os.rename(temporary, destination)
        fsync_directory(ARTIFACT_ROOT)
        return verify_exact_hashes(destination, hashes, "checkpoint_hash_mismatch")
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def train_arm(
    arm: Arm,
    pools: list[Pool],
    numpy: Any,
    torch: Any,
    functional: Any,
    model_type: Any,
    tokenizer: Any,
    scheduler_type: Any,
) -> dict[str, str]:
    reset_seed(numpy, torch)
    verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
    model = model_type.from_pretrained(
        MODEL_ROOT,
        local_files_only=True,
        trust_remote_code=False,
        use_safetensors=True,
    ).to(device="cpu", dtype=torch.float32)
    verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
    if getattr(model.config, "num_labels", None) != 1:
        raise ExperimentError("model_output_shape")
    optimizer = optimizer_for(torch, model)
    scheduler = scheduler_type(optimizer, WARMUP_STEPS, TRAINING_STEPS)
    shuffle = random.Random(SEED)
    steps = 0
    model.train()
    for _epoch in range(EPOCHS):
        epoch_pools = list(pools)
        shuffle.shuffle(epoch_pools)
        for pool in epoch_pools:
            optimizer.zero_grad(set_to_none=True)
            encoded = encode_pool(tokenizer, pool, True)
            logits = model(**encoded).logits.squeeze(-1)
            expected = len(pool.candidates) + 1
            if (
                logits.ndim != 1
                or logits.numel() != expected
                or not torch.isfinite(logits).all()
            ):
                raise ExperimentError("model_output_shape")
            loss = pool_loss(functional, logits, pool, arm)
            if not torch.isfinite(loss):
                raise ExperimentError("non_finite_loss")
            loss.backward()
            torch.nn.utils.clip_grad_norm_(
                model.parameters(), MAX_GRAD_NORM, error_if_nonfinite=True
            )
            optimizer.step()
            scheduler.step()
            steps += 1
    if steps != TRAINING_STEPS:
        raise ExperimentError("training_step_count")
    hashes = freeze_checkpoint(model, arm)
    verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
    return hashes


def load_frozen_checkpoint(
    arm: Arm,
    expected_hashes: dict[str, str],
    model_type: Any,
    torch: Any,
) -> Any:
    root = ARTIFACT_ROOT / arm
    verify_exact_hashes(root, expected_hashes, "checkpoint_hash_mismatch")
    model = model_type.from_pretrained(
        root,
        local_files_only=True,
        trust_remote_code=False,
        use_safetensors=True,
    ).to(device="cpu", dtype=torch.float32)
    verify_exact_hashes(root, expected_hashes, "checkpoint_hash_mismatch")
    if getattr(model.config, "num_labels", None) != 1:
        raise ExperimentError("model_output_shape")
    return model


def evaluate_arm(
    arm: Arm, model: Any, pools: list[Pool], tokenizer: Any, torch: Any
) -> dict[str, Any]:
    totals = {
        "answers": 0,
        "accepted_answers": 0,
        "accepted_total": 0,
        "supports": 0,
        "accepted_supports": 0,
        "high_risk_false_positives": 0,
        "no_answer_acceptances": 0,
        "exact_pools": 0,
        "pools": len(pools),
    }
    directions: dict[str, dict[str, int]] = defaultdict(
        lambda: {"answers": 0, "accepted_answers": 0}
    )
    model.eval()
    with torch.inference_mode():
        for pool in pools:
            encoded = encode_pool(tokenizer, pool, arm == "arm_b")
            logits = model(**encoded).logits.squeeze(-1)
            expected = len(pool.candidates) + (1 if arm == "arm_b" else 0)
            if (
                logits.ndim != 1
                or logits.numel() != expected
                or not torch.isfinite(logits).all()
            ):
                raise ExperimentError("model_output_shape")
            values = [float(value) for value in logits.tolist()]
            if arm == "arm_a":
                accepted = fixed_boundary_decisions(values)
            else:
                accepted = query_boundary_decisions(values[:-1], values[-1])
            exact = True
            for candidate, decision in zip(pool.candidates, accepted, strict=True):
                totals["accepted_total"] += int(decision)
                if candidate.role == "answer":
                    totals["answers"] += 1
                    totals["accepted_answers"] += int(decision)
                    directions[pool.direction]["answers"] += 1
                    directions[pool.direction]["accepted_answers"] += int(decision)
                    exact = exact and decision
                else:
                    exact = exact and not decision
                    if candidate.role == "support":
                        totals["supports"] += 1
                        totals["accepted_supports"] += int(decision)
                    elif decision and candidate.negative_kind in HIGH_RISK_KINDS:
                        totals["high_risk_false_positives"] += 1
                if not pool.answerable and decision:
                    totals["no_answer_acceptances"] += 1
            totals["exact_pools"] += int(exact)

    answer_recall = totals["accepted_answers"] / totals["answers"]
    precision = (
        totals["accepted_answers"] / totals["accepted_total"]
        if totals["accepted_total"]
        else 1.0
    )
    support_acceptance = totals["accepted_supports"] / totals["supports"]
    exact_pool_success = totals["exact_pools"] / totals["pools"]
    recall_by_direction = {
        direction: value["accepted_answers"] / value["answers"]
        for direction, value in sorted(directions.items())
    }
    expected_directions = {"en-to-en", "en-to-es", "es-to-en", "es-to-es"}
    if set(recall_by_direction) != expected_directions:
        raise ExperimentError("direction_coverage")
    passed = (
        answer_recall >= 0.90
        and all(value >= 0.85 for value in recall_by_direction.values())
        and precision >= 0.99
        and totals["no_answer_acceptances"] == 0
        and totals["high_risk_false_positives"] == 0
        and support_acceptance <= 0.10
        and exact_pool_success >= 0.85
    )
    return {
        "answer_recall": answer_recall,
        "counts": dict(sorted(totals.items())),
        "direction_counts": {
            direction: dict(sorted(value.items()))
            for direction, value in sorted(directions.items())
        },
        "exact_pool_success": exact_pool_success,
        "high_risk_false_positives": totals["high_risk_false_positives"],
        "no_answer_acceptances": totals["no_answer_acceptances"],
        "passed": passed,
        "precision": precision,
        "recall_by_direction": recall_by_direction,
        "support_acceptance": support_acceptance,
    }


def select_arm(results: dict[Arm, dict[str, Any]]) -> Arm | None:
    passing = [arm for arm in ("arm_a", "arm_b") if results[arm]["passed"]]
    if not passing:
        return None
    if len(passing) == 1:
        return passing[0]
    return min(
        passing,
        key=lambda arm: (
            results[arm]["high_risk_false_positives"],
            results[arm]["no_answer_acceptances"],
            -results[arm]["precision"],
            -results[arm]["answer_recall"],
            0 if arm == "arm_a" else 1,
        ),
    )


def create_attempt_receipt(
    versions: dict[str, str],
    model_hashes: dict[str, str],
) -> dict[str, Any]:
    if (
        os.environ.get(NETWORK_SANDBOX_MARKER) != EXPERIMENT_ID
        or not network_isolation_active()
    ):
        raise ExperimentError("network_isolation_unavailable")
    receipt = {
        "environment": environment_manifest(versions),
        "expected_corpus_hashes": dict(sorted(CORPUS_FILES.items())),
        "experiment_id": EXPERIMENT_ID,
        "model_hashes": dict(sorted(model_hashes.items())),
        "model_revision": MODEL_REVISION,
        "network_isolation": "macos_sandbox_exec_deny_network_confirmed",
        "process_environment": {
            key: os.environ.get(key) for key in sorted(REQUIRED_PROCESS_ENVIRONMENT)
        },
        "runner_sha256": sha256_file(RUNNER_PATH),
        "state": "started",
    }
    ATTEMPT_PATH.parent.mkdir(parents=True, exist_ok=True)
    try:
        descriptor = os.open(ATTEMPT_PATH, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError as error:
        raise ExperimentError("attempt_exists") from error
    with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
        json.dump(receipt, handle, indent=2, sort_keys=True)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())
    fsync_directory(ATTEMPT_PATH.parent)
    return receipt


def run_compatibility() -> None:
    if ATTEMPT_PATH.exists() or REPORT_PATH.exists():
        raise ExperimentError("attempt_exists")
    if any((ARTIFACT_ROOT / arm).exists() for arm in ("arm_a", "arm_b")):
        raise ExperimentError("checkpoint_state_conflict")

    versions = verified_package_versions()
    model_hashes = verify_model_ready()
    receipt = create_attempt_receipt(versions, model_hashes)
    try:
        corpus = load_corpus()
        receipt["verified_corpus_hashes"] = verify_exact_hashes(
            REPO_ROOT / "fixtures" / "selector" / "answerability-v1",
            CORPUS_FILES,
            "corpus_hash_mismatch",
        )
        numpy, torch, functional, model_type, tokenizer_bundle = configure_runtime()
        tokenizer_type, scheduler_type = tokenizer_bundle
        verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
        tokenizer = tokenizer_type.from_pretrained(
            MODEL_ROOT,
            local_files_only=True,
            trust_remote_code=False,
            use_fast=True,
        )
        verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")

        checkpoint_hashes: dict[Arm, dict[str, str]] = {}
        for arm in ("arm_a", "arm_b"):
            checkpoint_hashes[arm] = train_arm(
                arm,
                corpus["train"],
                numpy,
                torch,
                functional,
                model_type,
                tokenizer,
                scheduler_type,
            )

        results: dict[Arm, dict[str, Any]] = {}
        for arm in ("arm_a", "arm_b"):
            model = load_frozen_checkpoint(
                arm, checkpoint_hashes[arm], model_type, torch
            )
            results[arm] = evaluate_arm(arm, model, corpus["dev"], tokenizer, torch)
            del model

        verify_exact_hashes(MODEL_ROOT, MODEL_FILES, "model_hash_mismatch")
        for arm in ("arm_a", "arm_b"):
            verify_exact_hashes(
                ARTIFACT_ROOT / arm,
                checkpoint_hashes[arm],
                "checkpoint_hash_mismatch",
            )
        verify_exact_hashes(
            REPO_ROOT / "fixtures" / "selector" / "answerability-v1",
            CORPUS_FILES,
            "corpus_hash_mismatch",
        )
        if sha256_file(RUNNER_PATH) != receipt["runner_sha256"]:
            raise ExperimentError("runner_hash_mismatch")

        selected = select_arm(results)
        report = {
            "checkpoint_hashes": checkpoint_hashes,
            "decision": "continue" if selected else "reject",
            "experiment_id": EXPERIMENT_ID,
            "results": results,
            "selected_arm": selected,
            "started_attempt_sha256": sha256_file(ATTEMPT_PATH),
        }
        atomic_json(REPORT_PATH, report)
        receipt["report_sha256"] = sha256_file(REPORT_PATH)
        receipt["state"] = "completed"
        atomic_json(ATTEMPT_PATH, receipt)
    except BaseException as error:
        receipt["failure_code"] = (
            error.code if isinstance(error, ExperimentError) else "internal_failure"
        )
        receipt["state"] = "failed"
        atomic_json(ATTEMPT_PATH, receipt)
        raise


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("self-test")
    subparsers.add_parser("runtime-self-test")
    subparsers.add_parser("network-self-test")
    prepare = subparsers.add_parser("prepare-model")
    prepare.add_argument("--allow-network", action="store_true")
    subparsers.add_parser("run-compatibility")
    return parser.parse_args()


def ensure_process_environment(command: str) -> None:
    if command != "run-compatibility":
        return
    if all(
        os.environ.get(key) == value
        for key, value in REQUIRED_PROCESS_ENVIRONMENT.items()
    ):
        return
    environment = dict(os.environ)
    environment.update(REQUIRED_PROCESS_ENVIRONMENT)
    os.execve(
        sys.executable,
        [sys.executable, str(Path(__file__).resolve()), *sys.argv[1:]],
        environment,
    )


def main() -> int:
    arguments = parse_arguments()
    try:
        require_canonical_runner()
        require_safe_repository_paths()
        ensure_process_environment(arguments.command)
        ensure_network_isolation(arguments.command)
        if arguments.command == "self-test":
            self_test()
            print("self-test: pass")
        elif arguments.command == "runtime-self-test":
            runtime_self_test()
            print("runtime-self-test: pass")
        elif arguments.command == "network-self-test":
            if not network_isolation_active():
                raise ExperimentError("network_isolation_unavailable")
            print("network-self-test: pass")
        elif arguments.command == "prepare-model":
            prepare_model(arguments.allow_network)
            print("prepare-model: pass")
        else:
            run_compatibility()
            print("run-compatibility: completed")
        return 0
    except ExperimentError as error:
        print(f"experiment-error: {error.code}", file=sys.stderr)
        return 2
    except BaseException:
        print("experiment-error: internal_failure", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
