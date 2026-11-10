#!/usr/bin/env python3
"""Move the UUID repair's new Game methods verbatim into persist/object_memory.rs."""
from pathlib import Path
import hashlib
import re

root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/game/world/persist.rs"
target = source.parent / "persist/object_memory.rs"
names = {
    "mob_at", "mob_at_mut", "mob_object", "mob_object_mut", "mob_objects",
    "load_item_transferred", "load_orb_transferred", "load_arrow_transferred",
    "load_spit_or_bullet_transferred", "load_fireball_transferred",
    "load_throwable_transferred", "load_falling_transferred", "load_trident_transferred",
    "load_rocket_transferred", "load_tnt_transferred", "load_boat_transferred",
    "load_cart_transferred", "mob_uuid_admission_refused", "load_mob_transferred",
    "take_mob_passengers", "mob_passenger_objects", "mob_passenger_ids",
    "capture_vehicle_tree_state", "prepare_vehicle_transfer", "restore_vehicle_tree_state",
    "restore_vehicle_transfer", "restore_vehicle_object", "mob_object_slot",
    "mob_object_slots", "mob_object_tree_nbt", "mob_object_nbt", "retain_off_level_objects",
}
text = source.read_text()
assert not target.exists(), target
regions = []
for match in re.finditer(r"^    (?:pub[^\n]*? )?fn (\w+)", text, re.M):
    if match[1] not in names:
        continue
    start = match.start()
    while start and text[text.rfind("\n", 0, start - 1) + 1:start].startswith("    ///"):
        start = text.rfind("\n", 0, start - 1) + 1
    end = text.index("\n    }", match.end()) + len("\n    }\n")
    regions.append((start, end, match[1], text[start:end]))
assert {name for _, _, name, _ in regions} == names
moved = "\n".join(body for _, _, _, body in regions)
new = "//! Object-owned passenger memory and typed transfer loading.\nuse super::*;\n\nimpl Game<'_> {\n" + moved + "}\n"
for start, end, _, _ in reversed(regions):
    text = text[:start] + text[end:]
text = text.replace('mod loaded_pass;\n', 'mod loaded_pass;\n#[path = "persist/object_memory.rs"]\nmod object_memory;\n', 1)
for _, _, name, body in regions:
    assert new.count(body) == 1, name
    print(f"moved verbatim {name}: {hashlib.sha256(body.encode()).hexdigest()}")
source.write_text(text)
target.write_text(new)
