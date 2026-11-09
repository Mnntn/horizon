#!/usr/bin/env python3
"""Lossless syn-span move of the two central Parts interfaces and implementations.

Run this script as `move.py BASE_SHA` from a clean checkout of BASE_SHA.
Build part-table-layout in the script's checkout first. Only the scripted API
moves are applied; the generator, tests and guide are hand-written changes.
"""
import argparse
import json
import re
import subprocess
from pathlib import Path

SCRIPT_ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('base', help='exact starting commit of the clean target checkout')
args = parser.parse_args()
ROOT = Path(subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], text=True).strip())
head, base = subprocess.check_output(['git', 'rev-parse', 'HEAD', args.base], cwd=ROOT, text=True).splitlines()
if head != base or subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=all'], cwd=ROOT, text=True):
    parser.error('run from a clean checkout of the specified base')
DIRECTORY = ROOT / 'crates/horizon-mod-api/src/parts'
path = DIRECTORY / 'mod.rs'
source = path.read_text()
lines = source.splitlines(keepends=True)
layout = json.loads(subprocess.check_output([SCRIPT_ROOT / 'target/debug/part-table-layout'], input=source, text=True))
changed = {path}

def kind(name):
    if 'block_entity' in name:
        return 'block_entities'
    if name in ('block_key', 'attach_block_part', 'attach_block_part_tag'):
        return 'blocks'
    if name == 'effect_key':
        return 'effects'
    if name == 'declare_entity_attributes':
        return 'entity_attributes'
    if 'entity' in name:
        return 'entities'
    if 'fluid' in name:
        return 'fluids'
    if name.startswith('game_rule_'):
        return 'game_rules'
    if name == 'register_key_mapping':
        return 'key_mappings'
    if name == 'attach_menu_part':
        return 'menus'
    if name == 'add_structure_type':
        return 'structures'
    if name == 'register_ticket_controller':
        return 'tickets'
    if name == 'provided':
        return 'capabilities'
    assert 'item' in name or name in ('place_provider', 'register_tooltip_appender'), name
    return 'items'

traits = {}
impls = {}
docs = []
removed = []
for item in layout:
    block = ''.join(lines[item['start']-1:item['end']])
    match = re.search(r'pub trait (Registrar|CapabilityProviders) \{', block)
    implementation = re.search(r'impl (Registrar|CapabilityProviders) for (\w+) \{', block)
    if not (match or implementation):
        continue
    owner = (match or implementation)[1]
    for member in item['members']:
        name = re.search(r'\bfn (\w+)', member['text'])[1]
        group = kind(name)
        text = ''.join(lines[member['start']-1:member['end']])
        (traits if match else impls).setdefault((group, owner), []).append(text)
    if match:
        docs.append(block[:match.start()] + f'pub trait {owner} {{}}\n')
    else:
        lines[item['start']-1] = f'impl {owner} for {implementation[2]} {{}}\n'
    start = item['start']-1 if match else item['start']
    removed.extend(range(start, item['end']))
assert len(docs) == 2, 'Run only on the unsplit base'
path.write_text(''.join(line for i, line in enumerate(lines) if i not in removed))
(DIRECTORY / 'unions.rs.in').write_text(''.join(docs))
for (group, owner), methods in traits.items():
    trait = ''.join(s.title() for s in group.split('_')) + owner
    target = 'PartRegistrar' if owner == 'Registrar' else 'InstalledProviders'
    p = DIRECTORY / f'{group}.rs'
    changed.add(p)
    header = f'/// {owner} operations for {group.replace("_", " ")}.\n'
    if owner == 'CapabilityProviders':
        header += '// hidden: per-kind lookup implementation, exposed for generated provider bindings\n#[doc(hidden)]\n'
    p.write_text(p.read_text() + f'\n{header}pub trait {trait} {{\n' + '\n'.join(methods) + '}\n' + f'\nimpl {trait} for {target} {{\n' + '\n'.join(impls[(group, owner)]) + '}\n')
    print(group, trait, len(methods))

# The area trait is already in these tests' glob scope; the old union imports
# become unused when the declaring methods move to this file.
p = DIRECTORY / 'game_rules.rs'
text = p.read_text()
for name in ('bool_rule_declarations_validate_slots_and_fail_atomically',
             'category_registration_refuses_ignored_errors_and_installs_snapshot'):
    header = f'    pub(crate) fn {name}() {{\n'
    old = header + '        use super::Registrar;\n'
    assert text.count(old) == 1, name
    text = text.replace(old, header)
p.write_text(text)

# Documentation links must name the declaring trait; Rustdoc does not resolve
# a supertrait's method as Union::method. Object method calls stay unchanged.
for file, replacements in {
    'crates/horizon-mod-api/src/lib.rs': {
        '[`Registrar::block_key`]': '[`Registrar::block_key`](BlocksRegistrar::block_key)',
        '[`Registrar::fluid_key`]': '[`Registrar::fluid_key`](FluidsRegistrar::fluid_key)',
        '[`Registrar::effect_key`]': '[`Registrar::effect_key`](EffectsRegistrar::effect_key)',
        '([`Registrar::declare_entity_attributes`](crate::Registrar))': '([`Registrar::declare_entity_attributes`](EntityAttributesRegistrar::declare_entity_attributes))',
    },
    'crates/horizon-mod-api/src/world/ticket_controller.rs': {
        '(super::Registrar::register_ticket_controller)': '(super::TicketsRegistrar::register_ticket_controller)',
    },
    'crates/horizon-mod-api/src/block/item_place.rs': {
        '(super::Registrar::place_provider)': '(super::ItemsRegistrar::place_provider)',
    },
}.items():
    p = ROOT / file
    changed.add(p)
    text = p.read_text()
    for old, new in replacements.items():
        assert text.count(old) == 1, (file, old)
        text = text.replace(old, new)
    p.write_text(text)

subprocess.run(['rustfmt', '--edition', '2024', '--config', 'skip_children=true',
                *map(str, sorted(changed))], check=True)
