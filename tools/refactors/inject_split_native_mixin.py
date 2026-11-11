#!/usr/bin/env python3
"""Spell the mixin table `[[native.mixin]]` wherever the tree wrote `[[mixin]]`.

The table moved under `[native]` beside `bridge`, `state` and `module`. This rewrites the
spelling only (Rust strings and docs, the book, the CI fixture, launcher translations);
the model and its readers are changed by hand in the commits that follow.

    python3 tools/refactors/inject_split_native_mixin.py
"""

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PATHS = ["crates", "docs", "launcher/src", "launcher/lang", "tools/ci"]


def main():
    files = subprocess.run(
        ["git", "grep", "-lF", "[[mixin]]", "--", *PATHS],
        cwd=ROOT, text=True, stdout=subprocess.PIPE, check=False,
    ).stdout.split()
    for name in files:
        path = ROOT / name
        path.write_text(path.read_text(encoding="utf-8").replace("[[mixin]]", "[[native.mixin]]"),
                        encoding="utf-8")
    print(f"rewrote {len(files)} files")


if __name__ == "__main__":
    main()
