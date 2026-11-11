"""Keep the real catalog export out of unit discovery and fail closed in data."""
import importlib.util
from pathlib import Path
import re
import subprocess
import unittest

from test_ci_concurrency import yaml

ROOT = Path(__file__).resolve().parents[1]


class NightlyCatalogRouting(unittest.TestCase):
    def test_real_export_is_explicit_integration_not_a_discovered_unit_test(self):
        loader = unittest.TestLoader()
        discovered = loader.discover(str(ROOT / "tests"))
        self.assertEqual(loader.errors, [])
        pending = [discovered]
        identifiers = []
        while pending:
            case = pending.pop()
            if isinstance(case, unittest.TestSuite):
                pending.extend(case)
            else:
                identifiers.append(case.id())
        self.assertFalse(any(name.endswith(".test_actual_pinned_export_from_clean_checkout")
                             for name in identifiers))
        spec = importlib.util.spec_from_file_location(
            "nightly_catalog_integration", ROOT / "tools/ci/nightly_catalog_integration.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertEqual(unittest.TestLoader().getTestCaseNames(module.NightlyCatalogIntegration),
                         ["test_actual_pinned_export_from_clean_checkout"])

    def test_one_existing_shard_runs_export_and_failure_cannot_hide_item_coverage(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/main-data.yml").read_text())
        steps = workflow["jobs"]["ignored-tests-shards"]["steps"]
        integration = next(step for step in steps if step.get("id") == "catalog")
        self.assertEqual(integration["run"], "python3 tools/ci/nightly_catalog_integration.py -v")
        self.assertIs(integration["continue-on-error"], True)
        preparation = next(step for step in steps if step.get("id") == "data")
        self.assertLess(steps.index(integration), steps.index(preparation))
        propagate = next(step for step in steps if step.get("name") == "Require successful Nightly catalog integration")
        self.assertLess(steps.index(preparation), steps.index(propagate))
        for shard in ("horizon-items", "horizon-world", "horizon-client#1of2"):
            for outcome in ("success", "failure", "skipped"):
                for cancelled in (False, True):
                    values = {"matrix.shard": shard, "steps.catalog.outcome": outcome}
                    condition = re.sub(r"matrix\.shard|steps\.catalog\.outcome",
                                       lambda match: repr(values[match[0]]), propagate["if"])
                    condition = condition.replace("always()", "True").replace("cancelled()", repr(cancelled))
                    condition = condition.replace("&&", " and ")
                    condition = re.sub(r"!(?!=)", " not ", condition).strip()
                    self.assertEqual(eval(condition, {"__builtins__": {}}),
                                     not cancelled and shard == "horizon-items" and outcome != "success")
            predicate = integration["if"].replace("matrix.shard", repr(shard))
            self.assertEqual(eval(predicate, {"__builtins__": {}}), shard == "horizon-items")
        failed = subprocess.run(["bash", "-e", "-c", propagate["run"]], capture_output=True, text=True)
        self.assertEqual(failed.returncode, 1)
        self.assertIn("Nightly catalog integration did not succeed", failed.stdout)


if __name__ == "__main__":
    unittest.main()
