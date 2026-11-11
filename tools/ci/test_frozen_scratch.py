"""The Python owner reclaims static and incomplete frozen generations."""
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from tools.ci import frozen_scratch


class ScratchTest(unittest.TestCase):
    def test_failure_and_interruption_thaw_only_owned_directories(self):
        local = Path(__file__).resolve().parents[2] / ".local"
        local.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=local) as parent:
            outside = Path(parent) / "outside"
            outside.mkdir()
            outside.chmod(0o500)
            try:
                with patch.dict(os.environ, HORIZON_TEST_SCRATCH_PARENT=parent):
                    for failure in (RuntimeError, KeyboardInterrupt):
                        with self.assertRaises(failure):
                            with frozen_scratch.owned_scratch() as (root, env):
                                temp = Path(env["TMPDIR"])
                                tree = temp / "incomplete-static"
                                tree.mkdir()
                                (tree / "data").write_text("fixture")
                                (tree / "link").symlink_to(outside, target_is_directory=True)
                                (tree / "data").chmod(0o400)
                                tree.chmod(0o500)
                                raise failure()
                        self.assertFalse(root.exists())
                        self.assertEqual(stat.S_IMODE(outside.stat().st_mode), 0o500)
                    status = subprocess.run(
                        [sys.executable, str(Path(frozen_scratch.__file__).resolve()), sys.executable, "-c",
                         "import os,pathlib; p=pathlib.Path(os.environ['TMPDIR'])/'partial';p.mkdir();p.chmod(0o500);raise SystemExit(7)"],
                        check=False,
                    )
                    self.assertEqual(status.returncode, 7)
                    self.assertEqual(list(Path(parent).iterdir()), [outside])
            finally:
                outside.chmod(0o700)

    def test_rejects_memory_backed_parent(self):
        with patch.dict(os.environ, HORIZON_TEST_SCRATCH_PARENT="/tmp"), patch.object(frozen_scratch.subprocess, "check_output", return_value="tmpfs\n"):
            with self.assertRaisesRegex(RuntimeError, "disk-backed"):
                with frozen_scratch.owned_scratch():
                    self.fail("must refuse tmpfs")


if __name__ == "__main__":
    unittest.main()
