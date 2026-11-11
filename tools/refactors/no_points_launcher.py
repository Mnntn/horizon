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
