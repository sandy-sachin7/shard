#!/usr/bin/env python3
"""
Empirical ML checkpoint deduplication benchmark for Shard.

Tests the real-world ML weight deduplication premise across four realistic scenarios on
64 MiB IEEE-754 float32 safetensors-style checkpoints (and JSONL datasets):
  1. Full fine-tune epoch: every float32 parameter updated by a small gradient step (expected ~0% dedup).
  2. Frozen backbone + trained head/LoRA: 90% frozen weights + 10% updated weights.
  3. Header/tensor insertion over frozen backbone: 1 MiB prepended shifting downstream tensor offsets.
  4. JSONL dataset + 1 MiB prepend: structured text dataset with offset shift.

Usage:
    cargo build --release --locked
    uv run --with numpy python3 benchmarks/bench_checkpoints.py
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

import numpy as np

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BIN = REPO_ROOT / "target" / "release" / "shard"


def find_shard_bin() -> str:
    env_bin = os.environ.get("SHARD_BIN")
    if env_bin:
        return env_bin
    if DEFAULT_BIN.exists():
        return str(DEFAULT_BIN)
    raise FileNotFoundError(f"shard binary not found at {DEFAULT_BIN}. Run `cargo build --release --locked` first.")


def dir_size_bytes(path: Path) -> int:
    return sum(p.stat().st_size for p in path.rglob("*") if p.is_file())


def write_safetensors_like(path: Path, header_meta: dict, tensor_bytes: bytes) -> None:
    """Write a safetensors-framed binary file: 8-byte LE header length + JSON header + raw tensor bytes."""
    hdr = json.dumps(header_meta, separators=(",", ":")).encode("utf-8")
    with open(path, "wb") as f:
        f.write(struct.pack("<Q", len(hdr)))
        f.write(hdr)
        f.write(tensor_bytes)


def measure_scenario(shard_bin: str, label: str, v1_bytes: bytes, v2_bytes: bytes, chunker: str) -> dict:
    with tempfile.TemporaryDirectory(prefix=f"shard-ckpt-{chunker}-") as tmp:
        repo = Path(tmp) / "repo"
        repo.mkdir()
        ckpt = repo / "model.safetensors"

        subprocess.run([shard_bin, "init", "--chunker", chunker], cwd=repo, check=True, stdout=subprocess.DEVNULL)

        ckpt.write_bytes(v1_bytes)
        subprocess.run([shard_bin, "add", ckpt.name], cwd=repo, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(
            [shard_bin, "commit", "-m", "v1", "--author", "Bench <bench@local>"],
            cwd=repo,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        v1_store = dir_size_bytes(repo / ".shard" / "objects")

        ckpt.write_bytes(v2_bytes)
        subprocess.run([shard_bin, "add", ckpt.name], cwd=repo, check=True, stdout=subprocess.DEVNULL)
        subprocess.run(
            [shard_bin, "commit", "-m", "v2", "--author", "Bench <bench@local>"],
            cwd=repo,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        v2_store = dir_size_bytes(repo / ".shard" / "objects")
        delta = v2_store - v1_store
        # Measure chunk deduplication strictly against v1's compressed store size so Zstd
        # compression of float32 exponent bytes is not misattributed to deduplication.
        chunk_dedup_pct = max(0.0, (1.0 - (delta / max(v1_store, 1))) * 100.0)
        return {
            "scenario": label,
            "chunker": chunker,
            "v1_mb": round(len(v1_bytes) / (1024 * 1024), 2),
            "v2_mb": round(len(v2_bytes) / (1024 * 1024), 2),
            "v1_store_mb": round(v1_store / (1024 * 1024), 2),
            "v2_delta_mb": round(delta / (1024 * 1024), 2),
            "chunk_dedup_pct": round(chunk_dedup_pct, 1),
        }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--size-mb", type=int, default=64)
    parser.add_argument("--out", type=Path, default=None)
    args = parser.parse_args()

    shard_bin = find_shard_bin()
    rng = np.random.default_rng(42)
    n_floats = (args.size_mb * 1024 * 1024) // 4
    base_weights = rng.normal(0.0, 0.02, size=n_floats).astype(np.float32)

    # Frame v1 as a safetensors-like checkpoint
    def pack_weights(arr: np.ndarray, extra_prefix: bytes = b"") -> bytes:
        raw = extra_prefix + arr.tobytes()
        hdr = json.dumps({"weight": {"dtype": "F32", "shape": [len(arr)], "data_offsets": [len(extra_prefix), len(raw)]}}).encode("utf-8")
        return struct.pack("<Q", len(hdr)) + hdr + raw

    v1 = pack_weights(base_weights)

    # 1. Full fine-tune epoch: every float32 weight shifts by a small SGD/Adam update
    grad = rng.normal(0.0, 1e-4, size=n_floats).astype(np.float32)
    v2_full_ft = pack_weights(base_weights + grad)

    # 2. Frozen backbone (90%) + fine-tuned classifier head / adapter (tail 10%)
    frozen_weights = base_weights.copy()
    cut = int(n_floats * 0.90)
    frozen_weights[cut:] += grad[cut:]
    v2_frozen_head = pack_weights(frozen_weights)

    # 3. Prepended 1 MiB adapter/header tensor before frozen backbone (offset shift)
    adapter_bytes = rng.normal(0.0, 0.02, size=(1024 * 1024) // 4).astype(np.float32).tobytes()
    v2_prepend_frozen = pack_weights(base_weights, extra_prefix=adapter_bytes)

    scenarios = [
        ("Full fine-tune (100% FP32 weights updated)", v1, v2_full_ft),
        ("Frozen 90% backbone + 10% updated head", v1, v2_frozen_head),
        ("1 MiB adapter prepend + frozen backbone (offset shift)", v1, v2_prepend_frozen),
    ]

    rows = []
    for label, a, b in scenarios:
        for chunker in ("fixed", "fastcdc"):
            r = measure_scenario(shard_bin, label, a, b, chunker)
            rows.append(r)
            print(
                f"{r['scenario']:<55} | {r['chunker']:<7} | v1_store={r['v1_store_mb']:>6.2f} MiB | "
                f"v2_delta={r['v2_delta_mb']:>6.2f} MiB | chunk_dedup={r['chunk_dedup_pct']:>5.1f}%"
            )

    if args.out is not None:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps({"size_mb": args.size_mb, "results": rows}, indent=2) + "\n", encoding="utf-8")
        print(f"\nSaved results to {args.out}")


if __name__ == "__main__":
    main()
