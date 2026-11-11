#!/usr/bin/env python3
"""Own disk-backed fixture scratch, including process-static frozen roots.

Local author entrypoint:
HORIZON_TEST_SCRATCH_PARENT=$PWD/.local/author-scratch python3 tools/ci/frozen_scratch.py cargo test ...

The parent must be private writable disk storage. CI uses DYNA_CI_BUILD.
Normal exit, child failure and Python interruption clean owned directories.
SIGKILL cleanup belongs to the enclosing job/container lifecycle.
"""
import contextlib
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile


def remove_owned_tree(root):
    """Thaw only directories beneath our allocated root, never follow links."""
    for directory, dirs, _ in os.walk(root, topdown=True, followlinks=False):
        path = Path(directory)
        path.chmod(stat.S_IMODE(path.stat().st_mode) | stat.S_IRWXU)
        dirs[:] = [name for name in dirs if not (path / name).is_symlink()]
    shutil.rmtree(root)


@contextlib.contextmanager
def owned_scratch():
    parent = os.environ.get("HORIZON_TEST_SCRATCH_PARENT") or os.environ.get("DYNA_CI_BUILD")
    if not parent:
        raise RuntimeError("set HORIZON_TEST_SCRATCH_PARENT to private disk-backed storage (or DYNA_CI_BUILD in CI)")
    parent = Path(parent).resolve()
    parent.mkdir(parents=True, exist_ok=True)
    kind = subprocess.check_output(["stat", "-f", "-c", "%T", str(parent)], text=True).strip()
    if kind in {"tmpfs", "ramfs"}:
        raise RuntimeError("frozen fixture scratch must be disk-backed, not " + kind)
    root = Path(tempfile.mkdtemp(prefix="horizon-ignored-", dir=parent))
    try:
        temporary = root / "tmp"
        temporary.mkdir()
        yield root, {"TMPDIR": str(temporary), "HORIZON_FROZEN_SCRATCH": str(temporary)}
    finally:
        remove_owned_tree(root)


def main(command):
    if not command:
        raise SystemExit("usage: frozen_scratch.py COMMAND [ARG ...]")
    with owned_scratch() as (root, scratch_env):
        env = os.environ | scratch_env
        for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                    "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
            home = root / key.lower()
            home.mkdir()
            env[key] = str(home)
        env.update(HORIZON_NO_AUDIO="1", HORIZON_LAUNCHER_NO_SELF_UPDATE="1")
        return subprocess.call(command, env=env)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
