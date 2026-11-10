#!/usr/bin/env python3
"""Move render geometry verbatim; --verify COMMIT checks against the source at COMMIT."""
import argparse
from pathlib import Path
import subprocess

SOURCE = 'crates/horizon-mod-render/src/render.rs'
TARGET = 'crates/horizon-client-render/src/render_geometry.rs'
START = '/// An entity, item and effect vertex'
END = '#[doc(hidden)]\npub mod built'

def geometry(source):
    return source[source.index(START):source.index(END)]

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--verify')
args = parser.parse_args()
if args.verify:
    original = subprocess.check_output(['git', 'show', f'{args.verify}:{SOURCE}'], text=True)
    # rustfmt removes the blank separator before the next source module.
    assert Path(TARGET).read_text().removesuffix('\n') == geometry(original).removesuffix('\n\n')
else:
    path = Path(SOURCE)
    original = path.read_text()
    moved = geometry(original)
    Path(TARGET).write_text(moved)
    path.write_text(original.replace(moved, 'pub use horizon_client_render::render_geometry::{EntitySpan, Vertex};\n\n'))
