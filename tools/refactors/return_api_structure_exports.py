#!/usr/bin/env python3
"""Return the two mod-only export fragments to their facade crate, verbatim."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    for name in ('structure_template.rs', 'structure_type.rs'):
        source = ROOT / 'crates/horizon-mod-vocab/src/exports' / name
        target = ROOT / 'crates/horizon-mod-api/src/exports' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        source.rename(target)


if __name__ == '__main__':
    main()
