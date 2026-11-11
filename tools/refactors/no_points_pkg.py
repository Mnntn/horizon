#!/usr/bin/env python3
"""Delete the package schema and author tools for compiled injection points."""
from pathlib import Path
import shutil
root = Path(__file__).resolve().parents[2]
for name in ['crates/horizon-mod-package/src/manifest/inject.rs', 'crates/horizon-mod-package/src/manifest/inject', *['crates/cargo-horizon-mod/src/'+n+'.rs' for n in ['inject','inject_update','inject_sync','inject_sync_tests']], 'crates/cargo-horizon-mod/tests/mod_points.rs']:
    p = root / name
    if p.is_dir(): shutil.rmtree(p)
    elif p.exists(): p.unlink()
for name, lines in {'crates/horizon-mod-package/src/manifest.rs':['pub mod inject;'], 'crates/cargo-horizon-mod/src/main.rs':['mod inject;', 'mod inject_sync;']}.items():
    p = root / name
    p.write_text(''.join(line for line in p.read_text().splitlines(True) if line.strip() not in lines))
