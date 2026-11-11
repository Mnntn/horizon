#!/usr/bin/env python3
"""Pure-move existing snapshot validation out of the api_values hotspot."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/game/modding/api_values.rs"
destination = source.parent / "api_values/block_entity_writeback.rs"
text = source.read_text()
start = text.index("#[derive(Debug, PartialEq, Eq)]\nenum BlockEntitySnapshotError")
end = text.index("impl super::Game<'_> {", start)
moved = text[start:end]
assert "fn finish_block_entity_writeback" in moved
assert not destination.exists()
destination.parent.mkdir(parents=True, exist_ok=True)
destination.write_text(moved)
source.write_text(text[:start] + 'include!("api_values/block_entity_writeback.rs");\n\n' + text[end:])
assert destination.read_text() == moved
