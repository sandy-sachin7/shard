#!/usr/bin/env python3
"""
Reproducible local benchmark for Shard on realistic ML artifacts.

Measures real, non-extrapolated numbers across two distinct artifact classes:
  1. High-entropy binary weights (256 MiB pseudo-random bytes, representative of FP16/BF16/FP32 tensors)
     + incremental update (1 MiB prepended + 5% tail modified) comparing fixed vs Rabin CDC chunking.
  2. Structured ML dataset (256 MiB JSONL training records)
     + incremental mid-dataset insertion (1 MiB inserted near start) comparing fixed vs Rabin CDC chunking.

Also measures `shard verify` and `shard checkout` throughput.

Usage:
    python3 benchmarks/bench_real.py [--size-mb 256]
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import tempfile
import time
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BIN = REPO_ROOT / "target" / "release" / "shard"


def find_shard_bin() -> str:
    env_bin = os.environ.get("SHARD_BIN")
    if env_bin:
        return env_bin
    if DEFAULT_BIN.exists():
        return str(DEFAULT_BIN)
    found = shutil.which("shard")
    if found:
        return found
    raise FileNotFoundError(
        f"shard binary not found at {DEFAULT_BIN} or in PATH. Run `cargo build --release` first."
    )


def dir_size_bytes(path: Path) -> int:
    total = 0
    for root, _, files in os.walk(path):
        for fn in files:
            p = Path(root) / fn
            if p.is_file():
                total += p.stat().st_size
    return total


def run_timed(cmd: list[str], cwd: Path) -> tuple[float, str]:
    t0 = time.perf_counter()
    res = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, check=True)
    return time.perf_counter() - t0, res.stdout


def write_high_entropy_file(path: Path, size_mb: int) -> None:
    """Write truly independent high-entropy bytes (comparable compressibility to trained FP16/BF16 weights)."""
    with open(path, "wb") as f:
        for _ in range(size_mb):
            f.write(os.urandom(1024 * 1024))


def write_jsonl_dataset(path: Path, size_mb: int) -> None:
    """Write a realistic JSONL training dataset with unique sample IDs."""
    target_bytes = size_mb * 1024 * 1024
    written = 0
    idx = 0
    with open(path, "wb") as f:
        while written < target_bytes:
            batch = []
            for _ in range(1000):
                line = (
                    f'{{"id":{idx},"split":"train","label":{idx % 10},'
                    f'"prompt":"Summarize telemetry event {idx} from cluster node-{idx % 64:02d} '
                    f'with latency {(idx * 17) % 500}ms and status ok.",'
                    f'"tokens":[{idx % 32000},{(idx * 3) % 32000},{(idx * 7) % 32000},{(idx * 13) % 32000}]}}\n'
                )
                batch.append(line)
                idx += 1
            raw = "".join(batch).encode("utf-8")
            f.write(raw)
            written += len(raw)


def bench_artifact(
    shard_bin: str,
    name: str,
    base_file: Path,
    mutate_fn,
    chunker: str,
) -> dict:
    with tempfile.TemporaryDirectory(prefix=f"shard-bench-{name}-{chunker}-") as tmp:
        repo = Path(tmp) / "repo"
        repo.mkdir()
        work_file = repo / base_file.name
        shutil.copy2(base_file, work_file)
        file_bytes = work_file.stat().st_size
        file_mb = file_bytes / (1024 * 1024)

        run_timed([shard_bin, "init", "--chunker", chunker], cwd=repo)

        t_add1, _ = run_timed([shard_bin, "add", work_file.name], cwd=repo)
        t_commit1, out1 = run_timed(
            [shard_bin, "commit", "-m", "v1", "--author", "Bench <bench@local>", "--json"],
            cwd=repo,
        )
        commit1 = json.loads(out1)["commit_id"]
        store_v1_bytes = dir_size_bytes(repo / ".shard" / "objects")

        # Mutate the artifact and commit v2
        mutate_fn(work_file)
        v2_file_bytes = work_file.stat().st_size
        t_add2, _ = run_timed([shard_bin, "add", work_file.name], cwd=repo)
        t_commit2, out2 = run_timed(
            [shard_bin, "commit", "-m", "v2", "--author", "Bench <bench@local>", "--json"],
            cwd=repo,
        )
        commit2 = json.loads(out2)["commit_id"]
        store_v2_bytes = dir_size_bytes(repo / ".shard" / "objects")
        delta_store_bytes = store_v2_bytes - store_v1_bytes

        # Verify commit2 and checkout commit1
        t_verify, _ = run_timed([shard_bin, "verify", commit2], cwd=repo)
        t_checkout, _ = run_timed([shard_bin, "checkout", commit1], cwd=repo)

        return {
            "artifact": name,
            "chunker": chunker,
            "v1_size_mb": round(file_mb, 2),
            "v2_size_mb": round(v2_file_bytes / (1024 * 1024), 2),
            "v1_add_s": round(t_add1, 3),
            "v1_commit_s": round(t_commit1, 3),
            "v1_throughput_mb_s": round(file_mb / (t_add1 + t_commit1), 1),
            "v1_store_mb": round(store_v1_bytes / (1024 * 1024), 2),
            "v1_compression_ratio": round(file_bytes / max(store_v1_bytes, 1), 2),
            "v2_add_s": round(t_add2, 3),
            "v2_commit_s": round(t_commit2, 3),
            "v2_delta_store_mb": round(delta_store_bytes / (1024 * 1024), 2),
            "v2_dedup_savings_pct": round((1.0 - (delta_store_bytes / max(store_v1_bytes, 1))) * 100.0, 1),
            "verify_s": round(t_verify, 3),
            "verify_mb_s": round((v2_file_bytes / (1024 * 1024)) / max(t_verify, 1e-6), 1),
            "checkout_s": round(t_checkout, 3),
            "checkout_mb_s": round(file_mb / max(t_checkout, 1e-6), 1),
        }


def mutate_prepend_1mb(path: Path) -> None:
    """Prepend 1 MiB of new bytes (shifts every offset — worst case for fixed chunking, ideal for Rabin CDC)."""
    original = path.read_bytes()
    header = os.urandom(1024 * 1024)
    path.write_bytes(header + original)


def mutate_append_5pct(path: Path) -> None:
    """Append 5% new bytes to the end of the artifact."""
    size = path.stat().st_size
    extra = max(1024 * 1024, int(size * 0.05))
    with open(path, "ab") as f:
        f.write(os.urandom(extra))


def mutate_jsonl_insert_1mb_front(path: Path) -> None:
    """Prepend 1 MiB of new JSONL records (shifts all byte offsets)."""
    original = path.read_bytes()
    new_lines = []
    written = 0
    idx = 0
    while written < 1024 * 1024:
        line = f'{{"id":-1000000-{idx},"split":"delta","prompt":"Prepended new sample {idx}","tokens":[1,2,3,4]}}\n'
        new_lines.append(line)
        written += len(line)
        idx += 1
    path.write_bytes("".join(new_lines).encode("utf-8") + original)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--size-mb", type=int, default=256)
    args = parser.parse_args()

    shard_bin = find_shard_bin()
    version = subprocess.check_output([shard_bin, "--version"], text=True).strip()
    print(f"Binary: {version} | Host: {platform.system()} {platform.machine()} ({os.cpu_count()} CPUs)")

    results = {
        "version": version,
        "host": f"{platform.system()} {platform.machine()} ({os.cpu_count()} logical CPUs)",
        "size_mb": args.size_mb,
        "runs": [],
    }

    with tempfile.TemporaryDirectory(prefix="shard-bench-inputs-") as tmp:
        tmp_path = Path(tmp)
        weights = tmp_path / "weights.bin"
        dataset = tmp_path / "dataset.jsonl"

        print(f"Generating {args.size_mb} MiB high-entropy weights and {args.size_mb} MiB JSONL dataset...")
        write_high_entropy_file(weights, args.size_mb)
        write_jsonl_dataset(dataset, args.size_mb)

        scenarios = [
            ("weights_append_5pct", weights, mutate_append_5pct, "fixed"),
            ("weights_append_5pct", weights, mutate_append_5pct, "rabin"),
            ("weights_prepend_1mb", weights, mutate_prepend_1mb, "fixed"),
            ("weights_prepend_1mb", weights, mutate_prepend_1mb, "rabin"),
            ("jsonl_prepend_1mb", dataset, mutate_jsonl_insert_1mb_front, "fixed"),
            ("jsonl_prepend_1mb", dataset, mutate_jsonl_insert_1mb_front, "rabin"),
        ]

        for label, src, mut, chunker in scenarios:
            print(f"Running {label} ({chunker})...")
            r = bench_artifact(shard_bin, label, src, mut, chunker)
            results["runs"].append(r)

    print("\n=== JSON RESULTS ===")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
