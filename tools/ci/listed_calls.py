#!/usr/bin/env python3
"""Run the production listed-call export and the real-registry launcher proof together."""
import argparse
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
INPUTS = (
    "crates/horizon-mod-api-gen/", "crates/horizon-mod-api-macros/",
    "crates/horizon-mod-points/", "crates/horizon-mod-vocab/src/__private/",
    "launcher/src/native_mods/", "tools/ci/listed_calls.py",
    ".github/workflows/dyna-ci.yml",
)


def applies(paths):
    return any(path.startswith(INPUTS) for path in paths)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="HEAD^")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    if not args.force:
        changed = subprocess.check_output(
            ["git", "diff", "--name-only", f"{args.base}...HEAD"], cwd=ROOT, text=True,
        ).splitlines()
        if not applies(changed):
            print("Listed call proof: unrelated paths, not run")
            return
    target = ROOT / "target"
    target.mkdir(exist_ok=True)
    environment = dict(os.environ, HORIZON_LISTED_PROOF_OUT=str(target / "listed-port-proof.json"))
    start = time.monotonic()
    for command in [
        ["cargo", "test", "--locked", "-p", "horizon-mod-api-gen", "--lib", "production_listed_records", "--", "--ignored"],
        ["cargo", "test", "--locked", "--manifest-path", "launcher/Cargo.toml", "--lib", "production_port_listeners_attach_and_invoke", "--", "--ignored"],
    ]:
        subprocess.run(command, cwd=ROOT, env=environment, check=True)
    print(f"Listed call production export and real-registry proof: {time.monotonic() - start:.2f} seconds", flush=True)


if __name__ == "__main__":
    main()
