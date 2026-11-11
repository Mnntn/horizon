"""Selection is cheap; the opt-in negative proof runs the real prebuilt stage."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

from tools.ci.slot_inputs import applies, inputs

ROOT = Path(__file__).resolve().parents[1]


class StageSlotTests(unittest.TestCase):
    def test_named_sources_trigger_but_unrelated_rust_does_not(self):
        for name in (
            "crates/horizon-client/src/mobs/villager_secondary.rs",
            "crates/horizon-client/src/game/interface/sign_edit.rs",
            "crates/horizon-client/src/game/blocks/machines/mod.rs",
            "crates/horizon-mod-api-gen/slots/portizon.txt",
        ):
            with self.subTest(name=name):
                self.assertTrue(applies(ROOT, [name]))
        self.assertFalse(applies(ROOT, ["crates/horizon-core/src/random.rs"]))
        self.assertIn("crates/horizon-client/src/mobs/villager_secondary.rs", inputs(ROOT))

    @unittest.skipUnless(os.environ.get("HORIZON_STAGE_NEGATIVE") == "1", "real stage opt-in")
    def test_real_stage_refuses_stale_farmland_context(self):
        tree = ROOT / "target/stale-slot-tree"
        shutil.rmtree(tree, ignore_errors=True)
        subprocess.run(["git", "clone", "--quiet", "--shared", str(ROOT), str(tree)], check=True)
        row = tree / "crates/horizon-mod-api-gen/slots/portizon.txt"
        lines = row.read_text().splitlines()
        farmland = next(i for i, line in enumerate(lines) if "::sense_secondary |" in line)
        parts = lines[farmland].split("|")
        parts[-1] = " 0000000000000000"
        lines[farmland] = "|".join(parts)
        row.write_text("\n".join(lines) + "\n")
        # Share only tool artifacts, not staged sources, with the positive proof.
        tools = tree / "target/slot-stage-check/tools"
        tools.parent.mkdir(parents=True)
        cached = ROOT / "target/slot-stage-check/tools"
        cached.mkdir(parents=True, exist_ok=True)
        tools.symlink_to(cached, target_is_directory=True)
        try:
            result = subprocess.run(
                [sys.executable, "tools/ci/stage_slots.py", "--force", "--base", "HEAD"],
                cwd=tree, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                env=dict(os.environ, CARGO_INCREMENTAL="0"),
            )
            self.assertNotEqual(result.returncode, 0, result.stdout[-4000:])
            self.assertIn("E_INJECT_LOST", result.stdout)
            self.assertIn("sense_secondary", result.stdout)
            print("Stale farmland row: real stage exited nonzero with E_INJECT_LOST")
        finally:
            shutil.rmtree(tree)
