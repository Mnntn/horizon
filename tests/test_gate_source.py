"""Gate provenance must describe the actual synthetic merge, even in shallow clones."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from tools.ci import gate_source
from test_ci_concurrency import yaml

ROOT = Path(__file__).resolve().parents[1]


class GateSource(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.repo = self.directory / "repo"
        self.repo.mkdir()
        self.git("init", "-q")
        (self.repo / "base").write_text("base\n")
        self.git("add", "base")
        self.git("commit", "-qm", "base")
        self.base = self.git("rev-parse", "HEAD")
        self.git("checkout", "-qb", "pr")
        (self.repo / "pr").write_text("pr\n")
        self.git("add", "pr")
        self.git("commit", "-qm", "pr")
        self.head = self.git("rev-parse", "HEAD")
        self.git("checkout", "-qb", "tested", self.base)
        self.git("merge", "--no-ff", "pr", "-m", "tested merge")
        self.merge = self.git("rev-parse", "HEAD")
        self.env = dict(PR_NUMBER="3065", PR_HEAD_SHA=self.head, BASE_SHA="f" * 40,
                        GITHUB_SHA=self.merge, GITHUB_RUN_ID="123", GITHUB_RUN_ATTEMPT="2")

    def git(self, *args):
        return subprocess.check_output(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                                        "-c", "core.hooksPath=/dev/null", *args],
                                       cwd=self.repo, text=True, stderr=subprocess.DEVNULL).strip()

    def test_records_actual_base_not_payload_and_typed_identifiers(self):
        self.assertEqual(gate_source.record(self.repo, self.env),
                         dict(schema_version=1, pr=3065, run_id=123, attempt=2,
                              base=self.base, head=self.head, merge=self.merge))

    def test_parent_header_survives_depth_one_clone(self):
        shallow = self.directory / "shallow"
        self.git("clone", "-q", "--depth", "1", self.repo.as_uri(), str(shallow))
        self.assertTrue((shallow / ".git/shallow").exists())
        self.assertEqual(gate_source.record(shallow, self.env)["base"], self.base)

    def test_wrong_checkout_head_or_identifiers_refuse(self):
        for key, value in (("GITHUB_SHA", self.base), ("PR_HEAD_SHA", self.base),
                           ("PR_NUMBER", "0"), ("PR_NUMBER", "x"), ("GITHUB_RUN_ID", "-1"),
                           ("GITHUB_RUN_ATTEMPT", "0"), ("PR_HEAD_SHA", "not-a-sha")):
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                gate_source.record(self.repo, {**self.env, key: value})

    def test_non_merge_pr_checkout_refuses(self):
        self.git("checkout", "-q", self.head)
        with self.assertRaises(ValueError):
            gate_source.record(self.repo, {**self.env, "GITHUB_SHA": self.head})

    def test_main_push_and_manual_record_own_source(self):
        pushed = gate_source.record(self.repo, {**self.env, "PR_NUMBER": "", "BASE_SHA": self.base})
        self.assertIsNone(pushed["pr"])
        self.assertEqual((pushed["base"], pushed["head"], pushed["merge"]), (self.base, self.merge, self.merge))
        manual = gate_source.record(self.repo, {**self.env, "PR_NUMBER": "", "BASE_SHA": ""})
        self.assertEqual(manual["base"], self.merge)

    def test_cli_writes_exact_record_and_summary(self):
        summary = self.directory / "summary"
        result = subprocess.run([sys.executable, str(ROOT / "tools/ci/gate_source.py")], cwd=self.repo,
                                env={**os.environ, **self.env, "GITHUB_STEP_SUMMARY": str(summary)},
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads((self.repo / "gate-source.json").read_text()), gate_source.record(self.repo, self.env))
        for sha in (self.base, self.head, self.merge):
            self.assertIn(sha, summary.read_text())

    def test_gate_uploads_provenance_from_pinned_checkout(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/dyna-ci.yml").read_text())
        steps = workflow["jobs"]["changes"]["steps"]
        checkout = next(step for step in steps if step.get("uses") == "actions/checkout@v4")
        producer = next(step for step in steps if step.get("run") == "python3 tools/ci/gate_source.py")
        upload = next(step for step in steps if step.get("with", {}).get("path") == "gate-source.json")
        self.assertEqual(checkout["with"]["ref"], "${{ github.sha }}")
        self.assertLess(steps.index(checkout), steps.index(producer))
        self.assertLess(steps.index(producer), steps.index(upload))
        self.assertEqual(upload["with"]["name"], "gate-source-${{ github.run_attempt }}")
        self.assertEqual(upload["with"]["if-no-files-found"], "error")


if __name__ == "__main__":
    unittest.main()
