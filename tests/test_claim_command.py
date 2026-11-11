import json
import threading
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from tools.ci import claim_command as cc
from tools.ci import nightly_set as ns

ROOT = Path(__file__).resolve().parents[1]
NOW = datetime(2026, 10, 29, 12, tzinfo=timezone.utc)
# Fixture accounts: the decision is by numeric ID, so each test login has one. Ludovic's real ID may not be filled in
# yet (tools/ci/resolve_reviewer_ids.py), so tests use their own unless it is.
LUDOVIC_ID = ns.OWNER_ID or 1
ns.OWNER_ID = LUDOVIC_ID
cc.ns.OWNER_ID = LUDOVIC_ID  # the script's own import of nightly_set
IDS = {"ludovic111": LUDOVIC_ID, "kacpherek": 101, "rdbtcvs": 102, "levzzz5154": 104}


def actor(login):
    return ns.Actor(login, IDS.get(login.lower(), 9999))


def listing(*lines):
    """A reviewers file: `kacpherek` becomes `101 kacpherek`; other lines stay as written."""
    return ns.read_reviewers("".join((f"{IDS[l.lower()]} {l}" if l.lower() in IDS else l) + "\n" for l in lines))


LIST = listing("kacpherek", "rdbtCVS")


def thread(state="open", holders=(), pr=False, locked=False):
    t = {"state": state, "assignees": [{"login": h} for h in holders], "locked": locked}
    if pr:
        t["pull_request"] = {"merged_at": None}
    return t


class Fake:
    def __init__(self, threads=None, assignable=True, held=()):
        self.repo = "arismoko/horizon"
        self.threads = {5: thread()} if threads is None else threads
        self.assignable, self.calls, self.held = assignable, [], list(held)

    def held_issues(self, login):
        return self.held

    def label(self, n, add, remove):
        self.calls.append(("label", n, add, remove))

    def issue(self, n):
        return self.threads.get(n)

    def assign(self, n, login):
        self.calls.append(("assign", n, login))
        return cc.assignees(self.threads[n]) + ([login] if self.assignable else [])

    def unassign(self, n, login):
        self.calls.append(("unassign", n, login))

    def comment(self, n, text):
        self.calls.append(("comment", n, text))


def do(text, api=None, login="newcomer", reviewers=LIST, n=5, association="NONE", exempt=""):
    api = api or Fake()
    return cc.run(api, cc.parse(text), actor(login), n, reviewers, association, cc.exempt_logins(exempt)), api


def held(number, *prs):
    """An issue the claimant holds; each pr is (state, author, how), how being `closing` or `mention`."""
    closing = [{"state": s, "author": {"login": a}} for s, a, how in prs if how == "closing"]
    mentions = [{"source": {"state": s, "author": {"login": a}}} for s, a, how in prs if how == "mention"]
    return {"number": number, "closedByPullRequestsReferences": {"nodes": closing},
            "timelineItems": {"nodes": mentions}}


class ParseTests(unittest.TestCase):
    def test_commands(self):
        self.assertEqual(cc.parse("/claim"), cc.Command("claim"))
        self.assertEqual(cc.parse("  /CLAIM  \nI'll take it"), cc.Command("claim"))
        self.assertEqual(cc.parse("/unclaim"), cc.Command("unclaim"))
        self.assertEqual(cc.parse("/unclaim @Some-One"), cc.Command("unclaim", "Some-One"))
        self.assertEqual(cc.parse("/unclaim some-one"), cc.Command("unclaim", "some-one"))
        # An agent's comment starts with "🤖 Agent:" (CLAUDE.md); its command follows.
        self.assertEqual(cc.parse("🤖 Agent: /claim"), cc.Command("claim"))
        self.assertEqual(cc.parse("🤖 Agent: /unclaim @Some-One"), cc.Command("unclaim", "Some-One"))

    def test_not_commands(self):
        for text in ("", "claim", "/claimed", "/claims", "/claimed,", "/claims:", "🤖 Agent: /claimed", "thanks\n/claim", "/close", "/unclaimed"):
            self.assertIsNone(cc.parse(text), text)

    def test_claim_ignores_text_after_the_first_word(self):
        for prefix in ("", "🤖 Agent: "):
            for punctuation in ("", ",", ":", ".", "!", ";", "?"):
                for rest in ("", " for stage 1 only: this part", " @other", "\tthis one"):
                    text = f"{prefix}/claim{punctuation}{rest}"
                    with self.subTest(text=text):
                        self.assertEqual(cc.parse(text), cc.Command("claim"))

    def test_unclaim_only_targets_an_exact_login(self):
        for prefix in ("", "🤖 Agent: "):
            for punctuation in ("", ",", ":", "."):
                for rest, who in (("", None), (" @Some-One", "Some-One"), (" some-one", "some-one"),
                                  (" for stage 1 only", None), (" @a b", None), (" @-bad", None),
                                  (" <x>", None), (" @Some-One,", None)):
                    text = f"{prefix}/unclaim{punctuation}{rest}"
                    with self.subTest(text=text):
                        self.assertEqual(cc.parse(text), cc.Command("unclaim", who))


class ClaimTests(unittest.TestCase):
    def test_claim_with_explanation_still_assigns_the_commenter(self):
        for body in ("/claim, for stage 1 only: this part", "🤖 Agent: /claim: @other"):
            with self.subTest(body=body):
                text, api = do(body)
                self.assertIsNone(text)
                self.assertIn(("assign", 5, "newcomer"), api.calls)
                self.assertNotIn(("assign", 5, "other"), api.calls)

    def test_claim_a_free_issue(self):
        text, api = do("/claim")
        self.assertIsNone(text)  # the 👍 says it
        self.assertEqual(api.calls, [("assign", 5, "newcomer"), ("label", 5, "claimed", "help wanted")])

    def test_taken(self):
        with self.assertRaisesRegex(cc.Refused, "@other already claimed"):
            do("/claim", Fake({5: thread(holders=("other",))}))

    def test_claimed_on_discord(self):
        t = thread()
        t["labels"] = [{"name": "In progress"}]
        api = Fake({5: t})
        with self.assertRaisesRegex(cc.Refused, "on Discord"):
            do("/claim", api)
        self.assertEqual(api.calls, [])

    def test_trackers_and_proposals_are_not_claimed(self):
        for name in ("Tracker", "proposal", "not claimable"):
            t = thread()
            t["labels"] = [{"name": "bug"}, {"name": name}]
            api = Fake({5: t})
            with self.assertRaisesRegex(cc.Refused, "isn't claimed as a whole"):
                do("/claim", api)
            self.assertEqual(api.calls, [], name)

    def test_already_yours(self):
        with self.assertRaisesRegex(cc.Refused, "already hold"):
            do("/claim", Fake({5: thread(holders=("NewComer",))}))

    def test_not_claimable(self):
        for t in (thread(pr=True), thread(state="closed"), thread(locked=True)):
            api = Fake({5: t})
            with self.assertRaises(cc.Refused):
                do("/claim", api)
            self.assertEqual(api.calls, [])
        with self.assertRaisesRegex(cc.Refused, "no such"):
            do("/claim", Fake({}))

    def test_github_dropped_the_assignee(self):
        with self.assertRaisesRegex(cc.Refused, "didn't assign"):
            do("/claim", Fake(assignable=False))


class LimitTests(unittest.TestCase):
    def test_under_the_limit_claims(self):
        text, api = do("/claim", Fake(held=[held(n) for n in range(1, 5)]))
        self.assertIsNone(text)
        self.assertIn(("assign", 5, "newcomer"), api.calls)

    def test_at_the_limit_is_refused_naming_the_count(self):
        api = Fake(held=[held(n) for n in (12, 3, 40, 7, 9)])
        with self.assertRaises(cc.Refused) as refused:
            do("/claim", api)
        self.assertEqual(str(refused.exception),
                         "you already hold 5 claimed issues with no open pull request of yours (#3, #7, #9, #12, #40), "
                         "and the limit is 5. Opening a pull request that links one of them, or `/unclaim` on one, "
                         "frees a slot.")
        self.assertEqual(api.calls, [])

    def test_over_the_limit_is_refused_and_nothing_is_released(self):
        api = Fake(held=[held(n) for n in range(1, 8)])
        with self.assertRaisesRegex(cc.Refused, "hold 7 claimed issues"):
            do("/claim", api)
        self.assertEqual(api.calls, [])

    def test_issues_with_an_open_pull_request_of_theirs_do_not_count(self):
        mine = [held(1, ("OPEN", "NewComer", "closing")), held(2, ("OPEN", "newcomer", "mention"))]
        text, api = do("/claim", Fake(held=mine + [held(n) for n in range(3, 7)]))
        self.assertIsNone(text)
        self.assertIn(("assign", 5, "newcomer"), api.calls)

    def test_closed_or_someone_elses_pull_requests_still_count(self):
        others = [held(1, ("OPEN", "someone", "closing")), held(2, ("CLOSED", "newcomer", "mention")),
                  held(3, ("MERGED", "newcomer", "closing"))]
        with self.assertRaisesRegex(cc.Refused, "hold 5 claimed"):
            do("/claim", Fake(held=others + [held(4), held(6)]))

    def test_the_repositorys_people_and_trusted_reviewers_are_not_limited(self):
        full = [held(n) for n in range(10, 20)]
        for association in ("OWNER", "MEMBER", "COLLABORATOR"):
            text, api = do("/claim", Fake(held=full), association=association)
            self.assertIn(("assign", 5, "newcomer"), api.calls, association)
        text, api = do("/claim", Fake(held=full), login="kacpherek")
        self.assertIn(("assign", 5, "kacpherek"), api.calls)
        with self.assertRaises(cc.Refused):
            do("/claim", Fake(held=full), association="CONTRIBUTOR")

    def test_an_exempt_login_over_the_limit_claims(self):
        full = [held(n) for n in range(10, 20)]
        for listing in ("NewComer", " other , newcomer ", "@NEWCOMER,other"):
            text, api = do("/claim", Fake(held=full), exempt=listing)
            self.assertIn(("assign", 5, "newcomer"), api.calls, listing)

    def test_an_empty_or_unrelated_exempt_list_exempts_nobody(self):
        full = [held(n) for n in range(10, 20)]
        for listing in ("", " ", ",", "other", "newcomer2"):
            with self.assertRaises(cc.Refused, msg=listing):
                do("/claim", Fake(held=full), exempt=listing)

    def test_pull_requests_are_matched_to_the_claimant(self):
        issues = [held(1, ("OPEN", "a", "closing")), held(2, ("OPEN", "b", "mention")), held(3)]
        self.assertEqual(cc.without_pull_request(issues, "A"), [2, 3])


class Shared:
    """One claimant's assignments, seen by several runs at once; each run's first count waits at a barrier, so every
    run has counted before any assigns, as two workflow runs for different issues can."""

    def __init__(self, held, runs):
        self.held, self.lock, self.barrier = set(held), threading.Lock(), threading.Barrier(runs)
        self.waited = set()

    def api(self, number):
        shared = self

        class Run(Fake):
            def held_issues(self, login):
                with shared.lock:
                    now = [held(n) for n in sorted(shared.held)]
                if threading.get_ident() not in shared.waited:
                    shared.waited.add(threading.get_ident())
                    shared.barrier.wait()
                return now

            def assign(self, n, login):
                with shared.lock:
                    shared.held.add(n)
                return [login]

            def unassign(self, n, login):
                with shared.lock:
                    shared.held.discard(n)

        return Run({number: thread()})


class ConcurrentClaimTests(unittest.TestCase):
    def test_claims_made_at_once_never_pass_the_limit(self):
        shared = Shared(range(1, 5), runs=2)
        outcomes = {}

        def claim(number):
            try:
                outcomes[number] = do("/claim", shared.api(number), n=number)[0]
            except cc.Refused as refused:
                outcomes[number] = str(refused)

        runs = [threading.Thread(target=claim, args=(n,)) for n in (20, 21)]
        for run in runs:
            run.start()
        for run in runs:
            run.join(5)
        self.assertLessEqual(len(shared.held), cc.LIMIT)
        refused = [n for n, text in outcomes.items() if text is not None]
        self.assertTrue(refused)
        for n in refused:
            self.assertNotIn(n, shared.held)
            self.assertIn("at the same moment", outcomes[n])

    def test_the_recount_backs_out_only_past_the_limit(self):
        class Racing(Fake):
            """4 held at the first count; another claim lands before the second."""
            counts = iter([[held(n) for n in range(1, 5)], [held(n) for n in range(1, 7)]])

            def held_issues(self, login):
                return next(self.counts)

        with self.assertRaisesRegex(cc.Refused, "at the same moment"):
            do("/claim", Racing())
        text, api = do("/claim", Fake(held=[held(n) for n in range(1, 5)]))  # the recount sees 4 + this one: kept
        self.assertIsNone(text)


    def test_a_failed_recount_undoes_the_claim(self):
        class Failing(Fake):
            first = True

            def held_issues(self, login):
                if self.first:
                    self.first = False
                    return []
                raise RuntimeError("gh api graphql: rate limited")

        api = Failing()
        with self.assertRaises(RuntimeError):
            do("/claim", api)
        self.assertEqual(api.calls, [("assign", 5, "newcomer"), ("unassign", 5, "newcomer")])


class PaginationTests(unittest.TestCase):
    def test_every_page_of_an_issues_links_is_read(self):
        def page(nodes, more):
            return {"pageInfo": {"hasNextPage": more, "endCursor": "c" if more else None}, "nodes": nodes}

        mine = {"state": "OPEN", "author": {"login": "newcomer"}}
        other = {"state": "OPEN", "author": {"login": "someone"}}
        answers = [
            {"issues": page([{"number": 1, "closedByPullRequestsReferences": page([other] * 50, True),
                              "timelineItems": page([{"source": other}] * 100, True)}], False)},
            {"issue": {"closedByPullRequestsReferences": page([mine], False)}},
            {"issue": {"timelineItems": page([{"source": other}], False)}},
        ]
        sent = []

        def gh(*args, **_):
            sent.append(args)
            return json.dumps({"data": {"repository": answers.pop(0)}})

        real, cc.gh = cc.gh, gh
        try:
            issues = cc.Api("arismoko/horizon").held_issues("newcomer")
        finally:
            cc.gh = real
        self.assertEqual(answers, [])
        self.assertEqual(len(issues[0]["closedByPullRequestsReferences"]["nodes"]), 51)
        self.assertEqual(len(issues[0]["timelineItems"]["nodes"]), 101)
        self.assertIn("wantRefs=true", sent[1])
        self.assertIn("refs=c", sent[1])
        self.assertIn("events=c", sent[2])
        self.assertEqual(cc.without_pull_request(issues, "newcomer"), [])


class UnclaimTests(unittest.TestCase):
    def test_explanatory_text_releases_only_the_commenter(self):
        for body in ("/unclaim, for stage 1 only", "🤖 Agent: /unclaim: @other and me"):
            with self.subTest(body=body):
                text, api = do(body, Fake({5: thread(holders=("NewComer", "other"))}))
                self.assertIsNone(text)
                self.assertEqual(api.calls, [("unassign", 5, "NewComer")])

    def test_own(self):
        text, api = do("/unclaim", Fake({5: thread(holders=("NewComer",))}))
        self.assertIsNone(text)
        self.assertEqual(api.calls, [("unassign", 5, "NewComer"), ("label", 5, "help wanted", "claimed")])

    def test_a_remaining_holder_keeps_the_claimed_label(self):
        text, api = do("/unclaim", Fake({5: thread(holders=("NewComer", "other"))}))
        self.assertEqual(api.calls, [("unassign", 5, "NewComer")])

    def test_not_held(self):
        with self.assertRaisesRegex(cc.Refused, "don't hold"):
            do("/unclaim", Fake({5: thread(holders=("other",))}))

    def test_someone_elses_needs_the_team(self):
        api = Fake({5: thread(holders=("Other",))})
        with self.assertRaisesRegex(cc.Refused, "trusted reviewers"):
            do("/unclaim @other", api)
        self.assertEqual(api.calls, [])
        text, api = do("/unclaim @other", Fake({5: thread(holders=("Other",))}), login="kacpherek")
        self.assertIn("released @Other's claim, on behalf of @kacpherek", text)
        self.assertEqual(api.calls, [("unassign", 5, "Other"), ("label", 5, "help wanted", "claimed")])

    def test_repository_owner_releases_another_claim(self):
        text, api = do("/unclaim @other", Fake({5: thread(holders=("Other",))}), login="Arismoko")
        self.assertIn("released @Other's claim", text)
        self.assertEqual(api.calls, [("unassign", 5, "Other"), ("label", 5, "help wanted", "claimed")])

    def test_someone_who_holds_nothing(self):
        with self.assertRaisesRegex(cc.Refused, "`ghost` doesn't hold"):
            do("/unclaim @ghost", login="ludovic111")


def node(number=7, holders=("dev",), events=(), closing=()):
    return {"number": number, "assignees": {"nodes": [{"login": h} for h in holders]},
            "closedByPullRequestsReferences": {"nodes": [{"number": c} for c in closing]},
            "timelineItems": {"nodes": list(events)}}


def assigned(login, days_ago, actor="github-actions"):
    when = (NOW - timedelta(days=days_ago)).strftime("%Y-%m-%dT%H:%M:%SZ")
    return {"__typename": "AssignedEvent", "createdAt": when, "actor": {"login": actor}, "assignee": {"login": login}}


def mentioned(state):
    return {"__typename": "CrossReferencedEvent", "source": {"number": 9, "state": state}}


class ReleaseTests(unittest.TestCase):
    def test_stale_claim_released(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 6)]), NOW), ["dev"])

    def test_fresh_claim_kept(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 4)]), NOW), [])

    def test_newest_assignment_counts(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), assigned("dev", 1)]), NOW), [])

    def test_hand_assignments_kept(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 30, actor="kacpherek")]), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[]), NOW), [])

    def test_an_open_pull_request_keeps_it(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), mentioned("OPEN")]), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9)], closing=(9,)), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), mentioned("CLOSED")]), NOW), ["dev"])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), {"__typename": "CrossReferencedEvent",
                                                                          "source": {}}]), NOW), ["dev"])

    def test_release_unassigns_and_says_why(self):
        class Api(Fake):
            def assigned_issues(self):
                return [node(7, ("a", "b"), [assigned("a", 6), assigned("b", 2)]), node(8, ("c",), [assigned("c", 8)])]

        api = Api()
        self.assertEqual(cc.release(api, NOW, dry_run=True), [(7, "a"), (8, "c")])
        self.assertEqual(api.calls, [])
        cc.release(api, NOW, dry_run=False)
        # #8 lost its only holder, so it is offered again; #7 keeps b.
        self.assertEqual([c for c in api.calls if c[0] == "label"], [("label", 8, "help wanted", "claimed")])
        self.assertEqual([c[:3] for c in api.calls if c[0] == "unassign"], [("unassign", 7, "a"), ("unassign", 8, "c")])
        comments = [c[2] for c in api.calls if c[0] == "comment"]
        self.assertEqual(len(comments), 2)
        self.assertTrue(comments[0].startswith(cc.PREFIX + "released @a's claim: 5 days"))


class WorkflowTests(unittest.TestCase):
    def test_workflow_is_safe(self):
        text = (ROOT / ".github/workflows/claim-command.yml").read_text()
        run_lines = [l.strip() for l in text.splitlines() if l.strip().startswith("run:")]
        self.assertEqual(run_lines, ["run: python3 tools/ci/claim_command.py command",
                                     "run: python3 tools/ci/claim_command.py release"])
        self.assertEqual(text.count("persist-credentials: false"), 2)
        self.assertEqual(text.count("ref: main"), 2)
        self.assertNotIn("pull_request_target", text)
        self.assertNotIn("contents: write", text)
        self.assertNotIn("pull-requests: write", text)
        for line in text.splitlines():
            if "github.event.comment" in line and "run:" in line:
                self.fail(f"comment data in a shell line: {line}")


if __name__ == "__main__":
    unittest.main()
