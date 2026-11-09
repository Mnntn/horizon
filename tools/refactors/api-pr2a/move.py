#!/usr/bin/env python3
"""Replay API PR 2a on an unmoved checkout, including the resident-storage slice.

Run from the repository root. All source transformations, including the declared
registry seam, are here. A second run is a no-op. No generated API files change.
"""
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[3]
RUNTIME = ROOT / "crates/horizon-mod-runtime"
MOVED = ["attachments.rs", "attachments/index.rs", "attachments/lifetime.rs",
         "attachments/observer.rs", "attachments/resident.rs"]


def replace(text, old, new):
    assert old in text, f"missing source: {old[:100]}"
    return text.replace(old, new)


def extract(text, start, end):
    return text[text.index(start):text.index(end)]


def move():
    if (RUNTIME / "src/attachments.rs").exists():
        print("API PR 2a already applied")
        return
    changed = set()

    def write(path, text):
        path = ROOT / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        if path.suffix == ".rs":
            changed.add(path)

    for name in MOVED:
        source = ROOT / "crates/horizon-world/src" / name
        if not source.exists():
            assert name == "attachments/resident.rs", name
            continue
        destination = RUNTIME / "src" / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["git", "mv", str(source), str(destination)], cwd=ROOT, check=True)
        changed.add(destination)
    write("crates/horizon-mod-runtime/Cargo.toml", '''[package]
name = "horizon-mod-runtime"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true
publish = false
description = "Engine-independent mod storage, attachment indexes and registry slot tables"

[dependencies]
horizon-core = { path = "../horizon-core" }

[lints]
workspace = true
''')
    write("crates/horizon-mod-runtime/src/lib.rs", '''//! Engine-independent storage and registration machinery shared by the game and mod API.
pub mod attachments;
/// Load-owned registry names and numeric slots, without game or API key types.
pub mod registry_keys;
''')
    # The only engine-free game helper becomes generic over an already resolved holder.
    path = ROOT / "crates/horizon-client/src/game/modding/attachment_index/mod.rs"
    text = path.read_text()
    bind = extract(text, "pub(super) fn bind_entity_map(", "#[cfg(test)]")
    text = text.replace(bind, "")
    text = text.replace("AttachmentLifetime, Attachments, Holder", "AttachmentLifetime, Holder")
    text = replace(text,
        "bind_entity_map(map, index, mod_api::EntityRef::new(level.epoch(), id), slot);",
        "slot.bind(map, index, AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id)));")
    write(path, text)
    path = ROOT / "crates/horizon-mod-runtime/src/attachments/lifetime.rs"
    text = path.read_text()
    method = bind.replace("pub(super) fn bind_entity_map(", "pub fn bind<H: Copy + Eq + std::hash::Hash + Send + Sync + 'static>(\n    &mut self,")
    method = method.replace("map: &mut Attachments", "map: &mut super::Attachments")
    method = method.replace("AttachmentIndex<AttachmentHolder>", "super::index::AttachmentIndex<H>")
    method = method.replace("entity: mod_api::EntityRef,", "holder: H,")
    method = method.replace("    slot: &mut AttachmentLifetime,\n", "")
    method = method.replace("*slot = AttachmentLifetime::new(map.bind_index(index, AttachmentHolder::Entity(entity)));",
                            "*self = AttachmentLifetime::new(map.bind_index(index, holder));")
    method = method.replace("Arc<Mutex<", "std::sync::Arc<std::sync::Mutex<")
    text = replace(text, "impl AttachmentLifetime {", "impl AttachmentLifetime {\n    /// Bind an authoritative entity's map to its already resolved holder identity.\n" + method)
    write(path, text)
    path = ROOT / "crates/horizon-client/src/game/modding/mod_api_host.rs"
    text = path.read_text()
    text = replace(text, "super::attachment_index::bind_entity_map(", "self.paintings[i].attachment_lifetime.bind(")
    text = replace(text, "                        &mut self.paintings[i].attachment_lifetime,\n", "")
    text = replace(text, "                        mod_api::EntityRef::new(level.epoch(), id),", "                        mod_api::AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id)),")
    write(path, text)

    # Split the generic name tables from API keys and the owning Parts state.
    path = ROOT / "crates/horizon-mod-api/src/content/registry_keys.rs"
    api = path.read_text()
    tables = extract(api, "#[derive(Clone, Default)]", "impl BlockKey {")
    api = api.replace(tables, "use horizon_mod_runtime::registry_keys::{Effects, Names, Requests};\n\n")
    tables = tables.replace("pub(crate)", "pub")
    tables = tables.replace("    entries:", "    pub entries:").replace("    utf16_slots:", "    pub utf16_slots:").replace("    world_fluids:", "    pub world_fluids:")
    tables = tables.replace("struct Request {", "pub struct Request {").replace("    id: String,", "    pub id: String,").replace("    mods: Vec<&'static str>,", "    pub mods: Vec<&'static str>,")
    tables = tables.replace("Vec<Option<FluidKey>>", "Vec<Option<u32>>")
    tables = tables.replace("!horizon_mod_package::content::namespaced_id(&id)", "horizon_core::identifier::Identifier::parse_namespaced(&id).is_none()")

    dimensions = extract(api, "/// The vanilla dimensions,", "impl DimensionKey {")
    api = api.replace(dimensions, "")
    funcs = extract(api, "/// The key of dimension `id`,", "/// A fluid requested by a mod")
    api = api.replace(funcs, '''/// The key of dimension `id`, numbering it when the game meets it for the first time.
pub fn dimension_key(id: &str) -> DimensionKey {
    DimensionKey(horizon_mod_runtime::registry_keys::dimension_slot(id))
}

/// The name of a dimension's key, None for a slot the game never gave out.
pub fn dimension_id(key: DimensionKey) -> Option<&'static str> {
    horizon_mod_runtime::registry_keys::dimension_id(key.0)
}

''')
    funcs = funcs.replace("dimension_key(id: &str) -> DimensionKey", "dimension_slot(id: &str) -> u32")
    funcs = funcs.replace("DimensionKey(slot as u32)", "slot as u32").replace("DimensionKey(first + slot)", "first + slot")
    funcs = funcs.replace("dimension_id(key: DimensionKey)", "dimension_id(slot: u32)").replace("let slot = key.0 as usize;", "let slot = slot as usize;")

    effects = extract(api, "/// The resolved effect slots.", "/// Resolve an effect operation")
    api = api.replace(effects, '''/// Resolve requested effects once, after every mod's content has been registered.
pub fn resolve_effects<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<usize>,
) -> Result<(), Vec<String>> {
    let effects = horizon_mod_runtime::registry_keys::resolve_effects(
        &crate::part_table::requested_keys().effects, ids, resolve,
    )?;
    let _ = crate::part_table::resolved_effects().set(effects);
    Ok(())
}

''')
    effects = effects.replace("pub(crate)", "pub").replace("    keys: Vec<EffectKey>", "    pub keys: Vec<u32>")
    effects = replace(effects, '''pub fn resolve_effects() -> Result<(), Vec<String>> {
    use horizon_client_core::potions;
    let installed = crate::part_table::requested_keys();
    let all: Vec<_> = potions::all_effects().collect();
    let mut names = installed.effects.clone();''', '''pub fn resolve_effects<'a>(
    requests: &Names,
    ids: impl IntoIterator<Item = &'a str>,
    mut resolve: impl FnMut(&str) -> Option<usize>,
) -> Result<Effects, Vec<String>> {
    let all: Vec<_> = ids.into_iter().collect();
    let mut names = requests.clone();''')
    effects = replace(effects, "for def in &all {", "for id in &all {")
    effects = replace(effects, '.request("horizon", potions::namespaced(def.id))', '.request("horizon", id)')
    effects = replace(effects, "|id| all.iter().position(|def| potions::namespaced(def.id) == id)", "&mut resolve")
    effects = effects.replace("vec![EffectKey(u32::MAX); all.len()]", "vec![u32::MAX; all.len()]").replace("keys[index] = EffectKey(slot as u32)", "keys[index] = slot as u32")
    effects = replace(effects, "let _ = crate::part_table::resolved_effects().set(Effects {", "Ok(Effects {")
    effects = replace(effects, "    });\n    Ok(())", "    })")

    be = extract(api, "/// Resolve all block entity types", "/// Lookup an engine-owned name")
    api = api.replace(be, '''/// Resolve all block entity types after registration and content loading.
pub fn resolve_block_entity_types<'a, T>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<T>,
) -> Result<(), Vec<String>> {
    let names = horizon_mod_runtime::registry_keys::resolve_block_entity_types(
        &crate::part_table::requested_keys().block_entity_types, ids, resolve,
    )?;
    let _ = crate::part_table::resolved_block_entity_types().set(names);
    Ok(())
}

''')
    be = replace(be, '''pub fn resolve_block_entity_types(
    types: &horizon_client_core::block_entity_types::BlockEntityTypes,
) -> Result<(), Vec<String>> {
    let mut names = crate::part_table::requested_keys()
        .block_entity_types
        .clone();''', '''pub fn resolve_block_entity_types<'a, T>(
    requests: &Names,
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<T>,
) -> Result<Names, Vec<String>> {
    let mut names = requests.clone();''')
    be = be.replace("|id| types.key(id)", "resolve").replace("for id in types.names()", "for id in ids")
    be = replace(be, "    let _ = crate::part_table::resolved_block_entity_types().set(names);\n    Ok(())", "    Ok(names)")
    api = api.replace("use std::collections::HashMap;\n", "").replace("use std::sync::{LazyLock, RwLock};\n", "")
    api = api.replace(".copied().map(FluidKey)", ".copied()")
    api = replace(api, "        .flatten()", "        .flatten()\n        .map(FluidKey)")
    api = replace(api, "        .copied()\n}", "        .copied()\n        .map(EffectKey)\n}")
    # Parts state uses the runtime types directly, not a crate-private re-export.
    for p in (ROOT / "crates/horizon-mod-api/src/part_table").glob("*.rs"):
        src = p.read_text()
        for name in ("Requests", "Names", "Effects"):
            src = src.replace(f"super::registry_keys::{name}", f"horizon_mod_runtime::registry_keys::{name}")
        if src != p.read_text():
            write(p, src)
    api = api.replace("use horizon_mod_runtime::registry_keys::{Effects, Names, Requests};\n", "")
    write(path, api)
    write("crates/horizon-mod-runtime/src/registry_keys.rs", '''//! Load-owned declarations and resolved mappings for registration-slot keys.
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

''' + tables + dimensions + funcs + effects + be)

    # Rewrite typed registry callers at the boundary, including test fixtures. The
    # expressions are retained; only their names and lookup functions cross inward.
    for path in (ROOT / "crates").rglob("*.rs"):
        if path.name == "registry_keys.rs":
            continue
        text = path.read_text()
        previous = text
        pattern = re.compile(r"((?:crate::registry_keys|mod_api::__private)::resolve_effects)\(\)")
        text = pattern.sub(lambda m: m[1] + "(horizon_client_core::potions::all_effects().map(|def| horizon_client_core::potions::namespaced(def.id)), |id| horizon_client_core::potions::effect(id).map(|def| def.index))", text)
        pattern = re.compile(r"((?:crate::registry_keys|mod_api::__private)::resolve_block_entity_types)\(")
        # Existing arguments may be constructor calls, so match balanced parentheses.
        for match in reversed(list(pattern.finditer(text))):
            start = match.end()
            depth, end = 1, start
            while depth:
                if text[end] == "(": depth += 1
                elif text[end] == ")": depth -= 1
                end += 1
            arg = text[start:end - 1].strip().removesuffix(",").strip()
            if re.fullmatch(r"&?\w+", arg):
                binding = arg.removeprefix("&")
                call = match[1] + "(" + binding + ".names(), |id| " + binding + ".key(id))"
            else:
                call = "{ let types = " + arg + "; " + match[1] + "(types.names(), |id| types.key(id)) }"
            text = text[:match.start()] + call + text[end:]
        if text != previous:
            write(path, text)

    # Direct attachment paths, grouped imports, and macro users across all crates.
    for path in (ROOT / "crates").rglob("*.rs"):
        text = path.read_text()
        old = text
        text = text.replace("horizon_world::attachments", "horizon_mod_runtime::attachments")
        text = text.replace("horizon_world::impl_attachment_lifetime", "horizon_mod_runtime::impl_attachment_lifetime")
        pattern = re.compile(r"horizon_world::\{\s*attachments::(\{[^{}]*\}|\w+),", re.S)
        def imports(match):
            prefix, body = match[1], match[2]
            updated = pattern.sub(lambda m: "horizon_mod_runtime::attachments::" + m[1] + ", horizon_world::{", body)
            if updated != body and not body.lstrip().startswith("{"):
                updated = "{" + updated + "}"
            return prefix + updated + ";"
        text = re.sub(r"((?:pub )?use )([^;]+);", imports, text)
        if path == ROOT / "crates/horizon-world/src/lib.rs":
            text = replace(text, "pub mod attachments;\n", "")
        if text != old:
            write(path, text)

    path = ROOT / "crates/horizon-mod-api-gen/src/wasm/index.rs"
    write(path, replace(path.read_text(),
        '("horizon_core", "crates/horizon-core/src"),',
        '("horizon_core", "crates/horizon-core/src"),\n    ("horizon_mod_runtime", "crates/horizon-mod-runtime/src"),'))
    # Every crate compiling the affected sources needs a direct dependency. The
    # client-lib manifest uses sources physically stored in horizon-client.
    for crate in ["horizon-world", "horizon-client", "horizon-client-lib", "horizon-client-render", "horizon-mod-api", "horizon-protocol"]:
        path = ROOT / "crates" / crate / "Cargo.toml"
        text = path.read_text()
        text = replace(text, "[dependencies]\n", '[dependencies]\nhorizon-mod-runtime = { path = "../horizon-mod-runtime" }\n')
        write(path, text)
    path = ROOT / "Cargo.toml"
    write(path, replace(path.read_text(), '    "crates/horizon-core",', '    "crates/horizon-core",\n    "crates/horizon-mod-runtime",'))
    # Source-path references outside Rust: no stale old attachment paths in docs,
    # port maps, internals or workflow inputs. Do not rewrite this script/proof.
    for name in subprocess.check_output(["git", "ls-files"], cwd=ROOT, text=True).splitlines():
        path = ROOT / name
        if path.suffix == ".rs" or name.startswith("tools/refactors/api-pr2a/") or not path.is_file():
            continue
        data = path.read_bytes()
        if b"\0" in data:
            continue
        try:
            text = data.decode()
        except UnicodeDecodeError:
            continue
        new = text.replace("crates/horizon-world/src/attachments", "crates/horizon-mod-runtime/src/attachments").replace("horizon_world::attachments", "horizon_mod_runtime::attachments")
        if name == "AGENTS.md":
            row = "| `horizon-world` |"
            new = replace(new, row, "| `horizon-mod-runtime` | Engine-independent mod attachment storage, query index and generic registry slot tables; depends only on `horizon-core` |\n" + row)
        if new != text:
            write(path, new)
    write("crates/horizon-mod-runtime/tests/registry_keys.rs",
          (Path(__file__).parent / "registry_tests.rs").read_text())
    path = ROOT / "docs/modding/src/concepts/trait-mods/028-server-attachments.md"
    text = path.read_text()
    text += '\nThe API owns these public names. Their engine-independent storage and query index\nlive in `horizon-mod-runtime`, which depends only on the shared vocabulary in\n`horizon-core`; world and game storage use the same implementation.\n'
    write(path, text)
    subprocess.run(["rustfmt", "--edition", "2024", "--config", "skip_children=true", *map(str, sorted(changed))], cwd=ROOT, check=True)
    # Resolve the workspace graph without compiling. --no-deps does not refresh
    # Cargo.lock's package edges, so use full metadata for the new runtime edge.
    subprocess.run(["cargo", "metadata", "--format-version", "1"], cwd=ROOT, stdout=subprocess.DEVNULL, check=True)
    # The launcher is a separate workspace but also consumes world/protocol.
    subprocess.run(["cargo", "update", "--workspace", "--offline", "--manifest-path", "launcher/Cargo.toml"], cwd=ROOT, check=True)
    print(f"API PR 2a: moved attachment files and generic key tables; formatted {len(changed)} Rust files")


if __name__ == "__main__":
    move()
