#!/usr/bin/env python3
"""Pure move of Ingredient matching before adding placement cancellation."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/crafting/ingredient.rs"
text = source.read_text()
start = text.index("    /// `test` with the loop context the game already holds")
end = text.index("\n}\n\n#[cfg(test)]", start)
method = text[start:end]
text = text[:start] + text[end:]
marker = "//! Engine recipe ingredients and stack matching.\n"
assert text.count(marker) == 1
source.write_text(text.replace(marker, marker + '#[path = "ingredient/matching.rs"]\nmod matching;\n'))
target = source.parent / "ingredient/matching.rs"
assert not target.exists()
target.parent.mkdir(exist_ok=True)
target.write_text("//! Concrete stack matching, shared by ordinary crafting and placement.\nuse super::*;\nimpl Ingredient {\n" + method + "\n}\n")
