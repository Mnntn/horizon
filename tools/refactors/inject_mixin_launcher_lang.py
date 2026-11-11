#!/usr/bin/env python3
"""Remove retired inject source-route translations; rename the slot refusal."""
import json
from pathlib import Path
OLD = 'E_INJECT_NO_SLOT: {id}: {reason}; slot declaration {declaration}; access = "api" never rebuilds the game; use access = "internals" where supported or request a hook'
NEW = 'E_INJECT_NO_SLOT: {id}: {reason}; slot declaration {declaration}; [[inject]] never compiles game code: use [[mixin]] in a native mod, or request the slot'
REMOVED = ['expression requires native internals source rebuilding; use internals, or request a hook', 'modify_local requires native internals source rebuilding; use internals, or request a hook', 'field requires native internals source rebuild; use attachments for API, wasm, persistence or sync', 'source injects pending (U5)']
RENAMED = {
    "Native mods are native code that runs inside your game with your rights. Allow native mods in Settings, Storage, or pass --trust-native-code.": "Native mods run inside your game with your rights; mixins compile code into the game. Allow native mods in Settings, Storage, or pass --trust-native-code.",
    "E_INTERNAL_SOURCE_INJECTION: internals sync checks library code, raw listeners and the mod's own modules, not source listeners or helpers. Use cargo horizon-mod internals add for this mod.": "E_INTERNAL_SOURCE_INJECTION: internals sync checks library code, slot listeners and supporting modules, not mixin helpers. Use cargo horizon-mod internals add for this mod.",
    "Injects outside the slots": "Mixins compile game code",
    "{mod} injects into the game's code, so the game builds from source": "{mod}: mixins compile game code",
}
for path in Path("launcher/lang").glob("*.json"):
    data = json.loads(path.read_text())
    for old in RENAMED:
        data.pop(old, None)
    for key in list(data):
        if key.startswith(("E_INJECT_FIELD_TYPE:", "E_INJECT_EXPRESSION_TYPE:")):
            data.pop(key)
    for key in REMOVED:
        data.pop(key, None)
    if OLD in data:
        data.pop(OLD)
        # A changed sentence needs human translation; do not retain stale text.
        if path.stem in ("en", "en_us", "en-US"):
            data[NEW] = NEW
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")
