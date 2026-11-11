"""Resource-backed test processes receive private, disposable homes."""
import os
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools import lab
from tools.ci import ignored_tests
from tools.ci import frozen_scratch
import tempfile


class IgnoredTestsHomeTest(unittest.TestCase):
    def test_each_process_gets_fresh_homes_without_changing_the_parent(self):
        homes = []

        def execute(command, **kwargs):
            env = kwargs["env"]
            home = Path(env["HORIZON_HOME"])
            self.assertTrue(home.is_dir())
            self.assertFalse((home / "fixture-state").exists())
            (home / "fixture-state").touch()
            self.assertEqual(env["HORIZON_TEST_RESOURCES"], "prepared-fixture")
            for key in ("HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
                self.assertTrue(Path(env[key]).is_dir())
                self.assertEqual(Path(env[key]).parent, home.parent)
            for key in ("HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
                self.assertEqual(Path(env[key]).parent, home.parent)
            temporary = Path(env["TMPDIR"])
            self.assertEqual(str(temporary), env["HORIZON_FROZEN_SCRATCH"])
            frozen = temporary / "static-snapshot"
            frozen.mkdir()
            (frozen / "input").write_text("private")
            (frozen / "input").chmod(0o400)
            frozen.chmod(0o500)
            homes.append(home)
            return SimpleNamespace(
                returncode=0,
                stdout=(
                    "     Running unittests src/lib.rs (target/debug/deps/example)\n"
                    "running 1 test\n"
                    "test fixture ... ok\n"
                    "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n"
                ),
            )

        local = Path(__file__).resolve().parents[2] / ".local"
        local.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=local) as parent, patch.dict(os.environ, HORIZON_HOME="parent-home", HORIZON_TEST_RESOURCES="prepared-fixture", HORIZON_TEST_SCRATCH_PARENT=parent), patch.object(frozen_scratch.subprocess, "check_output", return_value="ext2/ext3\n"):
            with patch.object(ignored_tests.lab, "game_env", wraps=lab.game_env) as sandbox:
                with patch.object(ignored_tests.subprocess, "run", side_effect=execute):
                    for _ in range(2):
                        self.assertEqual(ignored_tests.run("horizon-client", []), {"fixture": True})
                        self.assertEqual(os.environ["HORIZON_HOME"], "parent-home")
                self.assertEqual(sandbox.call_count, 2)
            self.assertNotEqual(homes[0], homes[1])
            self.assertTrue(all(not home.parent.exists() for home in homes))


if __name__ == "__main__":
    unittest.main()
