#!/usr/bin/env python3
"""Compare every access level and call (including args) against the base tree.

Reads the actual generated game_host_modules.rs, not a predicted generator model.
Writes the full expected/actual tables beside it for inspection.
"""
import glob
import json
import os
import re
import runpy
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = sys.argv[1]
blocks = runpy.run_path(str(Path(__file__).with_name('rename.py')))['blocks']
renames = dict(row.split('\t') for row in Path(__file__).with_name('forwarder-renames.tsv').read_text().splitlines())


def old(path):
    return subprocess.check_output(['git', 'show', f'{BASE}:{path}'], text=True)


phase = old('crates/horizon-mod-api/src/world/phase.rs')
metadata = set(re.findall(r'"(\w+)"', phase[phase.index('    let metadata ='):phase.index('    let settings =')]))
expected = {}
files = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', BASE, 'crates/horizon-client/src/game/host'], text=True).splitlines()
for path in files:
    for impl, body, _ in blocks(old(path), r"impl (\w+) for Game<'_>\s*\{"):
        owner = impl[1]
        if not re.search(r'(?:Host|Backend|BackendMut)$', owner) or owner in {'Host', 'ServerHost', 'ClientHost', 'Backend', 'BackendMut', 'ClientBackendMut'}:
            continue
        for method, body, _ in blocks(body, r'\bfn (\w+)\s*\([^{}]*?\)[^{}]*\{'):
            name = method[1]
            access = re.search(r'LoadAccess::(\w+)', body)[1]
            if name in metadata or name.startswith('block_state_'):
                access = 'Metadata'
            call = re.search(r'(self\.|Self::)(native_\w+)\((.*?)\)\s*$', body, re.S)
            if call:
                receiver, target, args = call.groups()
                args = re.sub(r'\s+', '', args)
                assert renames[target] == name, (owner, name, target)
                if receiver == 'self.':
                    args = 'self' + (',' + args if args else '')
                call = f'Game::{name}({args})'
            else:
                assert body.strip().endswith('self'), (owner, name)
                call = 'self'
            expected[f'{owner}::{name}'] = {'access': access, 'call': call}

out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(max(glob.glob(str(ROOT / 'target/debug/build/horizon-client-*/out/game_host_modules.rs')), key=os.path.getmtime))
actual = {}
source = out.read_text()
for impl, body, _ in blocks(source, r"impl mod_api::prelude::(\w+) for Game<'_>\s*\{"):
    for method, body, _ in blocks(body, r'\bfn (\w+)\s*\([^{}]*?\)[^{}]*\{'):
        name = method[1]
        check = re.search(r'check_access\(\s*"(\w+)",\s*mod_api::__private::LoadAccess::(\w+),?\s*\);', body)
        assert check and check[1] == name, (impl[1], name)
        call = re.sub(r'\s+', '', body[check.end():])
        # Rustfmt allows trailing commas in calls.
        call = call.replace(',)', ')')
        key = f'{impl[1]}::{name}'
        assert key not in actual, key
        actual[key] = {'access': check[2], 'call': call}
for label, table in [('expected', expected), ('actual', actual)]:
    (out.parent / f'access-{label}.json').write_text(json.dumps(table, indent=2, sort_keys=True) + '\n')
if expected != actual:
    for key in sorted(expected.keys() | actual.keys()):
        if expected.get(key) != actual.get(key):
            print(key, 'old:', expected.get(key), 'new:', actual.get(key))
    sys.exit('access/target comparison failed')
print(f'Access and full call tables identical: {len(actual)} area methods; {out}')
