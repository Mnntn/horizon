#!/usr/bin/env python3
"""Pure-move the dropped-item setter before adding its scoped routing."""
from pathlib import Path
root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/game/native_hooks/shared.rs"
destination = root / "crates/horizon-client/src/game/modding/dropped_item_setter.rs"
text = source.read_text()
start = text.index("    pub(super) fn native_set_item_entity(")
# Include the complete preceding documentation block.
while text[text.rfind('\n', 0, start - 1) + 1:start].lstrip().startswith('///'):
    start = text.rfind('\n', 0, start - 1) + 1
end = text.index("    /// Runs `change` on the mod block entity", start)
moved = text[start:end]
assert not destination.exists()
destination.write_text("use super::Game;\nuse horizon_client::items::ItemStack;\n\nimpl Game<'_> {\n" + moved + "}\n")
source.write_text(text[:start] + text[end:])
assert moved in destination.read_text()
