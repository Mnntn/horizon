#!/usr/bin/env python3
"""Extract baseline read bodies and put proposed overlay reads behind thin entries."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
BASE = "686c686e75"
AREA = "crates/horizon-client/src/game/modding"

def method(source, name):
    start = source.index("    pub(super) fn " + name + "(")
    end = source.index("\n    ///", start)
    return source[start:end] + "\n"

for file, name, signature, argument, cold in [
    ("native_entity_refs", "native_entity_target", "&self, entity: EntityRef", "entity", """
        if self.tick_item_view(entity.id()).is_some_and(|stack| stack.is_none()) { return None; }
        self.unscoped_native_entity_target(entity)
"""),
    ("native_item_entity_refs", "item_entity_stack", "&self, item: ItemEntityRef", "item", """
        self.unscoped_item_entity_index(item)?;
        if let Some(stack) = self.tick_item_view(item.entity().id()) { return stack.cloned(); }
        self.unscoped_item_entity_stack(item)
"""),
    ("native_item_places", "game_stack", "&mut self, place: &ItemPlace", "place", """
        if !self.places_simulated() { return None; }
        if let ItemPlace::ItemEntity { id } = place {
            if let Some(stack) = self.tick_item_view(*id) { return stack.cloned(); }
        }
        self.unscoped_game_stack(place)
"""),
]:
    path = ROOT / AREA / (file + ".rs")
    base = subprocess.check_output(["git", "show", f"{BASE}:{AREA}/{file}.rs"], cwd=ROOT, text=True)
    original = method(base, name)
    result = original.split(" -> ", 1)[1].split(" {", 1)[0]
    raw = original.replace("fn " + name + "(", "fn unscoped_" + name + "(", 1)
    extra = ""
    if name == "item_entity_stack":
        raw = raw.replace("self.native_item_entity_index(item)", "self.unscoped_item_entity_index(item)")
        start = base.index("    fn native_item_entity_index(")
        end = base.index("\n    ///", start)
        extra = base[start:end].replace("fn native_item_entity_index(", "fn unscoped_item_entity_index(").replace("self.native_entity_target(", "self.unscoped_native_entity_target(") + "\n"
    wrapper = f"    pub(super) fn {name}({signature}) -> {result} {{\n        if self.block_entity_tick_items.is_some() {{ return self.scoped_{name}({argument}); }}\n        self.unscoped_{name}({argument})\n    }}\n"
    included = "impl Game<'_> {\n" + raw + extra + f"    #[cold]\n    #[inline(never)]\n    fn scoped_{name}({signature}) -> {result} {{\n" + cold + "    }\n}\n"
    destination = path.parent / file / "tick_item_read.rs"
    assert not destination.exists(), destination
    destination.parent.mkdir(exist_ok=True)
    destination.write_text(included)
    text = path.read_text()
    current = method(text, name)
    assert text.count(current) == 1
    text = text.replace(current, wrapper)
    text += f'\ninclude!("{file}/tick_item_read.rs");\n'
    path.write_text(text)
