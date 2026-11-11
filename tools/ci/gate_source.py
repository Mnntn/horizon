#!/usr/bin/env python3
"""Record typed gate provenance; PR base comes from the tested merge's first parent."""
import json
import os
from pathlib import Path
import re
import subprocess
from collections.abc import Mapping


def record(root: Path, env: Mapping[str, str]) -> dict:
    def sha(value: str) -> str:
        if not re.fullmatch(r"[0-9a-f]{40}", value):
            raise ValueError("invalid gate source SHA")
        return value

    actual = sha(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip())
    if actual != sha(env["GITHUB_SHA"]):
        raise ValueError("gate checkout does not match its triggering SHA")
    pr_text = env.get("PR_NUMBER", "")
    if pr_text and (not pr_text.isascii() or not pr_text.isdecimal() or int(pr_text) <= 0):
        raise ValueError("invalid gate PR number")
    pr = int(pr_text) if pr_text else None
    if pr is not None:
        # Unlike rev-list, cat-file preserves parent headers in a depth-one checkout.
        header = subprocess.check_output(["git", "cat-file", "-p", "HEAD"], cwd=root, text=True).split("\n\n", 1)[0]
        parents = [sha(line.removeprefix("parent ")) for line in header.splitlines() if line.startswith("parent ")]
        head = sha(env["PR_HEAD_SHA"])
        if len(parents) != 2 or parents[1] != head:
            raise ValueError("gate did not test the expected synthetic PR merge")
        base = parents[0]
    else:
        base = sha(env.get("BASE_SHA") or actual)
        head = actual
    identifiers = []
    for name in ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT"):
        value = env[name]
        if not value.isascii() or not value.isdecimal() or int(value) <= 0:
            raise ValueError("invalid gate run identifier")
        identifiers.append(int(value))
    return dict(schema_version=1, pr=pr, run_id=identifiers[0], attempt=identifiers[1],
                base=base, head=head, merge=actual)


def main() -> None:
    data = record(Path.cwd(), os.environ)
    Path("gate-source.json").write_text(json.dumps(data, sort_keys=True) + "\n")
    with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as summary:
        summary.write("## Gate source provenance\n\n")
        for label, key in (("Tested base", "base"), ("PR head / source head", "head"),
                           ("Tested merge / checkout", "merge")):
            summary.write(f"- {label}: `{data[key]}`\n")
        summary.write(f"- Run: {data['run_id']}, attempt {data['attempt']}\n")


if __name__ == "__main__":
    main()
