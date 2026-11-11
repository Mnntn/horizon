"""Path selection for the informational Windows prebuilt check."""
import ast
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from tests.test_ci_concurrency import yaml

ROOT = Path(__file__).resolve().parents[1]

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


class WindowsPrebuiltHistory(unittest.TestCase):
    def test_renames_select_original_and_destination_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, text=True).strip()

            git("init", "-q")
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.invalid")
            (root / "crates").mkdir()
            (root / "docs").mkdir()
            (root / "crates/item.rs").write_text("unchanged content\n")
            git("add", ".")
            git("commit", "-qm", "Initial")
            for source, destination in (("crates/item.rs", "docs/item.rs"),
                                        ("docs/item.rs", "crates/item.rs")):
                before = git("rev-parse", "HEAD")
                git("mv", source, destination)
                git("commit", "-qm", "Move")
                # Point the CLI at the fixture repository, preserving argument
                # parsing and the actual git diff subprocess.
                result = subprocess.check_output([
                    sys.executable, "-c",
                    "import sys; from pathlib import Path; "
                    f"sys.path.insert(0, {str(ROOT / 'tools/ci')!r}); "
                    "import windows_prebuilt as w; w.ROOT=Path.cwd(); w.main()",
                    "--select", before, "HEAD",
                ], cwd=root, text=True)
                self.assertEqual(result.strip(), "required=true")


class WindowsPrebuiltWorkflow(unittest.TestCase):
    def test_independent_job_and_selection_binding(self):
        jobs = yaml.safe_load((ROOT / ".github/workflows/dyna-ci.yml").read_text())["jobs"]
        job = jobs["windows-prebuilt-check"]
        self.assertEqual(job["needs"], "changes")
        self.assertNotIn("windows-prebuilt-check", jobs["verify"]["needs"])
        self.assertEqual(job["if"], "needs.changes.outputs.windows_prebuilt_required == 'true'")
        self.assertEqual(jobs["changes"]["outputs"]["windows_prebuilt_required"],
                         "${{ steps.windows_prebuilt.outputs.required || 'false' }}")
        selector = next(step for step in jobs["changes"]["steps"]
                        if step.get("id") == "windows_prebuilt")
        self.assertIn('windows_prebuilt.py --select "$BASE_SHA" "$HEAD_SHA"', selector["run"])
        checkout = next(step for step in job["steps"]
                        if step.get("uses", "").startswith("actions/checkout@"))
        self.assertEqual(checkout["with"]["ref"], "${{ github.sha }}")

    def test_picker_does_not_require_new_job(self):
        tree = ast.parse((ROOT / "tools/release_train.py").read_text())
        calls = [node for node in ast.walk(tree) if isinstance(node, ast.Call)
                 and isinstance(node.func, ast.Name) and node.func.id == "gate_jobs_passed"]
        self.assertTrue(calls)
        strings = {node.value for node in ast.walk(tree)
                   if isinstance(node, ast.Constant) and isinstance(node.value, str)}
        self.assertNotIn("windows-prebuilt-check", strings)


if __name__ == "__main__":
    unittest.main()
