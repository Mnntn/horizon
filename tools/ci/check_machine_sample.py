#!/usr/bin/env python3
"""Compile an immutable machine sample revision against this checkout's API.

API-breaking PRs may pair a sample update using a standalone body line:
Sample commit: <40-character SHA>
Update PIN in the same fork PR, and link the paired sample PR in its description.
The body line is an optional consistency check, never a source of mutable pins.
"""
import json
import os
from pathlib import Path
import re
import subprocess
import time
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PIN = "2c4100a3b4ace3f65e6622f1fe7ae39bcfa67cc6"


def main():
    event_path = os.environ.get("GITHUB_EVENT_PATH")
    event = json.loads(Path(event_path).read_text()) if event_path else {}
    body = event.get("pull_request", {}).get("body") or ""
    match = re.search(r"^Sample commit: ([0-9a-f]{40})\s*$", body, re.MULTILINE)
    if match and match[1] != PIN:
        raise SystemExit("Sample commit must match the committed PIN; update it in this PR")
    revision = PIN
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="machine-sample-", dir=ROOT.parent) as directory:
        check(Path(directory), revision)
    elapsed = time.monotonic() - started
    summary = f"Machine sample `{revision}`: {elapsed:.1f} seconds ({elapsed / 60:.2f} runner minutes).\n"
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as output:
            output.write(summary)


def check(directory, revision):
    sample = directory / "sample"
    subprocess.run(["git", "clone", "--no-checkout", "https://github.com/arismoko/horizon-machine-sample", str(sample)], check=True)
    subprocess.run(["git", "fetch", "origin", revision], cwd=sample, check=True)
    subprocess.run(["git", "checkout", "--detach", revision], cwd=sample, check=True)
    # Cargo source patches replace the pinned game dependency with this PR's API.
    patch = sample / "api-patch.toml"
    patch.write_text('[patch."https://github.com/arismoko/horizon"]\n'
                     + 'horizon-mod-api = { path = ' + json.dumps(str(ROOT / "crates/horizon-mod-api")) + ' }\n')
    subprocess.run(["cargo", "check", "--all-targets", "--config", str(patch)], cwd=sample, check=True)


if __name__ == "__main__":
    main()
