import importlib.util
import tempfile
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "native_probe_ci", Path(__file__).resolve().parents[1] / "tools/native_probe_ci.py")
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class SelectionTests(unittest.TestCase):
    def test_sdk_graph_and_boundaries(self):
        root = Path(__file__).resolve().parents[1]
        roots = ci.sdk_roots(root)
        for path in ("Cargo.lock", "Cargo.toml", "crates/new-crate/Cargo.toml",
                     "crates/horizon-mod-api-gen/src/main.rs", "crates/horizon-client/src/game.rs",
                     "crates/cargo-horizon-mod/template/native/lib.rs", "tools/native_dylib_probe/a.rs",
                     "launcher/src/rust_toolchain.rs", "rust-toolchain.toml",
                     "packaging/windows/exe.rs", "packaging/executable_link.rs"):
            with self.subTest(path=path):
                self.assertTrue(ci.selected([path], roots))
        for path in ("launcher/src/main.rs", "launcher/src/native_mods/plan.rs", "docs/native.md",
                     "tests/test_other.py", "crates/horizon-client/tests/other.rs",
                     "crates/horizon-client/examples/other.rs", "crates/horizon-client/README.md"):
            with self.subTest(path=path):
                self.assertFalse(ci.selected([path], roots))

    def test_discovers_transitive_build_target_dependencies_not_dev(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text('[workspace.dependencies]\nd = {path="crates/d"}\n')
            for name, manifest in {
                "horizon-mod-host": '[build-dependencies]\nb = {path="../b"}\n',
                "horizon-mod-api-gen": '',
                "b": '[target."cfg(windows)".dependencies]\nc = {path="../c"}\n[dev-dependencies]\nmissing = {path="../missing"}\n',
                "c": '[dependencies]\nd.workspace = true\n',
                "d": '',
            }.items():
                folder = root / "crates" / name
                folder.mkdir(parents=True)
                (folder / "Cargo.toml").write_text(manifest)
            self.assertIn("crates/c/", ci.sdk_roots(root))
            self.assertIn("crates/d/", ci.sdk_roots(root))


class SummaryTests(unittest.TestCase):
    def test_baseline_filters(self):
        good = dict(id=1, run_number=1, event="push", head_branch="main",
                    head_repository={"full_name": "fork/repo"}, conclusion="success")
        runs = [good]
        for patch in ({"id": 9, "run_number": 9}, {"event": "pull_request"},
                      {"head_branch": "topic"}, {"head_repository": {"full_name": "upstream/repo"}},
                      {"conclusion": "failure"}, {"head_repository": None}):
            runs.append({**good, "id": 2, "run_number": 2, **patch})
        self.assertEqual(ci.baseline(runs, dict(id=9, run_number=9, repository="fork/repo")), good)
        self.assertIsNone(ci.baseline([], dict(id=9, run_number=9, repository="fork/repo")))

    def test_no_baseline_and_only_fork_merge_prs(self):
        self.assertIn("No successful", ci.summary("fork/repo", "head", None, [], lambda _: []))
        def pr(repo, number, sha="commit"):
            return dict(number=number, merge_commit_sha=sha, merged_at="date",
                        base={"ref": "main", "repo": {"full_name": repo}}, title="$(untrusted)")
        report = ci.summary("fork/repo", "head", dict(html_url="green", head_sha="base"),
                            [{"sha": "commit"}], lambda _: [pr("fork/repo", 42), pr("upstream/repo", 99),
                                                           pr("fork/repo", 43, "other")])
        self.assertIn("fork/repo/pull/42", report)
        self.assertNotIn("pull/99", report)
        self.assertNotIn("pull/43", report)
        self.assertNotIn("untrusted", report)
        self.assertIn("base...head", report)


class RecoveryTests(unittest.TestCase):
    def test_coalesced_sdk_push_then_readme_still_selects(self):
        root = Path(__file__).resolve().parents[1]
        # A was last green; B changed SDK and is replaced in the queue by C.
        changes = {"A": ["crates/horizon-client/src/game.rs", "crates/horizon-client/README.md"],
                   "B": ["crates/horizon-client/README.md"]}
        self.assertFalse(ci.selected(changes["B"], ci.sdk_roots(root)))
        self.assertTrue(ci.select_interval(root, {"head_sha": "A"}, changes.__getitem__))
        self.assertTrue(ci.select_interval(root, None, lambda _: self.fail("must not diff unknown baseline")))

    def test_history_ceiling_is_not_first_run(self):
        current = dict(id=20, run_number=20, repository="fork/repo")
        self.assertIsNone(ci.find_green(lambda _: {"total_count": 0, "workflow_runs": []}, None, current))
        with self.assertRaisesRegex(RuntimeError, "baseline unavailable"):
            ci.find_green(lambda _: {"total_count": 1001, "workflow_runs": []}, None, current)

    def test_known_range_survives_compare_or_pr_failure(self):
        green = dict(head_sha="A", html_url="green-url")
        for fail_compare in (True, False):
            output = []
            def pages(path, key=None):
                # Range must already be durable before the first request.
                self.assertIn("A...C", output[0])
                self.assertIn("green-url", output[0])
                if fail_compare or path.startswith("commits/"):
                    raise OSError("network unavailable")
                return [{"sha": "B"}]
            ci.write_failure("fork/repo", "C", green, pages, output.append)
            self.assertIn("lookup failed", output[-1])
            self.assertNotIn("No successful", "\n".join(output))


class InputBoundaryTests(unittest.TestCase):
    def test_packaging_inputs_reach_both_selection_layers(self):
        root = Path(__file__).resolve().parents[1]
        workflow = (root / ".github/workflows/native-dylib-probe.yml").read_text()
        push_filter = workflow.split("permissions:", 1)[0]
        for path in ("packaging/windows/exe.rs", "packaging/executable_link.rs"):
            with self.subTest(path=path):
                self.assertIn("- '" + path + "'", push_filter)
                self.assertTrue(ci.selected([path], ci.sdk_roots(root)))

    def test_rename_to_docs_preserves_deleted_sdk_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            real_run = subprocess.run
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, text=True).strip()
            git("init", "-q")
            source = root / "crates/sdk/src/lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text("pub fn sdk() {}\n")
            git("add", ".")
            git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "SDK")
            base = git("rev-parse", "HEAD")
            (root / "docs").mkdir()
            git("mv", "crates/sdk/src/lib.rs", "docs/moved.rs")
            git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "Move")
            with patch.object(ci.subprocess, "run", side_effect=lambda *a, **kw: real_run(*a, cwd=root, **kw)):
                paths = ci.changed_paths(base, lambda _: self.fail("diff should succeed"))
            self.assertIn("crates/sdk/src/lib.rs", paths)
            self.assertIn("docs/moved.rs", paths)
            self.assertTrue(ci.selected(paths, ["crates/sdk/"]))

    def test_api_timeout_conservatively_proves(self):
        with tempfile.TemporaryDirectory() as tmp:
            output, summary = Path(tmp) / "output", Path(tmp) / "summary"
            env = dict(PROBE_SELECT="1", GITHUB_REPOSITORY="fork/repo", GH_TOKEN="test",
                       GITHUB_RUN_ID="1", GITHUB_OUTPUT=str(output), GITHUB_STEP_SUMMARY=str(summary))
            with patch.dict(ci.os.environ, env), patch.object(ci.urllib.request, "urlopen", side_effect=TimeoutError) as request:
                ci.main()
            self.assertEqual(request.call_args.kwargs["timeout"], 30)
            self.assertEqual(output.read_text(), "selected=true\n")
            self.assertIn("Baseline history unavailable", summary.read_text())


class BaselineDiscoveryTests(unittest.TestCase):
    def run_record(self, number):
        return dict(id=number, run_number=number, event="push", head_branch="main",
                    head_repository={"full_name": "fork/repo"}, conclusion="success", head_sha=str(number))

    def discover(self, batches, jobs):
        current = dict(id=200, run_number=200, repository="fork/repo")
        def api(path):
            page = int(path.rsplit("page=", 1)[1])
            return dict(total_count=sum(map(len, batches)), workflow_runs=batches[page - 1].copy())
        def pages(path, key):
            self.assertEqual(key, "jobs")
            number = int(path.split("/")[2])
            return [dict(name="linkage " + str(i), conclusion=status)
                    for i, status in enumerate(jobs[number])]
        return ci.find_green(api, pages, current)

    def test_noop_above_proof_and_coalesced_selection(self):
        a, b = self.run_record(1), self.run_record(2)
        green = self.discover([[b, a]], {1: ["success", "success"], 2: []})
        self.assertEqual(green, a)
        root = Path(__file__).resolve().parents[1]
        changes = {"1": ["crates/horizon-client/src/game.rs"], "2": ["crates/horizon-client/README.md"]}
        self.assertTrue(ci.select_interval(root, green, changes.__getitem__))

    def test_one_successful_leg_is_not_green(self):
        a, b = self.run_record(1), self.run_record(2)
        for other in ("failure", "skipped"):
            with self.subTest(other=other):
                self.assertEqual(self.discover([[b, a]], {1: ["success", "success"],
                                                       2: ["success", other]}), a)

    def test_baseline_on_second_page(self):
        a = self.run_record(1)
        noops = [self.run_record(n) for n in range(101, 1, -1)]
        jobs = {r["id"]: [] for r in noops}
        jobs[1] = ["success", "success"]
        self.assertEqual(self.discover([noops, [a]], jobs), a)

    def test_current_and_newer_proofs_are_excluded(self):
        a = self.run_record(1)
        self.assertEqual(self.discover([[self.run_record(201), self.run_record(200), a]],
                                       {1: ["success", "success"]}), a)

    def test_missing_baseline_sha_conservatively_proves(self):
        output = []
        with patch.object(ci.subprocess, "run", return_value=ci.subprocess.CompletedProcess([], 128, "", "bad revision")):
            root = Path(__file__).resolve().parents[1]
            self.assertTrue(ci.select_interval(root, {"head_sha": "gone"},
                                               lambda base: ci.changed_paths(base, output.append)))
        self.assertIn("conservatively proving", output[0])


if __name__ == "__main__":
    unittest.main()
