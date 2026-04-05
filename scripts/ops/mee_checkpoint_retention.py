#!/usr/bin/env python3
"""
MEE checkpoint retention for mainframe.

Preserves:
- checkpoints for architecture IDs present in benchmark/deployment leaderboards
- checkpoints modified within a recent window
- a minimum newest set as a restart cushion

Deletes older, unreferenced checkpoint directories by default only when --apply is passed.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path


DEFAULT_EXPERIMENTS_DIR = Path("/srv/swarm/apps/swarm-mainframe/model_research/experiments")
LEGACY_EXPERIMENTS_DIR = Path("/opt/swarm-mainframe/model_research/experiments")


@dataclass
class CheckpointInfo:
    path: Path
    arch_id: str
    size_bytes: int
    mtime: float


def load_json(path: Path):
    return json.loads(path.read_text())


def resolve_default_experiments_dir() -> Path:
    if DEFAULT_EXPERIMENTS_DIR.exists():
        return DEFAULT_EXPERIMENTS_DIR
    return LEGACY_EXPERIMENTS_DIR


def load_experiment_index(all_experiments_path: Path) -> dict[str, dict]:
    data = load_json(all_experiments_path)
    if isinstance(data, dict):
        data = data.get("all_experiments") or data.get("experiments") or data.get("items") or []
    index: dict[str, dict] = {}
    for exp in data:
        if not isinstance(exp, dict):
            continue
        experiment_id = exp.get("experiment_id")
        arch = exp.get("architecture") or {}
        arch_id = arch.get("arch_id")
        if experiment_id and arch_id:
            index[experiment_id] = exp
    return index


def leaderboard_arch_ids(leaderboard_path: Path, experiment_index: dict[str, dict]) -> set[str]:
    leaderboard = load_json(leaderboard_path)
    preserve_ids: set[str] = set()
    for key in ("benchmark_top_models", "deployment_top_models", "top_models"):
        for item in leaderboard.get(key, []) or []:
            experiment_id = item.get("experiment_id")
            exp = experiment_index.get(experiment_id)
            arch = (exp or {}).get("architecture") or {}
            arch_id = arch.get("arch_id")
            if arch_id:
                preserve_ids.add(arch_id)
    return preserve_ids


def checkpoint_infos(checkpoints_dir: Path) -> list[CheckpointInfo]:
    infos: list[CheckpointInfo] = []
    for path in checkpoints_dir.iterdir():
        if not path.is_dir():
            continue
        arch_id = path.name.split("-", 1)[0]
        size_bytes = sum(p.stat().st_size for p in path.rglob("*") if p.is_file())
        infos.append(
            CheckpointInfo(
                path=path,
                arch_id=arch_id,
                size_bytes=size_bytes,
                mtime=path.stat().st_mtime,
            )
        )
    return sorted(infos, key=lambda item: item.mtime, reverse=True)


def format_gb(size_bytes: int) -> float:
    return round(size_bytes / (1024 ** 3), 3)


def main() -> int:
    parser = argparse.ArgumentParser(description="Prune stale MEE checkpoints on mainframe.")
    parser.add_argument("--recent-hours", type=float, default=6.0)
    parser.add_argument("--min-newest", type=int, default=200)
    parser.add_argument("--experiments-dir", type=Path, default=resolve_default_experiments_dir())
    parser.add_argument("--leaderboard-path", type=Path)
    parser.add_argument("--all-experiments-path", type=Path)
    parser.add_argument("--checkpoints-dir", type=Path)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()

    experiments_dir = args.experiments_dir
    leaderboard_path = args.leaderboard_path or experiments_dir / "leaderboard.json"
    all_experiments_path = args.all_experiments_path or experiments_dir / "all_experiments.json"
    checkpoints_dir = args.checkpoints_dir or experiments_dir / "checkpoints"

    if not checkpoints_dir.exists():
        print(json.dumps({"error": "checkpoints_missing", "path": str(checkpoints_dir)}))
        return 1

    experiment_index = load_experiment_index(all_experiments_path)
    preserved_arch_ids = leaderboard_arch_ids(leaderboard_path, experiment_index)
    infos = checkpoint_infos(checkpoints_dir)
    now = datetime.now(timezone.utc).timestamp()
    recent_cutoff = now - timedelta(hours=args.recent_hours).total_seconds()

    newest_names = {info.path.name for info in infos[: args.min_newest]}
    keep: list[CheckpointInfo] = []
    delete: list[CheckpointInfo] = []
    for info in infos:
        should_keep = (
            info.arch_id in preserved_arch_ids
            or info.path.name in newest_names
            or info.mtime >= recent_cutoff
        )
        (keep if should_keep else delete).append(info)

    deleted_bytes = 0
    deleted_dirs = 0
    if args.apply:
        for info in delete:
            shutil.rmtree(info.path)
            deleted_bytes += info.size_bytes
            deleted_dirs += 1

    summary = {
        "checkpoints_total": len(infos),
        "leaderboard_arch_ids": sorted(preserved_arch_ids),
        "kept_count": len(keep),
        "delete_count": len(delete),
        "min_newest": args.min_newest,
        "recent_hours": args.recent_hours,
        "experiments_dir": str(experiments_dir),
        "apply": args.apply,
        "reclaimable_gb": format_gb(sum(info.size_bytes for info in delete)),
        "deleted_gb": format_gb(deleted_bytes),
        "deleted_dirs": deleted_dirs,
        "sample_delete": [info.path.name for info in delete[:20]],
    }
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
