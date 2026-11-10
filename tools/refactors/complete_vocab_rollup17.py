#!/usr/bin/env python3
"""Complete the vocabulary move for the root module added in rollup 17."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[2]
subprocess.run(["git", "mv", "crates/horizon-mod-api/src/particle_groups.rs",
                "crates/horizon-mod-vocab/src/particle_groups.rs"], cwd=root, check=True)
