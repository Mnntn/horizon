"""Dyna's switch routes only supported, same-repository events to its pool."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BASH = shutil.which("bash")
POSIX_BASH = BASH is not None and not (
    os.name == "nt" and os.path.dirname(os.path.normcase(BASH)).endswith(os.path.normcase(r"\system32")))


class DynaRoutingTests(unittest.TestCase):
    def route(self, event, same_repo="true", enabled="true", lease=None):
        env = dict(os.environ, EVENT=event, SAME_REPO=same_repo, ENABLED=enabled,
                   LEASE=str(int(time.time()) + 3600) if lease is None else lease)
        return subprocess.check_output(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"], env=env, text=True).strip()

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_only_allowed_events_and_same_repository(self):
        for event in ("push", "workflow_dispatch", "pull_request"):
            self.assertEqual(self.route(event), "self_hosted=true", event)
        for event in ("pull_request_target", "workflow_run", "issue_comment", "schedule", ""):
            self.assertEqual(self.route(event), "self_hosted=false", event)
        self.assertEqual(self.route("pull_request", same_repo="false"), "self_hosted=false")

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_switch_and_lease_fail_closed(self):
        self.assertEqual(self.route("push", enabled="false"), "self_hosted=false")
        for lease in ("", "bad", "0"):
            self.assertEqual(self.route("push", lease=lease), "self_hosted=false")

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_live_and_snapshot_lease_routing_on_main(self):
        from test_ci_runners import jobs
        import textwrap
        route = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["changes"]
        script = textwrap.dedent(route.split('        run: |\n          if [ "$SAME_REPO"', 1)[1]
                                .split("      - id: hosted", 1)[0])
        script = 'if [ "$SAME_REPO" ' + script
        script = script.replace("${{ inputs.github-fallback }}", "false")
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            for live, snapshot, expected in (("9999999999", "0", "true"),
                                              ("", "9999999999", "true"),
                                              ("", "1794272194", "false"),
                                              ("0", "9999999999", "false")):
                stub = 'gh() { case "$*" in *SELF_HOSTED*) echo true;; *created_at*) date -u +%FT%TZ;; *) echo "$LIVE";; esac; };\n'
                env = dict(os.environ, EVENT="push", SAME_REPO="true", VARS_ENABLED="true",
                           VARS_LEASE=snapshot, LIVE=live, GITHUB_REPOSITORY="arismoko/horizon",
                           GITHUB_OUTPUT=str(output), GITHUB_RUN_ID="1")
                subprocess.run(["bash", "-e", "-c", stub + script], env=env, cwd=ROOT,
                               capture_output=True, text=True, check=True)
                self.assertIn(f"self_hosted={expected}\n", output.read_text())
                output.unlink()

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_snapshot_lease_is_bounded_by_run_age(self):
        # source, lease, created, now, expected: values independent of wall clock.
        cases = [
            ("live", "1100", "900", "1000", True),
            ("live", "950", "900", "1000", False),
            ("snapshot", "950", "900", "1000", True),
            ("snapshot", "899", "900", "1000", False),
            ("snapshot", "950", "900", "4500", True),
            ("snapshot", "950", "900", "4501", False),
            ("snapshot", "9999", "", "1000", False),
            ("snapshot", "9999", "bad", "1000", False),
            ("snapshot", "9999", "1001", "1000", False),
        ]
        for source, lease, created, now, expected in cases:
            with self.subTest(source=source, lease=lease, created=created, now=now):
                env = dict(os.environ, EVENT="push", SAME_REPO="true", ENABLED="true",
                           LEASE=lease, LEASE_SOURCE=source, CREATED=created, NOW=now)
                result = subprocess.check_output(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"],
                                                 env=env, text=True)
                self.assertEqual(result.strip(), f"self_hosted={str(expected).lower()}")

    @unittest.skipIf(os.name == "nt", "the script installs dependencies on Dyna's Ubuntu runner")
    def test_cached_dependencies_export_persistent_paths(self):
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            prefix = cache / "apt-prefix"
            prefix.mkdir()
            (prefix / ".dyna-hash").write_text(hashlib.sha256(script.read_bytes()).hexdigest() + "\n")
            envfile, pathfile = cache / "env", cache / "path"
            # Dyna sets CARGO_HOME and CARGO_TARGET_DIR on the runner, as in session boxes, so the
            # host's shared compiler cache matches; the script keeps them and only puts Cargo's bin on PATH.
            cargo_home = cache / "dyna-cargo"
            env = dict(os.environ, DYNA_CI_BUILD=directory, GITHUB_ENV=str(envfile), GITHUB_PATH=str(pathfile),
                       CARGO_HOME=str(cargo_home))
            subprocess.run(["bash", script], env=env, check=True)
            exports = envfile.read_text()
            self.assertIn(f"RUSTUP_HOME={cache / 'rustup'}\n", exports)
            for key in ("CARGO_TARGET_DIR", "CARGO_HOME", "CARGO_INCREMENTAL"):
                self.assertNotIn(key, exports)
            self.assertIn(str(cargo_home / "bin"), pathfile.read_text())

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_baked_packages_skip_the_download_and_a_partial_image_falls_back(self):
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        for missing in (False, True):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                stubs = root / "stubs"
                stubs.mkdir()
                (stubs / "dpkg-query").write_text(
                    '#!/bin/bash\nif [ "$MISSING" = 1 ] && [ "${!#}" = libudev-dev ]; then exit 1; fi\nprintf installed\n')
                (stubs / "apt-get").write_text('#!/bin/bash\nprintf "%s\\n" "$*" >> "$APT_CALLS"\n')
                (stubs / "dpkg-deb").write_text("#!/bin/bash\n")
                for stub in stubs.iterdir():
                    stub.chmod(0o755)
                (root / "build").mkdir()
                env = dict(os.environ, PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}", HOME=str(root / "home"),
                           DYNA_CI_BUILD=str(root / "build"), CARGO_HOME=str(root / "cargo"),
                           GITHUB_ENV=str(root / "env"), GITHUB_PATH=str(root / "path"),
                           MISSING=str(int(missing)), APT_CALLS=str(root / "apt-calls"), DYNA_CI_APT_TRIES="1")
                result = subprocess.run(["bash", script], env=env, capture_output=True, text=True)
                calls = (root / "apt-calls").read_text() if (root / "apt-calls").exists() else ""
                if missing:
                    self.assertTrue(calls, "a missing package must run the download fallback")
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(calls, "")
                    self.assertIn("already installed", result.stdout)

    def run_deps_with_stub_apt(self, failures):
        """Run the script with an apt-get that fails `failures` times per command, then succeeds."""
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stubs = root / "stubs"
            stubs.mkdir()
            (stubs / "apt-get").write_text(
                '#!/usr/bin/env bash\n'
                f'n=$(cat "{root}/count" 2>/dev/null || echo 0); echo $((n + 1)) > "{root}/count"\n'
                f'if [ "$n" -lt {failures} ]; then echo "E: File has unexpected size (mirror sync)" >&2; exit 100; fi\n')
            (stubs / "dpkg-deb").write_text("#!/usr/bin/env bash\n")
            # No baked packages: the rootless download fallback is what these cases exercise.
            (stubs / "dpkg-query").write_text("#!/usr/bin/env bash\nexit 1\n")
            for stub in stubs.iterdir():
                stub.chmod(0o755)
            home = root / "home"
            home.mkdir()
            env = dict(os.environ, PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}", HOME=str(home),
                       DYNA_CI_BUILD=str(root / "build"), CARGO_HOME=str(root / "cargo"),
                       GITHUB_ENV=str(root / "env"), GITHUB_PATH=str(root / "path"),
                       DYNA_CI_APT_PAUSE="0", DYNA_CI_APT_TRIES="3")
            (root / "build").mkdir()
            result = subprocess.run(["bash", script], env=env, capture_output=True, text=True)
            count = int((root / "count").read_text()) if (root / "count").exists() else 0
            return result, count

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_apt_is_retried_a_bounded_number_of_times_and_then_succeeds(self):
        # The first command (update) fails twice, then both commands pass: 2 failures + 2 successes.
        result, count = self.run_deps_with_stub_apt(failures=2)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(count, 4)
        self.assertEqual(result.stderr.count("retrying in"), 2)

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_apt_that_keeps_failing_fails_with_the_real_error_after_the_last_try(self):
        result, count = self.run_deps_with_stub_apt(failures=99)
        self.assertEqual(result.returncode, 100)
        self.assertEqual(count, 3)
        self.assertEqual(result.stderr.count("E: File has unexpected size"), 3)
        self.assertIn("apt-get failed after 3 tries (exit 100)", result.stderr)

    def test_lint_enforces_api_boundary_under_central_selection(self):
        from test_ci_runners import jobs
        lint = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["lint"]
        self.assertIn("if: ${{ always() && !cancelled()", lint)
        # Pin the executable contract, including the selection and source-only mode:
        # deleting the step or making it unconditional must fail this test.
        self.assertIn(
            "      - name: Check mod API boundary\n"
            "        if: needs.changes.outputs.rust_required != 'false'\n"
            "        run: python3 tools/ci/check_api_boundary.py --source-only\n", lint)

    def test_workflow_wires_route_and_keeps_privileges_off_pool(self):
        from test_ci_runners import jobs
        workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()
        blocks = dict(jobs(workflow))
        route = blocks["changes"]
        for item in ("vars.DYNA_CI_SELF_HOSTED", "vars.DYNA_CI_POOL_UNTIL", "github.event_name",
                     "github.event.pull_request.head.repo.full_name == github.repository", "dyna_ci_runner.sh"):
            self.assertIn(item, route)
        for name, label in (("verify", "dyna-large"), ("launcher", "dyna-medium")):
            block = blocks[name]
            self.assertIn("needs: changes", block)
            self.assertIn(f"needs.changes.outputs.self_hosted == 'true' && '{label}'", block)
            self.assertNotIn("secrets.", block)
            self.assertNotIn("permissions:", block)
        self.assertIn("runs-on: ubuntu-latest", blocks["pool-watch"])
        self.assertIn("permissions:\n  contents: read", workflow)

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_prepared_data_sentinel_reports_and_rejects_missing_coverage(self):
        from test_ci_runners import jobs
        import textwrap
        workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()
        aggregate = dict(jobs(workflow))["ignored-tests"]
        condition = aggregate.split("if: >-", 1)[1].split("runs-on:", 1)[0]
        self.assertIn("always()", condition)
        self.assertNotIn("self_hosted", condition)
        self.assertIn("github.event_name == 'push'", condition)
        self.assertIn("github.event_name == 'workflow_dispatch'", condition)
        self.assertNotIn("pull_request", condition)
        script = textwrap.dedent(aggregate.split("run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            summary = Path(directory) / "summary"
            for result in ("success", "skipped", "failure", "cancelled", ""):
                for route in ("true", "false"):
                    env = dict(os.environ, RESULT=result, ROUTE=route,
                               REASON="enabled=true; lease=0; live=unreadable",
                               CHANGES="success", FORCED_GITHUB="false",
                               GITHUB_STEP_SUMMARY=str(summary))
                    run = subprocess.run(["bash", "-e", "-c", script], env=env,
                                         capture_output=True, text=True)
                    self.assertEqual(run.returncode, 0 if result == "success" else 1)
                    self.assertIn("live=unreadable", summary.read_text())
                    self.assertIn(f"Shard result: {result}", summary.read_text())
                    summary.unlink()

    def test_tooling_tests_run_only_after_every_earlier_verify_step_succeeded(self):
        # After a failed step the Rust install is skipped, so the Python tests that launch rustc can't run (#2920).
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        step = verify.split("- name: Test offline research tooling\n", 1)[1].split("\n      - ", 1)[0]
        condition = step.split("if:", 1)[1].split("\n", 1)[0]
        self.assertIn("success()", condition)
        self.assertNotIn("cancelled()", condition)
        self.assertNotIn("always()", condition)

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_missing_prepared_data_fails_push_and_nightly_dispatch(self):
        from test_ci_runners import jobs
        import textwrap
        workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()
        shard = dict(jobs(workflow))["ignored-tests-shards"]
        script = textwrap.dedent(shard.split("        run: |\n          for dir", 1)[1]
                                .split("      - name: Generate", 1)[0])
        script = "for dir " + script
        with tempfile.TemporaryDirectory() as directory:
            script = script.replace("/cache/horizon-local", directory + "/missing")
            summary = Path(directory) / "summary"
            for event in ("push", "workflow_dispatch"):
                env = dict(os.environ, EVENT=event, GITHUB_STEP_SUMMARY=str(summary))
                run = subprocess.run(["bash", "-e", "-c", script], env=env,
                                     capture_output=True, text=True)
                self.assertEqual(run.returncode, 1)
                self.assertIn(f"Prepared data unavailable: minecraft ({event})", summary.read_text())
                summary.unlink()

    def test_telemetry_ingest_checks_require_manual_dispatch(self):
        from test_ci_runners import triggers
        workflow = (ROOT / ".github/workflows/telemetry-server.yml").read_text()
        self.assertEqual(triggers(workflow), {"workflow_dispatch"})

    def test_schedule_only_dispatches_and_parity_only_runs_on_dispatch(self):
        from test_ci_runners import jobs, triggers
        workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()
        self.assertEqual(triggers(workflow), {"pull_request", "push", "workflow_dispatch", "schedule"})
        blocks = dict(jobs(workflow))
        dispatcher = blocks["nightly-dispatch"]
        self.assertIn("github.event_name == 'schedule'", dispatcher)
        self.assertIn("runs-on: ubuntu-latest", dispatcher)
        self.assertIn('gh workflow run dyna-ci.yml --repo "$GITHUB_REPOSITORY" --ref main', dispatcher)
        self.assertIn("if: github.event_name != 'schedule'", blocks["changes"])
        parity = blocks["vanilla-parity"]
        self.assertIn("github.event_name == 'workflow_dispatch'", parity)
        self.assertIn("github.repository == 'arismoko/horizon'", parity)
        self.assertIn("runs-on: dyna-large", parity)
        self.assertIn("persist-credentials: false", parity)


if __name__ == "__main__":
    unittest.main()
