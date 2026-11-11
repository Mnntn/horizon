#!/usr/bin/env python3
"""Move the dropped-item removal helper out of the Game hotspot unchanged."""
from pathlib import Path
root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/game.rs"
destination = source.parent / "game/modding/dropped_item_removal.rs"
text = source.read_text()
start = text.index("    /// Item entity `i` leaves the level")
end = text.index("    /// Experience orb `i` leaves", start)
moved = text[start:end]
assert not destination.exists()
destination.write_text("use super::Game;\nuse horizon_client::entities::ItemEntity;\n\nimpl Game<'_> {\n" + moved + "}\n")
source.write_text(text[:start] + text[end:])
assert moved in destination.read_text()
