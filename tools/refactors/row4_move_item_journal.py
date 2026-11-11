#!/usr/bin/env python3
"""Move the item-entity journal into an included implementation file unchanged."""
from pathlib import Path
root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/game/runtime/transactions.rs"
destination = source.parent / "transactions/item_entity.rs"
text = source.read_text()
start = text.index("    pub(super) fn journal_set_item_entity(")
while text[text.rfind('\n', 0, start - 1) + 1:start].lstrip().startswith('///'):
    start = text.rfind('\n', 0, start - 1) + 1
end = text.index("    /// `drop_plain`", start)
moved = text[start:end]
assert not destination.exists()
destination.parent.mkdir(parents=True, exist_ok=True)
destination.write_text("impl Game<'_> {\n" + moved + "}\n")
source.write_text(text[:start] + text[end:] + '\ninclude!("transactions/item_entity.rs");\n')
assert moved in destination.read_text()
