#!/usr/bin/env python3
"""Move one existing private helper per oversized file; run each mode in its own commit."""
import hashlib
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parents[2]
choices = {
    "commands": ("game/administration/commands/mod.rs", "game/administration/commands/world_cmds.rs", "poi_type", True),
    "creatures": ("game/mobs/creatures/mod.rs", "game/mobs/creatures/borrowed_tick.rs", "mob_damage_source", True),
    "mobs": ("mobs.rs", "mobs/living.rs", "totem_death_effects", False),
}
a, b, name, method = choices[sys.argv[1]]
folder = root / "crates/horizon-client/src"
a, b = folder / a, folder / b
source, target = a.read_text(), b.read_text()
indent = "    " if method else ""
match = re.search(r"^" + indent + r"fn " + name + r"\(", source, re.M)
assert match
start = match.start()
while start:
    previous = source.rfind("\n", 0, start - 1) + 1
    if not source[previous:start].startswith(indent + "///"):
        break
    start = previous
end = source.index("\n" + indent + "}\n", match.end()) + len("\n" + indent + "}\n")
body = source[start:end]
assert name not in target
source = source[:start] + source[end:]
if method:
    target += "\nimpl Game<'_> {\n" + body + "}\n"
else:
    source = re.sub(r"^(?=(?:pub )?use )", "use living::totem_death_effects;\n", source, count=1, flags=re.M)
    target += "\n" + body
assert target.count(body) == 1
print(f"moved verbatim {name}: {hashlib.sha256(body.encode()).hexdigest()}")
a.write_text(source)
b.write_text(target)
