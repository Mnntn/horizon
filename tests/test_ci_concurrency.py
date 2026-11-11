"""Pin the actual parsed workflow's main queue and PR replacement behavior."""
import ast
from pathlib import Path
import re
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
# Use the same offline parser as the repository guard, not a runner package.
sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
import yaml

WORKFLOW = ROOT / ".github/workflows/dyna-ci.yml"


def expression_value(expression, github):
    """Evaluate this workflow's context/equality/boolean subset; reject other syntax.

    These contexts use nonzero PR numbers, lowercase event/ref strings and nonempty
    SHAs, whose truthiness/equality matches Actions. Boolean operators return their
    operands, so the group's fallback chain is exercised rather than pinned as text.
    """
    def evaluate(node):
        if isinstance(node, ast.Constant):
            return node.value
        if isinstance(node, ast.Name):
            return {"github": github, "true": True, "false": False}[node.id]
        if isinstance(node, ast.Attribute):
            return evaluate(node.value).get(node.attr, "")
        if isinstance(node, ast.Compare) and len(node.ops) == 1 and isinstance(node.ops[0], ast.Eq):
            return evaluate(node.left) == evaluate(node.comparators[0])
        if isinstance(node, ast.BoolOp):
            for value in node.values:
                result = evaluate(value)
                if isinstance(node.op, ast.Or) and result:
                    return result
                if isinstance(node.op, ast.And) and not result:
                    return result
            return result
        raise AssertionError(f"unsupported Actions expression syntax: {ast.dump(node)}")

    return evaluate(ast.parse(expression.replace("&&", " and ").replace("||", " or ").strip(), mode="eval").body)


def concurrency_for(event, sha, number=None, filename="dyna-ci.yml"):
    workflow_path = WORKFLOW.with_name(filename)
    workflow = yaml.safe_load(workflow_path.read_text())
    policy = workflow["concurrency"]
    github = {"event_name": event, "sha": sha, "ref": "refs/heads/main",
              "event": {"pull_request": {"number": number or ""}}}
    if number is not None:
        github["ref"] = f"refs/pull/{number}/merge"
    group = re.sub(r"\$\{\{(.*?)\}\}",
                   lambda match: str(expression_value(match.group(1), github)), policy["group"])
    cancel = policy["cancel-in-progress"]
    if isinstance(cancel, str):
        match = re.fullmatch(r"\$\{\{(.*?)\}\}", cancel)
        if match is None:
            raise AssertionError("cancellation must be a boolean or an Actions expression")
        cancel = expression_value(match.group(1), github)
    if not isinstance(cancel, bool):
        raise AssertionError("cancellation did not evaluate to a boolean")
    return group, cancel


class ConcurrencyTests(unittest.TestCase):
    def test_gate_and_fixture_event_sha_namespaces(self):
        groups = set()
        for filename in ("dyna-ci.yml", "wasm-mods.yml"):
            for event in ("push", "workflow_dispatch", "schedule", "pull_request"):
                group, cancel = concurrency_for(event, "a" * 40, filename=filename)
                self.assertNotIn(group, groups)
                groups.add(group)
                self.assertEqual(cancel, event == "pull_request")
            first = concurrency_for("push", "a" * 40, filename=filename)
            self.assertEqual(first, concurrency_for("push", "a" * 40, filename=filename))
            self.assertNotEqual(first[0], concurrency_for("push", "b" * 40, filename=filename)[0])

    def test_data_replaces_pending_only_and_never_gate(self):
        # Model Actions' one-active/one-pending semantics using the parsed groups.
        active, pending = {}, {}
        for sha in ("A", "B", "C"):
            for filename in ("dyna-ci.yml", "wasm-mods.yml", "main-data.yml"):
                group, cancel = concurrency_for("push", sha, filename=filename)
                self.assertFalse(cancel)
                if group not in active:
                    active[group] = sha
                else:
                    pending[group] = sha
        data_group = concurrency_for("push", "A", filename="main-data.yml")[0]
        self.assertEqual(active[data_group], "A")
        self.assertEqual(pending[data_group], "C")
        for filename in ("dyna-ci.yml", "wasm-mods.yml"):
            for sha in ("A", "B", "C"):
                group = concurrency_for("push", sha, filename=filename)[0]
                self.assertEqual(active[group], sha)
                self.assertNotIn(group, pending)
        self.assertEqual(pending, {data_group: "C"})
        for event in ("push", "workflow_dispatch"):
            self.assertEqual(concurrency_for(event, "different", filename="main-data.yml"),
                             (data_group, False))
        self.assertNotEqual(concurrency_for("schedule", "C", filename="main-data.yml")[0], data_group)

    def test_pr_updates_cancel_only_the_same_pr_group(self):
        first = concurrency_for("pull_request", "a" * 40, 101)
        update = concurrency_for("pull_request", "b" * 40, 101)
        other_pr = concurrency_for("pull_request", "c" * 40, 102)
        main = concurrency_for("push", "d" * 40)
        self.assertEqual(first[0], update[0], "updates must replace checks for the same PR")
        self.assertNotEqual(first[0], other_pr[0], "one PR must not cancel another PR")
        self.assertNotEqual(first[0], main[0], "a PR must not cancel main")
        for _, cancel in (first, update, other_pr):
            self.assertIs(cancel, True, "PR events must still cancel their previous checks")


if __name__ == "__main__":
    unittest.main()
