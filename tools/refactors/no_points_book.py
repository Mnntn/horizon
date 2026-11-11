#!/usr/bin/env python3
"""Delete point-only documentation and fixtures; run from the repository root."""
from pathlib import Path
import shutil

for name in (
    'docs/modding/src/injections.md',
    'docs/modding/src/concepts/guest-point-artifact.md',
    'tools/ci/mixins-check', 'tools/ci/mixins_check.sh',
    'tools/ci/mixins_area_execution.py', 'tools/ci/test_mixins_area_execution.py',
    'tools/ci/listed_calls.py', 'tools/fixtures/native-slot-order',
    '.github/workflows/mixins-check.yml',
):
    path = Path(name)
    if path.is_dir():
        shutil.rmtree(path)
    elif path.exists():
        path.unlink()
summary = Path('docs/modding/src/SUMMARY.md')
summary.write_text(''.join(line for line in summary.read_text().splitlines(True)
                           if 'injections.md' not in line and 'guest-point-artifact.md' not in line))
