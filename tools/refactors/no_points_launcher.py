#!/usr/bin/env python3
"""Remove obsolete launcher point modules and their module declarations."""
from pathlib import Path
import shutil
root = Path(__file__).resolve().parents[2]
base = root / 'launcher/src/native_mods'
names = ['inject', 'inject_cache', 'inject_text', 'mod_points', 'slots', 'sites', 'source_server']
for name in names:
    (base / (name + '.rs')).unlink(missing_ok=True)
    if (base / name).is_dir():
        shutil.rmtree(base / name)
p = root / 'launcher/src/native_mods.rs'
s = p.read_text()
for name in names:
    s = s.replace('mod ' + name + ';\n', '')
p.write_text(s)

# Drop obsolete point diagnostics from every translation, preserving line formatting.
# Tool-protocol mixin diagnostics still used by Rust are intentionally retained.
import json
source = '\n'.join(p.read_text() for p in (root / 'launcher/src').rglob('*.rs'))
for path in (root / 'launcher/lang').glob('*.json'):
    rows = path.read_text().splitlines(keepends=True)
    kept = []
    for row in rows:
        if row.startswith('  "'):
            key = json.JSONDecoder().raw_decode(row.lstrip())[0]
            obsolete = any(word in key.lower() for word in ('inject', 'slot', 'point'))
            if obsolete and json.dumps(key, ensure_ascii=False)[1:-1] not in source:
                continue
        kept.append(row)
    path.write_text(''.join(kept))
