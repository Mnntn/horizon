"""Dry picks must never become release bookkeeping, including after failure."""
import contextlib
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest
from unittest import mock

from tools.ci import nightly

ROOT = Path(__file__).resolve().parents[1]


class DryStateTests(unittest.TestCase):
    def test_only_real_builds_write_state_and_consume_the_daily_quota(self):
        for dry in (True, False):
            with self.subTest(dry=dry), tempfile.TemporaryDirectory() as folder, contextlib.chdir(folder), \
                    mock.patch.dict(os.environ, {"GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "2"}, clear=True), \
                    mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                    mock.patch.object(nightly.ns, "assemble", return_value="a" * 40), \
                    mock.patch.object(nightly.ns, "builds_today", return_value=[]) as quota, \
                    mock.patch.object(nightly.ns, "push", return_value=True) as push, \
                    mock.patch.object(nightly.ns, "update_state") as update, \
                    mock.patch.object(nightly, "git", return_value=""), \
                    mock.patch.object(nightly, "merged_since", return_value=[]), \
                    mock.patch.object(nightly.train, "known_issues", return_value=[]), \
                    mock.patch.object(nightly, "output") as output:
                nightly.build_set(nightly.Ref("main", "a" * 40, dt.datetime.now(dt.timezone.utc)),
                                  [], [], "2026-11-10", True, dry)
                expected = "nightly/dry-run-123-2" if dry else "nightly/build-2026-11-10-1"
                self.assertEqual(push.call_args.args[:2], ("a" * 40, expected))
                self.assertEqual(output.call_args.kwargs["source"], expected)
                if dry:
                    quota.assert_not_called()
                    update.assert_not_called()
                else:
                    quota.assert_called_once_with("2026-11-10")
                    update.assert_called_once()
                    _, history, _ = update.call_args.args[0]({}, "")
                    self.assertEqual(json.loads(history)["ref"], expected)
                    self.assertFalse(json.loads(history)["dry_run"])

    def test_dry_pick_cannot_push_without_an_unambiguous_cleanup_identity(self):
        with mock.patch.dict(os.environ, {}, clear=True), \
                mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                mock.patch.object(nightly.ns, "assemble", return_value="a" * 40), \
                mock.patch.object(nightly, "git", return_value=""), \
                mock.patch.object(nightly.ns, "push") as push, \
                mock.patch.object(nightly.ns, "update_state") as update:
            with self.assertRaises(SystemExit):
                nightly.build_set(nightly.Ref("main", "a" * 40, None), [], [], "2026-11-10", True, True)
            push.assert_not_called()
            update.assert_not_called()

    def test_dry_schedule_never_refreshes_the_saved_selection(self):
        with mock.patch.dict(os.environ, {"DRY_RUN": "true", "GITHUB_EVENT_NAME": "schedule"}, clear=True), \
                mock.patch.object(nightly, "git", return_value=""), \
                mock.patch.object(nightly, "green_base", return_value=nightly.Ref("main", "a" * 40, None)), \
                mock.patch.object(nightly.ns, "load_state", return_value=({"prs": [{"number": 1}]}, "", None)), \
                mock.patch.object(nightly.ns, "update_state") as update, \
                mock.patch.object(nightly, "build_set") as build:
            nightly.pick(True)
            update.assert_not_called()
            self.assertIs(build.call_args.args[-1], True)

    def test_cleanup_is_independent_of_pick_outputs_and_fails_on_api_errors(self):
        workflow = (ROOT / ".github/workflows/nightly-build.yml").read_text()
        job = workflow.split("\n  cleanup-dry-run:\n", 1)[1].split("\n  answer:", 1)[0]
        self.assertIn("needs: [pick, build, sign, publish]", job)
        self.assertIn("if: always() && github.event_name == 'workflow_dispatch' && (inputs.dry_run || github.ref != 'refs/heads/main')", job)
        script = textwrap.dedent(job.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            curl = path / "curl"
            curl.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$CAPTURE"\nprintf "%s" "$STATUS"\n')
            curl.chmod(0o755)
            for status, success in (("204", True), ("404", True), ("403", False), ("500", False)):
                with self.subTest(status=status):
                    env = {**os.environ, "PATH": folder + os.pathsep + os.environ["PATH"],
                           "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "2", "GH_TOKEN": "test",
                           "GITHUB_API_URL": "https://api.github.com", "GITHUB_REPOSITORY": "arismoko/horizon",
                           "RUNNER_TEMP": folder, "CAPTURE": str(path / "request"), "STATUS": status}
                    result = subprocess.run(["bash", "-euo", "pipefail", "-c", script], env=env, capture_output=True)
                    self.assertEqual(result.returncode == 0, success)
                    last_attempt = 2 if success else 1
                    self.assertIn(f"https://api.github.com/repos/arismoko/horizon/git/refs/heads/nightly/dry-run-123-{last_attempt}",
                                  (path / "request").read_text().splitlines())

    def test_failed_cleanup_retry_removes_the_producers_earlier_attempt_only(self):
        workflow = (ROOT / ".github/workflows/nightly-build.yml").read_text()
        job = workflow.split("\n  cleanup-dry-run:\n", 1)[1].split("\n  answer:", 1)[0]
        script = textwrap.dedent(job.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            original = "nightly/dry-run-123-1"
            unrelated = {"nightly/dry-run-1234-1", "nightly/dry-run-124-1", "nightly/build-2026-11-10-1"}
            (path / "refs").write_text(json.dumps(sorted(unrelated | {original})))
            (path / "fail-once").touch()
            curl = path / "curl"
            curl.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ["RUNNER_TEMP"])
ref = sys.argv[-1].split("/git/refs/heads/", 1)[1]
if (root / "fail-once").exists():
    (root / "fail-once").unlink()
    print("500", end="")
else:
    refs = set(json.loads((root / "refs").read_text()))
    status = "204" if ref in refs else "404"
    refs.discard(ref)
    (root / "refs").write_text(json.dumps(sorted(refs)))
    print(status, end="")
''')
            curl.chmod(0o755)
            env = {**os.environ, "PATH": folder + os.pathsep + os.environ["PATH"],
                   "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "1", "GH_TOKEN": "test",
                   "GITHUB_API_URL": "https://api.github.com", "GITHUB_REPOSITORY": "arismoko/horizon",
                   "RUNNER_TEMP": folder}
            first = subprocess.run(["bash", "-euo", "pipefail", "-c", script], env=env, capture_output=True)
            self.assertNotEqual(first.returncode, 0)
            self.assertIn(original, json.loads((path / "refs").read_text()))
            # Successful pick was not rerun: only cleanup has a new attempt number.
            env["GITHUB_RUN_ATTEMPT"] = "2"
            retry = subprocess.run(["bash", "-euo", "pipefail", "-c", script], env=env, capture_output=True)
            self.assertEqual(retry.returncode, 0, retry.stderr)
            self.assertEqual(set(json.loads((path / "refs").read_text())), unrelated)
