#!/usr/bin/env python3
"""Delete the whole-game native mod build branch and its exclusive cache."""
from pathlib import Path
root = Path(__file__).resolve().parents[2]
p = root / 'launcher/src/native_mods.rs'
s = p.read_text()
a = s.index('    } else if !exists {', s.index('fn pipeline('))
z = s.index('    } else {\n        // Used again:', a)
s = s[:a] + s[z:]
s = s.replace('mod mixin_cache;\n', '')
p.write_text(s)
(root / 'launcher/src/native_mods/mixin_cache.rs').unlink(missing_ok=True)

# Whole-game staging and transformed-source snapshots no longer have consumers.
for relative in ['launcher/src/storage.rs', 'launcher/src/storage/inventory.rs', 'launcher/src/storage/tests.rs']:
    p = root / relative
    lines = p.read_text().splitlines(keepends=True)
    names = ('Stage', 'PendingStage', 'NewStage', 'Patched')
    p.write_text(''.join(line for line in lines if not any(
        ('BuildCache::' + name in line) or ('Self::' + name + ' =>' in line) or line.strip() == name + ','
        for name in names
    )).replace('[BuildCache; 13]', '[BuildCache; 9]'))

# Remove translations belonging to deleted launcher code, not unrelated unused keys.
import json
import subprocess
baseline = '3dec9ba153'
changed = subprocess.check_output(['git', 'diff', '--name-only', baseline, '--', 'launcher/src'], text=True).splitlines()
old_source = '\n'.join(subprocess.check_output(['git', 'show', f'{baseline}:{name}'], text=True) for name in changed)
source = '\n'.join(p.read_text() for p in (root / 'launcher/src').rglob('*.rs'))
for p in (root / 'launcher/lang').glob('*.json'):
    kept = []
    for line in p.read_text().splitlines(keepends=True):
        if line.startswith('  "'):
            key = json.JSONDecoder().raw_decode(line.lstrip())[0]
            literal = json.dumps(key, ensure_ascii=False)[1:-1]
            if literal in old_source and literal not in source:
                continue
        kept.append(line)
    p.write_text(''.join(kept))
