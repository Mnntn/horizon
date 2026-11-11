#!/usr/bin/env python3
"""Exercise the prebuilt source stage on the checked-out merge result, without a game build."""
import argparse
import importlib.util
from pathlib import Path
import subprocess

from slot_inputs import applies
import shutil
import time

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="HEAD^")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    changed = subprocess.check_output(
        ["git", "diff", "--name-only", args.base, "HEAD"], cwd=ROOT, text=True,
    ).splitlines()
    if not args.force and not applies(ROOT, changed):
        print("Prebuilt source stage: unrelated paths, not run")
        return
    spec = importlib.util.spec_from_file_location("prebuilt_game", ROOT / "tools/prebuilt_game.py")
    prebuilt = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(prebuilt)
    target = ROOT / "target/slot-stage-check"
    start = time.monotonic()
    try:
        prebuilt.stage(target)
        print(f"Prebuilt source stage: passed in {time.monotonic() - start:.2f} seconds")
    finally:
        shutil.rmtree(target / "stage", ignore_errors=True)


if __name__ == "__main__":
    main()
