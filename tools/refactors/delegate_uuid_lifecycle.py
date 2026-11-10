#!/usr/bin/env python3
"""Move new lifecycle blocks verbatim behind one-line sibling delegations, one mode per commit."""
from pathlib import Path
import hashlib
import sys

root = Path(__file__).resolve().parents[2] / 'crates/horizon-client/src/game'
mode = sys.argv[1]
if mode in ('boat', 'cart-command'):
    a, b = root/'administration/commands/mod.rs', root/'entities/passengers.rs'
    kind = 'boats' if mode == 'boat' else 'carts'
    name = 'dismount_killed_boat' if mode == 'boat' else 'dismount_killed_cart'
    s, t = a.read_text(), b.read_text()
    start = s.index(f'                if let Some(y) = self.{kind}.iter().find(|b| b.id == id).map(|b| b.position.y) {{') if kind == 'boats' else s.index('                if let Some(y) = self.carts.iter().find(|c| c.id == id).map(|c| c.position.y) {')
    end = s.index('\n                }', start) + len('\n                }\n')
    replacement = f'                self.{name}(id);\n'
    signature = f'    pub(super) fn {name}(&mut self, id: u32)'
elif mode == 'cart':
    a, b = root/'entities/minecarts.rs', root/'entities/passengers.rs'
    s, t = a.read_text(), b.read_text()
    begin = s.index('    pub(super) fn eject_cart_mobs(')
    start = s.index('            let Some(mob) = self.mob_object(rider)', begin)
    end = s.index('\n        }\n    }', start) + 1
    replacement = '            self.dismount_cart_mob_at(rider, y);\n'
    signature = '    pub(super) fn dismount_cart_mob_at(&mut self, rider: u32, y: f64)'
elif mode == 'remote':
    a, b = root/'network/remote/mod.rs', root/'modding/entity_initialization.rs'
    s, t = a.read_text(), b.read_text()
    begin = s.index('            C::RemoveEntities(p) =>')
    start = s.index('                    self.forget_entity_initialization(Holder::Entity', begin)
    end = s.index('                    if let Some(r) = self.remote.as_mut()', start)
    replacement = '                    self.forget_remote_entity_construction(id);\n'
    signature = '    pub(super) fn forget_remote_entity_construction(&mut self, id: i32)'
elif mode == 'creatures':
    a, b = root/'mobs/creatures/mod.rs', root/'mobs/creatures/borrowed_tick.rs'
    s, t = a.read_text(), b.read_text()
    start = s.index('        let passenger_objects: Vec<_> = if self.persist.off_level.is_empty()')
    end = s.index('        let mut pending', start)
    replacement = '        let passenger_objects = self.retained_passenger_snapshot(uuid);\n'
    signature = '    pub(super) fn retained_passenger_snapshot(&self, uuid: u128) -> Vec<Seen>'
else:
    raise ValueError(mode)
body = s[start:end]
assert replacement not in s
s = s[:start] + replacement + s[end:]
t += '\nimpl Game<\'_> {\n' + signature + ' {\n' + body
if mode == 'creatures':
    t += '        passenger_objects\n'
t += '    }\n}\n'
assert t.count(body) == 1
print(f'moved verbatim {mode}: {hashlib.sha256(body.encode()).hexdigest()}')
a.write_text(s)
b.write_text(t)
