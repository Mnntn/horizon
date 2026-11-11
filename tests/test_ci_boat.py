"""Boat routing must distinguish runner outages from real test failures."""
import importlib.util
from pathlib import Path
import unittest
import sys
from unittest.mock import patch
import os
import tempfile
import io
import json
import zipfile
import zlib
import subprocess
from types import SimpleNamespace
from datetime import datetime, timezone

spec = importlib.util.spec_from_file_location("boat_health", Path(__file__).parents[1] / "tools/ci/boat_health.py")
boat = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boat)
sys.modules["boat_health"] = boat
watch_spec = importlib.util.spec_from_file_location("boat_watch", Path(__file__).parents[1] / "tools/ci/boat_watch.py")
watch = importlib.util.module_from_spec(watch_spec)
watch_spec.loader.exec_module(watch)
controller_spec = importlib.util.spec_from_file_location("boat_controller", Path(__file__).parents[1] / "tools/ci/boat_controller.py")
controller = importlib.util.module_from_spec(controller_spec)
controller_spec.loader.exec_module(controller)
NOW = datetime(2026, 11, 10, 12, tzinfo=timezone.utc)


def job(**changes):
    return dict(id=42, labels=["boat-large"], status="queued", conclusion=None,
                created_at="2026-11-10T11:49:59Z", steps=[], **changes)


class BoatHealthTests(unittest.TestCase):
    def test_stale_queue_falls_back(self):
        label, reason = boat.select_label("on", ["alice"], "alice", "alice", [job()], NOW)
        self.assertEqual(label, "ubuntu-latest")
        self.assertIn("42", reason)

    def test_threshold_is_strict(self):
        sample = job()
        sample["created_at"] = "2026-11-10T11:50:00Z"
        self.assertEqual(boat.health_reason([sample], NOW), "")

    def test_setup_failure(self):
        sample = job()
        sample.update(status="completed", conclusion="failure", steps=[
            dict(name="Set up job", started_at="2026-11-10T11:51:00Z", conclusion="failure"),
            dict(name="Complete job", started_at="2026-11-10T11:51:01Z")])
        self.assertIn("first step", boat.health_reason([sample], NOW))

    def test_real_failure_is_not_retried(self):
        sample = job()
        sample.update(status="completed", conclusion="failure", steps=[
            dict(name="Run tests", started_at="2026-11-10T11:51:00Z", conclusion="failure"),
            dict(name="Complete job", started_at="2026-11-10T11:51:01Z")])
        self.assertEqual(boat.health_reason([sample], NOW), "")

    def test_other_runners_do_not_poison_boat(self):
        sample = job()
        sample["labels"] = ["dyna-large"]
        self.assertEqual(boat.health_reason([sample], NOW), "")

    def test_mixed_real_failure_does_not_replace_run(self):
        queued = job()
        queued["name"] = "verify"
        failed = job()
        failed.update(name="lint", status="completed", conclusion="failure", steps=[
            dict(name="Run tests", started_at="2026-11-10T11:51:00Z")])
        self.assertEqual(watch.recovery_reason([queued, failed], NOW), "")
        failed["name"] = "launcher"
        self.assertEqual(watch.recovery_reason([queued, failed], NOW), "")
        failed["conclusion"] = "timed_out"
        self.assertEqual(watch.recovery_reason([queued, failed], NOW), "")
        self.assertEqual(boat.previous_attempt_reason([queued, failed], NOW), "")
        self.assertIn("queued", watch.recovery_reason([queued], NOW))

    def test_previous_cancelled_queue_retains_outage(self):
        sample = job()
        sample.update(name="lint", status="completed", conclusion="cancelled", completed_at="2026-11-10T12:00:01Z")
        self.assertIn("previous attempt", boat.previous_attempt_reason([sample], NOW))
        sample["completed_at"] = "2026-11-10T11:50:00Z"
        self.assertEqual(boat.previous_attempt_reason([sample], NOW), "")
        sample["completed_at"] = "2026-11-10T12:00:01Z"
        sample["steps"] = [dict(name="Run tests", started_at="2026-11-10T11:51:00Z")]
        self.assertEqual(boat.previous_attempt_reason([sample], NOW), "")

    def test_controller_limits_to_first_fork_attempt(self):
        run = dict(workflow_id=123, event="pull_request", run_attempt=1, status="queued", head_repository=dict(full_name="other/horizon"))
        self.assertTrue(controller.eligible(run, "arismoko/horizon", 123))
        for key, value in [("run_attempt", 2), ("event", "push"), ("workflow_id", 999), ("head_repository", dict(full_name="arismoko/horizon"))]:
            self.assertFalse(controller.eligible(dict(run, **{key: value}), "arismoko/horizon", 123))

    def test_controller_cancel_then_rerun(self):
        sample = job()
        sample["name"] = "lint"
        cancelled = dict(sample, status="completed", conclusion="cancelled", completed_at="2026-11-10T12:00:01Z")
        with patch.object(controller, "api", side_effect=[{"jobs": [sample]}, {"status": "completed"}, {"jobs": [cancelled]}]), patch.object(controller, "post") as post, patch.object(controller, "log"), patch.object(controller, "datetime") as clock:
            clock.now.return_value = NOW
            controller.recover("arismoko/horizon", dict(id=123, status="in_progress"))
        self.assertEqual([call.args[0] for call in post.call_args_list], [
            "repos/arismoko/horizon/actions/runs/123/cancel", "repos/arismoko/horizon/actions/runs/123/rerun"])

    def test_controller_does_not_rerun_late_real_failure(self):
        sample = job()
        sample["name"] = "lint"
        failed = dict(sample, status="completed", conclusion="failure", steps=[dict(name="Run tests", started_at="2026-11-10T11:51:00Z")])
        with patch.object(controller, "api", side_effect=[{"jobs": [sample]}, {"status": "completed"}, {"jobs": [failed]}]), patch.object(controller, "post") as post, patch.object(controller, "log"), patch.object(controller, "datetime") as clock:
            clock.now.return_value = NOW
            controller.recover("arismoko/horizon", dict(id=123, status="in_progress"))
        self.assertEqual(len(post.call_args_list), 1)
        self.assertTrue(post.call_args.args[0].endswith("/cancel"))

    def test_controller_completed_setup_failure_needs_no_cancel(self):
        sample = job()
        sample.update(name="verify", status="completed", conclusion="failure")
        with patch.object(controller, "api", return_value={"jobs": [sample]}), patch.object(controller, "post") as post, patch.object(controller, "log"), patch.object(controller, "datetime") as clock:
            clock.now.return_value = NOW
            controller.recover("arismoko/horizon", dict(id=123, status="completed"))
        post.assert_called_once_with("repos/arismoko/horizon/actions/runs/123/rerun")

    def test_controller_real_failure_never_calls_api_writes(self):
        sample = job()
        sample.update(name="lint", status="completed", conclusion="failure", steps=[dict(name="Run tests", started_at="2026-11-10T11:51:00Z")])
        with patch.object(controller, "api", return_value={"jobs": [sample]}), patch.object(controller, "post") as post, patch.object(controller, "log"), patch.object(controller, "datetime") as clock:
            clock.now.return_value = NOW
            controller.recover("arismoko/horizon", dict(id=123, status="completed"))
        post.assert_not_called()

    def test_rerun_gate_outputs_forced_github_and_summary(self):
        sample = job()
        sample.update(name="lint", status="completed", conclusion="cancelled", completed_at="2026-11-10T12:00:01Z")
        with tempfile.TemporaryDirectory() as directory:
            output, summary = Path(directory) / "output", Path(directory) / "summary"
            with patch.dict(os.environ, dict(GITHUB_REPOSITORY="arismoko/horizon", GITHUB_RUN_ID="123", GITHUB_RUN_ATTEMPT="2", BOAT_CI="off", AUTHOR="alice", ACTOR="alice", GITHUB_OUTPUT=str(output), GITHUB_STEP_SUMMARY=str(summary))), patch.object(boat, "api", return_value={"jobs": [sample]}):
                boat.main()
            self.assertIn("label=ubuntu-latest", output.read_text())
            self.assertIn("forced_github=true", output.read_text())
            self.assertIn("previous attempt", summary.read_text())

    def test_support_requires_original_trusted_gate_and_unmodified_pr(self):
        run = dict(pull_requests=[dict(base=dict(sha="base"))], head_sha="head")
        with patch.object(controller, "api", side_effect=[dict(sha="same")] * 4 + [dict(files=[], merge_base_commit=dict(sha="base"))]):
            self.assertEqual(controller.unsupported_reason("repo", run, "trusted"), "")
        with patch.object(controller, "api", side_effect=[dict(sha="same")] * 4 + [dict(files=[], merge_base_commit=dict(sha="older"))]):
            self.assertIn("not an ancestor", controller.unsupported_reason("repo", run, "trusted"))
        with patch.object(controller, "api", side_effect=[dict(sha="old"), dict(sha="new")]):
            self.assertIn("original base", controller.unsupported_reason("repo", run, "trusted"))
        for files in ([dict(filename="tools/ci/boat_health.py")], [dict(filename="renamed", previous_filename=".github/workflows/dyna-ci.yml")], [dict(filename="other")] * 300):
            with patch.object(controller, "api", side_effect=[dict(sha="same")] * 4 + [dict(files=files, merge_base_commit=dict(sha="base"))]):
                self.assertTrue(controller.unsupported_reason("repo", run, "trusted"))
        with patch.object(controller, "api") as api, patch.object(controller, "recorded_base", return_value=""):
            self.assertIn("unavailable", controller.unsupported_reason("repo", dict(pull_requests=[]), "trusted"))
            api.assert_not_called()

    def test_active_candidates_are_paginated_separately_from_failures(self):
        first_page = [dict(id=index) for index in range(100)]
        with patch.object(controller, "api", side_effect=[dict(workflow_runs=first_page), dict(workflow_runs=[dict(id=100)]), dict(workflow_runs=[]), dict(workflow_runs=[])]) as api:
            result = list(controller.candidates("repo", 123))
        self.assertEqual(len(result), 101)
        urls = [call.args[0] for call in api.call_args_list]
        self.assertIn("status=queued", urls[0])
        self.assertIn("page=2", urls[1])
        self.assertIn("status=in_progress", urls[2])
        self.assertIn("status=failure", urls[3])

    def test_real_fork_shape_uses_validated_recorded_base(self):
        # Observed API shape: run 42566379987 (KBroneZ/horizon), no PR association.
        run = dict(id=42566379987, head_sha="72d5f90f5aacc33db72a0592ffd7c58ace36dbf0", pull_requests=[])
        metadata = dict(run_id=str(run["id"]), attempt="1", head=run["head_sha"], base="b" * 40)
        for field, value in [(None, None), ("run_id", "123"), ("head", "c" * 40), ("base", "not-a-sha"), ("attempt", "2")]:
            record = dict(metadata)
            if field:
                record[field] = value
            archive = io.BytesIO()
            with zipfile.ZipFile(archive, "w") as zipped:
                zipped.writestr("boat-base.json", json.dumps(record))
            artifact = dict(id=99, name="boat-base-1", expired=False, size_in_bytes=len(archive.getvalue()))
            with patch.object(controller, "api", return_value=dict(artifacts=[artifact])), patch.object(controller.subprocess, "run", return_value=SimpleNamespace(stdout=archive.getvalue())):
                self.assertEqual(controller.recorded_base("repo", run), "" if field else "b" * 40)
        with patch.object(controller, "recorded_base", return_value="b" * 40), patch.object(controller, "api", side_effect=[dict(sha="same")] * 4 + [dict(files=[], merge_base_commit=dict(sha="b" * 40))]):
            self.assertEqual(controller.unsupported_reason("repo", run, "trusted"), "")

    def test_metadata_artifact_is_bounded_and_never_extracted(self):
        run = dict(id=123, head_sha="a" * 40)
        for artifacts in ([], [dict(id=99, name="boat-base-1", expired=False, size_in_bytes=65537)]):
            with patch.object(controller, "api", return_value=dict(artifacts=artifacts)), patch.object(controller.subprocess, "run") as download:
                self.assertEqual(controller.recorded_base("repo", run), "")
                download.assert_not_called()
        for filename, data in [("../boat-base.json", "{}"), ("boat-base.json", "x" * 4097)]:
            archive = io.BytesIO()
            with zipfile.ZipFile(archive, "w") as zipped:
                zipped.writestr(filename, data)
            artifact = dict(id=99, name="boat-base-1", expired=False, size_in_bytes=len(archive.getvalue()))
            with patch.object(controller, "api", return_value=dict(artifacts=[artifact])), patch.object(controller.subprocess, "run", return_value=SimpleNamespace(stdout=archive.getvalue())):
                self.assertEqual(controller.recorded_base("repo", run), "")

    def test_sweep_continues_after_one_candidate_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            event = Path(directory) / "event.json"
            event.write_text("{}")
            with patch.dict(os.environ, dict(GITHUB_REPOSITORY="repo", GITHUB_EVENT_NAME="schedule", GITHUB_EVENT_PATH=str(event), GITHUB_WORKFLOW_SHA="trusted")), patch.object(controller, "api", return_value=dict(id=123)), patch.object(controller, "candidates", return_value=[dict(id=1), dict(id=2)]), patch.object(controller, "consider", side_effect=[zlib.error("corrupt PR data"), None]) as consider, patch.object(controller, "log") as log:
                controller.main()
            self.assertEqual([call.args[1]["id"] for call in consider.call_args_list], [1, 2])
            self.assertIn("Run 1", log.call_args.args[0])

    def test_prior_api_failure_forces_github_instead_of_aborting(self):
        for failure in (subprocess.CalledProcessError(1, "gh"), json.JSONDecodeError("bad", "", 0), KeyError("jobs")):
            with tempfile.TemporaryDirectory() as directory:
                output, summary = Path(directory) / "output", Path(directory) / "summary"
                with patch.dict(os.environ, dict(GITHUB_REPOSITORY="repo", GITHUB_RUN_ID="123", GITHUB_RUN_ATTEMPT="2", BOAT_CI="off", AUTHOR="alice", ACTOR="alice", GITHUB_OUTPUT=str(output), GITHUB_STEP_SUMMARY=str(summary))), patch.object(boat, "api", side_effect=failure):
                    boat.main()
                self.assertIn("forced_github=true", output.read_text())
                self.assertIn("label=ubuntu-latest", output.read_text())
                self.assertIn("history is unavailable", summary.read_text())

    def test_corrupt_deflate_artifact_fails_closed(self):
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zipped:
            zipped.writestr("boat-base.json", "{}")
        corrupted = bytearray(archive.getvalue())
        corrupted[30 + len("boat-base.json")] = 0xff  # Reserved deflate block type.
        with zipfile.ZipFile(io.BytesIO(corrupted)) as zipped:
            with self.assertRaises(zlib.error):
                zipped.read("boat-base.json")
        artifact = dict(id=99, name="boat-base-1", expired=False, size_in_bytes=len(corrupted))
        with patch.object(controller, "api", return_value=dict(artifacts=[artifact])), patch.object(controller.subprocess, "run", return_value=SimpleNamespace(stdout=bytes(corrupted))):
            self.assertEqual(controller.recorded_base("repo", dict(id=123, head_sha="a" * 40)), "")

    def test_failure_sweep_only_inspects_recently_updated_runs(self):
        old = dict(id=1, updated_at="2026-11-10T10:59:59Z")
        recent = dict(id=2, updated_at="2026-11-10T11:30:00Z")
        with patch.object(controller, "api", side_effect=[dict(workflow_runs=[]), dict(workflow_runs=[]), dict(workflow_runs=[old, recent])]), patch.object(controller, "datetime", wraps=datetime) as clock:
            clock.now.return_value = NOW
            result = list(controller.candidates("repo", 123))
        self.assertEqual(result, [recent])

    def test_cancel_timeout_never_requests_rerun(self):
        sample = job()
        sample["name"] = "lint"
        with patch.object(controller, "api", side_effect=[dict(jobs=[sample])] + [dict(status="in_progress")] * 20), patch.object(controller, "post") as post, patch.object(controller, "log"), patch.object(controller.time, "sleep"), patch.object(controller, "datetime") as clock:
            clock.now.return_value = NOW
            with self.assertRaises(RuntimeError):
                controller.recover("repo", dict(id=123, status="in_progress"))
        post.assert_called_once_with("repos/repo/actions/runs/123/cancel")

    def test_allowlist_and_switch(self):
        for enabled, author, actor in [("off", "alice", "alice"), ("on", "bob", "alice"), ("on", "alice", "bob")]:
            self.assertEqual(boat.select_label(enabled, ["alice"], author, actor, [], NOW)[0], "ubuntu-latest")
        self.assertEqual(boat.select_label("on", ["alice"], "alice", "alice", [], NOW)[0], "boat-large")


REPO = "arismoko/horizon"


def routing_run(**changes):
    run = dict(id=77, path=".github/workflows/boat-recovery.yml", head_branch="main", event="schedule",
               repository=dict(full_name=REPO), head_repository=dict(full_name=REPO))
    run.update(changes)
    return run


def routing_artifact(**changes):
    artifact = dict(id=5, name="boat-routing", expired=False, size_in_bytes=300, created_at="2026-11-10T11:50:00Z",
                    workflow_run=dict(id=77, repository_id=1, head_repository_id=1))
    artifact.update(changes)
    return artifact


def routing_zip(document=None, members=None):
    archive = io.BytesIO()
    with zipfile.ZipFile(archive, "w") as zipped:
        for name, data in (members or [("boat-routing.json", json.dumps(document))]):
            zipped.writestr(name, data)
    return archive.getvalue()


VALID = dict(schema=1, enabled="on", users=["alice"], run_id="77")


class BoatRoutingTests(unittest.TestCase):
    def read(self, runs=None, artifacts=None, archive=None):
        responses = {"default": dict(default_branch="main"),
                     "runs": dict(workflow_runs=[routing_run()] if runs is None else runs),
                     "artifacts": dict(artifacts=[routing_artifact()] if artifacts is None else artifacts)}

        def api(path):
            if path == f"repos/{REPO}":
                return responses["default"]
            return responses["runs" if "/workflows/" in path else "artifacts"]
        with patch.object(boat, "api", side_effect=api), patch.object(boat, "download", return_value=routing_zip(VALID) if archive is None else archive) as download:
            return boat.published_routing(REPO, NOW), download

    def test_fresh_trusted_artifact_is_read(self):
        (result, reason), _ = self.read()
        self.assertEqual(result, ("on", ["alice"]))
        self.assertEqual(reason, "")

    def test_untrusted_runs_are_never_read(self):
        for run in (routing_run(path=".github/workflows/dyna-ci.yml"), routing_run(head_branch="feature"),
                    routing_run(event="pull_request"), routing_run(head_repository=dict(full_name="fork/horizon")),
                    routing_run(repository=dict(full_name="fork/horizon"))):
            (result, reason), download = self.read(runs=[run])
            self.assertIsNone(result)
            self.assertIn("no recent", reason)
            download.assert_not_called()

    def test_bad_artifacts_fail_closed(self):
        cases = [
            [routing_artifact(), routing_artifact(id=6)],
            [routing_artifact(created_at="2026-11-10T11:39:59Z")],
            [routing_artifact(size_in_bytes=16385)],
            [routing_artifact(expired=True)],
            [routing_artifact(workflow_run=dict(id=78, repository_id=1, head_repository_id=1))],
            [routing_artifact(workflow_run=dict(id=77, repository_id=1, head_repository_id=2))],
        ]
        for artifacts in cases:
            (result, reason), download = self.read(artifacts=artifacts)
            self.assertIsNone(result, artifacts)
            self.assertTrue(reason)
            download.assert_not_called()

    def test_bad_documents_fail_closed(self):
        archives = [routing_zip(members=[("boat-routing.json", "{")]),
                    routing_zip(members=[("boat-routing.json", json.dumps(VALID)), ("other", "x")]),
                    routing_zip(members=[("../boat-routing.json", json.dumps(VALID))]),
                    routing_zip(members=[("boat-routing.json", " " * 8193 + json.dumps(VALID))]),
                    b"not a zip"]
        for change in (dict(schema=2), dict(enabled="yes"), dict(users=["bad login"]), dict(users=["-alice"]),
                       dict(users=["a"] * 101), dict(users="alice"), dict(run_id="78"), dict(extra=1)):
            archives.append(routing_zip({**VALID, **change}))
        for archive in archives:
            (result, reason), _ = self.read(archive=archive)
            self.assertIsNone(result)
            self.assertTrue(reason)

    def test_publisher_round_trips_and_refuses_bad_lists(self):
        text = boat.publish("on", " alice,bob ,", 77)
        self.assertEqual(boat.routing(json.loads(text), 77), ("on", ["alice", "bob"]))
        self.assertEqual(json.loads(boat.publish("", "", 77))["enabled"], "off")
        for users in ("not a login", ",".join(["a"] * 101)):
            with self.assertRaises(SystemExit):
                boat.publish("on", users, 77)

    def run_main(self, published=None, failure=None, **env):
        values = dict(GITHUB_REPOSITORY=REPO, GITHUB_RUN_ID="9", AUTHOR="alice", ACTOR="alice", BOAT_CI="",
                      BOAT_USERS="", BOAT_FALLBACK="false")
        values.update(env)
        with tempfile.TemporaryDirectory() as directory:
            output, summary = Path(directory) / "output", Path(directory) / "summary"
            values.update(GITHUB_OUTPUT=str(output), GITHUB_STEP_SUMMARY=str(summary))
            reader = patch.object(boat, "published_routing", side_effect=failure, return_value=published)
            with patch.dict(os.environ, values, clear=True), reader as read, patch.object(boat, "api", return_value=dict(workflow_runs=[])), patch("builtins.print"):
                boat.main()
            return output.read_text(), read

    def test_fork_run_uses_published_routing(self):
        output, read = self.run_main(published=(("on", ["alice"]), ""))
        self.assertIn("label=boat-large", output)
        read.assert_called_once()

    def test_unlisted_author_or_actor_stays_on_github(self):
        for env in (dict(AUTHOR="mallory"), dict(ACTOR="mallory")):
            output, _ = self.run_main(published=(("on", ["alice"]), ""), **env)
            self.assertIn("label=ubuntu-latest", output)

    def test_unavailable_routing_stays_on_github(self):
        for published, failure in (((None, "no recent boat routing artifact"), None), (None, subprocess.CalledProcessError(1, "gh")), (None, KeyError("artifacts"))):
            output, _ = self.run_main(published=published, failure=failure)
            self.assertIn("label=ubuntu-latest", output)

    def test_fallback_wins_and_never_reads(self):
        output, read = self.run_main(published=(("on", ["alice"]), ""), BOAT_FALLBACK="true")
        self.assertIn("label=ubuntu-latest", output)
        read.assert_not_called()
        output, read = self.run_main(published=(("on", ["alice"]), ""), BOAT_FALLBACK="true", BOAT_CI="on", BOAT_USERS="alice")
        self.assertIn("label=ubuntu-latest", output)
        read.assert_not_called()

    def test_present_variables_are_never_overridden(self):
        output, read = self.run_main(published=(("on", ["alice"]), ""), BOAT_CI="off", BOAT_USERS="alice")
        self.assertIn("label=ubuntu-latest", output)
        read.assert_not_called()
        output, read = self.run_main(BOAT_CI="on", BOAT_USERS="bob, alice")
        self.assertIn("label=boat-large", output)
        read.assert_not_called()


class RustupRetryTests(unittest.TestCase):
    def run_script(self, failures):
        script = Path(__file__).parents[1] / "tools/ci/rustup_install.sh"
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            count = bin_dir / "count"
            (bin_dir / "rustup").write_text(f'#!/bin/sh\necho "$@" >> {count}\n[ $(wc -l < {count}) -gt {failures} ]\n')
            (bin_dir / "sleep").write_text("#!/bin/sh\n")
            for name in ("rustup", "sleep"):
                (bin_dir / name).chmod(0o755)
            result = subprocess.run(["bash", str(script), "stable", "--profile", "minimal"], capture_output=True, text=True,
                                    env={**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}"})
            return result.returncode, count.read_text().splitlines()

    @unittest.skipIf(os.name == "nt", "a bash script for Linux runners")
    def test_transient_failures_are_retried_then_bounded(self):
        status, calls = self.run_script(failures=2)
        self.assertEqual((status, len(calls)), (0, 3))
        self.assertEqual(calls[0], "toolchain install stable --profile minimal")
        status, calls = self.run_script(failures=10)
        self.assertNotEqual(status, 0)
        self.assertEqual(len(calls), 4)


class RunnerPackagesTests(unittest.TestCase):
    def test_hosted_steps_install_the_image_list_and_the_fallback_stays_within_it(self):
        root = Path(__file__).parents[1]
        listed = [line for line in (root / "tools/ci/runner-packages.txt").read_text().splitlines() if not line.startswith("#")]
        self.assertEqual(len(listed), len(set(listed)))
        workflow = (root / ".github/workflows/dyna-ci.yml").read_text()
        self.assertEqual(workflow.count("$(grep -v '^#' tools/ci/runner-packages.txt)"), 2)
        deps = (root / "tools/ci/dyna_ci_deps.sh").read_text()
        fallback = deps.split("packages=(", 1)[1].split(")", 1)[0].split()
        self.assertTrue(fallback)
        self.assertLessEqual(set(fallback), set(listed))

    def test_the_image_check_fails_on_any_difference(self):
        spec = importlib.util.spec_from_file_location("check_runner_packages", Path(__file__).parents[1] / "tools/ci/check_runner_packages.py")
        check = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(check)
        image = ("FROM ${DYNA_CI_BASE}\nRUN apt-get update && apt-get install -y --no-install-recommends \\\n"
                 "    xvfb xauth x11-xkb-utils xkb-data pkg-config libasound2-dev libudev-dev libxkbcommon-x11-dev \\\n"
                 "    libwayland-dev libx11-xcb-dev libfontconfig-dev libvulkan-dev libxcursor1 libxi6 libxrandr2 \\\n"
                 "    && rm -rf /var/lib/apt/lists/*\n")
        listed = check.LIST.read_text()
        with tempfile.TemporaryDirectory() as directory:
            containerfile, listing = Path(directory) / "Containerfile", Path(directory) / "packages.txt"

            def outcome(image_text, list_text):
                containerfile.write_text(image_text)
                listing.write_text(list_text)
                with patch.object(sys, "argv", ["check", str(containerfile)]), patch("builtins.print"):
                    try:
                        check.main(listing)
                    except SystemExit as failure:
                        return str(failure)
                return None
            self.assertIsNone(outcome(image, listed))
            for dropped in ("xauth", "x11-xkb-utils", "xkb-data"):
                self.assertIn(f"lacks: {dropped}", outcome(image, listed.replace(f"\n{dropped}\n", "\n")))
            self.assertIn("lacks: extra-package", outcome(image.replace("libxrandr2", "libxrandr2 extra-package"), listed))
            self.assertIn("beyond the image: mesa-vulkan-drivers", outcome(image, listed + "mesa-vulkan-drivers\n"))
            self.assertIn("no apt-get install", outcome("FROM base\n", listed))
