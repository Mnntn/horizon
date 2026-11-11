#!/usr/bin/env python3
"""Replace synthetic acting-player hints with canonical local-player fixture binding."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
PATTERNS = [
    (re.compile(r"(?m)^(?P<indent>[ \t]*)game\.lan_acting_uuid = Some\((?P<uuid>\d+)\);\n(?P=indent)game\.lan_acting_entity = Some\((?P<id>\d+)\);")),
    (re.compile(r"(?m)^(?P<indent>[ \t]*)game\.lan_acting_entity = Some\((?P<id>\d+)\);\n(?P=indent)game\.lan_acting_uuid = Some\((?P<uuid>\d+)\);")),
]

for path in sorted((ROOT / 'crates/horizon-client/src/game/modding').glob('native_*.rs')):
    # This statistics fixture has an acting service context but no living body.
    if path.name == 'native_player_stats.rs':
        continue
    original = path.read_text()
    text = original
    for pattern in PATTERNS:
        def replace(match):
            assert '#[cfg(test)]' in text[:match.start()], path
            return (match['indent'] + 'crate::game::native_entity_refs::tests::bind_local_player('
                    + f"game, {match['id']}, {match['uuid']});")
        text = pattern.sub(replace, text)
    if text != original:
        path.write_text(text)
        print(path.relative_to(ROOT))
