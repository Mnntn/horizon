#!/usr/bin/env python3
"""Exercise the prebuilt source stage on the checked-out merge result, without a game build."""
import importlib.util
from pathlib import Path
import shutil
import time

ROOT = Path(__file__).resolve().parents[2]


def main():
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
