#!/usr/bin/env python3
"""Real release-build catalog export integration, run by main-data, not unit discovery.

Rust and a clean committed source tree are required. Missing prerequisites fail;
this check must never turn missing integration coverage into a skipped success.
"""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("nightly_catalog", ROOT / "tools/ci/nightly_catalog.py")
CATALOG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CATALOG)


class NightlyCatalogIntegration(unittest.TestCase):
    def test_actual_pinned_export_from_clean_checkout(self):
        commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "mod-catalog.json"
            CATALOG.export(ROOT, commit, output)
            document = json.loads(output.read_text())
            self.assertEqual(document["schema_version"], 1)
            hook = next(hook for hook in document["hooks"] if hook["id"] == "use_item")
            self.assertIn("hand", [argument["name"] for argument in hook["arguments"]])
            self.assertTrue(document["points"])
            self.assertFalse((ROOT / ".horizon-source-staged").exists())


if __name__ == "__main__":
    unittest.main()
