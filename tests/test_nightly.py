import datetime as dt
import unittest
from unittest import mock
from pathlib import Path

from tools.ci import nightly
from tools.ci import nightly_set as ns

NOW = "2026-10-28T12:00:00+00:00"
PATTERNS = ns.guarded_patterns((Path(__file__).resolve().parents[1] / ".github/CODEOWNERS").read_text())


def pr(n, head="h", state="open", files=("crates/horizon-world/src/lib.rs",), base="main"):
    return ns.PullRequest(n, state, f"{head}{n}", f"Fix {n}", f"dev{n}", base, list(files))


def lookup(*prs):
    known = {p.number: p for p in prs}
    return lambda n: known.get(n)


def run(state, text, login="kacpherek", *prs):
    return ns.apply(state, ns.parse(text), ns.Actor(login, 101 if login == "kacpherek" else 9999), NOW,
                    lookup(*prs), PATTERNS)

T = dt.datetime(2026, 10, 28, 12, 0, tzinfo=dt.timezone.utc)


def merged_pr(n, title, labels, issues=(), paths=("crates/horizon-world/src/lib.rs",)):
    return {"number": n, "title": title, "labels": labels, "issues": list(issues), "author": f"dev{n}",
            "paths": list(paths)}


def notes(chosen=None, picked=(), left_out=(), merged=(), issues=()):
    chosen = chosen or nightly.Ref("main", "b" * 40, T)
    return nightly.notes("2026-10-28", chosen, nightly.Ref("main", "m" * 40, T), list(picked), list(left_out),
                         list(merged), list(issues))


class NightlyTests(unittest.TestCase):
    def test_only_old_dated_nightlies_are_pruned(self):
        tags = [{"tagName": "nightly", "isPrerelease": True},
                {"tagName": "nightly-2026-10-20", "isPrerelease": True},
                {"tagName": "nightly-2026-10-21", "isPrerelease": True},
                {"tagName": "nightly-2026-10-19", "isPrerelease": False},
                {"tagName": "v0.2.80", "isPrerelease": False},
                {"tagName": "nightly-2026-10-15-extra", "isPrerelease": True}]
        self.assertEqual(nightly.stale(tags, dt.date(2026, 10, 28), 7), ["nightly-2026-10-20"])

    def test_notes_list_what_reached_main_as_a_release_did(self):
        merged = [merged_pr(1, "Fix pistons", ["feature"], issues=[7]), merged_pr(2, "Water flows", []),
                  merged_pr(3, "CI only", ["skip-release"]), merged_pr(4, "Docs", [], paths=[])]
        text = notes(merged=merged, issues=["- Known thing"])
        self.assertIn("Nightly 2026-10-28 · bbbbbbb: main (mmmmmmm), built from `main`", text)
        self.assertIn("### New\n- Fix pistons (#1, fixes #7)", text)
        self.assertIn("### Fixed\n- Water flows (#2)", text)
        self.assertNotIn("CI only", text)
        self.assertNotIn("Docs", text)
        self.assertIn("### Known issues\n- Known thing", text)
        self.assertIn("Thanks to @dev1, @dev2.", text)
        self.assertNotIn("### Not merged yet", text)
        self.assertIn("Nothing new for players on main", notes())

    def test_notes_open_with_the_way_to_the_launcher(self):
        # A raw HTML line that launchers skip, as on a game release.
        with mock.patch.object(nightly.train, "REPO", "horizon-rust/horizon"):
            self.assertTrue(notes().startswith(nightly.train.PLAYERS + "\n\nNightly 2026-10-28 · bbbbbbb: "))
            self.assertIn(nightly.PLAYERS, notes())

    def test_fork_notes_use_loaded_intro_and_keep_build_details(self):
        with mock.patch.object(nightly.train, "REPO", "arismoko/horizon"):
            # An older selected checkout may not contain the template. It must already be loaded.
            with mock.patch.object(Path, "read_text", side_effect=AssertionError("late template read")):
                text = notes(merged=[merged_pr(9, "Fix fork install", [])], issues=["- Known issue"])
        self.assertTrue(text.startswith(nightly.FORK_PLAYERS + "\n\n"))
        self.assertNotIn(nightly.PLAYERS, text)
        self.assertIn("Nightly 2026-10-28 · bbbbbbb: main (mmmmmmm)", text)
        self.assertIn("nightly-source/2026-10-28-bbbbbbb", text)
        self.assertIn("### Fixed\n- Fix fork install (#9)", text)
        self.assertIn("### Known issues\n- Known issue", text)
        self.assertTrue(text.endswith(nightly.train.DISCLAIMER + "\n"))

    def test_other_repositories_keep_the_upstream_intro(self):
        with mock.patch.object(nightly.train, "REPO", "another/horizon"):
            self.assertTrue(notes().startswith(nightly.train.PLAYERS + "\n\n"))
            self.assertIn(nightly.PLAYERS, notes())

    def test_notes_name_the_commit_and_its_permanent_source_tag(self):
        sha = "0c0206714abc" + "0" * 28
        text = notes(chosen=nightly.Ref("main", sha, None))
        self.assertIn(f"Source: commit {sha}, kept for good as the tag `nightly-source/2026-10-28-0c02067`", text)

    def test_the_source_tag_outlives_every_prune(self):
        tag = nightly.source_tag("2026-10-20", "a" * 40)
        self.assertEqual(nightly.stale([{"tagName": tag, "isPrerelease": True}], dt.date(2026, 11, 28), 7), [])
        self.assertIsNone(nightly.DATED.match(tag))



class ScheduledTests(unittest.TestCase):
    def test_the_scheduled_nightly_drops_merged_closed_and_refused_ones(self):
        state = run({}, "/nightly 1 2 3 4", "kacpherek", pr(1), pr(2), pr(3), pr(4)).state
        get = lookup(pr(1, head="new"), pr(2, state="merged"), pr(3, state="closed"),
                     pr(4, files=[".github/workflows/x.yml"]))
        kept, left_out = nightly.refresh(state["prs"], get, PATTERNS)
        self.assertEqual([(e["number"], e["head"]) for e in kept], [(1, "h1")])
        self.assertEqual(len(left_out), 3)
        self.assertIn("merged", left_out[0])

    def test_the_scheduled_nightly_builds_the_head_that_was_picked(self):
        state = run({}, "/nightly 1", "kacpherek", pr(1)).state
        kept, left_out = nightly.refresh(state["prs"], lookup(pr(1, head="new")), PATTERNS)
        self.assertEqual([(e["number"], e["head"]) for e in kept], [(1, "h1")])
        self.assertEqual(left_out, [])

    def test_an_unreviewed_pull_request_stays_in_the_scheduled_set(self):
        # No review is looked at: a pull request nobody reviewed is kept as long as nothing else refuses it.
        state = run({}, "/nightly 1", "kacpherek", pr(1)).state
        kept, left_out = nightly.refresh(state["prs"], lookup(pr(1)), PATTERNS)
        self.assertEqual([e["number"] for e in kept], [1])
        self.assertEqual(left_out, [])

    def test_notes_list_each_pull_request_and_who_picked_it(self):
        prs = run({}, "/nightly 1 2", "kacpherek", pr(1), pr(2)).state["prs"]
        text = notes(chosen=nightly.Ref("nightly/build-2026-10-28-1", "b" * 40, None), picked=prs,
                     left_out=["#3 Fix 3: closed, left out"])
        self.assertIn("main (mmmmmmm) with 2 pull requests merged in", text)
        self.assertIn("### Not merged yet (2)\n- Fix 1 (#1) by dev1, at h1, picked by kacpherek", text)
        self.assertIn("### Left out (1)", text)

if __name__ == "__main__":
    unittest.main()


WORKFLOWS = Path(__file__).resolve().parents[1] / ".github" / "workflows"


class FirstNightlyTests(unittest.TestCase):
    def test_fetch_succeeds_with_or_without_an_existing_nightly_tag(self):
        class ReachedGreenBase(Exception):
            pass

        for existing in (False, True):
            with self.subTest(existing=existing):
                calls = []
                def git(*args):
                    calls.append(args)
                    if args[0] == "ls-remote":
                        return "abc\\trefs/tags/nightly" if existing else ""
                    return ""
                with mock.patch.object(nightly, "git", side_effect=git), mock.patch.object(nightly, "green_base", side_effect=ReachedGreenBase):
                    with self.assertRaises(ReachedGreenBase):
                        nightly.pick(False)
                fetched = next(args for args in calls if args[0] == "fetch")
                self.assertIn("+refs/heads/main:refs/remotes/origin/main", fetched)
                self.assertEqual("+refs/tags/nightly:refs/tags/nightly" in fetched, existing)


class UntrustedBuildTests(unittest.TestCase):
    """A nightly can hold pull requests nobody reviewed: nothing its build leaves may reach a release (game-build.yml)."""

    def setUp(self):
        self.game = (WORKFLOWS / "game-build.yml").read_text()

    def test_no_nightly_build_is_trusted(self):
        # The train only starts `nightly build` (Nightly only since 2026-10-28); it builds nothing itself.
        release = (WORKFLOWS / "release.yml").read_text()
        self.assertNotIn("game-build.yml", release)
        self.assertIn("gh workflow run nightly-build.yml", release)
        nightly_build = (WORKFLOWS / "nightly-build.yml").read_text()
        self.assertIn("trusted: false", nightly_build)
        self.assertNotIn("trusted: true", nightly_build)
        self.assertIn("default: false", self.game.split("trusted:", 1)[1].split("developer_id:", 1)[0])

    def test_an_untrusted_build_neither_shares_nor_saves_the_release_cache(self):
        self.assertIn("shared-key: ${{ inputs.trusted && 'release' || 'nightly' }}-${{ matrix.name }}", self.game)
        self.assertIn("save-if: ${{ inputs.trusted }}", self.game)
        self.assertEqual(self.game.count("Swatinem/rust-cache"), 1)

    def test_an_untrusted_build_never_runs_on_the_release_runner(self):
        runs_on = [line for line in self.game.splitlines() if "runs-on: ${{" in line]
        self.assertEqual(len(runs_on), 3)
        # The nightly's own runner: never the release runner, and never behind pull requests' parity queue.
        nightly_default = "'[\"self-hosted\", \"macOS\", \"horizon-nightly\"]'"
        for line, variable in zip(runs_on, ("NIGHTLY_EXPORTERS_RUNNER", "NIGHTLY_MAC_RUNNER")):
            self.assertIn("inputs.trusted && toJSON(vars.MAC_RUNNER)", line)
            self.assertIn(f"vars.DYNA_CI_SELF_HOSTED == 'true' && vars.{variable}", line)
            self.assertIn(f"vars.SELF_HOSTED_CI != 'off' && {nightly_default}", line)
        self.assertNotIn("NIGHTLY_LINUX_RUNNER", runs_on[1])
        self.assertIn("vars.NIGHTLY_LINUX_RUNNER", runs_on[2])

    def test_a_trusted_build_never_reads_the_shared_sccache(self):
        calls = self.game.count("bash tools/ci/runner-env.sh")
        self.assertEqual(self.game.count("HORIZON_NO_SCCACHE: ${{ inputs.trusted && '1' || '' }}"), calls)
        env = (Path(__file__).resolve().parents[1] / "tools/ci/runner-env.sh").read_text()
        self.assertEqual(env.count('-z "${HORIZON_NO_SCCACHE:-}"'), 2)


class PruneBuildsTests(unittest.TestCase):
    TODAY = dt.date(2026, 11, 10)

    def test_only_old_unpublished_builds_go(self):
        branches = {
            "nightly/build-2026-10-27-1": "a",   # old: goes
            "nightly/build-2026-10-27-2": "pub",  # old, but a published nightly-<date> points to it: stays
            "nightly/build-2026-11-02-1": "c",   # old enough by one day: goes
            "nightly/build-2026-11-03-1": "d",   # exactly 7 days: stays
            "nightly/build-2026-11-09-3": "e",   # recent: stays
        }
        self.assertEqual(nightly.stale_builds(branches, {"pub"}, self.TODAY, 7),
                         ["nightly/build-2026-10-27-1", "nightly/build-2026-11-02-1"])

    def test_the_newest_build_always_stays(self):
        branches = {"nightly/build-2026-10-01-1": "a", "nightly/build-2026-10-01-2": "b"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), ["nightly/build-2026-10-01-1"])
        self.assertEqual(nightly.stale_builds({"nightly/build-2026-10-01-1": "a"}, set(), self.TODAY, 7), [])
        # the newest by number, not by text: -10 is newer than -9
        branches = {"nightly/build-2026-10-01-9": "a", "nightly/build-2026-10-01-10": "b"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), ["nightly/build-2026-10-01-9"])

    def test_nothing_else_is_touched(self):
        branches = {"nightly-state": "s", "nightly/build-x": "x", "main": "m", "nightly/build-2026-10-01-1-fix": "f",
                    "feature/nightly/build-2026-10-01-1": "g", "nightly/build-2026-10-02-1": "n"}
        self.assertEqual(nightly.stale_builds(branches, set(), self.TODAY, 7), [])

    def test_the_source_is_archived_from_git_signed_attached_and_tagged(self):
        build = (WORKFLOWS / "nightly-build.yml").read_text()
        pick = build.split("\n  build:", 1)[0]
        self.assertIn('git archive --format=tar.gz --prefix="horizon-${SHA:0:7}/" -o horizon-source.tar.gz "$SHA"',
                      pick)
        sign = build.split("\n  sign:", 1)[1].split("\n  publish:", 1)[0]
        self.assertIn("dist/horizon-source.tar.gz", sign.split("sign_release.sh nightly", 1)[1])
        publish = build.split("\n  publish:", 1)[1].split("\n  answer:", 1)[0]
        self.assertIn('"dist/horizon-source.tar.gz#Source of this Nightly', publish)
        self.assertIn('nightly.py keep-source "$SHA" "$DATE"', publish)

    def test_the_workflow_prunes_on_schedule_only(self):
        build = (WORKFLOWS / "nightly-build.yml").read_text()
        step = build.split("Old nightly/build-* branches deleted", 1)[1].split("- uses:", 1)[0]
        self.assertIn("if: github.event_name == 'schedule'", step)
        self.assertIn("prune-builds --days 7", step)
        self.assertIn("GITHUB_STEP_SUMMARY", step)



class SdkPairTests(unittest.TestCase):
    def test_only_complete_published_exact_pair_is_eligible(self):
        assets = [{"name": name} for name in (
            "SHA256SUMS", "horizon-prebuilt-v5-x86_64-unknown-linux-gnu.tar.gz",
            "horizon-prebuilt-v5-x86_64-unknown-linux-gnu-server.tar.gz",
            "horizon-prebuilt-v5-x86_64-pc-windows-gnu.tar.gz",
            "horizon-prebuilt-v5-aarch64-apple-darwin.tar.gz")]
        release = {"draft": False, "tag_name": "prebuilt-" + "a" * 40, "assets": assets}
        for changes, expected in [({}, True), ({"draft": True}, False),
                                  ({"tag_name": "prebuilt-" + "b" * 40}, False)]:
            with mock.patch.object(nightly.train, "gh_json", return_value=[[], [{**release, **changes}]]):
                self.assertEqual(nightly.sdk_ready("a" * 40), expected)
        for index in range(len(assets)):
            with mock.patch.object(nightly.train, "gh_json", return_value=[[{
                    **release, "assets": assets[:index] + assets[index + 1:]}]]):
                self.assertFalse(nightly.sdk_ready("a" * 40))
        with mock.patch.object(nightly.train, "gh_json", return_value=[[]]):
            self.assertFalse(nightly.sdk_ready("a" * 40))
        with mock.patch.object(nightly.train, "gh_json", side_effect=SystemExit("API failed")):
            with self.assertRaises(SystemExit):
                nightly.sdk_ready("a" * 40)

    def test_explicit_missing_sdk_stops_without_selecting_another_base(self):
        with mock.patch.dict(nightly.os.environ, {"NIGHTLY_BASE": "a" * 40}), \
                mock.patch.object(nightly, "is_ancestor", return_value=True), \
                mock.patch.object(nightly, "git", return_value="a" * 40), \
                mock.patch.object(nightly, "sdk_ready", return_value=False), \
                mock.patch.object(nightly.train, "green_main_head") as automatic:
            with self.assertRaises(SystemExit):
                nightly.green_base("arismoko/horizon", "")
            automatic.assert_not_called()

    def test_final_sha_change_or_disappearing_sdk_stops_before_pushing(self):
        for sha, sdk_ready in (("a" * 40, False), ("b" * 40, True)):
            with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "arismoko/horizon"}), \
                    mock.patch.object(nightly.ns, "fetch_heads", return_value=[]), \
                    mock.patch.object(nightly.ns, "assemble", return_value=sha), \
                    mock.patch.object(nightly, "sdk_ready", return_value=sdk_ready), \
                    mock.patch.object(nightly.ns, "push") as push:
                with self.assertRaises(SystemExit):
                    nightly.build_set(nightly.Ref("main", "a" * 40, T), [], [], "2026-11-11", True, False)
                push.assert_not_called()

    def test_automatic_selection_reads_inventory_once_for_all_candidates(self):
        def choose(*, parity, eligible):
            self.assertTrue(parity)
            self.assertFalse(eligible("b" * 40))
            self.assertTrue(eligible("a" * 40))
            return "a" * 40
        with mock.patch.dict(nightly.os.environ, {"NIGHTLY_BASE": ""}), \
                mock.patch.object(nightly, "ready_sdk_shas", return_value={"a" * 40}) as inventory, \
                mock.patch.object(nightly.train, "green_main_head", side_effect=choose), \
                mock.patch.object(nightly, "is_ancestor", return_value=True), \
                mock.patch.object(nightly, "git", side_effect=["a" * 40, NOW]):
            self.assertEqual(nightly.green_base("arismoko/horizon", "").sha, "a" * 40)
            inventory.assert_called_once_with()

    def test_publication_checks_fresh_inventory_and_leaves_upstream_unchanged(self):
        with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "arismoko/horizon"}), \
                mock.patch.object(nightly, "ready_sdk_shas", side_effect=[{"a" * 40}, set()]) as inventory:
            nightly.require_sdk("a" * 40)
            with self.assertRaisesRegex(SystemExit, "publication refused"):
                nightly.require_sdk("a" * 40)
            self.assertEqual(inventory.call_count, 2)
        with mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": "horizon-rust/horizon"}), \
                mock.patch.object(nightly, "sdk_ready") as sdk:
            nightly.require_sdk("a" * 40)
            sdk.assert_not_called()

    def test_fork_pick_refuses_explicit_prs_ignores_saved_prs_and_upstream_keeps_both(self):
        main = nightly.Ref("main", "a" * 40, T)
        saved = [{"number": 17, "head": "b" * 40}]
        explicit = [{"number": 19, "head": "c" * 40}]
        for repo in ("arismoko/horizon", "horizon-rust/horizon"):
            for spec in ("", "19"):
                with self.subTest(repo=repo, spec=spec), \
                        mock.patch.dict(nightly.os.environ, {"GITHUB_REPOSITORY": repo, "NIGHTLY_PRS": spec,
                                                            "GITHUB_EVENT_NAME": "workflow_dispatch", "DRY_RUN": "true"}), \
                        mock.patch.object(nightly, "git", return_value=""), \
                        mock.patch.object(nightly, "green_base", return_value=main), \
                        mock.patch.object(nightly.ns, "load_state", return_value=({"prs": saved}, "", "")) as state, \
                        mock.patch.object(nightly.ns, "lookup"), \
                        mock.patch.object(nightly, "requested", return_value=explicit) as requested, \
                        mock.patch.object(nightly, "build_set") as build:
                    if repo == "arismoko/horizon" and spec:
                        with self.assertRaises(SystemExit):
                            nightly.pick(False)
                        build.assert_not_called()
                        requested.assert_not_called()
                        state.assert_not_called()
                    else:
                        nightly.pick(False)
                        expected = [] if repo == "arismoko/horizon" else explicit if spec else saved
                        self.assertEqual(build.call_args.args[:3], (main, expected, []))
                        if repo == "arismoko/horizon":
                            state.assert_not_called()
                            requested.assert_not_called()
