#!/usr/bin/env python3
"""Mechanical guard-argument addition before placement callback checks.

Run at the parent of the generated commit; the next commit implements the guard.
"""
from pathlib import Path
import re

root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/menu/place/custom.rs"
tests = root / "crates/horizon-client/src/menu/place/custom_tests.rs"
text = source.read_text()
old = "        allow_drop: bool,\n    ) -> Result<PostPlace, Refused> {"
assert text.count(old) == 1
source.write_text(text.replace(old, "        allow_drop: bool,\n        _guard: impl Fn() -> bool,\n    ) -> Result<PostPlace, Refused> {"))
text, count = re.subn(r"(menu\.place_custom_recipe\([^\n]*, (?:false|true))(\),)", r"\1, || true\2", tests.read_text())
assert count > 0
tests.write_text(text)
