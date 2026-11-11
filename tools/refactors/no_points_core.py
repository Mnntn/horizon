#!/usr/bin/env python3
"""Delete the injection-point machinery of the engine and the generator (no-points).

Run once from the repository root. Whole files and crates only; the hand commits after this one
build the replacement (`horizon-mod-api-gen/src/replace.rs`, the staging pass's head checks) and
fix what referenced the deleted code.

- the crates `horizon-mod-points`, `horizon-mod-point-shape`, `horizon-mod-point-emit`;
- the generator's call sites (`sites*`), mod points (`points/`) and their tests;
- `#[point]` (`horizon-mod-api-macros/src/point.rs`) and the slot tables (`__private/points.rs`);
- the slot lists move to `replaceable/`: a function line stays, a call-site line becomes its
  enclosing function when that function is below the game crate (every game-module function is
  replaceable already), and the per-list comments about slots go.
"""
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
GEN = ROOT / "crates/horizon-mod-api-gen"


def git(*args: str) -> None:
    subprocess.run(["git", *args], cwd=ROOT, check=True)


def main() -> None:
    git(
        "rm", "-r", "-q",
        "crates/horizon-mod-points",
        "crates/horizon-mod-point-shape",
        "crates/horizon-mod-point-emit",
        "crates/horizon-mod-api-gen/src/sites.rs",
        "crates/horizon-mod-api-gen/src/sites",
        "crates/horizon-mod-api-gen/src/points",
        "crates/horizon-mod-api-gen/tests/knockback_entries.rs",
        "crates/horizon-mod-api-gen/tests/knockback_fixture",
        "crates/horizon-mod-api-gen/tests/return_authority.rs",
        "crates/horizon-mod-api-macros/src/point.rs",
        "crates/horizon-mod-vocab/src/__private/points.rs",
    )
    # The generic source walk outlives the points: it moves, and the hand commit cuts the rest.
    git("mv", "crates/horizon-mod-api-gen/src/points.rs", "crates/horizon-mod-api-gen/src/source.rs")
    functions: set[str] = set()
    for file in sorted((GEN / "slots").glob("*.txt")):
        for line in file.read_text().splitlines():
            line = line.split("#")[0].strip()
            if not line:
                continue
            target = line.split("|")[0].strip()
            if not target.startswith("horizon_client::"):
                functions.add(target)
    git("rm", "-r", "-q", "crates/horizon-mod-api-gen/slots")
    out = GEN / "replaceable.txt"
    out.write_text(
        "# Functions below the game crate that a mixin can replace, one path per line. Every eligible\n"
        "# function of horizon_client::game is replaceable already.\n"
        + "".join(f"{path}\n" for path in sorted(functions))
    )
    git("add", str(out))


if __name__ == "__main__":
    main()
