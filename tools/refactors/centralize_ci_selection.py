#!/usr/bin/env python3
"""Move dyna-checks' repeated selection into changes (base: 3fe084c56b)."""
from pathlib import Path
import re

path = Path(__file__).resolve().parents[2] / '.github/workflows/dyna-ci.yml'
source = path.read_text()
if 'steps.selection.outputs.rust_required' in source:
    print('selection is already centralized')
    raise SystemExit(0)
source = source.replace("steps.rust.outputs.required", "needs.changes.outputs.rust_required")
source = source.replace("steps.rust.outputs.sample", "needs.changes.outputs.sample_required")
source = source.replace("steps.launcher.outputs.required", "needs.changes.outputs.launcher_required")
# Remove the three per-job selector steps, not the surrounding checks.
source, count = re.subn(r'      - name: Does this pull request (?:need Rust checks\?|change the launcher or a crate it builds\?)\n.*?        run: python3 tools/ci/rust_changes.py[^\n]*\n', '', source, flags=re.S)
assert count == 3, count
source = source.replace('    outputs:\n', '''    outputs:
      rust_required: ${{ steps.selection.outputs.rust_required || 'true' }}
      launcher_required: ${{ steps.selection.outputs.launcher_required || 'true' }}
      tooling_required: ${{ steps.selection.outputs.tooling_required || 'true' }}
      sample_required: ${{ steps.selection.outputs.sample_required || 'true' }}
''', 1)
anchor = '      # workflow_run omits pull_requests for fork runs.'
selection = '''      - name: Fetch this commit's history for selection
        if: github.event_name == 'pull_request'
        run: git fetch -q --no-tags --unshallow origin "$(git rev-parse HEAD)"
      - name: Select checks once for this pull request
        if: github.event_name == 'pull_request'
        id: selection
        env:
          BASE_SHA: ${{ github.event.pull_request.base.sha }}
          HEAD_SHA: ${{ github.event.pull_request.head.sha }}
        run: python3 tools/ci/rust_changes.py --all "$BASE_SHA" "$HEAD_SHA" >> "$GITHUB_OUTPUT"
'''
assert source.count(anchor) == 1
source = source.replace(anchor, selection + anchor)
# History in lint and launcher served only their now-centralized selector.
for name in ('lint', 'launcher'):
    start = source.index(f'  {name}:\n')
    following = re.search(r'^  [\w-]+:\n', source[start + len(name) + 5:], re.M)
    stop = start + len(name) + 5 + following.start() if following else len(source)
    block = source[start:stop]
    block, count = re.subn(r'      # Only this commit.s own history.*?        run: git fetch -q --no-tags --unshallow origin "\$\(git rev-parse HEAD\)"\n', '', block, flags=re.S)
    assert count == 1, (name, count)
    source = source[:start] + block + source[stop:]
path.write_text(source)
