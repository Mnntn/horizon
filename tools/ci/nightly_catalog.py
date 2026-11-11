#!/usr/bin/env python3
"""Export the pinned Nightly's existing SDK catalog, without staging its source."""
from __future__ import annotations

import argparse
import importlib.util
from pathlib import Path
import subprocess
import shutil


def export(root: Path, commit: str, output: Path) -> None:
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    if actual != commit:
        raise ValueError(f"catalog checkout is {actual}, not selected Nightly commit {commit}")
    subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=root, check=True)
    spec = importlib.util.spec_from_file_location("prebuilt_game", root / "tools/prebuilt_game.py")
    producer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(producer)
    # The SDK writes its catalog before widening the disposable source copy. Reuse
    # that exact staging/export path; this does not compile the game or an SDK.
    staged = producer.stage(root / "target/nightly-catalog")
    shutil.copy2(staged / producer.CATALOG, output)
    # Staging must not change the tracked source the catalog describes.
    subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=root, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    export(args.root.resolve(), args.commit, args.output.resolve())


if __name__ == "__main__":
    main()
