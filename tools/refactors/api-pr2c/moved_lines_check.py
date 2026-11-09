#!/usr/bin/env python3
"""Prove rename-only Rust files preserve their line multiset.

Declared edits: the two TSV rename lists, rustfmt whitespace/wrapping, and removal
of area impls now generated (the access/target gate checks those independently).
Generator, macro, phase and declaration changes are reviewed separately, not
claimed as pure moves. All alias names are canonicalised in both trees, including
unchanged homonyms; compiler-edits.jsonl records the internal caller span edits.
"""
import collections
import json
import re
import runpy
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).parent
BASE = sys.argv[1]
parent = {}


def find(n):
    parent.setdefault(n, n)
    if parent[n] != n:
        parent[n] = find(parent[n])
    return parent[n]


for table in ('internal-renames.tsv', 'forwarder-renames.tsv'):
    rows = (HERE / table).read_text().splitlines()
    print(f'Declared edits ({table}, {len(rows)}):')
    for row in rows:
        print('  ' + row)
        old, new = row.split('\t')[:2]
        parent[find(old)] = find(new)
pattern = re.compile(r'\b(' + '|'.join(map(re.escape, parent)) + r')\b')
blocks = runpy.run_path(str(HERE / 'rename.py'))['blocks']


def canonical(source):
    source = pattern.sub(lambda m: find(m[0]), source)
    output = subprocess.run(['rustfmt', '--edition', '2024', '--config', 'skip_children=true', '--emit', 'stdout'],
                            input=source, text=True, capture_output=True, check=True).stdout
    return collections.Counter(line for line in output.splitlines() if line.strip())


import_edits = json.loads((HERE / 'unused-import-edits.json').read_text())
print(f'Declared rustc unused-import removals: {len(import_edits)}')
for record in import_edits:
    print('  ' + record['path'] + ': ' + repr(record['old']))
checked = 0
for name in subprocess.check_output(['git', 'diff', '--name-only', '--diff-filter=M', BASE, '--', '*.rs'], text=True).splitlines():
    if name.startswith(('crates/horizon-mod-api-gen/', 'crates/horizon-mod-api-macros/', 'crates/horizon-mod-api/src/host/', 'crates/horizon-mod-api/src/hooks/')) or name in {
        'crates/horizon-mod-api/src/world/phase.rs', 'crates/horizon-client/src/game/host/mod.rs',
    }:
        continue
    source = subprocess.check_output(['git', 'show', f'{BASE}:{name}'], text=True)
    if name in {'crates/horizon-client/src/game/host/draw.rs', 'crates/horizon-client/src/game/host/services.rs'}:
        edits = []
        for impl, body, offset in blocks(source, r"impl (\w+) for Game<'_>\s*\{"):
            if impl[1] in {'DrawClientHost', 'ServicesServerHost', 'ServicesClientHost'}:
                edits.append((impl.start(), offset + len(body) + 1))
        for start, end in reversed(edits):
            source = source[:start] + source[end:]
    current = Path(name).read_text()
    for record in import_edits:
        if record['path'] == name:
            assert source.count(record['old']) == 1 and record['old'] not in current, name
            source = source.replace(record['old'], '')
    old, new = canonical(source), canonical(current)
    if old != new:
        print(f'FAILED {name}:')
        print('removed:', old-new)
        print('added:', new-old)
        sys.exit(1)
    checked += 1
print(f'Moved-lines check passed: {checked} rename-only Rust files, normalised line multisets identical.')
