#!/usr/bin/env python3
"""Switch engine manifests to vocabulary without rewriting Rust source (#2534)."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ('horizon-client', 'horizon-client-lib', 'horizon-client-render')
COMMENT = ('# Engine-layer vocabulary, named mod_api to preserve source paths (#2534); '
           'the platform facade is test-only.')


def main():
    for crate in CRATES:
        path = ROOT / 'crates' / crate / 'Cargo.toml'
        normal, marker, tests = path.read_text().partition('[dev-dependencies]')
        normal = normal.replace(
            'horizon-mod-api = { path = "../horizon-mod-api" }',
            'mod_api = { package = "horizon-mod-vocab", path = "../horizon-mod-vocab" } ' + COMMENT,
        )
        tests = tests.replace(
            'horizon-mod-api = { path = "../horizon-mod-api",',
            'mod_api_facade = { package = "horizon-mod-api", path = "../horizon-mod-api",',
        )
        path.write_text(normal + marker + tests)
    path = ROOT / 'tools/layers.json'
    config = json.loads(path.read_text())
    removed = {f'{crate} -> horizon-mod-api' for crate in CRATES} | {
        'horizon-client -> horizon-mod-api-gen',
        'horizon-client -> horizon-mod-api-macros',
        'horizon-client-render -> horizon-mod-api-macros',
    }
    config['allowed'] = [row for row in config['allowed'] if row not in removed]
    path.write_text(json.dumps(config, indent=2) + '\n')


if __name__ == '__main__':
    main()
