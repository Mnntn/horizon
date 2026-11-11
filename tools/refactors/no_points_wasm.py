"""Remove wasm point-only modules and dependency declarations."""
from pathlib import Path

for name in ('src/points.rs', 'tests/native_listener.rs', 'tests/no_wire_registration.rs', 'tests/fixture/mod-points.json', 'tests/fixture/src/points.rs', 'tests/support/catalog.rs'):
    Path('crates/horizon-mod-wasm', name).unlink(missing_ok=True)
Path('crates/cargo-horizon-mod/src/wasm/group/points.rs').unlink()
p = Path('crates/horizon-mod-wasm/Cargo.toml')
s = p.read_text()
s = '\n'.join(line for line in s.split('\n') if not any(x in line for x in ('mod_points =', 'horizon-mod-point-emit =')))
s = s[:s.index('# Synthetic points')]
p.write_text(s)
for p in Path('crates/horizon-mod-wasm/tests').rglob('Cargo.toml'):
    p.write_text('\n'.join(l for l in p.read_text().split('\n') if 'horizon-mod-points' not in l))
