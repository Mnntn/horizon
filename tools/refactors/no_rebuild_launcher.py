#!/usr/bin/env python3
"""Delete the whole-game native mod build branch and its exclusive cache."""
from pathlib import Path
root = Path(__file__).resolve().parents[2]
p = root / 'launcher/src/native_mods.rs'
s = p.read_text()
a = s.index('    } else if !exists {', s.index('fn pipeline('))
z = s.index('    } else {\n        // Used again:', a)
s = s[:a] + s[z:]
s = s.replace('mod mixin_cache;\n', '')
p.write_text(s)
(root / 'launcher/src/native_mods/mixin_cache.rs').unlink(missing_ok=True)
