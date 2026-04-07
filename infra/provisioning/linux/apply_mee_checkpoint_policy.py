#!/usr/bin/env python3
"""Apply checkpoint persistence policy to a live MEE training orchestrator.

This keeps checkpoint behavior under repo control even when the full
training/orchestrator source tree only exists on deployed nodes.
"""

from __future__ import annotations

import argparse
from pathlib import Path


NEEDLE = 'torch.save(model.state_dict(), ckpt_dir / "best.pt")'
GUARD = (
    'if os.environ.get("MEE_SAVE_BEST_CHECKPOINTS", "1") '
    'not in {"0", "false", "False", "no", "NO"}:\n'
    '                        # Save best checkpoint\n'
    '                        torch.save(model.state_dict(), ckpt_dir / "best.pt")'
)


def patch_orchestrator(path: Path) -> bool:
    text = path.read_text()
    if "MEE_SAVE_BEST_CHECKPOINTS" in text:
        return False
    if NEEDLE not in text:
        raise RuntimeError(f"checkpoint save line not found in {path}")
    updated = text.replace(
        '                        # Save best checkpoint\n'
        '                        torch.save(model.state_dict(), ckpt_dir / "best.pt")',
        GUARD,
        1,
    )
    path.write_text(updated)
    return True


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("orchestrator_path", type=Path)
    args = parser.parse_args()
    changed = patch_orchestrator(args.orchestrator_path)
    print("patched" if changed else "already_patched")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
