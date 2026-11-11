"""Path selection for the informational Windows prebuilt check."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "windows_prebuilt", Path(__file__).resolve().parents[1] / "tools/ci/windows_prebuilt.py",
)
windows_prebuilt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(windows_prebuilt)


class WindowsPrebuiltPaths(unittest.TestCase):
    def test_stage_and_toolchain_inputs(self):
        for path in ("crates/horizon-world/src/lib.rs", "Cargo.lock", "Cargo.toml",
                     "packaging/windows/exe.rs", "tools/prebuilt_game.py",
                     ".cargo/config.toml", "rust-toolchain.toml", "rust-toolchain"):
            with self.subTest(path=path):
                self.assertTrue(windows_prebuilt.applies([path]))

    def test_unrelated_changes_do_not_select(self):
        self.assertFalse(windows_prebuilt.applies([
            "docs/modding/src/introduction.md", "README.md", "tools/lab.py",
            "tools/ci/windows_prebuilt.py", ".github/workflows/dyna-ci.yml",
        ]))
        self.assertFalse(windows_prebuilt.applies([]))


if __name__ == "__main__":
    unittest.main()
