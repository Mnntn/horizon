#!/usr/bin/env python3
"""Pure-move the fluid declaration join out of oversized mod_content.rs."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
source = root / "crates/horizon-client/src/mod_content.rs"
target = source.with_name("mod_content_fluid_declarations.rs")
text = source.read_text()
start = text.index("/// Joins the world fluids native mods declare")
end = text.index("/// How the fluid `id` looks", start)
target.write_text("use super::*;\n\n" + text[start:end])
source.write_text(text[:start] + '#[path = "mod_content_fluid_declarations.rs"]\nmod fluid_declarations;\npub use fluid_declarations::accept_world_fluids;\n\n' + text[end:])
