#!/usr/bin/env python3
"""Replace area default bodies with one access declaration (not service traits)."""
import re
import runpy
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = 'cb01ae304c7136f1598f97650c63ccfc1da47b90'
BLOCKS = runpy.run_path(str(Path(__file__).with_name('rename.py')))['blocks']
PHASE = ROOT / 'crates/horizon-mod-api/src/world/phase.rs'


def main():
    old_phase = subprocess.check_output(['git', 'show', BASE + ':crates/horizon-mod-api/src/world/phase.rs'], text=True)
    start = old_phase.index('    let metadata = matches!(')
    end = old_phase.index('    let settings =', start)
    names = set(re.findall(r'"(\w+)"', old_phase[start:end]))
    for path in sorted((ROOT / 'crates/horizon-mod-api/src/host').glob('*.rs')):
        source = path.read_text()
        edits = []
        for trait, body, offset in BLOCKS(source, r'pub trait (\w+)\s*\{'):
            if not re.search(r'(?:Host|Backend|BackendMut)$', trait[1]):
                continue
            pattern = r'(    fn (\w+)\([^{}]*?\)[^{}]*?) \{\s*crate::phase::unprovided\(\s*"[^"]+",\s*"\w+",\s*crate::phase::LoadAccess::(\w+),?\s*\)\s*\}'
            def replace(m):
                signature, name, access = m.groups()
                if name in names or name.startswith('block_state_'):
                    access = 'Metadata'
                marker = '    #[static_call]\n' if name in {'is_loaded', 'config_files'} else ''
                if path.stem == 'services':
                    marker = '    #[service]\n'
                return f'    #[access({access})]\n{marker}{signature};'
            new_body, count = re.subn(pattern, replace, body)
            if count:
                edits.append((trait.start(), offset + len(body), '#[horizon_mod_api_macros::host_area]\n' + source[trait.start():offset] + new_body))
        for start, end, replacement in reversed(edits):
            source = source[:start] + replacement + source[end:]
        source = re.sub(r'(?m)^//!.*$', '//! Host method declarations; access defaults and game forwarders are generated.', source, count=1)
        path.write_text(source)
    phase = PHASE.read_text()
    if '    let metadata = matches!(' in phase:
        start = phase.index('    let metadata = matches!(')
        end = phase.index('    let settings =', start)
        phase = phase[:start] + '    let metadata = access == LoadAccess::Metadata;\n' + phase[end:]
        phase = phase.replace('    WorldOnly,', '    WorldOnly,\n    /// Registry and immutable metadata available in setup and reload.\n    Metadata,', 1)
    PHASE.write_text(phase)


if __name__ == '__main__':
    main()
