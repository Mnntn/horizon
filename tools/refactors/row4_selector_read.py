#!/usr/bin/env python3
"""Give the all-members selector the same thin entry as other scoped reads."""
from pathlib import Path
import subprocess
root = Path(__file__).resolve().parents[2]
relative = "crates/horizon-client/src/game/modding/native_entity_refs.rs"
path = root / relative
base = subprocess.check_output(["git", "show", "686c686e75:" + relative], cwd=root, text=True)
start = base.index("    pub(super) fn level_all_entities(")
end = base.index("\n    ///", start)
body = base[start:end]
raw = body.replace("fn level_all_entities(", "fn unscoped_level_all_entities(", 1)
cold = raw.replace("pub(super) fn unscoped_level_all_entities(", "#[cold]\n    #[inline(never)]\n    fn scoped_level_all_entities(", 1).replace(".map(|id| {", ".filter_map(|id| {")
cold = cold.replace("        ids.into_iter()", "        Some(ids.into_iter()").replace("            .collect()\n    }", "            .collect())\n    }")
destination = path.parent / "native_entity_refs/tick_item_selector.rs"
assert not destination.exists()
destination.write_text("impl Game<'_> {\n" + raw + "\n" + cold + "\n}\n")
wrapper = """    pub(super) fn level_all_entities(&self, level: LevelRef) -> Option<Vec<EntityRef>> {
        if self.block_entity_tick_items.is_some() { return self.scoped_level_all_entities(level); }
        self.unscoped_level_all_entities(level)
    }"""
text = path.read_text()
assert text.count(body) == 1
path.write_text(text.replace(body, wrapper) + '\ninclude!("native_entity_refs/tick_item_selector.rs");\n')
