#!/usr/bin/env python3
"""Complete the vocabulary move for root modules newly added in rollup 16."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[2]
for name in ("java_math.rs", "strict_math.rs", "strict_math_tests.rs"):
    subprocess.run(["git", "mv", f"crates/horizon-mod-api/src/{name}",
                    f"crates/horizon-mod-vocab/src/{name}"], cwd=root, check=True)
